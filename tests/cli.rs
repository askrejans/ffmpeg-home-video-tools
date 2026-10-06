//! Command-line behaviour.

mod common;

use assert_cmd::Command;
use common::corpus;
use predicates::prelude::*;

fn cli() -> Command {
    Command::cargo_bin("ffmpeg-video-processor").unwrap()
}

/// A folder with a few clips copied from the corpus.
fn folder(names: &[&str]) -> std::path::PathBuf {
    let dir = tempfile::tempdir().unwrap().keep();
    for n in names {
        std::fs::copy(corpus().path(n), dir.join(n)).unwrap();
    }
    dir
}

#[test]
fn help_lists_commands() {
    cli()
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("render").and(predicate::str::contains("process")));
}

#[test]
fn process_is_compatible_with_0_2() {
    let _tools = require_ffmpeg!();
    let input = folder(&["tiny.avi", "portrait_silent.mkv", "notes.txt"]);
    let output = tempfile::tempdir().unwrap().keep();
    cli()
        .args(["process", "--no-tui", "--profile", "fast"])
        .arg(&input)
        .arg(&output)
        .assert()
        .success();
    let made: Vec<_> = std::fs::read_dir(&output)
        .unwrap()
        .filter_map(|e| e.ok())
        .collect();
    assert_eq!(
        made.len(),
        1,
        "only the movie is written to the output folder"
    );
    let name = made[0].file_name().to_string_lossy().into_owned();
    assert!(
        name.starts_with("processed_vod_") && name.ends_with(".mp4"),
        "{name}"
    );
    assert_eq!(
        common::probe_entry(&made[0].path(), "v:0", "width,height"),
        "3840,2160"
    );
}

#[test]
fn validate_reports_bad_files() {
    let _tools = require_ffmpeg!();
    let good = folder(&["tiny.avi", "landscape.mp4"]);
    cli()
        .arg("validate")
        .arg(&good)
        .assert()
        .success()
        .stdout(predicate::str::contains("Valid:   2"));
    let bad = folder(&["tiny.avi", "broken.mp4"]);
    cli()
        .arg("validate")
        .arg(&bad)
        .assert()
        .failure()
        .stdout(predicate::str::contains("Invalid: 1"));
}

#[test]
fn json_render_streams_events_and_outcome() {
    let _tools = require_ffmpeg!();
    let input = folder(&["landscape.mp4", "rotated90.mp4"]);
    let out = input.join("out/movie.mp4");
    let assert = cli()
        .args([
            "render",
            "--json",
            "--preset",
            "320x180",
            "--quality",
            "draft",
            "--transition",
            "tape_rewind",
            "-o",
        ])
        .arg(&out)
        .arg(&input)
        .assert()
        .success();
    let stdout = String::from_utf8_lossy(&assert.get_output().stdout).into_owned();
    let lines: Vec<serde_json::Value> = stdout
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert!(lines.iter().any(|l| l["type"] == "progress"));
    assert!(lines.iter().any(|l| l["type"] == "done"));
    let outcome = lines.last().unwrap();
    assert_eq!(outcome["type"], "outcome");
    assert_eq!(outcome["outcome"]["width"], 320);
    assert!(out.exists());
}

#[test]
fn dry_run_plans_without_rendering() {
    let _tools = require_ffmpeg!();
    let input = folder(&["landscape.mp4", "landscape.mov"]);
    let out = input.join("never.mp4");
    let assert = cli()
        .args([
            "render",
            "--dry-run",
            "--json",
            "--fps",
            "25",
            "--trim",
            "landscape.mov=0.5-",
            "-o",
        ])
        .arg(&out)
        .arg(&input)
        .assert()
        .success();
    let plan: serde_json::Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();
    assert_eq!(plan["fps"], 25);
    assert_eq!(plan["segments"].as_array().unwrap().len(), 2);
    assert!(!out.exists());
}

#[test]
fn probe_json_and_errors() {
    let _tools = require_ffmpeg!();
    let assert = cli()
        .args(["probe", "--json"])
        .arg(corpus().path("camcorder.dv"))
        .assert()
        .success();
    let infos: serde_json::Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();
    assert_eq!(infos[0]["video"]["interlaced"], true);
    cli()
        .args(["render", "-o", "x.mp4", "/definitely/missing"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("error:"));
}
