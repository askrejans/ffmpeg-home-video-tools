//! Per-frame drawing. Everything here reads the immutable [`Scene`]; the only
//! allocations are small offscreen buffers for blurred or masked text.

use super::color::Rgba;
use super::effects::{
    GRAIN_TILES, Rng, apply_grain, blur_pixmap, composite_upscaled, darken_by_mask, hash_signed,
    hash2, over_px, over_span, scale_span,
};
use super::scene::{
    FillLayer, GrainLayer, Kind, LetterboxLayer, ParticlesLayer, ScanlinesLayer, Scene, ShapeLayer,
    TextLayer, VignetteLayer,
};
use super::spec::{Direction, NoiseBandSpec, ParticleKind, ShapeKind, SweepSpec};
use super::text::GlyphShape;
use tiny_skia::{
    BlendMode, FillRule, FilterQuality, GradientStop, LinearGradient, Paint, Path, PathBuilder,
    Pixmap, PixmapMut, PixmapPaint, Point, Rect, Shader, SpreadMode, Transform,
};

/// A glyph placed for one frame.
struct Placed {
    tf: Transform,
    alpha: f32,
    shape: usize,
}

/// Axis-aligned bounds accumulator in canvas pixels.
#[derive(Clone, Copy)]
struct Bounds {
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
}

impl Bounds {
    fn empty() -> Bounds {
        Bounds {
            x0: f32::MAX,
            y0: f32::MAX,
            x1: f32::MIN,
            y1: f32::MIN,
        }
    }
    fn add(&mut self, r: Rect) {
        self.x0 = self.x0.min(r.left());
        self.y0 = self.y0.min(r.top());
        self.x1 = self.x1.max(r.right());
        self.y1 = self.y1.max(r.bottom());
    }
    fn is_empty(&self) -> bool {
        self.x1 <= self.x0 || self.y1 <= self.y0
    }
    fn offset(&self, dx: f32, dy: f32) -> Bounds {
        Bounds {
            x0: self.x0 + dx,
            y0: self.y0 + dy,
            x1: self.x1 + dx,
            y1: self.y1 + dy,
        }
    }
}

/// A soft diagonal light band: `n` is the unit normal, `c` the band centre
/// along it and `half` the half-width, all in canvas pixels.
#[derive(Clone, Copy)]
struct Band {
    n: (f32, f32),
    c: f32,
    half: f32,
}

impl Band {
    /// Band crossing `b` at `progress` (0: before the left edge, 1: past the
    /// right edge), tilted `angle` degrees from vertical.
    fn across(b: Bounds, progress: f32, angle: f32, half: f32) -> Band {
        let a = angle.to_radians();
        let n = (a.cos(), a.sin());
        let proj = [
            b.x0 * n.0 + b.y0 * n.1,
            b.x1 * n.0 + b.y0 * n.1,
            b.x0 * n.0 + b.y1 * n.1,
            b.x1 * n.0 + b.y1 * n.1,
        ];
        let lo = proj.iter().copied().fold(f32::MAX, f32::min);
        let hi = proj.iter().copied().fold(f32::MIN, f32::max);
        Band {
            n,
            c: lo - half + progress * (hi - lo + 2.0 * half),
            half,
        }
    }

    fn gradient(&self, color: Rgba, tf: Transform) -> Option<Shader<'static>> {
        let (n, c, h) = (self.n, self.c, self.half);
        LinearGradient::new(
            Point::from_xy(n.0 * (c - h), n.1 * (c - h)),
            Point::from_xy(n.0 * (c + h), n.1 * (c + h)),
            vec![
                GradientStop::new(0.0, color.with_alpha(0.0).to_skia()),
                GradientStop::new(0.5, color.to_skia()),
                GradientStop::new(1.0, color.with_alpha(0.0).to_skia()),
            ],
            SpreadMode::Pad,
            tf,
        )
    }

    /// Smooth 0..1 weight of a canvas point.
    fn weight(&self, x: f32, y: f32) -> f32 {
        let v = (x * self.n.0 + y * self.n.1 - self.c) / self.half;
        let f = (1.0 - v * v).max(0.0);
        f * f
    }
}

/// A transparent buffer covering part of the canvas, optionally at reduced
/// resolution (`k` canvas pixels per buffer pixel).
struct Offscreen {
    pm: Pixmap,
    x0: f32,
    y0: f32,
    k: f32,
}

