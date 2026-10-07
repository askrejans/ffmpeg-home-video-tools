//! Fonts, fallback, shaping, line fitting and glyph outlines.
//!
//! Everything here runs once, while a renderer is built. Text is split into
//! runs by font coverage (layer font, then its fallbacks, then the template's
//! fallbacks, then any other template font) so that bundled fonts are always
//! preferred; only characters no template font covers are handed to
//! cosmic-text's system font fallback.

use cosmic_text::skrifa::instance::{LocationRef, Size};
use cosmic_text::skrifa::outline::{DrawSettings, OutlinePen};
use cosmic_text::skrifa::raw::TableProvider;
use cosmic_text::skrifa::{self, GlyphId, MetadataProvider, Tag};
use cosmic_text::{
    Attrs, Buffer, CacheKey, CacheKeyFlags, Family, FontSystem, Metrics, Shaping, SwashCache,
    SwashContent, Wrap, fontdb,
};
use std::collections::{BTreeSet, HashMap};
use std::sync::{Arc, OnceLock};
use tiny_skia::{Path, PathBuilder, Pixmap, Rect, Transform};

/// Raw font file bytes shared between templates and renderers.
pub(crate) type FontBlob = Arc<dyn AsRef<[u8]> + Send + Sync>;

/// A concrete face picked for a family/weight/style request.
#[derive(Debug, Clone)]
pub(crate) struct Face {
    pub id: fontdb::ID,
    pub family: String,
    pub weight: fontdb::Weight,
    pub style: fontdb::Style,
    pub stretch: fontdb::Stretch,
}

impl Face {
    fn attrs(&self) -> Attrs<'_> {
        Attrs::new()
            .family(Family::Name(&self.family))
            .weight(self.weight)
            .style(self.style)
            .stretch(self.stretch)
    }
}

/// Family names declared by a set of font files.
pub(crate) fn families_in(blobs: &[FontBlob]) -> BTreeSet<String> {
    let mut db = fontdb::Database::new();
    for b in blobs {
        db.load_font_source(fontdb::Source::Binary(b.clone()));
    }
    db.faces()
        .flat_map(|f| f.families.iter().map(|(n, _)| n.clone()))
        .collect()
}

/// Number of faces that parse in the given blob.
pub(crate) fn face_count(blob: &FontBlob) -> usize {
    let mut db = fontdb::Database::new();
    db.load_font_source(fontdb::Source::Binary(blob.clone()));
    db.len()
}

fn system_db() -> &'static fontdb::Database {
    static DB: OnceLock<fontdb::Database> = OnceLock::new();
    DB.get_or_init(|| {
        let mut db = fontdb::Database::new();
        db.load_system_fonts();
        db
    })
}

pub(crate) struct Fonts {
    fs: FontSystem,
    template_faces: Vec<fontdb::ID>,
    swash: SwashCache,
    pub(crate) uses_system: bool,
}

/// One glyph after shaping, relative to the line origin (baseline, y down).
#[derive(Debug, Clone)]
pub(crate) struct ShapedGlyph {
    pub font_id: fontdb::ID,
    /// Font size in pixels (fallback runs may differ from the layer size).
    pub size: f32,
    pub weight: fontdb::Weight,
    pub flags: CacheKeyFlags,
    pub glyph_id: u16,
    pub x: f32,
    pub y: f32,
    pub advance: f32,
    /// Index of the grapheme cluster within the line (visual order).
    pub cluster: usize,
}

#[derive(Debug, Clone)]
pub(crate) struct ShapedLine {
    pub glyphs: Vec<ShapedGlyph>,
    pub width: f32,
    pub clusters: usize,
}

/// Rendered form of one glyph at its final size, origin on the baseline.
#[derive(Debug, Clone)]
pub(crate) enum GlyphShape {
    Path(Path),
    /// Colour glyph (emoji) as a premultiplied sprite; `left`/`top` place it
    /// relative to the glyph origin.
    Image {
        pixmap: Pixmap,
        left: f32,
        top: f32,
    },
    Empty,
}

