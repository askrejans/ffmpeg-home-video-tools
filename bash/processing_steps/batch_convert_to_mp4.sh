#!/bin/bash
##
# Small shell script to identify and batch convert all files in the called folder 
# to high-quality/low-compression mp4 in unified fullHD resolution and 25fps.
# Useful for batch video standardization.
##

## CONFIG - START
input_video_path=$1
output_video_path=$2
converted_video_path="${output_video_path}/preprocessed/"
target_fps=25
## CONFIG - END

# Ensure input path is provided
if [ -z "$input_video_path" ] || [ -z "$output_video_path" ]; then
  echo "Usage: $0 <input_video_path> <output_video_path>"
  exit 1
fi

# Create "preprocessed" directory if it doesn't exist
mkdir -p "$converted_video_path"

# Iterate over each file in the specified directory
for file in "$input_video_path/"*.*; do
  # Check if there are matching files
  if [ -e "$file" ]; then
    base_filename=$(basename "$file")

    # Get input video frame rate
    input_fps=$(ffprobe -v error -select_streams v:0 -show_entries stream=r_frame_rate -of default=nw=1:nk=1 "$file")
    input_fps=$(printf "%.0f" $(echo "$input_fps" | bc -l))  # Convert to integer

    # Determine ffmpeg action based on video properties
    if ffprobe -v error -select_streams v:0 -show_entries stream_tags=rotate -of default=nw=1:nk=1 "$file" | grep -q '^90'; then
      # Vertically rotated video is upscaled to 1080p with black bars padded to the sides
      ffmpeg -i "$file" -vf "scale=w=1080:h=1920:force_original_aspect_ratio=1,pad=1080:1920:(ow-iw)/2:(oh-ih)/2" -c:v libx264 -preset veryfast -crf 18 -c:a aac -b:a 320k -r $target_fps "${converted_video_path}${base_filename}.mp4" -hide_banner
    elif [ "$input_fps" -eq "$target_fps" ]; then
      # Video already at target frame rate, reencode to mp4 with high quality/low compression
      ffmpeg -i "$file" -c:v libx264 -preset veryfast -crf 18 -c:a aac -b:a 320k -r $target_fps "${converted_video_path}${base_filename}.mp4" -hide_banner
    else
      # Lower-res video is reencoded to 1080p upscaled mp4 with high quality/low compression and converted to 25fps
      ffmpeg -i "$file" -vf "scale=1920:1080" -c:v libx264 -preset veryfast -crf 18 -c:a aac -b:a 320k -r $target_fps "${converted_video_path}${base_filename}.mp4" -hide_banner
    fi
  fi
done
