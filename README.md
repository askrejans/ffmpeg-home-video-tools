# FFmpeg Home Video Tools

Turn a folder of mixed home videos into **one polished movie**.

`ffmpeg-video-processor` takes clips from phones, camcorders, DVDs, drones and old cameras, in any format and shape FFmpeg can read. It produces a single MP4:

- **Ordered.** Clips are sorted by recording time, using metadata, file-name dates or modification times.
- **Resized without black bars.** Portrait, square or old 4:3 clips sit on a soft, blurred copy of themselves. Clips that are nearly the right shape are filled and cropped.
- **Cleaned up.** Rotation metadata is honoured, interlaced footage is deinterlaced, anamorphic DV/HDV is shown at the right shape, and HDR (HLG/PQ) is tone-mapped to SDR.
- **Joined smoothly.** Choose cut, crossfade, fade through black or white, slide, wipe, zoom, blur dissolve, tape rewind, or a tasteful mix. Picture and sound transition together.
- **Introduced.** An optional animated title (title, subtitle and date) comes from built-in or custom templates.
- **Sounding even.** Clips without sound get silence, clips are levelled against each other, and the finished soundtrack is normalised to **EBU R128 in two passes**: -23 LUFS and -1 dBTP by default.
- **Encoded once.** A frame server feeds one hardware or OS encoder (VideoToolbox, NVENC, Quick Sync, AMF, Media Foundation; x264 or OpenH264 as fallbacks). That means one generation of quality loss, no audio drift, and no glitches at the joins.
- **Verified.** Every output is checked before it reaches you: streams, length, A/V sync, fast-start layout, and a decode test. Existing files are never overwritten.

It is a command-line tool with a full-screen progress view, and a Rust library for building your own apps.

## Install

You need **FFmpeg 6.1 or newer** (`ffmpeg` and `ffprobe`). Any build works, including LGPL-only builds; the tool uses no GPL-only filters.

```bash
# From source (Rust 1.99+)
cargo install --git https://github.com/askrejans/ffmpeg-home-video-tools

# Or build the Docker image from the source:
docker build -t home-video-tools .
docker run --rm -v "$PWD/in:/input" -v "$PWD/out:/output" home-video-tools render /input -o /output/movie.mp4 --no-tui
```

## Quick start

```bash
# Everything in a folder (and its sub-folders) → one Full HD movie
ffmpeg-video-processor render ~/Videos/Summer -o ~/Movies/Summer.mp4

# With an animated title, slide transitions and a 4K canvas
ffmpeg-video-processor render ~/Videos/Summer -o Summer.mp4 \
  --title "Summer at Grandma's" --subtitle "Jūrmala" --date "July 2026" \
  --template cinematic --transition slide --preset 4k

# Phone-friendly vertical video from a few files, trimming one of them
ffmpeg-video-processor render a.mov b.mp4 c.avi -o reel.mp4 --preset vertical --trim "b.mp4=2.5-14"

# What would be used, and what is left out (photos, sidecars, broken files)?
ffmpeg-video-processor scan ~/Videos/Summer

# The 0.2 command still works: every video in a folder → output/processed_vod_<time>.mp4 (4K)
ffmpeg-video-processor process ~/Videos/Summer ~/Movies
```

When run in a terminal, `render` shows a full-screen view with clips, progress, speed and time left. Press `q` twice to cancel. Use `--no-tui` for plain progress, or `--json` for one JSON event per line (useful when another program drives the tool).

### Commands

| Command | What it does |
|---|---|
| `render [INPUTS…] -o OUT` | Join files and folders into one movie (or `--project project.json`) |
| `process IN_DIR OUT_DIR` | 0.2-compatible folder render (4K, cuts, natural name order) |
| `scan INPUTS…` | Show usable videos in order, and what was left out and why |
| `probe FILES…` | Show what FFmpeg reports (`--json` for machines) |
| `validate DIR` | Check that every video in a folder can be read |
| `encoders` | List H.264 encoders that actually work on this machine |
| `templates` | List the built-in title templates and their fields |
| `thumbnail`, `filmstrip`, `proxy` | Preview helpers: a still, evenly spaced frames, a small scrub-friendly copy |

Main `render` options:

