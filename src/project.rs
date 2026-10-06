//! The description of a movie to render (serialisable as JSON).

use crate::error::{Error, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

/// A movie: clips in order, plus optional intro, transitions and finishing.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Project {
    pub clips: Vec<Clip>,
    #[serde(default)]
    pub intro: Option<Intro>,
    #[serde(default)]
    pub transition: Transition,
    pub output: Output,
    #[serde(default)]
    pub audio: AudioOptions,
    #[serde(default)]
    pub watermark: Option<Watermark>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Clip {
    pub path: PathBuf,
    #[serde(default)]
    pub trim: Option<Trim>,
}

impl Clip {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            trim: None,
        }
    }
}

/// Part of a clip to keep, in seconds from the clip's start.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Trim {
    #[serde(default)]
    pub start: f64,
    /// `None` keeps everything to the end.
    #[serde(default)]
    pub end: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Intro {
    /// A built-in template name (`clean`, `cinematic`, `retro`) or a path to a
    /// template directory.
    pub template: String,
    #[serde(default)]
    pub fields: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransitionKind {
    Cut,
    #[default]
    Crossfade,
    FadeBlack,
    FadeWhite,
    Slide,
    Wipe,
    Zoom,
    BlurDissolve,
    TapeRewind,
    /// A varied, tasteful rotation of the transitions above.
    Mix,
}

impl TransitionKind {
    pub const ALL: [TransitionKind; 10] = [
        TransitionKind::Cut,
        TransitionKind::Crossfade,
        TransitionKind::FadeBlack,
        TransitionKind::FadeWhite,
        TransitionKind::Slide,
        TransitionKind::Wipe,
        TransitionKind::Zoom,
        TransitionKind::BlurDissolve,
        TransitionKind::TapeRewind,
        TransitionKind::Mix,
    ];

    pub fn name(self) -> &'static str {
        match self {
            TransitionKind::Cut => "cut",
            TransitionKind::Crossfade => "crossfade",
            TransitionKind::FadeBlack => "fade_black",
            TransitionKind::FadeWhite => "fade_white",
            TransitionKind::Slide => "slide",
            TransitionKind::Wipe => "wipe",
            TransitionKind::Zoom => "zoom",
            TransitionKind::BlurDissolve => "blur_dissolve",
            TransitionKind::TapeRewind => "tape_rewind",
            TransitionKind::Mix => "mix",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Transition {
    #[serde(default)]
    pub kind: TransitionKind,
    /// Seconds. Clamped so a transition never takes more than 40 % of either
    /// neighbouring clip.
    #[serde(default = "default_transition_duration")]
    pub duration: f64,
}

fn default_transition_duration() -> f64 {
    1.0
}

impl Default for Transition {
    fn default() -> Self {
        Self {
            kind: TransitionKind::Crossfade,
            duration: default_transition_duration(),
        }
    }
}

/// Output canvas.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Preset {
    /// 3840×2160
    #[serde(rename = "4k")]
    Uhd4k,
    /// 1920×1080
    #[default]
    #[serde(rename = "1080p")]
    Fhd1080,
    /// 1280×720
    #[serde(rename = "720p")]
    Hd720,
    /// 1080×1920 (9:16 for phones)
    #[serde(rename = "vertical")]
    Vertical,
    /// `Output::width` × `Output::height`.
    #[serde(rename = "custom")]
    Custom,
}

impl Preset {
    /// Canvas size; `None` for `Custom`.
    pub fn size(self) -> Option<(u32, u32)> {
        match self {
            Preset::Uhd4k => Some((3840, 2160)),
            Preset::Fhd1080 => Some((1920, 1080)),
            Preset::Hd720 => Some((1280, 720)),
            Preset::Vertical => Some((1080, 1920)),
            Preset::Custom => None,
        }
    }

    /// Parse `4k`, `1080p`, `720p`, `vertical` or `WIDTHxHEIGHT`.
    pub fn parse(name: &str) -> Option<(Self, Option<(u32, u32)>)> {
        Some(match name.to_ascii_lowercase().as_str() {
            "4k" | "uhd" | "2160p" => (Preset::Uhd4k, None),
            "1080p" | "fhd" | "1080" => (Preset::Fhd1080, None),
            "720p" | "hd" | "720" => (Preset::Hd720, None),
            "vertical" | "portrait" | "9:16" => (Preset::Vertical, None),
            other => {
                let (w, h) = other.split_once('x')?;
                (Preset::Custom, Some((w.parse().ok()?, h.parse().ok()?)))
            }
        })
    }
}

/// Frame rates that divide 48 kHz audio into whole samples per frame.
pub const FRAME_RATES: [u32; 5] = [24, 25, 30, 50, 60];

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum FrameRate {
    /// The rate covering most of the footage, snapped to a standard rate.
    #[default]
    Auto,
    Fixed(u32),
}

impl Serialize for FrameRate {
    fn serialize<S: serde::Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        match self {
            FrameRate::Auto => s.serialize_str("auto"),
            FrameRate::Fixed(n) => s.serialize_u32(*n),
        }
    }
}

impl<'de> Deserialize<'de> for FrameRate {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Raw {
            Number(u32),
            Text(String),
        }
        match Raw::deserialize(d)? {
            Raw::Number(n) => Ok(FrameRate::Fixed(n)),
            Raw::Text(t) if t == "auto" => Ok(FrameRate::Auto),
            Raw::Text(t) => t
                .parse()
                .map(FrameRate::Fixed)
                .map_err(|_| serde::de::Error::custom("expected \"auto\" or a frame rate")),
        }
    }
}

