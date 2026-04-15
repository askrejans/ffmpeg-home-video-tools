use crate::error::{Result, VideoProcessorError};
use crate::ffmpeg::FFmpegWrapper;
use crate::types::ProcessingConfig;
use std::path::Path;
use tracing::{info, warn};
use walkdir::WalkDir;

/// Resample all audio to target sample rate with async compensation.
/// Also adds silent audio tracks to videos that have no audio.
pub async fn resample_audio<F>(
    output_path: &Path,
    config: &ProcessingConfig,
    ffmpeg: &FFmpegWrapper,
    progress_callback: F,
) -> Result<()>
where
    F: Fn(crate::types::ProgressUpdate) + Send + Sync,
{
    info!(
        "[RESAMPLE] Starting audio resampling to {}Hz stereo",
        config.audio_sample_rate
    );

    let preprocessed_path = output_path.join("preprocessed");
    if !preprocessed_path.exists() {
        info!("[RESAMPLE] Preprocessed directory not found, skipping resample step");
        return Ok(());
    }

    let mut video_files: Vec<_> = WalkDir::new(&preprocessed_path)
        .max_depth(1)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|entry| {
            entry
                .path()
                .extension()
                .and_then(|s| s.to_str())
                .is_some_and(|ext| ext.eq_ignore_ascii_case("mp4"))
        })
        .map(|e| e.path().to_path_buf())
        .collect();

    if video_files.is_empty() {
        info!("[RESAMPLE] No files to process");
        return Ok(());
    }

    video_files.sort();
    info!("[RESAMPLE] Processing {} file(s)", video_files.len());

    for (idx, video_file) in video_files.iter().enumerate() {
        let file_name = video_file
            .file_name()
            .unwrap()
            .to_string_lossy()
            .to_string();

        progress_callback(crate::types::ProgressUpdate {
            step: crate::types::ProcessingStep::Resample,
            current: idx + 1,
            total: video_files.len(),
            file_name: Some(file_name.clone()),
            message: None,
            is_complete: false,
        });

        // Output goes to the main output directory with resampled_ prefix
        let output_file = output_path.join(format!("resampled_{}", file_name));

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

        let metadata = match ffmpeg.probe_video(video_file) {
            Ok(m) => m,
            Err(e) => {
                warn!(
                    "[RESAMPLE] Failed to probe {}: {}",
                    video_file.display(),
                    e
                );
                continue;
            }
        };

        let mut cmd = ffmpeg.ffmpeg_cmd();
        cmd.arg("-y").arg("-i").arg(&video_file);

        if metadata.has_audio {
            // Resample existing audio
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
            // Add silent audio track
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

        ffmpeg.execute_command(cmd).await.map_err(|e| {
            VideoProcessorError::ResamplingFailed {
                file: video_file.clone(),
                reason: e.to_string(),
            }
        })?;

        if output_file.exists() && output_file.metadata()?.len() > 0 {
            // Delete the original preprocessed file after successful resampling
            std::fs::remove_file(video_file)?;
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
