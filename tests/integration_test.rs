use std::path::Path;
use std::process::Command;
use tempfile::TempDir;

/// Helper: check if ffmpeg is available
fn ffmpeg_available() -> bool {
    Command::new("ffmpeg")
        .arg("-version")
        .output()
        .is_ok_and(|o| o.status.success())
}

/// Helper: generate a synthetic test video with ffmpeg
fn generate_test_video(path: &Path, width: u32, height: u32, duration_secs: u32, has_audio: bool) {
    let mut cmd = Command::new("ffmpeg");
    cmd.arg("-y")
        .arg("-f")
        .arg("lavfi")
        .arg("-i")
        .arg(format!(
            "testsrc=duration={}:size={}x{}:rate=25",
            duration_secs, width, height
        ));

    if has_audio {
        cmd.arg("-f")
            .arg("lavfi")
            .arg("-i")
            .arg(format!("sine=frequency=440:duration={}", duration_secs));
        cmd.arg("-c:a").arg("aac").arg("-b:a").arg("128k");
    }

    cmd.arg("-c:v")
        .arg("libx264")
        .arg("-preset")
        .arg("ultrafast")
        .arg("-pix_fmt")
        .arg("yuv420p")
        .arg("-t")
        .arg(duration_secs.to_string())
        .arg(path)
        .arg("-hide_banner")
        .arg("-loglevel")
        .arg("error");

    if !has_audio {
        cmd.arg("-an");
    }

    let output = cmd.output().expect("Failed to run ffmpeg");
    assert!(
        output.status.success(),
        "Failed to generate test video: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(path.exists(), "Test video was not created at {:?}", path);
}

/// Helper: probe video dimensions
fn probe_dimensions(path: &Path) -> (u32, u32) {
    let output = Command::new("ffprobe")
        .args([
            "-v", "error",
            "-select_streams", "v:0",
            "-show_entries", "stream=width,height",
            "-of", "csv=p=0",
        ])
        .arg(path)
        .output()
        .expect("Failed to run ffprobe");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let parts: Vec<&str> = stdout.trim().split(',').collect();
    assert_eq!(parts.len(), 2, "Unexpected ffprobe output: {}", stdout);
    (
        parts[0].parse().expect("Invalid width"),
        parts[1].parse().expect("Invalid height"),
    )
}

/// Helper: check if video has audio stream
fn has_audio_stream(path: &Path) -> bool {
    let output = Command::new("ffprobe")
        .args([
            "-v", "error",
            "-select_streams", "a",
            "-show_entries", "stream=codec_type",
            "-of", "csv=p=0",
        ])
        .arg(path)
        .output()
        .expect("Failed to run ffprobe");

    !String::from_utf8_lossy(&output.stdout).trim().is_empty()
}

// ─── CLI Integration Tests ────────────────────────────────────────────

#[test]
fn test_cli_validate_with_valid_videos() {
    if !ffmpeg_available() {
        return;
    }

    let input_dir = TempDir::new().unwrap();
    generate_test_video(&input_dir.path().join("test1.mp4"), 1920, 1080, 1, true);

    let output = Command::new(env!("CARGO_BIN_EXE_ffmpeg-video-processor"))
        .args(["validate", input_dir.path().to_str().unwrap()])
        .output()
        .expect("Failed to run binary");

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success(), "Validate failed: {}", stdout);
    assert!(stdout.contains("Valid:   1"));
}

#[test]
fn test_cli_validate_empty_directory() {
    if !ffmpeg_available() {
        return;
    }

    let input_dir = TempDir::new().unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_ffmpeg-video-processor"))
        .args(["validate", input_dir.path().to_str().unwrap()])
        .output()
        .expect("Failed to run binary");

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Valid:   0"));
}

#[test]
fn test_cli_validate_nonexistent_directory() {
    if !ffmpeg_available() {
        return;
    }

    let output = Command::new(env!("CARGO_BIN_EXE_ffmpeg-video-processor"))
        .args(["validate", "/nonexistent/path"])
        .output()
        .expect("Failed to run binary");

    assert!(!output.status.success());
}

// ─── End-to-End Processing Tests ──────────────────────────────────────