impl GlyphShape {
    pub(crate) fn bounds(&self) -> Option<Rect> {
        match self {
            GlyphShape::Path(p) => Some(p.bounds()),
            GlyphShape::Image { pixmap, left, top } => {
                Rect::from_xywh(*left, -*top, pixmap.width() as f32, pixmap.height() as f32)
            }
            GlyphShape::Empty => None,
        }
    }
}

struct PathPen(PathBuilder);

impl OutlinePen for PathPen {
    fn move_to(&mut self, x: f32, y: f32) {
        self.0.move_to(x, -y);
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.0.line_to(x, -y);
    }
    fn quad_to(&mut self, cx0: f32, cy0: f32, x: f32, y: f32) {
        self.0.quad_to(cx0, -cy0, x, -y);
    }
    fn curve_to(&mut self, cx0: f32, cy0: f32, cx1: f32, cy1: f32, x: f32, y: f32) {
        self.0.cubic_to(cx0, -cy0, cx1, -cy1, x, -y);
    }
    fn close(&mut self) {
        self.0.close();
    }
}

/// Characters that must stay in the same font run as the preceding character.
fn attaches_to_previous(c: char) -> bool {
    matches!(c as u32,
        0x0300..=0x036F | 0x0483..=0x0489 | 0x1AB0..=0x1AFF | 0x1DC0..=0x1DFF |
        0x20D0..=0x20FF | 0xFE20..=0xFE2F | 0x200C | 0x200D | 0xFE00..=0xFE0F |
        0x1F3FB..=0x1F3FF | 0xE0020..=0xE007F | 0xE0100..=0xE01EF)
}

fn face_weight(
    db: &fontdb::Database,
    id: fontdb::ID,
    requested: u16,
    default: fontdb::Weight,
) -> fontdb::Weight {
    let range = db
        .with_face_data(id, |data, index| {
            let font = skrifa::FontRef::from_index(data, index).ok()?;
            let axis = font.axes().get_by_tag(Tag::new(b"wght"))?;
            Some((axis.min_value(), axis.max_value()))
        })
        .flatten();
    match range {
        Some((lo, hi)) => fontdb::Weight((requested as f32).clamp(lo, hi).round() as u16),
        None => default,
    }
}

impl Fonts {
    /// Build a font system from template fonts, optionally with the
    /// installed system fonts as a last-resort fallback.
    pub(crate) fn new(blobs: &[FontBlob], with_system: bool) -> Fonts {
        let mut db = if with_system {
            let mut db = system_db().clone();
            // Template fonts win over installed fonts with the same family name.
            let ours = families_in(blobs);
            let clashing: Vec<fontdb::ID> = db
                .faces()
                .filter(|f| f.families.iter().any(|(n, _)| ours.contains(n)))
                .map(|f| f.id)
                .collect();
            for id in clashing {
                db.remove_face(id);
            }
            db
        } else {
            fontdb::Database::new()
        };
        let mut template_faces = Vec::new();
        for b in blobs {
            template_faces.extend(db.load_font_source(fontdb::Source::Binary(b.clone())));
        }
        Fonts {
            fs: FontSystem::new_with_locale_and_db("en-US".to_string(), db),
            template_faces,
            swash: SwashCache::new(),
            uses_system: with_system,
        }
    }

    /// Pick the closest face of `family` for the requested weight/style.
    pub(crate) fn resolve(&self, family: &str, weight: u16, italic: bool) -> Option<Face> {
        let db = self.fs.db();
        let style = if italic {
            fontdb::Style::Italic
        } else {
            fontdb::Style::Normal
        };
        let id = db.query(&fontdb::Query {
            families: &[fontdb::Family::Name(family)],
            weight: fontdb::Weight(weight),
            stretch: fontdb::Stretch::Normal,
            style,
        })?;
        let info = db.face(id)?;
        // fontdb falls back to any face when the family is unknown; reject that.
        if !info.families.iter().any(|(n, _)| n == family) {
            return None;
        }
        // A variable font can render the requested weight itself.
        let weight = face_weight(db, id, weight, info.weight);
        Some(Face {
            id,
            family: family.to_string(),
            weight,
            style: info.style,
            stretch: info.stretch,
        })
    }

