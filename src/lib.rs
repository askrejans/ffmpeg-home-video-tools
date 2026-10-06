//! Join, trim and normalise home videos into one polished movie.
//!
//! The library drives the `ffmpeg` and `ffprobe` executables. It probes any
//! input FFmpeg can read, plans a frame-exact timeline, and renders it in a
//! single encoding pass. Mismatched shapes get a blurred fill instead of black
//! bars, and clips without sound get silence. Optional transitions, animated
//! title intros, loudness levelling and a watermark are supported.

pub mod error;
pub mod titles;

pub use error::{Error, Result};
