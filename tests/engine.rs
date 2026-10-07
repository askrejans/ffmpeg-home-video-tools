//! End-to-end tests against real FFmpeg, using a generated edge-case corpus.

mod common;

use common::{VIDEOS, corpus, integrated_loudness, mean_luma, probe_entry};
use ffmpeg_video_processor::discover::{ScanKind, ScanOptions, scan};
use ffmpeg_video_processor::project::{
    AudioOptions, Clip, FrameRate, LoudnessTarget, Output, Preset, Project, Quality, Transition,
    TransitionKind, Trim, Watermark,
};
use ffmpeg_video_processor::render::{RenderOptions, RenderOutcome, render};
use ffmpeg_video_processor::{CancelToken, Error, Event, FfmpegTools, preview, probe};
use std::path::{Path, PathBuf};

fn project(clips: &[&str], out: &Path, w: u32, h: u32) -> Project {
    Project {
        clips: clips.iter().map(|c| Clip::new(corpus().path(c))).collect(),
        intro: None,
        transition: Transition::default(),
        output: Output {
            preset: Preset::Custom,
            width: Some(w),
            height: Some(h),
            quality: Quality::Draft,
            ..Output::new(out)
        },
        audio: AudioOptions::default(),
        watermark: None,
    }
}

fn run(tools: &FfmpegTools, project: &Project) -> (RenderOutcome, Vec<Event>) {
    let mut events = Vec::new();
    let outcome = render(
        tools,
        project,
        &RenderOptions::default(),
        &mut |e| events.push(e),
        &CancelToken::new(),
    )
    .unwrap_or_else(|e| panic!("render failed: {e} ({})", e.code()));
    (outcome, events)
}

fn out_dir() -> PathBuf {
    tempfile::tempdir().unwrap().keep()
}

#[test]
fn scan_classifies_the_corpus() {
    let tools = require_ffmpeg!();
    let items = scan(
        &tools,
        &[corpus().dir.clone()],
        &ScanOptions::default(),
        &CancelToken::new(),
        &|_| {},
    )
    .unwrap();
    let kind_of = |name: &str| {
        items
            .iter()
            .find(|i| i.path.file_name().unwrap() == name)
            .map(|i| i.kind)
    };
    for video in VIDEOS {
        assert_eq!(kind_of(video), Some(ScanKind::Video), "{video}");
    }
    assert_eq!(kind_of("broken.mp4"), Some(ScanKind::Unreadable));
    assert_eq!(kind_of("really_a_photo.mp4"), Some(ScanKind::Photo));
    assert_eq!(kind_of("song_only.mp4"), Some(ScanKind::AudioOnly));
    // Dot-files (including AppleDouble `._*` forks) are not even listed.
    assert_eq!(kind_of("._landscape.mp4"), None);
    assert_eq!(kind_of("GOPR0001.LRV"), Some(ScanKind::Skipped));
    assert_eq!(kind_of("notes.txt"), Some(ScanKind::Skipped));
}

#[test]
fn probe_reads_edge_cases() {
    let tools = require_ffmpeg!();
    let c = CancelToken::new();
    let p = |name: &str| probe(&tools, &corpus().path(name), &c).unwrap();
    let v = p("rotated90.mp4").video.unwrap();
    assert_eq!((v.display_width, v.display_height), (360, 640));
    assert_eq!(p("rotated180.mp4").video.unwrap().rotation, 180);
    let dv = p("camcorder.dv").video.unwrap();
    assert!(dv.interlaced);
    assert_eq!((dv.display_width, dv.display_height), (1024, 576));
    assert!(p("dvd.mpg").video.unwrap().interlaced);
    assert!(p("hdr_hlg.mkv").video.unwrap().hdr.is_some());
    assert!(p("portrait_silent.mkv").audio.is_none());
    let two = p("two_tracks.mkv");
    assert_eq!(two.audio_streams, 2);
    assert_eq!(two.audio.unwrap().stream_index, 2);
    let cover = p("with_cover.mp4");
    assert_eq!(cover.video.unwrap().width, 640);
    let vfr = p("phone_vfr.mkv").video.unwrap();
    assert!((vfr.frame_rate - 30.0).abs() < 3.0, "{}", vfr.frame_rate);
}