| Option | Description |
|---|---|
| `--preset` | Canvas: `4k`, `1080p` (default), `720p`, `vertical` (1080×1920) or `WIDTHxHEIGHT` |
| `--fps` | `auto` (default: the rate covering most footage, snapped to 24/25/30/50/60) or a fixed rate |
| `--quality` | `draft`, `standard` (default) or `high` |
| `--transition`, `--transition-duration` | One of the transitions above (default: crossfade, 1 s). A transition never takes more than 40 % of a clip. |
| `--title`, `--subtitle`, `--date`, `--template` | Animated intro. Built-in templates: `clean`, `cinematic`, `retro`; or a template folder. `--intro-only` renders just the intro. |
| `--sort` | `recorded` (default), `name` (natural order) or `as-given` |
| `--trim NAME=START-END` | Keep part of a clip (seconds; `END` optional); repeatable |
| `--loudness`, `--true-peak`, `--no-loudness`, `--no-level-clips` | Loudness targets and switches |
| `--watermark PNG` | Burn an image into the lower-right corner |
| `--encoder` | Force an FFmpeg encoder; otherwise the best working one is picked |
| `--dry-run`, `--save-project` | Show or save the plan without (or before) rendering |
| `--ffmpeg`, `--ffprobe` | Use specific FFmpeg executables |

## Project files

A render is described by a small JSON document. The CLI writes one with `--save-project` and reads one with `--project`.

```json
{
  "clips": [
    { "path": "IMG_0001.MOV", "trim": { "start": 1.2, "end": 14.0 } },
    { "path": "MOV_0042.AVI" }
  ],
  "intro": { "template": "retro", "fields": { "title": "Summer", "date": "July 2026" } },
  "transition": { "kind": "crossfade", "duration": 1.0 },
  "output": { "path": "Summer.mp4", "preset": "1080p", "fps": "auto", "quality": "standard" },
  "audio": { "level_clips": true, "loudness": { "integrated": -23.0, "true_peak": -1.0 } },
  "watermark": { "image": "logo.png", "anchor": "bottom_right", "width_fraction": 0.22, "opacity": 0.85 }
}
```

See [docs/library.md](docs/library.md) for every field.

## Using it as a library

```rust
use ffmpeg_video_processor::{render, CancelToken, Event, FfmpegTools, ToolPaths};
use ffmpeg_video_processor::project::{Clip, Output, Project};

let tools = FfmpegTools::locate(&ToolPaths::default())?;
let project = Project {
    clips: vec![Clip::new("a.mp4"), Clip::new("b.mov")],
    intro: None,
    transition: Default::default(),
    output: Output::new("movie.mp4"),
    audio: Default::default(),
    watermark: None,
};
let cancel = CancelToken::new();
let outcome = render(&tools, &project, &Default::default(), &mut |event: Event| {
    if let Event::Progress { fraction, .. } = event {
        println!("{:.0}%", fraction * 100.0);
    }
}, &cancel)?;
println!("saved {}", outcome.output.display());
```

The library is synchronous and thread-friendly: run a job on a worker thread and cancel it from anywhere with `CancelToken::cancel`. Every error has a stable `code()`.

Hosts whose child processes cannot open the user's files themselves, such as sandboxed apps, can use `FfmpegTools::with_input_access(InputAccess::InheritedFd)`. Each input is then opened in-process and handed to FFmpeg as an inherited file descriptor.

Full API, JSON contract, events and error codes: [docs/library.md](docs/library.md). Title template format: [docs/templates.md](docs/templates.md).

## How it works

1. **Scan and probe.** `ffprobe` JSON gives display-matrix rotation, pixel aspect, field order, colour transfer, streams and dates. Folders are walked recursively, skipping hidden files, AppleDouble `._*` files and camera sidecars (`.LRV`, `.THM`, `.LRF`).
2. **Plan** (a pure function). Frame rate, trims and transition overlaps become exact frame and sample counts. Audio and video therefore always end on the same frame.
3. **Audio.** Each clip's sound is decoded to 48 kHz stereo and measured (EBU R128), then mixed with equal-power crossfades. `loudnorm` pass 1 measures the mix, and pass 2 applies linear normalisation while encoding AAC.
4. **Video.** One FFmpeg decoder per clip (at most two at once) produces canvas-sized frames, using only LGPL filters (`gblur`, `bwdif`, `zscale`/`tonemap`, `scale`, `overlay`…). A Rust compositor draws transitions, the intro and the watermark, and streams frames into one encoder.
5. **Verify, then move.** The file is checked in a work folder and only then moved into place.

## Development

```bash
cargo test            # unit tests + end-to-end tests (needs ffmpeg/ffprobe on PATH)
cargo clippy --all-targets -- -D warnings
```

The end-to-end tests generate a corpus of awkward inputs with FFmpeg's own sources: rotated, anamorphic DV, interlaced MPEG-2, MPEG-TS offsets, HLG, VFR, late audio, 5.1, two audio tracks, H.263 3GP, cover art, odd sizes, broken files, photos and unicode names. They then render it to several canvases.

The `bash/` folder holds the original shell scripts. They are kept for reference and are not maintained.

## Licence

GPL-3.0-only. See [LICENSE](LICENSE). Releases up to 0.2.1 were published under the BSD 2-Clause licence; from 0.3.0 the project is licensed under the GNU General Public License v3.0.
