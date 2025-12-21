#!/bin/bash
##
# Goes over the converted files and checks if there are any that have a resolution
# more than fullHD. These are cropped where needed.
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

echo "[CROP] Starting crop check for oversized videos..."

# Check if preprocessed directory exists
if [ ! -d "${output_video_path}preprocessed" ]; then
  echo "[CROP] Preprocessed directory not found, skipping crop step"
  exit 0
fi

shopt -s nullglob
files=("${output_video_path}preprocessed/"*.mp4)
shopt -u nullglob

if [ ${#files[@]} -eq 0 ]; then
  echo "[CROP] No files to process"
  exit 0
fi

cropped_count=0

# Iterate over each file in the specified directory
for file in "${files[@]}"; do
  if [ ! -f "$file" ]; then
    continue
  fi
  
  # Get video dimensions safely
  width=$(ffprobe -v error -select_streams v:0 -show_entries stream=width -of default=nw=1:nk=1 "$file" 2>/dev/null || echo "1920")
  height=$(ffprobe -v error -select_streams v:0 -show_entries stream=height -of default=nw=1:nk=1 "$file" 2>/dev/null || echo "1080")
  size="${width}x${height}"

  # Only crop if resolution exceeds 1920x1080
  if [ "$width" -gt 1920 ] || [ "$height" -gt 1080 ]; then
    echo "[CROP] Found oversized video: $(basename "$file") (${size})"
    base_filename=$(basename "$file")
    temp_output="${output_video_path}preprocessed/cropped_${base_filename}"
    
    ffmpeg -y -i "$file" -vf "crop=min(iw\,ih*(16/9)):ow/(16/9)" \
      -c:v libx264 -preset veryfast -crf 18 -c:a aac -b:a 320k \
      "$temp_output" -hide_banner -loglevel error || {
      echo "[CROP]   ERROR: Failed to crop video" >&2
      exit 1
    }
    
    # Verify output was created successfully
    if [ -f "$temp_output" ] && [ -s "$temp_output" ]; then
      rm "$file" || {
        echo "[CROP]   WARNING: Failed to remove original file" >&2
      }
      ((cropped_count++))
      echo "[CROP]   ✓ Cropped successfully"
    else
      echo "[CROP]   ERROR: Output file not created properly" >&2
      rm -f "$temp_output"
      exit 1
    fi
  fi
done

if [ $cropped_count -gt 0 ]; then
  echo "[CROP] Cropping completed: ${cropped_count} file(s) cropped"
else
  echo "[CROP] No files required cropping"
fi
