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

    /// Dump diagnostic info and exit (no TUI)
    #[arg(long)]
    dump: bool,
}

/// Extract the latest sample from all FTDC chunks as MetricEntry list.
/// Returns (metrics, timestamp_epoch_ms, sample_count).
fn latest_sample(chunks: &[DecodedChunk]) -> Option<(Vec<MetricEntry>, Option<i64>, usize)> {
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

    Some((metrics, timestamp, num_samples))
}

/// Diagnostic info from a load attempt.
struct LoadResult {
    chunks: Vec<DecodedChunk>,
    info: String,
}

/// Summary of samples in chunks.
fn chunk_summary(chunks: &[DecodedChunk]) -> String {
    let total_samples: usize = chunks
        .iter()
        .map(|c| c.metrics.first().map_or(0, |m| m.values.len()))
        .sum();
    let n = chunks.len();
    format!("{n}ch/{total_samples}s")
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
                    let summary = chunk_summary(&chunks);
                    parts.push(format!("interim: {summary}@try{attempt}"));
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
            if !chunks.is_empty() {
                let summary = chunk_summary(&chunks);
                parts.push(format!("{name}: {summary}"));
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
    if let Some((metrics, ts, sample_count)) = latest_sample(&result.chunks) {
        if let Some(epoch_ms) = ts {
            app.sample_epoch_ms = Some(epoch_ms);
        }
        app.sample_count = sample_count;
        app.update(Message::UpdateMetrics(metrics));
    }
    log.flush()?;

    // Diagnostic dump mode: read twice with a delay, show what changed
    if cli.dump {
        // Show metric indices for key metrics
        let key_names = [
            "start",
            "serverStatus.opcounters.insert",
            "serverStatus.opcounters.query",
            "serverStatus.opcounters.command",
            "serverStatus.uptimeMillis",
            "serverStatus.connections.current",
        ];
        if let Some(last_chunk) = result.chunks.last() {
            eprintln!("=== Metric indices (of {}) ===", last_chunk.metrics.len());
            for name in &key_names {
                if let Some(idx) = last_chunk.metrics.iter().position(|m| m.path == *name) {
                    let m = &last_chunk.metrics[idx];
                    let has_nonzero = m.values.windows(2).any(|w| w[0] != w[1]);
                    eprintln!(
                        "  [{idx}] {name} ref={} last={} changes={}",
                        m.values[0],
                        m.values[m.values.len() - 1],
                        has_nonzero
                    );
                }
            }
            // Show first few values of key metrics to verify delta alignment
            for idx in [0, 1, 769, 1972] {
                if idx < last_chunk.metrics.len() {
                    let m = &last_chunk.metrics[idx];
                    let first5: Vec<i64> = m.values.iter().take(5).copied().collect();
                    let last3: Vec<i64> = m.values.iter().rev().take(3).rev().copied().collect();
                    eprintln!("  [{idx}] {} first5={first5:?} last3={last3:?}", m.path);
                }
            }
        }
        eprintln!(
            "=== Read 1: {} metrics, {} samples ===",
            app.metrics.len(),
            app.sample_count
        );
        for name in key_names {
            if let Some(m) = app.metrics.iter().find(|m| m.path == name) {
                eprintln!("  {name} = {} (delta={:?})", m.current, m.delta());
            }
        }
        eprintln!("Waiting 12 seconds for interim file update...");
        std::thread::sleep(Duration::from_secs(12));
        let result2 = load_all_chunks(&path);
        if let Some((metrics2, _, sc2)) = latest_sample(&result2.chunks) {
            eprintln!(
                "=== Read 2: {} metrics, {} samples ===",
                metrics2.len(),
                sc2
            );
            for name in [
                "start",
                "serverStatus.opcounters.insert",
                "serverStatus.opcounters.query",
                "serverStatus.opcounters.command",
                "serverStatus.uptimeMillis",
                "serverStatus.connections.current",
            ] {
                if let Some(m) = metrics2.iter().find(|m| m.path == name) {
                    let old = app.metrics.iter().find(|o| o.path == name);
                    let changed = old.is_some_and(|o| o.current != m.current);
                    eprintln!(
                        "  {name} = {} (delta={:?}) {}",
                        m.current,
                        m.previous.map(|p| m.current.wrapping_sub(p)),
                        if changed { "CHANGED" } else { "" }
                    );
                }
            }
        } else {
            eprintln!("No data on second read!");
        }
        return Ok(());
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
                let mut tick_log = format!("tick:{} {}", app.tick_count, result.info);
                if let Some((metrics, ts, sample_count)) = latest_sample(&result.chunks) {
                    if let Some(epoch_ms) = ts {
                        tick_log.push_str(&format!(" | ts={epoch_ms}"));
                        app.sample_epoch_ms = Some(epoch_ms);
                    }
                    app.sample_count = sample_count;
                    app.update(Message::UpdateMetrics(metrics));
                } else {
                    tick_log.push_str(" | no-sample");
                }
                let _ = writeln!(log, "{tick_log}");
                let _ = log.flush();
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
