# FFmpeg Home Video Processor

A video processing tool built in Rust. Standardizes and concatenates home videos from diverse formats into a single **4K UHD (3840x2160)** MP4 file optimized for TV playback.

## Features

### Core Functionality
- **Interactive TUI** with real-time progress tracking, FFmpeg logs, and configuration display
- **Batch video conversion** to standardized 4K UHD MP4 at 25fps (PAL)
- **Intelligent padding** for non-standard aspect ratios with blurred backgrounds
- **Smart cropping** for oversized videos
- **Audio normalization** with silent track generation for videos without audio
- **Audio resampling** to 48kHz stereo with async compensation
- **Video concatenation** into a single output file
- **TV-optimized encoding** - CRF 20, medium preset, great quality without excessive bitrate

## Installation

### Prerequisites
- **FFmpeg** and **ffprobe** must be installed and available in PATH
- Rust 1.70+ (for building from source)

### From Source
```bash
git clone https://github.com/askrejans/ffmpeg-home-video-tools.git
cd ffmpeg-home-video-tools
cargo build --release
```

The compiled binary will be at `target/release/ffmpeg-video-processor`

### Installation
```bash
cargo install --path .
```

## Usage

### TUI Mode (Interactive)

Launch the interactive terminal UI:
```bash
ffmpeg-video-processor process /path/to/input /path/to/output
```

**TUI Features:**
- Set input/output paths interactively (press 'i' for input, 'o' for output)
- Real-time progress bars for overall and per-step processing
- Live FFmpeg log viewer
- Configuration display panel
- Tab between panels (Main, Logs, Config)
- Start processing with 's', quit with 'q'

### CLI Mode (with progress bars)
```bash
ffmpeg-video-processor process /path/to/input /path/to/output --no-tui
```

### Basic Usage

Process videos with default 4K settings:
```bash
ffmpeg-video-processor process /path/to/input /path/to/output
```

### CLI Mode (with progress bars)
```bash
ffmpeg-video-processor process /path/to/input /path/to/output --no-tui
```

### Processing Profiles

**Fast** (CRF 24, faster preset - good for testing):
```bash
ffmpeg-video-processor process input/ output/ --profile fast
```

**Balanced** (CRF 20, medium preset - default, TV-optimized):
```bash
ffmpeg-video-processor process input/ output/ --profile balanced
```

**Quality** (CRF 18, slow preset - maximum quality):
```bash
ffmpeg-video-processor process input/ output/ --profile quality
```

### Advanced Options

Keep intermediate files for debugging:
```bash
ffmpeg-video-processor process input/ output/ --keep-intermediates
```

Parallel processing (4 jobs):
```bash
ffmpeg-video-processor process input/ output/ --jobs 4
```

Dry run (validate without processing):
```bash
ffmpeg-video-processor process input/ output/ --dry-run
```

Use custom configuration:
```bash
ffmpeg-video-processor process input/ output/ --config custom-config.toml
```

### Resume from Checkpoint

If processing is interrupted, resume from the last checkpoint:
```bash
ffmpeg-video-processor resume --checkpoint /path/to/output/processing_checkpoint.json
```

### Validate Videos

Check video files without processing:
```bash
ffmpeg-video-processor validate /path/to/input
```

### Configuration Management

Show current configuration:
```bash
ffmpeg-video-processor config show
```

Create default configuration file:
```bash
ffmpeg-video-processor config init
```

Show configuration file path:
```bash
ffmpeg-video-processor config path
```

## Configuration

The tool uses a TOML configuration file. Create one with `config init` or manually:

