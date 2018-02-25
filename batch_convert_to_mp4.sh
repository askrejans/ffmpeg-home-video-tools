#!/bin/bash

##
# Main transcode script - transforms the zoo to mezzanine mp4's
##
##CONFIG - START
#Path to zoo videos
input_video_path="videos/"
#Path to videos to check
output_video_path="videos/converted/"
##CONFIG - END

for file in ${input_video_path}*.*
do 
currentDate=$(date +%s%3N)

eval $(ffprobe -v error -of flat=s=_ -select_streams v:0 -show_entries stream=height,width "$file")
size=${streams_stream_0_width}x${streams_stream_0_height};

rotate=$(ffprobe -loglevel error -select_streams v:0 -show_entries stream_tags=rotate -of default=nw=1:nk=1 "$file"
)

#logic for portrait videos - adds black bars. Upscales to fullHD.
if [ "${rotate}" == "90" ]; then

ffmpeg -i "$file" -r 25 -f mp4 -vf "scale=w=1920:h=1080:force_original_aspect_ratio=1,pad=1920:1080:(ow-iw)/2:(oh-ih)/2" -c:v libx264 -preset veryfast -crf 18 -c:a aac -b:a 320k "${output_video_path}${currentDate}.mp4" -hide_banner;

#Does no resizing for fullHD, just conversion.
elif [ "${size}" == "1920x1080" ]; then

ffmpeg -i "$file" -r 25 -f mp4 -c:v libx264 -preset veryfast -crf 18 -c:a aac -b:a 320k "${output_video_path}${currentDate}.mp4" -hide_banner;
#For all other cases upscales before conversion.
else
ffmpeg -i "$file" -r 25 -f mp4 -vf "scale=-1:1080:flags=lanczos" -c:v libx264 -preset veryfast -crf 18 -c:a aac -b:a 320k "${output_video_path}${currentDate}.mp4" -hide_banner;

fi
done