    /// Template faces other than those in `chain`, in a stable order, used as
    /// a last bundled fallback before system fonts.
    pub(crate) fn other_template_faces(&self, chain: &[Face], weight: u16) -> Vec<Face> {
        let db = self.fs.db();
        let mut faces: Vec<Face> = self
            .template_faces
            .iter()
            .filter(|id| !chain.iter().any(|f| f.id == **id))
            .filter_map(|id| {
                let info = db.face(*id)?;
                Some(Face {
                    id: *id,
                    family: info.families.first()?.0.clone(),
                    weight: face_weight(db, *id, weight, info.weight),
                    style: info.style,
                    stretch: info.stretch,
                })
            })
            .collect();
        faces.sort_by(|a, b| {
            (a.style != fontdb::Style::Normal)
                .cmp(&(b.style != fontdb::Style::Normal))
                .then(
                    a.weight
                        .0
                        .abs_diff(weight)
                        .cmp(&b.weight.0.abs_diff(weight)),
                )
                .then(a.family.cmp(&b.family))
                .then(a.weight.0.cmp(&b.weight.0))
        });
        faces
    }

    fn covers(&self, id: fontdb::ID, c: char) -> bool {
        self.fs
            .db()
            .with_face_data(id, |data, index| {
                skrifa::FontRef::from_index(data, index)
                    .ok()
                    .and_then(|f| f.charmap().map(c))
                    .is_some()
            })
            .unwrap_or(false)
    }

    /// True when some template face covers every visible character of `text`.
    pub(crate) fn template_covers(&self, text: &str) -> bool {
        text.chars()
            .filter(|c| !c.is_whitespace() && !c.is_control() && !attaches_to_previous(*c))
            .all(|c| self.template_faces.iter().any(|id| self.covers(*id, c)))
    }

    /// Split `text` into runs, each assigned the first face in `chain` that
    /// covers it (`None`: no template face does; let system fallback try).
    fn split_runs(
        &self,
        text: &str,
        chain: &[Face],
    ) -> Vec<(std::ops::Range<usize>, Option<usize>)> {
        let mut runs: Vec<(std::ops::Range<usize>, Option<usize>)> = Vec::new();
        let mut prev_was_zwj = false;
        for (i, c) in text.char_indices() {
            let end = i + c.len_utf8();
            let follow = c.is_whitespace() || attaches_to_previous(c) || prev_was_zwj;
            prev_was_zwj = c == '\u{200D}';
            if follow && let Some(last) = runs.last_mut() {
                last.0.end = end;
                continue;
            }
            let face = chain.iter().position(|f| self.covers(f.id, c));
            match runs.last_mut() {
                Some(last) if last.1 == face => last.0.end = end,
                _ => runs.push((i..end, face)),
            }
        }
        runs
    }

