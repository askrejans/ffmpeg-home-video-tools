//! Synthetic test media, generated once per test binary with FFmpeg's own
//! sources and LGPL-only encoders (no binary fixtures in the repository).
#![allow(dead_code)]

use ffmpeg_video_processor::{FfmpegTools, ToolPaths};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

pub fn tools() -> Option<FfmpegTools> {
    FfmpegTools::locate(&ToolPaths::default()).ok()
}

/// Skip a test when FFmpeg is not installed.
#[macro_export]
macro_rules! require_ffmpeg {
    () => {
        match common::tools() {
            Some(t) => t,
            None => {
                eprintln!("ffmpeg/ffprobe not found; skipping");
                return;
            }
        }
    };
}

fn ffmpeg(args: &[&str]) {
    let status = Command::new("ffmpeg")
        .args(["-nostdin", "-hide_banner", "-loglevel", "error", "-y"])
        .args(args)
        .status()
        .expect("ffmpeg runs");
    assert!(status.success(), "ffmpeg {args:?} failed");
}

/// Video source: `testsrc2` at the given size and rate.
fn video(size: &str, rate: u32, seconds: f64) -> String {
    format!("testsrc2=s={size}:r={rate}:d={seconds}")
}

fn tone(freq: u32, seconds: f64, volume: f64) -> String {
    format!("sine=f={freq}:d={seconds}:sample_rate=48000,volume={volume}")
}

pub struct Corpus {
    pub dir: PathBuf,
}

impl Corpus {
    pub fn path(&self, name: &str) -> PathBuf {
        self.dir.join(name)
    }
}

/// Generate (once) and return the corpus.
pub fn corpus() -> &'static Corpus {
    static CORPUS: OnceLock<Corpus> = OnceLock::new();
    CORPUS.get_or_init(|| {
        let dir = tempfile::Builder::new()
            .prefix("fvp-corpus-")
            .tempdir()
            .expect("temp dir")
            .keep();
        build(&dir);
        Corpus { dir }
    })
}

fn p(dir: &Path, name: &str) -> String {
    dir.join(name).to_string_lossy().into_owned()
}

