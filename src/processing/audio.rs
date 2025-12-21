use crate::error::{Result, VideoProcessorError};
use crate::ffmpeg::FFmpegWrapper;
use crate::types::ProcessingConfig;
use std::path::Path;
use std::process::Command;
use tracing::{info, warn};
use walkdir::WalkDir;

/// Resample all audio to target sample rate with async compensation
pub async fn resample_audio<F>(
    output_path: &Path,
    config: &ProcessingConfig,
    ffmpeg: &FFmpegWrapper,
    progress_callback: F,
) -> Result<()>
where
    F: Fn(crate::types::ProgressUpdate) + Send + Sync,
{
    info!("[RESAMPLE] Starting audio resampling to {}Hz stereo", config.audio_sample_rate);

    let preprocessed_path = output_path.join("preprocessed");
    if !preprocessed_path.exists() {
        info!("[RESAMPLE] Preprocessed directory not found, skipping resample step");
        return Ok(());
    }

    let mut video_files = Vec::new();
    for entry in WalkDir::new(&preprocessed_path)
        .max_depth(1)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        if entry.path().extension().and_then(|s| s.to_str()) == Some("mp4") {
            video_files.push(entry.path().to_path_buf());
        }
    }

    if video_files.is_empty() {
        info!("[RESAMPLE] No files to process");
        return Ok(());
    }

    info!("[RESAMPLE] Processing {} file(s)", video_files.len());

    for (idx, video_file) in video_files.iter().enumerate() {
        progress_callback(crate::types::ProgressUpdate {
            step: crate::types::ProcessingStep::Resample,
            current: idx + 1,
            total: video_files.len(),
            file_name: Some(video_file.file_name().unwrap().to_string_lossy().to_string()),
            message: None,
            is_complete: false,
        });
        
        let file_name = video_file.file_name().unwrap().to_string_lossy();
        // Strip any prefix (padded_, cropped_) and use clean filename
        let clean_name = file_name
            .strip_prefix("padded_")
            .or_else(|| file_name.strip_prefix("cropped_"))
            .unwrap_or(&file_name);
        let output_file = output_path.join(format!("resampled_{}", clean_name));

        info!(
            "[RESAMPLE] [{}/{}] Processing: {}",
            idx + 1,
            video_files.len(),
            file_name
        );

        if output_file.exists() {
            info!("[RESAMPLE]   Already resampled, skipping");
            continue;
        }

        // Check if video has audio
        let metadata = match ffmpeg.probe_video(video_file) {
            Ok(m) => m,
            Err(e) => {
                warn!("[RESAMPLE] Failed to probe {}: {}", video_file.display(), e);
                continue;
            }
        };

        // Resample audio with async compensation, or add silent audio if missing
        let mut cmd = Command::new("ffmpeg");
        cmd.arg("-y")
            .arg("-i")
            .arg(&video_file);
        
        if metadata.has_audio {
            // Normal resample with existing audio
            cmd.arg("-c:v")
                .arg("copy")
                .arg("-c:a")
                .arg(&config.audio_codec)
                .arg("-b:a")
                .arg(format!("{}k", config.audio_bitrate))
                .arg("-ar")
                .arg(config.audio_sample_rate.to_string())
                .arg("-ac")
                .arg(config.audio_channels.to_string())
                .arg("-af")
                .arg(format!("aresample=async={}", config.async_compensation));
        } else {
            // Add silent audio track for videos without audio
            info!("[RESAMPLE]   No audio detected, adding silent track");
            cmd.arg("-f")
                .arg("lavfi")
                .arg("-i")
                .arg(format!(
                    "anullsrc=channel_layout=stereo:sample_rate={}",
                    config.audio_sample_rate
                ))
                .arg("-c:v")
                .arg("copy")
                .arg("-c:a")
                .arg(&config.audio_codec)
                .arg("-b:a")
                .arg(format!("{}k", config.audio_bitrate))
                .arg("-shortest");
        }
        
        cmd.arg(&output_file)
            .arg("-hide_banner")
            .arg("-loglevel")
            .arg("error");

        ffmpeg.execute_command(cmd).map_err(|e| {
            VideoProcessorError::ResamplingFailed {
                file: video_file.clone(),
                reason: e.to_string(),
            }
        })?;

            if output_file.exists() && output_file.metadata()?.len() > 0 {
                // Delete the original preprocessed file after successful resampling
                std::fs::remove_file(&video_file)?;
                info!("[RESAMPLE]   ✓ Completed");
            } else {
                return Err(VideoProcessorError::ResamplingFailed {
                    file: video_file.clone(),
                    reason: "Output file not created properly".to_string(),
                });
            }
    }

    info!(
        "[RESAMPLE] Audio resampling completed: {} file(s) processed",
        video_files.len()
    );

    // Clean up preprocessed directory
    if preprocessed_path.exists() {
        info!("[RESAMPLE] Cleaning up preprocessed directory");
        std::fs::remove_dir_all(&preprocessed_path)?;
    }

    Ok(())
}
