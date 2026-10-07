//! Command-line interface.

use clap::{Args, Parser, Subcommand, ValueEnum};
use ffmpeg_video_processor::discover::{self, ScanKind, ScanOptions, SortOrder};
use ffmpeg_video_processor::encoders::working_video_encoders;
use ffmpeg_video_processor::preview;
use ffmpeg_video_processor::project::{
    AudioOptions, Clip, FrameRate, Intro, LoudnessTarget, Output, Preset, Project, Quality,
    Transition, TransitionKind, Trim, Watermark,
};
use ffmpeg_video_processor::render::{RenderOptions, render_probed};
use ffmpeg_video_processor::titles::TitleTemplate;
use ffmpeg_video_processor::{
    CancelToken, Error, Event, FfmpegTools, MediaInfo, Result, ToolPaths,
};
use std::collections::BTreeMap;
use std::io::IsTerminal;
use std::path::{Path, PathBuf};

#[derive(Parser, Debug)]
#[command(
    name = "ffmpeg-video-processor",
    version,
    about = "Join, trim and normalise home videos into one polished movie",
    long_about = "Join any videos FFmpeg can read into one MP4. Clips are ordered by recording time, \
                  resized with a blurred fill instead of black bars, given silence when they have no \
                  sound, joined with transitions, optionally preceded by an animated title, and the \
                  soundtrack is normalised to EBU R128 in two passes."
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,

    /// Path to the ffmpeg executable (default: next to --ffprobe, $FFMPEG_PATH, or PATH).
    #[arg(long, global = true, value_name = "PATH")]
    pub ffmpeg: Option<PathBuf>,

    /// Path to the ffprobe executable.
    #[arg(long, global = true, value_name = "PATH")]
    pub ffprobe: Option<PathBuf>,

    /// Log level: error, warn, info, debug or trace.
    #[arg(short, long, global = true, value_name = "LEVEL")]
    pub log_level: Option<String>,

    /// Shorthand for --log-level debug.
    #[arg(short, long, global = true)]
    pub verbose: bool,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Join videos (files and/or folders) into one movie.
    Render(Box<RenderArgs>),
    /// Join every video in a folder into a timestamped 4K movie (0.2 compatible).
    Process(ProcessArgs),
    /// List which files would be used, and why others are left out.
    Scan(ScanArgs),
    /// Show what FFmpeg reports about files.
    Probe {
        #[arg(required = true)]
        files: Vec<PathBuf>,
        /// Print JSON.
        #[arg(long)]
        json: bool,
    },
    /// Check that every video in a folder can be read (0.2 compatible).
    Validate { input: PathBuf },
    /// Show the H.264 encoders that work on this machine.
    Encoders {
        /// Canvas size to test, e.g. 1920x1080.
        #[arg(long, default_value = "1920x1080")]
        size: String,
    },
    /// List the built-in title templates and their fields.
    Templates,
    /// Save one frame as a JPEG.
    Thumbnail {
        file: PathBuf,
        #[arg(short, long)]
        output: PathBuf,
        /// Seconds into the video.
        #[arg(long, default_value_t = 1.0)]
        at: f64,
        #[arg(long, default_value_t = 320)]
        width: u32,
    },
    /// Save evenly spaced frames as JPEGs.
    Filmstrip {
        file: PathBuf,
        /// Output folder.
        #[arg(short, long)]
        output: PathBuf,
        #[arg(long, default_value_t = 24)]
        count: u32,
        #[arg(long, default_value_t = 90)]
        height: u32,
    },
    /// Make a small preview copy that scrubs smoothly.
    Proxy {
        file: PathBuf,
        #[arg(short, long)]
        output: PathBuf,
        #[arg(long, default_value_t = 540)]
        height: u32,
    },
}

#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum SortArg {
    Recorded,
    Name,
    AsGiven,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum QualityArg {
    Draft,
    Standard,
    High,
}

