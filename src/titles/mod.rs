//! Animated title intros.
//!
//! A template is a directory with `template.json` (layout, layers, keyframes)
//! and the fonts it uses. [`TitleRenderer`] turns a template plus the user's
//! field values into premultiplied RGBA overlay frames at any canvas size.
//!
//! INTERFACE CONTRACT — the renderer is driven by `render::intro`; keep these
//! signatures stable.

use crate::error::{Error, Result};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

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
    _private: (),
}

impl TitleTemplate {
    /// Load a template from a directory containing `template.json`.
    pub fn load(_dir: &Path) -> Result<Self> {
        Err(Error::Template(
            "title templates are not implemented yet".into(),
        ))
    }

    /// Load one of the templates bundled with the library.
    pub fn builtin(name: &str) -> Result<Self> {
        Err(Error::Template(format!(
            "unknown built-in template {name:?}"
        )))
    }

    /// Names accepted by [`TitleTemplate::builtin`].
    pub fn builtin_names() -> &'static [&'static str] {
        &[]
    }

    pub fn name(&self) -> &str {
        ""
    }

    /// Intro length in seconds.
    pub fn duration(&self) -> f64 {
        0.0
    }

    pub fn background(&self) -> TitleBackground {
        TitleBackground::Solid { r: 0, g: 0, b: 0 }
    }

    /// Optional audio file played under the intro.
    pub fn sound(&self) -> Option<PathBuf> {
        None
    }

    pub fn fields(&self) -> Vec<FieldSpec> {
        Vec::new()
    }
}

/// Renders overlay frames for one template, field set and canvas size.
///
/// Construction does all text shaping and glyph rasterisation; rendering a
/// frame only draws, so the renderer is `Send + Sync` and frames can be
/// rendered in parallel.
pub struct TitleRenderer {
    _private: (),
}

impl TitleRenderer {
    pub fn new(
        _template: &TitleTemplate,
        _fields: &BTreeMap<String, String>,
        _width: u32,
        _height: u32,
        _fps: u32,
    ) -> Result<Self> {
        Err(Error::Template(
            "title templates are not implemented yet".into(),
        ))
    }

    /// Number of frames in the intro: `round(duration * fps)`, at least 1.
    pub fn frame_count(&self) -> u64 {
        0
    }

    /// Render frame `index` as premultiplied RGBA8 into `out`
    /// (`width * height * 4` bytes, row-major). Transparent pixels let the
    /// background show through.
    pub fn render_frame(&self, _index: u64, _out: &mut [u8]) {}
}
