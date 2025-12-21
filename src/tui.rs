use crate::cli::ProcessArgs;
use crate::config::AppConfig;
use crate::error::{Result, VideoProcessorError};
use crate::orchestrator::Orchestrator;
use crate::types::{ProcessingStep, ProgressUpdate};
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{
        Block, Borders, Gauge, List, ListItem, Paragraph, Wrap,
    },
    Frame, Terminal,
};
use std::io;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use sysinfo::System;
use tokio::sync::mpsc;

/// TUI Application State
struct App {
    // Input/Output paths
    input_path: Option<PathBuf>,
    output_path: Option<PathBuf>,
    
    // Processing state
    is_processing: bool,
    is_completed: bool,
    current_step: Option<ProcessingStep>,
    step_progress: f32,
    overall_progress: f32,
    current_file: Option<String>,
    total_files: usize,
    processed_files: usize,
    
    // Logs
    logs: Vec<String>,
    max_logs: usize,
    
    // UI state
    selected_panel: Panel,
    input_buffer: String,
    input_mode: InputMode,
    
    // Status
    status_message: String,
    last_update: Instant,
    
    // Configuration
    config: AppConfig,
    
    // System monitoring
    cpu_usage: f32,
    ram_usage: u64,
    ram_total: u64,
    ffmpeg_pid: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Panel {
    Main,
    Logs,
    Config,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum InputMode {
    Normal,
    EditingInput,
    EditingOutput,
}

impl App {
    fn new(args: &ProcessArgs) -> Self {
        let config = if let Some(config_path) = &args.config {
            AppConfig::load_from_file(config_path).unwrap_or_else(|_| args.profile.to_config())
        } else {
            args.profile.to_config()
        };

        let input_path = if args.input.as_os_str().is_empty() {
            None
        } else {
            Some(args.input.clone())
        };

        let output_path = if args.output.as_os_str().is_empty() {
            None
        } else {
            Some(args.output.clone())
        };

        Self {
            input_path,
            output_path,
            is_processing: false,
            is_completed: false,
            current_step: None,
            step_progress: 0.0,
            overall_progress: 0.0,
            current_file: None,
            total_files: 0,
            processed_files: 0,
            logs: Vec::new(),
            max_logs: 1000,
            selected_panel: Panel::Main,
            input_buffer: String::new(),
            input_mode: InputMode::Normal,
            status_message: "Press 'i' to set input, 'o' for output, 's' to start, 'q' to quit".to_string(),
            last_update: Instant::now(),
            config,
            cpu_usage: 0.0,
            ram_usage: 0,
            ram_total: 0,
            ffmpeg_pid: None,
        }
    }

    fn add_log(&mut self, message: String) {
        let timestamp = chrono::Local::now().format("%H:%M:%S");
        self.logs.push(format!("[{}] {}", timestamp, message));
        if self.logs.len() > self.max_logs {
            self.logs.remove(0);
        }
    }

    fn can_start_processing(&self) -> bool {
        self.input_path.is_some() && self.output_path.is_some() && !self.is_processing
    }

    fn update_system_stats(&mut self) {
        // Create system instance and refresh
        let mut system = System::new_all();
        system.refresh_memory();
        
        // Refresh CPU info - need to wait for accurate readings
        system.refresh_cpu_all();
        std::thread::sleep(std::time::Duration::from_millis(200));
        system.refresh_cpu_all();
        
        // Get global CPU usage (average across all CPUs)
        let cpus = system.cpus();
        if !cpus.is_empty() {
            self.cpu_usage = cpus.iter().map(|cpu| cpu.cpu_usage()).sum::<f32>() / cpus.len() as f32;
        }
        
        // Get RAM usage
        self.ram_usage = system.used_memory();
        self.ram_total = system.total_memory();
        
        // Find any FFmpeg process (PID changes for each video)
        // Only update PID if we're actively processing and ffmpeg is found
        if self.current_step.is_some() && !self.is_completed {
            let mut found_ffmpeg = false;
            for (pid, process) in system.processes() {
                if let Some(name) = process.name().to_str() {
                    let name_lower = name.to_lowercase();
                    if name_lower.contains("ffmpeg") || name_lower == "ffmpeg" {
                        // Only update if we don't have a PID yet, or if this is a different PID
                        // This prevents rapid PID changes in the UI
                        if self.ffmpeg_pid.is_none() || self.ffmpeg_pid != Some(pid.as_u32()) {
                            self.ffmpeg_pid = Some(pid.as_u32());
                        }
                        found_ffmpeg = true;
                        break; // Take the first ffmpeg process found
                    }
                }
            }
            // Clear PID if no ffmpeg process found and we're between videos
            if !found_ffmpeg {
                self.ffmpeg_pid = None;
            }
        } else {
            // Not processing, clear the PID
            self.ffmpeg_pid = None;
        }
    }
    
    fn update_progress(&mut self, update: ProgressUpdate) {
        self.current_step = Some(update.step);
        self.current_file = update.file_name.clone();
        
        // Check for explicit completion signal
        if update.is_complete {
            self.is_completed = true;
            self.is_processing = false;
            self.add_log("✓ Processing completed successfully!".to_string());
            self.status_message = "Processing complete! Press 'q' to quit".to_string();
            return;
        }
        
        // Update file counts
        if update.total > 0 {
            self.total_files = update.total;
            self.processed_files = update.current;
        }
        
        let step_percent = update.progress_percentage();
        self.step_progress = step_percent;
        
        // Calculate overall progress (6 steps total)
        let step_index = match update.step {
            ProcessingStep::BatchConvert => 0,
            ProcessingStep::Pad => 1,
            ProcessingStep::Crop => 2,
            ProcessingStep::Resample => 3,
            ProcessingStep::Concatenate => 4,
        };
        
        self.overall_progress = ((step_index as f32 * 100.0) + step_percent) / 6.0;
        
        // Log with more detail
        if let Some(msg) = &update.message {
            self.add_log(msg.clone());
        } else if let Some(file) = &update.file_name {
            self.add_log(format!("{}: [{}/{}] {}", 
                update.step.name(), 
                update.current, 
                update.total,
                file
            ));
        } else {
            self.add_log(format!("{}: {:.1}% complete", update.step.name(), step_percent));
        }
        
        // Check for completion
        if self.overall_progress >= 99.9 {
            self.is_completed = true;
            self.is_processing = false;
            self.add_log("✓ Processing completed successfully!".to_string());
            self.status_message = "Processing complete! Press 'q' to quit".to_string();
        }
        
        self.last_update = Instant::now();
    }
}

/// Run processing in TUI mode with ratatui interface
pub async fn run_tui_mode(args: &ProcessArgs) -> Result<()> {
    // Setup terminal
    enable_raw_mode().map_err(|e| VideoProcessorError::ConfigError(e.to_string()))?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)
        .map_err(|e| VideoProcessorError::ConfigError(e.to_string()))?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)
        .map_err(|e| VideoProcessorError::ConfigError(e.to_string()))?;