    /// Shape one line of text at `px` pixels. Runs set in a fallback face
    /// are scaled so their cap height matches the primary face.
    pub(crate) fn shape_line(&mut self, text: &str, chain: &[Face], px: f32) -> ShapedLine {
        let runs = self.split_runs(text, chain);
        let mut buffer = Buffer::new_empty(Metrics::new(px, px * 1.2));
        buffer.set_wrap(Wrap::None);
        buffer.set_size(None, None);
        let primary_cap = self.cap_height(&chain[0], 1.0);
        let spans: Vec<(&str, Attrs<'_>)> = runs
            .iter()
            .map(|(range, face)| {
                let face_index = face.unwrap_or(0);
                let mut attrs = chain[face_index].attrs();
                if face_index > 0 {
                    let ratio =
                        (primary_cap / self.cap_height(&chain[face_index], 1.0)).clamp(0.7, 1.3);
                    if (ratio - 1.0).abs() > 0.01 {
                        attrs = attrs.metrics(Metrics::new(px * ratio, px * 1.2));
                    }
                }
                (&text[range.clone()], attrs)
            })
            .collect();
        buffer.set_rich_text(spans, &chain[0].attrs(), Shaping::Advanced, None);
        buffer.shape_until_scroll(&mut self.fs, false);

        let mut glyphs = Vec::new();
        let mut width: f32 = 0.0;
        let mut clusters = 0usize;
        let mut last_start = usize::MAX;
        for run in buffer.layout_runs() {
            width = width.max(run.line_w);
            for g in run.glyphs {
                if g.start != last_start {
                    if last_start != usize::MAX {
                        clusters += 1;
                    }
                    last_start = g.start;
                }
                glyphs.push(ShapedGlyph {
                    font_id: g.font_id,
                    size: g.font_size,
                    weight: g.font_weight,
                    flags: g.cache_key_flags,
                    glyph_id: g.glyph_id,
                    x: g.x + g.font_size * g.x_offset,
                    y: g.y - g.font_size * g.y_offset,
                    advance: g.w,
                    cluster: clusters,
                });
            }
        }
        let clusters = if glyphs.is_empty() { 0 } else { clusters + 1 };
        ShapedLine {
            glyphs,
            width,
            clusters,
        }
    }

    /// Cap height of `face` at `px`, falling back to 70% of the size.
    pub(crate) fn cap_height(&self, face: &Face, px: f32) -> f32 {
        self.fs
            .db()
            .with_face_data(face.id, |data, index| {
                let font = skrifa::FontRef::from_index(data, index).ok()?;
                let loc = font
                    .axes()
                    .location([(Tag::new(b"wght"), face.weight.0 as f32)]);
                font.metrics(Size::new(px), &loc).cap_height
            })
            .flatten()
            .filter(|c| *c > 0.0)
            .unwrap_or(px * 0.7)
    }

    /// Vector outline (or colour sprite) of a shaped glyph at its size.
    pub(crate) fn glyph_shape(&mut self, g: &ShapedGlyph) -> GlyphShape {
        let px = g.size;
        let Some(font) = self.fs.get_font(g.font_id, g.weight) else {
            return GlyphShape::Empty;
        };
        let index = self.fs.db().face(g.font_id).map(|f| f.index).unwrap_or(0);
        let Ok(font_ref) = skrifa::FontRef::from_index(font.data(), index) else {
            return GlyphShape::Empty;
        };
        let colour = font_ref.colr().is_ok()
            || font_ref.data_for_tag(Tag::new(b"sbix")).is_some()
            || font_ref.data_for_tag(Tag::new(b"CBDT")).is_some();
        if colour && let Some(sprite) = self.colour_sprite(g, px) {
            return sprite;
        }
        let loc = font_ref
            .axes()
            .location([(Tag::new(b"wght"), g.weight.0 as f32)]);
        let Some(outline) = font_ref
            .outline_glyphs()
            .get(GlyphId::new(g.glyph_id as u32))
        else {
            return GlyphShape::Empty;
        };
        let mut pen = PathPen(PathBuilder::new());
        let settings = DrawSettings::unhinted(Size::new(px), LocationRef::from(&loc));
        if outline.draw(settings, &mut pen).is_err() {
            return GlyphShape::Empty;
        }
        let Some(mut path) = pen.0.finish() else {
            return GlyphShape::Empty;
        };
        if g.flags.contains(CacheKeyFlags::FAKE_ITALIC) {
            let skew = Transform::from_row(1.0, 0.0, -(14f32.to_radians().tan()), 1.0, 0.0, 0.0);
            match path.clone().transform(skew) {
                Some(p) => path = p,
                None => return GlyphShape::Empty,
            }
        }
        GlyphShape::Path(path)
    }

    fn colour_sprite(&mut self, g: &ShapedGlyph, px: f32) -> Option<GlyphShape> {
        let (key, _, _) = CacheKey::new(g.font_id, g.glyph_id, px, (0.0, 0.0), g.weight, g.flags);
        let image = self.swash.get_image_uncached(&mut self.fs, key)?;
        if !matches!(image.content, SwashContent::Color) {
            return None;
        }
        let (w, h) = (image.placement.width, image.placement.height);
        let mut pixmap = Pixmap::new(w, h)?;
        for (dst, src) in pixmap
            .data_mut()
            .as_chunks_mut::<4>()
            .0
            .iter_mut()
            .zip(image.data.as_chunks::<4>().0)
        {
            let a = src[3] as u16;
            dst[0] = ((src[0] as u16 * a + 127) / 255) as u8;
            dst[1] = ((src[1] as u16 * a + 127) / 255) as u8;
            dst[2] = ((src[2] as u16 * a + 127) / 255) as u8;
            dst[3] = src[3];
        }
        Some(GlyphShape::Image {
            pixmap,
            left: image.placement.left as f32,
            top: image.placement.top as f32,
        })
    }
}

#[cfg(test)]
mod variable_font_tests {
    use super::*;

