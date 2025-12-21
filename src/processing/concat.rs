use crate::error::{Result, VideoProcessorError};
use crate::ffmpeg::FFmpegWrapper;
use crate::types::ProcessingConfig;
use std::fs::File;
use std::io::Write;
use std::path::Path;
use std::process::Command;
use tracing::info;
use walkdir::WalkDir;

/// Concatenate all processed videos into final output
pub async fn concatenate_videos<F>(
    output_path: &Path,
    config: &ProcessingConfig,
    ffmpeg: &FFmpegWrapper,
    progress_callback: F,
) -> Result<()>
where
    F: Fn(crate::types::ProgressUpdate) + Send + Sync,
{
    info!("[CONCAT] Starting video concatenation");
    
    // Send initial progress
    progress_callback(crate::types::ProgressUpdate {
        step: crate::types::ProcessingStep::Concatenate,
        current: 0,
        total: 1,
        file_name: None,
        message: Some("Preparing concatenation...".to_string()),
        is_complete: false,
    });

    let concat_list_path = output_path.join("concat_list.txt");

    // Find only resampled MP4 files to concatenate
    let mut video_files = Vec::new();
    for entry in WalkDir::new(output_path)
        .max_depth(1)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        let path = entry.path();
        let filename = path.file_name().unwrap().to_string_lossy();
        // Only process resampled files - they are the final preprocessed output
        if filename.starts_with("resampled_") && path.extension().and_then(|s| s.to_str()) == Some("mp4") {
            video_files.push(path.to_path_buf());
        }
    }

    if video_files.is_empty() {
        return Err(VideoProcessorError::ConcatenationFailed(
            "No MP4 files found to concatenate".to_string(),
        ));
    }

    // Sort files by name for consistent ordering
    video_files.sort();

    info!("[CONCAT] Found {} file(s) to concatenate", video_files.len());

    // Create concat list file
    let mut concat_file = File::create(&concat_list_path)?;
    for video_file in &video_files {
        let file_name = video_file.file_name().unwrap().to_string_lossy();
        // Escape single quotes in filename
        let escaped_name = file_name.replace("'", "'\\''");
        writeln!(concat_file, "file '{}'", escaped_name)?;
    }
    concat_file.flush()?;

    info!("[CONCAT] Concatenation list created");

    // Generate output filename with timestamp
    let timestamp = chrono::Local::now().format("%Y%m%d_%H%M%S");
    let output_filename = format!("processed_vod_{}.mp4", timestamp);
    let output_file = output_path.join(&output_filename);

    info!("[CONCAT] Creating final video: {}", output_filename);
    info!("[CONCAT] Re-encoding all videos for consistency and final quality");

    // Execute concatenation with re-encoding
    let mut cmd = Command::new("ffmpeg");
    cmd.arg("-y")
        .arg("-f")
        .arg("concat")
        .arg("-safe")
        .arg("0")
        .arg("-i")
        .arg(&concat_list_path)
        .arg("-c:v")
        .arg(&config.video_codec)
        .arg("-preset")
        .arg(&config.video_preset)
        .arg("-crf")
        .arg(config.video_crf.to_string())
        .arg("-c:a")
        .arg(&config.audio_codec)
        .arg("-b:a")
        .arg(format!("{}k", config.audio_bitrate))
        .arg("-r")
        .arg(config.target_fps.to_string())
        .arg("-s")
        .arg(format!("{}x{}", config.target_resolution.0, config.target_resolution.1))
        .arg(&output_file)
        .arg("-hide_banner")
        .arg("-loglevel")
        .arg("error");

    ffmpeg.execute_command(cmd).map_err(|e| {
        VideoProcessorError::ConcatenationFailed(format!("FFmpeg concatenation failed: {}", e))
    })?;

    // Verify output was created
    if !output_file.exists() || output_file.metadata()?.len() == 0 {
        return Err(VideoProcessorError::ConcatenationFailed(
            "Output file not created or is empty".to_string(),
        ));
    }

    info!("[CONCAT] ✓ Concatenation successful: {}", output_filename);

    // Get final file size
    if let Ok(metadata) = output_file.metadata() {
        let file_size_mb = metadata.len() / (1024 * 1024);
        info!("[CONCAT] Final file size: {} MB", file_size_mb);
    }

    // Cleanup intermediate files if not keeping them
    if !config.keep_intermediates {
        info!("[CONCAT] Cleaning up intermediate files");
        let mut removed_count = 0;

        for video_file in &video_files {
            let file_name = video_file.file_name().unwrap().to_string_lossy();
            if file_name.starts_with("resampled_") {
                if std::fs::remove_file(video_file).is_ok() {
                    removed_count += 1;
                }
            }
        }

        // Remove preprocessed directory if it still exists
        let preprocessed_path = output_path.join("preprocessed");
        if preprocessed_path.exists() {
            let _ = std::fs::remove_dir_all(&preprocessed_path);
        }

        // Remove concat list
        let _ = std::fs::remove_file(&concat_list_path);

        info!(
            "[CONCAT] Cleanup completed: {} intermediate file(s) removed",
            removed_count
        );
    }

    info!("[CONCAT] Final output: {}", output_file.display());

    Ok(())
}
