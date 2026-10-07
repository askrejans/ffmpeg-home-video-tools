//! Turning a [`Project`] and probe results into a frame-exact timeline.
//!
//! Planning is pure: no I/O, so every timing rule is unit-tested.

use crate::error::{Error, Result};
use crate::probe::{MediaInfo, MediaKind};
use crate::project::{FRAME_RATES, FrameRate, Project, TransitionKind};
use serde::Serialize;

/// Audio sample rate of every render.
pub const SAMPLE_RATE: u32 = 48_000;

/// Longest share of a clip a transition may use.
const MAX_TRANSITION_SHARE: f64 = 0.4;

/// What a timeline segment shows.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SegmentSource {
    Intro,
    Clip {
        /// Index into `Project::clips`.
        clip: usize,
        media: Box<MediaInfo>,
        /// Normalized additional clockwise rotation, after source orientation.
        rotation: u32,
        /// Seconds into the source file.
        start: f64,
        end: f64,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Segment {
    pub source: SegmentSource,
    /// Frames this segment contributes, including frames shared with
    /// neighbouring transitions.
    pub frames: u64,
}

impl Segment {
    pub fn clip_index(&self) -> Option<usize> {
        match &self.source {
            SegmentSource::Clip { clip, .. } => Some(*clip),
            SegmentSource::Intro => None,
        }
    }
}

/// How segment `i` hands over to segment `i + 1`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Join {
    /// Never `Mix`; `Cut` when `frames` is 0.
    pub kind: TransitionKind,
    /// Frames during which both segments are visible.
    pub frames: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RenderPlan {
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub segments: Vec<Segment>,
    /// `segments.len() - 1` joins.
    pub joins: Vec<Join>,
    pub total_frames: u64,
    pub warnings: Vec<PlanWarning>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PlanWarning {
    pub code: &'static str,
    pub message: String,
    pub clip: Option<usize>,
}

impl RenderPlan {
    pub fn samples_per_frame(&self) -> u64 {
        u64::from(SAMPLE_RATE / self.fps)
    }

    pub fn total_samples(&self) -> u64 {
        self.total_frames * self.samples_per_frame()
    }

    pub fn duration_seconds(&self) -> f64 {
        self.total_frames as f64 / f64::from(self.fps)
    }

    /// Timeline frame at which segment `index` starts.
    pub fn segment_start(&self, index: usize) -> u64 {
        (0..index)
            .map(|i| self.segments[i].frames - self.joins[i].frames)
            .sum()
    }
}

const MIX: [TransitionKind; 8] = [
    TransitionKind::Crossfade,
    TransitionKind::Slide,
    TransitionKind::Crossfade,
    TransitionKind::Zoom,
    TransitionKind::Crossfade,
    TransitionKind::Wipe,
    TransitionKind::Crossfade,
    TransitionKind::BlurDissolve,
];

fn snap_rate(rate: f64) -> u32 {
    *FRAME_RATES
        .iter()
        .min_by(|a, b| {
            (f64::from(**a) - rate)
                .abs()
                .total_cmp(&(f64::from(**b) - rate).abs())
        })
        .expect("non-empty")
}

/// The standard frame rate covering the most footage.
pub fn auto_frame_rate(clips: &[(f64, f64)]) -> u32 {
    let mut weights = [0.0f64; FRAME_RATES.len()];
    for (rate, seconds) in clips {
        let snapped = snap_rate(*rate);
        let slot = FRAME_RATES
            .iter()
            .position(|r| *r == snapped)
            .expect("snapped");
        weights[slot] += seconds.max(0.001);
    }
    let best = (0..FRAME_RATES.len())
        .max_by(|a, b| weights[*a].total_cmp(&weights[*b]).then(a.cmp(b)))
        .expect("non-empty");
    if weights[best] == 0.0 {
        30
    } else {
        FRAME_RATES[best]
    }
}

/// Build the timeline. `media[i]` must describe `project.clips[i]`;
/// `intro_seconds` is the intro template's duration when there is one.
pub fn plan(
    project: &Project,
    media: &[MediaInfo],
    intro_seconds: Option<f64>,
) -> Result<RenderPlan> {
    project.validate()?;
    if media.len() != project.clips.len() {
        return Err(Error::InvalidProject(format!(
            "{} clips but {} probe results",
            project.clips.len(),
            media.len()
        )));
    }
    let (width, height) = project.output.size();

    // Resolve trims first; they feed the automatic frame rate.
    let mut ranges = Vec::with_capacity(media.len());
    for (i, (clip, info)) in project.clips.iter().zip(media).enumerate() {
        if info.kind != MediaKind::Video || info.video.is_none() {
            return Err(Error::Unreadable {
                path: clip.path.clone(),
                reason: "it is not a video".into(),
            });
        }
        let trim = clip.trim.unwrap_or(crate::project::Trim {
            start: 0.0,
            end: None,
        });
        let start = trim.start.min(info.duration);
        let end = trim.end.unwrap_or(info.duration).min(info.duration);
        if end - start <= 0.0 {
            return Err(Error::InvalidProject(format!(
                "clip {} has nothing left after trimming",
                i + 1
            )));
        }
        ranges.push((start, end));
    }

    let fps = match project.output.fps {
        FrameRate::Fixed(fps) => fps,
        FrameRate::Auto => auto_frame_rate(
            &media
                .iter()
                .zip(&ranges)
                .map(|(m, (s, e))| (m.video.as_ref().map_or(30.0, |v| v.frame_rate), e - s))
                .collect::<Vec<_>>(),
        ),
    };

    let mut segments = Vec::new();
    if let Some(seconds) = intro_seconds {
        segments.push(Segment {
            source: SegmentSource::Intro,
            frames: ((seconds * f64::from(fps)).round() as u64).max(1),
        });
    }
    for (i, (info, (start, end))) in media.iter().zip(&ranges).enumerate() {
        segments.push(Segment {
            source: SegmentSource::Clip {
                clip: i,
                media: Box::new(info.with_rotation(project.clips[i].rotation)?),
                rotation: project.clips[i].rotation.rem_euclid(360) as u32,
                start: *start,
                end: *end,
            },
            frames: (((end - start) * f64::from(fps)).round() as u64).max(1),
        });
    }

    let mut warnings = Vec::new();
    let wanted = (project.transition.duration * f64::from(fps)).round() as u64;
    let joins = (0..segments.len().saturating_sub(1))
        .map(|i| {
            let kind = match project.transition.kind {
                TransitionKind::Mix => MIX[i % MIX.len()],
                other => other,
            };
            let shortest = segments[i].frames.min(segments[i + 1].frames);
            let limit = (shortest as f64 * MAX_TRANSITION_SHARE).floor() as u64;
            let frames = if kind == TransitionKind::Cut {
                0
            } else {
                wanted.min(limit)
            };
            if kind != TransitionKind::Cut && frames < wanted {
                warnings.push(PlanWarning {
                    code: "transition_shortened",
                    message: format!("transition {} was shortened to fit a short clip", i + 1),
                    clip: segments[i + 1].clip_index(),
                });
            }
            if frames < 2 {
                Join {
                    kind: TransitionKind::Cut,
                    frames: 0,
                }
            } else {
                Join { kind, frames }
            }
        })
        .collect::<Vec<_>>();

    let total_frames = segments.iter().map(|s| s.frames).sum::<u64>()
        - joins.iter().map(|j| j.frames).sum::<u64>();

    Ok(RenderPlan {
        width,
        height,
        fps,
        segments,
        joins,
        total_frames,
        warnings,
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::probe::VideoInfo;
    use crate::project::{Clip, Output, Transition, Trim};
    use std::path::PathBuf;

    pub(crate) fn video(name: &str, seconds: f64, fps: f64) -> MediaInfo {
        MediaInfo {
            path: PathBuf::from(name),
            kind: MediaKind::Video,
            format: "mov,mp4".into(),
            duration: seconds,
            size_bytes: 1,
            video: Some(VideoInfo {
                stream_index: 0,
                codec: "h264".into(),
                width: 1920,
                height: 1080,
                sar_num: 1,
                sar_den: 1,
                rotation: 0,
                manual_rotation: 0,
                display_width: 1920,
                display_height: 1080,
                frame_rate: fps,
                interlaced: false,
                hdr: None,
                pix_fmt: "yuv420p".into(),
            }),
            audio: None,
            audio_streams: 0,
            recorded_at: None,
            recorded_at_source: None,
        }
    }

    fn project(clips: &[&str]) -> Project {
        Project {
            clips: clips.iter().map(|c| Clip::new(*c)).collect(),
            intro: None,
            transition: Transition::default(),
            output: Output::new("out.mp4"),
            audio: Default::default(),
            watermark: None,
        }
    }

    #[test]
    fn manual_rotation_follows_metadata_without_mutating_probe_results() {
        let mut media = video("a", 2.0, 30.0);
        let original = media.video.as_mut().unwrap();
        original.rotation = 90;
        original.display_width = 1080;
        original.display_height = 1920;
        for (degrees, normalized, width, height) in [
            (90, 90, 1920, 1080),
            (180, 180, 1080, 1920),
            (270, 270, 1920, 1080),
            (-90, 270, 1920, 1080),
        ] {
            let mut p = project(&["a"]);
            p.clips[0].rotation = degrees;
            let timeline = plan(&p, &[media.clone()], None).unwrap();
            let SegmentSource::Clip {
                media: adjusted,
                rotation,
                ..
            } = &timeline.segments[0].source
            else {
                panic!("expected clip");
            };
            let adjusted = adjusted.video.as_ref().unwrap();
            assert_eq!(*rotation, normalized);
            assert_eq!(adjusted.rotation, 90);
            assert_eq!(adjusted.manual_rotation, normalized);
            assert_eq!(
                (adjusted.display_width, adjusted.display_height),
                (width, height)
            );
        }
        assert_eq!(media.video.unwrap().manual_rotation, 0);
    }

    #[test]
    fn frame_exact_timeline_with_crossfades() {
        let p = project(&["a", "b", "c"]);
        let media = [
            video("a", 4.0, 30.0),
            video("b", 2.0, 29.97),
            video("c", 3.0, 30.0),
        ];
        let plan = plan(&p, &media, None).unwrap();
        assert_eq!(plan.fps, 30);
        assert_eq!(
            plan.segments.iter().map(|s| s.frames).collect::<Vec<_>>(),
            [120, 60, 90]
        );
        assert!(plan.joins.iter().all(|j| j.frames == 24)); // 40 % of the 2 s clip
        assert_eq!(plan.total_frames, 120 + 60 + 90 - 48);
        assert_eq!(plan.segment_start(2), 120 - 24 + 60 - 24);
        assert_eq!(plan.samples_per_frame(), 1600);
        assert_eq!(plan.warnings.len(), 2);
    }

    #[test]
    fn trims_and_intro() {
        let mut p = project(&["a", "b"]);
        p.clips[0].trim = Some(Trim {
            start: 1.0,
            end: Some(3.5),
        });
        p.clips[1].trim = Some(Trim {
            start: 0.5,
            end: Some(99.0),
        });
        p.transition = Transition {
            kind: TransitionKind::Cut,
            duration: 1.0,
        };
        let plan = plan(
            &p,
            &[video("a", 10.0, 25.0), video("b", 4.0, 25.0)],
            Some(5.0),
        )
        .unwrap();
        assert_eq!(plan.fps, 25);
        assert_eq!(plan.segments[0].source, SegmentSource::Intro);
        assert_eq!(
            plan.segments.iter().map(|s| s.frames).collect::<Vec<_>>(),
            [125, 63, 88]
        );
        assert!(
            plan.joins
                .iter()
                .all(|j| j.kind == TransitionKind::Cut && j.frames == 0)
        );
        assert_eq!(plan.total_frames, 125 + 63 + 88);
        match &plan.segments[2].source {
            SegmentSource::Clip { start, end, .. } => assert_eq!((*start, *end), (0.5, 4.0)),
            _ => unreachable!(),
        }
    }

    #[test]
    fn auto_rate_follows_most_footage() {
        assert_eq!(auto_frame_rate(&[(59.94, 10.0), (29.97, 3.0)]), 60);
        assert_eq!(auto_frame_rate(&[(25.0, 10.0), (30.0, 11.0)]), 30);
        assert_eq!(auto_frame_rate(&[(23.976, 1.0)]), 24);
        assert_eq!(auto_frame_rate(&[(48.0, 1.0)]), 50);
        assert_eq!(auto_frame_rate(&[(120.0, 1.0)]), 60);
        assert_eq!(auto_frame_rate(&[]), 30);
    }

    #[test]
    fn mix_rotates_and_tiny_clips_cut() {
        let mut p = project(&["a", "b", "c", "d"]);
        p.transition = Transition {
            kind: TransitionKind::Mix,
            duration: 0.5,
        };
        let media = [
            video("a", 5.0, 30.0),
            video("b", 5.0, 30.0),
            video("c", 0.1, 30.0),
            video("d", 5.0, 30.0),
        ];
        let plan = plan(&p, &media, None).unwrap();
        assert_eq!(plan.joins[0].kind, TransitionKind::Crossfade);
        assert_eq!(plan.joins[0].frames, 15);
        // A 3-frame clip cannot hold a transition.
        assert_eq!(plan.joins[1].kind, TransitionKind::Cut);
        assert_eq!(plan.joins[2].kind, TransitionKind::Cut);
        assert!(plan.joins.iter().all(|j| j.kind != TransitionKind::Mix));
    }

    #[test]
    fn rejects_non_videos_and_empty_trims() {
        let p = project(&["a"]);
        let mut photo = video("a", 1.0, 30.0);
        photo.kind = MediaKind::Photo;
        assert_eq!(
            plan(&p, &[photo], None).unwrap_err().code(),
            "clip_unreadable"
        );
        let mut p = project(&["a"]);
        p.clips[0].trim = Some(Trim {
            start: 9.0,
            end: None,
        });
        assert_eq!(
            plan(&p, &[video("a", 5.0, 30.0)], None).unwrap_err().code(),
            "invalid_project"
        );
        assert_eq!(plan(&p, &[], None).unwrap_err().code(), "invalid_project");
    }
}
