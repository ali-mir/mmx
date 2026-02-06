mod app;
mod event;
mod format;
mod theme;
mod ui;

use std::io;
use std::io::Write;
use std::path::PathBuf;
use std::time::Duration;

use clap::Parser;
use crossterm::event::{KeyCode, KeyModifiers};
use crossterm::terminal::{self, EnterAlternateScreen, LeaveAlternateScreen};
use crossterm::{event::KeyEventKind, execute};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;

use app::{App, AppMode, Message, MetricEntry};
use event::{Event, EventHandler};

use mmx_ftdc::chunk::DecodedChunk;
use mmx_ftdc::reader;

#[derive(Parser, Debug)]
#[command(
    name = "mmx",
    about = "mongometrics - terminal ui for mongod FTDC metrics"
)]
struct Cli {
    /// Path to FTDC file or directory containing FTDC files
    path: PathBuf,
}

/// Extract the latest sample from all FTDC chunks as MetricEntry list.
/// Returns (metrics, timestamp_epoch_ms).
fn latest_sample(chunks: &[DecodedChunk]) -> Option<(Vec<MetricEntry>, Option<i64>)> {
    let last = chunks.last()?;
    if last.metrics.is_empty() {
        return None;
    }

    let num_samples = last.metrics[0].values.len();
    if num_samples == 0 {
        return None;
    }

    let last_idx = num_samples - 1;
    let prev_idx = if num_samples >= 2 {
        Some(num_samples - 2)
    } else {
        None
    };

    let mut timestamp = None;

    let metrics = last
        .metrics
        .iter()
        .map(|m| {
            let current = m.values[last_idx];
            let previous = prev_idx.map(|i| m.values[i]);
            if m.path == "start" {
                timestamp = Some(current);
            }
            MetricEntry {
                path: m.path.clone(),
                current,
                previous,
                history: Vec::new(),
            }
        })
        .collect();

    Some((metrics, timestamp))
}

/// Diagnostic info from a load attempt.
struct LoadResult {
    chunks: Vec<DecodedChunk>,
    info: String,
}

/// Try to read an FTDC file. Returns (chunks, error_message).
fn try_read_ftdc_file(path: &std::path::Path) -> (Vec<DecodedChunk>, Option<String>) {
    let data = match std::fs::read(path) {
        Ok(d) => d,
        Err(e) => return (Vec::new(), Some(format!("read: {e}"))),
    };
    let file_len = data.len();

    // Check BSON doc completeness
    if data.len() < 4 {
        return (Vec::new(), Some(format!("{file_len}B too small")));
    }
    let doc_size = u32::from_le_bytes([data[0], data[1], data[2], data[3]]) as usize;
    if file_len < doc_size {
        return (Vec::new(), Some(format!("{file_len}B < {doc_size}B bson")));
    }

    let cursor = io::Cursor::new(data);
    match reader::read_ftdc_file(cursor) {
        Ok(chunks) if chunks.is_empty() => (Vec::new(), Some(format!("{file_len}B, 0 chunks"))),
        Ok(chunks) => (chunks, None),
        Err(e) => (Vec::new(), Some(format!("{e}"))),
    }
}

/// Load all FTDC chunks. Retries the interim file (the live data source)
/// a few times since it's being actively rewritten by mongod every second.
fn load_all_chunks(path: &std::path::Path) -> LoadResult {
    let files = match reader::find_ftdc_files(path) {
        Ok(f) => f,
        Err(e) => {
            return LoadResult {
                chunks: Vec::new(),
                info: format!("find: {e}"),
            };
        }
    };

    let mut all_chunks: Vec<DecodedChunk> = Vec::new();
    let mut parts: Vec<String> = Vec::new();

    for file_path in &files {
        let name = file_path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        let is_interim = name.contains("interim");

        if is_interim {
            for attempt in 0..5 {
                let (chunks, err) = try_read_ftdc_file(file_path);
                if !chunks.is_empty() {
                    let n = chunks.len();
                    parts.push(format!("interim: {n}ch@try{attempt}"));
                    all_chunks.extend(chunks);
                    break;
                }
                if attempt < 4 {
                    std::thread::sleep(Duration::from_millis(50));
                }
                if attempt == 4 {
                    let reason = err.unwrap_or_else(|| "unknown".into());
                    parts.push(format!("interim: fail({reason})"));
                }
            }
        } else {
            let (chunks, err) = try_read_ftdc_file(file_path);
            let n = chunks.len();
            if n > 0 {
                parts.push(format!("{name}: {n}ch"));
            } else {
                let reason = err.unwrap_or_else(|| "0 chunks".into());
                parts.push(format!("{name}: {reason}"));
            }
            all_chunks.extend(chunks);
        }
    }

    LoadResult {
        chunks: all_chunks,
        info: parts.join(" | "),
    }
}