impl Offscreen {
    fn new(b: Bounds, margin: f32, k: f32, cw: u32, ch: u32) -> Option<Offscreen> {
        let x0 = (b.x0 - margin).floor().max(0.0);
        let y0 = (b.y0 - margin).floor().max(0.0);
        let x1 = (b.x1 + margin).ceil().min(cw as f32);
        let y1 = (b.y1 + margin).ceil().min(ch as f32);
        if x1 <= x0 || y1 <= y0 {
            return None;
        }
        let pw = ((x1 - x0) / k).ceil() as u32 + 1;
        let ph = ((y1 - y0) / k).ceil() as u32 + 1;
        Some(Offscreen {
            pm: Pixmap::new(pw, ph)?,
            x0,
            y0,
            k,
        })
    }

    /// Canvas coordinates to buffer coordinates.
    fn local(&self) -> Transform {
        Transform::from_scale(1.0 / self.k, 1.0 / self.k).pre_translate(-self.x0, -self.y0)
    }

    /// Draw the buffer onto the canvas. With `tint`, only its alpha is used,
    /// as coverage of that colour (glows and shadows).
    fn composite(&self, canvas: &mut PixmapMut<'_>, opacity: f32, tint: Option<Rgba>) {
        let opacity = opacity.clamp(0.0, 1.0);
        if opacity <= 0.0 {
            return;
        }
        if self.k == 1.0 {
            // Glyphs were already drawn in their final colour.
            let paint = PixmapPaint {
                opacity,
                ..PixmapPaint::default()
            };
            canvas.draw_pixmap(
                self.x0 as i32,
                self.y0 as i32,
                self.pm.as_ref(),
                &paint,
                Transform::identity(),
                None,
            );
        } else {
            let (dw, dh) = (canvas.width() as usize, canvas.height() as usize);
            let (sw, sh) = (self.pm.width() as usize, self.pm.height() as usize);
            composite_upscaled(
                canvas.data_mut(),
                dw,
                dh,
                self.pm.data(),
                sw,
                sh,
                self.x0 as usize,
                self.y0 as usize,
                self.k as usize,
                opacity,
                tint.map(|c| {
                    let [r, g, b] = c.to_u8();
                    [r as u32, g as u32, b as u32]
                }),
            );
        }
    }

    /// Keep only the part of the buffer under a light band.
    fn apply_band(&mut self, band: &Band) {
        let pw = self.pm.width() as usize;
        let (x0, y0, k) = (self.x0, self.y0, self.k);
        for (j, row) in self.pm.data_mut().chunks_exact_mut(pw * 4).enumerate() {
            let y = y0 + (j as f32 + 0.5) * k;
            for (i, px) in row.as_chunks_mut::<4>().0.iter_mut().enumerate() {
                let f = band.weight(x0 + (i as f32 + 0.5) * k, y);
                scale_span(px, (f * 256.0) as u32);
            }
        }
    }

    /// Multiply the buffer by a soft wipe over the canvas-space bounds `b`.
    fn apply_reveal(&mut self, b: Bounds, reveal: f32, from: Direction, softness: f32) {
        if reveal >= 1.0 {
            return;
        }
        let horizontal = matches!(from, Direction::Left | Direction::Right | Direction::Center);
        let extent = if horizontal { b.x1 - b.x0 } else { b.y1 - b.y0 }.max(1.0);
        let soft = (softness * extent).max(1.0);
        let coverage = |c: f32| -> f32 {
            let v = match from {
                Direction::Left => {
                    let e = b.x0 + reveal * (extent + soft);
                    (e - c) / soft
                }
                Direction::Right => {
                    let e = b.x1 - reveal * (extent + soft);
                    (c - e) / soft
                }
                Direction::Top => {
                    let e = b.y0 + reveal * (extent + soft);
                    (e - c) / soft
                }
                Direction::Bottom => {
                    let e = b.y1 - reveal * (extent + soft);
                    (c - e) / soft
                }
                Direction::Center => {
                    let mid = (b.x0 + b.x1) / 2.0;
                    let e = reveal * (extent / 2.0 + soft);
                    (e - (c - mid).abs()) / soft
                }
            };
            v.clamp(0.0, 1.0)
        };
        let (pw, ph) = (self.pm.width() as usize, self.pm.height() as usize);
        let (x0, y0, k) = (self.x0, self.y0, self.k);
        let data = self.pm.data_mut();
        if horizontal {
            let cols: Vec<u32> = (0..pw)
                .map(|i| (coverage(x0 + (i as f32 + 0.5) * k) * 256.0) as u32)
                .collect();
            for row in data.chunks_exact_mut(pw * 4) {
                for (px, &f) in row.as_chunks_mut::<4>().0.iter_mut().zip(&cols) {
                    scale_span(px, f);
                }
            }
        } else {
            for (j, row) in data.chunks_exact_mut(pw * 4).enumerate().take(ph) {
                let f = (coverage(y0 + (j as f32 + 0.5) * k) * 256.0) as u32;
                scale_span(row, f);
            }
        }
    }
}

