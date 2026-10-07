//! Animated title intros.
//!
//! A template is a directory with `template.json` (layout, layers, keyframes)
//! and the fonts it uses. [`TitleRenderer`] turns a template plus the user's
//! field values into premultiplied RGBA overlay frames at any canvas size.
//! The template format is documented in `docs/templates.md`.
//!
//! INTERFACE CONTRACT — the renderer is driven by `render::intro`; keep these
//! signatures stable.

mod anim;
mod color;
mod draw;
mod effects;
mod scene;
mod spec;
mod text;

#[cfg(test)]
mod tests;

use crate::error::{Error, Result};
use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use text::FontBlob;

/// What sits behind the title overlay.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TitleBackground {
    /// Blurred, dimmed footage of the first clip. `dim` is 0..1 (fraction of
    /// brightness removed); `blur` scales the blur strength (1.0 = default).
    Footage { dim: f32, blur: f32 },
    /// A flat colour (sRGB).
    Solid { r: u8, g: u8, b: u8 },
}

/// A user-fillable text field declared by a template.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldSpec {
    pub name: String,
    pub max_chars: usize,
    pub required: bool,
}

/// A parsed template with its fonts loaded.
#[derive(Debug, Clone)]
pub struct TitleTemplate {
    inner: Arc<TemplateData>,
}

struct TemplateData {
    spec: spec::TemplateSpec,
    fonts: Vec<FontBlob>,
    sound: Option<PathBuf>,
    source: String,
}

impl fmt::Debug for TemplateData {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let bytes: usize = self.fonts.iter().map(|b| (**b).as_ref().len()).sum();
        f.debug_struct("TitleTemplate")
            .field("name", &self.spec.name)
            .field("source", &self.source)
            .field("duration", &self.spec.duration)
            .field(
                "fonts",
                &format_args!("{} files, {} bytes", self.fonts.len(), bytes),
            )
            .field("sound", &self.sound)
            .finish_non_exhaustive()
    }
}

fn is_font_file(p: &Path) -> bool {
    p.extension().and_then(|e| e.to_str()).is_some_and(|e| {
        matches!(
            e.to_ascii_lowercase().as_str(),
            "ttf" | "otf" | "ttc" | "otc"
        )
    })
}

fn template_err(context: &Path, msg: impl fmt::Display) -> Error {
    Error::Template(format!("{}: {msg}", context.display()))
}

/// Resolve a path declared in `template.json` relative to the template.
fn relative(dir: &Path, json: &Path, rel: &str, what: &str) -> Result<PathBuf> {
    let p = Path::new(rel);
    if p.is_absolute() {
        return Err(template_err(
            json,
            format!("{what} path {rel:?} must be relative to the template"),
        ));
    }
    Ok(dir.join(p))
}

impl TitleTemplate {
    /// Load a template from a directory containing `template.json`.
    ///
    /// Fonts are read from the `fonts/` subdirectory plus any files or
    /// directories listed under `"fonts"`; an optional `"sound"` file is
    /// resolved relative to the directory and must exist.
    pub fn load(dir: &Path) -> Result<Self> {
        let json_path = dir.join("template.json");
        let json = std::fs::read_to_string(&json_path)
            .map_err(|e| template_err(&json_path, format!("cannot read template: {e}")))?;
        // Tolerate a byte-order mark left by some editors.
        let json = json.strip_prefix('\u{feff}').unwrap_or(&json);
        let spec = spec::parse_template(json).map_err(|e| template_err(&json_path, e))?;

        let mut font_paths: Vec<PathBuf> = Vec::new();
        let add_dir = |d: &Path, out: &mut Vec<PathBuf>| -> Result<()> {
            let entries = std::fs::read_dir(d)
                .map_err(|e| template_err(d, format!("cannot read font directory: {e}")))?;
            let mut found: Vec<PathBuf> = entries
                .filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| p.is_file() && is_font_file(p))
                .collect();
            found.sort();
            out.extend(found);
            Ok(())
        };
        let default_dir = dir.join("fonts");
        if default_dir.is_dir() {
            add_dir(&default_dir, &mut font_paths)?;
        }
        for rel in &spec.fonts {
            let p = relative(dir, &json_path, rel, "font")?;
            if p.is_dir() {
                add_dir(&p, &mut font_paths)?;
            } else if p.is_file() {
                font_paths.push(p);
            } else {
                return Err(template_err(
                    &json_path,
                    format!("font path {rel:?} does not exist"),
                ));
            }
        }
        let mut seen = std::collections::BTreeSet::new();
        let mut fonts: Vec<FontBlob> = Vec::new();
        for p in font_paths {
            let key = p.canonicalize().unwrap_or_else(|_| p.clone());
            if !seen.insert(key) {
                continue;
            }
            let bytes = std::fs::read(&p)
                .map_err(|e| Error::io(format!("reading font {}", p.display()), e))?;
            let blob: FontBlob = Arc::new(bytes);
            if text::face_count(&blob) == 0 {
                return Err(template_err(&p, "not a usable TrueType/OpenType font"));
            }
            fonts.push(blob);
        }

