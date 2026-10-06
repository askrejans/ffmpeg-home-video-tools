//! The `template.json` format: parsing, portrait overrides and validation.
//!
//! See `docs/templates.md` for the user-facing description of every key.

use super::anim::{Anim, Easing, with_default_ease};
use super::color::Rgba;
use super::{FieldSpec, TitleBackground};
use serde::Deserialize;
use serde::de::{self, DeserializeOwned, Deserializer};
use serde_json::{Map, Value};
use std::collections::{BTreeMap, BTreeSet};

/// Highest `format` number this library understands.
pub(crate) const FORMAT_VERSION: u32 = 1;

/// A fully parsed template, with separate layer lists for landscape and
/// portrait canvases (portrait overrides already merged in).
#[derive(Debug, Clone)]
pub(crate) struct TemplateSpec {
    pub name: String,
    pub duration: f64,
    pub background: TitleBackground,
    pub sound: Option<String>,
    pub fonts: Vec<String>,
    pub fallback_fonts: Vec<String>,
    pub fields: Vec<FieldSpec>,
    pub landscape: Layout,
    pub portrait: Layout,
}

#[derive(Debug, Clone)]
pub(crate) struct Layout {
    pub safe_area: SafeArea,
    pub stacks: BTreeMap<String, StackSpec>,
    pub layers: Vec<LayerSpec>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawTemplate {
    #[serde(default)]
    format: Option<u32>,
    name: String,
    #[serde(default)]
    #[allow(dead_code)]
    description: Option<String>,
    duration: f64,
    background: Value,
    #[serde(default)]
    sound: Option<String>,
    #[serde(default)]
    fonts: Vec<String>,
    #[serde(default)]
    fallback_fonts: Vec<String>,
    #[serde(default)]
    safe_area: Option<Value>,
    #[serde(default)]
    stacks: Option<Value>,
    #[serde(default)]
    portrait: Option<Value>,
    fields: Vec<RawField>,
    layers: Vec<Value>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawField {
    name: String,
    max_chars: usize,
    #[serde(default)]
    required: bool,
    #[serde(default)]
    #[allow(dead_code)]
    label: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawGlobals {
    #[serde(default)]
    safe_area: Option<SafeArea>,
    #[serde(default)]
    stacks: BTreeMap<String, StackSpec>,
}

#[derive(Deserialize, Debug, Clone, Copy)]
#[serde(deny_unknown_fields)]
pub(crate) struct SafeArea {
    #[serde(default = "d_safe_x")]
    pub x: f32,
    #[serde(default = "d_safe_y")]
    pub y: f32,
}

impl Default for SafeArea {
    fn default() -> Self {
        SafeArea {
            x: d_safe_x(),
            y: d_safe_y(),
        }
    }
}

#[derive(Deserialize, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub(crate) struct StackSpec {
    #[serde(default = "half")]
    pub y: f32,
    #[serde(default)]
    pub valign: VAlign,
    #[serde(default)]
    pub frame: Frame,
}

#[derive(Deserialize, Debug, Clone, Copy, Default, PartialEq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Frame {
    #[default]
    Canvas,
    Safe,
}

#[derive(Deserialize, Debug, Clone, Copy, Default, PartialEq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum HAlign {
    Left,
    #[default]
    Center,
    Right,
}

impl HAlign {
    pub(crate) fn factor(self) -> f32 {
        match self {
            HAlign::Left => 0.0,
            HAlign::Center => 0.5,
            HAlign::Right => 1.0,
        }
    }
}

#[derive(Deserialize, Debug, Clone, Copy, Default, PartialEq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum VAlign {
    Top,
    #[default]
    Middle,
    Bottom,
    Baseline,
}

#[derive(Deserialize, Debug, Clone, Copy, Default, PartialEq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Direction {
    #[default]
    Left,
    Right,
    Top,
    Bottom,
    Center,
}

#[derive(Deserialize, Debug, Clone, Copy, Default, PartialEq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum StaggerOrder {
    #[default]
    Forward,
    Reverse,
    Center,
    Edges,
    Random,
}

#[derive(Deserialize, Debug, Clone, Copy, PartialEq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ParticleKind {
    Bokeh,
    Dust,
    Confetti,
}

#[derive(Deserialize, Debug, Clone, Copy, Default, PartialEq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum GradientKind {
    #[default]
    Linear,
    Radial,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum ShapeKind {
    Rect,
    Ellipse,
    Triangle,
}

/// Keys shared by every layer type.
#[derive(Deserialize, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub(crate) struct Common {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default = "yes")]
    pub enabled: bool,
    #[serde(default)]
    pub start: Option<f64>,
    #[serde(default)]
    pub end: Option<f64>,
    #[serde(default = "one_anim")]
    pub opacity: Anim<f32>,
    #[serde(default)]
    pub requires: Vec<String>,
}

