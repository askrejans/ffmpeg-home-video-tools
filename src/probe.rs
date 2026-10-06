//! Reading media properties with `ffprobe`.

use crate::error::{Error, Result};
use crate::events::CancelToken;
use crate::tools::FfmpegTools;
use chrono::{
    DateTime, Datelike, FixedOffset, Local, NaiveDate, NaiveDateTime, NaiveTime, TimeZone,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// What a file turned out to contain.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MediaKind {
    Video,
    Photo,
    AudioOnly,
    Unsupported,
}

/// HDR transfer characteristics that need tone-mapping to SDR.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HdrTransfer {
    Hlg,
    Pq,
}

/// Where a recording time came from, from most to least reliable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DateSource {
    Metadata,
    FileName,
    FileModified,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VideoInfo {
    /// Absolute stream index in the file.
    pub stream_index: usize,
    pub codec: String,
    /// Coded size in pixels.
    pub width: u32,
    pub height: u32,
    /// Sample (pixel) aspect ratio.
    pub sar_num: u32,
    pub sar_den: u32,
    /// Clockwise rotation needed for display: 0, 90, 180 or 270.
    pub rotation: u32,
    /// Size as displayed, after pixel aspect and rotation.
    pub display_width: u32,
    pub display_height: u32,
    /// Average frame rate.
    pub frame_rate: f64,
    pub interlaced: bool,
    pub hdr: Option<HdrTransfer>,
    pub pix_fmt: String,
}

impl VideoInfo {
    /// Display aspect ratio (width / height).
    pub fn display_aspect(&self) -> f64 {
        self.display_width as f64 / self.display_height.max(1) as f64
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AudioInfo {
    pub stream_index: usize,
    pub codec: String,
    pub channels: u32,
    pub sample_rate: u32,
}

/// Everything the planner needs to know about one input.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MediaInfo {
    pub path: PathBuf,
    pub kind: MediaKind,
    /// Container format name reported by FFmpeg.
    pub format: String,
    /// Duration in seconds (0 when unknown).
    pub duration: f64,
    pub size_bytes: u64,
    pub video: Option<VideoInfo>,
    pub audio: Option<AudioInfo>,
    pub audio_streams: usize,
    pub recorded_at: Option<DateTime<FixedOffset>>,
    pub recorded_at_source: Option<DateSource>,
}

#[derive(Debug, Default, Deserialize)]
struct ProbeOutput {
    #[serde(default)]
    streams: Vec<ProbeStream>,
    #[serde(default)]
    format: ProbeFormat,
}

#[derive(Debug, Default, Deserialize)]
struct ProbeFormat {
    format_name: Option<String>,
    duration: Option<String>,
    #[serde(default)]
    tags: BTreeMap<String, String>,
}

#[derive(Debug, Default, Deserialize)]
struct ProbeStream {
    index: usize,
    codec_type: Option<String>,
    codec_name: Option<String>,
    width: Option<u32>,
    height: Option<u32>,
    sample_aspect_ratio: Option<String>,
    pix_fmt: Option<String>,
    field_order: Option<String>,
    color_transfer: Option<String>,
    r_frame_rate: Option<String>,
    avg_frame_rate: Option<String>,
    duration: Option<String>,
    nb_frames: Option<String>,
    sample_rate: Option<String>,
    channels: Option<u32>,
    #[serde(default)]
    disposition: BTreeMap<String, i64>,
    #[serde(default)]
    tags: BTreeMap<String, String>,
    #[serde(default)]
    side_data_list: Vec<serde_json::Value>,
}

impl ProbeStream {
    fn is(&self, kind: &str) -> bool {
        self.codec_type.as_deref() == Some(kind)
    }