    #[test]
    #[ignore = "set VIDEO_PROCESSOR_TEST_VARIABLE_FONT to a variable font with a wght axis"]
    fn variable_bundled_fallback_uses_requested_weight() {
        let path = std::env::var("VIDEO_PROCESSOR_TEST_VARIABLE_FONT").unwrap();
        let blob: FontBlob = Arc::new(std::fs::read(path).unwrap());
        let fonts = Fonts::new(&[blob], false);
        let fallback = fonts.other_template_faces(&[], 600);
        assert!(!fallback.is_empty());
        assert!(
            fallback
                .iter()
                .all(|face| face.weight == fontdb::Weight(600))
        );
        for face in &fallback {
            let primary = fonts.resolve(&face.family, 600, false).unwrap();
            assert_eq!(primary.weight, face.weight);
        }
    }
}

/// Result of fitting text into the available width.
#[derive(Debug, Clone)]
pub(crate) struct Fitted {
    pub px: f32,
    pub lines: Vec<String>,
}

/// Choose line breaks and font size so `text` fits `max_width` pixels.
///
/// Order of preference: one line at full size; one line shrunk by at most
/// 15%; balanced wrapping into up to `max_lines` lines at full size; the best
/// wrapping shrunk to fit.
pub(crate) fn fit_text(
    fonts: &mut Fonts,
    text: &str,
    chain: &[Face],
    px: f32,
    max_width: f32,
    max_lines: usize,
    tracking_em: f32,
) -> Fitted {
    let mut cache: HashMap<String, f32> = HashMap::new();
    let mut measure = |fonts: &mut Fonts, s: &str| -> f32 {
        if let Some(w) = cache.get(s) {
            return *w;
        }
        let line = fonts.shape_line(s, chain, px);
        let w = line.width + tracking_em * px * line.clusters.saturating_sub(1) as f32;
        cache.insert(s.to_string(), w);
        w
    };
    let single = measure(fonts, text);
    if single <= max_width {
        return Fitted {
            px,
            lines: vec![text.to_string()],
        };
    }
    let shrink_single = Fitted {
        px: px * max_width / single,
        lines: vec![text.to_string()],
    };
    let breaks: Vec<usize> = text
        .char_indices()
        .filter(|(_, c)| *c == ' ')
        .map(|(i, _)| i)
        .collect();
    if max_lines < 2 || breaks.is_empty() || single <= max_width / 0.85 {
        return shrink_single;
    }

    let split = |cuts: &[usize]| -> Vec<String> {
        let mut out = Vec::new();
        let mut from = 0;
        for &c in cuts {
            out.push(text[from..c].trim().to_string());
            from = c + 1;
        }
        out.push(text[from..].trim().to_string());
        out
    };
    // Candidates: (widest line, lines). Balanced breaks minimise the widest.
    let mut candidates: Vec<(f32, Vec<String>)> = Vec::new();
    for (i, &a) in breaks.iter().enumerate() {
        let mut cuts = vec![vec![a]];
        if max_lines >= 3 {
            cuts.extend(breaks[i + 1..].iter().map(|&b| vec![a, b]));
        }
        for c in cuts {
            let lines = split(&c);
            if lines.iter().any(|l| l.is_empty()) {
                continue;
            }
            let widest = lines.iter().map(|l| measure(fonts, l)).fold(0.0, f32::max);
            candidates.push((widest, lines));
        }
    }
    // Fewest lines that fit at full size, else the largest possible size.
    let fitting = candidates
        .iter()
        .filter(|(w, _)| *w <= max_width)
        .min_by(|a, b| a.1.len().cmp(&b.1.len()).then(a.0.total_cmp(&b.0)));
    if let Some((_, lines)) = fitting {
        return Fitted {
            px,
            lines: lines.clone(),
        };
    }
    match candidates
        .into_iter()
        .min_by(|a, b| a.0.total_cmp(&b.0).then(a.1.len().cmp(&b.1.len())))
    {
        Some((widest, lines)) if widest < single => Fitted {
            px: px * max_width / widest,
            lines,
        },
        _ => shrink_single,
    }
}