fn build(dir: &Path) {
    // Plain landscape clip with sound.
    ffmpeg(&[
        "-f",
        "lavfi",
        "-i",
        &video("640x360", 30, 2.0),
        "-f",
        "lavfi",
        "-i",
        &tone(440, 2.0, 0.5),
        "-shortest",
        "-c:v",
        "mpeg4",
        "-q:v",
        "4",
        "-c:a",
        "aac",
        "-b:a",
        "128k",
        &p(dir, "landscape.mp4"),
    ]);
    // Same stem, different container (must not collide).
    ffmpeg(&[
        "-f",
        "lavfi",
        "-i",
        &video("640x360", 30, 1.5),
        "-f",
        "lavfi",
        "-i",
        &tone(550, 1.5, 0.5),
        "-shortest",
        "-c:v",
        "mpeg4",
        "-q:v",
        "4",
        "-c:a",
        "aac",
        &p(dir, "landscape.mov"),
    ]);
    // Phone portrait via display matrix, and an upside-down clip.
    for (deg, name) in [
        ("90", "rotated90.mp4"),
        ("180", "rotated180.mp4"),
        ("-90", "rotated270.mp4"),
    ] {
        ffmpeg(&[
            "-display_rotation",
            deg,
            "-i",
            &p(dir, "landscape.mp4"),
            "-c",
            "copy",
            &p(dir, name),
        ]);
    }
    // Natively portrait, no audio, in Matroska.
    ffmpeg(&[
        "-f",
        "lavfi",
        "-i",
        &video("360x640", 30, 1.5),
        "-c:v",
        "mpeg4",
        "-q:v",
        "4",
        &p(dir, "portrait_silent.mkv"),
    ]);
    // PAL DV: anamorphic 16:9, interlaced, PCM audio.
    ffmpeg(&[
        "-f",
        "lavfi",
        "-i",
        &video("720x576", 25, 1.6),
        "-f",
        "lavfi",
        "-i",
        &tone(330, 1.6, 0.4),
        "-shortest",
        "-c:v",
        "dvvideo",
        "-pix_fmt",
        "yuv420p",
        "-aspect",
        "16:9",
        "-c:a",
        "pcm_s16le",
        "-ar",
        "48000",
        "-ac",
        "2",
        "-f",
        "dv",
        &p(dir, "camcorder.dv"),
    ]);
    // Interlaced MPEG-2 program stream with MP2 audio (DVD / old camcorder).
    ffmpeg(&[
        "-f",
        "lavfi",
        "-i",
        &video("720x576", 25, 1.6),
        "-f",
        "lavfi",
        "-i",
        &tone(660, 1.6, 0.6),
        "-shortest",
        "-vf",
        "setfield=tff",
        "-c:v",
        "mpeg2video",
        "-flags",
        "+ilme+ildct",
        "-b:v",
        "4M",
        "-c:a",
        "mp2",
        "-b:a",
        "192k",
        &p(dir, "dvd.mpg"),
    ]);
    // MPEG-TS with a non-zero start time.
    ffmpeg(&[
        "-f",
        "lavfi",
        "-i",
        &video("1280x720", 25, 1.2),
        "-f",
        "lavfi",
        "-i",
        &tone(500, 1.2, 0.5),
        "-shortest",
        "-c:v",
        "mpeg2video",
        "-b:v",
        "3M",
        "-c:a",
        "mp2",
        "-output_ts_offset",
        "10",
        &p(dir, "broadcast.ts"),
    ]);
    // HLG-tagged 10-bit clip (lossless FFV1 keeps it LGPL-only).
    ffmpeg(&[
        "-f",
        "lavfi",
        "-i",
        &video("640x360", 25, 1.2),
        "-vf",
        "setparams=color_primaries=bt2020:color_trc=arib-std-b67:colorspace=bt2020nc",
        "-pix_fmt",
        "yuv420p10le",
        "-c:v",
        "ffv1",
        &p(dir, "hdr_hlg.mkv"),
    ]);
    // Variable frame rate.
    ffmpeg(&[
        "-f",
        "lavfi",
        "-i",
        &video("640x360", 30, 1.5),
        "-vf",
        "setpts='N/(30*TB)+if(mod(N\\,4)\\,0\\,0.012/TB)'",
        "-fps_mode",
        "vfr",
        "-c:v",
        "mpeg4",
        "-q:v",
        "4",
        &p(dir, "phone_vfr.mkv"),
    ]);
    // Audio that starts 0.4 s after the picture.
    ffmpeg(&[
        "-f",
        "lavfi",
        "-i",
        &video("640x360", 30, 2.0),
        "-itsoffset",
        "0.4",
        "-f",
        "lavfi",
        "-i",
        &tone(700, 1.6, 0.5),
        "-c:v",
        "mpeg4",
        "-q:v",
        "4",
        "-c:a",
        "aac",
        &p(dir, "late_audio.mp4"),
    ]);
    // 5.1 audio, and two audio tracks with the second marked default.
    ffmpeg(&[
        "-f",
        "lavfi",
        "-i",
        &video("640x360", 25, 1.2),
        "-f",
        "lavfi",
        "-i",
        "sine=f=300:d=1.2:sample_rate=48000",
        "-filter_complex",
        "[1:a]pan=5.1|FL=c0|FR=c0|FC=c0|LFE=c0|BL=c0|BR=c0[a]",
        "-map",
        "0:v",
        "-map",
        "[a]",
        "-c:v",
        "mpeg4",
        "-c:a",
        "aac",
        &p(dir, "surround.mkv"),
    ]);
    ffmpeg(&[
        "-f",
        "lavfi",
        "-i",
        &video("640x360", 25, 1.2),
        "-f",
        "lavfi",
        "-i",
        &tone(200, 1.2, 0.3),
        "-f",
        "lavfi",
        "-i",
        &tone(800, 1.2, 0.3),
        "-map",
        "0:v",
        "-map",
        "1:a",
        "-map",
        "2:a",
        "-c:v",
        "mpeg4",
        "-c:a",
        "aac",
        "-disposition:a:0",
        "0",
        "-disposition:a:1",
        "default",
        &p(dir, "two_tracks.mkv"),
    ]);
    // Old phone: H.263 in 3GP with 8 kHz mono AAC.
    ffmpeg(&[
        "-f",
        "lavfi",
        "-i",
        &video("176x144", 15, 1.4),
        "-f",
        "lavfi",
        "-i",
        "sine=f=400:d=1.4:sample_rate=8000",
        "-shortest",
        "-c:v",
        "h263",
        "-c:a",
        "aac",
        "-ac",
        "1",
        "-ar",
        "8000",
        &p(dir, "oldphone.3gp"),
    ]);
    // Cover art as an extra video stream.
    ffmpeg(&[
        "-f",
        "lavfi",
        "-i",
        "color=c=red:s=300x300",
        "-frames:v",
        "1",
        &p(dir, "cover.png"),
    ]);
    ffmpeg(&[
        "-i",
        &p(dir, "landscape.mp4"),
        "-i",
        &p(dir, "cover.png"),
        "-map",
        "0",
        "-map",
        "1",
        "-c",
        "copy",
        "-c:v:1",
        "png",
        "-disposition:v:1",
        "attached_pic",
        &p(dir, "with_cover.mp4"),
    ]);
    // Odd and tiny sizes.
    ffmpeg(&[
        "-f",
        "lavfi",
        "-i",
        &video("1001x701", 25, 1.0),
        "-pix_fmt",
        "yuv444p",
        "-c:v",
        "ffv1",
        &p(dir, "odd_size.mkv"),
    ]);
    ffmpeg(&[
        "-f",
        "lavfi",
        "-i",
        &video("160x120", 25, 1.0),
        "-c:v",
        "mpeg4",
        &p(dir, "tiny.avi"),
    ]);
    // Things that are not usable videos.
    let landscape = std::fs::read(dir.join("landscape.mp4")).expect("landscape");
    std::fs::write(
        dir.join("broken.mp4"),
        &landscape[..landscape.len().min(1500)],
    )
    .expect("broken");
    std::fs::copy(dir.join("cover.png"), dir.join("really_a_photo.mp4")).expect("photo");
    ffmpeg(&[
        "-f",
        "lavfi",
        "-i",
        &tone(440, 1.0, 0.5),
        "-c:a",
        "aac",
        "-f",
        "mp4",
        &p(dir, "song_only.mp4"),
    ]);
    std::fs::write(dir.join("._landscape.mp4"), b"AppleDouble").expect("appledouble");
    std::fs::write(dir.join("GOPR0001.LRV"), &landscape).expect("lrv");
    std::fs::write(dir.join("notes.txt"), b"hello").expect("notes");
    // A name with quotes, diacritics and emoji.
    std::fs::copy(
        dir.join("landscape.mp4"),
        dir.join("it's \"Jūrmala\" 🎬.mp4"),
    )
    .expect("unicode");
    std::fs::remove_file(dir.join("cover.png")).expect("cleanup");
}