/// Encoding effort / size trade-off.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Quality {
    /// Fast and small, for previews.
    Draft,
    #[default]
    Standard,
    High,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Output {
    /// Destination file (`.mp4`). When it exists and `overwrite` is false, a
    /// numbered name like `Movie (2).mp4` is used instead.
    pub path: PathBuf,
    #[serde(default)]
    pub preset: Preset,
    /// Canvas size for the `custom` preset.
    #[serde(default)]
    pub width: Option<u32>,
    #[serde(default)]
    pub height: Option<u32>,
    #[serde(default)]
    pub fps: FrameRate,
    #[serde(default)]
    pub quality: Quality,
    /// Force a specific FFmpeg video encoder instead of auto-detection.
    #[serde(default)]
    pub encoder: Option<String>,
    #[serde(default)]
    pub overwrite: bool,
    /// Movie title written to the file's metadata.
    #[serde(default)]
    pub title: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct AudioOptions {
    /// Bring every clip to a similar loudness before mixing.
    #[serde(default = "yes")]
    pub level_clips: bool,
    /// Two-pass EBU R128 normalisation of the finished soundtrack.
    #[serde(default = "default_loudness")]
    pub loudness: Option<LoudnessTarget>,
}

fn yes() -> bool {
    true
}

fn default_loudness() -> Option<LoudnessTarget> {
    Some(LoudnessTarget::default())
}

impl Default for AudioOptions {
    fn default() -> Self {
        Self {
            level_clips: true,
            loudness: default_loudness(),
        }
    }
}

/// EBU R128 targets.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct LoudnessTarget {
    /// Integrated loudness in LUFS (EBU R128: -23).
    #[serde(default = "default_integrated")]
    pub integrated: f64,
    /// Maximum true peak in dBTP (EBU R128: -1).
    #[serde(default = "default_true_peak")]
    pub true_peak: f64,
}

fn default_integrated() -> f64 {
    -23.0
}

fn default_true_peak() -> f64 {
    -1.0
}

impl Default for LoudnessTarget {
    fn default() -> Self {
        Self {
            integrated: default_integrated(),
            true_peak: default_true_peak(),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Anchor {
    TopLeft,
    TopRight,
    BottomLeft,
    #[default]
    BottomRight,
}

/// An image burned into every frame.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Watermark {
    /// PNG with transparency.
    pub image: PathBuf,
    #[serde(default)]
    pub anchor: Anchor,
    /// Width as a fraction of the canvas width.
    #[serde(default = "default_wm_width")]
    pub width_fraction: f64,
    /// Margin as a fraction of the canvas' shorter side.
    #[serde(default = "default_wm_margin")]
    pub margin_fraction: f64,
    #[serde(default = "default_wm_opacity")]
    pub opacity: f64,
}

fn default_wm_width() -> f64 {
    0.22
}

fn default_wm_margin() -> f64 {
    0.03
}

fn default_wm_opacity() -> f64 {
    0.85
}

impl Output {
    /// Canvas size in pixels.
    pub fn size(&self) -> (u32, u32) {
        self.preset
            .size()
            .unwrap_or((self.width.unwrap_or(0), self.height.unwrap_or(0)))
    }

    /// An output with default settings.
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            preset: Preset::default(),
            width: None,
            height: None,
            fps: FrameRate::Auto,
            quality: Quality::default(),
            encoder: None,
            overwrite: false,
            title: None,
        }
    }
}