#[test]
fn renders_every_kind_of_clip_landscape_and_portrait() {
    let tools = require_ffmpeg!();
    let dir = out_dir();
    for (w, h) in [(640, 360), (360, 640)] {
        let out = dir.join(format!("all-{w}x{h}.mp4"));
        let mut p = project(VIDEOS, &out, w, h);
        p.transition = Transition {
            kind: TransitionKind::Mix,
            duration: 0.4,
        };
        p.output.fps = FrameRate::Fixed(25);
        let (outcome, events) = run(&tools, &p);
        assert_eq!(outcome.output, out);
        assert_eq!((outcome.width, outcome.height, outcome.fps), (w, h, 25));
        assert_eq!(probe_entry(&out, "v:0", "width,height"), format!("{w},{h}"));
        assert_eq!(
            probe_entry(&out, "v:0", "nb_frames"),
            outcome.frames.to_string()
        );
        assert!(events.iter().any(|e| matches!(e, Event::Done { .. })));
        // Every clip contributed: total equals the sum minus transitions (checked by the plan),
        // and the duration is far beyond any single clip.
        assert!(
            outcome.duration_seconds > 15.0,
            "{}",
            outcome.duration_seconds
        );
    }
}

#[test]
fn same_stem_files_are_both_used() {
    let tools = require_ffmpeg!();
    let out = out_dir().join("stems.mp4");
    let mut p = project(&["landscape.mp4", "landscape.mov"], &out, 320, 180);
    p.transition.kind = TransitionKind::Cut;
    p.output.fps = FrameRate::Fixed(30);
    let (outcome, _) = run(&tools, &p);
    assert_eq!(outcome.frames, 60 + 45);
}

#[test]
fn trims_are_frame_exact_and_audio_matches_video() {
    let tools = require_ffmpeg!();
    let out = out_dir().join("trim.mp4");
    let mut p = project(&["late_audio.mp4", "landscape.mp4"], &out, 320, 180);
    p.clips[0].trim = Some(Trim {
        start: 0.25,
        end: Some(1.25),
    });
    p.clips[1].trim = Some(Trim {
        start: 0.5,
        end: None,
    });
    p.transition.kind = TransitionKind::Cut;
    p.output.fps = FrameRate::Fixed(30);
    let (outcome, _) = run(&tools, &p);
    assert_eq!(outcome.frames, 30 + 45);
    let video: f64 = probe_entry(&out, "v:0", "duration").parse().unwrap();
    let audio: f64 = probe_entry(&out, "a:0", "duration").parse().unwrap();
    assert!((video - 2.5).abs() < 0.04, "{video}");
    assert!((audio - video).abs() < 0.06, "audio {audio} video {video}");
}

#[test]
fn every_transition_renders() {
    let tools = require_ffmpeg!();
    let dir = out_dir();
    for kind in TransitionKind::ALL {
        let out = dir.join(format!("{}.mp4", kind.name()));
        let mut p = project(&["landscape.mp4", "rotated90.mp4"], &out, 320, 180);
        p.transition = Transition {
            kind,
            duration: 0.6,
        };
        p.output.fps = FrameRate::Fixed(30);
        let (outcome, _) = run(&tools, &p);
        let expected = if kind == TransitionKind::Cut {
            120
        } else {
            120 - 18
        };
        assert_eq!(outcome.frames, expected, "{kind:?}");
    }
}

