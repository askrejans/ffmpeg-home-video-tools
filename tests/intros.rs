//! Animated title intros rendered end to end.

mod common;

use common::{corpus, mean_luma, probe_entry};
use ffmpeg_video_processor::project::{
    AudioOptions, Clip, FrameRate, Intro, Output, Preset, Project, Quality, Transition,
};
use ffmpeg_video_processor::render::{RenderOptions, render};
use ffmpeg_video_processor::titles::TitleTemplate;
use ffmpeg_video_processor::{CancelToken, Error};
use std::collections::BTreeMap;
use std::path::Path;

fn project(template: &str, title: &str, out: &Path, w: u32, h: u32) -> Project {
    let mut fields = BTreeMap::new();
    fields.insert("title".to_string(), title.to_string());
    fields.insert(
        "subtitle".to_string(),
        "Jūrmala · Ελλάδα · Москва".to_string(),
    );
    fields.insert("date".to_string(), "July 2026".to_string());
    Project {
        clips: vec![
            Clip::new(corpus().path("landscape.mp4")),
            Clip::new(corpus().path("rotated90.mp4")),
        ],
        intro: Some(Intro {
            template: template.to_string(),
            fields,
        }),
        transition: Transition::default(),
        output: Output {
            preset: Preset::Custom,
            width: Some(w),
            height: Some(h),
            fps: FrameRate::Fixed(25),
            quality: Quality::Draft,
            ..Output::new(out)
        },
        audio: AudioOptions::default(),
        watermark: None,
    }
}

#[test]
fn every_builtin_template_renders_in_both_orientations() {
    let tools = require_ffmpeg!();
    let dir = tempfile::tempdir().unwrap().keep();
    assert!(
        !TitleTemplate::builtin_names().is_empty(),
        "no built-in templates"
    );
    for name in TitleTemplate::builtin_names() {
        let template = TitleTemplate::builtin(name).unwrap();
        for (w, h) in [(640, 360), (360, 640)] {
            let out = dir.join(format!("{name}-{w}x{h}.mp4"));
            let p = project(name, "Vasara pie vecmāmiņas", &out, w, h);
            let outcome = render(
                &tools,
                &p,
                &RenderOptions::default(),
                &mut |_| {},
                &CancelToken::new(),
            )
            .unwrap_or_else(|e| panic!("{name} {w}x{h}: {e}"));
            let intro_frames = (template.duration() * 25.0).round() as u64;
            assert!(outcome.frames > intro_frames, "{name}: intro plus clips");
            assert_eq!(probe_entry(&out, "v:0", "width,height"), format!("{w},{h}"));
        }
    }
}

#[test]
fn intro_only_renders_a_title_preview_with_visible_text() {
    let tools = require_ffmpeg!();
    let dir = tempfile::tempdir().unwrap().keep();
    let name = TitleTemplate::builtin_names()[0];
    let out = dir.join("preview.mp4");
    let p = project(name, "Ģimenes svētki", &out, 640, 360);
    let outcome = render(
        &tools,
        &p,
        &RenderOptions {
            intro_only: true,
            ..Default::default()
        },
        &mut |_| {},
        &CancelToken::new(),
    )
    .unwrap();
    let template = TitleTemplate::builtin(name).unwrap();
    assert_eq!(outcome.frames, (template.duration() * 25.0).round() as u64);
    // Mid-intro the title area should not be a flat backdrop: compare the
    // centre band with the same band at the very first frame.
    let mid = outcome.frames / 2;
    let centre_mid = mean_luma(&out, mid as u32, 80, 120, 480, 120);
    let centre_start = mean_luma(&out, 0, 80, 120, 480, 120);
    assert!(
        (centre_mid - centre_start).abs() > 1.0,
        "title not visible: {centre_start} → {centre_mid}"
    );
}

#[test]
fn missing_required_title_is_reported() {
    let tools = require_ffmpeg!();
    let dir = tempfile::tempdir().unwrap().keep();
    let mut p = project(
        TitleTemplate::builtin_names()[0],
        "",
        &dir.join("x.mp4"),
        320,
        180,
    );
    p.intro.as_mut().unwrap().fields.remove("title");
    let err = render(
        &tools,
        &p,
        &RenderOptions::default(),
        &mut |_| {},
        &CancelToken::new(),
    )
    .unwrap_err();
    assert!(matches!(err, Error::Template(_)), "{err}");
}
