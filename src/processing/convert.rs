use crate::error::{Result, VideoProcessorError};
use crate::ffmpeg::FFmpegWrapper;
use crate::types::ProcessingConfig;
use std::path::Path;
use tracing::{debug, info};
use walkdir::WalkDir;

const VIDEO_EXTENSIONS: &[&str] = &["mp4", "avi", "mov", "mkv", "m4v", "3gp"];

/// Convert all videos in input directory to standardized MP4 format.
/// Handles: codec normalization, rotation correction, fps normalization.
/// Does NOT handle resolution — that's the pad step's job.
pub async fn convert_videos<F>(
    input_path: &Path,
    output_path: &Path,
    config: &ProcessingConfig,
    ffmpeg: &FFmpegWrapper,
    progress_callback: F,
) -> Result<()>
where
    F: Fn(crate::types::ProgressUpdate) + Send + Sync,
{
    info!("[CONVERT] Starting batch conversion to MP4");

    let preprocessed_path = output_path.join("preprocessed");
    std::fs::create_dir_all(&preprocessed_path)?;

    let mut video_files: Vec<_> = WalkDir::new(input_path)
        .max_depth(1)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|entry| {
            entry
                .path()
                .extension()
                .and_then(|ext| ext.to_str())
                .is_some_and(|ext| {
                    VIDEO_EXTENSIONS
                        .iter()
                        .any(|&e| e.eq_ignore_ascii_case(ext))
                })
        })
        .map(|e| e.path().to_path_buf())
        .collect();

    if video_files.is_empty() {
        return Err(VideoProcessorError::NoVideoFilesFound(
            input_path.to_path_buf(),
        ));
    }

    // Sort for deterministic ordering
    video_files.sort();

    info!(
        "[CONVERT] Found {} video file(s) to process",
        video_files.len()
    );

    for (idx, input_file) in video_files.iter().enumerate() {
        let file_name = input_file
            .file_stem()
            .unwrap()
            .to_string_lossy()
            .to_string();
        let output_file = preprocessed_path.join(format!("{}.mp4", file_name));

        progress_callback(crate::types::ProgressUpdate {
            step: crate::types::ProcessingStep::BatchConvert,
            current: idx + 1,
            total: video_files.len(),
            file_name: Some(
                input_file
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .to_string(),
            ),
            message: None,
            is_complete: false,
        });

        info!(
            "[CONVERT] [{}/{}] Processing: {}",
            idx + 1,
            video_files.len(),
            input_file.file_name().unwrap().to_string_lossy()
        );

        // Skip if already processed
        if output_file.exists() {
            info!("[CONVERT]   Output already exists, skipping");
            continue;
        }

        // Probe video metadata
        let metadata = ffmpeg.probe_video(input_file).map_err(|e| {
            VideoProcessorError::ConversionFailed {
                file: input_file.clone(),
                reason: format!("Failed to probe video: {}", e),
            }
        })?;

        debug!(
            "[CONVERT]   Metadata: {}x{} @ {:.2}fps, rotation: {:?}",
            metadata.width, metadata.height, metadata.fps, metadata.rotation
        );

        let cmd = ffmpeg.build_convert_command(input_file, &output_file, &metadata, config);

        ffmpeg.execute_command(cmd).await.map_err(|e| {
            VideoProcessorError::ConversionFailed {
                file: input_file.clone(),
                reason: e.to_string(),
            }
        })?;

        info!("[CONVERT]   ✓ Completed");
    }

    info!(
        "[CONVERT] Batch conversion completed: {} file(s) processed",
        video_files.len()
    );

    Ok(())
}