fn solid(color: Rgba) -> Paint<'static> {
    Paint {
        shader: Shader::SolidColor(color.to_skia()),
        anti_alias: true,
        ..Paint::default()
    }
}

/// Fill placed glyphs with one colour.
fn fill_glyphs(
    pm: &mut PixmapMut<'_>,
    placed: &[Placed],
    shapes: &[GlyphShape],
    color: Rgba,
    alpha: f32,
    pre: Transform,
) {
    for p in placed {
        let a = alpha * p.alpha;
        if a < 0.002 {
            continue;
        }
        let tf = pre.pre_concat(p.tf);
        match &shapes[p.shape] {
            GlyphShape::Path(path) => {
                pm.fill_path(
                    path,
                    &solid(color.with_alpha(a)),
                    FillRule::Winding,
                    tf,
                    None,
                );
            }
            GlyphShape::Image { pixmap, left, top } => {
                let paint = PixmapPaint {
                    opacity: a.clamp(0.0, 1.0),
                    quality: FilterQuality::Bilinear,
                    ..PixmapPaint::default()
                };
                pm.draw_pixmap(
                    0,
                    0,
                    pixmap.as_ref(),
                    &paint,
                    tf.pre_translate(*left, -*top),
                    None,
                );
            }
            GlyphShape::Empty => {}
        }
    }
}

fn rounded_rect(x: f32, y: f32, w: f32, h: f32, r: f32) -> Option<Path> {
    let r = r.min(w / 2.0).min(h / 2.0).max(0.0);
    if r < 0.01 {
        return Some(PathBuilder::from_rect(Rect::from_xywh(x, y, w, h)?));
    }
    let c = r * 0.552_284_8;
    let mut pb = PathBuilder::new();
    pb.move_to(x + r, y);
    pb.line_to(x + w - r, y);
    pb.cubic_to(x + w - r + c, y, x + w, y + r - c, x + w, y + r);
    pb.line_to(x + w, y + h - r);
    pb.cubic_to(x + w, y + h - r + c, x + w - r + c, y + h, x + w - r, y + h);
    pb.line_to(x + r, y + h);
    pb.cubic_to(x + r - c, y + h, x, y + h - r + c, x, y + h - r);
    pb.line_to(x, y + r);
    pb.cubic_to(x, y + r - c, x + r - c, y, x + r, y);
    pb.close();
    pb.finish()
}

impl Scene {
    /// Draw frame `index` into `out` (premultiplied RGBA8).
    pub(crate) fn render(&self, index: u64, out: &mut [u8]) {
        let expected = self.w as usize * self.h as usize * 4;
        assert_eq!(
            out.len(),
            expected,
            "title frame buffer must be width * height * 4 bytes"
        );
        out.fill(0);
        let index = index.min(self.frames - 1);
        let t = index as f64 / self.fps as f64;
        let Some(mut canvas) = PixmapMut::from_bytes(out, self.w, self.h) else {
            return;
        };
        for layer in &self.layers {
            if t < layer.start || t > layer.end {
                continue;
            }
            let o = layer.opacity.at(t).clamp(0.0, 1.0);
            if o <= 0.001 {
                continue;
            }
            match &layer.kind {
                Kind::Text(tl) => self.draw_text(&mut canvas, tl, t, o),
                Kind::Shape(sh) => self.draw_shape(&mut canvas, sh, t, o),
                Kind::Letterbox(lb) => self.draw_letterbox(&mut canvas, lb, t, o),
                Kind::Fill(f) => self.draw_fill(&mut canvas, f, t, o),
                Kind::Vignette(v) => self.draw_vignette(&mut canvas, v, o),
                Kind::Grain(g) => self.draw_grain(&mut canvas, g, t, o),
                Kind::Scanlines(sc) => self.draw_scanlines(&mut canvas, sc, t, o),
                Kind::Sweep(sw) => self.draw_sweep(&mut canvas, sw, t, o),
                Kind::Particles(p) => self.draw_particles(&mut canvas, p, t, o),
                Kind::NoiseBand(n) => self.draw_noise_band(&mut canvas, n, t, o),
            }
        }
    }

