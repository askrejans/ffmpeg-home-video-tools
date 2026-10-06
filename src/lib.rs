//! Join, trim and normalise home videos into one polished movie.
//!
//! The library drives the `ffmpeg` and `ffprobe` executables. It probes any
//! input FFmpeg can read, plans a frame-exact timeline, and renders it in a
//! single encoding pass. Mismatched shapes get a blurred fill instead of black
//! bars, and clips without sound get silence. Optional transitions, animated
//! title intros, two-pass EBU R128 loudness normalisation and a watermark are
//! supported.

pub mod discover;
pub mod encoders;
pub mod error;
pub mod events;
pub mod plan;
pub mod preview;
pub mod probe;
pub mod project;
pub mod render;
pub mod titles;
pub mod tools;

mod frame;
mod transitions;

pub use discover::{ScanItem, ScanKind, ScanOptions, SortOrder, scan, sort_media};
pub use error::{Error, Result};
pub use events::{CancelToken, Event, Stage};
pub use plan::{RenderPlan, plan};
pub use probe::{MediaInfo, MediaKind, probe};
pub use project::{Clip, Intro, Output, Preset, Project, Transition, TransitionKind, Trim};
pub use render::{RenderOptions, RenderOutcome, render, render_probed};
pub use tools::{FfmpegTools, InputAccess, ToolPaths};