#[derive(Args, Debug)]
pub struct RenderArgs {
    /// Video files and/or folders.
    pub inputs: Vec<PathBuf>,

    /// Output file (.mp4).
    #[arg(short, long)]
    pub output: Option<PathBuf>,

    /// Render a project file (JSON) instead of inputs and options.
    #[arg(long, value_name = "FILE", conflicts_with = "inputs")]
    pub project: Option<PathBuf>,

    /// Write the project that would be rendered to FILE (and still render).
    #[arg(long, value_name = "FILE")]
    pub save_project: Option<PathBuf>,

    /// Canvas: 4k, 1080p, 720p, vertical or WIDTHxHEIGHT.
    #[arg(long, default_value = "1080p")]
    pub preset: String,

    /// Frame rate: auto, 24, 25, 30, 50 or 60.
    #[arg(long, default_value = "auto")]
    pub fps: String,

    #[arg(long, value_enum, default_value_t = QualityArg::Standard)]
    pub quality: QualityArg,

    /// cut, crossfade, fade_black, fade_white, slide, wipe, zoom, blur_dissolve, tape_rewind or mix.
    #[arg(long, default_value = "crossfade")]
    pub transition: String,

    /// Transition length in seconds.
    #[arg(long, default_value_t = 1.0)]
    pub transition_duration: f64,

    /// Title for an animated intro.
    #[arg(long)]
    pub title: Option<String>,
    #[arg(long)]
    pub subtitle: Option<String>,
    /// Date line for the intro, e.g. "July 2026".
    #[arg(long)]
    pub date: Option<String>,
    /// Built-in template name or template folder.
    #[arg(long, default_value = "clean")]
    pub template: String,
    /// Render only the intro (a title preview).
    #[arg(long)]
    pub intro_only: bool,

    #[arg(long, value_enum, default_value_t = SortArg::Recorded)]
    pub sort: SortArg,
    /// Do not look inside sub-folders.
    #[arg(long)]
    pub no_recursive: bool,
    /// Include files whose names start with a dot.
    #[arg(long)]
    pub include_hidden: bool,
    /// Trim a clip: NAME=START-END in seconds (END optional), NAME = file name or path.
    #[arg(long, value_name = "NAME=START-END")]
    pub trim: Vec<String>,

    /// Do not bring clips to a similar loudness before mixing.
    #[arg(long)]
    pub no_level_clips: bool,
    /// Skip EBU R128 loudness normalisation.
    #[arg(long)]
    pub no_loudness: bool,
    /// Integrated loudness target in LUFS.
    #[arg(long, default_value_t = -23.0, allow_negative_numbers = true)]
    pub loudness: f64,
    /// True-peak ceiling in dBTP.
    #[arg(long, default_value_t = -1.0, allow_negative_numbers = true)]
    pub true_peak: f64,

    /// PNG burned into the lower-right corner.
    #[arg(long)]
    pub watermark: Option<PathBuf>,
    /// Force an FFmpeg video encoder.
    #[arg(long)]
    pub encoder: Option<String>,
    /// Replace the output file if it exists.
    #[arg(long)]
    pub overwrite: bool,

    #[command(flatten)]
    pub run: RunArgs,
}

/// Options shared by commands that render.
#[derive(Args, Debug)]
pub struct RunArgs {
    /// Folder for temporary files (default: system temp).
    #[arg(long)]
    pub work_dir: Option<PathBuf>,
    /// Keep temporary files.
    #[arg(long)]
    pub keep_work: bool,
    /// Decode in software only.
    #[arg(long)]
    pub no_hw_decode: bool,
    /// Plain progress output instead of the full-screen view.
    #[arg(long)]
    pub no_tui: bool,
    /// Print events as JSON lines (for scripts and other programs).
    #[arg(long)]
    pub json: bool,
    /// Show what would be rendered without rendering.
    #[arg(long)]
    pub dry_run: bool,
}