        let sound = match &spec.sound {
            Some(rel) => {
                let p = relative(dir, &json_path, rel, "sound")?;
                if !p.is_file() {
                    return Err(template_err(
                        &json_path,
                        format!("sound file {rel:?} does not exist"),
                    ));
                }
                Some(
                    p.canonicalize()
                        .map_err(|e| Error::io(format!("resolving {}", p.display()), e))?,
                )
            }
            None => None,
        };
        Self::finish(spec, fonts, sound, dir.display().to_string())
    }

    fn finish(
        spec: spec::TemplateSpec,
        fonts: Vec<FontBlob>,
        sound: Option<PathBuf>,
        source: String,
    ) -> Result<Self> {
        let families = text::families_in(&fonts);
        let ctx = format!("template {:?}", spec.name);
        let check = |fam: &str, what: &str| -> Result<()> {
            if families.contains(fam) {
                return Ok(());
            }
            let available = if families.is_empty() {
                "none — add .ttf/.otf files to the fonts/ directory".to_string()
            } else {
                families
                    .iter()
                    .map(|f| format!("{f:?}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            };
            Err(Error::Template(format!(
                "{ctx}: {what} uses font family {fam:?}, which is not among the template fonts \
                 (available: {available})"
            )))
        };
        for fam in &spec.fallback_fonts {
            check(fam, "\"fallback_fonts\"")?;
        }
        for layout in [&spec.landscape, &spec.portrait] {
            for (i, layer) in layout.layers.iter().enumerate() {
                if let spec::LayerKind::Text(t, _) = &layer.kind {
                    let what = format!("layer {i}");
                    check(&t.font, &what)?;
                    for fam in &t.fallback {
                        check(fam, &what)?;
                    }
                }
            }
        }
        Ok(TitleTemplate {
            inner: Arc::new(TemplateData {
                spec,
                fonts,
                sound,
                source,
            }),
        })
    }

    /// Load one of the templates bundled with the library.
    pub fn builtin(name: &str) -> Result<Self> {
        #[cfg(feature = "builtin-templates")]
        {
            let json = builtin::json(name).ok_or_else(|| {
                Error::Template(format!(
                    "unknown built-in template {name:?} (available: {})",
                    builtin::NAMES.join(", ")
                ))
            })?;
            let spec = spec::parse_template(json)
                .map_err(|e| Error::Template(format!("built-in template {name:?}: {e}")))?;
            let fonts = builtin::FONTS
                .iter()
                .map(|bytes| Arc::new(*bytes) as FontBlob)
                .collect();
            Self::finish(spec, fonts, None, format!("built-in:{name}"))
        }
        #[cfg(not(feature = "builtin-templates"))]
        {
            Err(Error::Template(format!(
                "built-in template {name:?} is not available: this build has the \
                 \"builtin-templates\" feature disabled"
            )))
        }
    }

    /// Names accepted by [`TitleTemplate::builtin`].
    pub fn builtin_names() -> &'static [&'static str] {
        #[cfg(feature = "builtin-templates")]
        {
            builtin::NAMES
        }
        #[cfg(not(feature = "builtin-templates"))]
        {
            &[]
        }
    }

    pub fn name(&self) -> &str {
        &self.inner.spec.name
    }

    /// Intro length in seconds.
    pub fn duration(&self) -> f64 {
        self.inner.spec.duration
    }

    pub fn background(&self) -> TitleBackground {
        self.inner.spec.background
    }

    /// Optional audio file played under the intro (absolute path; only disk
    /// templates that declare `"sound"` have one).
    pub fn sound(&self) -> Option<PathBuf> {
        self.inner.sound.clone()
    }

    pub fn fields(&self) -> Vec<FieldSpec> {
        self.inner.spec.fields.clone()
    }
}

/// Renders overlay frames for one template, field set and canvas size.
///
/// Construction does all text shaping and glyph rasterisation; rendering a
/// frame only draws, so the renderer is `Send + Sync` and frames can be
/// rendered in parallel.
pub struct TitleRenderer {
    scene: scene::Scene,
}

const _: () = {
    const fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<TitleRenderer>();
    assert_send_sync::<TitleTemplate>();
};

impl TitleRenderer {
    /// Prepare a template for a `width` x `height` canvas at `fps`.
    ///
    /// Portrait layout (the template's `portrait` overrides) is used when
    /// `height > width`. Unknown keys in `fields` are ignored; a required
    /// field that is missing or blank is an [`Error::Template`].
    pub fn new(
        template: &TitleTemplate,
        fields: &BTreeMap<String, String>,
        width: u32,
        height: u32,
        fps: u32,
    ) -> Result<Self> {
        let scene = scene::build_scene(
            &template.inner.spec,
            &template.inner.fonts,
            fields,
            width,
            height,
            fps,
        )?;
        Ok(TitleRenderer { scene })
    }

    /// Named premultiplied RGBA images required by this scene.
    pub fn image_slots(&self) -> Vec<String> {
        let mut slots: Vec<_> = self
            .scene
            .layers
            .iter()
            .filter_map(|layer| match &layer.kind {
                scene::Kind::Image(i) => Some(i.slot.clone()),
                _ => None,
            })
            .collect();
        slots.sort();
        slots.dedup();
        slots
    }
    /// Bind a tightly packed premultiplied RGBA image. Cover/contain fitting
    /// happens once; render_frame then shares the prepared texture safely.
    pub fn bind_image(&self, slot: &str, width: u32, height: u32, rgba: &[u8]) -> Result<()> {
        use tiny_skia::{FilterQuality, IntSize, Pixmap, PixmapPaint, Transform};
        if width == 0
            || height == 0
            || width > 16384
            || height > 16384
            || rgba.len() != width as usize * height as usize * 4
        {
            return Err(Error::Template("invalid image slot buffer".into()));
        }
        let source = Pixmap::from_vec(rgba.to_vec(), IntSize::from_wh(width, height).unwrap())
            .ok_or_else(|| Error::Template("invalid image".into()))?;
        let mut bound = false;
        for layer in &self.scene.layers {
            let scene::Kind::Image(image) = &layer.kind else {
                continue;
            };
            if image.slot != slot {
                continue;
            }
            let (w, h) = (
                image.w.round().max(1.0) as u32,
                image.h.round().max(1.0) as u32,
            );
            let mut target = Pixmap::new(w, h)
                .ok_or_else(|| Error::Template("image layer is too large".into()))?;
            let (sx, sy) = (w as f32 / width as f32, h as f32 / height as f32);
            let scale = match image.fit {
                spec::ImageFit::Cover => sx.max(sy),
                spec::ImageFit::Contain => sx.min(sy),
            };
            let transform = Transform::from_translate(
                (w as f32 - width as f32 * scale) / 2.0,
                (h as f32 - height as f32 * scale) / 2.0,
            )
            .pre_scale(scale, scale);
            target.draw_pixmap(
                0,
                0,
                source.as_ref(),
                &PixmapPaint {
                    quality: FilterQuality::Bicubic,
                    ..Default::default()
                },
                transform,
                None,
            );
            *image.texture.write().unwrap() = Some(target);
            bound = true;
        }
        if !bound {
            return Err(Error::Template(format!(
                "image slot {slot:?} is not declared"
            )));
        }
        Ok(())
    }

    /// Number of frames in the intro: `round(duration * fps)`, at least 1.
    pub fn frame_count(&self) -> u64 {
        self.scene.frames
    }

    /// Render frame `index` as premultiplied RGBA8 into `out`
    /// (`width * height * 4` bytes, row-major). Transparent pixels let the
    /// background show through. Indices past the end render the last frame.
    ///
    /// # Panics
    ///
    /// Panics if `out` is not exactly `width * height * 4` bytes long.
    pub fn render_frame(&self, index: u64, out: &mut [u8]) {
        self.scene.render(index, out);
    }
}

#[cfg(feature = "builtin-templates")]
mod builtin {
    pub(super) const NAMES: &[&str] = &["clean", "cinematic", "retro"];

    pub(super) fn json(name: &str) -> Option<&'static str> {
        Some(match name {
            "clean" => include_str!("../../templates/clean/template.json"),
            "cinematic" => include_str!("../../templates/cinematic/template.json"),
            "retro" => include_str!("../../templates/retro/template.json"),
            _ => return None,
        })
    }

    /// Fonts shared by the bundled templates (SIL Open Font License 1.1, see
    /// `templates/fonts/README.md`).
    pub(super) const FONTS: &[&[u8]] = &[
        include_bytes!("../../templates/fonts/IBMPlexSans-Light.ttf"),
        include_bytes!("../../templates/fonts/IBMPlexSans-Regular.ttf"),
        include_bytes!("../../templates/fonts/IBMPlexSans-SemiBold.ttf"),
        include_bytes!("../../templates/fonts/NotoSerifDisplay-Light.ttf"),
        include_bytes!("../../templates/fonts/Play-Regular.ttf"),
        include_bytes!("../../templates/fonts/Play-Bold.ttf"),
        include_bytes!("../../templates/fonts/VT323-Regular.ttf"),
    ];
}
