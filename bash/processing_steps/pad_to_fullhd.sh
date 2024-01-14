#!/bin/bash
##
# Goes over the converted files and checks if there are any that are not exactly 1920x1080.
# These are padded with a more blurred background video.
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

    # Overlay the original video on top of a more blurred background
    ffmpeg -i "$file" -filter_complex "[0]scale=1920*2:1080*2,boxblur=luma_radius=min(h\,w)/20:luma_power=1:chroma_radius=min(cw\,ch)/20:chroma_power=1[bg];[0]scale=-1:1080[ov];[bg][ov]overlay=(W-w)/2:(H-h)/2:format=yuv444,crop=w=1920:h=1080" -c:v libx264 -preset veryfast -crf 23 -c:a aac -b:a 320k -movflags +faststart "${output_video_path}preprocessed/padded_${base_filename}" -hide_banner;

    # Delete the original file
    rm "$file"
  fi
done
