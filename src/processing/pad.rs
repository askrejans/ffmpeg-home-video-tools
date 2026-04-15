use crate::error::{Result, VideoProcessorError};
use crate::ffmpeg::FFmpegWrapper;
use crate::types::ProcessingConfig;
use std::path::Path;
use tracing::{info, warn};
use walkdir::WalkDir;

/// Pad videos that aren't exactly target resolution with blurred background.
/// Uses split→blur→overlay filter to create a visually appealing blurred
/// background behind the original video content.
pub async fn pad_videos<F>(
    output_path: &Path,
    config: &ProcessingConfig,
    ffmpeg: &FFmpegWrapper,
    progress_callback: F,
) -> Result<()>
where
    F: Fn(crate::types::ProgressUpdate) + Send + Sync,
{
    info!("[PAD] Starting padding check for non-standard resolution videos");

    let preprocessed_path = output_path.join("preprocessed");
    if !preprocessed_path.exists() {
        info!("[PAD] Preprocessed directory not found, skipping padding step");
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
        info!("[PAD] No files to process");
        return Ok(());
    }

    video_files.sort();
    let mut padded_count = 0;
    let total_count = video_files.len();
    let (target_width, target_height) = config.target_resolution;

    for (idx, video_file) in video_files.iter().enumerate() {
        let file_name = video_file.file_name().unwrap().to_string_lossy().to_string();

        progress_callback(crate::types::ProgressUpdate {
            step: crate::types::ProcessingStep::Pad,
            current: idx + 1,
            total: total_count,
            file_name: Some(file_name.clone()),
            message: None,
            is_complete: false,
        });

        let metadata = match ffmpeg.probe_video(video_file) {
            Ok(m) => m,
            Err(e) => {
                warn!("[PAD] Failed to probe {}: {}", video_file.display(), e);
                continue;
            }
        };

        if metadata.width == target_width && metadata.height == target_height {
            info!("[PAD] {} already at target resolution, skipping", file_name);
            continue;
        }

        info!(
            "[PAD] Padding: {} ({}x{} → {}x{})",
            file_name, metadata.width, metadata.height, target_width, target_height
        );

        // Write to a temp file in the same directory, then atomically replace
        let temp_file = preprocessed_path.join(format!(".padding_{}", file_name));

        let cmd = ffmpeg.build_pad_command(video_file, &temp_file, config);

        ffmpeg.execute_command(cmd).await.map_err(|e| {
            // Clean up temp file on failure
            let _ = std::fs::remove_file(&temp_file);
            VideoProcessorError::PaddingFailed {
                file: video_file.to_path_buf(),
                reason: e.to_string(),
            }
        })?;

        // Verify output and atomically replace original
        if temp_file.exists() && temp_file.metadata()?.len() > 0 {
            std::fs::rename(&temp_file, video_file)?;
            padded_count += 1;
            info!("[PAD]   ✓ Padded successfully");
        } else {
            let _ = std::fs::remove_file(&temp_file);
            return Err(VideoProcessorError::PaddingFailed {
                file: video_file.to_path_buf(),
                reason: "Output file not created properly".to_string(),
            });
        }
    }

    if padded_count > 0 {
        info!("[PAD] Padding completed: {} file(s) padded", padded_count);
    } else {
        info!("[PAD] No files required padding");
    }

    Ok(())
}