impl Project {
    pub fn from_json(json: &str) -> Result<Self> {
        let project: Project = serde_json::from_str(json)?;
        project.validate()?;
        Ok(project)
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).expect("project serialises")
    }

    /// Check values that serde cannot.
    pub fn validate(&self) -> Result<()> {
        let bad = |m: String| Err(Error::InvalidProject(m));
        if self.clips.is_empty() {
            return Err(Error::NoClips);
        }
        for (i, clip) in self.clips.iter().enumerate() {
            if let Some(trim) = clip.trim {
                if !trim.start.is_finite() || trim.start < 0.0 {
                    return bad(format!("clip {}: trim start must be ≥ 0", i + 1));
                }
                if let Some(end) = trim.end
                    && (!end.is_finite() || end <= trim.start)
                {
                    return bad(format!("clip {}: trim end must be after its start", i + 1));
                }
            }
        }
        let (w, h) = self.output.size();
        if w < 64 || h < 64 || w > 8192 || h > 8192 || w % 2 != 0 || h % 2 != 0 {
            return bad(format!(
                "output size {w}×{h} must be even and between 64 and 8192"
            ));
        }
        if let FrameRate::Fixed(fps) = self.output.fps
            && !FRAME_RATES.contains(&fps)
        {
            return bad(format!("frame rate must be one of {FRAME_RATES:?}"));
        }
        if !self.transition.duration.is_finite() || self.transition.duration < 0.0 {
            return bad("transition duration must be ≥ 0".into());
        }
        if let Some(target) = self.audio.loudness
            && (!(-70.0..=-5.0).contains(&target.integrated)
                || !(-9.0..=0.0).contains(&target.true_peak))
        {
            return bad(
                "loudness target must be -70..-5 LUFS with a true peak of -9..0 dBTP".into(),
            );
        }
        if let Some(wm) = &self.watermark
            && (!(0.01..=1.0).contains(&wm.width_fraction)
                || !(0.0..=0.4).contains(&wm.margin_fraction)
                || !(0.0..=1.0).contains(&wm.opacity))
        {
            return bad("watermark size, margin or opacity is out of range".into());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn minimal_json_uses_defaults() {
        let p = Project::from_json(r#"{"clips":[{"path":"a.mp4"}],"output":{"path":"out.mp4"}}"#)
            .unwrap();
        assert_eq!(p.output.preset, Preset::Fhd1080);
        assert_eq!(p.output.fps, FrameRate::Auto);
        assert_eq!(p.transition.kind, TransitionKind::Crossfade);
        assert!(p.audio.level_clips);
        assert_eq!(p.audio.loudness.unwrap().integrated, -23.0);
    }

    #[test]
    fn full_json_round_trips() {
        let json = r#"{
          "clips":[{"path":"a.mov","trim":{"start":1.5,"end":9}},{"path":"b.mp4"}],
          "intro":{"template":"retro","fields":{"title":"Vasara","date":"2026"}},
          "transition":{"kind":"tape_rewind","duration":0.5},
          "output":{"path":"o.mp4","preset":"custom","width":854,"height":480,"fps":25,"quality":"draft"},
          "audio":{"level_clips":false,"loudness":null},
          "watermark":{"image":"m.png"}
        }"#;
        let p = Project::from_json(json).unwrap();
        assert_eq!(p.output.preset, Preset::Custom);
        assert_eq!(p.output.size(), (854, 480));
        assert_eq!(p.output.fps, FrameRate::Fixed(25));
        assert!(p.audio.loudness.is_none());
        assert_eq!(p.watermark.as_ref().unwrap().anchor, Anchor::BottomRight);
        let again = Project::from_json(&p.to_json()).unwrap();
        assert_eq!(again, p);
        let p = Project::from_json(
            r#"{"clips":[{"path":"a"}],"output":{"path":"o","preset":"vertical","fps":"auto"}}"#,
        )
        .unwrap();
        assert_eq!(p.output.size(), (1080, 1920));
    }

    #[test]
    fn rejects_bad_values() {
        let err = |json: &str| Project::from_json(json).unwrap_err().code();
        assert_eq!(err(r#"{"clips":[],"output":{"path":"o"}}"#), "no_clips");
        assert_eq!(
            err(r#"{"clips":[{"path":"a","trim":{"start":5,"end":2}}],"output":{"path":"o"}}"#),
            "invalid_project"
        );
        assert_eq!(
            err(r#"{"clips":[{"path":"a"}],"output":{"path":"o","fps":29}}"#),
            "invalid_project"
        );
        assert_eq!(
            err(
                r#"{"clips":[{"path":"a"}],"output":{"path":"o","preset":"custom","width":641,"height":480}}"#
            ),
            "invalid_project"
        );
    }

    #[test]
    fn preset_parsing() {
        assert_eq!(Preset::parse("4K"), Some((Preset::Uhd4k, None)));
        assert_eq!(Preset::parse("vertical"), Some((Preset::Vertical, None)));
        assert_eq!(
            Preset::parse("640x360"),
            Some((Preset::Custom, Some((640, 360))))
        );
        assert_eq!(Preset::parse("huge"), None);
    }
}