#[derive(Args, Debug)]
pub struct ProcessArgs {
    pub input: PathBuf,
    pub output: PathBuf,
    #[arg(short, long, value_enum, default_value_t = Profile::Balanced)]
    pub profile: Profile,
    /// Keep temporary files.
    #[arg(long)]
    pub keep_intermediates: bool,
    #[arg(long)]
    pub no_tui: bool,
    #[arg(long)]
    pub dry_run: bool,
    #[arg(long)]
    pub json: bool,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum Profile {
    Fast,
    Balanced,
    Quality,
}

#[derive(Args, Debug)]
pub struct ScanArgs {
    #[arg(required = true)]
    pub inputs: Vec<PathBuf>,
    #[arg(long, value_enum, default_value_t = SortArg::Recorded)]
    pub sort: SortArg,
    #[arg(long)]
    pub no_recursive: bool,
    #[arg(long)]
    pub include_hidden: bool,
    #[arg(long)]
    pub json: bool,
}

fn tools(cli: &Cli) -> Result<FfmpegTools> {
    FfmpegTools::locate(&ToolPaths {
        ffmpeg: cli.ffmpeg.clone(),
        ffprobe: cli.ffprobe.clone(),
    })
}

fn sort_order(sort: SortArg) -> SortOrder {
    match sort {
        SortArg::Recorded => SortOrder::Recorded,
        SortArg::Name => SortOrder::Name,
        SortArg::AsGiven => SortOrder::AsGiven,
    }
}

fn invalid(message: impl Into<String>) -> Error {
    Error::InvalidProject(message.into())
}

/// Scan inputs and return the usable videos (sorted) plus everything left out.
fn gather(
    tools: &FfmpegTools,
    inputs: &[PathBuf],
    opts: &ScanOptions,
    sort: SortOrder,
    cancel: &CancelToken,
) -> Result<(Vec<MediaInfo>, Vec<discover::ScanItem>)> {
    let items = discover::scan(tools, inputs, opts, cancel, &|_| {})?;
    let (videos, skipped): (Vec<_>, Vec<_>) =
        items.into_iter().partition(|i| i.kind == ScanKind::Video);
    let mut media: Vec<MediaInfo> = videos.into_iter().filter_map(|i| i.media).collect();
    discover::sort_media(&mut media, sort);
    Ok((media, skipped))
}

fn parse_trims(specs: &[String]) -> Result<Vec<(String, Trim)>> {
    specs
        .iter()
        .map(|spec| {
            let (name, range) = spec
                .rsplit_once('=')
                .ok_or_else(|| invalid(format!("--trim {spec:?} must look like NAME=START-END")))?;
            let (start, end) = range.split_once('-').unwrap_or((range, ""));
            let start: f64 = if start.trim().is_empty() {
                0.0
            } else {
                start
                    .trim()
                    .parse()
                    .map_err(|_| invalid(format!("bad trim start in {spec:?}")))?
            };
            let end = if end.trim().is_empty() {
                None
            } else {
                Some(
                    end.trim()
                        .parse::<f64>()
                        .map_err(|_| invalid(format!("bad trim end in {spec:?}")))?,
                )
            };
            Ok((name.to_string(), Trim { start, end }))
        })
        .collect()
}

fn build_project(args: &RenderArgs, media: &[MediaInfo]) -> Result<Project> {
    let output = args
        .output
        .clone()
        .ok_or_else(|| invalid("--output is required"))?;
    let (preset, size) = Preset::parse(&args.preset)
        .ok_or_else(|| invalid(format!("unknown preset {:?}", args.preset)))?;
    let fps = match args.fps.as_str() {
        "auto" => FrameRate::Auto,
        n => FrameRate::Fixed(n.parse().map_err(|_| invalid(format!("bad --fps {n:?}")))?),
    };
    let kind = TransitionKind::ALL
        .into_iter()
        .find(|k| k.name() == args.transition.replace('-', "_"))
        .ok_or_else(|| invalid(format!("unknown transition {:?}", args.transition)))?;
    let trims = parse_trims(&args.trim)?;
    let clips = media
        .iter()
        .map(|m| {
            let name = m
                .path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            let trim = trims
                .iter()
                .find(|(n, _)| *n == name || Path::new(n) == m.path)
                .map(|(_, t)| *t);
            Clip {
                path: m.path.clone(),
                trim,
                rotation: 0,
            }
        })
        .collect();
    let mut fields = BTreeMap::new();
    for (key, value) in [
        ("title", &args.title),
        ("subtitle", &args.subtitle),
        ("date", &args.date),
    ] {
        if let Some(v) = value.as_ref().filter(|v| !v.trim().is_empty()) {
            fields.insert(key.to_string(), v.clone());
        }
    }
    let intro = (fields.contains_key("title") || args.intro_only).then(|| Intro {
        template: args.template.clone(),
        fields,
    });
    let project = Project {
        clips,
        intro,
        transition: Transition {
            kind,
            duration: args.transition_duration,
        },
        output: Output {
            preset,
            width: size.map(|s| s.0),
            height: size.map(|s| s.1),
            fps,
            quality: match args.quality {
                QualityArg::Draft => Quality::Draft,
                QualityArg::Standard => Quality::Standard,
                QualityArg::High => Quality::High,
            },
            encoder: args.encoder.clone(),
            overwrite: args.overwrite,
            title: args.title.clone(),
            ..Output::new(output)
        },
        audio: AudioOptions {
            level_clips: !args.no_level_clips,
            loudness: (!args.no_loudness).then_some(LoudnessTarget {
                integrated: args.loudness,
                true_peak: args.true_peak,
            }),
        },
        watermark: args.watermark.clone().map(|image| Watermark {
            image,
            anchor: Default::default(),
            width_fraction: 0.22,
            margin_fraction: 0.03,
            opacity: 0.85,
        }),
    };
    project.validate()?;
    Ok(project)
}

fn print_left_out(skipped: &[discover::ScanItem]) {
    if skipped.is_empty() {
        return;
    }
    eprintln!("Left out {} file(s):", skipped.len());
    for item in skipped {
        eprintln!(
            "  {}  ({})",
            item.path.display(),
            item.reason.as_deref().unwrap_or("not a video")
        );
    }
}

pub fn run(cli: Cli) -> Result<()> {
    match &cli.command {
        Command::Render(args) => cmd_render(&cli, args),
        Command::Process(args) => cmd_process(&cli, args),
        Command::Scan(args) => cmd_scan(&cli, args),
        Command::Probe { files, json } => cmd_probe(&cli, files, *json),
        Command::Validate { input } => cmd_validate(&cli, input),
        Command::Encoders { size } => {
            let (_, size) = Preset::parse(size).ok_or_else(|| invalid("bad --size"))?;
            let (w, h) = size.unwrap_or((1920, 1080));
            let tools = tools(&cli)?;
            println!("{}", tools.version()?);
            for e in working_video_encoders(&tools, w, h, &CancelToken::new())? {
                println!(
                    "  {:<20} {}",
                    e.id,
                    if e.hardware { "hardware" } else { "software" }
                );
            }
            println!(
                "audio: {}",
                ffmpeg_video_processor::encoders::select_audio_encoder(
                    &tools,
                    &CancelToken::new()
                )?
            );
            Ok(())
        }
        Command::Templates => {
            for name in TitleTemplate::builtin_names() {
                let t = TitleTemplate::builtin(name)?;
                let fields: Vec<String> = t
                    .fields()
                    .iter()
                    .map(|f| format!("{}{}", f.name, if f.required { "*" } else { "" }))
                    .collect();
                println!(
                    "{name:<12} {:.1}s  fields: {}",
                    t.duration(),
                    fields.join(", ")
                );
            }
            Ok(())
        }
        Command::Thumbnail {
            file,
            output,
            at,
            width,
        } => {
            let tools = tools(&cli)?;
            let cancel = CancelToken::new();
            let media = ffmpeg_video_processor::probe(&tools, file, &cancel)?;
            let path = preview::thumbnail(&tools, &media, *at, *width, output, &cancel)?;
            println!("{}", path.display());
            Ok(())
        }
        Command::Filmstrip {
            file,
            output,
            count,
            height,
        } => {
            let tools = tools(&cli)?;
            let cancel = CancelToken::new();
            let media = ffmpeg_video_processor::probe(&tools, file, &cancel)?;
            let stem = file
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| "frame".into());
            for path in preview::filmstrip(&tools, &media, *count, *height, output, &stem, &cancel)?
            {
                println!("{}", path.display());
            }
            Ok(())
        }
        Command::Proxy {
            file,
            output,
            height,
        } => {
            let tools = tools(&cli)?;
            let cancel = CancelToken::new();
            let media = ffmpeg_video_processor::probe(&tools, file, &cancel)?;
            let path = preview::proxy(&tools, &media, *height, output, &mut |_| {}, &cancel)?;
            println!("{}", path.display());
            Ok(())
        }
    }
}

