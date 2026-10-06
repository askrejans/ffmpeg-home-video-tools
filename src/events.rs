use crate::error::{Error, Result};
use serde::Serialize;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// Cooperative cancellation shared between a caller and a running job.
///
/// Cancelling kills any FFmpeg processes the job started and makes the job
/// return [`Error::Cancelled`].
#[derive(Debug, Clone, Default)]
pub struct CancelToken(Arc<AtomicBool>);

impl CancelToken {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst);
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }

    pub(crate) fn check(&self) -> Result<()> {
        if self.is_cancelled() {
            Err(Error::Cancelled)
        } else {
            Ok(())
        }
    }
}

/// The phases a render goes through, in order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    Preparing,
    Audio,
    Loudness,
    Video,
    Verifying,
    Finishing,
}

impl Stage {
    /// Share of the whole job's progress bar covered by this stage.
    pub(crate) fn span(self) -> (f64, f64) {
        match self {
            Stage::Preparing => (0.0, 0.02),
            Stage::Audio => (0.02, 0.12),
            Stage::Loudness => (0.12, 0.16),
            Stage::Video => (0.16, 0.97),
            Stage::Verifying => (0.97, 0.995),
            Stage::Finishing => (0.995, 1.0),
        }
    }
}

/// Progress and status reported while a job runs.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    Stage {
        stage: Stage,
    },
    Progress {
        /// Whole-job progress, 0..=1.
        fraction: f64,
        stage: Stage,
        /// Frames written so far (video stage) or processed (other stages).
        done: u64,
        total: u64,
        /// Current processing speed in frames per second, when known.
        fps: Option<f64>,
        eta_seconds: Option<f64>,
        /// Index into the project's clips that is currently being processed.
        clip: Option<usize>,
    },
    Warning {
        code: String,
        message: String,
        clip: Option<usize>,
    },
    Done {
        output: PathBuf,
        duration_seconds: f64,
        size_bytes: u64,
    },
}

/// Receives events from a running job.
pub type EventSink<'a> = &'a mut dyn FnMut(Event);

pub(crate) fn warning(code: &str, message: impl Into<String>, clip: Option<usize>) -> Event {
    Event::Warning {
        code: code.to_string(),
        message: message.into(),
        clip,
    }
}
