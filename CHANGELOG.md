# Changelog

## 0.3.0 — 2026-10-06

A rewrite of the Rust tool as a library plus CLI.

### Licence
- The project is now licensed under **GPL-3.0-only** (previously BSD 2-Clause).

### New
- A library crate (`ffmpeg_video_processor`) with a JSON project format, typed events, stable error codes and cancellation.
- A frame-exact timeline renderer with one encoding pass. It uses hardware and OS encoders where available: VideoToolbox, NVENC, Quick Sync, AMF and Media Foundation, plus x264 and OpenH264.
- Transitions: cut, crossfade, fade to black or white, slide, wipe, zoom, blur dissolve, tape rewind, mix.
- Animated title intros from built-in or custom templates (`clean`, `cinematic`, `retro`).
- Two-pass EBU R128 loudness normalisation (`loudnorm` measure, then linear normalise) and per-clip levelling.
- Trimming, watermarks, presets (4K, 1080p, 720p, vertical 9:16, custom) and automatic frame rate.
- Deinterlacing, anamorphic pixel-aspect correction, HDR (HLG/PQ) tone-mapping and BT.709 colour tagging.
- Recursive folder scans that skip hidden files, AppleDouble files and camera sidecars, with photos, audio-only and broken files reported. Clips are ordered by recording time.
- Output verification before files are moved into place, and existing files are never overwritten.
- Preview helpers: thumbnails, filmstrips and scrub-friendly proxies.
- CLI commands `render`, `scan`, `probe`, `encoders`, `templates`, `thumbnail`, `filmstrip` and `proxy`, plus `--json` event output and `--dry-run`. There is a new full-screen progress view.
- Inherited-file-descriptor input access for sandboxed hosts.

### Fixed
- Inputs sharing a name stem (`clip.mov`, `clip.mp4`) silently dropped one video.
- Rotation was read from the legacy `rotate` tag that modern FFmpeg no longer writes; 180° clips could come out upside down.
- The blurred background used `boxblur`, which LGPL FFmpeg builds do not have.
- Clips were ordered byte-wise (`clip10` before `clip2`).
- Two or three lossy re-encodes per clip.
- The crop step never ran and assumed 16:9.
- A clip whose audio step failed to probe was deleted without notice.
- Partial files from interrupted runs were treated as finished.
- Stream duration `N/A` became 0 instead of using the container duration.
- The disk-space check looked at the wrong volume.
- Audio and video lengths were never equalised per clip, so A/V drift accumulated.
- No deinterlacing, pixel-aspect handling or HDR tone-mapping; no colour tags.
- Cover art could be picked as the video stream; `r_frame_rate` was used for variable-frame-rate footage.
- Every output was forced to 25 fps.
- Only six extensions and one folder level were searched, and an AppleDouble file aborted the whole run.
- No progress inside a file and no cancellation; the TUI tracked any process named "ffmpeg".
- Options that did nothing: `--jobs`, checkpoints and resume, several config keys, `--verbose`.
- Encoder options only worked with libx264.
- Intermediate files were written into the user's output folder.
- `Cargo.toml` and `LICENSE` disagreed, the README's Rust version was wrong, and the Dockerfile could not build.

### Removed
- The `resume` and `config` commands, the TOML configuration file and the checkpoint state. None of them worked. Use `--project` files instead.
- Dependencies `tokio`, `anyhow`, `toml`, `dirs`, `lazy_static`, `atty`, `sysinfo`, `regex`, `tracing-appender` and `mockall`.

### Updated
- Rust 1.99, edition 2024, and every dependency at its latest release.
