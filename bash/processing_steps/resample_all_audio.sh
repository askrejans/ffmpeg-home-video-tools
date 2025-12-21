#!/bin/bash
##
# Resamples audio to 48000 stereo and adds compensation where needed.
# Useful if normal concat produces out-of-sync audio and video.
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

echo "[RESAMPLE] Starting audio resampling to 48kHz stereo..."

# Check if preprocessed directory exists
if [ ! -d "${output_video_path}preprocessed" ]; then
  echo "[RESAMPLE] Preprocessed directory not found, skipping resample step"
  exit 0
fi

shopt -s nullglob
files=("${output_video_path}preprocessed/"*.mp4)
shopt -u nullglob

if [ ${#files[@]} -eq 0 ]; then
  echo "[RESAMPLE] No files to process"
  exit 0
fi

total_files=${#files[@]}
current=0

echo "[RESAMPLE] Processing ${total_files} file(s)..."

# Iterate over each .mp4 file in the specified directory
for file in "${files[@]}"; do
  ((current++))
  base_filename=$(basename "$file")
  output_file="${output_video_path}resampled_${base_filename}"
  
  echo "[RESAMPLE] [${current}/${total_files}] Processing: ${base_filename}"
  
  # Skip if already processed
  if [ -f "$output_file" ]; then
    echo "[RESAMPLE]   Already resampled, skipping"
    continue
  fi
  
  ffmpeg -y -i "$file" -c:v copy -c:a aac -ar 48000 -ac 2 -af "aresample=async=1000" \
    "$output_file" -hide_banner -loglevel error || {
    echo "[RESAMPLE]   ERROR: Failed to resample audio" >&2
    exit 1
  }
  
  # Verify output
  if [ ! -f "$output_file" ] || [ ! -s "$output_file" ]; then
    echo "[RESAMPLE]   ERROR: Output file not created properly" >&2
    exit 1
  fi
  
  echo "[RESAMPLE]   ✓ Completed"
done

echo "[RESAMPLE] Audio resampling completed: ${current} file(s) processed"

# Clean up preprocessed directory
if [ -d "${output_video_path}preprocessed" ]; then
  echo "[RESAMPLE] Cleaning up preprocessed directory..."
  rm -Rf "${output_video_path}preprocessed" || {
    echo "[RESAMPLE] WARNING: Failed to remove preprocessed directory" >&2
  }
fi