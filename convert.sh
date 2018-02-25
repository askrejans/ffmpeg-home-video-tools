#!/bin/bash
for file in *.*
do 
currentDate=$(date +%s%3N)

eval $(ffprobe -v error -of flat=s=_ -select_streams v:0 -show_entries stream=height,width "$file")
size=${streams_stream_0_width}x${streams_stream_0_height};

rotate=$(ffprobe -loglevel error -select_streams v:0 -show_entries stream_tags=rotate -of default=nw=1:nk=1 "$file"
)

if [ "${rotate}" == "90" ]; then

ffmpeg -i "$file" -f mp4 -filter_complex '[0:v]scale=ih*16/9:-1,boxblur=luma_radius=min(h\,w)/20:luma_power=1:chroma_radius=min(cw\,ch)/20:chroma_power=1[bg];[bg][0:v]overlay=(W-w)/2:(H-h)/2,crop=h=iw*9/16' -vcodec libx264 -crf 18 -preset veryfast -profile:v main -acodec aac "converted/${currentDate}.mp4" -hide_banner;

elif [ "${size}" == "1920x1080" ]; then
ffmpeg -i "$file" -f mp4 -vcodec libx264 -crf 18 -preset veryfast -profile:v main -acodec aac "converted/${currentDate}.mp4" -hide_banner;

else
ffmpeg -i "$file" -f mp4 -vf scale=-1:1080:flags=neighbor -vcodec libx264 -crf 18 -preset veryfast -profile:v main -acodec aac "converted/${currentDate}.mp4" -hide_banner;

fi
done
