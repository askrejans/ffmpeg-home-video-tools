# Changelog

## 0.4.0 — 2026-10-08

Shared rendering and soundtrack APIs for hosts that use native media codecs.

### New
- Public `audio` APIs mix decoded stereo 48 kHz float PCM, apply bounded clip levelling, equal-power overlaps and cut micro-fades, and normalize the complete soundtrack with EBU R128 measurement and a true-peak limiter. Processing streams from files, supports cancellation, and writes verified float WAV/RF64 output atomically.
- Public `compositor::blend` accepts opaque packed RGBA or BGRA frames and uses the same transition algorithms as the process renderer.
- Title templates support named caller-bound image layers, with cached cover/contain fitting. The process renderer supplies the first trimmed video frame to the `first_clip` slot.
- Optional `Clip.rotation` applies an additional clockwise multiple of 90 degrees after automatic source orientation. Planning, thumbnails, filmstrips, proxies and title pictures share this orientation rule.
- Public `encoders::video_bitrate_kbps` exposes the same encoding policy to native hosts.

### Changed
- Desktop soundtrack normalization now uses the same Rust PCM implementation as in-process hosts; FFmpeg decodes the inputs and encodes the finished soundtrack instead of applying its separate `loudnorm` filter.
- Standard 720p, 1080p and 2160p presets use explicit bitrate budgets, including portrait layouts; arbitrary canvases retain pixel-count scaling.

### Fixed
- Explicit encoder identifiers take precedence over codec aliases, so selecting the Media Foundation software path does not select its hardware variant.
- Variable fallback fonts honor the requested text weight.

### Rust API migration
- `Clip` adds `rotation`; prefer `Clip::new(path)` or add `rotation: 0` to existing struct literals.
- `VideoInfo` adds `manual_rotation`; add `manual_rotation: 0` to existing literals.
- `SegmentSource::Clip` adds `rotation`; update exhaustive field patterns or use `..` for fields the caller does not need.
- Existing Project and media JSON without the added rotation fields still deserialize with zero rotation. The crate remains GPL-3.0-only.

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