#[test]
fn crossfades_never_dip_to_black() {
    let tools = require_ffmpeg!();
    let out = out_dir().join("noblack.mp4");
    let mut p = project(
        &["landscape.mp4", "landscape.mov", "tiny.avi"],
        &out,
        320,
        180,
    );
    p.output.fps = FrameRate::Fixed(30);
    run(&tools, &p);
    let probe = std::process::Command::new("ffmpeg")
        .args(["-nostdin", "-hide_banner", "-i"])
        .arg(&out)
        .args([
            "-vf",
            "blackdetect=d=0.04:pix_th=0.1",
            "-an",
            "-f",
            "null",
            "-",
        ])
        .output()
        .unwrap();
    let log = String::from_utf8_lossy(&probe.stderr);
    assert!(!log.contains("black_start"), "{log}");
}

#[test]
fn two_pass_ebu_r128_hits_the_target() {
    let tools = require_ffmpeg!();
    for target in [-23.0, -16.0] {
        let out = out_dir().join(format!("loud{target}.mp4"));
        let mut p = project(
            &["landscape.mp4", "dvd.mpg", "late_audio.mp4"],
            &out,
            320,
            180,
        );
        p.audio.loudness = Some(LoudnessTarget {
            integrated: target,
            true_peak: -1.0,
        });
        let (outcome, _) = run(&tools, &p);
        let report = outcome.loudness.expect("loudness report");
        assert!(report.input_integrated.is_finite());
        let measured = integrated_loudness(&out);
        assert!(
            (measured - target).abs() < 1.0,
            "target {target}, got {measured}"
        );
    }
}

#[test]
fn silent_movie_skips_normalisation_but_has_audio() {
    let tools = require_ffmpeg!();
    let out = out_dir().join("silent.mp4");
    let p = project(&["portrait_silent.mkv", "hdr_hlg.mkv"], &out, 320, 180);
    let (outcome, _) = run(&tools, &p);
    assert!(outcome.loudness.is_none());
    assert_eq!(probe_entry(&out, "a:0", "codec_name"), "aac");
}

#[test]
fn watermark_is_burned_into_the_corner() {
    let tools = require_ffmpeg!();
    let dir = out_dir();
    let mark = dir.join("mark.png");
    let status = std::process::Command::new("ffmpeg")
        .args([
            "-nostdin",
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            "color=c=white:s=200x100,format=rgba",
            "-frames:v",
            "1",
        ])
        .arg(&mark)
        .status()
        .unwrap();
    assert!(status.success());
    let out = dir.join("marked.mp4");
    let mut p = project(&["portrait_silent.mkv"], &out, 640, 360);
    p.watermark = Some(Watermark {
        image: mark,
        anchor: Default::default(),
        width_fraction: 0.25,
        margin_fraction: 0.03,
        opacity: 1.0,
    });
    run(&tools, &p);
    // The 160×80 mark sits 10 px from the bottom-right corner.
    let corner = mean_luma(&out, 5, 640 - 10 - 150, 360 - 10 - 70, 140, 60);
    assert!(corner > 225.0, "corner luma {corner}");
}

#[test]
fn cancelling_stops_and_cleans_up() {
    let tools = require_ffmpeg!();
    let dir = out_dir();
    let work = dir.join("work");
    std::fs::create_dir(&work).unwrap();
    let out = dir.join("cancelled.mp4");
    let p = project(VIDEOS, &out, 1280, 720);
    let cancel = CancelToken::new();
    let trigger = cancel.clone();
    let mut seen_video = false;
    let result = render(
        &tools,
        &p,
        &RenderOptions {
            work_dir: Some(work.clone()),
            ..Default::default()
        },
        &mut |e| {
            if matches!(
                e,
                Event::Progress {
                    stage: ffmpeg_video_processor::Stage::Video,
                    ..
                }
            ) && !seen_video
            {
                seen_video = true;
                trigger.cancel();
            }
        },
        &cancel,
    );
    assert!(matches!(result, Err(Error::Cancelled)), "{result:?}");
    assert!(!out.exists());
    assert_eq!(
        std::fs::read_dir(&work).unwrap().count(),
        0,
        "work folder left behind"
    );
}

