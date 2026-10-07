//! The soundtrack: per-clip decoding and levelling, crossfaded mixing, and
//! two-pass EBU R128 loudness normalisation.

use crate::encoders::audio_args;
use crate::error::{Error, Result};
use crate::events::{CancelToken, Event, EventSink, Stage};
use crate::plan::{RenderPlan, SegmentSource};
use crate::project::AudioOptions;
use crate::render::decode::decode_audio;
use crate::tools::FfmpegTools;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

/// Clips are levelled towards this loudness when no target is set.
const REFERENCE_LUFS: f64 = -23.0;
pub use crate::audio::LoudnessReport;

pub(crate) struct AudioJob<'a> {
    pub tools: &'a FfmpegTools,
    pub plan: &'a RenderPlan,
    pub options: &'a AudioOptions,
    pub intro_sound: Option<&'a Path>,
    pub work: &'a Path,
    pub codec: &'a str,
    pub cancel: &'a CancelToken,
}

/// Build `audio.m4a` for the whole timeline.
pub(crate) fn render_audio(
    job: &AudioJob,
    events: EventSink,
) -> Result<(PathBuf, Option<LoudnessReport>)> {
    let plan = job.plan;
    let spf = plan.samples_per_frame();
    let n = plan.segments.len();
    let reference = job
        .options
        .loudness
        .map_or(REFERENCE_LUFS, |t| t.integrated);

    // 1. Decode every segment to raw PCM, measuring each clip's loudness.
    let mut files: Vec<Option<PathBuf>> = Vec::with_capacity(n);
    let mut gains = Vec::with_capacity(n);
    for (i, segment) in plan.segments.iter().enumerate() {
        job.cancel.check()?;
        events(progress(
            Stage::Audio,
            i as u64,
            n as u64,
            segment.clip_index(),
        ));
        let samples = segment.frames * spf;
        let seconds = segment.frames as f64 / f64::from(plan.fps);
        let source = match &segment.source {
            SegmentSource::Clip { media, start, .. } => media
                .audio
                .as_ref()
                .map(|a| (media.path.clone(), Some(a.stream_index), *start, false)),
            SegmentSource::Intro => job.intro_sound.map(|p| (p.to_path_buf(), None, 0.0, true)),
        };
        let Some((path, stream, start, by_path)) = source else {
            files.push(None);
            gains.push(1.0);
            continue;
        };
        let pcm = job.work.join(format!("segment-{i}.pcm"));
        let mut out = BufWriter::new(
            File::create(&pcm).map_err(|e| Error::io("creating audio work file", e))?,
        );
        let measure = job.options.level_clips && segment.clip_index().is_some();
        let mut meter = measure
            .then(|| ebur128::EbuR128::new(2, 48_000, ebur128::Mode::I))
            .transpose()
            .map_err(|e| Error::io("loudness meter", std::io::Error::other(e.to_string())))?;
        let got = decode_audio(
            job.tools,
            &path,
            stream,
            start,
            seconds,
            samples,
            by_path,
            job.cancel,
            |chunk| {
                if let Some(m) = meter.as_mut() {
                    let _ = m.add_frames_f32(chunk);
                }
                crate::audio::write_f32(&mut out, chunk)
            },
        )?;
        out.flush()
            .map_err(|e| Error::io("writing audio work file", e))?;
        let gain = match meter.and_then(|m| m.loudness_global().ok()) {
            Some(lufs) if got => crate::audio::clip_gain(lufs, reference),
            _ => 1.0,
        };
        files.push(Some(pcm));
        gains.push(gain);
    }
    events(progress(Stage::Audio, n as u64, n as u64, None));

    // 2. Mix the timeline with equal-power crossfades.
    let program = job.work.join("program.pcm");
    let segments: Vec<_> = plan
        .segments
        .iter()
        .enumerate()
        .map(|(i, s)| crate::audio::PcmSegment {
            pcm: files[i].clone(),
            frames: s.frames,
            overlap_frames: plan.joins.get(i).map_or(0, |j| j.frames),
            level: s.clip_index().is_some(),
        })
        .collect();
    crate::audio::mix_pcm(&segments, plan.fps, &files, &gains, &program, job.cancel)?;
    for file in files.into_iter().flatten() {
        let _ = std::fs::remove_file(file);
    }

    // 3 + 4. EBU R128: measure, then normalise linearly while encoding.
    events(Event::Stage {
        stage: Stage::Loudness,
    });
    let output = job.work.join("audio.m4a");
    let wav = job.work.join("normalised.wav");
    let shared =
        crate::audio::normalize_pcm(&program, &wav, job.options.loudness, job.work, job.cancel)?;
    events(progress(Stage::Loudness, 1, 2, None));
    let mut cmd = job.tools.ffmpeg();
    cmd.input_path(&wav)
        .args(audio_args(job.codec))
        .args(["-ar", "48000", "-ac", "2", "-y"])
        .output_path(&output);
    cmd.run(job.cancel)?;
    let report = job
        .options
        .loudness
        .and_then(|_| shared.input_integrated.is_finite().then_some(shared));
    let _ = std::fs::remove_file(wav);
    events(progress(Stage::Loudness, 2, 2, None));
    let _ = std::fs::remove_file(&program);
    Ok((output, report))
}

fn progress(stage: Stage, done: u64, total: u64, clip: Option<usize>) -> Event {
    let (from, to) = stage.span();
    let share = if total == 0 {
        1.0
    } else {
        done as f64 / total as f64
    };
    Event::Progress {
        fraction: from + (to - from) * share,
        stage,
        done,
        total,
        fps: None,
        eta_seconds: None,
        clip,
    }
}

#[cfg(test)]
mod tests {
    use crate::audio::Envelope;

    #[test]
    fn envelopes() {
        let cross = Envelope {
            len: 100,
            fade_in: 10,
            fade_out: 10,
            cross_in: true,
            cross_out: true,
        };
        assert!(cross.at(0) < 0.1);
        assert_eq!(cross.at(50), 1.0);
        assert!(cross.at(99) < 0.1);
        // Equal power: the two sides of a crossfade sum to unit power.
        let x = cross.at(4);
        let y = Envelope {
            len: 10,
            fade_in: 0,
            fade_out: 10,
            cross_in: false,
            cross_out: true,
        }
        .at(4);
        assert!((x * x + y * y - 1.0).abs() < 1e-5);
        let cut = Envelope {
            len: 1000,
            fade_in: 384,
            fade_out: 0,
            cross_in: false,
            cross_out: false,
        };
        assert!(cut.at(0) < 0.01 && cut.at(383) > 0.99 && cut.at(999) == 1.0);
    }
}
