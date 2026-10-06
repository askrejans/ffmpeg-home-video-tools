//! Turning a template plus field values into a prepared, immutable scene for
//! one canvas size. All shaping, layout, font loading and precomputation
//! happens here.

use super::anim::Anim;
use super::color::Rgba;
use super::effects::{Rng, grain_tiles, smoothstep, vignette_map};
use super::spec::{
    Chroma, Direction, Frame, Glow, GlyphAnim, GradientKind, HAlign, Jitter, LayerKind,
    NoiseBandSpec, ParticleKind, ParticlesSpec, Placement, Shadow, ShapeKind, StaggerOrder,
    SweepSpec, TemplateSpec, TextSpec, TextSweep, VAlign,
};
use super::text::{Face, FontBlob, Fonts, GlyphShape, fit_text};
use crate::error::{Error, Result};
use std::collections::{BTreeMap, HashMap};
use tiny_skia::{
    GradientStop, LinearGradient, Pixmap, Point, RadialGradient, Shader, SpreadMode, Transform,
};

/// A rectangle in canvas pixels.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Area {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Area {
    fn right(&self) -> f32 {
        self.x + self.w
    }
}

/// Where a text block or shape sits and how it moves.
#[derive(Debug, Clone)]
pub(crate) struct Pos {
    pub x: Anim<f32>,
    pub y: Anim<f32>,
    pub area: Area,
    /// Block top resolved by a stack (overrides `y`/`valign`).
    pub top: Option<f32>,
    pub align: HAlign,
    pub valign: VAlign,
    pub dx: Anim<f32>,
    pub dy: Anim<f32>,
    pub scale: Anim<f32>,
    pub rotation: Anim<f32>,
}

impl Pos {
    fn new(p: &Placement, canvas: Area, safe: Area) -> Pos {
        Pos {
            x: p.x.clone(),
            y: p.y.clone(),
            area: if p.frame == Frame::Safe { safe } else { canvas },
            top: None,
            align: p.align,
            valign: p.valign,
            dx: p.dx.clone(),
            dy: p.dy.clone(),
            scale: p.scale.clone(),
            rotation: p.rotation.clone(),
        }
    }

    pub(crate) fn anchor_x(&self, t: f64, s: f32) -> f32 {
        self.area.x + self.x.at(t) * self.area.w + self.dx.at(t) * s
    }

