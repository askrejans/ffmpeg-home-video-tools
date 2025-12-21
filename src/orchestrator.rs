use crate::error::{Result, VideoProcessorError};
use crate::ffmpeg::FFmpegWrapper;
use crate::types::{ProcessingConfig, ProcessingState, ProcessingStep};
use std::path::{Path, PathBuf};
use tracing::{info, warn};

/// Orchestrator manages the entire video processing pipeline
pub struct Orchestrator {
    input_path: PathBuf,
    output_path: PathBuf,
    config: ProcessingConfig,
    state: ProcessingState,
    ffmpeg: FFmpegWrapper,
}

impl Orchestrator {
    /// Create a new orchestrator
    pub fn new(
        input_path: PathBuf,
        output_path: PathBuf,
        config: ProcessingConfig,
    ) -> Result<Self> {
        if !input_path.exists() {
            return Err(VideoProcessorError::InputDirectoryNotFound(input_path));
        }

        // Create output directory
        std::fs::create_dir_all(&output_path).map_err(|_| {
            VideoProcessorError::OutputDirectoryCreationFailed(output_path.clone())
        })?;

        let state = ProcessingState::new(input_path.clone(), output_path.clone());
        let ffmpeg = FFmpegWrapper::new()?;

        Ok(Self {
            input_path,
            output_path,
            config,
            state,
            ffmpeg,
        })
    }

    /// Load orchestrator from checkpoint file (deprecated - no longer used)
    #[allow(dead_code)]
    pub fn from_checkpoint(checkpoint_path: &Path) -> Result<Self> {
        let contents = std::fs::read_to_string(checkpoint_path)?;
        let state: ProcessingState = serde_json::from_str(&contents)?;

        let config = ProcessingConfig::default();
        let ffmpeg = FFmpegWrapper::new()?;

        Ok(Self {
            input_path: state.input_path.clone(),
            output_path: state.output_path.clone(),
            config,
            state,
            ffmpeg,
        })
    }

    /// Validate inputs without processing
    pub async fn validate(&self) -> Result<()> {
        info!("Validating inputs...");
        
        // Check disk space
        self.check_disk_space()?;
        
        // Count input files
        let files = self.find_input_videos()?;
        if files.is_empty() {
            return Err(VideoProcessorError::NoVideoFilesFound(
                self.input_path.clone(),
            ));
        }

        info!("Found {} video file(s)", files.len());
        Ok(())
    }

    /// Run the processing pipeline
    pub async fn process<F>(&mut self, progress_callback: F) -> Result<()>
    where
        F: Fn(crate::types::ProgressUpdate) + Send + Sync + Clone,
    {
        info!("Starting video processing pipeline");

        // Validate first
        self.validate().await?;

        // Execute each step
        for step in ProcessingStep::all_steps() {
            if self.state.is_step_completed(step) {
                info!("Skipping completed step: {}", step.name());
                continue;
            }

            info!("Executing step: {}", step.name());
            self.state.set_current_step(step);
            self.execute_step(step, progress_callback.clone()).await?;
            self.state.mark_step_completed(step);
        }

        info!("Processing pipeline completed successfully");
        
        // Send final completion signal
        progress_callback(crate::types::ProgressUpdate {
            step: ProcessingStep::Concatenate,
            current: 1,
            total: 1,
            file_name: None,
            message: Some("All processing completed!".to_string()),
            is_complete: true,
        });
        
        Ok(())
    }

    /// Execute a single processing step
    async fn execute_step<F>(&self, step: ProcessingStep, progress_callback: F) -> Result<()>
    where
        F: Fn(crate::types::ProgressUpdate) + Send + Sync + Clone,
    {
        match step {
            ProcessingStep::BatchConvert => {
                crate::processing::convert_videos(
                    &self.input_path,
                    &self.output_path,
                    &self.config,
                    &self.ffmpeg,
                    progress_callback,
                ).await?;
            }
            ProcessingStep::Pad => {
                crate::processing::pad_videos(&self.output_path, &self.config, &self.ffmpeg, progress_callback)
                    .await?;
            }
            ProcessingStep::Crop => {
                crate::processing::crop_videos(&self.output_path, &self.config, &self.ffmpeg, progress_callback)
                    .await?;
            }
            ProcessingStep::Resample => {
                crate::processing::resample_audio(&self.output_path, &self.config, &self.ffmpeg, progress_callback)
                    .await?;
            }
            ProcessingStep::Concatenate => {
                crate::processing::concatenate_videos(&self.output_path, &self.config, &self.ffmpeg, progress_callback)
                    .await?;
            }
        }

        Ok(())
    }

    /// Find all video files in input directory
    fn find_input_videos(&self) -> Result<Vec<PathBuf>> {
        use walkdir::WalkDir;

        let video_extensions = ["mp4", "avi", "mov", "mkv", "m4v", "3gp"];
        let mut videos = Vec::new();

        for entry in WalkDir::new(&self.input_path)
            .max_depth(1)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            let path = entry.path();
            if let Some(ext) = path.extension() {
                if video_extensions
                    .iter()
                    .any(|&e| e.eq_ignore_ascii_case(&ext.to_string_lossy()))
                {
                    videos.push(path.to_path_buf());
                }
            }
        }

        Ok(videos)
    }

    /// Check if sufficient disk space is available
    fn check_disk_space(&self) -> Result<()> {
        use sysinfo::Disks;
        
        // Calculate required space: estimate 2x the size of input videos
        let input_size = self.calculate_input_size()?;
        let required_space = input_size * 2; // Conservative estimate for processing
        
        // Get available space on output directory's disk
        let disks = Disks::new_with_refreshed_list();
        let output_canonical = self.output_path.canonicalize().unwrap_or_else(|_| self.output_path.clone());
        
        // Find the disk that contains the output path
        let mut available_space = None;
        for disk in disks.list() {
            let mount_point = disk.mount_point();
            if output_canonical.starts_with(mount_point) {
                available_space = Some(disk.available_space());
                info!(
                    "[DISK] Output disk: {} - Available: {:.2} GB, Required (estimated): {:.2} GB",
                    mount_point.display(),
                    disk.available_space() as f64 / (1024.0 * 1024.0 * 1024.0),
                    required_space as f64 / (1024.0 * 1024.0 * 1024.0)
                );
                break;
            }
        }
        
        let available = available_space.unwrap_or_else(|| {
            warn!("[DISK] Could not determine available disk space, proceeding anyway");
            u64::MAX
        });
        
        if available < required_space {
            let available_mb = available / (1024 * 1024);
            let required_mb = required_space / (1024 * 1024);
            return Err(VideoProcessorError::InsufficientDiskSpace {
                available_mb,
                required_mb,
            });
        }
        
        Ok(())
    }
    
    /// Calculate total size of input videos
    fn calculate_input_size(&self) -> Result<u64> {
        let videos = self.find_input_videos()?;
        let mut total_size = 0u64;
        
        for video in videos {
            if let Ok(metadata) = std::fs::metadata(&video) {
                total_size += metadata.len();
            }
        }
        
        info!(
            "[DISK] Input videos total size: {:.2} GB",
            total_size as f64 / (1024.0 * 1024.0 * 1024.0)
        );
        
        Ok(total_size)
    }

}
