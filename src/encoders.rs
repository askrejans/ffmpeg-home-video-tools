//! Choosing H.264 and AAC encoders that actually work on this machine.

use crate::error::{Error, Result};
use crate::events::CancelToken;
use crate::project::Quality;
use crate::tools::FfmpegTools;
use serde::Serialize;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

/// A video encoder configuration that passed a trial encode.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct VideoEncoder {
    /// Identifier, e.g. `h264_videotoolbox` or `h264_mf_hw`.
    pub id: String,
    /// FFmpeg encoder name.
    pub codec: String,
    /// Uses dedicated hardware.
    pub hardware: bool,
}

/// Candidate H.264 encoders, best first.
fn video_candidates() -> &'static [(&'static str, &'static str, bool)] {
    if cfg!(target_os = "macos") {
        &[
            ("h264_videotoolbox", "h264_videotoolbox", true),
            ("libx264", "libx264", false),
            ("libopenh264", "libopenh264", false),
        ]
    } else if cfg!(windows) {
        &[
            ("h264_nvenc", "h264_nvenc", true),
            ("h264_qsv", "h264_qsv", true),
            ("h264_amf", "h264_amf", true),
            ("h264_mf_hw", "h264_mf", true),
            ("h264_mf", "h264_mf", false),
            ("libx264", "libx264", false),
            ("libopenh264", "libopenh264", false),
        ]
    } else {
        &[
            ("h264_nvenc", "h264_nvenc", true),
            ("libx264", "libx264", false),
            ("libopenh264", "libopenh264", false),
        ]
    }
}

/// Target video bitrate in kbit/s.
pub(crate) fn video_bitrate_kbps(width: u32, height: u32, fps: u32, quality: Quality) -> u32 {
    let pixels = f64::from(width) * f64::from(height);
    // 16 Mbit/s for 1080p30, scaled sub-linearly with pixel count.
    let base = 16_000.0 * (pixels / (1920.0 * 1080.0)).powf(0.8);
    let motion = if fps >= 50 { 1.5 } else { 1.0 };
    let quality = match quality {
        Quality::Draft => 0.35,
        Quality::Standard => 1.0,
        Quality::High => 1.4,
    };
    ((base * motion * quality) as u32).clamp(500, 120_000)
}

/// Encoder-specific arguments (after `-c:v`).
pub(crate) fn video_args(id: &str, kbps: u32, fps: u32, quality: Quality) -> Vec<String> {
    let b = format!("{kbps}k");
    let max = format!("{}k", kbps * 3 / 2);
    let buf = format!("{}k", kbps * 2);
    let gop = (fps * 2).to_string();
    let mut args: Vec<String> = match id {
        "h264_videotoolbox" => vec![
            "-b:v".into(),
            b,
            "-maxrate".into(),
            max,
            "-bufsize".into(),
            buf,
            "-allow_sw".into(),
            "1".into(),
            "-realtime".into(),
            "0".into(),
            "-profile:v".into(),
            "high".into(),
        ],
        "libx264" => {
            let (preset, crf) = match quality {
                Quality::Draft => ("veryfast", "26"),
                Quality::Standard => ("medium", "19"),
                Quality::High => ("slow", "17"),
            };
            vec![
                "-preset".into(),
                preset.into(),
                "-crf".into(),
                crf.into(),
                "-maxrate".into(),
                max,
                "-bufsize".into(),
                buf,
                "-profile:v".into(),
                "high".into(),
                "-pix_fmt".into(),
                "yuv420p".into(),
            ]
        }
        "h264_nvenc" => vec![
            "-preset".into(),
            if quality == Quality::Draft {
                "p2"
            } else {
                "p5"
            }
            .into(),
            "-rc".into(),
            "vbr".into(),
            "-b:v".into(),
            b,
            "-maxrate".into(),
            max,
            "-bufsize".into(),
            buf,
            "-profile:v".into(),
            "high".into(),
        ],
        "h264_qsv" => vec![
            "-preset".into(),
            if quality == Quality::Draft {
                "veryfast"
            } else {
                "medium"
            }
            .into(),
            "-b:v".into(),
            b,
            "-maxrate".into(),
            max,
            "-bufsize".into(),
            buf,
            "-profile:v".into(),
            "high".into(),
        ],
        "h264_amf" => vec![
            "-quality".into(),
            if quality == Quality::Draft {
                "speed"
            } else {
                "balanced"
            }
            .into(),
            "-rc".into(),
            "vbr_peak".into(),
            "-b:v".into(),
            b,
            "-maxrate".into(),
            max,
            "-bufsize".into(),
            buf,
            "-profile:v".into(),
            "high".into(),
        ],
        "h264_mf_hw" | "h264_mf" => vec![
            "-hw_encoding".into(),
            if id == "h264_mf_hw" { "1" } else { "0" }.into(),
            "-rate_control".into(),
            "u_vbr".into(),
            "-b:v".into(),
            b,
        ],
        _ => vec![
            "-b:v".into(),
            b,
            "-maxrate".into(),
            max,
            "-bufsize".into(),
            buf,
        ],
    };
    args.extend(["-g".into(), gop]);
    args
}

type Key = (PathBuf, u32, u32);

fn cache() -> &'static Mutex<HashMap<Key, VideoEncoder>> {
    static CACHE: OnceLock<Mutex<HashMap<Key, VideoEncoder>>> = OnceLock::new();
    CACHE.get_or_init(Default::default)
}

