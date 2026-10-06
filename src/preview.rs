//! Stills, filmstrips and lightweight proxy copies for previewing clips.

use crate::encoders::{select_video_encoder, video_args};
use crate::error::{Error, Result};
use crate::events::{CancelToken, Event, EventSink, Stage};
use crate::probe::{MediaInfo, VideoInfo};
use crate::project::Quality;
use crate::render::filters::contain;
use crate::tools::FfmpegTools;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::Stdio;

fn video_of(media: &MediaInfo) -> Result<&VideoInfo> {
    media.video.as_ref().ok_or_else(|| Error::Unreadable {
        path: media.path.clone(),
        reason: "it is not a video".into(),
    })
}

/// Picture clean-up shared by all previews: deinterlace, tone-map, square
/// pixels at `w`×`h`.
fn picture_chain(tools: &FfmpegTools, video: &VideoInfo, w: u32, h: u32) -> String {
    let mut chain = Vec::new();
    if video.interlaced {
        chain.push("bwdif=mode=send_frame:deint=interlaced".to_string());
    }
    if video.hdr.is_some() && tools.has_filter("zscale") && tools.has_filter("tonemap") {
        chain.push(
            "zscale=t=linear:npl=100,format=gbrpf32le,zscale=p=bt709,tonemap=tonemap=hable:desat=0,\
             zscale=t=bt709:m=bt709:r=tv"
                .to_string(),
        );
    }
    chain.push(format!(
        "scale={w}:{h}:flags=bicubic:in_range=auto:out_range=tv:in_color_matrix=auto:out_color_matrix=bt709,setsar=1"
    ));
    chain.join(",")
}

fn size_for_width(video: &VideoInfo, width: u32) -> (u32, u32) {
    let w = (width.max(16) / 2) * 2;
    let h = ((f64::from(w) / video.display_aspect() / 2.0).round() as u32 * 2).max(2);
    (w, h)
}

fn size_for_height(video: &VideoInfo, height: u32) -> (u32, u32) {
    let h = (height.max(16) / 2) * 2;
    let w = ((f64::from(h) * video.display_aspect() / 2.0).round() as u32 * 2).max(2);
    (w, h)
}

fn temp_sibling(out: &Path) -> PathBuf {
    let name = out
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    out.with_file_name(format!(
        ".{name}.partial.{}",
        out.extension()
            .map(|e| e.to_string_lossy())
            .unwrap_or_default()
    ))
}

fn finish_file(tmp: &Path, out: &Path) -> Result<PathBuf> {
    if std::fs::metadata(tmp).map(|m| m.len() == 0).unwrap_or(true) {
        let _ = std::fs::remove_file(tmp);
        return Err(Error::ToolFailed {
            tool: "ffmpeg",
            status: None,
            stderr: "no picture was produced".into(),
        });
    }
    std::fs::rename(tmp, out).map_err(|e| Error::io(format!("saving {}", out.display()), e))?;
    Ok(out.to_path_buf())
}

/// Save one JPEG frame at `at` seconds, `width` pixels wide.
pub fn thumbnail(
    tools: &FfmpegTools,
    media: &MediaInfo,
    at: f64,
    width: u32,
    out: &Path,
    cancel: &CancelToken,
) -> Result<PathBuf> {
    let video = video_of(media)?;
    let (w, h) = size_for_width(video, width);
    let at = at.clamp(0.0, (media.duration - 0.1).max(0.0));
    let tmp = temp_sibling(out);
    let mut cmd = tools.ffmpeg();
    cmd.args(["-ss", &format!("{at:.3}")]);
    cmd.input(&media.path)?;
    cmd.args([
        "-map",
        &format!("0:{}", video.stream_index),
        "-frames:v",
        "1",
    ])
    .args(["-vf", &picture_chain(tools, video, w, h), "-q:v", "3"])
    .args(["-f", "image2", "-update", "1", "-y"])
    .output_path(&tmp);
    cmd.run(cancel)?;
    finish_file(&tmp, out)
}