    fn flag(&self, name: &str) -> bool {
        self.disposition.get(name).copied().unwrap_or(0) != 0
    }
}

const IMAGE_CODECS: &[&str] = &[
    "mjpeg", "png", "bmp", "tiff", "webp", "jpegls", "jpeg2000", "gif", "heif", "hevc", "av1",
    "qoi", "jpegxl", "pgm", "ppm", "pam",
];

/// Probe one file.
pub fn probe(tools: &FfmpegTools, path: &Path, cancel: &CancelToken) -> Result<MediaInfo> {
    let meta = std::fs::metadata(path).map_err(|e| Error::Unreadable {
        path: path.to_path_buf(),
        reason: e.to_string(),
    })?;
    if !meta.is_file() {
        return Err(Error::Unreadable {
            path: path.to_path_buf(),
            reason: "not a file".into(),
        });
    }
    let mut cmd = tools.ffprobe();
    cmd.args(["-print_format", "json", "-show_format", "-show_streams"]);
    let url = cmd.input_url(path)?;
    cmd.arg(url);
    let json = cmd.output(cancel).map_err(|e| match e {
        Error::ToolFailed { stderr, .. } => Error::Unreadable {
            path: path.to_path_buf(),
            reason: stderr,
        },
        other => other,
    })?;
    let modified = meta.modified().ok().map(DateTime::<Local>::from);
    let mut info = parse_probe(path, &json, meta.len(), modified)?;
    if info.kind == MediaKind::Video && info.duration <= 0.0 {
        info.duration = count_duration(tools, path, &info, cancel).unwrap_or(0.0);
    }
    if info.kind == MediaKind::Video && info.duration <= 0.0 {
        return Err(Error::Unreadable {
            path: path.to_path_buf(),
            reason: "the video has no readable duration".into(),
        });
    }
    Ok(info)
}

/// Last resort for streams without duration metadata: count packets.
fn count_duration(
    tools: &FfmpegTools,
    path: &Path,
    info: &MediaInfo,
    cancel: &CancelToken,
) -> Result<f64> {
    let video = info.video.as_ref().expect("video kind has video info");
    let mut cmd = tools.ffprobe();
    cmd.args([
        "-select_streams",
        &video.stream_index.to_string(),
        "-count_packets",
        "-show_entries",
        "stream=nb_read_packets",
        "-of",
        "csv=p=0",
    ]);
    let url = cmd.input_url(path)?;
    cmd.arg(url);
    let out = cmd.output(cancel)?;
    let packets: f64 = String::from_utf8_lossy(&out)
        .trim()
        .trim_end_matches(',')
        .parse()
        .unwrap_or(0.0);
    Ok(packets / video.frame_rate.max(1.0))
}

fn parse_f64(value: Option<&String>) -> Option<f64> {
    value
        .and_then(|v| v.trim().parse::<f64>().ok())
        .filter(|v| v.is_finite())
}

/// Parse "num/den" or a plain number.
fn parse_rate(value: Option<&String>) -> Option<f64> {
    let value = value?.trim();
    let rate = match value.split_once('/') {
        Some((n, d)) => {
            let (n, d) = (n.parse::<f64>().ok()?, d.parse::<f64>().ok()?);
            if d == 0.0 {
                return None;
            }
            n / d
        }
        None => value.parse().ok()?,
    };
    (rate.is_finite() && rate > 0.5 && rate <= 1000.0).then_some(rate)
}

fn parse_sar(value: Option<&String>) -> (u32, u32) {
    value
        .and_then(|v| v.split_once(':'))
        .and_then(|(n, d)| Some((n.parse::<u32>().ok()?, d.parse::<u32>().ok()?)))
        .filter(|(n, d)| *n > 0 && *d > 0)
        .unwrap_or((1, 1))
}

/// Clockwise display rotation from the display matrix (counter-clockwise
/// degrees in modern FFmpeg) or the legacy `rotate` tag (clockwise).
fn rotation(stream: &ProbeStream) -> u32 {
    let from_matrix = stream.side_data_list.iter().find_map(|sd| {
        let is_matrix = sd
            .get("side_data_type")
            .and_then(|t| t.as_str())
            .is_some_and(|t| t.eq_ignore_ascii_case("Display Matrix"));
        if !is_matrix {
            return None;
        }
        sd.get("rotation").and_then(|r| r.as_f64()).map(|ccw| -ccw)
    });
    let degrees = from_matrix.or_else(|| {
        stream
            .tags
            .get("rotate")
            .and_then(|r| r.trim().parse::<f64>().ok())
    });
    match degrees {
        Some(d) => {
            let quarter = (d / 90.0).round() as i64;
            (quarter.rem_euclid(4) * 90) as u32
        }
        None => 0,
    }
}

fn tag<'a>(tags: &'a BTreeMap<String, String>, key: &str) -> Option<&'a String> {
    tags.iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(key))
        .map(|(_, v)| v)
}