/// Encode a few frames to prove an encoder works at this size.
fn trial_video(
    tools: &FfmpegTools,
    id: &str,
    codec: &str,
    w: u32,
    h: u32,
    cancel: &CancelToken,
) -> bool {
    let mut cmd = tools.ffmpeg();
    cmd.args([
        "-f",
        "lavfi",
        "-i",
        &format!("testsrc2=s={w}x{h}:r=30:d=0.3"),
    ])
    .args(["-frames:v", "8", "-c:v", codec])
    .args(video_args(id, 2_000, 30, Quality::Draft))
    .args(["-f", "null", "-"]);
    cmd.run(cancel).is_ok()
}

/// The best working H.264 encoder for a `width`×`height` canvas. `forced`
/// selects a specific encoder id or FFmpeg encoder name instead.
pub fn select_video_encoder(
    tools: &FfmpegTools,
    width: u32,
    height: u32,
    forced: Option<&str>,
    cancel: &CancelToken,
) -> Result<VideoEncoder> {
    let available = tools.encoders()?;
    if let Some(name) = forced {
        let (id, codec, hardware) = video_candidates()
            .iter()
            .find(|(id, codec, _)| *id == name || *codec == name)
            .map(|(i, c, h)| (i.to_string(), c.to_string(), *h))
            .unwrap_or((name.to_string(), name.to_string(), false));
        if !available.contains(&codec) {
            return Err(Error::EncoderUnavailable { kind: "video" });
        }
        return Ok(VideoEncoder {
            id,
            codec,
            hardware,
        });
    }
    let key = (tools.ffmpeg_path().to_path_buf(), width, height);
    if let Some(found) = cache().lock().expect("cache").get(&key) {
        return Ok(found.clone());
    }
    for (id, codec, hardware) in video_candidates() {
        cancel.check()?;
        if available.contains(*codec) && trial_video(tools, id, codec, width, height, cancel) {
            let chosen = VideoEncoder {
                id: id.to_string(),
                codec: codec.to_string(),
                hardware: *hardware,
            };
            tracing::info!(encoder = %chosen.id, width, height, "selected video encoder");
            cache().lock().expect("cache").insert(key, chosen.clone());
            return Ok(chosen);
        }
    }
    Err(Error::EncoderUnavailable { kind: "video" })
}

/// Every candidate that works at the given size (for diagnostics).
pub fn working_video_encoders(
    tools: &FfmpegTools,
    width: u32,
    height: u32,
    cancel: &CancelToken,
) -> Result<Vec<VideoEncoder>> {
    let available = tools.encoders()?;
    let mut out = Vec::new();
    for (id, codec, hardware) in video_candidates() {
        cancel.check()?;
        if available.contains(*codec) && trial_video(tools, id, codec, width, height, cancel) {
            out.push(VideoEncoder {
                id: id.to_string(),
                codec: codec.to_string(),
                hardware: *hardware,
            });
        }
    }
    Ok(out)
}

/// The best working AAC encoder: the operating system's where available.
pub fn select_audio_encoder(tools: &FfmpegTools, cancel: &CancelToken) -> Result<String> {
    static CHOSEN: OnceLock<Mutex<HashMap<PathBuf, String>>> = OnceLock::new();
    let chosen = CHOSEN.get_or_init(Default::default);
    if let Some(found) = chosen.lock().expect("cache").get(tools.ffmpeg_path()) {
        return Ok(found.clone());
    }
    let available = tools.encoders()?;
    for codec in ["aac_at", "aac_mf", "aac"] {
        if !available.contains(codec) {
            continue;
        }
        let mut cmd = tools.ffmpeg();
        cmd.args([
            "-f",
            "lavfi",
            "-i",
            "anullsrc=r=48000:cl=stereo",
            "-t",
            "0.2",
        ])
        .args(["-c:a", codec, "-b:a", "192k", "-f", "null", "-"]);
        if cmd.run(cancel).is_ok() {
            chosen
                .lock()
                .expect("cache")
                .insert(tools.ffmpeg_path().to_path_buf(), codec.to_string());
            return Ok(codec.to_string());
        }
    }
    Err(Error::EncoderUnavailable { kind: "audio" })
}

/// Arguments for the chosen AAC encoder at 192 kbit/s stereo.
pub(crate) fn audio_args(codec: &str) -> Vec<String> {
    let mut args = vec![
        "-c:a".to_string(),
        codec.to_string(),
        "-b:a".into(),
        "192k".into(),
    ];
    if codec == "aac_at" {
        args.extend(["-aac_at_mode".into(), "cvbr".into()]);
    }
    args
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bitrates_scale_with_size_and_rate() {
        assert_eq!(
            video_bitrate_kbps(1920, 1080, 30, Quality::Standard),
            16_000
        );
        let uhd = video_bitrate_kbps(3840, 2160, 30, Quality::Standard);
        assert!((44_000..=50_000).contains(&uhd), "{uhd}");
        assert!(video_bitrate_kbps(1280, 720, 30, Quality::Standard) < 10_000);
        assert_eq!(
            video_bitrate_kbps(1080, 1920, 30, Quality::Standard),
            16_000
        );
        assert_eq!(
            video_bitrate_kbps(1920, 1080, 60, Quality::Standard),
            24_000
        );
        assert!(video_bitrate_kbps(1920, 1080, 30, Quality::Draft) < 6_000);
    }

    #[test]
    fn x264_options_only_for_x264() {
        let vt = video_args("h264_videotoolbox", 16_000, 30, Quality::Standard);
        assert!(!vt.contains(&"-crf".to_string()));
        assert!(vt.windows(2).any(|w| w == ["-b:v", "16000k"]));
        assert!(vt.windows(2).any(|w| w == ["-g", "60"]));
        let x = video_args("libx264", 16_000, 25, Quality::High);
        assert!(x.windows(2).any(|w| w == ["-crf", "17"]));
        let mf = video_args("h264_mf_hw", 8_000, 30, Quality::Standard);
        assert!(mf.windows(2).any(|w| w == ["-hw_encoding", "1"]));
    }
}