/// Save `count` evenly spaced JPEG frames, `height` pixels high, as
/// `<dir>/<prefix>-001.jpg`… Long clips are sampled from key frames only.
pub fn filmstrip(
    tools: &FfmpegTools,
    media: &MediaInfo,
    count: u32,
    height: u32,
    dir: &Path,
    prefix: &str,
    cancel: &CancelToken,
) -> Result<Vec<PathBuf>> {
    let video = video_of(media)?;
    let count = count.clamp(1, 200);
    let (w, h) = size_for_height(video, height);
    std::fs::create_dir_all(dir)
        .map_err(|e| Error::io(format!("creating {}", dir.display()), e))?;
    let rate = f64::from(count) / media.duration.max(0.1);
    let mut cmd = tools.ffmpeg();
    if media.duration > 30.0 {
        cmd.args(["-skip_frame", "nokey"]);
    }
    cmd.input(&media.path)?;
    cmd.args([
        "-map",
        &format!("0:{}", video.stream_index),
        "-an",
        "-sn",
        "-dn",
    ])
    .args([
        "-vf",
        &format!(
            "fps=fps={rate:.6}:start_time=0,{}",
            picture_chain(tools, video, w, h)
        ),
    ])
    .args([
        "-frames:v",
        &count.to_string(),
        "-q:v",
        "4",
        "-f",
        "image2",
        "-y",
    ])
    .output_path(&dir.join(format!("{prefix}-%03d.jpg")));
    cmd.run(cancel)?;
    let frames: Vec<PathBuf> = (1..=count)
        .map(|i| dir.join(format!("{prefix}-{i:03}.jpg")))
        .filter(|p| p.exists())
        .collect();
    if frames.is_empty() {
        return Err(Error::ToolFailed {
            tool: "ffmpeg",
            status: None,
            stderr: "no pictures were produced".into(),
        });
    }
    Ok(frames)
}

/// Make a small, upright, square-pixel H.264 copy that scrubs smoothly
/// (`height` pixels high, a key frame every half second).
pub fn proxy(
    tools: &FfmpegTools,
    media: &MediaInfo,
    height: u32,
    out: &Path,
    events: EventSink,
    cancel: &CancelToken,
) -> Result<PathBuf> {
    let video = video_of(media)?;
    let (w, h) = size_for_height(video, height);
    let (w, h) = contain(f64::from(w) / f64::from(h), w.min(1920), h);
    let fps = crate::plan::auto_frame_rate(&[(video.frame_rate, 1.0)]);
    let encoder = select_video_encoder(tools, w, h, None, cancel)?;
    let tmp = temp_sibling(out);
    let mut cmd = tools.ffmpeg();
    cmd.input(&media.path)?;
    cmd.args(["-map", &format!("0:{}", video.stream_index)]);
    if let Some(audio) = &media.audio {
        cmd.args(["-map", &format!("0:{}", audio.stream_index)])
            .args([
                "-af",
                "aresample=48000:async=1:first_pts=0",
                "-ac",
                "2",
                "-c:a",
                "aac",
                "-b:a",
                "96k",
            ]);
    }
    cmd.args([
        "-vf",
        &format!(
            "fps=fps={fps}:start_time=0,{}",
            picture_chain(tools, video, w, h)
        ),
    ])
    .args(["-c:v", &encoder.codec])
    .args(video_args(
        &encoder.id,
        1_500 * h / 540,
        fps,
        Quality::Draft,
    ))
    .args(["-g", &(fps / 2).max(1).to_string(), "-sn", "-dn"])
    .args([
        "-movflags",
        "+faststart",
        "-progress",
        "pipe:1",
        "-nostats",
        "-f",
        "mp4",
        "-y",
    ])
    .output_path(&tmp);
    let mut running = cmd.spawn(Stdio::null(), Stdio::piped())?;
    let stdout = running.take_stdout().expect("piped stdout");
    let (from, to) = Stage::Video.span();
    let total = media.duration.max(0.001);
    for line in BufReader::new(stdout)
        .lines()
        .map_while(std::result::Result::ok)
    {
        if cancel.is_cancelled() {
            break;
        }
        if let Some(us) = line
            .strip_prefix("out_time_us=")
            .and_then(|v| v.trim().parse::<f64>().ok())
        {
            let done = (us / 1e6 / total).clamp(0.0, 1.0);
            events(Event::Progress {
                fraction: from + (to - from) * done,
                stage: Stage::Video,
                done: (done * 1000.0) as u64,
                total: 1000,
                fps: None,
                eta_seconds: None,
                clip: None,
            });
        }
    }
    if let Err(e) = running.wait(cancel) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    finish_file(&tmp, out)
}