    /// Top edge of a box `height` tall; `baseline` is the distance from the
    /// top to the first baseline (text only).
    pub(crate) fn top_at(&self, t: f64, s: f32, height: f32, baseline: f32) -> f32 {
        let dy = self.dy.at(t) * s;
        if let Some(top) = self.top {
            return top + dy;
        }
        let ay = self.area.y + self.y.at(t) * self.area.h;
        dy + match self.valign {
            VAlign::Top => ay,
            VAlign::Middle => ay - height / 2.0,
            VAlign::Bottom => ay - height,
            VAlign::Baseline => ay - baseline,
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct LineInfo {
    pub width: f32,
    pub clusters: usize,
}

#[derive(Debug, Clone)]
pub(crate) struct GlyphInst {
    pub line: usize,
    pub shape: usize,
    pub x: f32,
    pub y: f32,
    pub advance: f32,
    pub cluster: usize,
    pub delay: f64,
    #[cfg_attr(not(test), allow(dead_code))]
    pub glyph_id: u16,
}

#[derive(Debug)]
pub(crate) struct TextLayer {
    pub pos: Pos,
    pub cap: f32,
    pub line_gap: f32,
    pub px: f32,
    pub lines: Vec<LineInfo>,
    pub glyphs: Vec<GlyphInst>,
    pub shapes: Vec<GlyphShape>,
    pub color: Anim<Rgba>,
    pub tracking: Anim<f32>,
    pub skew: f32,
    pub blur: Anim<f32>,
    pub reveal: Anim<f32>,
    pub reveal_from: Direction,
    pub reveal_softness: f32,
    pub glyph_anim: Option<GlyphAnim>,
    pub shadow: Option<Shadow>,
    pub glow: Option<Glow>,
    pub chroma: Option<Chroma>,
    pub jitter: Option<Jitter>,
    pub sweep: Option<TextSweep>,
    /// The final string drawn (after field substitution and case mapping).
    #[cfg_attr(not(test), allow(dead_code))]
    pub text: String,
}

impl TextLayer {
    pub(crate) fn block_height(&self) -> f32 {
        self.cap + self.line_gap * (self.lines.len().saturating_sub(1)) as f32
    }

    fn rest_width(&self, duration: f64) -> f32 {
        let tr = self.tracking.at(duration) * self.px;
        self.lines
            .iter()
            .map(|l| l.width + tr * l.clusters.saturating_sub(1) as f32)
            .fold(0.0, f32::max)
    }
}

#[derive(Debug)]
pub(crate) struct ShapeLayer {
    pub kind: ShapeKind,
    pub pos: Pos,
    pub w: f32,
    pub h: f32,
    pub radius: f32,
    pub color: Anim<Rgba>,
    pub reveal: Anim<f32>,
    pub reveal_from: Direction,
}

#[derive(Debug)]
pub(crate) struct LetterboxLayer {
    pub bar: f32,
    pub color: Rgba,
    pub progress: Anim<f32>,
}

#[derive(Debug)]
pub(crate) enum FillLayer {
    Solid(Anim<Rgba>),
    Gradient(Shader<'static>),
}

#[derive(Debug)]
pub(crate) struct VignetteLayer {
    pub map: Vec<u8>,
    pub rgb: [u32; 3],
}

#[derive(Debug)]
pub(crate) struct GrainLayer {
    pub tiles: Vec<i8>,
    /// Side of each (cell-expanded) tile in pixels.
    pub side: usize,
    pub amount: f32,
    pub fps: f32,
    pub seed: u64,
}

#[derive(Debug)]
pub(crate) struct ScanlinesLayer {
    pub period: f32,
    pub thickness: f32,
    pub strength: f32,
    pub rgb: [u32; 3],
    pub speed: f32,
}

#[derive(Debug)]
pub(crate) struct Particle {
    pub x0: f32,
    pub y0: f32,
    pub vx: f32,
    pub vy: f32,
    pub size: f32,
    pub color: usize,
    pub alpha: f32,
    pub phase: f32,
    pub freq: f32,
    pub spin: f32,
    pub wobble: f32,
}

#[derive(Debug)]
pub(crate) struct ParticlesLayer {
    pub kind: ParticleKind,
    pub particles: Vec<Particle>,
    pub colors: Vec<Rgba>,
    /// One soft-disc sprite per colour (bokeh and dust).
    pub sprites: Vec<Pixmap>,
    pub area: Area,
    pub twinkle: f32,
}

#[derive(Debug)]
pub(crate) enum Kind {
    Text(Box<TextLayer>),
    Shape(ShapeLayer),
    Letterbox(LetterboxLayer),
    Fill(FillLayer),
    Vignette(VignetteLayer),
    Grain(GrainLayer),
    Scanlines(ScanlinesLayer),
    Sweep(SweepSpec),
    Particles(ParticlesLayer),
    NoiseBand(NoiseBandSpec),
}

#[derive(Debug)]
pub(crate) struct Layer {
    pub start: f64,
    pub end: f64,
    pub opacity: Anim<f32>,
    pub kind: Kind,
}

/// Everything needed to draw any frame, immutable after construction.
#[derive(Debug)]
pub(crate) struct Scene {
    pub w: u32,
    pub h: u32,
    pub fps: u32,
    pub frames: u64,
    /// Short canvas side in pixels: the unit for sizes in templates.
    pub s: f32,
    pub layers: Vec<Layer>,
    /// Whether installed system fonts were needed for some characters.
    #[cfg_attr(not(test), allow(dead_code))]
    pub uses_system_fonts: bool,
}

/// Normalise a user value: collapse whitespace, drop control characters and
/// cap the length (truncated values end in an ellipsis).
pub(crate) fn clean_field(raw: &str, max_chars: usize) -> String {
    let cleaned: String = raw
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .filter(|c| !c.is_control())
        .collect();
    if cleaned.chars().count() <= max_chars {
        return cleaned;
    }
    let mut cut: String = cleaned.chars().take(max_chars.saturating_sub(1)).collect();
    cut = cut.trim_end().to_string();
    cut.push('…');
    cut
}

fn rgb_u32(c: Rgba) -> [u32; 3] {
    let [r, g, b] = c.to_u8();
    [r as u32, g as u32, b as u32]
}

struct Ctx<'a> {
    spec: &'a TemplateSpec,
    w: f32,
    h: f32,
    s: f32,
    canvas: Area,
    safe: Area,
}

/// Build the scene for one canvas.
pub(crate) fn build_scene(
    spec: &TemplateSpec,
    blobs: &[FontBlob],
    fields: &BTreeMap<String, String>,
    width: u32,
    height: u32,
    fps: u32,
) -> Result<Scene> {
    if width == 0 || height == 0 || width > 16384 || height > 16384 {
        return Err(Error::Template(format!(
            "canvas size {width}x{height} is not supported"
        )));
    }
    if fps == 0 || fps > 1000 {
        return Err(Error::Template(format!(
            "frame rate {fps} is not supported"
        )));
    }
    let layout = if height > width {
        &spec.portrait
    } else {
        &spec.landscape
    };
    let (w, h) = (width as f32, height as f32);
    let canvas = Area {
        x: 0.0,
        y: 0.0,
        w,
        h,
    };
    let sa = layout.safe_area;
    let safe = Area {
        x: sa.x * w,
        y: sa.y * h,
        w: w * (1.0 - 2.0 * sa.x),
        h: h * (1.0 - 2.0 * sa.y),
    };
    let ctx = Ctx {
        spec,
        w,
        h,
        s: w.min(h),
        canvas,
        safe,
    };

    // Field values.
    let mut values: BTreeMap<&str, String> = BTreeMap::new();
    for f in &spec.fields {
        let v = clean_field(
            fields.get(&f.name).map(String::as_str).unwrap_or(""),
            f.max_chars,
        );
        if f.required && v.is_empty() {
            return Err(Error::Template(format!("field {:?} is required", f.name)));
        }
        values.insert(f.name.as_str(), v);
    }
    let filled = |name: &str| values.get(name).is_some_and(|v| !v.is_empty());

    // Which layers take part, and the text each text layer shows.
    let mut texts: Vec<Option<String>> = Vec::with_capacity(layout.layers.len());
    let mut active = Vec::with_capacity(layout.layers.len());
    for layer in &layout.layers {
        let mut on = layer.common.enabled && layer.common.requires.iter().all(|f| filled(f));
        let mut text = None;
        if let LayerKind::Text(t, _) = &layer.kind {
            let mut s = match (&t.field, &t.text) {
                (Some(f), _) => values.get(f.as_str()).cloned().unwrap_or_default(),
                (None, Some(lit)) => clean_field(lit, 500),
                (None, None) => String::new(),
            };
            if t.uppercase {
                s = s.to_uppercase();
            }
            on &= !s.is_empty();
            text = Some(s);
        }
        active.push(on);
        texts.push(text);
    }

    // Fonts: bundled first; installed fonts only when some text needs them.
    let mut fonts = Fonts::new(blobs, false);
    let needs_system = layout
        .layers
        .iter()
        .zip(&texts)
        .zip(&active)
        .any(|((_, t), on)| *on && t.as_ref().is_some_and(|t| !fonts.template_covers(t)));
    if needs_system {
        fonts = Fonts::new(blobs, true);
    }

    // Text layers first: shapes may size themselves after them.
    let mut prepared: Vec<Option<Kind>> = Vec::with_capacity(layout.layers.len());
    let mut text_widths: HashMap<&str, f32> = HashMap::new();
    for (i, layer) in layout.layers.iter().enumerate() {
        let kind = match (&layer.kind, active[i]) {
            (LayerKind::Text(t, p), true) => {
                let text = texts[i].clone().unwrap_or_default();
                let tl = prepare_text(&ctx, &mut fonts, t, p, text)?;
                if let Some(id) = &layer.common.id {
                    text_widths.insert(id.as_str(), tl.rest_width(spec.duration));
                }
                Some(Kind::Text(Box::new(tl)))
            }
            _ => None,
        };
        prepared.push(kind);
    }
    for (i, layer) in layout.layers.iter().enumerate() {
        if !active[i] || prepared[i].is_some() {
            continue;
        }
        prepared[i] = prepare_other(&ctx, &layer.kind, &text_widths, width, height);
    }

    // Stacks: lay members out vertically, skipping absent ones.
    for (name, stack) in &layout.stacks {
        let mut members: Vec<(usize, f32, f32)> = Vec::new(); // (layer, height, gap)
        for (i, layer) in layout.layers.iter().enumerate() {
            let placement = match &layer.kind {
                LayerKind::Text(_, p) | LayerKind::Shape(_, _, p) => p,
                _ => continue,
            };
            if placement.stack.as_deref() != Some(name.as_str()) {
                continue;
            }
            let height = match &prepared[i] {
                Some(Kind::Text(t)) => t.block_height(),
                Some(Kind::Shape(sh)) => sh.h,
                _ => continue,
            };
            members.push((i, height, placement.gap * ctx.s));
        }
        let total: f32 = members
            .iter()
            .enumerate()
            .map(|(k, (_, hgt, gap))| hgt + if k == 0 { 0.0 } else { *gap })
            .sum();
        let area = if stack.frame == Frame::Safe {
            ctx.safe
        } else {
            ctx.canvas
        };
        let anchor = area.y + stack.y * area.h;
        let mut top = anchor
            - total
                * match stack.valign {
                    VAlign::Top => 0.0,
                    VAlign::Middle => 0.5,
                    VAlign::Bottom | VAlign::Baseline => 1.0,
                };
        for (k, (i, hgt, gap)) in members.iter().enumerate() {
            if k > 0 {
                top += gap;
            }
            match &mut prepared[*i] {
                Some(Kind::Text(t)) => t.pos.top = Some(top),
                Some(Kind::Shape(sh)) => sh.pos.top = Some(top),
                _ => {}
            }
            top += hgt;
        }
    }

    let duration = spec.duration;
    let layers = layout
        .layers
        .iter()
        .zip(prepared)
        .filter_map(|(layer, kind)| {
            Some(Layer {
                start: layer.common.start.unwrap_or(f64::NEG_INFINITY),
                end: layer.common.end.unwrap_or(f64::INFINITY),
                opacity: layer.common.opacity.clone(),
                kind: kind?,
            })
        })
        .collect();
    let frames = ((duration * fps as f64).round() as u64).max(1);
    Ok(Scene {
        w: width,
        h: height,
        fps,
        frames,
        s: ctx.s,
        layers,
        uses_system_fonts: fonts.uses_system,
    })
}

fn prepare_text(
    ctx: &Ctx<'_>,
    fonts: &mut Fonts,
    t: &TextSpec,
    p: &Placement,
    text: String,
) -> Result<TextLayer> {
    let primary = fonts.resolve(&t.font, t.weight, t.italic).ok_or_else(|| {
        Error::Template(format!(
            "font family {:?} is not available in this template",
            t.font
        ))
    })?;
    let mut chain: Vec<Face> = vec![primary.clone()];
    for fam in t.fallback.iter().chain(&ctx.spec.fallback_fonts) {
        if let Some(f) = fonts.resolve(fam, t.weight, t.italic)
            && !chain.iter().any(|c| c.id == f.id)
        {
            chain.push(f);
        }
    }
    let others = fonts.other_template_faces(&chain, t.weight);
    chain.extend(others);

    let pos = Pos::new(p, ctx.canvas, ctx.safe);
    // Width available at the resting position, inside the safe area.
    let ax = pos.area.x + p.x.at(ctx.spec.duration) * pos.area.w;
    let (left, right) = (ctx.safe.x, ctx.safe.right());
    let avail = match p.align {
        HAlign::Left => right - ax,
        HAlign::Right => ax - left,
        HAlign::Center => 2.0 * (ax - left).min(right - ax),
    }
    .max(ctx.s * 0.1);
    let max_width = (t.max_width * ctx.w).min(avail);
    let tracking_max = t.tracking.values().into_iter().fold(0.0f32, f32::max);
    let fitted = fit_text(
        fonts,
        &text,
        &chain,
        t.size * ctx.s,
        max_width,
        t.max_lines as usize,
        tracking_max,
    );
    let px = fitted.px;

    let mut lines = Vec::new();
    let mut glyphs = Vec::new();
    let mut shapes: Vec<GlyphShape> = Vec::new();
    let mut shape_index: HashMap<(cosmic_text::fontdb::ID, u16, u32), usize> = HashMap::new();
    let mut cluster_base = 0usize;
    for (li, line_text) in fitted.lines.iter().enumerate() {
        let line = fonts.shape_line(line_text, &chain, px);
        for g in &line.glyphs {
            let key = (g.font_id, g.glyph_id, g.flags.bits());
            let shape = match shape_index.get(&key) {
                Some(i) => *i,
                None => {
                    shapes.push(fonts.glyph_shape(g, px));
                    shape_index.insert(key, shapes.len() - 1);
                    shapes.len() - 1
                }
            };
            glyphs.push(GlyphInst {
                line: li,
                shape,
                x: g.x,
                y: g.y,
                advance: g.advance,
                cluster: g.cluster,
                delay: (cluster_base + g.cluster) as f64,
                glyph_id: g.glyph_id,
            });
        }
        cluster_base += line.clusters;
        lines.push(LineInfo {
            width: line.width,
            clusters: line.clusters,
        });
    }

    // Per-glyph delays from the stagger order (stored as cluster rank first).
    let total = cluster_base.max(1);
    if let Some(st) = &t.stagger {
        let mid = (total as f64 - 1.0) / 2.0;
        let perm: Vec<usize> = if st.order == StaggerOrder::Random {
            let mut idx: Vec<usize> = (0..total).collect();
            let mut rng = Rng::new(st.seed);
            for i in (1..idx.len()).rev() {
                let j = (rng.next_u64() % (i as u64 + 1)) as usize;
                idx.swap(i, j);
            }
            idx
        } else {
            Vec::new()
        };
        for g in &mut glyphs {
            let k = g.delay;
            let rank = match st.order {
                StaggerOrder::Forward => k,
                StaggerOrder::Reverse => total as f64 - 1.0 - k,
                StaggerOrder::Center => (k - mid).abs(),
                StaggerOrder::Edges => mid - (k - mid).abs(),
                StaggerOrder::Random => perm[k as usize] as f64,
            };
            g.delay = rank * st.each as f64;
        }
    } else {
        for g in &mut glyphs {
            g.delay = 0.0;
        }
    }

    let cap = fonts.cap_height(&primary, px);
    Ok(TextLayer {
        pos,
        cap,
        line_gap: t.line_height * px,
        px,
        lines,
        glyphs,
        shapes,
        color: t.color.clone(),
        tracking: t.tracking.clone(),
        skew: -(t.skew.to_radians().tan()),
        blur: t.blur.clone(),
        reveal: t.reveal.clone(),
        reveal_from: t.reveal_from,
        reveal_softness: t.reveal_softness.max(0.0),
        glyph_anim: t.glyph.clone(),
        shadow: t.shadow.clone(),
        glow: t.glow.clone(),
        chroma: t.chroma.clone(),
        jitter: t.jitter.clone(),
        sweep: t.sweep.clone(),
        text,
    })
}

fn prepare_other(
    ctx: &Ctx<'_>,
    kind: &LayerKind,
    text_widths: &HashMap<&str, f32>,
    width: u32,
    height: u32,
) -> Option<Kind> {
    let s = ctx.s;
    Some(match kind {
        LayerKind::Text(..) => return None,
        LayerKind::Shape(shape, sp, p) => {
            let w = match &sp.width_of {
                Some(id) => {
                    sp.width_factor * text_widths.get(id.as_str())? + sp.width.unwrap_or(0.0) * s
                }
                None => sp.width.unwrap_or(0.1) * s,
            };
            let default_h = if *shape == ShapeKind::Rect {
                0.004 * s
            } else {
                w
            };
            let h = sp.height.map(|h| h * s).unwrap_or(default_h);
            Kind::Shape(ShapeLayer {
                kind: *shape,
                pos: Pos::new(p, ctx.canvas, ctx.safe),
                w,
                h,
                radius: sp.radius * s,
                color: sp.color.clone(),
                reveal: sp.reveal.clone(),
                reveal_from: sp.reveal_from,
            })
        }
        LayerKind::Letterbox(l) => {
            let mut bar = l.size.map(|v| v * ctx.h).unwrap_or(0.0);
            if let Some(a) = l.aspect
                && ctx.w / ctx.h < a
            {
                bar = bar.max((ctx.h - ctx.w / a) / 2.0);
            }
            if l.size.is_none() && l.aspect.is_none() {
                bar = 0.1 * ctx.h;
            }
            Kind::Letterbox(LetterboxLayer {
                bar,
                color: l.color,
                progress: l.progress.clone(),
            })
        }
        LayerKind::Fill(f) => match (&f.color, &f.gradient) {
            (Some(c), _) => Kind::Fill(FillLayer::Solid(c.clone())),
            (None, Some(g)) => {
                let stops: Vec<GradientStop> = g
                    .stops
                    .iter()
                    .map(|(pos, c)| GradientStop::new(pos.clamp(0.0, 1.0), c.to_skia()))
                    .collect();
                let shader = match g.kind {
                    GradientKind::Linear => {
                        let a = g.angle.to_radians();
                        let (dx, dy) = (a.cos(), a.sin());
                        let half = (ctx.w / 2.0 * dx).abs() + (ctx.h / 2.0 * dy).abs();
                        let (cx, cy) = (ctx.w / 2.0, ctx.h / 2.0);
                        LinearGradient::new(
                            Point::from_xy(cx - dx * half, cy - dy * half),
                            Point::from_xy(cx + dx * half, cy + dy * half),
                            stops,
                            SpreadMode::Pad,
                            Transform::identity(),
                        )?
                    }
                    GradientKind::Radial => {
                        let c = Point::from_xy(g.center[0] * ctx.w, g.center[1] * ctx.h);
                        let r = g.radius * (ctx.w * ctx.w + ctx.h * ctx.h).sqrt() / 2.0;
                        RadialGradient::new(
                            c,
                            0.0,
                            c,
                            r.max(1.0),
                            stops,
                            SpreadMode::Pad,
                            Transform::identity(),
                        )?
                    }
                };
                Kind::Fill(FillLayer::Gradient(shader))
            }
            (None, None) => return None,
        },
        LayerKind::Vignette(v) => Kind::Vignette(VignetteLayer {
            map: vignette_map(width, height, v.strength, v.radius, v.softness),
            rgb: rgb_u32(v.color),
        }),
        LayerKind::Grain(g) => {
            let cell = ((g.size * s).round() as usize).clamp(1, 16);
            let (tiles, side) = grain_tiles(g.seed, cell);
            Kind::Grain(GrainLayer {
                tiles,
                side,
                amount: g.amount,
                fps: g.fps,
                seed: g.seed,
            })
        }
        LayerKind::Scanlines(sc) => Kind::Scanlines(ScanlinesLayer {
            period: (sc.spacing * s).max(2.0),
            thickness: sc.thickness,
            strength: sc.strength,
            rgb: rgb_u32(sc.color),
            speed: sc.speed * ctx.h,
        }),
        LayerKind::Sweep(sw) => Kind::Sweep(sw.clone()),
        LayerKind::Particles(p) => Kind::Particles(prepare_particles(ctx, p)),
        LayerKind::NoiseBand(n) => Kind::NoiseBand(n.clone()),
    })
}

fn prepare_particles(ctx: &Ctx<'_>, p: &ParticlesSpec) -> ParticlesLayer {
    let s = ctx.s;
    let [ax0, ay0, ax1, ay1] = p.area;
    let area = Area {
        x: ax0 * ctx.w,
        y: ay0 * ctx.h,
        w: ((ax1 - ax0) * ctx.w).max(1.0),
        h: ((ay1 - ay0) * ctx.h).max(1.0),
    };
    let (default_size, default_vel) = match p.kind {
        ParticleKind::Bokeh => ([0.015, 0.06], [0.0, -0.012]),
        ParticleKind::Dust => ([0.002, 0.006], [0.004, -0.008]),
        ParticleKind::Confetti => ([0.008, 0.016], [0.0, 0.12]),
    };
    let [smin, smax] = p.size.unwrap_or(default_size);
    let [vx, vy] = p.velocity.unwrap_or(default_vel);
    let mut rng = Rng::new(p.seed);
    let particles = (0..p.count)
        .map(|_| {
            let size = rng.range(smin, smax);
            // Larger bokeh discs drift a little faster (parallax).
            let depth = if smax > smin {
                (size - smin) / (smax - smin)
            } else {
                0.5
            };
            let speed = 0.6 + 0.8 * depth;
            Particle {
                x0: rng.range(0.0, 1.0) * area.w,
                y0: rng.range(0.0, 1.0) * area.h,
                vx: (vx * speed + rng.range(-1.0, 1.0) * p.spread) * s,
                vy: (vy * speed + rng.range(-1.0, 1.0) * p.spread) * s,
                size: size * s,
                color: (rng.next_u64() % p.colors.len() as u64) as usize,
                alpha: rng.range(0.45, 1.0),
                phase: rng.range(0.0, std::f32::consts::TAU),
                freq: rng.range(0.6, 1.8),
                spin: rng.range(-1.0, 1.0) * 6.0,
                wobble: rng.range(0.2, 1.0) * s * 0.01,
            }
        })
        .collect();
    let sprites = if p.kind == ParticleKind::Confetti {
        Vec::new()
    } else {
        let size = ((smax * s).ceil() as u32).clamp(8, 256);
        p.colors
            .iter()
            .map(|c| disc_sprite(size, *c, p.softness, p.kind == ParticleKind::Bokeh))
            .collect()
    };
    ParticlesLayer {
        kind: p.kind,
        particles,
        colors: p.colors.clone(),
        sprites,
        area,
        twinkle: p.twinkle.clamp(0.0, 1.0),
    }
}

/// A soft disc; bokeh discs get a slightly brighter rim like real lens bokeh.
fn disc_sprite(size: u32, color: Rgba, softness: f32, rim: bool) -> Pixmap {
    let mut pm = Pixmap::new(size, size).unwrap_or_else(|| Pixmap::new(1, 1).expect("1x1 pixmap"));
    let c = size as f32 / 2.0;
    let soft = softness.clamp(0.02, 1.0);
    let [r, g, b] = color.to_u8();
    for y in 0..size {
        for x in 0..size {
            let dx = (x as f32 + 0.5 - c) / c;
            let dy = (y as f32 + 0.5 - c) / c;
            let d = (dx * dx + dy * dy).sqrt();
            let edge = 1.0 - smoothstep(1.0 - soft, 1.0, d);
            let body = if rim {
                0.55 + 0.35 * smoothstep(0.55, 0.9, d)
            } else {
                1.0 - 0.6 * d
            };
            let a = (edge * body * color.a).clamp(0.0, 1.0);
            let i = ((y * size + x) * 4) as usize;
            let a8 = (a * 255.0).round() as u32;
            let data = pm.data_mut();
            data[i] = ((r as u32 * a8 + 127) / 255) as u8;
            data[i + 1] = ((g as u32 * a8 + 127) / 255) as u8;
            data[i + 2] = ((b as u32 * a8 + 127) / 255) as u8;
            data[i + 3] = a8 as u8;
        }
    }
    pm
}