```toml
[processing]
# 4K UHD Resolution
target_resolution = [3840, 2160]

# PAL standard for TV
target_fps = 25

# H.264 codec for wide compatibility
video_codec = "libx264"

# Medium preset - good balance for 4K
video_preset = "medium"

# CRF 20 - excellent quality without huge files
video_crf = 20

# Higher CRF for blurred padding regions
padding_crf = 24

# AAC audio
audio_codec = "aac"

# 256kbps - sufficient for stereo TV audio
audio_bitrate = 256

# Standard video audio sample rate
audio_sample_rate = 48000

# Stereo
audio_channels = 2

# Padding blur settings
blur_radius_divisor = 20
async_compensation = 1000

# 0 = auto-detect CPU count
parallel_jobs = 0

# Keep intermediate files for debugging
keep_intermediates = false

[logging]
level = "info"  # trace, debug, info, warn, error
log_to_file = true
log_dir = null  # null = use output directory

[behavior]
checkpoint_enabled = true
auto_resume = false
cleanup_on_success = true
min_disk_space_gb = 10
```

### Profile Presets

**Fast Profile:**
- Preset: faster
- CRF: 24 (video), 28 (padding)
- Audio bitrate: 192 kbps

**Balanced Profile (Default):**
- Preset: medium
- CRF: 20 (video), 24 (padding)
- Audio bitrate: 256 kbps

**Quality Profile:**
- Preset: slow
- CRF: 18 (video), 22 (padding)
- Audio bitrate: 320 kbps

**Quality Profile:**
- Preset: slow
- CRF: 18 (video), 22 (padding)
- Audio bitrate: 320 kbps

## Processing Pipeline

1. **Batch Convert** - Standardize all videos to MP4 format
   - Handles rotated videos (90° detection)
   - Scales to 3840x2160 (4K UHD)
   - Converts to 25fps
   - High-quality encoding (CRF 18)

2. **Pad** - Add blurred background to non-standard aspect ratios
   - Creates 2x scaled blurred background
   - Overlays original centered
   - Crops to exact 1920x1080

3. **Crop** - Crop oversized videos
   - Maintains 16:9 aspect ratio
   - Preserves maximum content

4. **Add Audio** - Generate silent audio for videos without audio
   - Creates stereo 48kHz silent track
   - Required for concatenation

5. **Resample** - Normalize all audio
   - Resamples to 48kHz stereo
   - Applies async compensation (prevents sync issues)

6. **Concatenate** - Merge all videos
   - Creates single output file with timestamp
   - Cleans up intermediate files (optional)

## Output

The final output will be named:
```
processed_vod_YYYYMMDD_HHMMSS.mp4
```

Intermediate files (if kept):
- `preprocessed/*.mp4` - Converted videos
- `resampled_*.mp4` - Audio-resampled videos
- `with_empty_audio_*.mp4` - Videos with added silent audio
- `processing_checkpoint.json` - Resume checkpoint

Logs:
- `logs/process_YYYYMMDD_HHMMSS.log` - Detailed processing log

## Development

### Running Tests
```bash
cargo test
```

### Running with Debug Logging
```bash
RUST_LOG=debug ffmpeg-video-processor process input/ output/
```

### Building for Release
```bash
cargo build --release --target x86_64-unknown-linux-gnu
```

## Architecture

### Module Structure
- `main.rs` - Entry point and logging setup
- `cli.rs` - Command-line interface with clap
- `tui.rs` - Terminal UI with ratatui (future implementation)
- `config.rs` - Configuration management with TOML
- `error.rs` - Custom error types with thiserror
- `types.rs` - Core data structures
- `ffmpeg.rs` - FFmpeg wrapper with command building
- `orchestrator.rs` - Pipeline orchestration with checkpoint/resume
- `processing/` - Individual processing steps
  - `convert.rs` - Video format conversion
  - `pad.rs` - Padding with blurred background
  - `crop.rs` - Cropping oversized videos
  - `audio.rs` - Audio track addition and resampling
  - `concat.rs` - Video concatenation

## License

MIT License - See LICENSE file for details

## Contributing

Contributions welcome! Please:
1. Fork the repository
2. Create a feature branch
3. Add tests for new functionality
4. Ensure all tests pass
5. Submit a pull request

## Bash Scripts

The original bash scripts are still available in the `bash/` directory with production hardening:
- Comprehensive error handling with `set -euo pipefail`
- Progress output for each step
- Input validation and pre-flight checks
- Detailed logging to files
- Cleanup on failure

To use the bash version:
```bash
cd bash
./process_videos.sh /path/to/input /path/to/output
```