fn cmd_scan(cli: &Cli, args: &ScanArgs) -> Result<()> {
    let tools = tools(cli)?;
    let opts = ScanOptions {
        recursive: !args.no_recursive,
        include_hidden: args.include_hidden,
    };
    let (media, skipped) = gather(
        &tools,
        &args.inputs,
        &opts,
        sort_order(args.sort),
        &CancelToken::new(),
    )?;
    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(
                &serde_json::json!({ "videos": media, "left_out": skipped })
            )?
        );
        return Ok(());
    }
    for m in &media {
        let v = m.video.as_ref().expect("videos have video info");
        println!(
            "{:>8.2}s  {:>4}×{:<4} {:>6.2} fps  {}{}  {}",
            m.duration,
            v.display_width,
            v.display_height,
            v.frame_rate,
            if m.audio.is_some() { "♪" } else { "-" },
            if v.interlaced { " interlaced" } else { "" },
            m.path.display()
        );
    }
    println!(
        "{} video(s), {:.1} s in total",
        media.len(),
        media.iter().map(|m| m.duration).sum::<f64>()
    );
    print_left_out(&skipped);
    Ok(())
}

fn cmd_probe(cli: &Cli, files: &[PathBuf], json: bool) -> Result<()> {
    let tools = tools(cli)?;
    let cancel = CancelToken::new();
    let mut infos = Vec::new();
    for file in files {
        infos.push(ffmpeg_video_processor::probe(&tools, file, &cancel)?);
    }
    if json {
        println!("{}", serde_json::to_string_pretty(&infos)?);
    } else {
        for info in &infos {
            println!("{}", info.path.display());
            println!(
                "  kind: {:?}, format: {}, duration: {:.3}s",
                info.kind, info.format, info.duration
            );
            if let Some(v) = &info.video {
                println!(
                    "  video: {} {}×{} (shown {}×{}), {:.3} fps, rotation {}°, {}{}",
                    v.codec,
                    v.width,
                    v.height,
                    v.display_width,
                    v.display_height,
                    v.frame_rate,
                    v.rotation,
                    v.pix_fmt,
                    match (v.interlaced, v.hdr) {
                        (true, _) => ", interlaced",
                        (_, Some(_)) => ", HDR",
                        _ => "",
                    }
                );
            }
            if let Some(a) = &info.audio {
                println!(
                    "  audio: {} {} ch, {} Hz ({} track(s))",
                    a.codec, a.channels, a.sample_rate, info.audio_streams
                );
            }
            if let Some(date) = info.recorded_at {
                println!(
                    "  recorded: {date} (from {:?})",
                    info.recorded_at_source.expect("source")
                );
            }
        }
    }
    Ok(())
}

