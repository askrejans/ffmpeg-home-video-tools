use crate::error::{Result, VideoProcessorError};
use crate::types::{ProcessingConfig, VideoMetadata};
use std::path::Path;
#[cfg(test)]
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::Duration;
use tracing::debug;

/// FFmpeg wrapper for executing video processing commands
#[derive(Debug, Clone)]
pub struct FFmpegWrapper {
    ffmpeg_path: String,
    ffprobe_path: String,
}

impl FFmpegWrapper {
    /// Create a new FFmpeg wrapper, verifying that ffmpeg and ffprobe are available
    pub fn new() -> Result<Self> {
        let ffmpeg_path =
            Self::find_executable("ffmpeg").ok_or(VideoProcessorError::FFmpegNotFound)?;
        let ffprobe_path =
            Self::find_executable("ffprobe").ok_or(VideoProcessorError::FFprobeNotFound)?;

        debug!("Found ffmpeg at: {}", ffmpeg_path);
        debug!("Found ffprobe at: {}", ffprobe_path);

        Ok(Self {
            ffmpeg_path,
            ffprobe_path,
        })
    }

    /// Find an executable in PATH
    fn find_executable(name: &str) -> Option<String> {
        #[cfg(target_os = "windows")]
        let name = format!("{}.exe", name);

        which::which(&name)
            .ok()
            .and_then(|p| p.to_str().map(String::from))
    }

    /// Get FFmpeg version
    #[allow(dead_code)]
    pub fn version(&self) -> Result<String> {
        let output = Command::new(&self.ffmpeg_path)
            .arg("-version")
            .stdin(Stdio::null())
            .output()
            .map_err(|e| VideoProcessorError::FFmpegExecutionFailed(e.to_string()))?;

        let version_str = String::from_utf8_lossy(&output.stdout);
        Ok(version_str.lines().next().unwrap_or("Unknown").to_string())
    }

    /// Create a new Command pre-configured with the correct ffmpeg path and stdin null
    pub fn ffmpeg_cmd(&self) -> Command {
        let mut cmd = Command::new(&self.ffmpeg_path);
        cmd.stdin(Stdio::null());
        cmd
    }

    /// Extract video metadata using ffprobe
    pub fn probe_video(&self, path: &Path) -> Result<VideoMetadata> {
        debug!("Probing video: {:?}", path);

        if !path.exists() {
            return Err(VideoProcessorError::InvalidVideoFile(path.to_path_buf()));
        }

        // Get video stream info
        let video_info = self.run_ffprobe(
            path,
            &[
                "-v",
                "error",
                "-select_streams",
                "v:0",
                "-show_entries",
                "stream=width,height,r_frame_rate,codec_name,duration",
                "-show_entries",
                "stream_tags=rotate",
                "-show_entries",
                "format=duration",
                "-of",
                "default=nw=1",
            ],
        )?;

        // Get audio stream count
        let audio_info = self.run_ffprobe(
            path,
            &[
                "-v",
                "error",
                "-select_streams",
                "a",
                "-show_entries",
                "stream=codec_type",
                "-of",
                "default=nw=1:nk=1",
            ],
        )?;

        let has_audio = !audio_info.trim().is_empty();

        // Parse dimensions
        let width = Self::extract_value(&video_info, "width=")
            .and_then(|s| s.parse::<u32>().ok())
            .ok_or_else(|| {
                VideoProcessorError::FFprobeParseError("Failed to parse width".to_string())
            })?;

        let height = Self::extract_value(&video_info, "height=")
            .and_then(|s| s.parse::<u32>().ok())
            .ok_or_else(|| {
                VideoProcessorError::FFprobeParseError("Failed to parse height".to_string())
            })?;

        // Parse frame rate
        let fps_str = Self::extract_value(&video_info, "r_frame_rate=").ok_or_else(|| {
            VideoProcessorError::FFprobeParseError("Failed to find frame rate".to_string())
        })?;
        let fps = Self::parse_frame_rate(fps_str)?;

        // Parse duration - try stream duration first, fall back to format duration
        let duration = Self::extract_value(&video_info, "duration=")
            .and_then(|s| s.parse::<f64>().ok())
            .unwrap_or(0.0);

        // Parse codec
        let codec = Self::extract_value(&video_info, "codec_name=")
            .unwrap_or("unknown")
            .to_string();

        // Parse rotation from stream tags
        let rotation = Self::extract_value(&video_info, "TAG:rotate=")
            .and_then(|s| s.parse::<i32>().ok());

        // Get file size
        let file_size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);