/// Parse ffprobe JSON. `modified` is the file's modification time, used when
/// no better recording time exists.
pub(crate) fn parse_probe(
    path: &Path,
    json: &[u8],
    size_bytes: u64,
    modified: Option<DateTime<Local>>,
) -> Result<MediaInfo> {
    let probe: ProbeOutput = serde_json::from_slice(json).map_err(|e| Error::Unreadable {
        path: path.to_path_buf(),
        reason: format!("unexpected ffprobe output: {e}"),
    })?;
    let format = probe.format.format_name.clone().unwrap_or_default();

    let mut videos: Vec<&ProbeStream> = probe
        .streams
        .iter()
        .filter(|s| s.is("video") && !s.flag("attached_pic") && s.width.unwrap_or(0) > 0)
        .collect();
    videos.sort_by_key(|s| {
        (
            !s.flag("default"),
            std::cmp::Reverse(u64::from(s.width.unwrap_or(0)) * u64::from(s.height.unwrap_or(0))),
            s.index,
        )
    });
    let audios: Vec<&ProbeStream> = probe
        .streams
        .iter()
        .filter(|s| s.is("audio") && s.channels.unwrap_or(1) > 0)
        .collect();
    let audio_stream = audios
        .iter()
        .find(|s| s.flag("default"))
        .or_else(|| audios.first())
        .copied();

    let format_duration = parse_f64(probe.format.duration.as_ref()).filter(|d| *d > 0.0);
    let video = videos.first().map(|s| video_info(s));
    let video_stream = videos.first().copied();

    let duration = format_duration
        .or_else(|| video_stream.and_then(|s| parse_f64(s.duration.as_ref())))
        .or_else(|| audio_stream.and_then(|s| parse_f64(s.duration.as_ref())))
        .filter(|d| *d > 0.0)
        .unwrap_or(0.0);

    let kind = match video_stream {
        None if audio_stream.is_some() => MediaKind::AudioOnly,
        None => MediaKind::Unsupported,
        Some(stream) => {
            let codec = stream.codec_name.as_deref().unwrap_or_default();
            let single_frame = stream
                .nb_frames
                .as_ref()
                .and_then(|n| n.parse::<u64>().ok())
                .is_some_and(|n| n <= 1);
            let image_container = format.contains("image2") || format.ends_with("_pipe");
            let still = single_frame || duration <= 0.1;
            if image_container || (audio_stream.is_none() && still && IMAGE_CODECS.contains(&codec))
            {
                MediaKind::Photo
            } else {
                MediaKind::Video
            }
        }
    };

    let (recorded_at, recorded_at_source) = recording_time(&probe, video_stream, path, modified);

    Ok(MediaInfo {
        path: path.to_path_buf(),
        kind,
        format,
        duration,
        size_bytes,
        video: if kind == MediaKind::Video {
            video
        } else {
            None
        },
        audio: audio_stream.map(|s| AudioInfo {
            stream_index: s.index,
            codec: s.codec_name.clone().unwrap_or_default(),
            channels: s.channels.unwrap_or(2),
            sample_rate: s
                .sample_rate
                .as_ref()
                .and_then(|r| r.parse().ok())
                .unwrap_or(48_000),
        }),
        audio_streams: audios.len(),
        recorded_at,
        recorded_at_source,
    })
}

