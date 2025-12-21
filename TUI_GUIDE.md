# TUI Guide - Interactive Video Processing

This guide covers the full-featured Terminal User Interface (TUI) for the FFmpeg Home Video Processor.

## Quick Start

Launch the TUI by running:
```bash
ffmpeg-video-processor process /path/to/input /path/to/output
```

Or launch with empty paths to set them interactively:
```bash
ffmpeg-video-processor process . .
```

## TUI Layout

The interface is divided into three main areas:

```
┌─────────────────────────────────────────────────────────┐
│              FFmpeg Video Processor                     │
└─────────────────────────────────────────────────────────┘
┌──────────────────────┬──────────────────────────────────┐
│                      │                                  │
│   Control Panel      │   Logs / Configuration Panel     │
│   - Paths            │   - Processing Logs              │
│   - Progress Bars    │   - FFmpeg Output                │
│   - Current Op       │   - Config Display               │
│                      │                                  │
└──────────────────────┴──────────────────────────────────┘
┌─────────────────────────────────────────────────────────┐
│                      Status Bar                         │
└─────────────────────────────────────────────────────────┘
```

## Keyboard Controls

### Main Navigation
- **`i`** - Set input path (opens text input mode)
- **`o`** - Set output path (opens text input mode)
- **`s`** - Start processing (once paths are set)
- **`Tab`** - Switch between panels (Main → Logs → Config → Main)
- **`q`** - Quit application (only when not processing)
- **`Ctrl+C`** - Force quit

### Text Input Mode
When editing paths:
- **Type** - Enter your path
- **Enter** - Confirm and save the path
- **Esc** - Cancel without saving
- **Backspace** - Delete last character

## Panel Descriptions

### Left Panel: Control & Progress

**Paths & Controls Section:**
- Displays current input and output directory paths
- Shows keyboard shortcuts
- Updates status when paths are set

**Progress Section:**
- **Overall Progress Bar** - Shows total pipeline completion (0-100%)
- **Current Step Bar** - Shows progress of the active processing step
- Color-coded: Green for overall, Cyan for current step

**Current Operation:**
- Displays what's currently being processed
- Shows filenames being converted
- Updates in real-time during processing

### Right Panel: Logs & Configuration

Switch between two views using **Tab**:

**Logs View:**
- Real-time FFmpeg processing logs
- Timestamped entries (HH:MM:SS format)
- Auto-scrolls to show latest entries
- Keeps up to 1000 log entries
- Shows:
  - Step transitions
  - Files being processed
  - FFmpeg operations
  - Errors and warnings

**Configuration View:**
- Current processing settings
- Output specifications:
  - Resolution: 3840x2160 (4K UHD)
  - Frame Rate: 25 fps
  - Video Codec: H.264 (libx264)
  - Video Preset: medium
  - Video CRF: 20
  - Audio: AAC 256kbps, 48kHz stereo
- Note about editing config.toml for changes

## Processing Workflow

### 1. Launch the Application
```bash
ffmpeg-video-processor process . .
```

### 2. Set Input Directory
1. Press **`i`**
2. Type the full path to your video folder (e.g., `/Users/john/Videos/Raw`)
3. Press **Enter** to confirm
4. The status bar will show "Input path: /Users/john/Videos/Raw"

### 3. Set Output Directory
1. Press **`o`**
2. Type the full path for output (e.g., `/Users/john/Videos/Processed`)
3. Press **Enter** to confirm
4. The status bar will show "Output path: /Users/john/Videos/Processed"

### 4. Start Processing
1. Press **`s`** to start
2. Watch the progress bars update in real-time
3. Monitor logs for detailed operation info
4. Switch to Config panel (Tab) to review settings anytime

### 5. Monitor Progress

The TUI shows:
- **Overall Progress**: Current position in the 6-step pipeline
- **Step Progress**: Completion of current operation (e.g., "Batch Convert: 45.3%")
- **Current File**: Name of the video being processed
- **Live Logs**: FFmpeg operations and status updates

### 6. Completion

When processing finishes:
- Overall progress reaches 100%
- Final log entry shows "Processing complete"
- You can press **`q`** to quit

## Processing Steps (6 Total)

The pipeline processes videos through these steps:

1. **Batch Convert** (16.7% of total)
   - Standardizes all videos to MP4
   - Handles rotation (90° vertical videos)
   - Converts to 25fps, 4K resolution

2. **Pad to 4K** (33.3% of total)
   - Adds blurred background padding to non-standard resolutions
   - Maintains aspect ratio with professional-looking bars

