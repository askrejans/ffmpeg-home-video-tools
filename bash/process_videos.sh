#!/bin/bash
##
# Master script to process videos using various sub-scripts.
##

## CONFIG - START
input_video_path="$1"
output_video_path="$2"
## CONFIG - END

# Ensure input and output paths are provided
if [ -z "$input_video_path" ] || [ -z "$output_video_path" ]; then
  echo "Usage: $0 <input_video_path> <output_video_path>"
  exit 1
fi

steps_dir="./processing_steps"

# Run each script sequentially in their respective subdirectories
$steps_dir/batch_convert_to_mp4.sh "$input_video_path" "$output_video_path"
$steps_dir/pad_to_fullhd.sh "$output_video_path/"
$steps_dir/crop_to_fullhd.sh "$output_video_path/"
$steps_dir/add_missing_audio_tracks.sh "$output_video_path/"
$steps_dir/resample_all_audio.sh "$output_video_path/"
$steps_dir/concat_videos.sh "$output_video_path/"

echo "Processing completed."