#[test]
fn existing_outputs_are_never_overwritten() {
    let tools = require_ffmpeg!();
    let dir = out_dir();
    let out = dir.join("Holiday.mp4");
    std::fs::write(&out, b"keep me").unwrap();
    let p = project(&["tiny.avi"], &out, 320, 180);
    let (outcome, _) = run(&tools, &p);
    assert_eq!(outcome.output, dir.join("Holiday (2).mp4"));
    assert_eq!(std::fs::read(&out).unwrap(), b"keep me");
}

#[test]
fn unreadable_clips_fail_with_a_clear_code() {
    let tools = require_ffmpeg!();
    let out = out_dir().join("bad.mp4");
    let p = project(&["landscape.mp4", "broken.mp4"], &out, 320, 180);
    let err = render(
        &tools,
        &p,
        &RenderOptions::default(),
        &mut |_| {},
        &CancelToken::new(),
    )
    .unwrap_err();
    assert_eq!(err.code(), "clip_unreadable");
}

#[test]
fn previews() {
    let tools = require_ffmpeg!();
    let c = CancelToken::new();
    let dir = out_dir();
    let media = probe(&tools, &corpus().path("rotated90.mp4"), &c).unwrap();
    let thumb = preview::thumbnail(&tools, &media, 0.5, 200, &dir.join("t.jpg"), &c).unwrap();
    assert_eq!(probe_entry(&thumb, "v:0", "width,height"), "200,356");
    let strip = preview::filmstrip(&tools, &media, 6, 60, &dir.join("strip"), "f", &c).unwrap();
    assert!(strip.len() >= 5, "{}", strip.len());
    let dv = probe(&tools, &corpus().path("camcorder.dv"), &c).unwrap();
    let proxy = preview::proxy(&tools, &dv, 180, &dir.join("p.mp4"), &mut |_| {}, &c).unwrap();
    assert_eq!(probe_entry(&proxy, "v:0", "width,height"), "320,180");
    assert_eq!(probe_entry(&proxy, "v:0", "sample_aspect_ratio"), "1:1");
}

