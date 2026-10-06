//! The frame server: decoders → compositor → one encoder.

use crate::encoders::{VideoEncoder, video_args, video_bitrate_kbps};
use crate::error::{Error, Result};
use crate::events::{CancelToken, Event, EventSink, Stage};
use crate::frame::{Stamp, frame_len};
use crate::plan::{RenderPlan, SegmentSource};
use crate::project::Quality;
use crate::render::decode::{ClipDecoder, DecodeSummary};
use crate::render::filters::clip_graph;
use crate::render::intro::IntroSource;
use crate::tools::FfmpegTools;
use crate::transitions::blend;
use std::io::Write;
use std::path::Path;
use std::process::Stdio;
use std::sync::mpsc::sync_channel;
use std::time::{Duration, Instant};

pub(crate) struct VideoJob<'a> {
    pub tools: &'a FfmpegTools,
    pub plan: &'a RenderPlan,
    pub encoder: &'a VideoEncoder,
    pub quality: Quality,
    pub audio: &'a Path,
    pub output: &'a Path,
    pub title: Option<&'a str>,
    pub stamp: Option<&'a Stamp>,
    pub intro: Option<&'a IntroSource>,
    pub tonemap: bool,
    pub hardware_decode: bool,
    pub cancel: &'a CancelToken,
}

enum Source {
    Clip(Box<ClipDecoder>),
    Intro(crate::render::intro::IntroStream),
}

impl Source {
    fn next_frame(&mut self) -> Result<Vec<u8>> {
        match self {
            Source::Clip(d) => d.next_frame(),
            Source::Intro(i) => i.next_frame(),
        }
    }

    fn finish(self) -> Option<DecodeSummary> {
        match self {
            Source::Clip(d) => Some(d.finish()),
            Source::Intro(i) => {
                i.finish();
                None
            }
        }
    }
}

fn open(job: &VideoJob, index: usize) -> Result<Source> {
    let plan = job.plan;
    let segment = &plan.segments[index];
    let seconds = segment.frames as f64 / f64::from(plan.fps);
    match &segment.source {
        SegmentSource::Intro => {
            let intro = job.intro.expect("intro segment needs an intro source");
            Ok(Source::Intro(intro.stream(
                job.tools,
                segment.frames,
                job.cancel,
            )?))
        }
        SegmentSource::Clip { media, start, .. } => {
            let video = media.video.as_ref().expect("planned clips are videos");
            let graph = clip_graph(video, plan.width, plan.height, plan.fps, job.tonemap);
            Ok(Source::Clip(Box::new(ClipDecoder::spawn(
                job.tools,
                &media.path,
                *start,
                seconds,
                graph,
                plan.width,
                plan.height,
                job.hardware_decode,
                job.cancel,
            )?)))
        }
    }
}

/// Encode the whole timeline to `job.output`. Returns per-segment decode
/// summaries (index, summary) for clips that needed padding.
pub(crate) fn render_video(
    job: &VideoJob,
    events: EventSink,
) -> Result<Vec<(usize, DecodeSummary)>> {
    let plan = job.plan;
    let (w, h) = (plan.width, plan.height);
    let kbps = video_bitrate_kbps(w, h, plan.fps, job.quality);

    let mut cmd = job.tools.ffmpeg();
    cmd.args(["-f", "rawvideo", "-pixel_format", "yuv420p"])
        .args([
            "-video_size",
            &format!("{w}x{h}"),
            "-framerate",
            &plan.fps.to_string(),
        ])
        .args(["-i", "pipe:0"])
        .input_path(job.audio)
        .args(["-map", "0:v:0", "-map", "1:a:0", "-c:v", &job.encoder.codec])
        .args(video_args(&job.encoder.id, kbps, plan.fps, job.quality))
        .args(["-c:a", "copy"])
        .args([
            "-color_primaries",
            "bt709",
            "-color_trc",
            "bt709",
            "-colorspace",
            "bt709",
            "-color_range",
            "tv",
        ])
        .args(["-tag:v", "avc1", "-movflags", "+faststart"]);
    if let Some(title) = job.title {
        cmd.args(["-metadata", &format!("title={title}")]);
    }
    cmd.args([
        "-metadata",
        &format!(
            "creation_time={}",
            chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S%.6fZ")
        ),
    ])
    .args(["-f", "mp4", "-y"])
    .output_path(job.output);
    let mut encoder = cmd.spawn(Stdio::piped(), Stdio::null())?;
    let mut stdin = encoder.take_stdin().expect("piped stdin");

    // A writer thread lets compositing overlap with encoding.
    let (tx, rx) = sync_channel::<Vec<u8>>(2);
    let writer = std::thread::spawn(move || -> std::io::Result<()> {
        for frame in rx {
            stdin.write_all(&frame)?;
        }
        stdin.flush()
    });

    let result = composite(job, w, h, &tx, events);
    drop(tx);
    let written = writer
        .join()
        .unwrap_or_else(|_| Err(std::io::Error::other("writer panicked")));
    match result {
        Ok(summaries) => {
            // The encoder's own error explains a broken pipe best.
            encoder.wait(job.cancel)?;
            written.map_err(|e| Error::io("sending frames to the encoder", e))?;
            Ok(summaries)
        }
        Err(Error::Cancelled) => {
            encoder.kill();
            Err(Error::Cancelled)
        }
        Err(e) => {
            if matches!(e, Error::Io { .. }) {
                // Probably the encoder died; prefer its message.
                encoder.wait(job.cancel)?;
            }
            encoder.kill();
            Err(e)
        }
    }
}