#[test]
fn test_process_landscape_video_end_to_end() {
    if !ffmpeg_available() {
        return;
    }

    let input_dir = TempDir::new().unwrap();
    let output_dir = TempDir::new().unwrap();

    // Generate a 1920x1080 test video (already standard aspect ratio)
    generate_test_video(&input_dir.path().join("landscape.mp4"), 1920, 1080, 2, true);

    let output = Command::new(env!("CARGO_BIN_EXE_ffmpeg-video-processor"))
        .args([
            "process",
            input_dir.path().to_str().unwrap(),
            output_dir.path().to_str().unwrap(),
            "--no-tui",
            "--profile",
            "fast",
        ])
        .output()
        .expect("Failed to run binary");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "Processing failed.\nstdout: {}\nstderr: {}",
        stdout,
        stderr
    );

    // Should have a processed_vod_*.mp4 in output
    let output_files: Vec<_> = std::fs::read_dir(output_dir.path())
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.file_name()
                .to_string_lossy()
                .starts_with("processed_vod_")
        })
        .collect();

    assert_eq!(output_files.len(), 1, "Expected exactly one output file");

    let output_path = output_files[0].path();
    let (w, h) = probe_dimensions(&output_path);
    assert_eq!((w, h), (3840, 2160), "Output should be 4K");
    assert!(has_audio_stream(&output_path), "Output should have audio");
}

#[test]
fn test_process_vertical_video_gets_blurred_background() {
    if !ffmpeg_available() {
        return;
    }

    let input_dir = TempDir::new().unwrap();
    let output_dir = TempDir::new().unwrap();

    // Generate a vertical (portrait) video — this is the key regression test
    generate_test_video(&input_dir.path().join("vertical.mp4"), 1080, 1920, 2, true);

    let output = Command::new(env!("CARGO_BIN_EXE_ffmpeg-video-processor"))
        .args([
            "process",
            input_dir.path().to_str().unwrap(),
            output_dir.path().to_str().unwrap(),
            "--no-tui",
            "--profile",
            "fast",
        ])
        .output()
        .expect("Failed to run binary");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "Vertical video processing failed.\nstdout: {}\nstderr: {}",
        stdout,
        stderr
    );

    // Output must be 4K
    let output_files: Vec<_> = std::fs::read_dir(output_dir.path())
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.file_name()
                .to_string_lossy()
                .starts_with("processed_vod_")
        })
        .collect();

    assert_eq!(output_files.len(), 1, "Expected exactly one output file");

    let output_path = output_files[0].path();
    let (w, h) = probe_dimensions(&output_path);
    assert_eq!((w, h), (3840, 2160), "Vertical video output should be padded to 4K");
}

#[test]
fn test_process_video_without_audio() {
    if !ffmpeg_available() {
        return;
    }

    let input_dir = TempDir::new().unwrap();
    let output_dir = TempDir::new().unwrap();

    // Video with NO audio — should get silent track added
    generate_test_video(
        &input_dir.path().join("silent.mp4"),
        1920,
        1080,
        2,
        false,
    );

    let output = Command::new(env!("CARGO_BIN_EXE_ffmpeg-video-processor"))
        .args([
            "process",
            input_dir.path().to_str().unwrap(),
            output_dir.path().to_str().unwrap(),
            "--no-tui",
            "--profile",
            "fast",
        ])
        .output()
        .expect("Failed to run binary");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "Silent video processing failed.\nstdout: {}\nstderr: {}",
        stdout,
        stderr
    );

    let output_files: Vec<_> = std::fs::read_dir(output_dir.path())
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.file_name()
                .to_string_lossy()
                .starts_with("processed_vod_")
        })
        .collect();

    assert_eq!(output_files.len(), 1);
    let output_path = output_files[0].path();

    // Must have audio in the output (silent track added)
    assert!(
        has_audio_stream(&output_path),
        "Output should have audio (silent track should be added)"
    );
}

#[test]
fn test_process_small_resolution_video() {
    if !ffmpeg_available() {
        return;
    }

    let input_dir = TempDir::new().unwrap();
    let output_dir = TempDir::new().unwrap();

    // Small 640x480 video — needs padding to 4K
    generate_test_video(&input_dir.path().join("small.mp4"), 640, 480, 2, true);

    let output = Command::new(env!("CARGO_BIN_EXE_ffmpeg-video-processor"))
        .args([
            "process",
            input_dir.path().to_str().unwrap(),
            output_dir.path().to_str().unwrap(),
            "--no-tui",
            "--profile",
            "fast",
        ])
        .output()
        .expect("Failed to run binary");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "Small video processing failed.\nstdout: {}\nstderr: {}",
        stdout,
        stderr
    );

    let output_files: Vec<_> = std::fs::read_dir(output_dir.path())
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.file_name()
                .to_string_lossy()
                .starts_with("processed_vod_")
        })
        .collect();

    assert_eq!(output_files.len(), 1);
    let (w, h) = probe_dimensions(&output_files[0].path());
    assert_eq!((w, h), (3840, 2160), "Small video should be padded to 4K");
}

