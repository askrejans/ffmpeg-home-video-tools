//! Finding, classifying and ordering input videos.

use crate::error::{Error, Result};
use crate::events::CancelToken;
use crate::probe::{MediaInfo, MediaKind, probe};
use crate::tools::FfmpegTools;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScanOptions {
    /// Descend into sub-folders of folder inputs.
    pub recursive: bool,
    /// Include files whose names start with a dot.
    pub include_hidden: bool,
}

impl Default for ScanOptions {
    fn default() -> Self {
        Self {
            recursive: true,
            include_hidden: false,
        }
    }
}

/// Why a scanned file is or is not usable as a clip.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScanKind {
    Video,
    Photo,
    AudioOnly,
    /// A known non-video file (document, archive, camera sidecar…).
    Skipped,
    /// FFmpeg could not read it, or it contains nothing playable.
    Unreadable,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScanItem {
    pub path: PathBuf,
    pub kind: ScanKind,
    /// Human-readable reason for anything other than `Video`.
    pub reason: Option<String>,
    /// Probe result for videos.
    pub media: Option<MediaInfo>,
}

/// Order of discovered videos.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SortOrder {
    /// Earliest recording first; ties by name.
    #[default]
    Recorded,
    /// Natural file-name order (`clip2` before `clip10`).
    Name,
    /// Keep the order inputs were given / found in.
    AsGiven,
}

const PHOTO_EXTENSIONS: &[&str] = &[
    "jpg", "jpeg", "png", "heic", "heif", "webp", "bmp", "tif", "tiff", "dng", "raw", "cr2", "cr3",
    "nef", "arw", "orf", "rw2", "raf", "psd", "svg", "avif", "jxl",
];
const AUDIO_EXTENSIONS: &[&str] = &[
    "mp3", "wav", "m4a", "aac", "flac", "ogg", "opus", "wma", "aiff", "aif", "amr", "mid",
];
/// Files that are never videos, plus camera sidecars that duplicate a real clip
/// (GoPro `.lrv`/`.thm`, DJI `.lrf`).
const SKIP_EXTENSIONS: &[&str] = &[
    "txt", "pdf", "doc", "docx", "xls", "xlsx", "ppt", "pptx", "md", "json", "xml", "csv", "html",
    "htm", "zip", "rar", "7z", "gz", "tar", "dmg", "exe", "msi", "app", "lnk", "ini", "db",
    "plist", "log", "thm", "lrv", "lrf", "srt", "ass", "vtt", "sub", "idx", "cue", "nfo", "url",
    "ds_store", "aae", "xmp", "bup", "ifo", "sav", "bin", "dat", "cpi", "mpl", "bdm", "tid", "dll",
    "sys", "pkg",
];
const SKIP_NAMES: &[&str] = &["thumbs.db", "desktop.ini", ".ds_store", "icon\r"];

fn extension(path: &Path) -> String {
    path.extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default()
}

fn hidden(path: &Path) -> bool {
    path.file_name()
        .map(|n| n.to_string_lossy().starts_with('.'))
        .unwrap_or(false)
}

/// Classify a file by name alone. `None` means it must be probed.
fn quick_kind(path: &Path, opts: &ScanOptions) -> Option<(ScanKind, &'static str)> {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    // AppleDouble resource forks written to FAT/exFAT drives by macOS.
    if name.starts_with("._") {
        return Some((ScanKind::Skipped, "macOS metadata file"));
    }
    if SKIP_NAMES.contains(&name.as_str()) || (!opts.include_hidden && hidden(path)) {
        return Some((ScanKind::Skipped, "hidden or system file"));
    }
    let ext = extension(path);
    if PHOTO_EXTENSIONS.contains(&ext.as_str()) {
        return Some((ScanKind::Photo, "photos are not supported"));
    }
    if AUDIO_EXTENSIONS.contains(&ext.as_str()) {
        return Some((ScanKind::AudioOnly, "audio files are not supported"));
    }
    if SKIP_EXTENSIONS.contains(&ext.as_str()) {
        return Some((ScanKind::Skipped, "not a video file"));
    }
    None
}

