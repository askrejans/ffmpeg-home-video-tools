#!/bin/bash
##
# Small shell script to identify and batch convert all files in the called folder 
# to high-quality/low-compression mp4 in unified fullHD resolution and 25fps.
# Useful for batch video standardization.
# Production-hardened version with error handling and progress tracking.
##

set -euo pipefail

## CONFIG - START
input_video_path="${1:-}"
output_video_path="${2:-}"
converted_video_path="${output_video_path}/preprocessed/"
target_fps=25
## CONFIG - END

# Ensure input path is provided
if [ -z "$input_video_path" ] || [ -z "$output_video_path" ]; then
  echo "Usage: $0 <input_video_path> <output_video_path>" >&2
  exit 1
fi

echo "[CONVERT] Starting batch conversion to MP4..."
echo "[CONVERT] Input: ${input_video_path}"
echo "[CONVERT] Output: ${converted_video_path}"

# Create "preprocessed" directory if it doesn't exist
mkdir -p "$converted_video_path" || {
  echo "[CONVERT] ERROR: Failed to create output directory" >&2
  exit 1
}

# Count total files for progress tracking
shopt -s nullglob
files=("$input_video_path/"*.{mp4,avi,mov,mkv,m4v,3gp,MP4,AVI,MOV,MKV,M4V,3GP})
shopt -u nullglob
total_files=${#files[@]}

if [ "$total_files" -eq 0 ]; then
  echo "[CONVERT] WARNING: No video files found in input directory" >&2
  exit 0
fi

echo "[CONVERT] Processing ${total_files} file(s)..."
current=0

# Iterate over each file in the specified directory
for file in "${files[@]}"; do
  ((current++))
  base_filename=$(basename "$file")

  ((current++))
  base_filename=$(basename "$file")
  base_filename_noext="${base_filename%.*}"
  output_file="${converted_video_path}${base_filename_noext}.mp4"
  
  echo "[CONVERT] [${current}/${total_files}] Processing: ${base_filename}"
  
  # Skip if output already exists
  if [ -f "$output_file" ]; then
    echo "[CONVERT]   Output already exists, skipping"
    continue
  fi
  
  # Validate input file is a video
  if ! ffprobe -v error -select_streams v:0 -show_entries stream=codec_type -of default=nw=1:nk=1 "$file" 2>/dev/null | grep -q 'video'; then
    echo "[CONVERT]   WARNING: Not a valid video file, skipping" >&2
    continue
  fi

  # Get input video frame rate
  input_fps=$(ffprobe -v error -select_streams v:0 -show_entries stream=r_frame_rate -of default=nw=1:nk=1 "$file" 2>/dev/null || echo "25/1")
  if command -v bc &> /dev/null; then
    input_fps=$(printf "%.0f" $(echo "$input_fps" | bc -l 2>/dev/null || echo "25"))
  else
    input_fps=25  # Default if bc not available
  fi

  # Determine ffmpeg action based on video properties
  rotation=$(ffprobe -v error -select_streams v:0 -show_entries stream_tags=rotate -of default=nw=1:nk=1 "$file" 2>/dev/null || echo "")
  
  if [[ "$rotation" == "90" ]]; then
    echo "[CONVERT]   Detected 90° rotation, scaling to 1080x1920 with padding"
    ffmpeg -y -i "$file" -vf "scale=w=1080:h=1920:force_original_aspect_ratio=1,pad=1080:1920:(ow-iw)/2:(oh-ih)/2" \
      -c:v libx264 -preset veryfast -crf 18 -c:a aac -b:a 320k -r $target_fps \
      "$output_file" -hide_banner -loglevel error || {
      echo "[CONVERT]   ERROR: Failed to convert rotated video" >&2
      exit 1
    }
  elif [ "$input_fps" -eq "$target_fps" ]; then
    echo "[CONVERT]   Already ${target_fps}fps, re-encoding to MP4"
    ffmpeg -y -i "$file" -c:v libx264 -preset veryfast -crf 18 -c:a aac -b:a 320k -r $target_fps \
      "$output_file" -hide_banner -loglevel error || {
      echo "[CONVERT]   ERROR: Failed to re-encode video" >&2
      exit 1
    }
  else
    echo "[CONVERT]   Scaling to 1920x1080 and converting to ${target_fps}fps"
    ffmpeg -y -i "$file" -vf "scale=1920:1080" -c:v libx264 -preset veryfast -crf 18 -c:a aac -b:a 320k -r $target_fps \
      "$output_file" -hide_banner -loglevel error || {
      echo "[CONVERT]   ERROR: Failed to scale and convert video" >&2
      exit 1
    }
  fi
  
  echo "[CONVERT]   ✓ Completed"
done

echo "[CONVERT] Batch conversion completed: ${current} file(s) processed"