fn video_info(stream: &ProbeStream) -> VideoInfo {
    let width = stream.width.unwrap_or(0);
    let height = stream.height.unwrap_or(0);
    let (sar_num, sar_den) = parse_sar(stream.sample_aspect_ratio.as_ref());
    let rotation = rotation(stream);
    let shown_width =
        ((f64::from(width) * f64::from(sar_num) / f64::from(sar_den)).round() as u32).max(1);
    let (display_width, display_height) = if rotation % 180 == 90 {
        (height, shown_width)
    } else {
        (shown_width, height)
    };
    let codec = stream.codec_name.clone().unwrap_or_default();
    let interlaced = match stream.field_order.as_deref() {
        Some("tt" | "bb" | "tb" | "bt") => true,
        Some("progressive") => false,
        _ => codec == "dvvideo",
    };
    let hdr = match stream.color_transfer.as_deref() {
        Some("arib-std-b67") => Some(HdrTransfer::Hlg),
        Some("smpte2084") => Some(HdrTransfer::Pq),
        _ => None,
    };
    let frame_rate = parse_rate(stream.avg_frame_rate.as_ref())
        .filter(|r| *r <= 240.0)
        .or_else(|| parse_rate(stream.r_frame_rate.as_ref()).filter(|r| *r <= 240.0))
        .unwrap_or(30.0);
    VideoInfo {
        stream_index: stream.index,
        codec,
        width,
        height,
        sar_num,
        sar_den,
        rotation,
        display_width,
        display_height,
        frame_rate,
        interlaced,
        hdr,
        pix_fmt: stream.pix_fmt.clone().unwrap_or_default(),
    }
}

fn plausible(date: &DateTime<FixedOffset>) -> bool {
    let year = date.year();
    year >= 1980 && year <= Local::now().year() + 1
}

fn parse_metadata_date(value: &str) -> Option<DateTime<FixedOffset>> {
    let value = value.trim();
    DateTime::parse_from_rfc3339(value)
        .ok()
        .or_else(|| DateTime::parse_from_str(value, "%Y-%m-%dT%H:%M:%S%z").ok())
        .or_else(|| DateTime::parse_from_str(value, "%Y-%m-%dT%H:%M:%S%.f%z").ok())
        .or_else(|| {
            NaiveDateTime::parse_from_str(value, "%Y-%m-%d %H:%M:%S")
                .ok()
                .map(|n| n.and_utc().fixed_offset())
        })
        .filter(plausible)
}

fn recording_time(
    probe: &ProbeOutput,
    video: Option<&ProbeStream>,
    path: &Path,
    modified: Option<DateTime<Local>>,
) -> (Option<DateTime<FixedOffset>>, Option<DateSource>) {
    let candidates = [
        tag(&probe.format.tags, "com.apple.quicktime.creationdate"),
        tag(&probe.format.tags, "creation_time"),
        video.and_then(|s| tag(&s.tags, "creation_time")),
    ];
    if let Some(date) = candidates
        .into_iter()
        .flatten()
        .find_map(|v| parse_metadata_date(v))
    {
        return (Some(date), Some(DateSource::Metadata));
    }
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    if let Some(naive) = date_from_file_name(&stem)
        && let Some(local) = Local.from_local_datetime(&naive).earliest()
    {
        return (Some(local.fixed_offset()), Some(DateSource::FileName));
    }
    match modified {
        Some(m) => (Some(m.fixed_offset()), Some(DateSource::FileModified)),
        None => (None, None),
    }
}

