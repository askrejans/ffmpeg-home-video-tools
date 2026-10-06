//! Decoding clips into raw canvas-sized frames and float audio.

use crate::error::{Error, Result};
use crate::events::CancelToken;
use crate::frame::{BLACK, frame_len, solid};
use crate::render::filters::AUDIO_CHAIN;
use crate::tools::{FfmpegTools, RunningTool};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::mpsc::{Receiver, sync_channel};
use std::thread::JoinHandle;

/// Extra input read beyond the planned length, so rounding never starves us.
const READ_MARGIN: f64 = 0.5;

struct Process {
    tool: RunningTool,
    rx: Receiver<std::io::Result<Vec<u8>>>,
    reader: Option<JoinHandle<()>>,
}

/// What happened while decoding a clip.
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct DecodeSummary {
    /// Frames the decoder actually produced.
    pub received: u64,
    /// Frames repeated because the clip ended early.
    pub padded: u64,
    /// The decoder reported an error after producing frames.
    pub truncated: bool,
}

/// Streams canvas-sized `yuv420p` frames of one clip segment.
///
/// Always delivers as many frames as asked for: if the decoder ends early,
/// the last frame is repeated.
pub(crate) struct ClipDecoder {
    tools: FfmpegTools,
    path: PathBuf,
    start: f64,
    seconds: f64,
    graph: String,
    width: u32,
    height: u32,
    hardware: bool,
    cancel: CancelToken,
    process: Option<Process>,
    pending: Option<Vec<u8>>,
    last: Option<Vec<u8>>,
    started: bool,
    exhausted: bool,
    summary: DecodeSummary,
}

impl ClipDecoder {
    #[allow(clippy::too_many_arguments)]
    pub fn spawn(
        tools: &FfmpegTools,
        path: &Path,
        start: f64,
        seconds: f64,
        graph: String,
        width: u32,
        height: u32,
        hardware: bool,
        cancel: &CancelToken,
    ) -> Result<Self> {
        let mut decoder = Self {
            tools: tools.clone(),
            path: path.to_path_buf(),
            start,
            seconds,
            graph,
            width,
            height,
            hardware,
            cancel: cancel.clone(),
            process: None,
            pending: None,
            last: None,
            started: false,
            exhausted: false,
            summary: DecodeSummary::default(),
        };
        decoder.process = Some(decoder.start_process()?);
        Ok(decoder)
    }