const COMMON_KEYS: &[&str] = &["id", "enabled", "start", "end", "opacity", "requires"];

/// Position keys of text and shape layers.
#[derive(Deserialize, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub(crate) struct Placement {
    #[serde(default = "half_anim")]
    pub x: Anim<f32>,
    #[serde(default = "half_anim")]
    pub y: Anim<f32>,
    #[serde(default)]
    pub frame: Frame,
    #[serde(default)]
    pub stack: Option<String>,
    #[serde(default)]
    pub gap: f32,
    #[serde(default)]
    pub align: HAlign,
    #[serde(default)]
    pub valign: VAlign,
    #[serde(default = "zero_anim")]
    pub dx: Anim<f32>,
    #[serde(default = "zero_anim")]
    pub dy: Anim<f32>,
    #[serde(default = "one_anim")]
    pub scale: Anim<f32>,
    #[serde(default = "zero_anim")]
    pub rotation: Anim<f32>,
}

const PLACEMENT_KEYS: &[&str] = &[
    "x", "y", "frame", "stack", "gap", "align", "valign", "dx", "dy", "scale", "rotation",
];

#[derive(Debug, Clone)]
pub(crate) struct LayerSpec {
    pub common: Common,
    pub kind: LayerKind,
}

#[derive(Debug, Clone)]
pub(crate) enum LayerKind {
    Text(Box<TextSpec>, Placement),
    Shape(ShapeKind, ShapeSpec, Placement),
    Letterbox(LetterboxSpec),
    Fill(FillSpec),
    Vignette(VignetteSpec),
    Grain(GrainSpec),
    Scanlines(ScanlinesSpec),
    Sweep(SweepSpec),
    Particles(ParticlesSpec),
    NoiseBand(NoiseBandSpec),
}

#[derive(Deserialize, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub(crate) struct TextSpec {
    #[serde(default)]
    pub field: Option<String>,
    #[serde(default)]
    pub text: Option<String>,
    pub font: String,
    #[serde(default = "d_weight")]
    pub weight: u16,
    #[serde(default)]
    pub italic: bool,
    #[serde(default)]
    pub fallback: Vec<String>,
    #[serde(default = "d_text_size")]
    pub size: f32,
    #[serde(default = "white_anim")]
    pub color: Anim<Rgba>,
    #[serde(default = "one_f")]
    pub max_width: f32,
    #[serde(default = "one_u8")]
    pub max_lines: u8,
    #[serde(default = "d_line_height")]
    pub line_height: f32,
    #[serde(default)]
    pub uppercase: bool,
    #[serde(default = "zero_anim")]
    pub tracking: Anim<f32>,
    #[serde(default)]
    pub skew: f32,
    #[serde(default = "zero_anim")]
    pub blur: Anim<f32>,
    #[serde(default = "one_anim")]
    pub reveal: Anim<f32>,
    #[serde(default)]
    pub reveal_from: Direction,
    #[serde(default = "d_softness")]
    pub reveal_softness: f32,
    #[serde(default)]
    pub stagger: Option<Stagger>,
    #[serde(default)]
    pub glyph: Option<GlyphAnim>,
    #[serde(default)]
    pub shadow: Option<Shadow>,
    #[serde(default)]
    pub glow: Option<Glow>,
    #[serde(default)]
    pub chroma: Option<Chroma>,
    #[serde(default)]
    pub jitter: Option<Jitter>,
    #[serde(default)]
    pub sweep: Option<TextSweep>,
}

#[derive(Debug, Clone)]
pub(crate) struct Stagger {
    pub each: f32,
    pub order: StaggerOrder,
    pub seed: u64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawStagger {
    each: f32,
    #[serde(default)]
    order: StaggerOrder,
    #[serde(default)]
    seed: u64,
}

impl<'de> Deserialize<'de> for Stagger {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let v = Value::deserialize(d)?;
        if let Some(each) = v.as_f64() {
            return Ok(Stagger {
                each: each as f32,
                order: StaggerOrder::Forward,
                seed: 0,
            });
        }
        let raw: RawStagger = serde_json::from_value(v).map_err(de::Error::custom)?;
        Ok(Stagger {
            each: raw.each,
            order: raw.order,
            seed: raw.seed,
        })
    }
}

#[derive(Deserialize, Debug, Clone, Default)]
#[serde(deny_unknown_fields)]
pub(crate) struct GlyphAnim {
    #[serde(default)]
    pub opacity: Option<Anim<f32>>,
    #[serde(default)]
    pub dx: Option<Anim<f32>>,
    #[serde(default)]
    pub dy: Option<Anim<f32>>,
    #[serde(default)]
    pub scale: Option<Anim<f32>>,
    #[serde(default)]
    pub rotation: Option<Anim<f32>>,
}

