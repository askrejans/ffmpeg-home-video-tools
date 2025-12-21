#!/bin/bash
##
# Creates a concat_list.txt of all the converted mp4's and joins them together in a concatenated mp4.
# This is ready for encoding in a lower bitrate if needed.
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

echo "[CONCAT] Starting video concatenation..."

concat_list="${output_video_path}concat_list.txt"

# Remove existing concat list if present
rm -f "$concat_list"

# Create a temporary text file containing a list of mp4 files to join
shopt -s nullglob
files=("${output_video_path}"*.mp4)
shopt -u nullglob

if [ ${#files[@]} -eq 0 ]; then
  echo "[CONCAT] ERROR: No MP4 files found to concatenate" >&2
  exit 1
fi

echo "[CONCAT] Found ${#files[@]} file(s) to concatenate"

# Build concat list (sorted by name for consistency)
for file in "${files[@]}"; do
  base_filename=$(basename "$file")
  # Escape single quotes in filename
  escaped_filename="${base_filename//\'/\'\\\'\'}"
  echo "file '${escaped_filename}'" >> "$concat_list"
done

# Verify concat list was created
if [ ! -s "$concat_list" ]; then
  echo "[CONCAT] ERROR: Failed to create concat list" >&2
  exit 1
fi

echo "[CONCAT] Concatenation list created with ${#files[@]} file(s)"

# Use ffmpeg to concatenate the list of files and create the final joined video
current_datetime=$(date "+%Y%m%d_%H%M%S")
output_vod_name="processed_vod_${current_datetime}.mp4"
output_vod_path="${output_video_path}${output_vod_name}"

echo "[CONCAT] Creating final video: ${output_vod_name}"

ffmpeg -y -f concat -safe 0 -i "$concat_list" -c copy \
  "$output_vod_path" -hide_banner -loglevel error || {
  echo "[CONCAT] ERROR: ffmpeg concatenation failed" >&2
  exit 1
}

# Verify output was created
if [ ! -f "$output_vod_path" ] || [ ! -s "$output_vod_path" ]; then
  echo "[CONCAT] ERROR: Output file not created or is empty" >&2
  exit 1
fi

echo "[CONCAT] ✓ Concatenation successful: ${output_vod_name}"

# Get final file size for reporting
if command -v du &> /dev/null; then
  file_size=$(du -h "$output_vod_path" | cut -f1)
  echo "[CONCAT] Final file size: ${file_size}"
fi

# Clean up intermediate files
echo "[CONCAT] Cleaning up intermediate files..."

shopt -s nullglob
resampled_files=("${output_video_path}resampled_"*.mp4)
empty_audio_files=("${output_video_path}with_empty_audio_"*.mp4)
shopt -u nullglob

removed_count=0

for file in "${resampled_files[@]}" "${empty_audio_files[@]}"; do
  if [ -f "$file" ]; then
    rm "$file" && ((removed_count++)) || {
      echo "[CONCAT] WARNING: Failed to remove $(basename "$file")" >&2
    }
  fi
done

if [ -d "${output_video_path}preprocessed" ]; then
  rm -Rf "${output_video_path}preprocessed" || {
    echo "[CONCAT] WARNING: Failed to remove preprocessed directory" >&2
  }
fi

if [ -f "$concat_list" ]; then
  rm "$concat_list" || {
    echo "[CONCAT] WARNING: Failed to remove concat list" >&2
  }
fi

echo "[CONCAT] Cleanup completed: ${removed_count} intermediate file(s) removed"
echo "[CONCAT] Final output: ${output_vod_path}"
