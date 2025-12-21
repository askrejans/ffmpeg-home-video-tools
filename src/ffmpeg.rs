use crate::error::{Result, VideoProcessorError};
use crate::types::{ProcessingConfig, VideoMetadata};
use std::path::Path;
#[cfg(test)]
use std::path::PathBuf;
use std::process::{Command, Stdio};
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
        let ffmpeg_path = Self::find_executable("ffmpeg")
            .ok_or(VideoProcessorError::FFmpegNotFound)?;
        let ffprobe_path = Self::find_executable("ffprobe")
            .ok_or(VideoProcessorError::FFprobeNotFound)?;

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
            .output()
            .map_err(|e| VideoProcessorError::FFmpegExecutionFailed(e.to_string()))?;

        let version_str = String::from_utf8_lossy(&output.stdout);
        Ok(version_str.lines().next().unwrap_or("Unknown").to_string())
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
        let fps_str = Self::extract_value(&video_info, "r_frame_rate=")
            .ok_or_else(|| {
                VideoProcessorError::FFprobeParseError("Failed to find frame rate".to_string())
            })?;
        let fps = Self::parse_frame_rate(&fps_str)?;

        // Parse duration
        let duration = Self::extract_value(&video_info, "duration=")
            .and_then(|s| s.parse::<f64>().ok())
            .unwrap_or(0.0);

        // Parse codec
        let codec = Self::extract_value(&video_info, "codec_name=")
            .unwrap_or("unknown")
            .to_string();

        // Parse rotation
        let rotation = Self::extract_value(&video_info, "TAG:rotate=")
            .and_then(|s| s.parse::<i32>().ok());

        // Get file size
        let file_size = std::fs::metadata(path)
            .map(|m| m.len())
            .unwrap_or(0);

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
                VideoProcessorError::FFprobeParseError(format!("Invalid fps denominator: {}", den))
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

    /// Build FFmpeg command for conversion
    pub fn build_convert_command(
        &self,
        input: &Path,
        output: &Path,
        metadata: &VideoMetadata,
        config: &ProcessingConfig,
    ) -> Command {
        let mut cmd = Command::new(&self.ffmpeg_path);
        
        cmd.arg("-y") // Overwrite output
            .arg("-noautorotate") // Prevent auto-rotation from metadata
            .arg("-i")
            .arg(input);

        // Apply video filters based on metadata
        if metadata.is_vertical() {
            // Vertical video needs blurred background padding
            // Scale background to 2x target size, apply blur, then overlay original centered
            
            let (target_w, target_h) = config.target_resolution;
            let blur_divisor = config.blur_radius_divisor;
            
            let filter = if metadata.is_rotated_vertical() {
                // Need to transpose first for rotated videos (90 or 270 degrees)
                format!(
                    "transpose=1,[0:v]scale={}*2:{}*2,boxblur=luma_radius=min(h\\,w)/{}:luma_power=1:chroma_radius=min(cw\\,ch)/{}:chroma_power=1[bg];[0:v]transpose=1,scale=-1:{}[ov];[bg][ov]overlay=(W-w)/2:(H-h)/2:format=yuv444,crop=w={}:h={},fps={}",
                    target_w,
                    target_h,
                    blur_divisor,
                    blur_divisor,
                    target_h,
                    target_w,
                    target_h,
                    config.target_fps
                )
            } else {
                // Already vertical, no transpose needed
                format!(
                    "[0:v]scale={}*2:{}*2,boxblur=luma_radius=min(h\\,w)/{}:luma_power=1:chroma_radius=min(cw\\,ch)/{}:chroma_power=1[bg];[0:v]scale=-1:{}[ov];[bg][ov]overlay=(W-w)/2:(H-h)/2:format=yuv444,crop=w={}:h={},fps={}",
                    target_w,
                    target_h,
                    blur_divisor,
                    blur_divisor,
                    target_h,
                    target_w,
                    target_h,
                    config.target_fps
                )
            };
            
            cmd.arg("-filter_complex").arg(filter);
        } else if metadata.fps.round() as u32 == config.target_fps
            && metadata.width == config.target_resolution.0
            && metadata.height == config.target_resolution.1
        {
            // Already correct resolution and fps, just re-encode
            // No filter needed
        } else {
            // Scale to target resolution with fps
            cmd.arg("-vf").arg(format!(
                "scale={}:{},fps={}",
                config.target_resolution.0, config.target_resolution.1, config.target_fps
            ));
        }

        // Video encoding settings
        cmd.arg("-c:v")
            .arg(&config.video_codec)
            .arg("-preset")
            .arg(&config.video_preset)
            .arg("-b:v")
            .arg(format!("{}M", config.intermediate_bitrate))
            .arg("-r")
            .arg(config.target_fps.to_string());

        // Audio encoding settings
        cmd.arg("-c:a")
            .arg(&config.audio_codec)
            .arg("-b:a")
            .arg(format!("{}k", config.audio_bitrate));

        cmd.arg(output)
            .arg("-hide_banner")
            .arg("-loglevel")
            .arg("error");

        cmd
    }

    /// Execute an FFmpeg command and return the output
    pub fn execute_command(&self, mut cmd: Command) -> Result<String> {
        debug!("Executing command: {:?}", cmd);

        let output = cmd
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .map_err(|e| VideoProcessorError::FFmpegExecutionFailed(e.to_string()))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(VideoProcessorError::FFmpegExecutionFailed(
                stderr.to_string(),
            ));
        }

        Ok(String::from_utf8_lossy(&output.stdout).to_string())
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
        assert_eq!(FFmpegWrapper::parse_frame_rate("30000/1001").unwrap(), 29.97003);
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
        // This test will pass if ffmpeg is in PATH, otherwise skip
        if let Some(path) = FFmpegWrapper::find_executable("ffmpeg") {
            assert!(!path.is_empty());
        }
    }

    // Mock tests for FFmpeg operations (require mockall in production tests)
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

        assert_eq!(metadata.aspect_ratio(), 1920.0 / 1080.0);
        assert!(!metadata.is_rotated_vertical());
    }

    #[test]
    fn test_video_metadata_rotation() {
        let metadata = VideoMetadata {
            path: PathBuf::from("test.mp4"),
            width: 1080,
            height: 1920,
            fps: 25.0,
            duration: 60.0,
            has_audio: true,
            rotation: Some(90),
            codec: "h264".to_string(),
            file_size: 1000000,
        };

        assert!(metadata.is_rotated_vertical());
    }
}
