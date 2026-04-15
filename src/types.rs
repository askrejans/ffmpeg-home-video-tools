use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Video metadata extracted from ffprobe
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VideoMetadata {
    pub path: PathBuf,
    pub width: u32,
    pub height: u32,
    pub fps: f32,
    pub duration: f64,
    pub has_audio: bool,
    pub rotation: Option<i32>,
    pub codec: String,
    pub file_size: u64,
}

impl VideoMetadata {
    pub fn is_rotated_vertical(&self) -> bool {
        self.rotation == Some(90) || self.rotation == Some(270)
    }

    #[allow(dead_code)]
    pub fn is_vertical(&self) -> bool {
        let (w, h) = self.effective_dimensions();
        h > w
    }

    /// Get the effective dimensions after accounting for rotation metadata.
    /// Videos with 90/270 rotation have swapped width/height in their metadata.
    #[allow(dead_code)]
    pub fn effective_dimensions(&self) -> (u32, u32) {
        if self.is_rotated_vertical() {
            (self.height, self.width) // swap for rotated videos
        } else {
            (self.width, self.height)
        }
    }

    #[allow(dead_code)]
    pub fn aspect_ratio(&self) -> f64 {
        self.width as f64 / self.height as f64
    }
}

/// Processing configuration with encoding settings
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessingConfig {
    /// Target resolution (width, height)
    pub target_resolution: (u32, u32),
    
    /// Target frames per second
    pub target_fps: u32,
    
    /// Video codec
    pub video_codec: String,
    
    /// Video encoding preset (ultrafast, veryfast, fast, medium, slow)
    pub video_preset: String,
    
    /// Constant Rate Factor for video quality (0-51, lower is better)
    pub video_crf: u8,
    
    /// Constant Rate Factor for padding operations
    pub padding_crf: u8,
    
    /// Bitrate for intermediate files (in Mbps) to avoid double CRF degradation
    pub intermediate_bitrate: u32,
    
    /// Audio codec
    pub audio_codec: String,
    
    /// Audio bitrate in kbps
    pub audio_bitrate: u32,
    
    /// Audio sample rate
    pub audio_sample_rate: u32,
    
    /// Audio channels
    pub audio_channels: u32,
    
    /// Blur radius for padding (relative to dimension)
    pub blur_radius_divisor: u32,
    
    /// Async compensation for audio resampling
    pub async_compensation: u32,
    
    /// Number of parallel processing jobs (0 = CPU count)
    pub parallel_jobs: usize,
    
    /// Keep intermediate files for debugging
    pub keep_intermediates: bool,
}

impl Default for ProcessingConfig {
    fn default() -> Self {
        Self {
            target_resolution: (3840, 2160),  // 4K UHD
            target_fps: 25,  // PAL standard
            video_codec: "libx264".to_string(),
            video_preset: "medium".to_string(),
            video_crf: 18,
            padding_crf: 22,
            intermediate_bitrate: 20,
            audio_codec: "aac".to_string(),
            audio_bitrate: 320, 
            audio_sample_rate: 48000,
            audio_channels: 2,
            blur_radius_divisor: 20,
            async_compensation: 1000,
            parallel_jobs: 0,
            keep_intermediates: false,
        }
    }
}

impl ProcessingConfig {
    /// Create a fast processing profile (lower quality, faster encoding)
    pub fn fast() -> Self {
        Self {
            video_preset: "faster".to_string(),
            video_crf: 24,
            padding_crf: 28,
            audio_bitrate: 192,
            ..Default::default()
        }
    }

    /// Create a balanced processing profile (4K TV optimized - default)
    pub fn balanced() -> Self {
        Self::default()
    }

    /// Create a quality processing profile (higher quality, slower encoding)
    pub fn quality() -> Self {
        Self {
            video_preset: "slow".to_string(),
            video_crf: 18,  // Excellent 4K quality
            padding_crf: 22,
            audio_bitrate: 320,  // Higher bitrate for quality profile
            ..Default::default()
        }
    }
}

/// Processing step enumeration
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProcessingStep {
    BatchConvert,
    Pad,
    Crop,
    Resample,
    Concatenate,
}

impl ProcessingStep {
    pub fn name(&self) -> &'static str {
        match self {
            Self::BatchConvert => "Batch Convert",
            Self::Pad => "Pad to 4K",
            Self::Crop => "Crop to 4K",
            Self::Resample => "Resample Audio",
            Self::Concatenate => "Concatenate Videos",
        }
    }

    pub fn description(&self) -> &'static str {
        match self {
            Self::BatchConvert => "Convert and standardize video formats to MP4",
            Self::Pad => "Add padding to videos with non-standard aspect ratios",
            Self::Crop => "Crop oversized videos to target resolution",
            Self::Resample => "Resample audio to 48kHz stereo (adds silent track if missing)",
            Self::Concatenate => "Concatenate all processed videos into final output",
        }
    }

    pub fn all_steps() -> Vec<Self> {
        vec![
            Self::BatchConvert,
            Self::Pad,
            Self::Crop,
            Self::Resample,
            Self::Concatenate,
        ]
    }
}

/// Processing state for checkpoint/resume functionality
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessingState {
    pub input_path: PathBuf,
    pub output_path: PathBuf,
    pub completed_steps: Vec<ProcessingStep>,
    pub current_step: Option<ProcessingStep>,
    pub processed_files: Vec<PathBuf>,
    pub started_at: chrono::DateTime<chrono::Utc>,
    pub last_updated: chrono::DateTime<chrono::Utc>,
}

impl ProcessingState {
    pub fn new(input_path: PathBuf, output_path: PathBuf) -> Self {
        let now = chrono::Utc::now();
        Self {
            input_path,
            output_path,
            completed_steps: Vec::new(),
            current_step: None,
            processed_files: Vec::new(),
            started_at: now,
            last_updated: now,
        }
    }

    pub fn is_step_completed(&self, step: ProcessingStep) -> bool {
        self.completed_steps.contains(&step)
    }

    pub fn mark_step_completed(&mut self, step: ProcessingStep) {
        if !self.completed_steps.contains(&step) {
            self.completed_steps.push(step);
        }
        self.current_step = None;
        self.last_updated = chrono::Utc::now();
    }

    pub fn set_current_step(&mut self, step: ProcessingStep) {
        self.current_step = Some(step);
        self.last_updated = chrono::Utc::now();
    }
}

/// Progress update information
#[derive(Debug, Clone)]
pub struct ProgressUpdate {
    pub step: ProcessingStep,
    pub current: usize,
    pub total: usize,
    pub file_name: Option<String>,
    pub message: Option<String>,
    pub is_complete: bool,
}

impl ProgressUpdate {
    pub fn progress_percentage(&self) -> f32 {
        if self.total == 0 {
            0.0
        } else {
            (self.current as f32 / self.total as f32) * 100.0
        }
    }
}
