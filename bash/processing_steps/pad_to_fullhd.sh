#!/bin/bash
##
# Goes over the converted files and checks if there are any that are not exactly 1920x1080.
# These are padded with a more blurred background video.
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

echo "[PAD] Starting padding check for non-1920x1080 videos..."

# Check if preprocessed directory exists
if [ ! -d "${output_video_path}preprocessed" ]; then
  echo "[PAD] Preprocessed directory not found, skipping padding step"
  exit 0
fi

shopt -s nullglob
files=("${output_video_path}preprocessed/"*.mp4)
shopt -u nullglob

if [ ${#files[@]} -eq 0 ]; then
  echo "[PAD] No files to process"
  exit 0
fi

padded_count=0

# Iterate over each file in the specified directory
for file in "${files[@]}"; do
  if [ ! -f "$file" ]; then
    continue
  fi
  
  # Get video dimensions safely
  width=$(ffprobe -v error -select_streams v:0 -show_entries stream=width -of default=nw=1:nk=1 "$file" 2>/dev/null || echo "1920")
  height=$(ffprobe -v error -select_streams v:0 -show_entries stream=height -of default=nw=1:nk=1 "$file" 2>/dev/null || echo "1080")
  size="${width}x${height}"

  if [ "${size}" != "1920x1080" ]; then
    echo "[PAD] Found non-standard resolution: $(basename "$file") (${size})"
    base_filename=$(basename "$file")
    temp_output="${output_video_path}preprocessed/padded_${base_filename}"

    # Overlay the original video on top of a more blurred background
    # gblur is available in every FFmpeg build (boxblur needs a GPL build).
    ffmpeg -y -i "$file" -filter_complex "[0]split[a][b];[a]scale=1920:1080:force_original_aspect_ratio=increase,crop=1920:1080,gblur=sigma=40[bg];[b]scale=1920:1080:force_original_aspect_ratio=decrease:force_divisible_by=2[ov];[bg][ov]overlay=(W-w)/2:(H-h)/2,setsar=1" \
      -c:v libx264 -preset veryfast -crf 23 -c:a aac -b:a 320k -movflags +faststart \
      "$temp_output" -hide_banner -loglevel error || {
      echo "[PAD]   ERROR: Failed to pad video" >&2
      exit 1
    }

    # Verify output was created successfully
    if [ -f "$temp_output" ] && [ -s "$temp_output" ]; then
      rm "$file" || {
        echo "[PAD]   WARNING: Failed to remove original file" >&2
      }
      ((padded_count++))
      echo "[PAD]   ✓ Padded successfully"
    else
      echo "[PAD]   ERROR: Output file not created properly" >&2
      rm -f "$temp_output"
      exit 1
    fi
  fi
done

if [ $padded_count -gt 0 ]; then
  echo "[PAD] Padding completed: ${padded_count} file(s) padded"
else
  echo "[PAD] No files required padding"
fi