#[derive(Deserialize, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub(crate) struct Shadow {
    #[serde(default = "black")]
    pub color: Rgba,
    #[serde(default = "d_shadow_opacity")]
    pub opacity: Anim<f32>,
    #[serde(default = "d_shadow_blur")]
    pub blur: f32,
    #[serde(default)]
    pub dx: f32,
    #[serde(default = "d_shadow_dy")]
    pub dy: f32,
}

#[derive(Deserialize, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub(crate) struct Glow {
    #[serde(default = "white")]
    pub color: Rgba,
    #[serde(default = "d_glow_opacity")]
    pub opacity: Anim<f32>,
    #[serde(default = "d_glow_radius")]
    pub radius: f32,
}

#[derive(Deserialize, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub(crate) struct Chroma {
    #[serde(default = "d_chroma_offset")]
    pub offset: Anim<f32>,
    #[serde(default = "d_chroma_opacity")]
    pub opacity: Anim<f32>,
    #[serde(default = "d_chroma_left")]
    pub left: Rgba,
    #[serde(default = "d_chroma_right")]
    pub right: Rgba,
}

#[derive(Deserialize, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub(crate) struct Jitter {
    #[serde(default = "d_jitter_amount")]
    pub amount: Anim<f32>,
    #[serde(default = "d_jitter_rate")]
    pub rate: f32,
    #[serde(default = "one_u64")]
    pub seed: u64,
}

#[derive(Deserialize, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub(crate) struct TextSweep {
    pub progress: Anim<f32>,
    #[serde(default = "d_sweep_angle")]
    pub angle: f32,
    #[serde(default = "d_text_sweep_width")]
    pub width: f32,
    #[serde(default = "white")]
    pub color: Rgba,
    #[serde(default = "d_text_sweep_opacity")]
    pub opacity: f32,
}

#[derive(Deserialize, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub(crate) struct ShapeSpec {
    #[serde(default)]
    pub width: Option<f32>,
    #[serde(default)]
    pub width_of: Option<String>,
    #[serde(default = "one_f")]
    pub width_factor: f32,
    #[serde(default)]
    pub height: Option<f32>,
    #[serde(default)]
    pub radius: f32,
    #[serde(default = "white_anim")]
    pub color: Anim<Rgba>,
    #[serde(default = "one_anim")]
    pub reveal: Anim<f32>,
    #[serde(default)]
    pub reveal_from: Direction,
}

#[derive(Deserialize, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub(crate) struct LetterboxSpec {
    #[serde(default)]
    pub size: Option<f32>,
    #[serde(default)]
    pub aspect: Option<f32>,
    #[serde(default = "black")]
    pub color: Rgba,
    #[serde(default = "one_anim")]
    pub progress: Anim<f32>,
}

#[derive(Deserialize, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub(crate) struct FillSpec {
    #[serde(default)]
    pub color: Option<Anim<Rgba>>,
    #[serde(default)]
    pub gradient: Option<GradientSpec>,
}

#[derive(Deserialize, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub(crate) struct GradientSpec {
    #[serde(default)]
    pub kind: GradientKind,
    #[serde(default = "d_gradient_angle")]
    pub angle: f32,
    #[serde(default = "d_center")]
    pub center: [f32; 2],
    #[serde(default = "d_gradient_radius")]
    pub radius: f32,
    pub stops: Vec<(f32, Rgba)>,
}

#[derive(Deserialize, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub(crate) struct VignetteSpec {
    #[serde(default = "half")]
    pub strength: f32,
    #[serde(default = "d_vignette_radius")]
    pub radius: f32,
    #[serde(default = "d_vignette_softness")]
    pub softness: f32,
    #[serde(default = "black")]
    pub color: Rgba,
}

#[derive(Deserialize, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub(crate) struct GrainSpec {
    #[serde(default = "d_grain_amount")]
    pub amount: f32,
    #[serde(default = "d_grain_size")]
    pub size: f32,
    #[serde(default = "d_grain_fps")]
    pub fps: f32,
    #[serde(default = "one_u64")]
    pub seed: u64,
}

#[derive(Deserialize, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub(crate) struct ScanlinesSpec {
    #[serde(default = "d_scan_spacing")]
    pub spacing: f32,
    #[serde(default = "d_scan_thickness")]
    pub thickness: f32,
    #[serde(default = "d_scan_strength")]
    pub strength: f32,
    #[serde(default = "black")]
    pub color: Rgba,
    #[serde(default)]
    pub speed: f32,
}

#[derive(Deserialize, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub(crate) struct SweepSpec {
    pub progress: Anim<f32>,
    #[serde(default = "d_sweep_angle")]
    pub angle: f32,
    #[serde(default = "d_sweep_width")]
    pub width: f32,
    #[serde(default = "white")]
    pub color: Rgba,
    #[serde(default = "d_sweep_intensity")]
    pub intensity: f32,
}

