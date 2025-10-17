use std::error::Error;
use std::io::stdout;
use std::time::{Duration, Instant};

use crossbeam_channel::Receiver;
use crossbeam_channel::unbounded;
use crossterm::ExecutableCommand;
use crossterm::event::{self, Event as CEvent, KeyCode, KeyEvent};
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, Clear, Paragraph, Row, Table, Wrap};

use crate::control::api;
use crate::{MixerStatus, SourceStatus};

const TICK_RATE: Duration = Duration::from_millis(100);

#[derive(Default)]
struct AppState {
    status: Option<MixerStatus>,
    selected: usize,
    mode: Mode,
    message: Option<String>,
    last_update: Option<Instant>,
}

#[derive(Default, Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    #[default]
    Normal,
    GainInput,
}

struct GainEditor {
    buffer: String,
}

/// Run the ratatui-based developer console.
pub fn run() -> Result<(), Box<dyn Error>> {
    setup_terminal()?;

    let mut terminal = Terminal::new(CrosstermBackend::new(stdout()))?;
    terminal.clear()?;

    let (status_tx, status_rx) = unbounded();
    std::thread::spawn(move || {
        loop {
            let status = api::get_status();
            if status_tx.send(status).is_err() {
                break;
            }
            std::thread::sleep(TICK_RATE);
        }
    });

    let mut app = AppState::default();
    let mut gain_editor: Option<GainEditor> = None;

    loop {
        terminal.draw(|frame| draw(frame, &app, gain_editor.as_ref()))?;

        if let Some(status) = try_recv_latest(&status_rx) {
            app.status = status;
            app.last_update = Some(Instant::now());
            let source_len = app.status.as_ref().map(|s| s.sources.len()).unwrap_or(0);
            if source_len > 0 {
                app.selected = app.selected.min(source_len - 1);
            } else {
                app.selected = 0;
            }
        }

        if event::poll(Duration::from_millis(10))? {
            match event::read()? {
                CEvent::Key(key) => {
                    if handle_key(&mut app, &mut gain_editor, key)? {
                        break;
                    }
                }
                _ => {}
            }
        }
    }

    restore_terminal()?;
    Ok(())
}

fn setup_terminal() -> Result<(), Box<dyn Error>> {
    enable_raw_mode()?;
    stdout().execute(EnterAlternateScreen)?;
    Ok(())
}

fn restore_terminal() -> Result<(), Box<dyn Error>> {
    disable_raw_mode()?;
    stdout().execute(LeaveAlternateScreen)?;
    Ok(())
}

fn try_recv_latest<T>(rx: &Receiver<T>) -> Option<T> {
    let mut last = None;
    while let Ok(value) = rx.try_recv() {
        last = Some(value);
    }
    last
}

fn handle_key(
    app: &mut AppState,
    gain_editor: &mut Option<GainEditor>,
    key: KeyEvent,
) -> Result<bool, Box<dyn Error>> {
    match app.mode {
        Mode::Normal => match key.code {
            KeyCode::Char('q') => return Ok(true),
            KeyCode::Up => {
                if app.selected > 0 {
                    app.selected -= 1;
                }
            }
            KeyCode::Down => {
                if let Some(status) = &app.status {
                    if app.selected + 1 < status.sources.len() {
                        app.selected += 1;
                    }
                }
            }
            KeyCode::Char('m') => {
                if let Some(src) = current_source(app) {
                    let new_state = !src.muted;
                    if api::set_mute(src.id, new_state) {
                        app.message = Some(format!(
                            "Source {} {}",
                            src.name,
                            if new_state { "muted" } else { "unmuted" }
                        ));
                    }
                }
            }
            KeyCode::Char('g') => {
                if let Some(src) = current_source(app) {
                    gain_editor.replace(GainEditor {
                        buffer: format!("{:.1}", src.gain_db),
                    });
                    app.mode = Mode::GainInput;
                }
            }
            _ => {}
        },
        Mode::GainInput => match key.code {
            KeyCode::Esc => {
                gain_editor.take();
                app.mode = Mode::Normal;
            }
            KeyCode::Enter => {
                if let (Some(editor), Some(src)) = (gain_editor.take(), current_source(app)) {
                    if let Ok(value) = editor.buffer.trim().parse::<f32>() {
                        if api::set_gain(src.id, value) {
                            app.message = Some(format!("Set {} gain to {:.1} dB", src.name, value));
                        }
                    }
                }
                app.mode = Mode::Normal;
            }
            KeyCode::Backspace => {
                if let Some(editor) = gain_editor.as_mut() {
                    editor.buffer.pop();
                }
            }
            KeyCode::Char(c) => {
                if let Some(editor) = gain_editor.as_mut() {
                    if c.is_ascii_digit() || matches!(c, '.' | '-' | '+') {
                        editor.buffer.push(c);
                    }
                }
            }
            _ => {}
        },
    }
    Ok(false)
}