    /// Place every visible glyph of a text layer for time `t`.
    fn place_text(&self, tl: &TextLayer, t: f64, placed: &mut Vec<Placed>) -> Option<Bounds> {
        let s = self.s;
        let mut ax = tl.pos.anchor_x(t, s);
        if let Some(j) = &tl.jitter {
            let amount = j.amount.at(t) * s;
            if amount > 0.0 && j.rate > 0.0 {
                let step = (t * j.rate as f64).floor() as i64 as u64;
                let big = hash2(j.seed.wrapping_add(7), step).is_multiple_of(4);
                ax += amount * hash_signed(j.seed, step) * if big { 1.0 } else { 0.35 };
            }
        }
        let block_h = tl.block_height();
        let top = tl.pos.top_at(t, s, block_h, tl.cap);
        let tr = tl.tracking.at(t) * tl.px;
        let align = tl.pos.align.factor();
        let mut line_x = Vec::with_capacity(tl.lines.len());
        let (mut minx, mut maxx) = (f32::MAX, f32::MIN);
        for l in &tl.lines {
            let lw = l.width + tr * l.clusters.saturating_sub(1) as f32;
            let lx = ax - align * lw;
            line_x.push(lx);
            minx = minx.min(lx);
            maxx = maxx.max(lx + lw);
        }
        let (cx, cy) = ((minx + maxx) / 2.0, top + block_h / 2.0);
        let sc = tl.pos.scale.at(t);
        let rot = tl.pos.rotation.at(t);
        let layer_tf = if sc != 1.0 || rot != 0.0 {
            Transform::from_translate(cx, cy)
                .pre_rotate(rot)
                .pre_scale(sc, sc)
                .pre_translate(-cx, -cy)
        } else {
            Transform::identity()
        };
        let skew = Transform::from_row(1.0, 0.0, tl.skew, 1.0, 0.0, 0.0);
        let mut bounds = Bounds::empty();
        for g in &tl.glyphs {
            let shape = &tl.shapes[g.shape];
            let Some(shape_bounds) = shape.bounds() else {
                continue;
            };
            let (mut ga, mut gdx, mut gdy, mut gs, mut gr) =
                (1.0f32, 0.0f32, 0.0f32, 1.0f32, 0.0f32);
            if let Some(an) = &tl.glyph_anim {
                let tg = t - g.delay;
                if let Some(a) = &an.opacity {
                    ga = a.at(tg).clamp(0.0, 1.0);
                }
                if ga <= 0.0 {
                    continue;
                }
                if let Some(a) = &an.dx {
                    gdx = a.at(tg);
                }
                if let Some(a) = &an.dy {
                    gdy = a.at(tg);
                }
                if let Some(a) = &an.scale {
                    gs = a.at(tg);
                }
                if let Some(a) = &an.rotation {
                    gr = a.at(tg);
                }
            }
            if gs <= 0.0 {
                continue;
            }
            let gx = line_x[g.line] + g.x + tr * g.cluster as f32 + gdx * s;
            let gy = top + tl.cap + g.line as f32 * tl.line_gap + g.y + gdy * s;
            let mut tf = Transform::from_translate(gx, gy);
            if gs != 1.0 || gr != 0.0 {
                let (pivot_x, pivot_y) = (g.advance / 2.0, -tl.cap / 2.0);
                tf = tf
                    .pre_translate(pivot_x, pivot_y)
                    .pre_rotate(gr)
                    .pre_scale(gs, gs)
                    .pre_translate(-pivot_x, -pivot_y);
            }
            if tl.skew != 0.0 {
                tf = tf.pre_concat(skew);
            }
            let tf = layer_tf.pre_concat(tf);
            if let Some(r) = shape_bounds.transform(tf) {
                bounds.add(r);
            }
            placed.push(Placed {
                tf,
                alpha: ga,
                shape: g.shape,
            });
        }
        (!bounds.is_empty()).then_some(bounds)
    }