#[derive(Deserialize, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub(crate) struct ParticlesSpec {
    pub kind: ParticleKind,
    #[serde(default = "d_particle_count")]
    pub count: u32,
    #[serde(default = "one_u64")]
    pub seed: u64,
    #[serde(default)]
    pub size: Option<[f32; 2]>,
    #[serde(default = "d_particle_colors")]
    pub colors: Vec<Rgba>,
    #[serde(default)]
    pub velocity: Option<[f32; 2]>,
    #[serde(default = "d_particle_spread")]
    pub spread: f32,
    #[serde(default = "d_particle_softness")]
    pub softness: f32,
    #[serde(default = "d_particle_twinkle")]
    pub twinkle: f32,
    #[serde(default = "d_area")]
    pub area: [f32; 4],
}

#[derive(Deserialize, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub(crate) struct NoiseBandSpec {
    pub y: Anim<f32>,
    #[serde(default = "d_band_height")]
    pub height: f32,
    #[serde(default = "d_band_intensity")]
    pub intensity: f32,
    #[serde(default = "one_u64")]
    pub seed: u64,
}

fn yes() -> bool {
    true
}
fn half() -> f32 {
    0.5
}
fn one_f() -> f32 {
    1.0
}
fn one_u8() -> u8 {
    1
}
fn one_u64() -> u64 {
    1
}
fn d_safe_x() -> f32 {
    0.06
}
fn d_safe_y() -> f32 {
    0.08
}
fn d_weight() -> u16 {
    400
}
fn d_text_size() -> f32 {
    0.05
}
fn d_line_height() -> f32 {
    1.15
}
fn d_softness() -> f32 {
    0.15
}
fn white() -> Rgba {
    Rgba::WHITE
}
fn black() -> Rgba {
    Rgba::BLACK
}
fn white_anim() -> Anim<Rgba> {
    Anim::Const(Rgba::WHITE)
}
fn zero_anim() -> Anim<f32> {
    Anim::Const(0.0)
}
fn half_anim() -> Anim<f32> {
    Anim::Const(0.5)
}
fn one_anim() -> Anim<f32> {
    Anim::Const(1.0)
}
fn d_shadow_opacity() -> Anim<f32> {
    Anim::Const(0.5)
}
fn d_shadow_blur() -> f32 {
    0.008
}
fn d_shadow_dy() -> f32 {
    0.003
}
fn d_glow_opacity() -> Anim<f32> {
    Anim::Const(0.35)
}
fn d_glow_radius() -> f32 {
    0.02
}
fn d_chroma_offset() -> Anim<f32> {
    Anim::Const(0.003)
}
fn d_chroma_opacity() -> Anim<f32> {
    Anim::Const(0.6)
}
fn d_chroma_left() -> Rgba {
    Rgba {
        r: 1.0,
        g: 0.12,
        b: 0.3,
        a: 1.0,
    }
}
fn d_chroma_right() -> Rgba {
    Rgba {
        r: 0.12,
        g: 0.82,
        b: 1.0,
        a: 1.0,
    }
}
fn d_jitter_amount() -> Anim<f32> {
    Anim::Const(0.004)
}
fn d_jitter_rate() -> f32 {
    12.0
}
fn d_sweep_angle() -> f32 {
    20.0
}
fn d_text_sweep_width() -> f32 {
    0.12
}
fn d_text_sweep_opacity() -> f32 {
    0.9
}
fn d_gradient_angle() -> f32 {
    90.0
}
fn d_center() -> [f32; 2] {
    [0.5, 0.5]
}
fn d_gradient_radius() -> f32 {
    0.75
}
fn d_vignette_radius() -> f32 {
    0.35
}
fn d_vignette_softness() -> f32 {
    0.65
}
fn d_grain_amount() -> f32 {
    0.06
}
fn d_grain_size() -> f32 {
    0.0012
}
fn d_grain_fps() -> f32 {
    24.0
}
fn d_scan_spacing() -> f32 {
    0.004
}
fn d_scan_thickness() -> f32 {
    0.45
}
fn d_scan_strength() -> f32 {
    0.25
}
fn d_sweep_width() -> f32 {
    0.4
}
fn d_sweep_intensity() -> f32 {
    0.15
}
fn d_particle_count() -> u32 {
    30
}
fn d_particle_colors() -> Vec<Rgba> {
    vec![Rgba::WHITE]
}
fn d_particle_spread() -> f32 {
    0.01
}
fn d_particle_softness() -> f32 {
    0.6
}
fn d_particle_twinkle() -> f32 {
    0.3
}
fn d_area() -> [f32; 4] {
    [0.0, 0.0, 1.0, 1.0]
}
fn d_band_height() -> f32 {
    0.06
}
fn d_band_intensity() -> f32 {
    0.35
}