fn cmd_validate(cli: &Cli, input: &Path) -> Result<()> {
    let tools = tools(cli)?;
    let opts = ScanOptions {
        recursive: false,
        include_hidden: false,
    };
    let items = discover::scan(
        &tools,
        &[input.to_path_buf()],
        &opts,
        &CancelToken::new(),
        &|_| {},
    )?;
    let mut valid = 0;
    let mut invalid_count = 0;
    for item in &items {
        let name = item
            .path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        match (&item.kind, &item.media) {
            (ScanKind::Video, Some(m)) => {
                let v = m.video.as_ref().expect("video");
                println!(
                    "✓ {name} - {}x{} @ {:.2}fps",
                    v.display_width, v.display_height, v.frame_rate
                );
                valid += 1;
            }
            (ScanKind::Unreadable, _) => {
                println!(
                    "✗ {name} - Error: {}",
                    item.reason.as_deref().unwrap_or("unreadable")
                );
                invalid_count += 1;
            }
            _ => {}
        }
    }
    println!("\nValidation Summary:\n  Valid:   {valid}\n  Invalid: {invalid_count}");
    if invalid_count > 0 {
        return Err(invalid(format!(
            "{invalid_count} invalid video file(s) found"
        )));
    }
    if valid == 0 {
        return Err(Error::NoClips);
    }
    Ok(())
}

