//! Rendering a [`Project`] to a finished MP4.

mod audio;
mod decode;
pub(crate) mod filters;
mod intro;
mod verify;
mod video;

pub use audio::LoudnessReport;

use crate::encoders::{select_audio_encoder, select_video_encoder, video_bitrate_kbps};
use crate::error::{Error, Result};
use crate::events::{CancelToken, Event, EventSink, Stage, warning};
use crate::frame::Stamp;
use crate::plan::{RenderPlan, SegmentSource, plan};
use crate::probe::{MediaInfo, probe};
use crate::project::Project;
use crate::titles::{TitleRenderer, TitleTemplate};
use crate::tools::FfmpegTools;
use rayon::prelude::*;
use serde::Serialize;
use std::path::{Path, PathBuf};

/// Runtime options that are not part of the movie description.
#[derive(Debug, Clone, Default)]
pub struct RenderOptions {
    /// Where to create the job's temporary folder (default: the system
    /// temporary directory). The folder is removed afterwards.
    pub work_dir: Option<PathBuf>,
    /// Keep the temporary folder for debugging.
    pub keep_work_dir: bool,
    /// Decode in software only.
    pub no_hardware_decode: bool,
    /// Render only the intro (title preview). Still needs at least one clip
    /// for templates with a footage background.
    pub intro_only: bool,
}

/// What a finished render produced.
#[derive(Debug, Clone, Serialize)]
pub struct RenderOutcome {
    pub output: PathBuf,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub frames: u64,
    pub duration_seconds: f64,
    pub size_bytes: u64,
    pub video_encoder: String,
    pub audio_encoder: String,
    pub loudness: Option<LoudnessReport>,
    /// Warnings that were also emitted as events.
    pub warnings: Vec<Event>,
}

/// Load an intro template by built-in name or directory path.
pub fn load_template(name: &str) -> Result<TitleTemplate> {
    if TitleTemplate::builtin_names().contains(&name) {
        TitleTemplate::builtin(name)
    } else {
        TitleTemplate::load(Path::new(name))
    }
}

/// Probe every clip of a project (in parallel, in order).
pub fn probe_project(
    tools: &FfmpegTools,
    project: &Project,
    cancel: &CancelToken,
) -> Result<Vec<MediaInfo>> {
    project
        .clips
        .par_iter()
        .map(|clip| probe(tools, &clip.path, cancel))
        .collect()
}

/// Render a project. Progress, warnings and completion are reported through
/// `events`; cancelling `cancel` stops the job and removes its files.
pub fn render(
    tools: &FfmpegTools,
    project: &Project,
    options: &RenderOptions,
    events: EventSink,
    cancel: &CancelToken,
) -> Result<RenderOutcome> {
    project.validate()?;
    events(Event::Stage {
        stage: Stage::Preparing,
    });
    let media = probe_project(tools, project, cancel)?;
    render_probed(tools, project, &media, options, events, cancel)
}

