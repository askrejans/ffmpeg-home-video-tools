//! The soundtrack: per-clip decoding and levelling, crossfaded mixing, and
//! two-pass EBU R128 loudness normalisation.

use crate::encoders::audio_args;
use crate::error::{Error, Result};
use crate::events::{CancelToken, Event, EventSink, Stage};
use crate::plan::{RenderPlan, SegmentSource};
use crate::project::{AudioOptions, LoudnessTarget};
use crate::render::decode::decode_audio;
use crate::tools::FfmpegTools;
use serde::Serialize;
use std::collections::BTreeMap;
use std::fs::File;
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};

/// Clips are levelled towards this loudness when no target is set.
const REFERENCE_LUFS: f64 = -23.0;
/// Largest gain change applied to a single clip.
const MAX_CLIP_GAIN_DB: f64 = 12.0;
/// Fade applied at hard cuts and at the very end, to avoid clicks.
const MICRO_FADE: u64 = 384;

/// Result of the EBU R128 normalisation.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LoudnessReport {
    /// Measured before normalisation (pass 1).
    pub input_integrated: f64,
    pub input_true_peak: f64,
    pub input_range: f64,
    /// Reported by the normalising pass.
    pub output_integrated: Option<f64>,
    pub output_true_peak: Option<f64>,
    /// `linear` (pure gain) or `dynamic` (gain plus limiting).
    pub normalization: Option<String>,
}

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
                write_f32(&mut out, chunk)
            },
        )?;
        out.flush()
            .map_err(|e| Error::io("writing audio work file", e))?;
        let gain_db = match meter.and_then(|m| m.loudness_global().ok()) {
            Some(lufs) if got && lufs.is_finite() && lufs > -70.0 => {
                (reference - lufs).clamp(-MAX_CLIP_GAIN_DB, MAX_CLIP_GAIN_DB)
            }
            _ => 0.0,
        };
        files.push(Some(pcm));
        gains.push(10f64.powf(gain_db / 20.0) as f32);
    }
    events(progress(Stage::Audio, n as u64, n as u64, None));

    // 2. Mix the timeline with equal-power crossfades.
    let program = job.work.join("program.pcm");
    mix(plan, &files, &gains, &program, job.cancel)?;
    for file in files.into_iter().flatten() {
        let _ = std::fs::remove_file(file);
    }

    // 3 + 4. EBU R128: measure, then normalise linearly while encoding.
    events(Event::Stage {
        stage: Stage::Loudness,
    });
    let output = job.work.join("audio.m4a");
    let report = match job.options.loudness {
        Some(target) => {
            let measured = measure_loudness(job, &program, &target)?;
            events(progress(Stage::Loudness, 1, 2, None));
            encode(
                job,
                &program,
                &output,
                measured.as_ref().map(|m| (m, &target)),
            )?
        }
        None => encode(job, &program, &output, None)?,
    };
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

fn write_f32(out: &mut impl Write, samples: &[f32]) -> Result<()> {
    let mut bytes = Vec::with_capacity(samples.len() * 4);
    for s in samples {
        bytes.extend_from_slice(&s.to_le_bytes());
    }
    out.write_all(&bytes)
        .map_err(|e| Error::io("writing audio work file", e))
}

/// Gain envelope for sample `local` (per channel) of a segment.
#[derive(Clone, Copy)]
struct Envelope {
    len: u64,
    fade_in: u64,
    fade_out: u64,
    /// Equal-power curve (crossfade) or linear (micro fade).
    cross_in: bool,
    cross_out: bool,
}

impl Envelope {
    fn at(&self, local: u64) -> f32 {
        let mut g = 1.0f32;
        if local < self.fade_in {
            let x = (local as f32 + 0.5) / self.fade_in as f32;
            g *= if self.cross_in {
                (x * std::f32::consts::FRAC_PI_2).sin()
            } else {
                x
            };
        }
        let tail_start = self.len.saturating_sub(self.fade_out);
        if self.fade_out > 0 && local >= tail_start {
            let x = (local - tail_start) as f32 + 0.5;
            let x = x / self.fade_out as f32;
            g *= if self.cross_out {
                (x * std::f32::consts::FRAC_PI_2).cos()
            } else {
                1.0 - x
            };
        }
        g
    }
}

