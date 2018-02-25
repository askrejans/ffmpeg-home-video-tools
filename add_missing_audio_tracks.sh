#!/bin/bash
##
#Searches for files with missing audio and adds an empty track for these, so that concatenation does not loose all audio.
##
##CONFIG - START
converted_video_path="videos/converted/"
##CONFIG - END

for file in ${converted_video_path}*.mp4
do 

audio_track=$(ffprobe -i "$file" -show_streams 2>&1 | grep 'Stream #0:1')

if [[ -z "${audio_track// }" ]]; then

printf "Found file without an audio: ${file}}\n"
base_filename=$(basename $file)
ffmpeg -y -f lavfi -i anullsrc=channel_layout=stereo:sample_rate=44100 -i $file \
  -shortest -c:v copy -c:a aac "${converted_video_path}with_empty_audio_${base_filename}" -hide_banner

fi
done