/// [`render`] for callers that already probed the clips (`media[i]`
/// describes `project.clips[i]`).
pub fn render_probed(
    tools: &FfmpegTools,
    project: &Project,
    media: &[MediaInfo],
    options: &RenderOptions,
    events: EventSink,
    cancel: &CancelToken,
) -> Result<RenderOutcome> {
    project.validate()?;
    let mut warnings = Vec::new();
    let mut warn = |event: Event, events: &mut dyn FnMut(Event)| {
        warnings.push(event.clone());
        events(event);
    };
    let template = project
        .intro
        .as_ref()
        .map(|i| load_template(&i.template))
        .transpose()?;
    let mut plan = plan(project, media, template.as_ref().map(|t| t.duration()))?;
    if options.intro_only {
        if template.is_none() {
            return Err(Error::InvalidProject(
                "an intro-only render needs an intro".into(),
            ));
        }
        plan.segments.truncate(1);
        plan.joins.clear();
        plan.total_frames = plan.segments[0].frames;
        plan.warnings.clear();
    }
    for w in &plan.warnings {
        warn(warning(w.code, w.message.clone(), w.clip), &mut *events);
    }
    let tonemap = tools.has_filter("zscale") && tools.has_filter("tonemap");
    for (i, m) in media.iter().enumerate() {
        if m.video.as_ref().is_some_and(|v| v.hdr.is_some()) && !tonemap {
            warn(
                warning(
                    "hdr_not_tonemapped",
                    format!(
                        "{} is HDR but this FFmpeg cannot tone-map it; colours may look pale",
                        m.path.display()
                    ),
                    Some(i),
                ),
                &mut *events,
            );
        }
        if m.audio_streams > 1 {
            warn(
                warning(
                    "several_audio_tracks",
                    format!(
                        "{} has several audio tracks; using the main one",
                        m.path.display()
                    ),
                    Some(i),
                ),
                &mut *events,
            );
        }
    }
    cancel.check()?;

    let video_encoder = select_video_encoder(
        tools,
        plan.width,
        plan.height,
        project.output.encoder.as_deref(),
        cancel,
    )?;
    let audio_encoder = select_audio_encoder(tools, cancel)?;
    let base = options.work_dir.clone().unwrap_or_else(std::env::temp_dir);
    std::fs::create_dir_all(&base)
        .map_err(|e| Error::io(format!("creating {}", base.display()), e))?;
    let destination = destination_path(&project.output.path, project.output.overwrite)?;
    check_space(&plan, project, &base, &destination)?;
    let work = tempfile::Builder::new()
        .prefix("fvp-render-")
        .tempdir_in(&base)
        .map_err(|e| Error::io(format!("creating a work folder in {}", base.display()), e))?;

    let stamp = project
        .watermark
        .as_ref()
        .map(|wm| Stamp::new(&wm.image, plan.width, plan.height, wm))
        .transpose()?;
    let intro = match (&project.intro, &template) {
        (Some(intro), Some(template)) => {
            let renderer =
                TitleRenderer::new(template, &intro.fields, plan.width, plan.height, plan.fps)?;
            let first_clip = plan.segments.iter().find_map(|s| match &s.source {
                SegmentSource::Clip { media, start, .. } => Some((&**media, *start)),
                SegmentSource::Intro => None,
            });
            let fallback_media = media
                .first()
                .map(|m| m.with_rotation(project.clips[0].rotation))
                .transpose()?;
            let first_clip = first_clip.or_else(|| {
                fallback_media
                    .as_ref()
                    .map(|m| (m, project.clips[0].trim.map_or(0.0, |t| t.start)))
            });
            Some(intro::IntroSource::new(
                renderer,
                template.background(),
                first_clip,
                plan.width,
                plan.height,
                plan.fps,
                tonemap,
                !options.no_hardware_decode,
            ))
        }
        _ => None,
    };

    // Audio first: it is quick and the video encoder muxes it.
    events(Event::Stage {
        stage: Stage::Audio,
    });
    let sound = template.as_ref().and_then(|t| t.sound());
    let (audio_path, loudness) = audio::render_audio(
        &audio::AudioJob {
            tools,
            plan: &plan,
            options: &project.audio,
            intro_sound: sound.as_deref(),
            work: work.path(),
            codec: &audio_encoder,
            cancel,
        },
        &mut *events,
    )?;

    events(Event::Stage {
        stage: Stage::Video,
    });
    let rendered = work.path().join("movie.mp4");
    let summaries = video::render_video(
        &video::VideoJob {
            tools,
            plan: &plan,
            encoder: &video_encoder,
            quality: project.output.quality,
            audio: &audio_path,
            output: &rendered,
            title: project.output.title.as_deref(),
            stamp: stamp.as_ref(),
            intro: intro.as_ref(),
            tonemap,
            hardware_decode: !options.no_hardware_decode,
            cancel,
        },
        &mut *events,
    )?;
    for (segment, summary) in summaries {
        let clip = plan.segments[segment].clip_index();
        if summary.padded > 1 || summary.truncated {
            let name = clip
                .map(|c| project.clips[c].path.display().to_string())
                .unwrap_or_default();
            warn(
                warning(
                    "clip_ended_early",
                    format!("{name} ended earlier than expected; its last picture was held"),
                    clip,
                ),
                &mut *events,
            );
        }
    }

    events(Event::Stage {
        stage: Stage::Verifying,
    });
    verify::verify(tools, &rendered, &plan, cancel)?;

    events(Event::Stage {
        stage: Stage::Finishing,
    });
    let size_bytes = std::fs::metadata(&rendered).map(|m| m.len()).unwrap_or(0);
    let output = move_into_place(&rendered, &destination, project.output.overwrite)?;
    if options.keep_work_dir {
        let kept = work.keep();
        tracing::info!(path = %kept.display(), "kept work folder");
    }
    events(Event::Progress {
        fraction: 1.0,
        stage: Stage::Finishing,
        done: plan.total_frames,
        total: plan.total_frames,
        fps: None,
        eta_seconds: Some(0.0),
        clip: None,
    });
    events(Event::Done {
        output: output.clone(),
        duration_seconds: plan.duration_seconds(),
        size_bytes,
    });
    Ok(RenderOutcome {
        output,
        width: plan.width,
        height: plan.height,
        fps: plan.fps,
        frames: plan.total_frames,
        duration_seconds: plan.duration_seconds(),
        size_bytes,
        video_encoder: video_encoder.id,
        audio_encoder,
        loudness,
        warnings,
    })
}

