#!/bin/bash
##
# Goes over the converted files and checks if there are any that have a resolution
# more than fullHD. These are cropped where needed.
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
for file in "${output_video_path}preprocessed/"*.*; do 
  eval $(ffprobe -v error -of flat=s=_ -select_streams v:0 -show_entries stream=height,width "$file")
  size=${streams_stream_0_width}x${streams_stream_0_height};

  if [ "${size}" != "1920x1080" ]; then
    printf "Found non-standard aspect ratio: ${file}_${size}\n"
    base_filename=$(basename "$file")
    ffmpeg -i "$file" -vf "crop=min(iw\,ih*(16/9)):ow/(16/9)" -c:v libx264 -preset veryfast -crf 18 -c:a aac -b:a 320k "${output_video_path}preprocessed/cropped_${base_filename}" -hide_banner;
    # Delete the original file
    rm "$file"
  fi
done
