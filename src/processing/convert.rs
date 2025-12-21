use crate::error::{Result, VideoProcessorError};
use crate::ffmpeg::FFmpegWrapper;
use crate::types::ProcessingConfig;
use std::path::Path;
use tracing::{debug, info};
use walkdir::WalkDir;

/// Convert all videos in input directory to standardized MP4 format
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

    let video_extensions = ["mp4", "avi", "mov", "mkv", "m4v", "3gp", "MP4", "AVI", "MOV", "MKV", "M4V", "3GP"];
    let mut video_files = Vec::new();

    for entry in WalkDir::new(input_path)
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
                video_files.push(path.to_path_buf());
            }
        }
    }

    if video_files.is_empty() {
        return Err(VideoProcessorError::NoVideoFilesFound(
            input_path.to_path_buf(),
        ));
    }

    info!("[CONVERT] Found {} video file(s) to process", video_files.len());

    for (idx, input_file) in video_files.iter().enumerate() {
        let file_name = input_file
            .file_stem()
            .unwrap()
            .to_string_lossy()
            .to_string();
        let output_file = preprocessed_path.join(format!("{}.mp4", file_name));

        // Send progress update
        progress_callback(crate::types::ProgressUpdate {
            step: crate::types::ProcessingStep::BatchConvert,
            current: idx + 1,
            total: video_files.len(),
            file_name: Some(input_file.file_name().unwrap().to_string_lossy().to_string()),
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

        // Build and execute conversion command
        let cmd = ffmpeg.build_convert_command(input_file, &output_file, &metadata, config);

        ffmpeg.execute_command(cmd).map_err(|e| {
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

#[cfg(test)]
mod tests {
    #[test]
    fn test_convert_module_exists() {
        // Placeholder test - actual tests would use mocked FFmpeg
        assert!(true);
    }
}
