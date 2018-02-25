#!/bin/bash
##
#Resamples audio and adds compensation where needed. Useful if on normal concat audio goes out of sync.
##
##CONFIG - START
converted_video_path="videos/converted/"
##CONFIG - END

for file in ${converted_video_path}*.mp4
do 

base_filename=$(basename $file)

ffmpeg -i $file -c:v copy -c:a aac -af "aresample=async=1000" "${converted_video_path}resampled_${base_filename}" -hide_banner

done