/// Merge `over` into `base`: objects merge key by key, everything else is
/// replaced.
pub(crate) fn deep_merge(base: &mut Value, over: &Value) {
    match (base, over) {
        (Value::Object(b), Value::Object(o)) => {
            for (k, v) in o {
                match b.get_mut(k) {
                    Some(bv) if bv.is_object() && v.is_object() => deep_merge(bv, v),
                    _ => {
                        b.insert(k.clone(), v.clone());
                    }
                }
            }
        }
        (b, o) => *b = o.clone(),
    }
}

/// Move `keys` out of `obj` into a new object.
fn take_keys(obj: &mut Map<String, Value>, keys: &[&str]) -> Map<String, Value> {
    let mut out = Map::new();
    for k in keys {
        if let Some(v) = obj.remove(*k) {
            out.insert((*k).to_string(), v);
        }
    }
    out
}

fn from_map<T: DeserializeOwned>(map: Map<String, Value>) -> Result<T, String> {
    serde_json::from_value(Value::Object(map)).map_err(|e| e.to_string())
}

fn parse_layer(value: &Value) -> Result<LayerSpec, String> {
    let mut obj = value
        .as_object()
        .cloned()
        .ok_or_else(|| "a layer must be a JSON object".to_string())?;
    let ty = match obj.remove("type") {
        Some(Value::String(s)) => s,
        Some(_) => return Err("\"type\" must be a string".into()),
        None => return Err("missing \"type\"".into()),
    };
    // A layer-wide default easing for keyframes that do not name one.
    let ease = match obj.remove("ease") {
        Some(v) => Easing::parse(&v)?,
        None => Easing::Linear,
    };
    with_default_ease(ease, || parse_layer_body(&ty, obj))
}

fn parse_layer_body(ty: &str, mut obj: Map<String, Value>) -> Result<LayerSpec, String> {
    let common: Common = from_map(take_keys(&mut obj, COMMON_KEYS))?;
    let kind = match ty {
        "text" => {
            let placement = from_map(take_keys(&mut obj, PLACEMENT_KEYS))?;
            LayerKind::Text(Box::new(from_map(obj)?), placement)
        }
        "rect" | "line" | "ellipse" | "triangle" => {
            let shape = match ty {
                "ellipse" => ShapeKind::Ellipse,
                "triangle" => ShapeKind::Triangle,
                _ => ShapeKind::Rect,
            };
            let placement = from_map(take_keys(&mut obj, PLACEMENT_KEYS))?;
            LayerKind::Shape(shape, from_map(obj)?, placement)
        }
        "letterbox" => LayerKind::Letterbox(from_map(obj)?),
        "fill" => LayerKind::Fill(from_map(obj)?),
        "vignette" => LayerKind::Vignette(from_map(obj)?),
        "grain" => LayerKind::Grain(from_map(obj)?),
        "scanlines" => LayerKind::Scanlines(from_map(obj)?),
        "sweep" => LayerKind::Sweep(from_map(obj)?),
        "particles" => LayerKind::Particles(from_map(obj)?),
        "noise_band" => LayerKind::NoiseBand(from_map(obj)?),
        other => {
            return Err(format!(
                "unknown layer type {other:?} (expected text, rect, line, ellipse, triangle, \
                 letterbox, fill, vignette, grain, scanlines, sweep, particles or noise_band)"
            ));
        }
    };
    Ok(LayerSpec { common, kind })
}

fn describe_layer(i: usize, value: &Value) -> String {
    let ty = value.get("type").and_then(Value::as_str).unwrap_or("?");
    match value.get("id").and_then(Value::as_str) {
        Some(id) => format!("layer {i} ({ty} {id:?})"),
        None => format!("layer {i} ({ty})"),
    }
}

fn parse_background(v: &Value) -> Result<TitleBackground, String> {
    let obj = v
        .as_object()
        .ok_or_else(|| "\"background\" must be an object".to_string())?;
    let num = |key: &str, default: f32| -> Result<f32, String> {
        match obj.get(key) {
            None => Ok(default),
            Some(v) => v
                .as_f64()
                .map(|n| n as f32)
                .ok_or_else(|| format!("background {key:?} must be a number")),
        }
    };
    match obj.get("kind").and_then(Value::as_str) {
        Some("footage") => {
            if let Some(k) = obj
                .keys()
                .find(|k| !["kind", "dim", "blur"].contains(&k.as_str()))
            {
                return Err(format!("unknown background key {k:?}"));
            }
            let dim = num("dim", 0.45)?;
            let blur = num("blur", 1.0)?;
            if !(0.0..=1.0).contains(&dim) {
                return Err("background \"dim\" must be within 0..1".into());
            }
            if !(0.0..=10.0).contains(&blur) {
                return Err("background \"blur\" must be within 0..10".into());
            }
            Ok(TitleBackground::Footage { dim, blur })
        }
        Some("solid") => {
            if let Some(k) = obj
                .keys()
                .find(|k| !["kind", "color"].contains(&k.as_str()))
            {
                return Err(format!("unknown background key {k:?}"));
            }
            let color = obj
                .get("color")
                .and_then(Value::as_str)
                .ok_or_else(|| "a solid background needs a \"color\"".to_string())?;
            let [r, g, b] = Rgba::parse(color)?.to_u8();
            Ok(TitleBackground::Solid { r, g, b })
        }
        _ => Err("background \"kind\" must be \"footage\" or \"solid\"".into()),
    }
}

