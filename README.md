# FFmpeg Home Video Tools

Welcome to the FFmpeg Home Video Tools repository, a curated collection of shell scripts designed to streamline and enhance your home video organization using FFmpeg. These scripts have been developed to address the challenges of managing a diverse range of video formats commonly found in personal collections.

## Usage Warning

Please exercise caution and use these scripts at your own risk. While they have been tested to meet personal requirements, there is a potential risk of data loss if not used carefully. Ensure thorough testing on small batches before applying these tools to larger datasets.

## Getting Started

All Bash scripts are located in the "bash" directory. The primary entry point is the `process_videos.sh` script, which serves as the main interface for handling input and output parameters.

```bash
sh ./process_videos.sh input_folder output_folder
```

## Main Script: `batch_convert_to_mp4.sh`

This script identifies and batch converts files within a specified folder to high-quality, low-compression MP4 format with a unified Full HD resolution. The script performs the following actions based on the input file characteristics:

- Vertically rotated videos are upscaled to 1080p with blurred bars padded to the sides, encoded to MP4 with high quality/low compression.
- 1080p videos are reencoded to MP4 with high quality/low compression.
- Lower resolution videos are reencoded to 1080p upscaled MP4 with high quality/low compression.

Audio is consistently reencoded to 320k AAC, and the video frame rate is forced to 25 fps.

**Note:** Ensure a "converted" directory is created before running the script.

## Helper Scripts

### `pad_to_fullhd.sh`

This script checks converted files for resolutions other than 1920x1080 and pads them with black bars as needed. Some edge cases may not work as intended.

### `crop_to_fullhd.sh`

Similar to the padding script, this script crops video where the resolution exceeds Full HD. A more universal script is planned for future releases.

### `add_missing_audio_tracks.sh`

For videos lacking audio tracks, this script adds a silent audio track to facilitate concatenation.

### `resample_all_audio.sh`

If audio and video sync issues arise during concatenation, running this script can help by resampling audio using `aresample=async=1000`. Adjust the file selection configuration in `concat_videos.sh` if this step is skipped.

## Concatenation
```markdown
Script: `concat_videos.sh`

This script generates a `concat_list.txt` of all converted MP4 files and merges them into a concatenated MP4. This file is ready for further encoding at a lower bitrate if required.

## Master Script: `process_videos.sh`

This master script orchestrates the sequential execution of the above scripts, ensuring a smooth processing flow. Provide input and output paths as parameters:

```bash
sh ./process_videos.sh input_folder output_folder
```

Feel free to explore and adapt these tools to suit your specific needs. Your feedback and contributions are welcome!