    let app_state = Arc::new(Mutex::new(App::new(args)));
    let result = run_app(&mut terminal, app_state, args).await;

    // Restore terminal
    disable_raw_mode().map_err(|e| VideoProcessorError::ConfigError(e.to_string()))?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )
    .map_err(|e| VideoProcessorError::ConfigError(e.to_string()))?;
    terminal.show_cursor()
        .map_err(|e| VideoProcessorError::ConfigError(e.to_string()))?;

    result
}

async fn run_app(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app_state: Arc<Mutex<App>>,
    _args: &ProcessArgs,
) -> Result<()> {
    let (tx, mut rx) = mpsc::channel::<ProgressUpdate>(100);
    let tick_rate = Duration::from_millis(100);
    let mut last_tick = Instant::now();
    let mut last_stats_update = Instant::now();

    loop {
        // Draw UI
        {
            let app = app_state.lock().unwrap();
            terminal
                .draw(|f| ui(f, &app))
                .map_err(|e| VideoProcessorError::ConfigError(e.to_string()))?;
        }

        // Handle input with timeout
        let timeout = tick_rate
            .checked_sub(last_tick.elapsed())
            .unwrap_or_else(|| Duration::from_secs(0));

        if event::poll(timeout).map_err(|e| VideoProcessorError::ConfigError(e.to_string()))? {
            if let Event::Key(key) = event::read()
                .map_err(|e| VideoProcessorError::ConfigError(e.to_string()))?
            {
                let mut app = app_state.lock().unwrap();
                
                match app.input_mode {
                    InputMode::Normal => match key.code {
                        KeyCode::Char('q') => {
                            if !app.is_processing {
                                return Ok(());
                            }
                        }
                        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                            return Ok(());
                        }
                        KeyCode::Char('i') if !app.is_processing => {
                            app.input_mode = InputMode::EditingInput;
                            app.input_buffer.clear();
                            app.status_message = "Enter input path (press Enter to confirm, Esc to cancel)".to_string();
                        }
                        KeyCode::Char('o') if !app.is_processing => {
                            app.input_mode = InputMode::EditingOutput;
                            app.input_buffer.clear();
                            app.status_message = "Enter output path (press Enter to confirm, Esc to cancel)".to_string();
                        }
                        KeyCode::Char('s') if !app.is_processing => {
                            if app.can_start_processing() {
                                app.is_processing = true;
                                app.add_log("Starting video processing...".to_string());
                                
                                let input = app.input_path.clone().unwrap();
                                let output = app.output_path.clone().unwrap();
                                let config = app.config.processing.clone();
                                let tx_clone = tx.clone();
                                
                                tokio::spawn(async move {
                                    if let Ok(mut orchestrator) = Orchestrator::new(input, output, config) {
                                        let tx_inner = tx_clone.clone();
                                        let _ = orchestrator.process(move |update| {
                                            // Use try_send instead of blocking_send in async context
                                            let _ = tx_inner.try_send(update);
                                        }).await;
                                    }
                                });
                            } else {
                                app.status_message = "Please set both input and output paths first!".to_string();
                            }
                        }
                        KeyCode::Tab => {
                            app.selected_panel = match app.selected_panel {
                                Panel::Main => Panel::Logs,
                                Panel::Logs => Panel::Config,
                                Panel::Config => Panel::Main,
                            };
                        }
                        _ => {}
                    },
                    InputMode::EditingInput | InputMode::EditingOutput => match key.code {
                        KeyCode::Enter => {
                            let path = PathBuf::from(&app.input_buffer);
                            match app.input_mode {
                                InputMode::EditingInput => {
                                    app.input_path = Some(path.clone());
                                    app.add_log(format!("Input path set to: {}", path.display()));
                                    app.status_message = format!("Input path: {}", path.display());
                                }
                                InputMode::EditingOutput => {
                                    app.output_path = Some(path.clone());
                                    app.add_log(format!("Output path set to: {}", path.display()));
                                    app.status_message = format!("Output path: {}", path.display());
                                }
                                _ => {}
                            }
                            app.input_mode = InputMode::Normal;
                            app.input_buffer.clear();
                        }
                        KeyCode::Esc => {
                            app.input_mode = InputMode::Normal;
                            app.input_buffer.clear();
                            app.status_message = "Cancelled".to_string();
                        }
                        KeyCode::Backspace => {
                            app.input_buffer.pop();
                        }
                        KeyCode::Char(c) => {
                            app.input_buffer.push(c);
                        }
                        _ => {}
                    },
                }
            }
        }

        // Check for progress updates
        while let Ok(update) = rx.try_recv() {
            let mut app = app_state.lock().unwrap();
            app.update_progress(update);
        }
        
        // Update system stats every second
        if last_stats_update.elapsed() >= Duration::from_secs(1) {
            let mut app = app_state.lock().unwrap();
            app.update_system_stats();
            last_stats_update = Instant::now();
        }

        if last_tick.elapsed() >= tick_rate {
            last_tick = Instant::now();
        }
    }
}