/// Find a date (and optional time) in common camera/phone file names:
/// `VID_20230714_153012`, `PXL_20230714_153012345`, `DJI_20230714153012_0001`,
/// `2023-07-14 15.30.12`, `IMG-20230714-WA0003`.
pub(crate) fn date_from_file_name(stem: &str) -> Option<NaiveDateTime> {
    let bytes = stem.as_bytes();
    let digits = |at: usize, n: usize| -> Option<u32> {
        let slice = bytes.get(at..at + n)?;
        if slice.iter().all(u8::is_ascii_digit) {
            std::str::from_utf8(slice).ok()?.parse().ok()
        } else {
            None
        }
    };
    let is_sep = |at: usize, allowed: &[u8]| bytes.get(at).is_some_and(|b| allowed.contains(b));

    for start in 0..bytes.len() {
        if start > 0 && bytes[start - 1].is_ascii_digit() {
            continue;
        }
        let Some(year) = digits(start, 4) else {
            continue;
        };
        if !(1980..=2100).contains(&year) {
            continue;
        }
        let mut at = start + 4;
        let sep = is_sep(at, b"-_.");
        if sep {
            at += 1;
        }
        let Some(month) = digits(at, 2) else { continue };
        at += 2;
        if sep {
            if !is_sep(at, b"-_.") {
                continue;
            }
            at += 1;
        }
        let Some(day) = digits(at, 2) else { continue };
        at += 2;
        let Some(date) = NaiveDate::from_ymd_opt(year as i32, month, day) else {
            continue;
        };
        // Optional time: [sep] HH [sep] MM [sep] SS
        let mut t = at;
        if is_sep(t, b"-_. T") {
            t += 1;
        }
        let time = (|| {
            let hour = digits(t, 2)?;
            let mut p = t + 2;
            let tsep = is_sep(p, b".:-");
            if tsep {
                p += 1;
            }
            let minute = digits(p, 2)?;
            p += 2;
            if tsep {
                if !is_sep(p, b".:-") {
                    return None;
                }
                p += 1;
            }
            let second = digits(p, 2)?;
            NaiveTime::from_hms_opt(hour, minute, second)
        })();
        // A bare date followed by more digits (e.g. a counter) is not a date.
        if time.is_none() && bytes.get(at).is_some_and(u8::is_ascii_digit) {
            continue;
        }
        return Some(date.and_time(time.unwrap_or_default()));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(json: &str) -> MediaInfo {
        parse_probe(Path::new("/x/clip.mp4"), json.as_bytes(), 1000, None).unwrap()
    }

    #[test]
    fn reads_display_matrix_rotation() {
        let info = parse(
            r#"{"streams":[{"index":0,"codec_type":"video","codec_name":"h264","width":1920,"height":1080,
            "avg_frame_rate":"30/1","side_data_list":[{"side_data_type":"Display Matrix","rotation":-90}]}],
            "format":{"format_name":"mov,mp4","duration":"2.0"}}"#,
        );
        let v = info.video.unwrap();
        assert_eq!(v.rotation, 90);
        assert_eq!((v.display_width, v.display_height), (1080, 1920));
    }

    #[test]
    fn reads_upside_down_and_legacy_tag() {
        let info = parse(
            r#"{"streams":[{"index":0,"codec_type":"video","width":640,"height":360,
            "side_data_list":[{"side_data_type":"Display Matrix","rotation":-180}]}],"format":{"duration":"1"}}"#,
        );
        assert_eq!(info.video.unwrap().rotation, 180);
        let info = parse(
            r#"{"streams":[{"index":0,"codec_type":"video","width":640,"height":360,"tags":{"rotate":"270"}}],
            "format":{"duration":"1"}}"#,
        );
        assert_eq!(info.video.unwrap().rotation, 270);
    }

    #[test]
    fn anamorphic_and_interlaced_dv() {
        let info = parse(
            r#"{"streams":[{"index":0,"codec_type":"video","codec_name":"dvvideo","width":720,"height":576,
            "sample_aspect_ratio":"64:45","avg_frame_rate":"25/1"}],"format":{"duration":"3.5"}}"#,
        );
        let v = info.video.unwrap();
        assert!(v.interlaced);
        assert_eq!((v.display_width, v.display_height), (1024, 576));
    }

    #[test]
    fn falls_back_to_stream_duration_and_detects_hdr() {
        let info = parse(
            r#"{"streams":[{"index":0,"codec_type":"video","codec_name":"hevc","width":3840,"height":2160,
            "duration":"N/A"},{"index":1,"codec_type":"audio","channels":2,"duration":"4.25","sample_rate":"44100"}],
            "format":{"duration":"N/A"}}"#,
        );
        assert_eq!(info.duration, 4.25);
        assert_eq!(info.audio.as_ref().unwrap().sample_rate, 44_100);
        let info = parse(
            r#"{"streams":[{"index":0,"codec_type":"video","width":1920,"height":1080,"color_transfer":"arib-std-b67",
            "field_order":"progressive"}],"format":{"duration":"2"}}"#,
        );
        let v = info.video.unwrap();
        assert_eq!(v.hdr, Some(HdrTransfer::Hlg));
        assert!(!v.interlaced);
    }

    #[test]
    fn skips_cover_art_and_prefers_default_audio() {
        let info = parse(
            r#"{"streams":[
              {"index":0,"codec_type":"video","codec_name":"png","width":600,"height":600,"disposition":{"attached_pic":1}},
              {"index":1,"codec_type":"video","codec_name":"h264","width":1280,"height":720},
              {"index":2,"codec_type":"audio","channels":2},
              {"index":3,"codec_type":"audio","channels":6,"disposition":{"default":1}}],
              "format":{"format_name":"mov,mp4","duration":"10"}}"#,
        );
        assert_eq!(info.kind, MediaKind::Video);
        assert_eq!(info.video.unwrap().stream_index, 1);
        assert_eq!(info.audio.unwrap().stream_index, 3);
        assert_eq!(info.audio_streams, 2);
    }

    #[test]
    fn classifies_photos_and_audio() {
        let photo = parse(
            r#"{"streams":[{"index":0,"codec_type":"video","codec_name":"mjpeg","width":4000,"height":3000}],
            "format":{"format_name":"image2"}}"#,
        );
        assert_eq!(photo.kind, MediaKind::Photo);
        assert!(photo.video.is_none());
        let mp3 = parse(
            r#"{"streams":[{"index":0,"codec_type":"audio","channels":2},
            {"index":1,"codec_type":"video","codec_name":"mjpeg","width":500,"height":500,"disposition":{"attached_pic":1}}],
            "format":{"format_name":"mp3","duration":"180"}}"#,
        );
        assert_eq!(mp3.kind, MediaKind::AudioOnly);
        let empty = parse(r#"{"streams":[],"format":{}}"#);
        assert_eq!(empty.kind, MediaKind::Unsupported);
    }

    #[test]
    fn variable_frame_rate_uses_average() {
        let info = parse(
            r#"{"streams":[{"index":0,"codec_type":"video","width":1920,"height":1080,
            "r_frame_rate":"90000/1","avg_frame_rate":"2997/100"}],"format":{"duration":"5"}}"#,
        );
        assert!((info.video.unwrap().frame_rate - 29.97).abs() < 1e-9);
    }

    #[test]
    fn recording_time_sources() {
        let info = parse(
            r#"{"streams":[{"index":0,"codec_type":"video","width":8,"height":8}],
            "format":{"duration":"1","tags":{"com.apple.quicktime.creationdate":"2023-07-14T15:30:12+0300",
            "creation_time":"2023-07-14T12:30:12.000000Z"}}}"#,
        );
        assert_eq!(info.recorded_at_source, Some(DateSource::Metadata));
        assert_eq!(
            info.recorded_at.unwrap().offset().local_minus_utc(),
            3 * 3600
        );
        // Camera clocks that were never set are ignored.
        let info = parse_probe(
            Path::new("/x/VID_20190102_030405.mp4"),
            br#"{"streams":[{"index":0,"codec_type":"video","width":8,"height":8}],
            "format":{"duration":"1","tags":{"creation_time":"1970-01-01T00:00:00.000000Z"}}}"#,
            1,
            None,
        )
        .unwrap();
        assert_eq!(info.recorded_at_source, Some(DateSource::FileName));
    }

    #[test]
    fn file_name_dates() {
        let dt =
            |s: &str| date_from_file_name(s).map(|d| d.format("%Y-%m-%d %H:%M:%S").to_string());
        assert_eq!(
            dt("VID_20230714_153012").as_deref(),
            Some("2023-07-14 15:30:12")
        );
        assert_eq!(
            dt("PXL_20230714_153012345").as_deref(),
            Some("2023-07-14 15:30:12")
        );
        assert_eq!(
            dt("DJI_20230714153012_0001").as_deref(),
            Some("2023-07-14 15:30:12")
        );
        assert_eq!(
            dt("2023-07-14 15.30.12").as_deref(),
            Some("2023-07-14 15:30:12")
        );
        assert_eq!(
            dt("IMG-20230714-WA0003").as_deref(),
            Some("2023-07-14 00:00:00")
        );
        assert_eq!(dt("IMG_1234"), None);
        assert_eq!(dt("clip 20231345"), None);
        assert_eq!(dt("1234567890123"), None);
    }
}