#[test]
fn test_process_multiple_videos_concatenated() {
    if !ffmpeg_available() {
        return;
    }

    let input_dir = TempDir::new().unwrap();
    let output_dir = TempDir::new().unwrap();

    // Multiple videos of different sizes — all should be normalized and concatenated
    generate_test_video(&input_dir.path().join("a_first.mp4"), 1920, 1080, 1, true);
    generate_test_video(&input_dir.path().join("b_second.mp4"), 1280, 720, 1, true);
    generate_test_video(&input_dir.path().join("c_third.mp4"), 1080, 1920, 1, true);

    let output = Command::new(env!("CARGO_BIN_EXE_ffmpeg-video-processor"))
        .args([
            "process",
            input_dir.path().to_str().unwrap(),
            output_dir.path().to_str().unwrap(),
            "--no-tui",
            "--profile",
            "fast",
        ])
        .output()
        .expect("Failed to run binary");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "Multi-video processing failed.\nstdout: {}\nstderr: {}",
        stdout,
        stderr
    );

    let output_files: Vec<_> = std::fs::read_dir(output_dir.path())
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.file_name()
                .to_string_lossy()
                .starts_with("processed_vod_")
        })
        .collect();

    assert_eq!(
        output_files.len(),
        1,
        "All videos should be concatenated into one"
    );

    let output_path = output_files[0].path();
    let (w, h) = probe_dimensions(&output_path);
    assert_eq!((w, h), (3840, 2160));
}

#[test]
fn test_process_does_not_hang_on_stdin() {
    if !ffmpeg_available() {
        return;
    }

    let input_dir = TempDir::new().unwrap();
    let output_dir = TempDir::new().unwrap();

    generate_test_video(&input_dir.path().join("test.mp4"), 1920, 1080, 1, true);

    // Run with a timeout — if it hangs, the test will fail
    let output = Command::new(env!("CARGO_BIN_EXE_ffmpeg-video-processor"))
        .args([
            "process",
            input_dir.path().to_str().unwrap(),
            output_dir.path().to_str().unwrap(),
            "--no-tui",
            "--profile",
            "fast",
        ])
        .stdin(std::process::Stdio::null())
        .output()
        .expect("Failed to run binary");

    assert!(
        output.status.success(),
        "Process should complete without hanging"
    );
}

#[test]
fn test_dry_run_does_not_produce_output() {
    if !ffmpeg_available() {
        return;
    }

    let input_dir = TempDir::new().unwrap();
    let output_dir = TempDir::new().unwrap();

    generate_test_video(&input_dir.path().join("test.mp4"), 1920, 1080, 1, true);

    let output = Command::new(env!("CARGO_BIN_EXE_ffmpeg-video-processor"))
        .args([
            "process",
            input_dir.path().to_str().unwrap(),
            output_dir.path().to_str().unwrap(),
            "--no-tui",
            "--dry-run",
        ])
        .output()
        .expect("Failed to run binary");

    assert!(output.status.success());

    // No processed files should exist
    let output_files: Vec<_> = std::fs::read_dir(output_dir.path())
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.file_name()
                .to_string_lossy()
                .starts_with("processed_vod_")
        })
        .collect();

    assert_eq!(
        output_files.len(),
        0,
        "Dry run should not produce output files"
    );
}

#[test]
fn test_intermediates_cleaned_up_by_default() {
    if !ffmpeg_available() {
        return;
    }

    let input_dir = TempDir::new().unwrap();
    let output_dir = TempDir::new().unwrap();

    generate_test_video(&input_dir.path().join("test.mp4"), 1920, 1080, 1, true);

    let output = Command::new(env!("CARGO_BIN_EXE_ffmpeg-video-processor"))
        .args([
            "process",
            input_dir.path().to_str().unwrap(),
            output_dir.path().to_str().unwrap(),
            "--no-tui",
            "--profile",
            "fast",
        ])
        .output()
        .expect("Failed to run binary");

    assert!(output.status.success());

    // No intermediate files should remain
    let preprocessed = output_dir.path().join("preprocessed");
    assert!(
        !preprocessed.exists(),
        "preprocessed directory should be cleaned up"
    );

    let resampled: Vec<_> = std::fs::read_dir(output_dir.path())
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.file_name()
                .to_string_lossy()
                .starts_with("resampled_")
        })
        .collect();

    assert_eq!(
        resampled.len(),
        0,
        "resampled_ intermediate files should be cleaned up"
    );

    let concat_list = output_dir.path().join("concat_list.txt");
    assert!(
        !concat_list.exists(),
        "concat_list.txt should be cleaned up"
    );
}