    fn draw_text(&self, canvas: &mut PixmapMut<'_>, tl: &TextLayer, t: f64, o: f32) {
        let s = self.s;
        let reveal = tl.reveal.at(t).clamp(0.0, 1.0);
        if reveal <= 0.0 {
            return;
        }
        let mut placed = Vec::with_capacity(tl.glyphs.len());
        let Some(bounds) = self.place_text(tl, t, &mut placed) else {
            return;
        };
        let blur = tl.blur.at(t).max(0.0) * s;
        let mask = (reveal, tl.reveal_from, tl.reveal_softness);
        let sweep = tl.sweep.as_ref().and_then(|sw| {
            let p = sw.progress.at(t);
            (p > 0.0 && p < 1.0).then(|| {
                (
                    sw,
                    Band::across(bounds, p, sw.angle, (sw.width * s / 2.0).max(1.0)),
                )
            })
        });

        if let Some(glow) = &tl.glow {
            let a = glow.opacity.at(t) * o;
            if a > 0.003 {
                let sigma = glow.radius * s + blur;
                self.soft_pass(
                    canvas,
                    tl,
                    &placed,
                    bounds,
                    glow.color,
                    sigma,
                    a,
                    (0.0, 0.0),
                    mask,
                    None,
                );
            }
        }
        if let Some(sh) = &tl.shadow {
            let a = sh.opacity.at(t) * o;
            if a > 0.003 {
                let sigma = sh.blur * s + blur;
                let off = (sh.dx * s, sh.dy * s);
                self.soft_pass(
                    canvas, tl, &placed, bounds, sh.color, sigma, a, off, mask, None,
                );
            }
        }
        if let Some((sw, band)) = &sweep {
            // The light also blooms around the letters it passes over, which
            // keeps the sweep visible on near-white text.
            let a = sw.opacity.clamp(0.0, 1.0) * o;
            let sigma = (0.014 * s).max(band.half * 0.2) + blur;
            self.soft_pass(
                canvas,
                tl,
                &placed,
                bounds,
                sw.color,
                sigma,
                a,
                (0.0, 0.0),
                mask,
                Some(band),
            );
        }

        let color = tl.color.at(t);
        let chroma = tl
            .chroma
            .as_ref()
            .map(|c| (c, c.offset.at(t) * s, c.opacity.at(t)));
        let draw_main = |pm: &mut PixmapMut<'_>, pre: Transform, alpha: f32| {
            if let Some((c, off, ca)) = chroma
                && off.abs() > 0.05
                && ca > 0.0
            {
                fill_glyphs(
                    pm,
                    &placed,
                    &tl.shapes,
                    c.left,
                    alpha * ca,
                    pre.pre_translate(-off, 0.0),
                );
                fill_glyphs(
                    pm,
                    &placed,
                    &tl.shapes,
                    c.right,
                    alpha * ca,
                    pre.pre_translate(off, 0.0),
                );
            }
            fill_glyphs(pm, &placed, &tl.shapes, color, alpha, pre);
        };

        if blur < 0.6 && reveal >= 1.0 && sweep.is_none() {
            draw_main(canvas, Transform::identity(), o);
            return;
        }

