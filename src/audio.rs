//! Shared soundtrack mixing for hosts that already decoded stereo 48 kHz PCM.
use crate::{
    CancelToken, Error, Result,
    project::{AudioOptions, LoudnessTarget},
};
use serde::{Deserialize, Serialize};
use std::{
    fs::File,
    io::{BufReader, BufWriter, Read, Write},
    path::{Path, PathBuf},
};
const MICRO_FADE: u64 = 384;
#[derive(Debug, Clone, Deserialize)]
pub struct PcmSegment {
    pub pcm: Option<PathBuf>,
    pub frames: u64,
    #[serde(default)]
    pub overlap_frames: u64,
    #[serde(default = "yes")]
    pub level: bool,
}
fn yes() -> bool {
    true
}
/// Gain envelope for sample `local` (per channel) of a segment.
#[derive(Clone, Copy)]
pub(crate) struct Envelope {
    pub(crate) len: u64,
    pub(crate) fade_in: u64,
    pub(crate) fade_out: u64,
    /// Equal-power curve (crossfade) or linear (micro fade).
    pub(crate) cross_in: bool,
    pub(crate) cross_out: bool,
}

impl Envelope {
    pub(crate) fn at(&self, local: u64) -> f32 {
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
pub fn mix_pcm(
    segments: &[PcmSegment],
    fps: u32,
    files: &[Option<PathBuf>],
    gains: &[f32],
    program: &Path,
    cancel: &CancelToken,
) -> Result<()> {
    if !crate::project::FRAME_RATES.contains(&fps)
        || segments.is_empty()
        || files.len() != segments.len()
        || gains.len() != segments.len()
        || gains.iter().any(|g| !g.is_finite())
        || segments.iter().any(|s| s.overlap_frames > s.frames)
    {
        return Err(Error::InvalidProject("invalid PCM mix inputs".into()));
    }
    let spf = u64::from(48_000 / fps);
    let n = segments.len();
    let mut cursor = 0;
    let starts: Vec<u64> = segments
        .iter()
        .map(|s| {
            let start = cursor;
            cursor += (s.frames - s.overlap_frames) * spf;
            start
        })
        .collect();
    let envelopes: Vec<Envelope> = (0..n)
        .map(|i| {
            let len = segments[i].frames * spf;
            let join_in = (i > 0).then(|| segments[i - 1].overlap_frames * spf);
            let join_out = (i + 1 < n).then(|| segments[i].overlap_frames * spf);
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
    let total = cursor;
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
            bytes.fill(0);
            let mut got = 0;
            while got < bytes.len() {
                let read = reader
                    .read(&mut bytes[got..])
                    .map_err(|e| Error::io("reading audio work file", e))?;
                if read == 0 {
                    break;
                }
                got += read;
            }
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

/// Measurements from the shared two-pass PCM normaliser.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LoudnessReport {
    pub input_integrated: f64,
    pub input_true_peak: f64,
    pub input_range: f64,
    pub output_integrated: Option<f64>,
    pub output_true_peak: Option<f64>,
    pub normalization: Option<String>,
}
fn dsp_error(error: impl ToString) -> Error {
    Error::io("PCM soundtrack", std::io::Error::other(error.to_string()))
}
fn meter() -> Result<ebur128::EbuR128> {
    ebur128::EbuR128::new(
        2,
        48_000,
        ebur128::Mode::I | ebur128::Mode::LRA | ebur128::Mode::TRUE_PEAK,
    )
    .map_err(dsp_error)
}
fn read_chunk(reader: &mut impl Read, buffer: &mut [u8]) -> Result<usize> {
    let mut count = 0;
    while count < buffer.len() {
        let n = reader.read(&mut buffer[count..]).map_err(dsp_error)?;
        if n == 0 {
            break;
        }
        count += n;
    }
    if count % 8 != 0 {
        return Err(dsp_error("stereo float PCM ends inside a sample"));
    }
    Ok(count)
}
fn decode_floats(bytes: &[u8]) -> Result<Vec<f32>> {
    bytes
        .as_chunks::<4>()
        .0
        .iter()
        .map(|b| {
            let s = f32::from_le_bytes(*b);
            if s.is_finite() {
                Ok(s)
            } else {
                Err(dsp_error("PCM has non-finite samples"))
            }
        })
        .collect()
}
fn measure_file(path: &Path, cancel: &CancelToken) -> Result<ebur128::EbuR128> {
    let mut state = meter()?;
    let mut reader = BufReader::new(File::open(path).map_err(dsp_error)?);
    let mut bytes = vec![0; 4800 * 8];
    loop {
        cancel.check()?;
        let count = read_chunk(&mut reader, &mut bytes)?;
        if count == 0 {
            break;
        }
        state
            .add_frames_f32(&decode_floats(&bytes[..count])?)
            .map_err(dsp_error)?;
    }
    // Drain the interpolation filter so the last true peak is measured too.
    state.add_frames_f32(&[0.0; 128]).map_err(dsp_error)?;
    Ok(state)
}
fn true_peak_db(m: &ebur128::EbuR128) -> f64 {
    let peak = m
        .true_peak(0)
        .unwrap_or(0.0)
        .max(m.true_peak(1).unwrap_or(0.0));
    if peak > 0.0 {
        20.0 * peak.log10()
    } else {
        f64::NEG_INFINITY
    }
}
/// Per-clip gain clamped to ±12 dB; silence receives unity gain.
pub fn clip_gain(integrated: f64, target: f64) -> f32 {
    if integrated.is_finite() && integrated > -70.0 {
        10f64.powf((target - integrated).clamp(-12.0, 12.0) / 20.0) as f32
    } else {
        1.0
    }
}
fn next_stereo(reader: &mut impl Read) -> Result<Option<[f32; 2]>> {
    let mut bytes = [0; 8];
    if read_chunk(reader, &mut bytes)? == 0 {
        return Ok(None);
    }
    let values = decode_floats(&bytes)?;
    Ok(Some([values[0], values[1]]))
}
/// Apply gain with a stereo-linked 5 ms lookahead limiter and 50 ms release.
fn gain_limit(
    input: &Path,
    output: &Path,
    gain: f32,
    ceiling: f32,
    dynamic: bool,
    cancel: &CancelToken,
) -> Result<()> {
    use std::collections::VecDeque;
    let mut reader = BufReader::new(File::open(input).map_err(dsp_error)?);
    let mut writer = BufWriter::new(File::create(output).map_err(dsp_error)?);
    let mut frames = VecDeque::new();
    let mut peaks: VecDeque<(u64, f32)> = VecDeque::new();
    let mut index = 0u64;
    let mut loaded = 0u64;
    let mut attenuation = 1.0f32;
    let lookahead = if dynamic { 240 } else { 0 };
    let mut load = |reader: &mut BufReader<File>,
                    frames: &mut VecDeque<[f32; 2]>,
                    peaks: &mut VecDeque<(u64, f32)>|
     -> Result<bool> {
        let Some(frame) = next_stereo(reader)? else {
            return Ok(false);
        };
        let frame = [frame[0] * gain, frame[1] * gain];
        let peak = frame[0].abs().max(frame[1].abs());
        while peaks.back().is_some_and(|p| p.1 <= peak) {
            peaks.pop_back();
        }
        peaks.push_back((loaded, peak));
        loaded += 1;
        frames.push_back(frame);
        Ok(true)
    };
    for _ in 0..=lookahead {
        if !load(&mut reader, &mut frames, &mut peaks)? {
            break;
        }
    }
    while let Some(frame) = frames.pop_front() {
        if index.is_multiple_of(4800) {
            cancel.check()?;
        }
        let peak = peaks.front().map_or(0.0, |p| p.1);
        let desired = if peak > ceiling { ceiling / peak } else { 1.0 };
        attenuation = desired.min(attenuation + (1.0 - attenuation) * 0.00041658);
        write_f32(
            &mut writer,
            &[frame[0] * attenuation, frame[1] * attenuation],
        )?;
        if peaks.front().is_some_and(|p| p.0 == index) {
            peaks.pop_front();
        }
        index += 1;
        load(&mut reader, &mut frames, &mut peaks)?;
    }
    writer.flush().map_err(dsp_error)
}
fn wav_header(out: &mut impl Write, bytes: u64) -> Result<()> {
    let large = bytes > u64::from(u32::MAX) - 36;
    if large {
        out.write_all(b"RF64").map_err(dsp_error)?;
        out.write_all(&u32::MAX.to_le_bytes()).map_err(dsp_error)?;
        out.write_all(b"WAVEds64").map_err(dsp_error)?;
        out.write_all(&28u32.to_le_bytes()).map_err(dsp_error)?;
        for n in [bytes + 72, bytes, bytes / 8] {
            out.write_all(&n.to_le_bytes()).map_err(dsp_error)?;
        }
        out.write_all(&0u32.to_le_bytes()).map_err(dsp_error)?;
        out.write_all(b"fmt ").map_err(dsp_error)?;
    } else {
        out.write_all(b"RIFF").map_err(dsp_error)?;
        out.write_all(&((bytes + 36) as u32).to_le_bytes())
            .map_err(dsp_error)?;
        out.write_all(b"WAVEfmt ").map_err(dsp_error)?;
    }

    out.write_all(&16u32.to_le_bytes()).map_err(dsp_error)?;
    out.write_all(&3u16.to_le_bytes()).map_err(dsp_error)?;
    out.write_all(&2u16.to_le_bytes()).map_err(dsp_error)?;
    out.write_all(&48000u32.to_le_bytes()).map_err(dsp_error)?;
    out.write_all(&384000u32.to_le_bytes()).map_err(dsp_error)?;
    out.write_all(&8u16.to_le_bytes()).map_err(dsp_error)?;
    out.write_all(&32u16.to_le_bytes()).map_err(dsp_error)?;
    out.write_all(b"data").map_err(dsp_error)?;
    out.write_all(&(if large { u32::MAX } else { bytes as u32 }).to_le_bytes())
        .map_err(dsp_error)
}
/// Measure the mix, normalise it, verify the true peak and write float WAV.
/// The same implementation is used by process-based and in-process hosts.
pub fn normalize_pcm(
    input: &Path,
    output: &Path,
    target: Option<LoudnessTarget>,
    work: &Path,
    cancel: &CancelToken,
) -> Result<LoudnessReport> {
    if target.is_some_and(|t| {
        !(-70.0..=-5.0).contains(&t.integrated) || !(-9.0..=0.0).contains(&t.true_peak)
    }) {
        return Err(Error::InvalidProject("invalid loudness target".into()));
    }
    let before = measure_file(input, cancel)?;
    let integrated = before.loudness_global().unwrap_or(f64::NEG_INFINITY);
    let tp = true_peak_db(&before);
    let gain = target.filter(|_| integrated.is_finite()).map_or(1.0, |t| {
        10f64.powf((t.integrated - integrated) / 20.0) as f32
    });
    let ceiling_db = target.map_or(-0.2, |t| t.true_peak);
    let dynamic = tp + 20.0 * f64::from(gain).log10() > ceiling_db;
    let limited = work.join("normalised.pcm");
    gain_limit(
        input,
        &limited,
        gain,
        10f32.powf((ceiling_db as f32 - 0.1) / 20.0),
        dynamic,
        cancel,
    )?;
    let after = measure_file(&limited, cancel)?;
    let correction = (ceiling_db - true_peak_db(&after) - 0.02).min(0.0);
    let correction = 10f64.powf(correction / 20.0) as f32;
    let length = File::open(&limited)
        .map_err(dsp_error)?
        .metadata()
        .map_err(dsp_error)?
        .len();
    if let Some(parent) = output.parent() {
        std::fs::create_dir_all(parent).map_err(dsp_error)?;
    }
    let mut temp = tempfile::NamedTempFile::new_in(output.parent().unwrap_or(Path::new(".")))
        .map_err(dsp_error)?;
    wav_header(temp.as_file_mut(), length)?;
    let mut reader = BufReader::new(File::open(&limited).map_err(dsp_error)?);
    let mut bytes = vec![0; 4800 * 8];
    let mut verify = meter()?;
    loop {
        cancel.check()?;
        let count = read_chunk(&mut reader, &mut bytes)?;
        if count == 0 {
            break;
        }
        let mut samples = decode_floats(&bytes[..count])?;
        for s in &mut samples {
            *s *= correction;
        }
        verify.add_frames_f32(&samples).map_err(dsp_error)?;
        write_f32(temp.as_file_mut(), &samples)?;
    }
    verify.add_frames_f32(&[0.0; 128]).map_err(dsp_error)?;
    let output_peak = true_peak_db(&verify);
    if output_peak.is_finite() && output_peak > ceiling_db + 0.01 {
        return Err(Error::OutputInvalid(
            "soundtrack exceeded the true peak target".into(),
        ));
    }
    temp.as_file().sync_all().map_err(dsp_error)?;
    temp.persist(output).map_err(|e| dsp_error(e.error))?;
    Ok(LoudnessReport {
        input_integrated: integrated,
        input_true_peak: tp,
        input_range: before.loudness_range().unwrap_or(0.0),
        output_integrated: verify.loudness_global().ok().filter(|v| v.is_finite()),
        output_true_peak: output_peak.is_finite().then_some(output_peak),
        normalization: Some(if dynamic { "dynamic" } else { "linear" }.into()),
    })
}
/// Mix already decoded source audio to a frame-exact stereo float WAV.
pub fn render_pcm(
    segments: &[PcmSegment],
    fps: u32,
    options: &AudioOptions,
    work: &Path,
    output: &Path,
    cancel: &CancelToken,
) -> Result<LoudnessReport> {
    if segments.is_empty() || !crate::project::FRAME_RATES.contains(&fps) {
        return Err(Error::InvalidProject(
            "PCM mix needs segments and a supported frame rate".into(),
        ));
    }
    for (i, s) in segments.iter().enumerate() {
        let max_next = segments
            .get(i + 1)
            .map_or(0, |n| (n.frames as f64 * 0.4).round() as u64);
        if s.frames == 0
            || s.overlap_frames > (s.frames as f64 * 0.4).round() as u64
            || s.overlap_frames > max_next
        {
            return Err(Error::InvalidProject(
                "PCM overlap exceeds the shared timeline bounds".into(),
            ));
        }
    }
    std::fs::create_dir_all(work).map_err(dsp_error)?;
    let reference = options.loudness.map_or(-23.0, |t| t.integrated);
    let mut gains = Vec::new();
    for s in segments {
        let measured = if options.level_clips && s.level {
            s.pcm
                .as_ref()
                .map(|p| measure_file(p, cancel))
                .transpose()?
                .and_then(|m| m.loudness_global().ok())
        } else {
            None
        };
        gains.push(measured.map_or(1.0, |l| clip_gain(l, reference)));
    }
    let files: Vec<_> = segments.iter().map(|s| s.pcm.clone()).collect();
    let program = work.join("program.pcm");
    mix_pcm(segments, fps, &files, &gains, &program, cancel)?;
    normalize_pcm(&program, output, options.loudness, work, cancel)
}
pub(crate) fn write_f32(out: &mut impl Write, samples: &[f32]) -> Result<()> {
    let mut bytes = Vec::with_capacity(samples.len() * 4);
    for s in samples {
        bytes.extend_from_slice(&s.to_le_bytes());
    }
    out.write_all(&bytes)
        .map_err(|e| Error::io("writing audio work file", e))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn decoded_soundtrack_is_frame_exact_and_normalised() {
        let directory = tempfile::tempdir().unwrap();
        let pcm = directory.path().join("clip.pcm");
        let mut file = File::create(&pcm).unwrap();
        for index in 0..144000 {
            let s = 0.05 * (2.0 * std::f32::consts::PI * 997.0 * index as f32 / 48000.0).sin();
            write_f32(&mut file, &[s, s]).unwrap();
        }
        let output = directory.path().join("movie.wav");
        let segments = [
            PcmSegment {
                pcm: Some(pcm),
                frames: 90,
                overlap_frames: 0,
                level: true,
            },
            PcmSegment {
                pcm: None,
                frames: 30,
                overlap_frames: 0,
                level: true,
            },
        ];
        let report = render_pcm(
            &segments,
            30,
            &AudioOptions::default(),
            directory.path(),
            &output,
            &CancelToken::new(),
        )
        .unwrap();
        assert_eq!(std::fs::metadata(output).unwrap().len(), 44 + 192000 * 8);
        assert!((report.output_integrated.unwrap() + 23.0).abs() < 0.1);
        assert!(report.output_true_peak.unwrap() <= -1.0);
    }
    #[test]
    fn gain_and_bounds_are_shared() {
        assert!((clip_gain(-40.0, -23.0) - 10f32.powf(12.0 / 20.0)).abs() < 0.0001);
        assert_eq!(clip_gain(f64::NEG_INFINITY, -23.0), 1.0);
        let d = tempfile::tempdir().unwrap();
        let bad = [PcmSegment {
            pcm: None,
            frames: 30,
            overlap_frames: 1,
            level: true,
        }];
        assert!(
            render_pcm(
                &bad,
                30,
                &AudioOptions::default(),
                d.path(),
                &d.path().join("o.wav"),
                &CancelToken::new()
            )
            .is_err()
        );
        assert!(
            mix_pcm(
                &[],
                0,
                &[],
                &[],
                &d.path().join("o.pcm"),
                &CancelToken::new()
            )
            .is_err()
        );
    }
}
