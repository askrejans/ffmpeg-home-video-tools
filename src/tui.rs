//! Full-screen progress view for renders.

use crossterm::event::{self, Event as KeyEvent, KeyCode, KeyEventKind, KeyModifiers};
use ffmpeg_video_processor::render::{RenderOptions, RenderOutcome, render_probed};
use ffmpeg_video_processor::{
    CancelToken, Error, Event, FfmpegTools, MediaInfo, Project, Result, Stage,
};
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Gauge, List, ListItem, ListState, Paragraph, Wrap};
use std::sync::mpsc;
use std::time::{Duration, Instant};

struct View {
    title: String,
    clips: Vec<(String, f64)>,
    stage: Stage,
    fraction: f64,
    fps: Option<f64>,
    eta: Option<f64>,
    clip: Option<usize>,
    log: Vec<(Color, String)>,
    started: Instant,
    confirm_cancel: bool,
}

fn name(path: &std::path::Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn clock(seconds: f64) -> String {
    let s = seconds.max(0.0).round() as u64;
    format!("{:02}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
}

pub fn run(
    tools: FfmpegTools,
    project: Project,
    media: Vec<MediaInfo>,
    options: RenderOptions,
    cancel: CancelToken,
) -> Result<()> {
    let mut view = View {
        title: format!(
            "{} clip(s) → {}",
            project.clips.len(),
            project.output.path.display()
        ),
        clips: media.iter().map(|m| (name(&m.path), m.duration)).collect(),
        stage: Stage::Preparing,
        fraction: 0.0,
        fps: None,
        eta: None,
        clip: None,
        log: Vec::new(),
        started: Instant::now(),
        confirm_cancel: false,
    };
    let (tx, rx) = mpsc::channel::<Event>();
    let worker_cancel = cancel.clone();
    let worker = std::thread::spawn(move || -> Result<RenderOutcome> {
        render_probed(
            &tools,
            &project,
            &media,
            &options,
            &mut |e| {
                let _ = tx.send(e);
            },
            &worker_cancel,
        )
    });

    let mut terminal = ratatui::init();
    let mut list_state = ListState::default();
    let result = loop {
        while let Ok(event) = rx.try_recv() {
            match event {
                Event::Stage { stage } => view.stage = stage,
                Event::Progress {
                    fraction,
                    stage,
                    fps,
                    eta_seconds,
                    clip,
                    ..
                } => {
                    view.fraction = fraction;
                    view.stage = stage;
                    if fps.is_some() {
                        view.fps = fps;
                    }
                    view.eta = eta_seconds;
                    if clip.is_some() {
                        view.clip = clip;
                    }
                }
                Event::Warning { message, .. } => view.log.push((Color::Yellow, message)),
                Event::Done { output, .. } => view
                    .log
                    .push((Color::Green, format!("Saved {}", output.display()))),
            }
        }
        list_state.select(view.clip);
        let _ = terminal.draw(|f| draw(f, &view, &mut list_state));
        if worker.is_finished() {
            break worker.join().unwrap_or(Err(Error::Cancelled));
        }
        if event::poll(Duration::from_millis(100)).unwrap_or(false)
            && let Ok(KeyEvent::Key(key)) = event::read()
            && key.kind == KeyEventKind::Press
        {
            let wants_cancel = matches!(key.code, KeyCode::Char('q') | KeyCode::Esc)
                || (key.code == KeyCode::Char('c')
                    && key.modifiers.contains(KeyModifiers::CONTROL));
            if wants_cancel && view.confirm_cancel {
                cancel.cancel();
                view.log.push((Color::Red, "Cancelling…".into()));
            } else {
                view.confirm_cancel = wants_cancel;
            }
        }
    };
    ratatui::restore();
    match result {
        Ok(outcome) => {
            println!(
                "✓ {}  ({}×{} {} fps, {}, {:.1} MB, {})",
                outcome.output.display(),
                outcome.width,
                outcome.height,
                outcome.fps,
                clock(outcome.duration_seconds),
                outcome.size_bytes as f64 / 1_048_576.0,
                outcome.video_encoder
            );
            for w in &outcome.warnings {
                if let Event::Warning { message, .. } = w {
                    println!("  warning: {message}");
                }
            }
            Ok(())
        }
        Err(e) => Err(e),
    }
}

fn draw(f: &mut ratatui::Frame, view: &View, list_state: &mut ListState) {
    let [header, body, gauge, footer] = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(6),
        Constraint::Length(3),
        Constraint::Length(1),
    ])
    .areas(f.area());
    f.render_widget(
        Paragraph::new(view.title.clone())
            .block(Block::bordered().title(" ffmpeg-video-processor ")),
        header,
    );
    let [clips, log] =
        Layout::horizontal([Constraint::Percentage(55), Constraint::Percentage(45)]).areas(body);
    let items: Vec<ListItem> = view
        .clips
        .iter()
        .enumerate()
        .map(|(i, (n, d))| ListItem::new(format!("{:>3}. {n}  ({})", i + 1, clock(*d))))
        .collect();
    f.render_stateful_widget(
        List::new(items)
            .block(Block::bordered().title(" Clips "))
            .highlight_style(
                Style::default()
                    .add_modifier(Modifier::BOLD)
                    .fg(Color::Cyan),
            )
            .highlight_symbol("▶ "),
        clips,
        list_state,
    );
    let lines: Vec<Line> = view
        .log
        .iter()
        .rev()
        .take(log.height.saturating_sub(2) as usize)
        .map(|(c, m)| Line::from(Span::styled(m.clone(), Style::default().fg(*c))))
        .collect();
    f.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: true })
            .block(Block::bordered().title(" Messages ")),
        log,
    );
    let mut label = format!("{:?} {:>3.0}%", view.stage, view.fraction * 100.0);
    if let Some(fps) = view.fps {
        label.push_str(&format!("  {fps:.0} fps"));
    }
    if let Some(eta) = view.eta {
        label.push_str(&format!("  {} left", clock(eta)));
    }
    label.push_str(&format!(
        "  elapsed {}",
        clock(view.started.elapsed().as_secs_f64())
    ));
    f.render_widget(
        Gauge::default()
            .block(Block::bordered())
            .gauge_style(Style::default().fg(Color::Magenta))
            .ratio(view.fraction.clamp(0.0, 1.0))
            .label(label),
        gauge,
    );
    let hint = if view.confirm_cancel {
        "Press q again to cancel the render, any other key to continue"
    } else {
        "q: cancel"
    };
    f.render_widget(
        Paragraph::new(hint).style(Style::default().fg(Color::DarkGray)),
        footer,
    );
}