        let k = if blur >= 0.6 {
            (blur / 3.0).floor().clamp(1.0, 8.0)
        } else {
            1.0
        };
        let chroma_margin = chroma.map(|(_, off, _)| off.abs()).unwrap_or(0.0);
        let margin = blur * 3.0 + chroma_margin + 2.0;
        let Some(mut off) = Offscreen::new(bounds, margin, k, self.w, self.h) else {
            return;
        };
        let local = off.local();
        {
            let mut pm = off.pm.as_mut();
            draw_main(&mut pm, local, 1.0);
            if let Some((sw, band)) = &sweep {
                // Brighten the letters under the band (source-atop keeps the
                // highlight inside the glyphs).
                let hl = sw.color.with_alpha(sw.opacity.clamp(0.0, 1.0));
                if let Some(shader) = band.gradient(hl, local) {
                    let paint = Paint {
                        shader,
                        blend_mode: BlendMode::SourceAtop,
                        ..Paint::default()
                    };
                    let (w, h) = (pm.width() as f32, pm.height() as f32);
                    if let Some(r) = Rect::from_xywh(0.0, 0.0, w, h) {
                        pm.fill_rect(r, &paint, Transform::identity(), None);
                    }
                }
            }
        }
        if blur >= 0.6 {
            blur_pixmap(&mut off.pm, blur / k);
        }
        off.apply_reveal(bounds, reveal, tl.reveal_from, tl.reveal_softness);
        off.composite(canvas, o, None);
    }

    /// Glow, drop shadow or sweep bloom: a blurred, tinted copy of the
    /// placed glyphs, optionally limited to a light band.
    #[allow(clippy::too_many_arguments)]
    fn soft_pass(
        &self,
        canvas: &mut PixmapMut<'_>,
        tl: &TextLayer,
        placed: &[Placed],
        bounds: Bounds,
        color: Rgba,
        sigma: f32,
        alpha: f32,
        offset: (f32, f32),
        mask: (f32, Direction, f32),
        band: Option<&Band>,
    ) {
        let k = (sigma / 3.0).floor().clamp(1.0, 8.0);
        let b = bounds.offset(offset.0, offset.1);
        let Some(mut off) = Offscreen::new(b, sigma * 3.0 + 2.0, k, self.w, self.h) else {
            return;
        };
        let pre = off.local().pre_translate(offset.0, offset.1);
        let opaque = Rgba { a: 1.0, ..color };
        fill_glyphs(&mut off.pm.as_mut(), placed, &tl.shapes, opaque, 1.0, pre);
        blur_pixmap(&mut off.pm, sigma / k);
        if let Some(band) = band {
            off.apply_band(band);
        }
        off.apply_reveal(b, mask.0, mask.1, mask.2);
        off.composite(canvas, alpha * color.a, Some(opaque));
    }

    fn draw_shape(&self, canvas: &mut PixmapMut<'_>, sh: &ShapeLayer, t: f64, o: f32) {
        let s = self.s;
        let reveal = sh.reveal.at(t).clamp(0.0, 1.0);
        let color = sh.color.at(t);
        if reveal <= 0.0 || sh.w <= 0.0 || sh.h <= 0.0 || color.a * o <= 0.0 {
            return;
        }
        let ax = sh.pos.anchor_x(t, s);
        let top = sh.pos.top_at(t, s, sh.h, sh.h);
        let x0 = ax - sh.pos.align.factor() * sh.w;
        let (cx, cy) = (x0 + sh.w / 2.0, top + sh.h / 2.0);
        let sc = sh.pos.scale.at(t);
        let rot = sh.pos.rotation.at(t);
        let tf = Transform::from_translate(cx, cy)
            .pre_rotate(rot)
            .pre_scale(sc, sc)
            .pre_translate(-cx, -cy);
        let (w, h) = (sh.w, sh.h);
        let path = match sh.kind {
            ShapeKind::Rect => {
                let (x, y, rw, rh) = match sh.reveal_from {
                    Direction::Left => (x0, top, w * reveal, h),
                    Direction::Right => (x0 + w * (1.0 - reveal), top, w * reveal, h),
                    Direction::Center => (cx - w * reveal / 2.0, top, w * reveal, h),
                    Direction::Top => (x0, top, w, h * reveal),
                    Direction::Bottom => (x0, top + h * (1.0 - reveal), w, h * reveal),
                };
                rounded_rect(x, y, rw, rh, sh.radius)
            }
            ShapeKind::Ellipse => {
                let (rw, rh) = (w * reveal, h * reveal);
                Rect::from_xywh(cx - rw / 2.0, cy - rh / 2.0, rw, rh)
                    .and_then(PathBuilder::from_oval)
            }
            ShapeKind::Triangle => {
                let (rw, rh) = (w * reveal, h * reveal);
                let (lx, ty) = (cx - rw / 2.0, cy - rh / 2.0);
                let mut pb = PathBuilder::new();
                pb.move_to(lx, ty);
                pb.line_to(lx + rw, cy);
                pb.line_to(lx, ty + rh);
                pb.close();
                pb.finish()
            }
        };
        if let Some(path) = path {
            canvas.fill_path(
                &path,
                &solid(color.with_alpha(o)),
                FillRule::Winding,
                tf,
                None,
            );
        }
    }

    fn draw_letterbox(&self, canvas: &mut PixmapMut<'_>, lb: &LetterboxLayer, t: f64, o: f32) {
        let p = lb.progress.at(t).clamp(0.0, 1.0);
        let bar = lb.bar * p;
        if bar <= 0.0 {
            return;
        }
        let (w, h) = (self.w as f32, self.h as f32);
        let paint = solid(lb.color.with_alpha(o));
        for r in [
            Rect::from_xywh(0.0, 0.0, w, bar),
            Rect::from_xywh(0.0, h - bar, w, bar),
        ]
        .into_iter()
        .flatten()
        {
            canvas.fill_rect(r, &paint, Transform::identity(), None);
        }
    }

    fn draw_fill(&self, canvas: &mut PixmapMut<'_>, f: &FillLayer, t: f64, o: f32) {
        let Some(r) = Rect::from_xywh(0.0, 0.0, self.w as f32, self.h as f32) else {
            return;
        };
        match f {
            FillLayer::Solid(c) => {
                let c = c.at(t).with_alpha(o);
                if c.a > 0.0 {
                    canvas.fill_rect(r, &solid(c), Transform::identity(), None);
                }
            }
            FillLayer::Gradient(shader) => {
                let mut shader = shader.clone();
                shader.apply_opacity(o);
                let paint = Paint {
                    shader,
                    ..Paint::default()
                };
                canvas.fill_rect(r, &paint, Transform::identity(), None);
            }
        }
    }

    fn draw_vignette(&self, canvas: &mut PixmapMut<'_>, v: &VignetteLayer, o: f32) {
        let data = canvas.data_mut();
        if v.rgb == [0, 0, 0] {
            darken_by_mask(data, &v.map, (o * 256.0) as u16);
        } else {
            let k = (o * 256.0) as u32;
            for (px, &m) in data.as_chunks_mut::<4>().0.iter_mut().zip(&v.map) {
                if m != 0 {
                    over_px(px, v.rgb, (m as u32 * k) >> 8);
                }
            }
        }
    }

    fn draw_grain(&self, canvas: &mut PixmapMut<'_>, g: &GrainLayer, t: f64, o: f32) {
        let step = (t * g.fps as f64).floor() as i64 as u64;
        let tile = (hash2(g.seed, step) % GRAIN_TILES as u64) as usize;
        let ox = (hash2(g.seed.wrapping_add(1), step) % g.side as u64) as usize;
        let oy = (hash2(g.seed.wrapping_add(2), step) % g.side as u64) as usize;
        let w = self.w as usize;
        apply_grain(
            canvas.data_mut(),
            w,
            &g.tiles,
            g.side,
            tile,
            ox,
            oy,
            g.amount * o,
        );
    }

    fn draw_scanlines(&self, canvas: &mut PixmapMut<'_>, sc: &ScanlinesLayer, t: f64, o: f32) {
        let strength = sc.strength * o;
        if strength <= 0.0 {
            return;
        }
        let (w, h) = (self.w as usize, self.h as usize);
        let period = sc.period;
        let line = (sc.thickness * period).max(0.0);
        let shift = (sc.speed as f64 * t).rem_euclid(period as f64) as f32;
        let data = canvas.data_mut();
        for (y, row) in data.chunks_exact_mut(w * 4).enumerate().take(h) {
            // Exact coverage of row [y, y+1] by the dark line centred in each period.
            let pos = (y as f32 - shift).rem_euclid(period);
            let (a0, a1) = (period / 2.0 - line / 2.0, period / 2.0 + line / 2.0);
            let overlap = |lo: f32, hi: f32| ((pos + 1.0).min(hi) - pos.max(lo)).max(0.0);
            // The row may reach into the next period's line.
            let cover = (overlap(a0, a1) + overlap(a0 + period, a1 + period)).clamp(0.0, 1.0);
            let a = (cover * strength * 255.0).round() as u32;
            over_span(row, sc.rgb, a.min(255));
        }
    }

    fn draw_sweep(&self, canvas: &mut PixmapMut<'_>, sw: &SweepSpec, t: f64, o: f32) {
        let p = sw.progress.at(t);
        if !(0.0..=1.0).contains(&p) {
            return;
        }
        let (w, h) = (self.w as f32, self.h as f32);
        let canvas_bounds = Bounds {
            x0: 0.0,
            y0: 0.0,
            x1: w,
            y1: h,
        };
        let band = Band::across(
            canvas_bounds,
            p,
            sw.angle,
            (sw.width * self.s / 2.0).max(1.0),
        );
        let color = sw.color.with_alpha((sw.intensity * o).clamp(0.0, 1.0));
        let Some(shader) = band.gradient(color, Transform::identity()) else {
            return;
        };
        // Fill only the strip the band covers.
        let (n, c, half) = (band.n, band.c, band.half);
        let d = (-n.1, n.0);
        let long = w + h;
        let pt = |u: f32, v: f32| (n.0 * u + d.0 * v, n.1 * u + d.1 * v);
        let corners = [
            pt(c - half, -long),
            pt(c + half, -long),
            pt(c + half, long),
            pt(c - half, long),
        ];
        let mut pb = PathBuilder::new();
        pb.move_to(corners[0].0, corners[0].1);
        for (x, y) in &corners[1..] {
            pb.line_to(*x, *y);
        }
        pb.close();
        if let Some(path) = pb.finish() {
            let paint = Paint {
                shader,
                ..Paint::default()
            };
            canvas.fill_path(
                &path,
                &paint,
                FillRule::Winding,
                Transform::identity(),
                None,
            );
        }
    }

    fn draw_particles(&self, canvas: &mut PixmapMut<'_>, pl: &ParticlesLayer, t: f64, o: f32) {
        let tf32 = t as f32;
        let area = pl.area;
        for p in &pl.particles {
            let margin = p.size;
            let sway = (p.freq * tf32 + p.phase).sin() * p.wobble;
            let x = p.x0 + p.vx * tf32 + sway;
            let y = p.y0 + p.vy * tf32;
            let x = area.x - margin + (x + margin).rem_euclid(area.w + 2.0 * margin);
            let y = area.y - margin + (y + margin).rem_euclid(area.h + 2.0 * margin);
            let tw = 1.0 - pl.twinkle * (0.5 + 0.5 * (p.freq * 2.3 * tf32 + p.phase).sin());
            let a = (o * p.alpha * tw).clamp(0.0, 1.0);
            if a < 0.003 {
                continue;
            }
            match pl.kind {
                ParticleKind::Bokeh | ParticleKind::Dust => {
                    let sprite = &pl.sprites[p.color];
                    let k = p.size / sprite.width() as f32;
                    let tf = Transform::from_translate(x - p.size / 2.0, y - p.size / 2.0)
                        .pre_scale(k, k);
                    let paint = PixmapPaint {
                        opacity: a,
                        quality: FilterQuality::Bilinear,
                        ..PixmapPaint::default()
                    };
                    canvas.draw_pixmap(0, 0, sprite.as_ref(), &paint, tf, None);
                }
                ParticleKind::Confetti => {
                    let (w, h) = (p.size, p.size * 0.55);
                    let angle = p.phase.to_degrees() + p.spin * 40.0 * tf32;
                    let flip = (p.freq * 3.0 * tf32 + p.phase).cos();
                    let flip = if flip.abs() < 0.12 {
                        0.12f32.copysign(flip)
                    } else {
                        flip
                    };
                    let tf = Transform::from_translate(x, y)
                        .pre_rotate(angle)
                        .pre_scale(1.0, flip);
                    if let Some(r) = Rect::from_xywh(-w / 2.0, -h / 2.0, w, h) {
                        let shade = 0.75 + 0.25 * flip.abs();
                        let c = pl.colors[p.color];
                        let c = Rgba {
                            r: c.r * shade,
                            g: c.g * shade,
                            b: c.b * shade,
                            a: c.a * a,
                        };
                        canvas.fill_rect(r, &solid(c), tf, None);
                    }
                }
            }
        }
    }

    fn draw_noise_band(&self, canvas: &mut PixmapMut<'_>, n: &NoiseBandSpec, t: f64, o: f32) {
        let (w, h) = (self.w as f32, self.h as f32);
        let cy = n.y.at(t) * h;
        let half = n.height * h / 2.0;
        if half < 0.5 || cy + half < 0.0 || cy - half > h {
            return;
        }
        let frame = (t * self.fps as f64).round() as u64;
        let mut rng = Rng::new(hash2(n.seed, frame));
        let unit = (self.s / 1080.0).max(0.5);
        // A faint wash makes the band read as a tracking error, not just lines.
        if let Some(r) = Rect::from_xywh(0.0, cy - half * 0.6, w, half * 1.2) {
            let wash = Rgba {
                r: 1.0,
                g: 1.0,
                b: 1.0,
                a: 0.05 * n.intensity * o,
            };
            canvas.fill_rect(r, &solid(wash), Transform::identity(), None);
        }
        let count = ((half * 2.0) / (2.0 * unit)).clamp(10.0, 140.0) as usize;
        for _ in 0..count {
            let v = rng.f32() + rng.f32() - 1.0;
            let y = cy + v * half;
            let profile = 1.0 - v.abs();
            let len = rng.range(0.02, 0.45) * w;
            let x = rng.range(-0.1, 1.0) * w;
            let th = (rng.range(1.0, 3.0) * unit).max(1.0);
            let a = rng.range(0.25, 1.0) * n.intensity * profile * o;
            let dark = rng.f32() < 0.3;
            let c = if dark {
                Rgba {
                    r: 0.0,
                    g: 0.0,
                    b: 0.0,
                    a: a * 0.8,
                }
            } else {
                Rgba {
                    r: 1.0,
                    g: 1.0,
                    b: 1.0,
                    a,
                }
            };
            if let Some(r) = Rect::from_xywh(x, y, len, th) {
                let mut paint = solid(c);
                paint.anti_alias = false;
                canvas.fill_rect(r, &paint, Transform::identity(), None);
            }
        }
    }
}