fn current_source(app: &AppState) -> Option<SourceStatus> {
    app.status.as_ref()?.sources.get(app.selected).cloned()
}

fn draw(frame: &mut ratatui::Frame<'_>, app: &AppState, gain_editor: Option<&GainEditor>) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(8),
            Constraint::Length(3),
        ])
        .split(frame.size());

    draw_header(frame, chunks[0], app);
    draw_sources(frame, chunks[1], app);
    draw_footer(frame, chunks[2], app);

    if let Some(editor) = gain_editor {
        let area = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(0), Constraint::Length(3)])
            .split(frame.size())[1];

        let block = Block::default()
            .title("Set Gain (dB) — Enter to apply, Esc to cancel")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Yellow));

        let paragraph = Paragraph::new(editor.buffer.clone())
            .block(block)
            .wrap(Wrap { trim: false });

        frame.render_widget(Clear, area);
        frame.render_widget(paragraph, area);
    }
}

fn draw_header(frame: &mut ratatui::Frame<'_>, area: ratatui::prelude::Rect, app: &AppState) {
    let block = Block::default()
        .title("Loopback Mixer Console")
        .borders(Borders::ALL);

    let content = if let Some(status) = &app.status {
        let stats = format!(
            "Sample Rate: {} Hz    Buffer: {} frames    Latency: {:.2} ms    CPU: {:.1}%    Fill: {:.1}%    Drift: {:.1} ppm",
            status.sample_rate,
            status.buffer_frames,
            status.latency_ms,
            status.cpu_usage * 100.0,
            status.buffer_fill * 100.0,
            status.drift_ppm,
        );
        Paragraph::new(stats)
    } else {
        Paragraph::new(Line::from(vec![Span::styled(
            "No active mixer",
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        )]))
    };

    frame.render_widget(content.block(block), area);
}

fn draw_sources(frame: &mut ratatui::Frame<'_>, area: ratatui::prelude::Rect, app: &AppState) {
    let block = Block::default().title("Sources").borders(Borders::ALL);

    if let Some(status) = &app.status {
        let header = Row::new(vec![
            Cell::from(""),
            Cell::from("Name"),
            Cell::from("Gain (dB)"),
            Cell::from("Muted"),
            Cell::from("RMS"),
            Cell::from("Latency (frames)"),
            Cell::from("Buffer %"),
            Cell::from("Drift ppm"),
        ])
        .style(
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        );

        let rows = status.sources.iter().enumerate().map(|(idx, src)| {
            let indicator = if idx == app.selected { ">" } else { "" };
            let mut row = Row::new(vec![
                Cell::from(indicator.to_string()),
                Cell::from(src.name.clone()),
                Cell::from(format!("{:.1}", src.gain_db)),
                Cell::from(if src.muted { "Yes" } else { "No" }),
                Cell::from(format!("{:.2}", src.rms)),
                Cell::from(format!("{}", src.latency_frames)),
                Cell::from(format!("{:.1}", src.buffer_fill * 100.0)),
                Cell::from(format!("{:.1}", src.drift_ppm)),
            ]);
            if idx == app.selected {
                row = row.style(Style::default().fg(Color::Yellow));
            }
            row
        });

        let table = Table::new(
            rows,
            [
                Constraint::Length(2),
                Constraint::Length(20),
                Constraint::Length(12),
                Constraint::Length(8),
                Constraint::Length(8),
                Constraint::Length(16),
                Constraint::Length(12),
                Constraint::Length(12),
            ],
        )
        .header(header)
        .block(block)
        .column_spacing(2);

        frame.render_widget(table, area);
    } else {
        frame.render_widget(Paragraph::new("").block(block), area);
    }
}

fn draw_footer(frame: &mut ratatui::Frame<'_>, area: ratatui::prelude::Rect, app: &AppState) {
    let info = "Up/Down: Select  •  g: Set gain  •  m: Toggle mute  •  q: Quit";
    let mut lines = vec![Line::from(info)];
    if let Some(message) = &app.message {
        lines.push(Line::from(Span::styled(
            message.clone(),
            Style::default().fg(Color::Green),
        )));
    }
    if let Some(updated) = app.last_update {
        let ago = updated.elapsed().as_secs_f32();
        lines.push(Line::from(Span::styled(
            format!("Last update {:.1}s ago", ago),
            Style::default().fg(Color::DarkGray),
        )));
    }

    let paragraph = Paragraph::new(lines)
        .block(Block::default().borders(Borders::ALL).title("Help"))
        .wrap(Wrap { trim: true });
    frame.render_widget(paragraph, area);
}