        Ok(VideoMetadata {
            path: path.to_path_buf(),
            width,
            height,
            fps,
            duration,
            has_audio,
            rotation,
            codec,
            file_size,
        })
    }

    /// Run ffprobe with given arguments
    fn run_ffprobe(&self, input: &Path, args: &[&str]) -> Result<String> {
        let mut cmd = Command::new(&self.ffprobe_path);
        cmd.args(args);
        cmd.arg(input);
        cmd.stdin(Stdio::null());

        let output = cmd
            .output()
            .map_err(|e| VideoProcessorError::FFmpegExecutionFailed(e.to_string()))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(VideoProcessorError::FFprobeParseError(stderr.to_string()));
        }

        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    }

    /// Extract value from ffprobe output
    fn extract_value<'a>(output: &'a str, key: &str) -> Option<&'a str> {
        output
            .lines()
            .find(|line| line.starts_with(key))
            .and_then(|line| line.strip_prefix(key))
    }

    /// Parse frame rate from "num/den" format
    fn parse_frame_rate(fps_str: &str) -> Result<f32> {
        if let Some((num, den)) = fps_str.split_once('/') {
            let numerator = num.parse::<f32>().map_err(|_| {
                VideoProcessorError::FFprobeParseError(format!("Invalid fps numerator: {}", num))
            })?;
            let denominator = den.parse::<f32>().map_err(|_| {
                VideoProcessorError::FFprobeParseError(format!(
                    "Invalid fps denominator: {}",
                    den
                ))
            })?;

            if denominator == 0.0 {
                return Err(VideoProcessorError::FFprobeParseError(
                    "FPS denominator is zero".to_string(),
                ));
            }

            Ok(numerator / denominator)
        } else {
            fps_str.parse::<f32>().map_err(|_| {
                VideoProcessorError::FFprobeParseError(format!("Invalid fps format: {}", fps_str))
            })
        }
    }

    /// Build FFmpeg command for format normalization (convert step).
    /// Handles: codec normalization, rotation correction, fps normalization.
    /// Does NOT handle resolution — that's the pad step's job.
    pub fn build_convert_command(
        &self,
        input: &Path,
        output: &Path,
        metadata: &VideoMetadata,
        config: &ProcessingConfig,
    ) -> Command {
        let mut cmd = self.ffmpeg_cmd();

        cmd.arg("-y"); // Overwrite output

        // Prevent FFmpeg from auto-rotating based on metadata — we handle rotation explicitly
        if metadata.rotation.is_some() {
            cmd.arg("-noautorotate");
        }

        cmd.arg("-i").arg(input);

        // Build video filter chain
        let mut vf_parts: Vec<String> = Vec::new();

        // Handle rotation: transpose the video to correct orientation
        if metadata.is_rotated_vertical() {
            let transpose_dir = match metadata.rotation {
                Some(90) => "1",  // 90° CW → transpose=1
                Some(270) => "2", // 270° CW → transpose=2
                _ => "1",
            };
            vf_parts.push(format!("transpose={}", transpose_dir));
        }

        // Normalize fps
        vf_parts.push(format!("fps={}", config.target_fps));

        cmd.arg("-vf").arg(vf_parts.join(","));

        // Video encoding settings — use CRF for quality-based encoding
        cmd.arg("-c:v")
            .arg(&config.video_codec)
            .arg("-preset")
            .arg(&config.video_preset)
            .arg("-crf")
            .arg(config.video_crf.to_string())
            .arg("-pix_fmt")
            .arg("yuv420p");

        // Audio encoding settings
        cmd.arg("-c:a")
            .arg(&config.audio_codec)
            .arg("-b:a")
            .arg(format!("{}k", config.audio_bitrate));

        cmd.arg("-movflags")
            .arg("+faststart")
            .arg(output)
            .arg("-hide_banner")
            .arg("-loglevel")
            .arg("error");

        cmd
    }

    /// Build the filter_complex string for blurred-background padding.
    /// Works for any input resolution/aspect ratio → target resolution.
    pub fn build_pad_filter(config: &ProcessingConfig) -> String {
        let (tw, th) = config.target_resolution;
        let blur = config.blur_radius_divisor;

        // Split input into two streams:
        //   bg_in → scale up to cover target area, crop to exact target, then blur
        //   fg_in → scale to fit within target area (maintain aspect ratio)
        // Then overlay fg centered on bg
        format!(
            "[0:v]split=2[bg_in][fg_in];\
             [bg_in]scale={tw}:{th}:force_original_aspect_ratio=increase,\
             crop={tw}:{th},\
             boxblur=luma_radius=min(h\\,w)/{blur}:luma_power=1:\
             chroma_radius=min(cw\\,ch)/{blur}:chroma_power=1[bg];\
             [fg_in]scale={tw}:{th}:force_original_aspect_ratio=decrease:\
             force_divisible_by=2[fg];\
             [bg][fg]overlay=(W-w)/2:(H-h)/2",
            tw = tw,
            th = th,
            blur = blur,
        )
    }

    /// Build FFmpeg command for padding a video with blurred background
    pub fn build_pad_command(
        &self,
        input: &Path,
        output: &Path,
        config: &ProcessingConfig,
    ) -> Command {
        let mut cmd = self.ffmpeg_cmd();

        let filter = Self::build_pad_filter(config);

        cmd.arg("-y")
            .arg("-i")
            .arg(input)
            .arg("-filter_complex")
            .arg(&filter)
            .arg("-c:v")
            .arg(&config.video_codec)
            .arg("-preset")
            .arg(&config.video_preset)
            .arg("-crf")
            .arg(config.padding_crf.to_string())
            .arg("-pix_fmt")
            .arg("yuv420p")
            .arg("-c:a")
            .arg(&config.audio_codec)
            .arg("-b:a")
            .arg(format!("{}k", config.audio_bitrate))
            .arg("-movflags")
            .arg("+faststart")
            .arg(output)
            .arg("-hide_banner")
            .arg("-loglevel")
            .arg("error");

        cmd
    }

    /// Execute an FFmpeg command asynchronously (non-blocking on the tokio runtime).
    /// Returns stdout on success, error with stderr on failure.
    pub async fn execute_command(&self, mut cmd: Command) -> Result<String> {
        debug!("Executing command: {:?}", cmd);

        // Ensure stdin is null to prevent FFmpeg from blocking on input
        cmd.stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        // Run in a blocking thread so we don't block the tokio runtime (TUI stays responsive)
        let result = tokio::task::spawn_blocking(move || cmd.output())
            .await
            .map_err(|e| VideoProcessorError::JoinError(e.to_string()))?
            .map_err(|e| VideoProcessorError::FFmpegExecutionFailed(e.to_string()))?;

        if !result.status.success() {
            let stderr = String::from_utf8_lossy(&result.stderr);
            return Err(VideoProcessorError::FFmpegExecutionFailed(
                stderr.to_string(),
            ));
        }

        Ok(String::from_utf8_lossy(&result.stdout).to_string())
    }

    /// Execute a command with a timeout (in seconds)
    #[allow(dead_code)]
    pub async fn execute_command_with_timeout(
        &self,
        cmd: Command,
        timeout_secs: u64,
    ) -> Result<String> {
        tokio::time::timeout(Duration::from_secs(timeout_secs), self.execute_command(cmd))
            .await
            .map_err(|_| VideoProcessorError::Timeout { timeout_secs })?
    }

    /// Check if ffmpeg and ffprobe are available
    pub fn check_availability() -> Result<()> {
        Self::new().map(|_| ())
    }
}

