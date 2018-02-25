# ffmpeg home video tools

This is a small colection of useful shell scripts that utilize ffmpeg and assist in creating an order in the usual modern format zoo of the home video world. This came as a result of a personal need and worked well to bring a bit of an order to the chaos of my personal home video collection. For now there is no unified solution - scripts can be run as needed with careful inspection of the results until the desired is reached. In the future might create a more universal and configurable ffmpeg wrapper/tool.

## Main script batch_convert_to_mp4.sh
Small shell script to identify and batch convert all files in called folder to high quality/low compression mp4 in unified fullHD resolution - useful for batch video standardization.

Make sure to create "converted" directory there before running script, as for now the default path assumes videos to be stored relative to the scripts in the video directory and converted ones are created in video/converted. Feel free to change it to fit your needs.

There are three poossible ffmpeg actions in the main script:

-Vertically rotated video is upscaled to 1080p and black bars are padded to the sides. All encoded to mp4 with high quality/low compression

-1080p video is reencoded to mp4 with high quality/low compression

-Lower res. video ir reencoded to 1080p upscaled mp4 with high quality/low compression.

In all cases audio is reencoded to 320k aac. Video frame rate is forced to 25 fps.

This script takes care of the most cases, however some extra cases remain. For now there are two helper scripts to assist:

## pad_to_fullhd.sh
Goes over the converted files and checks if there are any that are not exactly 1920x1080. These are padded with black bars where needed. On some edge cases it does not work as intended for now.

## crop_to_fullhd.sh
Does the same as the padding script, however it crops video where the resoltion is more than fullhd. Can take care of most cases where padding won't do. Will create an universal script in the future.

## add_missing_audio_tracks.sh
As not all video contain audio, however for concat all files need an audio track, this script adds 0 audio to such tracks, so that everythin works.

Next stage is concatenation:
## concat_videos.sh
This creates a concat_list.txt of all the converted mp4's and joins them together in concatenated mp4. This is ready for encoding in a lower bitrate if needed.

#WARNING - use this at your own risk and carefully. This has so far been only tested to my own needs, so data loss is possible if you are not thinking what you are doing. I am not responsible for any lost data when using this. There might be some edge cases still, where everything goes horribly wrong. Please do tests on small batches before.