/// Usable videos in the corpus.
pub const VIDEOS: &[&str] = &[
    "landscape.mp4",
    "landscape.mov",
    "rotated90.mp4",
    "rotated180.mp4",
    "rotated270.mp4",
    "portrait_silent.mkv",
    "camcorder.dv",
    "dvd.mpg",
    "broadcast.ts",
    "hdr_hlg.mkv",
    "phone_vfr.mkv",
    "late_audio.mp4",
    "surround.mkv",
    "two_tracks.mkv",
    "oldphone.3gp",
    "with_cover.mp4",
    "odd_size.mkv",
    "tiny.avi",
    "it's \"Jūrmala\" 🎬.mp4",
];

/// ffprobe one stream entry of a file, e.g. `("v:0", "width")`.
pub fn probe_entry(path: &Path, stream: &str, entry: &str) -> String {
    let out = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-select_streams",
            stream,
            "-show_entries",
            &format!("stream={entry}"),
            "-of",
            "csv=p=0",
        ])
        .arg(path)
        .output()
        .expect("ffprobe runs");
    // FFmpeg 9 may list a stream again inside a stream group; take the first.
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or_default()
        .trim_end_matches(',')
        .to_string()
}

/// Integrated loudness (LUFS) of a file's audio, measured by FFmpeg.
pub fn integrated_loudness(path: &Path) -> f64 {
    let out = Command::new("ffmpeg")
        .args(["-nostdin", "-hide_banner", "-i"])
        .arg(path)
        .args(["-af", "ebur128=framelog=quiet", "-f", "null", "-"])
        .output()
        .expect("ffmpeg runs");
    let text = String::from_utf8_lossy(&out.stderr);
    let summary = text.rsplit("Summary:").next().unwrap_or_default();
    summary
        .lines()
        .find_map(|l| l.trim().strip_prefix("I:"))
        .and_then(|v| v.trim().trim_end_matches("LUFS").trim().parse().ok())
        .expect("integrated loudness in ebur128 summary")
}

/// Mean luma of a rectangle of frame `n`, from a grey decode.
pub fn mean_luma(path: &Path, n: u32, x: u32, y: u32, w: u32, h: u32) -> f64 {
    let out = Command::new("ffmpeg")
        .args(["-nostdin", "-v", "error", "-i"])
        .arg(path)
        .args([
            "-vf",
            &format!("select=eq(n\\,{n}),crop={w}:{h}:{x}:{y},format=gray"),
            "-frames:v",
            "1",
            "-f",
            "rawvideo",
            "-",
        ])
        .output()
        .expect("ffmpeg runs");
    assert!(!out.stdout.is_empty(), "no frame decoded");
    out.stdout.iter().map(|v| f64::from(*v)).sum::<f64>() / out.stdout.len() as f64
}