fn ui(f: &mut Frame, app: &App) {
    let size = f.area();

    // Main layout: Title, Content, Status
    let main_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(10),
            Constraint::Length(3),
        ])
        .split(size);

    // Title with system stats
    let ram_gb = app.ram_usage as f64 / 1024.0 / 1024.0 / 1024.0;
    let ram_total_gb = app.ram_total as f64 / 1024.0 / 1024.0 / 1024.0;
    let ffmpeg_status = if app.ffmpeg_pid.is_some() { "●" } else { "○" };
    
    let title_text = format!(
        "FFmpeg Video Processor  |  CPU: {:.1}%  RAM: {:.1}/{:.1}GB  FFmpeg: {}",
        app.cpu_usage, ram_gb, ram_total_gb, ffmpeg_status
    );
    
    let title = Paragraph::new(title_text)
        .style(Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD))
        .alignment(Alignment::Center)
        .block(Block::default().borders(Borders::ALL));
    f.render_widget(title, main_chunks[0]);

    // Content area split: Left (controls/progress) and Right (logs/config)
    let content_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(main_chunks[1]);

    // Left panel
    render_control_panel(f, app, content_chunks[0]);

    // Right panel (tabbed)
    match app.selected_panel {
        Panel::Main | Panel::Logs => render_logs_panel(f, app, content_chunks[1]),
        Panel::Config => render_config_panel(f, app, content_chunks[1]),
    }

    // Status bar
    let status_text = if app.input_mode != InputMode::Normal {
        format!("INPUT: {}_", app.input_buffer)
    } else {
        app.status_message.clone()
    };
    
    let status = Paragraph::new(status_text)
        .style(Style::default().fg(Color::Yellow))
        .block(Block::default().borders(Borders::ALL).title("Status"));
    f.render_widget(status, main_chunks[2]);
}