/// Write the whole timeline as interleaved stereo `f32` to `program`.
fn mix(
    plan: &RenderPlan,
    files: &[Option<PathBuf>],
    gains: &[f32],
    program: &Path,
    cancel: &CancelToken,
) -> Result<()> {
    let spf = plan.samples_per_frame();
    let n = plan.segments.len();
    let starts: Vec<u64> = (0..n).map(|i| plan.segment_start(i) * spf).collect();
    let envelopes: Vec<Envelope> = (0..n)
        .map(|i| {
            let len = plan.segments[i].frames * spf;
            let join_in = (i > 0).then(|| plan.joins[i - 1].frames * spf);
            let join_out = (i + 1 < n).then(|| plan.joins[i].frames * spf);
            let micro = MICRO_FADE.min(len / 4);
            let (fade_in, cross_in) = match join_in {
                Some(0) => (micro, false),
                Some(j) => (j, true),
                None => (0, false),
            };
            let (fade_out, cross_out) = match join_out {
                Some(0) | None => (micro, false),
                Some(j) => (j, true),
            };
            Envelope {
                len,
                fade_in,
                fade_out,
                cross_in,
                cross_out,
            }
        })
        .collect();

    let mut readers: Vec<Option<BufReader<File>>> = files
        .iter()
        .map(|f| {
            f.as_ref()
                .map(File::open)
                .transpose()
                .map(|f| f.map(BufReader::new))
        })
        .collect::<std::io::Result<_>>()
        .map_err(|e| Error::io("reading audio work file", e))?;
    let mut out =
        BufWriter::new(File::create(program).map_err(|e| Error::io("creating soundtrack", e))?);
    let total = plan.total_samples();
    const CHUNK: u64 = 4_800;
    let mut chunk = vec![0f32; CHUNK as usize * 2];
    let mut raw = vec![0u8; CHUNK as usize * 2 * 4];
    let mut first_active = 0usize;
    let mut position = 0u64;
    while position < total {
        cancel.check()?;
        let len = CHUNK.min(total - position);
        let buf = &mut chunk[..len as usize * 2];
        buf.fill(0.0);
        while first_active < n && starts[first_active] + envelopes[first_active].len <= position {
            readers[first_active] = None;
            first_active += 1;
        }
        for i in first_active..n {
            if starts[i] >= position + len {
                break;
            }
            let from = position.max(starts[i]);
            let to = (position + len).min(starts[i] + envelopes[i].len);
            if from >= to {
                continue;
            }
            let Some(reader) = readers[i].as_mut() else {
                continue;
            };
            let count = (to - from) as usize;
            let bytes = &mut raw[..count * 8];
            reader
                .read_exact(bytes)
                .map_err(|e| Error::io("reading audio work file", e))?;
            let offset = (from - position) as usize;
            for k in 0..count {
                let local = from - starts[i] + k as u64;
                let g = gains[i] * envelopes[i].at(local);
                for c in 0..2 {
                    let b = &bytes[(k * 2 + c) * 4..][..4];
                    buf[(offset + k) * 2 + c] += f32::from_le_bytes([b[0], b[1], b[2], b[3]]) * g;
                }
            }
        }
        write_f32(&mut out, buf)?;
        position += len;
    }
    out.flush().map_err(|e| Error::io("writing soundtrack", e))
}

/// Values measured by `loudnorm`'s first pass.
#[derive(Debug, Clone)]
struct Measured {
    integrated: f64,
    true_peak: f64,
    range: f64,
    threshold: f64,
    offset: f64,
}

/// Pull the JSON block `loudnorm` prints at the end of a run.
fn parse_loudnorm(stderr: &str) -> Option<BTreeMap<String, String>> {
    let end = stderr.rfind('}')?;
    let start = stderr[..end].rfind('{')?;
    serde_json::from_str(&stderr[start..=end]).ok()
}

