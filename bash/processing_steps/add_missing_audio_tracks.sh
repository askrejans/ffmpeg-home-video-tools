#!/bin/bash
##
# Adds zero audio tracks to videos without audio, making them ready for concatenation.
##

## CONFIG - START
output_video_path=$1
## CONFIG - END

# Ensure output path is provided
if [ -z "$output_video_path" ]; then
  echo "Usage: $0 <output_video_path>"
  exit 1
fi

# Iterate over each file in the specified directory
for file in "${output_video_path}"*.mp4; do 
  audio_track=$(ffprobe -i "$file" -show_streams 2>&1 | grep 'Stream #0:1')

  if [[ -z "${audio_track// }" ]]; then
    printf "Found file without audio: ${file}\n"
    base_filename=$(basename "$file")
    ffmpeg -y -f lavfi -i anullsrc=channel_layout=stereo:sample_rate=48000 -i "$file" -shortest -c:v copy -c:a aac "${output_video_path}with_empty_audio_${base_filename}" -hide_banner
  fi
done