fn render_control_panel(f: &mut Frame, app: &App, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(7),  // Paths
            Constraint::Length(8),  // Progress
            Constraint::Length(6),  // System info
            Constraint::Min(5),     // Current operation
        ])
        .split(area);

    // Paths section
    let input_text = if let Some(path) = &app.input_path {
        format!("Input:  {}", path.display())
    } else {
        "Input:  Not set (press 'i')".to_string()
    };
    
    let output_text = if let Some(path) = &app.output_path {
        format!("Output: {}", path.display())
    } else {
        "Output: Not set (press 'o')".to_string()
    };

    let paths_text = vec![
        Line::from(input_text),
        Line::from(output_text),
        Line::from(""),
        Line::from(vec![
            Span::styled("Commands: ", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw("'s' Start  'q' Quit  Tab=Switch Panel"),
        ]),
    ];

    let paths = Paragraph::new(paths_text)
        .block(Block::default().borders(Borders::ALL).title("Paths & Controls"))
        .style(Style::default().fg(Color::White));
    f.render_widget(paths, chunks[0]);

    // Progress section
    let overall_label = format!("Overall Progress: {:.1}%", app.overall_progress);
    let overall_gauge = Gauge::default()
        .block(Block::default().title(overall_label))
        .gauge_style(Style::default().fg(Color::Green).bg(Color::Black))
        .ratio(app.overall_progress as f64 / 100.0);

    let step_name = if let Some(step) = app.current_step {
        step.name()
    } else {
        "Waiting"
    };
    
    let step_label = format!("{}: {:.1}%", step_name, app.step_progress);
    let step_gauge = Gauge::default()
        .block(Block::default().title(step_label))
        .gauge_style(Style::default().fg(Color::Cyan).bg(Color::Black))
        .ratio(app.step_progress as f64 / 100.0);

    let progress_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Length(3)])
        .split(chunks[1]);

    f.render_widget(overall_gauge, progress_chunks[0]);
    f.render_widget(step_gauge, progress_chunks[1]);

    // System information
    let ram_gb = app.ram_usage as f64 / 1024.0 / 1024.0 / 1024.0;
    let ram_total_gb = app.ram_total as f64 / 1024.0 / 1024.0 / 1024.0;
    let ram_percent = if app.ram_total > 0 {
        (app.ram_usage as f64 / app.ram_total as f64) * 100.0
    } else {
        0.0
    };
    
    let ffmpeg_status = if let Some(pid) = app.ffmpeg_pid {
        format!("Running (PID: {})", pid)
    } else {
        "Idle".to_string()
    };
    
    let system_text = vec![
        Line::from(format!("CPU Usage: {:.1}%", app.cpu_usage)),
        Line::from(format!("RAM: {:.2}/{:.2} GB ({:.1}%)", ram_gb, ram_total_gb, ram_percent)),
        Line::from(format!("FFmpeg: {}", ffmpeg_status)),
    ];
    
    let system_info = Paragraph::new(system_text)
        .block(Block::default().borders(Borders::ALL).title("System Resources"))
        .style(Style::default().fg(Color::White));
    f.render_widget(system_info, chunks[2]);

    // Current operation
    let status_text = if app.is_completed {
        format!("✓ All processing completed successfully!\n\nTotal files processed: {}\n\nYou can now press 'q' to quit.", app.processed_files)
    } else if app.is_processing {
        if let Some(file) = &app.current_file {
            if app.total_files > 0 {
                format!("Processing: {}\n\nFile {} of {}\n\nStep: {}\nProgress: {:.1}%", 
                    file,
                    app.processed_files,
                    app.total_files,
                    app.current_step.map(|s| s.name()).unwrap_or("Unknown"),
                    app.step_progress
                )
            } else {
                format!("Processing: {}\n\nStep: {}", file, app.current_step.map(|s| s.name()).unwrap_or("Unknown"))
            }
        } else if let Some(step) = app.current_step {
            format!("Running: {}\n\n{}", step.name(), step.description())
        } else {
            "Starting pipeline...".to_string()
        }
    } else {
        "Ready to start processing\n\nPress 's' to begin".to_string()
    };

    let current_op = Paragraph::new(status_text)
        .block(Block::default().borders(Borders::ALL).title("Current Operation"))
        .wrap(Wrap { trim: true })
        .style(Style::default().fg(Color::White));
    f.render_widget(current_op, chunks[3]);
}