/// Expand inputs (files and folders) into candidate files, without probing.
/// Explicitly named files are always kept; folder contents are filtered.
pub fn collect(inputs: &[PathBuf], opts: &ScanOptions) -> Result<Vec<PathBuf>> {
    let mut seen = BTreeSet::new();
    let mut files = Vec::new();
    for input in inputs {
        let meta = std::fs::metadata(input).map_err(|e| Error::Unreadable {
            path: input.clone(),
            reason: e.to_string(),
        })?;
        if meta.is_file() {
            push_unique(&mut seen, &mut files, input.clone());
            continue;
        }
        let walker = walkdir::WalkDir::new(input)
            .follow_links(true)
            .max_depth(if opts.recursive { usize::MAX } else { 1 })
            .sort_by(|a, b| {
                natural_cmp(
                    &a.file_name().to_string_lossy(),
                    &b.file_name().to_string_lossy(),
                )
            })
            .into_iter()
            .filter_entry(|e| e.depth() == 0 || opts.include_hidden || !hidden(e.path()));
        for entry in walker.filter_map(|e| e.ok()) {
            if entry.file_type().is_file() {
                push_unique(&mut seen, &mut files, entry.into_path());
            }
        }
    }
    Ok(files)
}

fn push_unique(seen: &mut BTreeSet<PathBuf>, files: &mut Vec<PathBuf>, path: PathBuf) {
    let key = std::fs::canonicalize(&path).unwrap_or_else(|_| path.clone());
    if seen.insert(key) {
        files.push(path);
    }
}

/// Collect, classify and probe inputs. Probing runs in parallel; `on_item`
/// is called as each file is classified (in completion order).
pub fn scan(
    tools: &FfmpegTools,
    inputs: &[PathBuf],
    opts: &ScanOptions,
    cancel: &CancelToken,
    on_item: &(dyn Fn(&ScanItem) + Sync),
) -> Result<Vec<ScanItem>> {
    let files = collect(inputs, opts)?;
    let explicit: BTreeSet<&PathBuf> = inputs.iter().collect();
    let items: Vec<ScanItem> = files
        .par_iter()
        .map(|path| {
            if cancel.is_cancelled() {
                return None;
            }
            let item = classify(tools, path, explicit.contains(path), opts, cancel);
            on_item(&item);
            Some(item)
        })
        .collect::<Option<Vec<_>>>()
        .ok_or(Error::Cancelled)?;
    cancel.check()?;
    Ok(items)
}

fn classify(
    tools: &FfmpegTools,
    path: &Path,
    explicit: bool,
    opts: &ScanOptions,
    cancel: &CancelToken,
) -> ScanItem {
    let quick = if explicit {
        None
    } else {
        quick_kind(path, opts)
    };
    if let Some((kind, reason)) = quick {
        return ScanItem {
            path: path.to_path_buf(),
            kind,
            reason: Some(reason.to_string()),
            media: None,
        };
    }
    match probe(tools, path, cancel) {
        Ok(media) => {
            let (kind, reason) = match media.kind {
                MediaKind::Video => (ScanKind::Video, None),
                MediaKind::Photo => (ScanKind::Photo, Some("photos are not supported")),
                MediaKind::AudioOnly => (ScanKind::AudioOnly, Some("the file has no picture")),
                MediaKind::Unsupported => {
                    (ScanKind::Unreadable, Some("the file has no playable video"))
                }
            };
            ScanItem {
                path: path.to_path_buf(),
                kind,
                reason: reason.map(str::to_string),
                media: (kind == ScanKind::Video).then_some(media),
            }
        }
        Err(e) => ScanItem {
            path: path.to_path_buf(),
            kind: ScanKind::Unreadable,
            reason: Some(match e {
                Error::Unreadable { reason, .. } => first_line(&reason),
                other => other.to_string(),
            }),
            media: None,
        },
    }
}

fn first_line(text: &str) -> String {
    text.lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("unreadable")
        .to_string()
}

