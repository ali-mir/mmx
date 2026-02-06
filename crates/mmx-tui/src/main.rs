mod app;
mod event;
mod format;
mod theme;
mod ui;

use std::io;
use std::path::PathBuf;
use std::sync::Arc;
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
    about = "MongoMetrics - htop-like TUI for MongoDB FTDC data"
)]
struct Cli {
    /// Path to FTDC file or directory containing FTDC files
    path: PathBuf,
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

    // Initial load of FTDC data
    let ftdc_path = path.clone();
    let (initial_metrics, total_chunks) = load_ftdc_data(&ftdc_path)?;

    let mut app = App::new(path.display().to_string());
    app.total_chunks = total_chunks;
    app.update(Message::UpdateMetrics(initial_metrics));

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

    // Shared path for the file watcher reload
    let shared_path = Arc::new(ftdc_path);

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
                // Reload FTDC data on tick
                let path = Arc::clone(&shared_path);
                match load_ftdc_data(&path) {
                    Ok((metrics, chunks)) => {
                        app.total_chunks = chunks;
                        app.update(Message::UpdateMetrics(metrics));
                    }
                    Err(_) => {
                        // Silently ignore reload errors (file may be rotating)
                    }
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

fn load_ftdc_data(path: &std::path::Path) -> color_eyre::Result<(Vec<MetricEntry>, usize)> {
    let files = reader::find_ftdc_files(path)?;
    let mut all_chunks: Vec<DecodedChunk> = Vec::new();

    for file_path in &files {
        let file = std::fs::File::open(file_path)?;
        let reader = std::io::BufReader::new(file);
        match reader::read_ftdc_file(reader) {
            Ok(chunks) => all_chunks.extend(chunks),
            Err(_) => continue, // Skip files that can't be parsed
        }
    }

    let total_chunks = all_chunks.len();

    // Take the last chunk's values as current metrics
    let metrics = if let Some(last_chunk) = all_chunks.last() {
        last_chunk
            .metrics
            .iter()
            .map(|m| {
                let current = *m.values.last().unwrap_or(&0);
                let previous = if m.values.len() >= 2 {
                    Some(m.values[m.values.len() - 2])
                } else {
                    None
                };
                MetricEntry {
                    path: m.path.clone(),
                    current,
                    previous,
                    history: m.values.clone(),
                }
            })
            .collect()
    } else {
        Vec::new()
    };

    Ok((metrics, total_chunks))
}
