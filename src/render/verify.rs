//! Checking a rendered file before it is handed to the user.

use crate::error::{Error, Result};
use crate::events::CancelToken;
use crate::plan::RenderPlan;
use crate::tools::FfmpegTools;
use serde::Deserialize;
use std::io::Read;
use std::path::Path;

#[derive(Deserialize)]
struct Probe {
    #[serde(default)]
    streams: Vec<Stream>,
}

#[derive(Deserialize)]
struct Stream {
    codec_type: Option<String>,
    codec_name: Option<String>,
    width: Option<u32>,
    height: Option<u32>,
    avg_frame_rate: Option<String>,
    sample_aspect_ratio: Option<String>,
    pix_fmt: Option<String>,
    sample_rate: Option<String>,
    channels: Option<u32>,
    duration: Option<String>,
    nb_frames: Option<String>,
}

fn seconds(v: &Option<String>) -> Option<f64> {
    v.as_ref()?.parse().ok()
}

/// Top-level MP4 boxes in file order (stops at the first unreadable box).
pub(crate) fn top_level_boxes(path: &Path) -> std::io::Result<Vec<String>> {
    let mut file = std::fs::File::open(path)?;
    let len = file.metadata()?.len();
    let mut boxes = Vec::new();
    let mut offset = 0u64;
    let mut header = [0u8; 16];
    while offset + 8 <= len && boxes.len() < 64 {
        use std::io::Seek;
        file.seek(std::io::SeekFrom::Start(offset))?;
        file.read_exact(&mut header[..8])?;
        let mut size = u64::from(u32::from_be_bytes([
            header[0], header[1], header[2], header[3],
        ]));
        let kind = String::from_utf8_lossy(&header[4..8]).into_owned();
        if size == 1 {
            file.read_exact(&mut header[8..16])?;
            size = u64::from_be_bytes(header[8..16].try_into().expect("8 bytes"));
        } else if size == 0 {
            size = len - offset;
        }
        boxes.push(kind);
        if size < 8 {
            break;
        }
        offset += size;
    }
    Ok(boxes)
}

/// Verify streams, timing, layout and decodability of a finished render.
pub(crate) fn verify(
    tools: &FfmpegTools,
    path: &Path,
    plan: &RenderPlan,
    cancel: &CancelToken,
) -> Result<()> {
    let fail = |m: String| Err(Error::OutputInvalid(m));
    let mut cmd = tools.ffprobe();
    cmd.args(["-print_format", "json", "-show_streams"])
        .output_path(path);
    let json = cmd.output(cancel)?;
    let probe: Probe = serde_json::from_slice(&json)?;
    let video = probe
        .streams
        .iter()
        .find(|s| s.codec_type.as_deref() == Some("video"));
    let audio = probe
        .streams
        .iter()
        .find(|s| s.codec_type.as_deref() == Some("audio"));
    let (Some(video), Some(audio)) = (video, audio) else {
        return fail("the file must contain one video and one audio stream".into());
    };
    if video.codec_name.as_deref() != Some("h264") {
        return fail(format!("unexpected video codec {:?}", video.codec_name));
    }
    if (video.width, video.height) != (Some(plan.width), Some(plan.height)) {
        return fail(format!(
            "unexpected size {:?}×{:?}",
            video.width, video.height
        ));
    }
    if video.avg_frame_rate.as_deref() != Some(&format!("{}/1", plan.fps)) {
        return fail(format!("unexpected frame rate {:?}", video.avg_frame_rate));
    }
    if !matches!(
        video.sample_aspect_ratio.as_deref(),
        None | Some("1:1") | Some("0:1") | Some("N/A")
    ) {
        return fail(format!("non-square pixels {:?}", video.sample_aspect_ratio));
    }
    if !matches!(video.pix_fmt.as_deref(), Some("yuv420p") | Some("yuvj420p")) {
        return fail(format!("unexpected pixel format {:?}", video.pix_fmt));
    }
    if audio.codec_name.as_deref() != Some("aac")
        || audio.sample_rate.as_deref() != Some("48000")
        || audio.channels != Some(2)
    {
        return fail("audio must be 48 kHz stereo AAC".into());
    }
    let frame = 1.0 / f64::from(plan.fps);
    let expected = plan.duration_seconds();
    if let Some(frames) = video.nb_frames.as_ref().and_then(|n| n.parse::<u64>().ok())
        && frames.abs_diff(plan.total_frames) > 1
    {
        return fail(format!("{frames} frames instead of {}", plan.total_frames));
    }
    let vdur = seconds(&video.duration).unwrap_or(expected);
    let adur = seconds(&audio.duration).unwrap_or(vdur);
    if (vdur - expected).abs() > frame + 0.01 {
        return fail(format!("video lasts {vdur:.3}s instead of {expected:.3}s"));
    }
    // One AAC frame (1024 samples) of slack on top of one video frame.
    if (adur - vdur).abs() > frame + 0.025 {
        return fail(format!(
            "audio ({adur:.3}s) and video ({vdur:.3}s) lengths differ"
        ));
    }
    let boxes = top_level_boxes(path).map_err(|e| Error::io("reading the rendered file", e))?;
    let moov = boxes.iter().position(|b| b == "moov");
    let mdat = boxes.iter().position(|b| b == "mdat");
    if !matches!((moov, mdat), (Some(m), Some(d)) if m < d) {
        return fail("the file is not optimised for streaming (moov after mdat)".into());
    }
    // Decode the first and last seconds; any decoder complaint fails.
    for window in [["-t", "2"], ["-sseof", "-2"]] {
        let mut cmd = tools.ffmpeg();
        if window[0] == "-sseof" {
            cmd.args(window);
            cmd.input_path(path).args(["-f", "null", "-"]);
        } else {
            cmd.input_path(path).args(window).args(["-f", "null", "-"]);
        }
        let (_, stderr) = cmd.output_with_stderr(cancel)?;
        if !stderr.trim().is_empty() {
            return fail(format!(
                "decoding errors: {}",
                stderr.lines().next().unwrap_or_default()
            ));
        }
    }
    Ok(())
}