/// Sort probed videos in place.
pub fn sort_media(media: &mut [MediaInfo], order: SortOrder) {
    let by_name = |a: &MediaInfo, b: &MediaInfo| {
        natural_cmp(&a.path.to_string_lossy(), &b.path.to_string_lossy())
    };
    match order {
        SortOrder::AsGiven => {}
        SortOrder::Name => media.sort_by(by_name),
        SortOrder::Recorded => media.sort_by(|a, b| match (a.recorded_at, b.recorded_at) {
            (Some(x), Some(y)) => x.cmp(&y).then_with(|| by_name(a, b)),
            (Some(_), None) => Ordering::Less,
            (None, Some(_)) => Ordering::Greater,
            (None, None) => by_name(a, b),
        }),
    }
}

/// Natural, case-insensitive ordering: digit runs compare numerically.
pub fn natural_cmp(a: &str, b: &str) -> Ordering {
    let (mut x, mut y) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (x.peek().copied(), y.peek().copied()) {
            (None, None) => return a.cmp(b),
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(c), Some(d)) if c.is_ascii_digit() && d.is_ascii_digit() => {
                let take = |it: &mut std::iter::Peekable<std::str::Chars>| {
                    let mut s = String::new();
                    while let Some(ch) = it.peek().copied().filter(char::is_ascii_digit) {
                        s.push(ch);
                        it.next();
                    }
                    s
                };
                let (m, n) = (take(&mut x), take(&mut y));
                let (mt, nt) = (m.trim_start_matches('0'), n.trim_start_matches('0'));
                let ord = mt.len().cmp(&nt.len()).then_with(|| mt.cmp(nt));
                if ord != Ordering::Equal {
                    return ord;
                }
            }
            (Some(c), Some(d)) => {
                let ord = c.to_lowercase().cmp(d.to_lowercase()).then(Ordering::Equal);
                if ord != Ordering::Equal {
                    return ord;
                }
                x.next();
                y.next();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn natural_order() {
        let mut names = vec![
            "clip10.mp4",
            "Clip2.mp4",
            "clip1.mp4",
            "clip02b.mp4",
            "a.mp4",
        ];
        names.sort_by(|a, b| natural_cmp(a, b));
        assert_eq!(
            names,
            [
                "a.mp4",
                "clip1.mp4",
                "Clip2.mp4",
                "clip02b.mp4",
                "clip10.mp4"
            ]
        );
    }

    #[test]
    fn quick_classification() {
        let opts = ScanOptions::default();
        let k = |p: &str| quick_kind(Path::new(p), &opts).map(|(k, _)| k);
        assert_eq!(k("/v/._clip.mp4"), Some(ScanKind::Skipped));
        assert_eq!(k("/v/.hidden.mp4"), Some(ScanKind::Skipped));
        assert_eq!(k("/v/GOPR0001.LRV"), Some(ScanKind::Skipped));
        assert_eq!(k("/v/GOPR0001.THM"), Some(ScanKind::Skipped));
        assert_eq!(k("/v/IMG_0001.HEIC"), Some(ScanKind::Photo));
        assert_eq!(k("/v/song.mp3"), Some(ScanKind::AudioOnly));
        assert_eq!(k("/v/clip.MTS"), None);
        assert_eq!(k("/v/clip.unknownext"), None);
        assert_eq!(k("/v/Thumbs.db"), Some(ScanKind::Skipped));
    }

    #[test]
    fn collects_recursively_and_dedupes() {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("Day 2");
        std::fs::create_dir(&sub).unwrap();
        std::fs::create_dir(dir.path().join(".hidden")).unwrap();
        for p in ["clip10.mp4", "clip2.mp4", "Day 2/b.mov", ".hidden/x.mp4"] {
            std::fs::write(dir.path().join(p), b"x").unwrap();
        }
        let inputs = vec![dir.path().to_path_buf(), dir.path().join("clip2.mp4")];
        let found = collect(&inputs, &ScanOptions::default()).unwrap();
        let names: Vec<String> = found
            .iter()
            .map(|p| {
                p.strip_prefix(dir.path())
                    .unwrap()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        assert_eq!(names, ["clip2.mp4", "clip10.mp4", "Day 2/b.mov"]);
        let flat = collect(
            &[dir.path().to_path_buf()],
            &ScanOptions {
                recursive: false,
                include_hidden: false,
            },
        )
        .unwrap();
        assert_eq!(flat.len(), 2);
    }
}
