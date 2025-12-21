use crate::config::AppConfig;
use crate::error::{Result, VideoProcessorError};
use clap::{Parser, Subcommand, ValueEnum};
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(
    name = "ffmpeg-video-processor",
    version,
    about = "Production-ready FFmpeg video processing tool with interactive TUI and CLI",
    long_about = "Process and standardize home videos to 4K UHD (3840x2160) MP4 format optimized for TV playback. \
                  Features include comprehensive error handling, real-time progress tracking, interactive TUI with \
                  FFmpeg log viewer, and checkpoint/resume functionality. Default: 25fps PAL, CRF 18, 320kbps AAC audio."
)]
pub struct Cli {
    /// Command to execute
    #[command(subcommand)]
    pub command: Commands,

    /// Log level (trace, debug, info, warn, error)
    #[arg(short, long, global = true)]
    pub log_level: Option<String>,

    /// Verbose output
    #[arg(short, long, global = true)]
    pub verbose: bool,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Process videos from input directory to output directory
    Process(ProcessArgs),

    /// Resume processing from a checkpoint
    Resume {
        /// Path to checkpoint file
        #[arg(short, long)]
        checkpoint: PathBuf,
    },

    /// Validate video files without processing
    Validate {
        /// Input directory containing videos
        input: PathBuf,
    },

    /// Manage configuration
    #[command(subcommand)]
    Config(ConfigCommands),
}

#[derive(Parser, Debug)]
pub struct ProcessArgs {
    /// Input directory containing source videos
    pub input: PathBuf,

    /// Output directory for processed videos
    pub output: PathBuf,

    /// Processing profile: fast, balanced, or quality
    #[arg(short, long, value_enum, default_value = "balanced")]
    pub profile: Profile,

    /// Custom configuration file
    #[arg(short, long)]
    pub config: Option<PathBuf>,

    /// Disable TUI mode (use CLI mode with progress bars)
    #[arg(long)]
    pub no_tui: bool,

    /// Keep intermediate files for debugging
    #[arg(long)]
    pub keep_intermediates: bool,

    /// Dry run (validate inputs without processing)
    #[arg(long)]
    pub dry_run: bool,

    /// Number of parallel jobs (0 = auto)
    #[arg(short, long, default_value = "0")]
    pub jobs: usize,

    /// Disable checkpoint/resume functionality
    #[arg(long)]
    pub no_checkpoint: bool,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum Profile {
    /// Fast encoding (lower quality, faster)
    Fast,
    /// Balanced encoding (default)
    Balanced,
    /// High quality encoding (slower)
    Quality,
}

impl Profile {
    pub fn to_config(self) -> AppConfig {
        match self {
            Profile::Fast => AppConfig::with_fast_profile(),
            Profile::Balanced => AppConfig::with_balanced_profile(),
            Profile::Quality => AppConfig::with_quality_profile(),
        }
    }
}

#[derive(Subcommand, Debug)]
pub enum ConfigCommands {
    /// Show current configuration
    Show,
    
