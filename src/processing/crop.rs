use crate::error::{Result, VideoProcessorError};
use crate::ffmpeg::FFmpegWrapper;
use crate::types::ProcessingConfig;
use std::path::Path;
use tracing::{info, warn};
use walkdir::WalkDir;

/// Crop videos that exceed target resolution
pub async fn crop_videos<F>(
    output_path: &Path,
    config: &ProcessingConfig,
    ffmpeg: &FFmpegWrapper,
    progress_callback: F,
) -> Result<()>
where
    F: Fn(crate::types::ProgressUpdate) + Send + Sync,
{
    info!("[CROP] Starting crop check for oversized videos");

    let preprocessed_path = output_path.join("preprocessed");
    if !preprocessed_path.exists() {
        info!("[CROP] Preprocessed directory not found, skipping crop step");
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
        info!("[CROP] No files to process");
        return Ok(());
    }

    video_files.sort();
    let mut cropped_count = 0;
    let (target_width, target_height) = config.target_resolution;
    let total_count = video_files.len();

    for (idx, video_file) in video_files.iter().enumerate() {
        let file_name = video_file.file_name().unwrap().to_string_lossy().to_string();

        progress_callback(crate::types::ProgressUpdate {
            step: crate::types::ProcessingStep::Crop,
            current: idx + 1,
            total: total_count,
            file_name: Some(file_name.clone()),
            message: None,
            is_complete: false,
        });

        let metadata = match ffmpeg.probe_video(video_file) {
            Ok(m) => m,
            Err(e) => {
                warn!("[CROP] Failed to probe {}: {}", video_file.display(), e);
                continue;
            }
        };

        // Only crop if resolution exceeds target
        if metadata.width <= target_width && metadata.height <= target_height {
            continue;
        }

        info!(
            "[CROP] Cropping: {} ({}x{} → {}x{})",
            file_name, metadata.width, metadata.height, target_width, target_height
        );

        let temp_file = preprocessed_path.join(format!(".cropping_{}", file_name));

        // Crop to 16:9 aspect ratio maintaining maximum content
        let mut cmd = ffmpeg.ffmpeg_cmd();
        cmd.arg("-y")
            .arg("-i")
            .arg(&video_file)
            .arg("-vf")
            .arg(format!(
                "crop=min(iw\\,ih*(16/9)):ow/(16/9),scale={}:{}",
                target_width, target_height
            ))
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
            .arg(&temp_file)
            .arg("-hide_banner")
            .arg("-loglevel")
            .arg("error");

        ffmpeg.execute_command(cmd).await.map_err(|e| {
            let _ = std::fs::remove_file(&temp_file);
            VideoProcessorError::CroppingFailed {
                file: video_file.to_path_buf(),
                reason: e.to_string(),
            }
        })?;

        // Verify and atomically replace
        if temp_file.exists() && temp_file.metadata()?.len() > 0 {
            std::fs::rename(&temp_file, video_file)?;
            cropped_count += 1;
            info!("[CROP]   ✓ Cropped successfully");
        } else {
            let _ = std::fs::remove_file(&temp_file);
            return Err(VideoProcessorError::CroppingFailed {
                file: video_file.to_path_buf(),
                reason: "Output file not created properly".to_string(),
            });
        }
    }

    if cropped_count > 0 {
        info!(
            "[CROP] Cropping completed: {} file(s) cropped",
            cropped_count
        );
    } else {
        info!("[CROP] No files required cropping");
    }

    Ok(())
}