fn cmd_process(cli: &Cli, args: &ProcessArgs) -> Result<()> {
    if !args.input.is_dir() {
        return Err(Error::Unreadable {
            path: args.input.clone(),
            reason: "not a folder".into(),
        });
    }
    let output = args.output.join(format!(
        "processed_vod_{}.mp4",
        chrono::Local::now().format("%Y%m%d_%H%M%S")
    ));
    let render = RenderArgs {
        inputs: vec![args.input.clone()],
        output: Some(output),
        project: None,
        save_project: None,
        preset: "4k".into(),
        fps: "auto".into(),
        quality: match args.profile {
            Profile::Fast => QualityArg::Draft,
            Profile::Balanced => QualityArg::Standard,
            Profile::Quality => QualityArg::High,
        },
        transition: "cut".into(),
        transition_duration: 0.0,
        title: None,
        subtitle: None,
        date: None,
        template: "clean".into(),
        intro_only: false,
        sort: SortArg::Name,
        no_recursive: true,
        include_hidden: false,
        trim: Vec::new(),
        no_level_clips: false,
        no_loudness: false,
        loudness: -23.0,
        true_peak: -1.0,
        watermark: None,
        encoder: None,
        overwrite: false,
        run: RunArgs {
            work_dir: None,
            keep_work: args.keep_intermediates,
            no_hw_decode: false,
            no_tui: args.no_tui,
            json: args.json,
            dry_run: args.dry_run,
        },
    };
    cmd_render(cli, &render)
}