    /// Generate default configuration file
    Init {
        /// Output path for configuration file
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
    
    /// Show configuration file path
    Path,
}

/// Run processing in CLI mode (with progress bars)
pub async fn run_cli_mode(args: &ProcessArgs) -> Result<()> {
    use crate::orchestrator::Orchestrator;
    use indicatif::{MultiProgress, ProgressBar, ProgressStyle};

    println!("Running in CLI mode...");
    println!("Input:  {}", args.input.display());
    println!("Output: {}", args.output.display());
    println!("Profile: {:?}", args.profile);
    println!();

    // Load configuration
    let mut config = if let Some(config_path) = &args.config {
        AppConfig::load_from_file(config_path)?
    } else {
        args.profile.to_config()
    };

    // Apply command-line overrides
    if args.keep_intermediates {
        config.processing.keep_intermediates = true;
    }
    if args.jobs > 0 {
        config.processing.parallel_jobs = args.jobs;
    }
    if args.no_checkpoint {
        config.behavior.checkpoint_enabled = false;
    }

    // Create orchestrator
    let mut orchestrator = Orchestrator::new(
        args.input.clone(),
        args.output.clone(),
        config.processing,
    )?;

    if args.dry_run {
        println!("Dry run mode - validating inputs...");
        orchestrator.validate().await?;
        println!("✓ Validation successful");
        return Ok(());
    }

    // Create progress tracking
    let multi_progress = MultiProgress::new();
    let main_progress = multi_progress.add(ProgressBar::new(100));
    main_progress.set_style(
        ProgressStyle::default_bar()
            .template("[{elapsed_precise}] {bar:40.cyan/blue} {pos}/{len} {msg}")
            .unwrap()
            .progress_chars("=>-"),
    );

    // Run processing
    orchestrator.process(|update| {
        main_progress.set_length(update.total as u64);
        main_progress.set_position(update.current as u64);
        if let Some(file) = &update.file_name {
            main_progress.set_message(format!("{}: {}", update.step.name(), file));
        } else if let Some(msg) = &update.message {
            main_progress.set_message(msg.clone());
        } else {
            main_progress.set_message(update.step.name().to_string());
        }
    }).await?;

    main_progress.finish_with_message("✓ Processing complete");
    println!("\n✓ All videos processed successfully!");

    Ok(())
}

/// Resume processing from checkpoint
pub async fn resume_processing(checkpoint_path: &PathBuf) -> Result<()> {
    use crate::orchestrator::Orchestrator;

    println!("Resuming from checkpoint: {}", checkpoint_path.display());

    let mut orchestrator = Orchestrator::from_checkpoint(checkpoint_path)?;
    
    orchestrator.process(|update| {
        println!(
            "[{}/{}] {}: {}",
            update.current,
            update.total,
            update.step.name(),
            update.file_name.as_deref().unwrap_or("")
        );
    }).await?;

    println!("✓ Processing resumed and completed successfully!");

    Ok(())
}

/// Validate video files
pub async fn validate_videos(input_path: &PathBuf) -> Result<()> {
    use crate::ffmpeg::FFmpegWrapper;
    use walkdir::WalkDir;

    println!("Validating videos in: {}", input_path.display());

    if !input_path.exists() {
        return Err(VideoProcessorError::InputDirectoryNotFound(
            input_path.clone(),
        ));
    }

    let ffmpeg = FFmpegWrapper::new()?;
    let mut valid_count = 0;
    let mut invalid_count = 0;

    let video_extensions = ["mp4", "avi", "mov", "mkv", "m4v", "3gp"];

    for entry in WalkDir::new(input_path)
        .max_depth(1)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        let path = entry.path();
        if let Some(ext) = path.extension() {
            if video_extensions
                .iter()
                .any(|&e| e.eq_ignore_ascii_case(&ext.to_string_lossy()))
            {
                match ffmpeg.probe_video(path) {
                    Ok(metadata) => {
                        println!(
                            "✓ {} - {}x{} @ {:.2}fps",
                            path.file_name().unwrap().to_string_lossy(),
                            metadata.width,
                            metadata.height,
                            metadata.fps
                        );
                        valid_count += 1;
                    }
                    Err(e) => {
                        println!(
                            "✗ {} - Error: {}",
                            path.file_name().unwrap().to_string_lossy(),
                            e
                        );
                        invalid_count += 1;
                    }
                }
            }
        }
    }

    println!("\nValidation Summary:");
    println!("  Valid:   {}", valid_count);
    println!("  Invalid: {}", invalid_count);

    if invalid_count > 0 {
        Err(VideoProcessorError::ConfigError(format!(
            "{} invalid video file(s) found",
            invalid_count
        )))
    } else {
        Ok(())
    }
}

/// Handle configuration commands
pub fn handle_config_command(cmd: &ConfigCommands) -> Result<()> {
    match cmd {
        ConfigCommands::Show => {
            let config = AppConfig::load_or_default();
            let toml = toml::to_string_pretty(&config).map_err(|e| {
                VideoProcessorError::ConfigError(format!("Failed to serialize config: {}", e))
            })?;
            println!("{}", toml);
        }
        ConfigCommands::Init { output } => {
            let path = output
                .clone()
                .unwrap_or_else(|| AppConfig::default_config_path());
            let config = AppConfig::default();
            config.save_to_file(&path)?;
            println!("✓ Configuration file created: {}", path.display());
        }
        ConfigCommands::Path => {
            let path = AppConfig::default_config_path();
            println!("{}", path.display());
        }
    }
    Ok(())
}