/// Parse and validate `template.json` text.
pub(crate) fn parse_template(json: &str) -> Result<TemplateSpec, String> {
    let raw: RawTemplate = serde_json::from_str(json).map_err(|e| e.to_string())?;
    if let Some(f) = raw.format
        && f > FORMAT_VERSION
    {
        return Err(format!(
            "template format {f} needs a newer version of this library (supported: {FORMAT_VERSION})"
        ));
    }
    if raw.name.trim().is_empty() {
        return Err("\"name\" must not be empty".into());
    }
    if !(raw.duration.is_finite() && raw.duration > 0.0 && raw.duration <= 60.0) {
        return Err("\"duration\" must be between 0 and 60 seconds".into());
    }
    let background = parse_background(&raw.background)?;

    let mut fields = Vec::new();
    let mut field_names = BTreeSet::new();
    for f in &raw.fields {
        if f.name.trim().is_empty() {
            return Err("field names must not be empty".into());
        }
        if !field_names.insert(f.name.clone()) {
            return Err(format!("field {:?} is declared twice", f.name));
        }
        if f.max_chars == 0 {
            return Err(format!(
                "field {:?}: \"max_chars\" must be at least 1",
                f.name
            ));
        }
        fields.push(FieldSpec {
            name: f.name.clone(),
            max_chars: f.max_chars,
            required: f.required,
        });
    }

    // Global layout keys (safe area, stacks) with their portrait overrides.
    let mut globals = Map::new();
    if let Some(v) = &raw.safe_area {
        globals.insert("safe_area".into(), v.clone());
    }
    if let Some(v) = &raw.stacks {
        globals.insert("stacks".into(), v.clone());
    }
    let mut portrait_globals = Value::Object(globals.clone());
    if let Some(p) = &raw.portrait {
        let p = p
            .as_object()
            .ok_or_else(|| "\"portrait\" must be an object".to_string())?;
        if let Some(k) = p.keys().find(|k| *k != "safe_area" && *k != "stacks") {
            return Err(format!(
                "top-level \"portrait\" may only override \"safe_area\" and \"stacks\", found {k:?}"
            ));
        }
        deep_merge(&mut portrait_globals, &Value::Object(p.clone()));
    }
    let land_globals: RawGlobals =
        from_map(globals).map_err(|e| format!("safe_area/stacks: {e}"))?;
    let port_globals: RawGlobals = serde_json::from_value(portrait_globals)
        .map_err(|e| format!("portrait safe_area/stacks: {e}"))?;

    let mut land_layers = Vec::new();
    let mut port_layers = Vec::new();
    for (i, value) in raw.layers.iter().enumerate() {
        let ctx = describe_layer(i, value);
        let mut base = value.clone();
        let over = base.as_object_mut().and_then(|o| o.remove("portrait"));
        let land = parse_layer(&base).map_err(|e| format!("{ctx}: {e}"))?;
        let mut merged = base;
        if let Some(over) = &over {
            if !over.is_object() {
                return Err(format!("{ctx}: \"portrait\" must be an object"));
            }
            deep_merge(&mut merged, over);
        }
        let port = parse_layer(&merged).map_err(|e| format!("{ctx} (portrait): {e}"))?;
        land_layers.push(land);
        port_layers.push(port);
    }

    let spec = TemplateSpec {
        name: raw.name,
        duration: raw.duration,
        background,
        sound: raw.sound.filter(|s| !s.trim().is_empty()),
        fonts: raw.fonts,
        fallback_fonts: raw.fallback_fonts,
        fields,
        landscape: Layout {
            safe_area: land_globals.safe_area.unwrap_or_default(),
            stacks: land_globals.stacks,
            layers: land_layers,
        },
        portrait: Layout {
            safe_area: port_globals.safe_area.unwrap_or_default(),
            stacks: port_globals.stacks,
            layers: port_layers,
        },
    };
    validate_layout(&spec, &spec.landscape, "")?;
    validate_layout(&spec, &spec.portrait, " (portrait)")?;
    Ok(spec)
}

fn check_anim(a: &Anim<f32>, what: &str, min: f32, max: f32) -> Result<(), String> {
    if a.values().iter().all(|v| (min..=max).contains(v)) {
        Ok(())
    } else {
        Err(format!("{what} must stay within {min}..{max}"))
    }
}

