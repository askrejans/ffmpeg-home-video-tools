use crate::error::{Result, VideoProcessorError};
use crate::ffmpeg::FFmpegWrapper;
use crate::types::ProcessingConfig;
use std::path::Path;
use std::process::Command;
use tracing::{info, warn};
use walkdir::WalkDir;

/// Pad videos that aren't exactly target resolution with blurred background
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
        info!("[PAD] No files to process");
        return Ok(());
    }

    let mut padded_count = 0;
    let total_count = video_files.len();

    for (idx, video_file) in video_files.iter().enumerate() {
        progress_callback(crate::types::ProgressUpdate {
            step: crate::types::ProcessingStep::Pad,
            current: idx + 1,
            total: total_count,
            file_name: Some(video_file.file_name().unwrap().to_string_lossy().to_string()),
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

        let (target_width, target_height) = config.target_resolution;

        if metadata.width != target_width || metadata.height != target_height {
            info!(
                "[PAD] Found non-standard resolution: {} ({}x{})",
                video_file.file_name().unwrap().to_string_lossy(),
                metadata.width,
                metadata.height
            );

            let file_name = video_file.file_name().unwrap().to_string_lossy();
            let output_file = preprocessed_path.join(format!("padded_{}", file_name));

            // Build padding command with blurred background
            let filter_complex = format!(
                "[0]scale={}*2:{}*2,boxblur=luma_radius=min(h\\,w)/{}:luma_power=1:chroma_radius=min(cw\\,ch)/{}:chroma_power=1[bg];\
                 [0]scale=-1:{}[ov];\
                 [bg][ov]overlay=(W-w)/2:(H-h)/2:format=yuv444,crop=w={}:h={}",
                target_width,
                target_height,
                config.blur_radius_divisor,
                config.blur_radius_divisor,
                target_height,
                target_width,
                target_height
            );

            let mut cmd = Command::new("ffmpeg");
            cmd.arg("-y")
                .arg("-noautorotate")
                .arg("-i")
                .arg(&video_file)
                .arg("-filter_complex")
                .arg(&filter_complex)
                .arg("-c:v")
                .arg(&config.video_codec)
                .arg("-preset")
                .arg(&config.video_preset)
                .arg("-crf")
                .arg(config.padding_crf.to_string())
                .arg("-c:a")
                .arg(&config.audio_codec)
                .arg("-b:a")
                .arg(format!("{}k", config.audio_bitrate))
                .arg("-movflags")
                .arg("+faststart")
                .arg(&output_file)
                .arg("-hide_banner")
                .arg("-loglevel")
                .arg("error");

            ffmpeg.execute_command(cmd).map_err(|e| {
                VideoProcessorError::ConversionFailed {
                    file: video_file.to_path_buf(),
                    reason: e.to_string(),
                }
            })?;

            // Verify output and remove original
            if output_file.exists() && output_file.metadata()?.len() > 0 {
                std::fs::remove_file(&video_file)?;
                padded_count += 1;
                info!("[PAD]   ✓ Padded successfully");
            } else {
                return Err(VideoProcessorError::PaddingFailed {
                    file: video_file.to_path_buf(),
                    reason: "Output file not created properly".to_string(),
                });
            }
        }
    }

    if padded_count > 0 {
        info!("[PAD] Padding completed: {} file(s) padded", padded_count);
    } else {
        info!("[PAD] No files required padding");
    }

    Ok(())
}
