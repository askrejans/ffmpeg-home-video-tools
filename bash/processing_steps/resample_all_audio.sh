#!/bin/bash
##
# Resamples audio to 48000 stereo and adds compensation where needed.
# Useful if normal concat produces out-of-sync audio and video.
##

## CONFIG - START
output_video_path=$1
## CONFIG - END

# Ensure output path is provided
if [ -z "$output_video_path" ]; then
  echo "Usage: $0 <output_video_path>"
  exit 1
fi

# Iterate over each .mp4 file in the specified directory
for file in "${output_video_path}preprocessed/"*.mp4; do 
  base_filename=$(basename "$file")
  ffmpeg -i "$file" -c:v copy -c:a aac -ar 48000 -ac 2 -af "aresample=async=1000" "${output_video_path}resampled_${base_filename}" -hide_banner
done
rm -Rf ${output_video_path}preprocessed