/// Format an epoch-ms timestamp as a human-readable local time string.
fn format_sample_timestamp(epoch_ms: i64) -> String {
    let epoch_secs = epoch_ms / 1_000;

    let (year, month, day, hour, minute, second, wday) = unsafe {
        let time = epoch_secs as libc::time_t;
        let mut tm: libc::tm = std::mem::zeroed();
        libc::localtime_r(&time, &mut tm);
        (
            tm.tm_year + 1900,
            tm.tm_mon + 1,
            tm.tm_mday,
            tm.tm_hour,
            tm.tm_min,
            tm.tm_sec,
            tm.tm_wday,
        )
    };

    let day_name = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"][wday as usize];
    let month_name = [
        "", "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ][month as usize];

    let ordinal = match day {
        1 | 21 | 31 => "st",
        2 | 22 => "nd",
        3 | 23 => "rd",
        _ => "th",
    };

    format!("{hour:02}:{minute:02}:{second:02}, {day_name} {month_name} {day}{ordinal} {year}")
}

#[tokio::main]
async fn main() -> color_eyre::Result<()> {
    color_eyre::install()?;

    let cli = Cli::parse();
    let path = cli.path;

    if !path.exists() {
        eprintln!("Error: path '{}' does not exist", path.display());
        std::process::exit(1);
    }

    let mut app = App::new(path.display().to_string());

    // Open debug log
    let mut log = std::fs::File::create("mmx-debug.log")?;

    // Load initial data
    let result = load_all_chunks(&path);
    writeln!(log, "tick:0 {}", result.info)?;
    if let Some((metrics, ts)) = latest_sample(&result.chunks) {
        if let Some(epoch_ms) = ts {
            app.sample_timestamp = format_sample_timestamp(epoch_ms);
        }
        app.update(Message::UpdateMetrics(metrics));
    }

    // Set up terminal
    terminal::enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    terminal.clear()?;

    // Set up panic hook to restore terminal
    let original_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic| {
        let _ = terminal::disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen);
        original_hook(panic);
    }));

    // Event loop
    let mut events = EventHandler::new(Duration::from_secs(1), Duration::from_millis(100));

    loop {
        if app.should_quit {
            break;
        }

        let Some(event) = events.next().await else {
            break;
        };

        match event {
            Event::Render => {
                terminal.draw(|f| ui::render(f, &mut app))?;
            }
            Event::Tick => {
                app.tick_count += 1;
                // Re-read FTDC files from disk to pick up new data
                let result = load_all_chunks(&path);
                let _ = writeln!(log, "tick:{} {}", app.tick_count, result.info);
                let _ = log.flush();
                if let Some((metrics, ts)) = latest_sample(&result.chunks) {
                    if let Some(epoch_ms) = ts {
                        app.sample_timestamp = format_sample_timestamp(epoch_ms);
                    }
                    app.update(Message::UpdateMetrics(metrics));
                }
            }
            Event::Key(key) => {
                // Only handle key press events (not release/repeat)
                if key.kind != KeyEventKind::Press {
                    continue;
                }

                // Ctrl+C always quits
                if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
                    app.update(Message::Quit);
                    continue;
                }

                match app.mode {
                    AppMode::Search => match key.code {
                        KeyCode::Esc => app.update(Message::ExitSearch),
                        KeyCode::Backspace => app.update(Message::SearchBackspace),
                        KeyCode::Enter => {
                            app.mode = AppMode::Normal;
                        }
                        KeyCode::Char(c) => app.update(Message::SearchInput(c)),
                        _ => {}
                    },
                    AppMode::Help => match key.code {
                        KeyCode::Esc | KeyCode::Char('?') | KeyCode::Char('q') => {
                            app.update(Message::ToggleHelp);
                        }
                        _ => {}
                    },
                    AppMode::Normal => match key.code {
                        KeyCode::Char('q') => app.update(Message::Quit),
                        KeyCode::Char('j') | KeyCode::Down => app.update(Message::MoveDown),
                        KeyCode::Char('k') | KeyCode::Up => app.update(Message::MoveUp),
                        KeyCode::Char('g') | KeyCode::Home => app.update(Message::JumpTop),
                        KeyCode::Char('G') | KeyCode::End => app.update(Message::JumpBottom),
                        KeyCode::Char('p') => app.update(Message::TogglePin),
                        KeyCode::Char('/') => app.update(Message::EnterSearch),
                        KeyCode::Char('?') => app.update(Message::ToggleHelp),
                        KeyCode::Tab => app.update(Message::ToggleFocus),
                        KeyCode::Esc => {
                            if !app.filter.is_empty() {
                                app.update(Message::ExitSearch);
                            }
                        }
                        _ => {}
                    },
                }
            }
            Event::Resize(_, _) => {
                // Terminal will auto-redraw on next render tick
            }
            Event::Mouse(_) => {}
        }
    }

    // Restore terminal
    terminal::disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    Ok(())
}