fn validate_layout(spec: &TemplateSpec, layout: &Layout, suffix: &str) -> Result<(), String> {
    let sa = layout.safe_area;
    if !(0.0..0.45).contains(&sa.x) || !(0.0..0.45).contains(&sa.y) {
        return Err(format!("safe_area{suffix}: x and y must be within 0..0.45"));
    }
    let field_names: BTreeSet<&str> = spec.fields.iter().map(|f| f.name.as_str()).collect();
    let mut ids = BTreeSet::new();
    let mut text_ids = BTreeSet::new();
    for layer in &layout.layers {
        if let Some(id) = &layer.common.id {
            if !ids.insert(id.as_str()) {
                return Err(format!("layer id {id:?} is used twice{suffix}"));
            }
            if matches!(layer.kind, LayerKind::Text(..)) {
                text_ids.insert(id.as_str());
            }
        }
    }
    for (i, layer) in layout.layers.iter().enumerate() {
        let ctx = match &layer.common.id {
            Some(id) => format!("layer {i} ({id:?}){suffix}"),
            None => format!("layer {i}{suffix}"),
        };
        let err = |m: String| format!("{ctx}: {m}");
        let c = &layer.common;
        for f in &c.requires {
            if !field_names.contains(f.as_str()) {
                return Err(err(format!("\"requires\" names undeclared field {f:?}")));
            }
        }
        if let (Some(s), Some(e)) = (c.start, c.end)
            && e <= s
        {
            return Err(err("\"end\" must be after \"start\"".into()));
        }
        check_anim(&c.opacity, "opacity", 0.0, 1.0).map_err(err)?;
        let check_placement = |p: &Placement| -> Result<(), String> {
            if let Some(stack) = &p.stack
                && !layout.stacks.contains_key(stack)
            {
                return Err(err(format!(
                    "stack {stack:?} is not declared in \"stacks\""
                )));
            }
            Ok(())
        };
        match &layer.kind {
            LayerKind::Text(t, p) => {
                check_placement(p)?;
                match (&t.field, &t.text) {
                    (Some(f), None) => {
                        if !field_names.contains(f.as_str()) {
                            return Err(err(format!("uses undeclared field {f:?}")));
                        }
                    }
                    (None, Some(_)) => {}
                    _ => return Err(err("needs exactly one of \"field\" or \"text\"".into())),
                }
                if !(t.size > 0.0 && t.size <= 1.0) {
                    return Err(err("\"size\" must be within 0..1".into()));
                }
                if !(1..=3).contains(&t.max_lines) {
                    return Err(err("\"max_lines\" must be 1, 2 or 3".into()));
                }
                if !(t.max_width > 0.0 && t.max_width <= 1.0) {
                    return Err(err("\"max_width\" must be within 0..1".into()));
                }
                if !(0.5..=4.0).contains(&t.line_height) {
                    return Err(err("\"line_height\" must be within 0.5..4".into()));
                }
                if !(1..=1000).contains(&t.weight) {
                    return Err(err("\"weight\" must be within 1..1000".into()));
                }
                if !(-45.0..=45.0).contains(&t.skew) {
                    return Err(err("\"skew\" must be within -45..45 degrees".into()));
                }
                check_anim(&t.reveal, "reveal", 0.0, 1.0).map_err(err)?;
                check_anim(&t.blur, "blur", 0.0, 0.2).map_err(err)?;
                check_anim(&t.tracking, "tracking", -0.5, 3.0).map_err(err)?;
                if let Some(s) = &t.stagger
                    && !(0.0..=2.0).contains(&s.each)
                {
                    return Err(err("stagger \"each\" must be within 0..2 seconds".into()));
                }
                if let Some(s) = &t.shadow
                    && !(0.0..=0.2).contains(&s.blur)
                {
                    return Err(err("shadow \"blur\" must be within 0..0.2".into()));
                }
                if let Some(g) = &t.glow
                    && !(0.0..=0.2).contains(&g.radius)
                {
                    return Err(err("glow \"radius\" must be within 0..0.2".into()));
                }
                if let Some(j) = &t.jitter
                    && !(0.0..=120.0).contains(&j.rate)
                {
                    return Err(err("jitter \"rate\" must be within 0..120".into()));
                }
            }
            LayerKind::Shape(_, s, p) => {
                check_placement(p)?;
                if let Some(target) = &s.width_of
                    && !text_ids.contains(target.as_str())
                {
                    return Err(err(format!(
                        "\"width_of\" names unknown text layer {target:?}"
                    )));
                }
                check_anim(&s.reveal, "reveal", 0.0, 1.0).map_err(err)?;
            }
            LayerKind::Letterbox(l) => {
                if let Some(size) = l.size
                    && !(0.0..0.5).contains(&size)
                {
                    return Err(err("letterbox \"size\" must be within 0..0.5".into()));
                }
                if let Some(a) = l.aspect
                    && !(a > 0.1 && a < 10.0)
                {
                    return Err(err("letterbox \"aspect\" must be within 0.1..10".into()));
                }
            }
            LayerKind::Fill(f) => {
                if f.color.is_none() == f.gradient.is_none() {
                    return Err(err(
                        "a fill needs exactly one of \"color\" or \"gradient\"".into()
                    ));
                }
                if let Some(g) = &f.gradient
                    && g.stops.len() < 2
                {
                    return Err(err("a gradient needs at least two stops".into()));
                }
            }
            LayerKind::Vignette(v) => {
                if !(0.0..=1.0).contains(&v.strength) || v.softness <= 0.0 {
                    return Err(err(
                        "vignette needs strength 0..1 and positive softness".into()
                    ));
                }
            }
            LayerKind::Grain(g) => {
                if !(0.0..=1.0).contains(&g.amount) || g.size <= 0.0 || g.fps <= 0.0 {
                    return Err(err("grain needs amount 0..1, positive size and fps".into()));
                }
            }
            LayerKind::Scanlines(s) => {
                if s.spacing <= 0.0 || !(0.0..=1.0).contains(&s.thickness) {
                    return Err(err(
                        "scanlines need positive spacing and thickness 0..1".into()
                    ));
                }
            }
            LayerKind::Sweep(s) => {
                if s.width <= 0.0 {
                    return Err(err("sweep \"width\" must be positive".into()));
                }
            }
            LayerKind::Particles(p) => {
                if p.count > 2000 {
                    return Err(err("at most 2000 particles are allowed".into()));
                }
                if p.colors.is_empty() {
                    return Err(err("particles need at least one colour".into()));
                }
                if let Some([a, b]) = p.size
                    && !(a > 0.0 && b >= a && b <= 0.5)
                {
                    return Err(err(
                        "particle \"size\" must be [min, max] within 0..0.5".into()
                    ));
                }
            }
            LayerKind::NoiseBand(n) => {
                if n.height <= 0.0 {
                    return Err(err("noise_band \"height\" must be positive".into()));
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn minimal(layers: &str) -> String {
        format!(
            r##"{{"name": "t", "duration": 3, "background": {{"kind": "solid", "color": "#000"}},
                "fields": [{{"name": "title", "max_chars": 20, "required": true}}],
                "layers": {layers}}}"##
        )
    }

    #[test]
    fn portrait_overrides_merge_deeply() {
        let json = minimal(
            r#"[{"type": "text", "field": "title", "font": "X", "size": 0.1,
                 "glow": {"radius": 0.03},
                 "portrait": {"size": 0.05, "glow": {"opacity": 0.2}}}]"#,
        );
        let spec = parse_template(&json).unwrap();
        let LayerKind::Text(land, _) = &spec.landscape.layers[0].kind else {
            panic!()
        };
        let LayerKind::Text(port, _) = &spec.portrait.layers[0].kind else {
            panic!()
        };
        assert_eq!(land.size, 0.1);
        assert_eq!(port.size, 0.05);
        let glow = port.glow.as_ref().unwrap();
        assert_eq!(glow.radius, 0.03);
        assert_eq!(glow.opacity.at(0.0), 0.2);
    }

    #[test]
    fn layer_ease_is_the_keyframe_default() {
        let json = minimal(
            r#"[{"type": "vignette", "ease": "ease_in", "opacity": [[0, 0], [1, 1]]},
                {"type": "vignette", "ease": "ease_in", "opacity": [[0, 0], [1, 1, "linear"]]},
                {"type": "vignette", "opacity": [[0, 0], [1, 1]]}]"#,
        );
        let spec = parse_template(&json).unwrap();
        let at_half: Vec<f32> = spec
            .landscape
            .layers
            .iter()
            .map(|l| l.common.opacity.at(0.5))
            .collect();
        assert!((at_half[0] - 0.125).abs() < 1e-5);
        assert!((at_half[1] - 0.5).abs() < 1e-5);
        assert!((at_half[2] - 0.5).abs() < 1e-5);
    }

    #[test]
    fn helpful_errors() {
        let e = parse_template(&minimal(
            r#"[{"type": "text", "field": "nope", "font": "X"}]"#,
        ))
        .unwrap_err();
        assert!(e.contains("undeclared field \"nope\""), "{e}");
        let e = parse_template(&minimal(
            r#"[{"type": "text", "field": "title", "font": "X", "sise": 1}]"#,
        ))
        .unwrap_err();
        assert!(e.contains("layer 0 (text)") && e.contains("sise"), "{e}");
        let e = parse_template(&minimal(r#"[{"type": "blob"}]"#)).unwrap_err();
        assert!(e.contains("unknown layer type"), "{e}");
        let e = parse_template(&minimal(
            r#"[{"type": "vignette", "opacity": [[0, 0], [1, 2]]}]"#,
        ))
        .unwrap_err();
        assert!(e.contains("opacity"), "{e}");
    }
}