fn composite(
    job: &VideoJob,
    w: u32,
    h: u32,
    tx: &std::sync::mpsc::SyncSender<Vec<u8>>,
    events: EventSink,
) -> Result<Vec<(usize, DecodeSummary)>> {
    let plan = job.plan;
    let n = plan.segments.len();
    let (fw, fh) = (w as usize, h as usize);
    let total = plan.total_frames;
    let mut done = 0u64;
    let started = Instant::now();
    let mut last_report = Instant::now() - Duration::from_secs(1);
    let mut summaries = Vec::new();

    let send = |mut frame: Vec<u8>,
                done: &mut u64,
                clip: Option<usize>,
                events: &mut dyn FnMut(Event),
                last_report: &mut Instant|
     -> Result<()> {
        job.cancel.check()?;
        if let Some(stamp) = job.stamp {
            stamp.apply(&mut frame, fw, fh);
        }
        tx.send(frame).map_err(|_| {
            Error::io(
                "sending frames to the encoder",
                std::io::ErrorKind::BrokenPipe.into(),
            )
        })?;
        *done += 1;
        if last_report.elapsed() >= Duration::from_millis(200) || *done == total {
            *last_report = Instant::now();
            let elapsed = started.elapsed().as_secs_f64().max(1e-3);
            let fps = *done as f64 / elapsed;
            let (from, to) = Stage::Video.span();
            events(Event::Progress {
                fraction: from + (to - from) * (*done as f64 / total as f64),
                stage: Stage::Video,
                done: *done,
                total,
                fps: Some(fps),
                eta_seconds: (*done > 10).then(|| (total - *done) as f64 / fps),
                clip,
            });
        }
        Ok(())
    };

    let mut current = open(job, 0)?;
    let mut next = if n > 1 { Some(open(job, 1)?) } else { None };
    for i in 0..n {
        let segment = &plan.segments[i];
        let clip = segment.clip_index();
        let lead_in = if i > 0 { plan.joins[i - 1].frames } else { 0 };
        let lead_out = if i + 1 < n { plan.joins[i].frames } else { 0 };
        for _ in 0..segment.frames - lead_in - lead_out {
            let frame = current.next_frame()?;
            send(frame, &mut done, clip, &mut *events, &mut last_report)?;
        }
        if i + 1 == n {
            break;
        }
        let mut incoming = next.take().expect("next segment opened");
        let join = plan.joins[i];
        for k in 0..join.frames {
            let a = current.next_frame()?;
            let b = incoming.next_frame()?;
            let mut out = vec![0u8; frame_len(w, h)];
            let progress = (k as f32 + 0.5) / join.frames as f32;
            blend(join.kind, &a, &b, &mut out, w, h, progress, done);
            send(
                out,
                &mut done,
                plan.segments[i + 1].clip_index(),
                &mut *events,
                &mut last_report,
            )?;
        }
        if let Some(summary) = std::mem::replace(&mut current, incoming).finish() {
            summaries.push((i, summary));
        }
        if i + 2 < n {
            next = Some(open(job, i + 2)?);
        }
    }
    if let Some(summary) = current.finish() {
        summaries.push((n - 1, summary));
    }
    debug_assert_eq!(done, total);
    Ok(summaries)
}
