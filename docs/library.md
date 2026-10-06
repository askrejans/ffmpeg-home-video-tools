# Library reference

Crate: `ffmpeg-video-processor` (library name `ffmpeg_video_processor`). Disable default features to drop the CLI dependencies:

```toml
ffmpeg-video-processor = { git = "https://github.com/askrejans/ffmpeg-home-video-tools", default-features = false, features = ["builtin-templates"] }
```

| Feature | Default | Adds |
|---|---|---|
| `cli` | yes | the `ffmpeg-video-processor` binary (clap, indicatif) |
| `tui` | yes | full-screen progress view (ratatui, crossterm) |
| `builtin-templates` | yes | the `clean`, `cinematic` and `retro` title templates and their fonts |

Everything is synchronous. Long operations take a `&CancelToken` and an event callback, so run them on a worker thread.

## Tools

```rust
let tools = FfmpegTools::locate(&ToolPaths { ffmpeg: Some(path), ffprobe: None })?
    .with_input_access(InputAccess::InheritedFd); // optional
```

**Lookup order:**
1. Explicit paths.
2. `ffprobe` next to an explicit `ffmpeg`.
3. The `FFMPEG_PATH` / `FFPROBE_PATH` environment variables.
4. `PATH`.

`InputAccess::InheritedFd` (Unix) opens every input in the calling process and passes it to FFmpeg as `-fd N -i fd:`. Use it when child processes cannot open user files, for example from an app sandbox. Files the job creates itself are always passed by path inside its work folder.

**Introspection:** `tools.version()`, `tools.filters()`, `tools.encoders()`, `tools.has_filter(name)`.

## Discovering clips

```rust
let items = discover::scan(&tools, &[folder], &ScanOptions::default(), &cancel, &|item| { /* live */ })?;
let mut videos: Vec<MediaInfo> = items.into_iter().filter_map(|i| i.media).collect();
discover::sort_media(&mut videos, SortOrder::Recorded);
```

**`ScanItem.kind` values:**
- `video`
- `photo`
- `audio_only`
- `skipped`: documents, camera sidecars `.lrv/.thm/.lrf`, system files
- `unreadable`: with a reason

Hidden files and AppleDouble `._*` files inside folders are not listed. `discover::collect` expands inputs without probing.

**Probing a single file:** `probe(&tools, path, &cancel)` returns a `MediaInfo`:
- `kind`, `format`, `duration`, `size_bytes`.
- `video`: stream index, codec, coded and displayed size, pixel aspect, clockwise `rotation`, `frame_rate` (average), `interlaced`, `hdr` (`hlg`/`pq`), `pix_fmt`.
- `audio`: the default track, otherwise the first.
- `audio_streams`.
- `recorded_at` plus `recorded_at_source`:
  - `metadata`: QuickTime creation date, or `creation_time` if in 1980..now.
  - `file_name`: patterns such as `VID_20230714_153012` or `2023-07-14 15.30.12`.
  - `file_modified`.

## Project JSON

```jsonc
{
  "clips": [                                   // required, in playback order
    { "path": "a.mov", "trim": { "start": 1.5, "end": 9.0 } },   // trim optional; end optional
    { "path": "b.mp4" }
  ],
  "intro": {                                   // optional
    "template": "cinematic",                   // built-in name or template folder path
    "fields": { "title": "…", "subtitle": "…", "date": "…" }
  },
  "transition": { "kind": "crossfade", "duration": 1.0 },
  // kinds: cut, crossfade, fade_black, fade_white, slide, wipe, zoom, blur_dissolve, tape_rewind, mix
  "output": {
    "path": "movie.mp4",                       // ".mp4" added if missing; numbered if it exists
    "preset": "1080p",                         // 4k | 1080p | 720p | vertical | custom
    "width": 854, "height": 480,               // only for "custom" (even, 64..8192)
    "fps": "auto",                             // "auto" or 24 | 25 | 30 | 50 | 60
    "quality": "standard",                     // draft | standard | high
    "encoder": null,                           // force e.g. "h264_videotoolbox" or "libx264"
    "overwrite": false,
    "title": "Summer"                          // written to file metadata
  },
  "audio": {
    "level_clips": true,                       // per-clip loudness levelling (±12 dB max)
    "loudness": { "integrated": -23.0, "true_peak": -1.0 }   // null = no EBU R128 pass
  },
  "watermark": {                               // optional PNG burned into every frame
    "image": "logo.png", "anchor": "bottom_right",           // top_left | top_right | bottom_left | bottom_right
    "width_fraction": 0.22, "margin_fraction": 0.03, "opacity": 0.85
  }
}
```