fn number(map: &BTreeMap<String, String>, key: &str) -> Option<f64> {
    map.get(key)?
        .trim()
        .parse::<f64>()
        .ok()
        .filter(|v| v.is_finite())
}

fn pcm_input(cmd: &mut crate::tools::ToolCommand, program: &Path) {
    cmd.args(["-f", "f32le", "-ar", "48000", "-ac", "2"])
        .input_path(program);
}

/// EBU R128 pass 1. `None` when the soundtrack is silent.
fn measure_loudness(
    job: &AudioJob,
    program: &Path,
    target: &LoudnessTarget,
) -> Result<Option<Measured>> {
    let mut cmd = job.tools.ffmpeg();
    cmd.args(["-loglevel", "info"]);
    pcm_input(&mut cmd, program);
    cmd.args([
        "-af",
        &format!(
            "loudnorm=I={:.1}:TP={:.1}:LRA=11:print_format=json",
            target.integrated, target.true_peak
        ),
        "-f",
        "null",
        "-",
    ]);
    let (_, stderr) = cmd.output_with_stderr(job.cancel)?;
    let map = parse_loudnorm(&stderr).ok_or_else(|| Error::ToolFailed {
        tool: "ffmpeg",
        status: None,
        stderr: "loudness measurement produced no result".into(),
    })?;
    Ok((|| {
        Some(Measured {
            integrated: number(&map, "input_i")?,
            true_peak: number(&map, "input_tp")?,
            range: number(&map, "input_lra")?,
            threshold: number(&map, "input_thresh")?,
            offset: number(&map, "target_offset")?,
        })
    })())
}

/// Encode the soundtrack, applying EBU R128 pass 2 when measured.
fn encode(
    job: &AudioJob,
    program: &Path,
    output: &Path,
    normalise: Option<(&Measured, &LoudnessTarget)>,
) -> Result<Option<LoudnessReport>> {
    let mut cmd = job.tools.ffmpeg();
    cmd.args(["-loglevel", "info"]);
    pcm_input(&mut cmd, program);
    let filter = match normalise {
        Some((m, t)) => format!(
            "loudnorm=I={:.1}:TP={:.1}:LRA={:.1}:measured_I={:.2}:measured_TP={:.2}:measured_LRA={:.2}:\
             measured_thresh={:.2}:offset={:.2}:linear=true:print_format=json,aresample=48000",
            t.integrated,
            t.true_peak,
            m.range.clamp(7.0, 20.0),
            m.integrated,
            m.true_peak,
            m.range,
            m.threshold,
            m.offset
        ),
        // Without normalisation, only guard against clipping from clip gains.
        None => "alimiter=limit=0.977:level=false".to_string(),
    };
    cmd.args(["-af", &filter])
        .args(audio_args(job.codec))
        .args(["-ar", "48000", "-ac", "2", "-y"])
        .output_path(output);
    let (_, stderr) = cmd.output_with_stderr(job.cancel)?;
    Ok(normalise.map(|(m, _)| {
        let pass2 = parse_loudnorm(&stderr).unwrap_or_default();
        LoudnessReport {
            input_integrated: m.integrated,
            input_true_peak: m.true_peak,
            input_range: m.range,
            output_integrated: number(&pass2, "output_i"),
            output_true_peak: number(&pass2, "output_tp"),
            normalization: pass2.get("normalization_type").cloned(),
        }
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn parses_loudnorm_json() {
        let stderr = "[Parsed_loudnorm_0 @ 0x1]\n{\n\t\"input_i\" : \"-27.61\",\n\t\"input_tp\" : \"-4.47\",\n\
                      \t\"input_lra\" : \"8.06\",\n\t\"input_thresh\" : \"-38.20\",\n\t\"target_offset\" : \"0.58\",\n\
                      \t\"normalization_type\" : \"linear\"\n}\n";
        let map = parse_loudnorm(stderr).unwrap();
        assert_eq!(number(&map, "input_i"), Some(-27.61));
        assert_eq!(map["normalization_type"], "linear");
        let silent = parse_loudnorm("{\"input_i\" : \"-inf\"}").unwrap();
        assert_eq!(number(&silent, "input_i"), None);
    }
}