fn render_logs_panel(f: &mut Frame, app: &App, area: Rect) {
    let log_items: Vec<ListItem> = app
        .logs
        .iter()
        .rev()
        .take(100)
        .map(|log| ListItem::new(log.as_str()))
        .collect();

    let logs = List::new(log_items)
        .block(Block::default().borders(Borders::ALL).title("Processing Logs (Tab to switch)"))
        .style(Style::default().fg(Color::White));

    f.render_widget(logs, area);
}

fn render_config_panel(f: &mut Frame, app: &App, area: Rect) {
    let config = &app.config.processing;
    
    let config_text = vec![
        Line::from(vec![
            Span::styled("Output Configuration", Style::default().add_modifier(Modifier::BOLD).fg(Color::Cyan)),
        ]),
        Line::from(""),
        Line::from(format!("Resolution: {}x{}", config.target_resolution.0, config.target_resolution.1)),
        Line::from(format!("Frame Rate: {} fps", config.target_fps)),
        Line::from(format!("Video Codec: {}", config.video_codec)),
        Line::from(format!("Video Preset: {}", config.video_preset)),
        Line::from(format!("Video CRF: {} (quality)", config.video_crf)),
        Line::from(""),
        Line::from(format!("Audio Codec: {}", config.audio_codec)),
        Line::from(format!("Audio Bitrate: {} kbps", config.audio_bitrate)),
        Line::from(format!("Audio Sample Rate: {} Hz", config.audio_sample_rate)),
        Line::from(""),
        Line::from(vec![
            Span::styled("Note: ", Style::default().fg(Color::Yellow)),
            Span::raw("Edit config.toml to change settings"),
        ]),
    ];

    let config_display = Paragraph::new(config_text)
        .block(Block::default().borders(Borders::ALL).title("Configuration (Tab to switch)"))
        .style(Style::default().fg(Color::White))
        .wrap(Wrap { trim: true });

    f.render_widget(config_display, area);
}