/// The output path with `.mp4`, its folder created, and a free name unless
/// overwriting.
fn destination_path(requested: &Path, overwrite: bool) -> Result<PathBuf> {
    let mut path = requested.to_path_buf();
    if path.extension().is_none() {
        path.set_extension("mp4");
    }
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)
            .map_err(|e| Error::io(format!("creating {}", parent.display()), e))?;
    }
    if overwrite || !path.exists() {
        return Ok(path);
    }
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().into_owned())
        .unwrap_or_default();
    for n in 2.. {
        let candidate = path.with_file_name(format!("{stem} ({n}).{ext}"));
        if !candidate.exists() {
            return Ok(candidate);
        }
    }
    unreachable!()
}

/// Move the verified file into place without ever leaving a partial file at
/// the destination.
fn move_into_place(from: &Path, to: &Path, overwrite: bool) -> Result<PathBuf> {
    let to = if overwrite {
        to.to_path_buf()
    } else {
        destination_path(to, false)?
    };
    if std::fs::rename(from, &to).is_ok() {
        return Ok(to);
    }
    // Different volume: copy next to the destination, then rename.
    let partial = to.with_file_name(format!(
        ".{}.partial",
        to.file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default()
    ));
    std::fs::copy(from, &partial).map_err(|e| {
        let _ = std::fs::remove_file(&partial);
        Error::io(format!("copying the movie to {}", to.display()), e)
    })?;
    std::fs::rename(&partial, &to).map_err(|e| Error::io(format!("saving {}", to.display()), e))?;
    let _ = std::fs::remove_file(from);
    Ok(to)
}

fn available(path: &Path) -> Option<u64> {
    let mut probe = Some(path);
    while let Some(p) = probe {
        if p.exists() {
            return fs4::available_space(p).ok();
        }
        probe = p.parent();
    }
    None
}

/// Make sure the work folder and destination have room.
fn check_space(
    plan: &RenderPlan,
    project: &Project,
    work: &Path,
    destination: &Path,
) -> Result<()> {
    let seconds = plan.duration_seconds();
    let video = (f64::from(video_bitrate_kbps(
        plan.width,
        plan.height,
        plan.fps,
        project.output.quality,
    )) * 1.5
        + 256.0)
        * 1000.0
        / 8.0
        * seconds;
    // Segment PCM plus the mixed soundtrack, 32-bit float stereo at 48 kHz.
    let audio = plan.total_samples() as f64 * 8.0 * 2.0;
    let margin = 64.0 * 1024.0 * 1024.0;
    let work_needed = (video + audio + margin) as u64;
    let dest_needed = (video + margin) as u64;
    let same_volume = available(work) == available(destination.parent().unwrap_or(destination));
    for (path, needed) in [
        (
            work.to_path_buf(),
            if same_volume {
                work_needed + dest_needed
            } else {
                work_needed
            },
        ),
        (
            destination.parent().unwrap_or(destination).to_path_buf(),
            dest_needed,
        ),
    ] {
        if let Some(free) = available(&path)
            && free < needed
        {
            return Err(Error::DiskFull {
                path,
                needed_mb: needed / 1_048_576,
                available_mb: free / 1_048_576,
            });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn destination_gets_extension_and_free_name() {
        let dir = tempfile::tempdir().unwrap();
        let wanted = dir.path().join("Summer");
        assert_eq!(
            destination_path(&wanted, false).unwrap(),
            dir.path().join("Summer.mp4")
        );
        std::fs::write(dir.path().join("Summer.mp4"), b"x").unwrap();
        std::fs::write(dir.path().join("Summer (2).mp4"), b"x").unwrap();
        assert_eq!(
            destination_path(&wanted, false).unwrap(),
            dir.path().join("Summer (3).mp4")
        );
        assert_eq!(
            destination_path(&wanted, true).unwrap(),
            dir.path().join("Summer.mp4")
        );
    }

    #[test]
    fn moves_without_overwriting() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("work.mp4");
        std::fs::write(&src, b"movie").unwrap();
        std::fs::write(dir.path().join("Out.mp4"), b"old").unwrap();
        let out = move_into_place(&src, &dir.path().join("Out.mp4"), false).unwrap();
        assert_eq!(out, dir.path().join("Out (2).mp4"));
        assert_eq!(std::fs::read(dir.path().join("Out.mp4")).unwrap(), b"old");
        assert!(!src.exists());
    }
}
