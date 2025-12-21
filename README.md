# FFmpeg Home Video Tools

A video processing toolkit for standardizing and concatenating home videos to 4K UHD (3840x2160). Available in both Bash and Rust implementations.

## Quick Start

### Rust Version (Recommended)
```bash
# Build and install
cargo install --path .

# Process videos with TUI (if interactive terminal)
ffmpeg-video-processor process /path/to/input /path/to/output

# Or use CLI mode
ffmpeg-video-processor process /path/to/input /path/to/output --no-tui

# See all options
ffmpeg-video-processor --help
```

### Bash Version
```bash
cd bash
./process_videos.sh /path/to/input /path/to/output
```

## 🎯 What It Does

Standardizes diverse home video formats into a single, unified MP4 file:

1. **Batch Convert** - Standardize all videos to MP4 format (25fps PAL)
2. **Pad** - Add blurred background to non-standard aspect ratios
3. **Crop** - Crop oversized videos to target resolution
4. **Add Audio** - Generate silent audio for videos without audio
5. **Resample** - Normalize all audio to 48kHz stereo
6. **Concatenate** - Merge everything into final output

**Default Output Settings:**
- Resolution: 3840x2160 (4K UHD)
- Frame Rate: 25 fps (PAL standard)
- Video: H.264 (libx264), CRF 20, medium preset
- Audio: AAC 256kbps, 48kHz stereo

## Usage Warning

**⚠️ Important:** These tools modify video files. Always work on copies of your original files, not the originals themselves. Test on a small batch first!

## 📚 Bash Scripts Documentation

All Bash scripts are located in the `bash/` directory with production-hardening applied.

### Main Entry Point: `process_videos.sh`

The master script that orchestrates the entire pipeline with comprehensive logging and error handling:

```bash
cd bash
./process_videos.sh /path/to/input /path/to/output
```

**Features:**
- Pre-flight checks (FFmpeg availability, disk space)
- Detailed logging to `output/logs/process_YYYYMMDD_HHMMSS.log`
- Progress tracking for each step
- Automatic error detection and reporting
- Cleanup trap handlers

### Processing Steps

#### `batch_convert_to_mp4.sh`

This script identifies and batch converts files within a specified folder to high-quality, low-compression MP4 format with a unified Full HD resolution. The script performs the following actions based on the input file characteristics:

- Vertically rotated videos are upscaled to 1080p with blurred bars padded to the sides, encoded to MP4 with high quality/low compression.
- 1080p videos are reencoded to MP4 with high quality/low compression.
- Lower resolution videos are reencoded to 1080p upscaled MP4 with high quality/low compression.

Audio is consistently reencoded to 320k AAC, and the video frame rate is forced to 25 fps.

#### `pad_to_fullhd.sh`

Checks converted files for resolutions other than 1920x1080 and pads them with a blurred background.

#### `crop_to_fullhd.sh`

Crops videos where the resolution exceeds Full HD while maintaining 16:9 aspect ratio.

#### `add_missing_audio_tracks.sh`

Adds silent audio tracks to videos lacking audio, required for concatenation.

#### `resample_all_audio.sh`

Resamples audio using `aresample=async=1000` to prevent sync issues during concatenation.

#### `concat_videos.sh`

Concatenates all processed videos into a single timestamped output file and cleans up intermediate files.

## 🛠️ Requirements

- **FFmpeg** (with ffprobe) - Must be installed and in PATH
- **For Rust version:** Rust 1.70+ (for building from source)
- **For Bash version:** bash, bc (for frame rate calculations)

## 📖 Documentation

- **[Rust Implementation Guide](RUST_README.md)** - Complete documentation for the Rust version
- **[Example Configuration](config.example.toml)** - Sample configuration file
- **[Docker Support](Dockerfile)** - Containerized deployment

## 🤝 Contributing

Contributions are welcome! Please:
1. Fork the repository
2. Create a feature branch
3. Add tests for new functionality
4. Ensure all tests pass
5. Submit a pull request

## 📄 License

MIT License - See [LICENSE](LICENSE) file for details

## 🙏 Acknowledgments

Originally created as personal shell scripts, now evolved into a production-ready tool with both Bash and Rust implementations.

