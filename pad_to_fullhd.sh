#!/bin/bash

##
# Goes over transcoded files to check for any non-standart not quite fullHD aspect ratios. For these files, padding is added and a new padded_ file version is created.
##
##CONFIG - START
#Path to videos to check
output_video_path="videos/converted/"
##CONFIG - END

for file in ${output_video_path}*.*
do 

eval $(ffprobe -v error -of flat=s=_ -select_streams v:0 -show_entries stream=height,width "$file")
size=${streams_stream_0_width}x${streams_stream_0_height};

if [ "${size}" != "1920x1080" ]; then
printf "Found non-standart aspect ratio: ${file}_${size}\n"
base_filename=$(basename $file)
ffmpeg -i "$file" -r 25 -f mp4 -vf "scale=iw*sar:ih , pad=max(iw\,ih*(16/9)):ow/(16/9):(ow-iw)/2:(oh-ih)/2" -aspect 16:9 -c:v libx264 -preset veryfast -crf 18 -c:a aac -b:a 320k "${output_video_path}padded_${base_filename}" -hide_banner;
fi

done
