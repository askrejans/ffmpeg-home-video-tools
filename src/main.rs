mod cli;
mod config;
mod error;
mod ffmpeg;
mod orchestrator;
mod processing;
mod tui;
mod types;

use clap::Parser;
use cli::{Cli, Commands};
use error::Result;
use tracing::info;

#[tokio::main]
async fn main() -> Result<()> {
    // Parse command-line arguments
    let cli = Cli::parse();

    // Check if we're running in TUI mode early to avoid stdout pollution
    let is_tui_mode = if let Commands::Process(args) = &cli.command {
        !args.no_tui && atty::is(atty::Stream::Stdout)
    } else {
        false
    };

    // Initialize logging (silent for TUI mode)
    if is_tui_mode {
        init_tui_logging()?;
    } else {
        init_logging(&cli)?;
        info!("FFmpeg Video Processor v{}", env!("CARGO_PKG_VERSION"));
    }

    // Check for FFmpeg availability (only log in non-TUI mode)
    ffmpeg::FFmpegWrapper::check_availability()?;

    // Execute command
    match &cli.command {
        Commands::Process(args) => {
            if is_tui_mode {
                tui::run_tui_mode(args).await?;
            } else {
                cli::run_cli_mode(args).await?;
            }
        }
        Commands::Resume { checkpoint } => {
            cli::resume_processing(checkpoint).await?;
        }
        Commands::Validate { input } => {
            cli::validate_videos(input).await?;
        }
        Commands::Config(config_cmd) => {
            cli::handle_config_command(config_cmd)?;
        }
    }

    Ok(())
}

fn init_logging(cli: &Cli) -> Result<()> {
    use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

    let log_level = cli.log_level.as_deref().unwrap_or("info");
    
    let env_filter = EnvFilter::try_from_default_env()
        .or_else(|_| EnvFilter::try_new(log_level))
        .unwrap();

    let fmt_layer = fmt::layer()
        .with_target(false)
        .with_thread_ids(false)
        .with_thread_names(false);

    tracing_subscriber::registry()
        .with(env_filter)
        .with(fmt_layer)
        .init();

    Ok(())
}

fn init_tui_logging() -> Result<()> {
    use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

    // For TUI mode, only log errors to stderr (won't interfere with TUI)
    let env_filter = EnvFilter::try_new("error").unwrap();

    let fmt_layer = fmt::layer()
        .with_writer(std::io::stderr)
        .with_target(false)
        .with_thread_ids(false)
        .with_thread_names(false);

    // Clear previous subscriber and reinitialize
    tracing_subscriber::registry()
        .with(env_filter)
        .with(fmt_layer)
        .try_init()
        .ok(); // Ignore error if already initialized

    Ok(())
}