impl Default for FFmpegWrapper {
    fn default() -> Self {
        Self::new().expect("FFmpeg not found")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_frame_rate() {
        assert_eq!(FFmpegWrapper::parse_frame_rate("25/1").unwrap(), 25.0);
        assert_eq!(
            FFmpegWrapper::parse_frame_rate("30000/1001").unwrap(),
            29.97003
        );
        assert_eq!(FFmpegWrapper::parse_frame_rate("24.0").unwrap(), 24.0);
        assert!(FFmpegWrapper::parse_frame_rate("invalid").is_err());
        assert!(FFmpegWrapper::parse_frame_rate("25/0").is_err());
    }

    #[test]
    fn test_extract_value() {
        let output = "width=1920\nheight=1080\ncodec_name=h264";
        assert_eq!(FFmpegWrapper::extract_value(output, "width="), Some("1920"));
        assert_eq!(
            FFmpegWrapper::extract_value(output, "codec_name="),
            Some("h264")
        );
        assert_eq!(FFmpegWrapper::extract_value(output, "fps="), None);
    }

    #[test]
    fn test_find_executable() {
        if let Some(path) = FFmpegWrapper::find_executable("ffmpeg") {
            assert!(!path.is_empty());
        }
    }

    #[test]
    fn test_video_metadata_aspect_ratio() {
        let metadata = VideoMetadata {
            path: PathBuf::from("test.mp4"),
            width: 1920,
            height: 1080,
            fps: 25.0,
            duration: 60.0,
            has_audio: true,
            rotation: None,
            codec: "h264".to_string(),
            file_size: 1000000,
        };

        assert!((metadata.aspect_ratio() - 1920.0 / 1080.0).abs() < 0.001);
        assert!(!metadata.is_rotated_vertical());
        assert!(!metadata.is_vertical());
        assert_eq!(metadata.effective_dimensions(), (1920, 1080));
    }

    #[test]
    fn test_video_metadata_rotation() {
        let metadata = VideoMetadata {
            path: PathBuf::from("test.mp4"),
            width: 1920,
            height: 1080,
            fps: 25.0,
            duration: 60.0,
            has_audio: true,
            rotation: Some(90),
            codec: "h264".to_string(),
            file_size: 1000000,
        };

        assert!(metadata.is_rotated_vertical());
        assert!(metadata.is_vertical());
        // Effective dimensions should be swapped for rotated video
        assert_eq!(metadata.effective_dimensions(), (1080, 1920));
    }

    #[test]
    fn test_video_metadata_natural_vertical() {
        let metadata = VideoMetadata {
            path: PathBuf::from("test.mp4"),
            width: 1080,
            height: 1920,
            fps: 30.0,
            duration: 10.0,
            has_audio: true,
            rotation: None,
            codec: "h264".to_string(),
            file_size: 500000,
        };

        assert!(!metadata.is_rotated_vertical());
        assert!(metadata.is_vertical());
        assert_eq!(metadata.effective_dimensions(), (1080, 1920));
    }

    #[test]
    fn test_build_pad_filter_contains_required_parts() {
        let config = ProcessingConfig::default();
        let filter = FFmpegWrapper::build_pad_filter(&config);

        // Must use split to create two streams from one input
        assert!(filter.contains("[0:v]split=2[bg_in][fg_in]"));
        // Background: scale up to cover, crop, blur
        assert!(filter.contains("force_original_aspect_ratio=increase"));
        assert!(filter.contains("boxblur"));
        assert!(filter.contains("crop=3840:2160"));
        // Foreground: scale to fit
        assert!(filter.contains("force_original_aspect_ratio=decrease"));
        assert!(filter.contains("force_divisible_by=2"));
        // Overlay centered
        assert!(filter.contains("overlay=(W-w)/2:(H-h)/2"));
    }

    #[test]
    fn test_build_convert_command_handles_rotation() {
        if FFmpegWrapper::find_executable("ffmpeg").is_none() {
            return; // skip if ffmpeg not installed
        }
        let ffmpeg = FFmpegWrapper::new().unwrap();
        let config = ProcessingConfig::default();

        // Rotated 90° video
        let metadata = VideoMetadata {
            path: PathBuf::from("input.mp4"),
            width: 1920,
            height: 1080,
            fps: 30.0,
            duration: 10.0,
            has_audio: true,
            rotation: Some(90),
            codec: "h264".to_string(),
            file_size: 1000000,
        };

        let cmd = ffmpeg.build_convert_command(
            Path::new("input.mp4"),
            Path::new("output.mp4"),
            &metadata,
            &config,
        );

        let cmd_debug = format!("{:?}", cmd);
        // Should use -noautorotate for rotated videos
        assert!(cmd_debug.contains("noautorotate"));
        // Should have transpose in the -vf filter
        assert!(cmd_debug.contains("transpose=1"));
        // Should NOT have filter_complex (that's the pad step's job)
        assert!(!cmd_debug.contains("filter_complex"));
    }

    #[test]
    fn test_build_convert_command_no_rotation() {
        if FFmpegWrapper::find_executable("ffmpeg").is_none() {
            return;
        }
        let ffmpeg = FFmpegWrapper::new().unwrap();
        let config = ProcessingConfig::default();

        let metadata = VideoMetadata {
            path: PathBuf::from("input.mp4"),
            width: 1920,
            height: 1080,
            fps: 25.0,
            duration: 10.0,
            has_audio: true,
            rotation: None,
            codec: "h264".to_string(),
            file_size: 1000000,
        };

        let cmd = ffmpeg.build_convert_command(
            Path::new("input.mp4"),
            Path::new("output.mp4"),
            &metadata,
            &config,
        );

        let cmd_debug = format!("{:?}", cmd);
        // Should NOT use -noautorotate when there's no rotation
        assert!(!cmd_debug.contains("noautorotate"));
        // Should have fps filter
        assert!(cmd_debug.contains("fps=25"));
    }
}
