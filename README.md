# ffmpeg_batch_mp4_converter
Small shell script to identify and batch convert all files in called folder to high quality/low compression mp4 in unified fullHD resolution - useful for batch video standardization.

Make sure to create "converted" directory there before running script.
There are three poossible ffmpeg actions:

-Vertically rotated video is upscaled to 1080p and black bars are padded to the sides. All encoded to mp4 with high quality/low compression

-1080p video is reencoded to mp4 with high quality/low compression

-Lower res. video ir reencoded to 1080p upscaled mp4 with high quality/low compression.

In all cases audio is reencoded to 320k aac.
