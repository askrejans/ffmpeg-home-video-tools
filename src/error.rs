use std::path::PathBuf;
use thiserror::Error;

/// Custom error types for video processing operations
#[derive(Error, Debug)]
pub enum VideoProcessorError {
    #[error("FFmpeg not found in PATH. Please install FFmpeg.")]
    FFmpegNotFound,

    #[error("FFprobe not found in PATH. Please install FFmpeg with ffprobe.")]
    FFprobeNotFound,

    #[error("Failed to execute FFmpeg command: {0}")]
    FFmpegExecutionFailed(String),

    #[error("Failed to parse FFprobe output: {0}")]
    FFprobeParseError(String),

    #[error("Input directory not found: {0}")]
    InputDirectoryNotFound(PathBuf),

    #[error("Output directory creation failed: {0}")]
    OutputDirectoryCreationFailed(PathBuf),

    #[error("No video files found in: {0}")]
    NoVideoFilesFound(PathBuf),

    #[error("Invalid video file: {0}")]
    InvalidVideoFile(PathBuf),

    #[error("Insufficient disk space: required {required_mb}MB, available {available_mb}MB")]
    InsufficientDiskSpace {
        required_mb: u64,
        available_mb: u64,
    },

    #[error("Video conversion failed for {file}: {reason}")]
    ConversionFailed { file: PathBuf, reason: String },

    #[error("Padding operation failed for {file}: {reason}")]
    PaddingFailed { file: PathBuf, reason: String },

    #[error("Cropping operation failed for {file}: {reason}")]
    CroppingFailed { file: PathBuf, reason: String },

    #[error("Audio resampling failed for {file}: {reason}")]
    ResamplingFailed { file: PathBuf, reason: String },

    #[error("Concatenation failed: {0}")]
    ConcatenationFailed(String),

    #[allow(dead_code)]
    #[error("Command timed out after {timeout_secs} seconds")]
    Timeout { timeout_secs: u64 },

    #[error("Task join error: {0}")]
    JoinError(String),

    #[error("Configuration error: {0}")]
    ConfigError(String),

    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),

    #[error("Serialization error: {0}")]
    SerializationError(#[from] serde_json::Error),

    #[error("TOML parsing error: {0}")]
    TomlError(#[from] toml::de::Error),
}

/// Result type alias for video processing operations
pub type Result<T> = std::result::Result<T, VideoProcessorError>;