fn cmd_render(cli: &Cli, args: &RenderArgs) -> Result<()> {
    let tools = tools(cli)?;
    let cancel = CancelToken::new();
    let (project, media) = match &args.project {
        Some(file) => {
            let json = std::fs::read_to_string(file).map_err(|e| Error::Unreadable {
                path: file.clone(),
                reason: e.to_string(),
            })?;
            let project = Project::from_json(&json)?;
            let media = ffmpeg_video_processor::render::probe_project(&tools, &project, &cancel)?;
            (project, media)
        }
        None => {
            if args.inputs.is_empty() {
                return Err(invalid("give video files or folders, or --project"));
            }
            let opts = ScanOptions {
                recursive: !args.no_recursive,
                include_hidden: args.include_hidden,
            };
            let (media, skipped) =
                gather(&tools, &args.inputs, &opts, sort_order(args.sort), &cancel)?;
            if !args.run.json {
                print_left_out(&skipped);
            }
            if media.is_empty() {
                return Err(Error::NoClips);
            }
            (build_project(args, &media)?, media)
        }
    };
    if let Some(file) = &args.save_project {
        std::fs::write(file, project.to_json())
            .map_err(|e| Error::io(format!("writing {}", file.display()), e))?;
    }
    if args.run.dry_run {
        let template = project
            .intro
            .as_ref()
            .map(|i| ffmpeg_video_processor::render::load_template(&i.template))
            .transpose()?;
        let plan = ffmpeg_video_processor::plan(
            &project,
            &media,
            template.as_ref().map(|t| t.duration()),
        )?;
        if args.run.json {
            println!("{}", serde_json::to_string(&plan)?);
        } else {
            println!(
                "Would render {} clip(s) to {} at {}×{} {} fps, {:.1} s",
                project.clips.len(),
                project.output.path.display(),
                plan.width,
                plan.height,
                plan.fps,
                plan.duration_seconds()
            );
        }
        return Ok(());
    }
    let options = RenderOptions {
        work_dir: args.run.work_dir.clone(),
        keep_work_dir: args.run.keep_work,
        no_hardware_decode: args.run.no_hw_decode,
        intro_only: args.intro_only,
    };
    let interactive = std::io::stdout().is_terminal() && !args.run.no_tui && !args.run.json;
    #[cfg(feature = "tui")]
    if interactive {
        return crate::tui::run(tools, project, media, options, cancel);
    }
    if args.run.json {
        let result = render_probed(
            &tools,
            &project,
            &media,
            &options,
            &mut |e: Event| {
                println!("{}", serde_json::to_string(&e).expect("event serialises"));
            },
            &cancel,
        );
        return match result {
            Ok(outcome) => {
                println!(
                    "{}",
                    serde_json::json!({ "type": "outcome", "outcome": outcome })
                );
                Ok(())
            }
            Err(e) => {
                println!(
                    "{}",
                    serde_json::json!({ "type": "error", "code": e.code(), "message": e.to_string() })
                );
                Err(e)
            }
        };
    }
    plain_progress(&tools, &project, &media, &options, &cancel, interactive)
}

fn plain_progress(
    tools: &FfmpegTools,
    project: &Project,
    media: &[MediaInfo],
    options: &RenderOptions,
    cancel: &CancelToken,
    _interactive: bool,
) -> Result<()> {
    use indicatif::{ProgressBar, ProgressStyle};
    let bar = ProgressBar::new(1000);
    bar.set_style(
        ProgressStyle::with_template("{spinner} [{elapsed_precise}] {bar:40} {percent:>3}% {msg}")
            .expect("valid template"),
    );
    let outcome = render_probed(
        tools,
        project,
        media,
        options,
        &mut |event| match event {
            Event::Stage { stage } => bar.set_message(format!("{stage:?}").to_lowercase()),
            Event::Progress {
                fraction,
                stage,
                eta_seconds,
                fps,
                ..
            } => {
                bar.set_position((fraction * 1000.0) as u64);
                let mut msg = format!("{stage:?}").to_lowercase();
                if let Some(fps) = fps {
                    msg.push_str(&format!("  {fps:.0} fps"));
                }
                if let Some(eta) = eta_seconds {
                    msg.push_str(&format!("  ~{}s left", eta.round()));
                }
                bar.set_message(msg);
            }
            Event::Warning { message, .. } => bar.println(format!("warning: {message}")),
            Event::Done { .. } => {}
        },
        cancel,
    )?;
    bar.finish_and_clear();
    println!(
        "✓ {}  ({}×{} {} fps, {:.1} s, {:.1} MB, {})",
        outcome.output.display(),
        outcome.width,
        outcome.height,
        outcome.fps,
        outcome.duration_seconds,
        outcome.size_bytes as f64 / 1_048_576.0,
        outcome.video_encoder
    );
    if let Some(l) = &outcome.loudness {
        println!(
            "  loudness {:.1} LUFS → {} LUFS ({})",
            l.input_integrated,
            l.output_integrated
                .map_or("?".into(), |v| format!("{v:.1}")),
            l.normalization.as_deref().unwrap_or("?")
        );
    }
    Ok(())
}