    fn start_process(&self) -> Result<Process> {
        let mut cmd = self.tools.ffmpeg();
        if self.hardware {
            cmd.args(["-hwaccel", "auto"]);
        }
        cmd.args([
            "-ss",
            &format!("{:.6}", self.start),
            "-t",
            &format!("{:.6}", self.seconds + READ_MARGIN),
        ]);
        cmd.input(&self.path)?;
        cmd.args([
            "-filter_complex",
            &self.graph,
            "-map",
            "[v]",
            "-an",
            "-sn",
            "-dn",
        ])
        .args(["-f", "rawvideo", "-pix_fmt", "yuv420p", "pipe:1"]);
        let mut tool = cmd.spawn(Stdio::null(), Stdio::piped())?;
        let mut stdout = tool.take_stdout().expect("piped stdout");
        let len = frame_len(self.width, self.height);
        let (tx, rx) = sync_channel(2);
        let reader = std::thread::spawn(move || {
            loop {
                let mut buf = vec![0u8; len];
                match stdout.read_exact(&mut buf) {
                    Ok(()) => {
                        if tx.send(Ok(buf)).is_err() {
                            break;
                        }
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
                    Err(e) => {
                        let _ = tx.send(Err(e));
                        break;
                    }
                }
            }
        });
        Ok(Process {
            tool,
            rx,
            reader: Some(reader),
        })
    }

    /// Next decoded frame, or `None` once the decoder is done.
    fn receive(&mut self) -> Result<Option<Vec<u8>>> {
        if self.exhausted {
            return Ok(None);
        }
        let process = self.process.as_mut().expect("running");
        match process.rx.recv() {
            Ok(Ok(frame)) => {
                self.summary.received += 1;
                Ok(Some(frame))
            }
            Ok(Err(e)) => Err(Error::io(format!("reading {}", self.path.display()), e)),
            Err(_) => {
                self.exhausted = true;
                self.end_of_stream()?;
                Ok(None)
            }
        }
    }

    /// The decoder closed its output: find out whether it failed.
    fn end_of_stream(&mut self) -> Result<()> {
        let mut process = self.process.take().expect("running");
        if let Some(reader) = process.reader.take() {
            let _ = reader.join();
        }
        let result = process.tool.wait(&self.cancel);
        match result {
            Ok(()) => Ok(()),
            Err(Error::Cancelled) => Err(Error::Cancelled),
            Err(e) if self.summary.received == 0 && self.hardware => {
                tracing::warn!(path = %self.path.display(), error = %e, "hardware decoding failed, retrying in software");
                self.hardware = false;
                self.exhausted = false;
                self.process = Some(self.start_process()?);
                Ok(())
            }
            Err(e) if self.summary.received == 0 => Err(match e {
                Error::ToolFailed { stderr, .. } => Error::Unreadable {
                    path: self.path.clone(),
                    reason: stderr,
                },
                other => other,
            }),
            Err(_) => {
                self.summary.truncated = true;
                Ok(())
            }
        }
    }

    pub fn next_frame(&mut self) -> Result<Vec<u8>> {
        if !self.started {
            self.started = true;
            self.pending = self.receive_retrying()?;
        }
        match self.pending.take() {
            Some(current) => {
                self.pending = self.receive()?;
                if self.pending.is_none() {
                    // That was the decoder's last frame; keep a copy in case
                    // more frames are needed.
                    self.last = Some(current.clone());
                }
                Ok(current)
            }
            None => {
                self.summary.padded += 1;
                Ok(self
                    .last
                    .clone()
                    .unwrap_or_else(|| solid(self.width, self.height, BLACK)))
            }
        }
    }

    /// `receive`, but retry once in software when the first attempt produced
    /// nothing.
    fn receive_retrying(&mut self) -> Result<Option<Vec<u8>>> {
        let first = self.receive()?;
        if first.is_none() && !self.exhausted && self.process.is_some() {
            return self.receive();
        }
        Ok(first)
    }

    /// Stop decoding (killing the process if it is still producing frames).
    pub fn finish(mut self) -> DecodeSummary {
        if let Some(mut process) = self.process.take() {
            process.tool.kill();
            drop(process.rx);
            if let Some(reader) = process.reader.take() {
                let _ = reader.join();
            }
        }
        self.summary
    }
}

/// Decode `seconds` of a clip's audio stream (`stream` = absolute index, or
/// `None` for the first audio stream) as 48 kHz stereo `f32` samples, exactly
/// `samples` per channel (padded with silence or truncated).
#[allow(clippy::too_many_arguments)]
pub(crate) fn decode_audio(
    tools: &FfmpegTools,
    path: &Path,
    stream: Option<usize>,
    start: f64,
    seconds: f64,
    samples: u64,
    by_path: bool,
    cancel: &CancelToken,
    mut sink: impl FnMut(&[f32]) -> Result<()>,
) -> Result<bool> {
    let mut cmd = tools.ffmpeg();
    cmd.args([
        "-ss",
        &format!("{start:.6}"),
        "-t",
        &format!("{:.6}", seconds + READ_MARGIN),
    ]);
    if by_path {
        cmd.input_path(path);
    } else {
        cmd.input(path)?;
    }
    let map = stream.map_or_else(|| "0:a:0".to_string(), |s| format!("0:{s}"));
    cmd.args(["-map", &map, "-vn", "-sn", "-dn", "-af", AUDIO_CHAIN])
        .args(["-f", "f32le", "-ac", "2", "-ar", "48000", "pipe:1"]);
    let mut tool = cmd.spawn(Stdio::null(), Stdio::piped())?;
    let mut stdout = tool.take_stdout().expect("piped stdout");
    let wanted = samples as usize * 2;
    let mut produced = 0usize;
    let mut bytes = vec![0u8; 48_000 * 2 * 4];
    let mut floats = Vec::with_capacity(48_000 * 2);
    let mut carry = Vec::new();
    while produced < wanted {
        cancel.check()?;
        let n = match stdout.read(&mut bytes) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(Error::io(format!("reading audio of {}", path.display()), e)),
        };
        carry.extend_from_slice(&bytes[..n]);
        let whole = carry.len() / 4 * 4;
        floats.clear();
        floats.extend(
            carry[..whole]
                .as_chunks::<4>()
                .0
                .iter()
                .map(|b| f32::from_le_bytes(*b)),
        );
        carry.drain(..whole);
        let take = floats.len().min(wanted - produced);
        sink(&floats[..take])?;
        produced += take;
    }
    let got_audio = produced > 0;
    if produced < wanted {
        // Wait for the exit status: a clean early end is fine (silence pads it).
        drop(stdout);
        if let Err(e) = tool.wait(cancel)
            && !got_audio
        {
            if matches!(e, Error::Cancelled) {
                return Err(e);
            }
            tracing::warn!(path = %path.display(), error = %e, "audio could not be decoded; using silence");
        }
        let silence = vec![0f32; 48_000 * 2];
        while produced < wanted {
            let take = silence.len().min(wanted - produced);
            sink(&silence[..take])?;
            produced += take;
        }
    } else {
        tool.kill();
    }
    Ok(got_audio)
}
