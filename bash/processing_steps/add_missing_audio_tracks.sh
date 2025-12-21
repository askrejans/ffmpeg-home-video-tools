#!/bin/bash
##
# Adds zero audio tracks to videos without audio, making them ready for concatenation.
# Production-hardened version with error handling and progress tracking.
##

set -euo pipefail

## CONFIG - START
output_video_path="${1:-}"
## CONFIG - END

# Ensure output path is provided
if [ -z "$output_video_path" ]; then
  echo "Usage: $0 <output_video_path>" >&2
  exit 1
fi

echo "[AUDIO] Checking for videos without audio tracks..."

shopt -s nullglob
files=("${output_video_path}"*.mp4)
shopt -u nullglob

if [ ${#files[@]} -eq 0 ]; then
  echo "[AUDIO] No MP4 files found in output directory"
  exit 0
fi

added_count=0

# Iterate over each file in the specified directory
for file in "${files[@]}"; do
  if [ ! -f "$file" ]; then
    continue
  fi
  
  # Check for audio stream more reliably
  audio_streams=$(ffprobe -v error -select_streams a -show_entries stream=codec_type -of default=nw=1:nk=1 "$file" 2>/dev/null | wc -l | xargs)

  if [ "$audio_streams" -eq 0 ]; then
    echo "[AUDIO] Found file without audio: $(basename "$file")"
    base_filename=$(basename "$file")
    temp_output="${output_video_path}with_empty_audio_${base_filename}"
    
    ffmpeg -y -f lavfi -i anullsrc=channel_layout=stereo:sample_rate=48000 -i "$file" \
      -shortest -c:v copy -c:a aac \
      "$temp_output" -hide_banner -loglevel error || {
      echo "[AUDIO]   ERROR: Failed to add audio track" >&2
      exit 1
    }
    
    # Verify output was created successfully
    if [ -f "$temp_output" ] && [ -s "$temp_output" ]; then
      ((added_count++))
      echo "[AUDIO]   ✓ Silent audio track added"
    else
      echo "[AUDIO]   ERROR: Output file not created properly" >&2
      rm -f "$temp_output"
      exit 1
    fi
  fi
done

if [ $added_count -gt 0 ]; then
  echo "[AUDIO] Audio track addition completed: ${added_count} file(s) processed"
else
  echo "[AUDIO] All files already have audio tracks"
fi