#[test]
fn manual_rotation_matches_movies_and_all_preview_types_after_metadata() {
    use std::process::Command;
    let tools = require_ffmpeg!();
    let cancel = CancelToken::new();
    let dir = out_dir();
    let coded = dir.join("quadrants.mp4");
    let photo_template = dir.join("photo-template");
    std::fs::create_dir(&photo_template).unwrap();
    std::fs::write(photo_template.join("template.json"),
        r##"{"name":"photo","duration":0.4,"fields":[],"background":{"kind":"solid","color":"#000000"},"layers":[{"type":"image","slot":"first_clip","width":1,"height":1,"fit":"contain"}]}"##).unwrap();
    assert!(Command::new(tools.ffmpeg_path()).args([
        "-hide_banner", "-v", "error", "-f", "lavfi", "-i",
        "color=c=red:s=160x90:r=30:d=0.4,drawbox=x=80:y=0:w=80:h=45:color=lime:t=fill,drawbox=x=0:y=45:w=80:h=45:color=blue:t=fill,drawbox=x=80:y=45:w=80:h=45:color=yellow:t=fill",
        "-c:v", "mpeg4", "-q:v", "2", "-y",
    ]).arg(&coded).status().unwrap().success());
    let corner_colours = |path: &Path| {
        let size: Vec<usize> = probe_entry(path, "v:0", "width,height")
            .split(',')
            .map(|v| v.parse().unwrap())
            .collect();
        let output = Command::new(tools.ffmpeg_path())
            .args(["-hide_banner", "-v", "error", "-i"])
            .arg(path)
            .args([
                "-frames:v",
                "1",
                "-f",
                "rawvideo",
                "-pix_fmt",
                "rgb24",
                "pipe:1",
            ])
            .output()
            .unwrap();
        assert!(output.status.success());
        let w = size[0];
        let h = size[1];
        let palette = [[255i32, 0, 0], [0, 255, 0], [0, 0, 255], [255, 255, 0]];
        [(36, 36), (64, 36), (36, 64), (64, 64)].map(|(x, y)| {
            let offset = (h * y / 100 * w + w * x / 100) * 3;
            let pixel = &output.stdout[offset..offset + 3];
            palette
                .iter()
                .enumerate()
                .min_by_key(|(_, colour)| {
                    pixel
                        .iter()
                        .zip(*colour)
                        .map(|(&a, &b)| (i32::from(a) - b).abs())
                        .sum::<i32>()
                })
                .unwrap()
                .0
        })
    };
    for metadata in [0, 90] {
        let source = dir.join(format!("metadata-{metadata}.mp4"));
        assert!(
            Command::new(tools.ffmpeg_path())
                .args(["-hide_banner", "-v", "error", "-display_rotation"])
                .arg((-metadata).to_string())
                .arg("-i")
                .arg(&coded)
                .args(["-c", "copy", "-y"])
                .arg(&source)
                .status()
                .unwrap()
                .success()
        );
        let original = probe(&tools, &source, &cancel).unwrap();
        assert_eq!(original.video.as_ref().unwrap().rotation, metadata as u32);
        for manual in [90, 180, 270] {
            let expected = match (metadata + manual) % 360 {
                0 => [0, 1, 2, 3],
                90 => [2, 0, 3, 1],
                180 => [3, 2, 1, 0],
                _ => [1, 3, 0, 2],
            };
            let movie = dir.join(format!("movie-{metadata}-{manual}.mp4"));
            let mut p = project(&[], &movie, 160, 160);
            p.clips = vec![Clip::new(&source)];
            p.clips[0].rotation = manual;
            p.audio.loudness = None;
            run(&tools, &p);
            assert_eq!(
                corner_colours(&movie),
                expected,
                "movie metadata={metadata} manual={manual}"
            );
            let intro_output = dir.join(format!("intro-{metadata}-{manual}.mp4"));
            let mut intro_project = p.clone();
            intro_project.output.path = intro_output.clone();
            intro_project.intro = Some(ffmpeg_video_processor::Intro {
                template: photo_template.to_string_lossy().into_owned(),
                fields: Default::default(),
            });
            render(
                &tools,
                &intro_project,
                &RenderOptions {
                    intro_only: true,
                    ..Default::default()
                },
                &mut |_| {},
                &cancel,
            )
            .unwrap();
            assert_eq!(
                corner_colours(&intro_output),
                expected,
                "title photo metadata={metadata} manual={manual}"
            );
            let media = original.with_rotation(manual).unwrap();
            let thumbnail = preview::thumbnail(
                &tools,
                &media,
                0.0,
                160,
                &dir.join(format!("thumb-{metadata}-{manual}.jpg")),
                &cancel,
            )
            .unwrap();
            assert_eq!(
                corner_colours(&thumbnail),
                expected,
                "thumbnail metadata={metadata} manual={manual}"
            );
            let filmstrip = preview::filmstrip(
                &tools,
                &media,
                1,
                90,
                &dir.join(format!("strip-{metadata}-{manual}")),
                "frame",
                &cancel,
            )
            .unwrap();
            assert_eq!(
                corner_colours(&filmstrip[0]),
                expected,
                "filmstrip metadata={metadata} manual={manual}"
            );
            let proxy = preview::proxy(
                &tools,
                &media,
                90,
                &dir.join(format!("proxy-{metadata}-{manual}.mp4")),
                &mut |_| {},
                &cancel,
            )
            .unwrap();
            assert_eq!(
                corner_colours(&proxy),
                expected,
                "proxy metadata={metadata} manual={manual}"
            );
        }
    }
}
