mod cli;
#[cfg(feature = "tui")]
mod tui;

use clap::Parser;
use std::io::IsTerminal;
use std::process::ExitCode;

fn main() -> ExitCode {
    let cli = cli::Cli::parse();
    let full_screen = match &cli.command {
        cli::Command::Render(a) => !a.run.no_tui && !a.run.json && !a.run.dry_run,
        cli::Command::Process(a) => !a.no_tui && !a.json && !a.dry_run,
        _ => false,
    } && cfg!(feature = "tui")
        && std::io::stdout().is_terminal();
    let level = match (&cli.log_level, cli.verbose) {
        (Some(level), _) => level.clone(),
        (None, true) => "debug".into(),
        // Log lines would tear the full-screen view.
        (None, false) if full_screen => "off".into(),
        (None, false) => "warn".into(),
    };
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .or_else(|_| tracing_subscriber::EnvFilter::try_new(&level))
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .with_target(false)
        .init();

    match cli::run(cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(ffmpeg_video_processor::Error::Cancelled) => {
            eprintln!("cancelled");
            ExitCode::from(130)
        }
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}