3. **Crop to 4K** (50.0% of total)
   - Crops oversized videos to 3840x2160
   - Preserves 16:9 aspect ratio

4. **Add Missing Audio** (66.7% of total)
   - Detects videos without audio tracks
   - Generates silent stereo tracks where needed

5. **Resample Audio** (83.3% of total)
   - Normalizes all audio to 48kHz stereo
   - Applies async compensation for sync

6. **Concatenate Videos** (100% of total)
   - Merges all processed videos into final output
   - Creates timestamped output filename
   - Cleans up intermediate files

## Tips & Best Practices

### Path Entry
- Use absolute paths for reliability
- Tab completion works in your shell before launching
- The `~` shortcut works for home directory

### Monitoring
- Press **Tab** to switch between Logs and Config views
- Logs auto-scroll, so latest info is always visible
- Watch for any FFmpeg errors in the log stream

### Performance
- 4K encoding is CPU-intensive
- Progress bars may pause during intensive operations
- Log updates continue even when progress seems stalled
- Consider using `--profile fast` for testing

### Storage
- Ensure adequate disk space (4K files are large!)
- Default behavior cleans up intermediate files
- Use `--keep-intermediates` flag if you need to debug

### Interruption Handling
- Processing can be resumed using checkpoints
- If you force quit (Ctrl+C), use the resume command:
  ```bash
  ffmpeg-video-processor resume --checkpoint /path/to/output/processing_checkpoint.json
  ```

## Configuration Customization

To change processing settings:

1. Generate a config file:
   ```bash
   ffmpeg-video-processor config init
   ```

2. Edit the config file (shown in Config panel):
   ```bash
   # Location shown with:
   ffmpeg-video-processor config path
   ```

3. Modify settings in `config.toml`:
   ```toml
   [processing]
   target_resolution = [3840, 2160]  # Change to 1920x1080 for Full HD
   video_crf = 20  # Lower = better quality (18), higher = smaller files (24)
   video_preset = "medium"  # faster, fast, medium, slow, slower
   audio_bitrate = 256  # Increase to 320 for better audio
   ```

4. Launch with custom config:
   ```bash
   ffmpeg-video-processor process input/ output/ --config config.toml
   ```

## Troubleshooting

### TUI Doesn't Launch
- Ensure you have a compatible terminal (iTerm2, Terminal.app, Alacritty, etc.)
- Force CLI mode if needed: `--no-tui` flag

### Garbled Display
- Resize terminal window (minimum 80x24 recommended)
- Try a different terminal emulator
- Check terminal supports Unicode/UTF-8

### Progress Stuck
- Check logs panel for errors
- FFmpeg may be working on a large file
- CPU-intensive steps take time on 4K

### Can't Set Paths
- Make sure you're in Normal mode (press Esc)
- Verify paths exist and are accessible
- Check permissions on directories

### Processing Errors
- Review logs panel for FFmpeg error messages
- Verify FFmpeg is installed: `ffmpeg -version`
- Check input videos are valid: use `validate` command first

## Advanced Usage

### Pre-validation
Before starting TUI processing, validate your videos:
```bash
ffmpeg-video-processor validate /path/to/input
```

### Profile Selection
Launch with different quality profiles:
```bash
# Fast (for testing)
ffmpeg-video-processor process input/ output/ --profile fast

# Quality (maximum quality)
ffmpeg-video-processor process input/ output/ --profile quality
```

### Parallel Processing
Speed up encoding with multiple jobs:
```bash
ffmpeg-video-processor process input/ output/ --jobs 4
```

### Debugging
Keep intermediate files to investigate issues:
```bash
ffmpeg-video-processor process input/ output/ --keep-intermediates
```

## CLI Mode Alternative

If you prefer traditional CLI with progress bars instead of TUI:
```bash
ffmpeg-video-processor process input/ output/ --no-tui
```

CLI mode provides:
- Simple progress bars
- Text output to terminal
- Better for scripts and automation
- Works in any terminal environment

## Color Coding

The TUI uses colors to convey information:
- **Cyan** - Titles and current step progress
- **Green** - Overall progress, success messages
- **White** - General text and information
- **Yellow** - Status bar, input prompts, warnings
- **Red** - (Reserved for errors)

## Conclusion

The TUI provides a modern, intuitive interface for video processing with real-time feedback. It's designed for both quick batch jobs and long-running 4K encoding tasks, giving you full visibility into the FFmpeg processing pipeline.

For more information:
- See `RUST_README.md` for detailed CLI documentation
- See `README.md` for project overview
- See `config.example.toml` for all configuration options
