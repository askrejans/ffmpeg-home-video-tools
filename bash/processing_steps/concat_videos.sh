#!/bin/bash
##
# Creates a concat_list.txt of all the converted mp4's and joins them together in a concatenated mp4.
# This is ready for encoding in a lower bitrate if needed.
##

## CONFIG - START
output_video_path=$1
## CONFIG - END

# Ensure output path is provided
if [ -z "$output_video_path" ]; then
  echo "Usage: $0 <output_video_path>"
  exit 1
fi

# Create a temporary text file containing a list of mp4 files to join
for file in "${output_video_path}"*.mp4; do 
  base_filename=$(basename "$file")
  echo "file '$base_filename'" >> "${output_video_path}concat_list.txt"
done

# Check if there are files to concatenate
if [ -s "${output_video_path}concat_list.txt" ]; then
  # Use ffmpeg to concatenate the list of files and create the final joined video
  current_datetime=$(date "+%Y%m%d_%H%M%S")
  output_vod_name="processed_vod_${current_datetime}.mp4"

  ffmpeg -f concat -safe 0 -i "${output_video_path}concat_list.txt" -c copy "${output_video_path}${output_vod_name}"

  
  # Check if ffmpeg command succeeded
  if [ $? -eq 0 ]; then
    # Remove files with the correct prefix
    rm "${output_video_path}resampled_"*.mp4
    rm -Rf "${output_video_path}preprocessed"
    rm "${output_video_path}concat_list.txt"
    echo "Processing completed."
  else
    echo "Error: ffmpeg command failed."
  fi
else
  echo "No files to concatenate."
fi
