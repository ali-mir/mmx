mod app;
mod event;
mod format;
mod theme;
mod ui;

use std::io;
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

/// Flattened timeline: one entry per sample across all chunks.
/// Each entry is a Vec of (path, value) for every metric at that point in time.
struct Timeline {
    /// Metric paths (same order for every sample).
    paths: Vec<String>,
    /// samples[i][j] = value of metric j at sample i.
    samples: Vec<Vec<i64>>,
    /// Current playback position.
    cursor: usize,
}

impl Timeline {
    fn from_chunks(chunks: &[DecodedChunk]) -> Self {
        if chunks.is_empty() {
            return Timeline {
                paths: Vec::new(),
                samples: Vec::new(),
                cursor: 0,
            };
        }

        // Use the first chunk's metrics as the canonical path list
        let paths: Vec<String> = chunks
            .last()
            .unwrap()
            .metrics
            .iter()
            .map(|m| m.path.clone())
            .collect();

        let mut samples: Vec<Vec<i64>> = Vec::new();

        for chunk in chunks {
            if chunk.metrics.is_empty() {
                continue;
            }
            let num_samples = chunk.metrics[0].values.len();

            for sample_idx in 0..num_samples {
                let sample: Vec<i64> = chunk
                    .metrics
                    .iter()
                    .map(|m| m.values.get(sample_idx).copied().unwrap_or(0))
                    .collect();
                samples.push(sample);
            }
        }

        Timeline {
            paths,
            samples,
            cursor: 0,
        }
    }

    fn total_samples(&self) -> usize {
        self.samples.len()
    }

    /// Get the current sample as MetricEntry list, then advance the cursor.
    fn next_sample(&mut self) -> Option<Vec<MetricEntry>> {
        if self.samples.is_empty() {
            return None;
        }

        let current_idx = self.cursor;
        let prev_idx = current_idx.checked_sub(1);

        let current_values = &self.samples[current_idx];
        let prev_values = prev_idx.map(|i| &self.samples[i]);

        let metrics = self
            .paths
            .iter()
            .enumerate()
            .map(|(j, path)| {
                let current = current_values[j];
                let previous = prev_values.map(|pv| pv[j]);
                MetricEntry {
                    path: path.clone(),
                    current,
                    previous,
                    history: Vec::new(), // History tracked by App
                }
            })
            .collect();

        // Advance cursor, wrapping to start for replay
        self.cursor = (self.cursor + 1) % self.samples.len();

        Some(metrics)
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

    // Load all FTDC chunks
    let all_chunks = load_all_chunks(&path)?;
    let total_chunks = all_chunks.len();

    let mut timeline = Timeline::from_chunks(&all_chunks);
    let total_samples = timeline.total_samples();

    let mut app = App::new(path.display().to_string());
    app.total_chunks = total_chunks;
    app.sample_time = format!("sample 1/{total_samples}");

    // Load first sample
    if let Some(metrics) = timeline.next_sample() {
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
                // Advance to next sample in the timeline
                if let Some(metrics) = timeline.next_sample() {
                    app.sample_time = format!("sample {}/{total_samples}", timeline.cursor);
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

fn load_all_chunks(path: &std::path::Path) -> color_eyre::Result<Vec<DecodedChunk>> {
    let files = reader::find_ftdc_files(path)?;
    let mut all_chunks: Vec<DecodedChunk> = Vec::new();

    for file_path in &files {
        let file = std::fs::File::open(file_path)?;
        let reader = std::io::BufReader::new(file);
        match reader::read_ftdc_file(reader) {
            Ok(chunks) => all_chunks.extend(chunks),
            Err(_) => continue,
        }
    }

    Ok(all_chunks)
}
