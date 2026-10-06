use std::path::PathBuf;
use thiserror::Error;

/// Errors produced by the library.
///
/// Every variant has a stable machine-readable [`Error::code`] so that hosts can
/// map failures to their own (translated) messages.
#[derive(Debug, Error)]
pub enum Error {
    #[error("{tool} was not found; install FFmpeg or pass its path explicitly")]
    ToolNotFound { tool: &'static str },

    #[error("{tool} failed{}: {stderr}", status.map(|s| format!(" (exit code {s})")).unwrap_or_default())]
    ToolFailed {
        tool: &'static str,
        status: Option<i32>,
        stderr: String,
    },

    #[error("cannot read {path}: {reason}")]
    Unreadable { path: PathBuf, reason: String },

    #[error("there are no usable video clips to render")]
    NoClips,

    #[error("invalid project: {0}")]
    InvalidProject(String),

    #[error("invalid title template: {0}")]
    Template(String),

    #[error("no working {kind} encoder was found in this FFmpeg build")]
    EncoderUnavailable { kind: &'static str },

    #[error("not enough free space in {path}: {needed_mb} MB needed, {available_mb} MB available")]
    DiskFull {
        path: PathBuf,
        needed_mb: u64,
        available_mb: u64,
    },

    #[error("the rendered file failed verification: {0}")]
    OutputInvalid(String),

    #[error("cancelled")]
    Cancelled,

    #[error("{context}: {source}")]
    Io {
        context: String,
        #[source]
        source: std::io::Error,
    },

    #[error("invalid JSON: {0}")]
    Json(#[from] serde_json::Error),
}

impl Error {
    /// Stable identifier for this kind of failure.
    pub fn code(&self) -> &'static str {
        match self {
            Error::ToolNotFound { .. } => "tool_not_found",
            Error::ToolFailed { .. } => "tool_failed",
            Error::Unreadable { .. } => "clip_unreadable",
            Error::NoClips => "no_clips",
            Error::InvalidProject(_) => "invalid_project",
            Error::Template(_) => "template_invalid",
            Error::EncoderUnavailable { .. } => "encoder_unavailable",
            Error::DiskFull { .. } => "disk_full",
            Error::OutputInvalid(_) => "output_invalid",
            Error::Cancelled => "cancelled",
            Error::Io { .. } => "io",
            Error::Json(_) => "invalid_json",
        }
    }

    pub(crate) fn io(context: impl Into<String>, source: std::io::Error) -> Self {
        Error::Io {
            context: context.into(),
            source,
        }
    }
}

pub type Result<T, E = Error> = std::result::Result<T, E>;