`Project::from_json` parses and validates. `Project::validate` checks trims, sizes, frame rate, loudness range and watermark values.

**Timing rules:**
- **Clip length:** every clip contributes `round(seconds × fps)` frames, at least 1.
- **Transitions:**
  - A transition overlaps the end of one clip with the start of the next, for `round(duration × fps)` frames.
  - It is capped at 40 % of the shorter clip.
  - Joins shorter than 2 frames become cuts, and `mix` cycles through a fixed tasteful sequence.
- **Audio:** audio is 48 kHz with exactly `48000 / fps` samples per frame.

`plan(&project, &media, intro_seconds)` returns the `RenderPlan` (segments, joins, `total_frames`, warnings) without rendering.

## Rendering

```rust
let outcome = render(&tools, &project, &RenderOptions::default(), &mut |e| tx.send(e).unwrap(), &cancel)?;
// or, with probe results you already have:
let outcome = render_probed(&tools, &project, &media, &options, &mut sink, &cancel)?;
```

**`RenderOptions`:**
- `work_dir`: parent for the job's temporary folder; defaults to the system temp folder.
- `keep_work_dir`.
- `no_hardware_decode`.
- `intro_only`: render just the intro, as a title preview.

**`RenderOutcome`:** `output`, `width`, `height`, `fps`, `frames`, `duration_seconds`, `size_bytes`, `video_encoder`, `audio_encoder`, `loudness` (pass-1 input I/TP/LRA, pass-2 output I/TP, `linear` or `dynamic`) and `warnings`.

**What a render does:**
1. Probe and plan.
2. Pick the best working encoders:
   - Video: one trial encode per canvas size, cached.
   - Audio: `aac_at`, then `aac_mf`, then `aac`.
3. Check free space in the work folder and the destination.
4. Render the soundtrack.
5. Stream video frames into one encoder.
6. Verify the result.
7. Move it into place (copy, then rename across volumes).

The work folder is always removed, including on failure and cancellation.

### Events

Each event is serialised as JSON with a `type` tag. The CLI prints them with `--json`.

| `type` | Fields |
|---|---|
| `stage` | `stage`: `preparing`, `audio`, `loudness`, `video`, `verifying`, `finishing` |
| `progress` | `fraction` (whole job 0..1), `stage`, `done`, `total`, `fps`, `eta_seconds`, `clip` (index into `clips`) |
| `warning` | `code`, `message`, `clip` |
| `done` | `output`, `duration_seconds`, `size_bytes` |

**Warning codes:**
- `transition_shortened`
- `hdr_not_tonemapped`: the FFmpeg build lacks `zscale`/`tonemap`.
- `several_audio_tracks`
- `clip_ended_early`: the decoder produced fewer frames than the container promised, so the last picture was held.

### Error codes

`Error::code()` returns:
- `tool_not_found`
- `tool_failed`
- `clip_unreadable`
- `no_clips`
- `invalid_project`
- `template_invalid`
- `encoder_unavailable`
- `disk_full`
- `output_invalid`
- `cancelled`
- `io`
- `invalid_json`

## Previews

```rust
preview::thumbnail(&tools, &media, at_seconds, width, out_jpg, &cancel)?;
preview::filmstrip(&tools, &media, count, height, out_dir, prefix, &cancel)?;      // prefix-001.jpg …
preview::proxy(&tools, &media, height, out_mp4, &mut sink, &cancel)?;             // H.264, ½-second GOP
```

**What previews do:**
- Upright and square-pixel, deinterlaced and tone-mapped like the render.
- Written to a temporary sibling and renamed, so a half-written file never appears.

## Encoders

**Choosing an encoder:**
- `encoders::select_video_encoder(&tools, w, h, forced, &cancel)` picks the best working H.264 encoder for this machine.
- `encoders::working_video_encoders` lists every one that passes a trial.

**Candidates, best first:**

| Platform | Encoders |
|---|---|
| macOS | `h264_videotoolbox`, `libx264`, `libopenh264` |
| Windows | `h264_nvenc`, `h264_qsv`, `h264_amf`, `h264_mf` (hardware), `h264_mf` (software), `libx264`, `libopenh264` |
| Linux | `h264_nvenc`, `libx264`, `libopenh264` |

**Bitrate:** 16 Mbit/s for 1080p30, scaled by pixel count^0.8, ×1.5 at 50/60 fps. Draft is ×0.35 and High ×1.4.
