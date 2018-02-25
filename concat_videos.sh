#!/bin/bash

##
# Generates a tmp text file of all mp4 in the converted directory and outputs merged video to indicated path.
##
##CONFIG - START
#Base converted video path
converted_video_path="videos/converted/"
#Path where to store generated list of files to join.
concat_file="concat_list.txt"
#Output path for the final joined video.
output_video_path="${converted_video_path}concat_video.mp4"
##CONFIG - END

for f in ${converted_video_path}*.mp4; do echo "file '$f'" >> $concat_file; done
ffmpeg -f concat -safe 0 -i $concat_file -c copy $output_video_path
