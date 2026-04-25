mod app;
mod bson_ext;
mod event;
mod format;
mod metric;
mod source;
mod theme;
mod ui;

use std::io;
use std::time::Duration;

use clap::Parser;
use color_eyre::eyre::{Context, Result, eyre};
use crossterm::event::{KeyCode, KeyEventKind, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{self, EnterAlternateScreen, LeaveAlternateScreen};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;

use app::{App, AppMode, Message};
use event::{Event, EventHandler};
use source::{MetricSource, PollError, ReadPref, ServerStatusSource, SourceConfig};
use ui::chart::PANEL_COUNT;

#[derive(Parser, Debug)]
#[command(
    name = "mmx",
    about = "real-time terminal dashboard for MongoDB serverStatus metrics"
)]
struct Cli {
    /// MongoDB connection URI (e.g. mongodb://localhost:27017 or mongodb+srv://...).
    #[arg(long)]
    uri: String,

    /// Poll interval (e.g. 1s, 500ms).
    #[arg(long, default_value = "1s", value_parser = parse_duration)]
    interval: Duration,

    /// Read preference for the serverStatus command.
    #[arg(long, value_enum, default_value_t = ReadPref::Primary)]
    read_preference: ReadPref,

    /// Driver connect timeout.
    #[arg(long, default_value = "3s", value_parser = parse_duration)]
    connect_timeout: Duration,

    /// Driver server-selection timeout. Keep low so a missed tick fails fast.
    #[arg(long, default_value = "2s", value_parser = parse_duration)]
    server_selection_timeout: Duration,

    /// Application name reported to mongod.
    #[arg(long, default_value = "mmx")]
    app_name: String,

    /// Skip TLS certificate verification (insecure — local dev only).
    #[arg(long)]
    tls_allow_invalid_certs: bool,

    /// Connect, poll once, print a summary, and exit (no TUI).
    #[arg(long)]
    probe: bool,
}

fn parse_duration(s: &str) -> Result<Duration, String> {
    let s = s.trim();
    let (num, unit) = s
        .find(|c: char| c.is_alphabetic())
        .map(|i| s.split_at(i))
        .ok_or_else(|| format!("missing unit in '{s}' (expected ms/s/m)"))?;
    let n: u64 = num
        .parse()
        .map_err(|_| format!("invalid number in '{s}'"))?;
    match unit {
        "ms" => Ok(Duration::from_millis(n)),
        "s" => Ok(Duration::from_secs(n)),
        "m" => Ok(Duration::from_secs(n * 60)),
        other => Err(format!("unknown unit '{other}' (expected ms/s/m)")),
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    color_eyre::install()?;
    let cli = Cli::parse();

    // Connect up front so we fail fast on bad URIs / unreachable hosts.
    let cfg = SourceConfig {
        uri: cli.uri.clone(),
        app_name: cli.app_name.clone(),
        connect_timeout: cli.connect_timeout,
        server_selection_timeout: cli.server_selection_timeout,
        read_pref: cli.read_preference,
        tls_allow_invalid_certs: cli.tls_allow_invalid_certs,
    };
    let mut source = ServerStatusSource::connect(cfg)
        .await
        .map_err(|e| eyre!("failed to initialize MongoDB client: {e}"))?;
    let label = source.label.clone();

    if cli.probe {
        return probe(&mut source).await;
    }

    let mut app = App::new(label);

    terminal::enable_raw_mode().context("enable raw mode")?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    terminal.clear()?;

    // Restore terminal on panic so backtraces aren't eaten by raw mode.
    let original_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic| {
        let _ = terminal::disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen);
        original_hook(panic);
    }));

    let mut events = EventHandler::new(Duration::from_secs(1), Duration::from_millis(100));
    let poll_tx = events.sender();
    tokio::spawn(poll_loop(source, poll_tx, cli.interval));

    let result = run_event_loop(&mut terminal, &mut app, &mut events).await;

    terminal::disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    result
}

async fn probe(source: &mut ServerStatusSource) -> Result<()> {
    let sample = source.poll().await.map_err(|e| eyre!("poll failed: {e}"))?;
    println!("connected: {}", source.label);
    if let Some(host) = &sample.host {
        println!("host:      {host}");
    }
    if let Some(version) = &sample.version {
        println!("version:   {version}");
    }
    println!("metrics:   {}", sample.metrics.len());
    let probes = [
        "uptimeMillis",
        "connections.current",
        "opcounters.insert",
        "opcounters.query",
        "opcounters.command",
        "network.bytesIn",
        "network.bytesOut",
        "wiredTiger.cache.bytes currently in the cache",
    ];
    for path in probes {
        if let Some(m) = sample.metrics.iter().find(|m| m.path == path) {
            println!("  {path} = {}", m.value);
        }
    }
    Ok(())
}

async fn poll_loop(
    mut source: ServerStatusSource,
    tx: tokio::sync::mpsc::UnboundedSender<Event>,
    interval: Duration,
) {
    let mut ticker = tokio::time::interval(interval);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        ticker.tick().await;
        match source.poll().await {
            Ok(sample) => {
                if tx.send(Event::Sample(sample)).is_err() {
                    return;
                }
            }
            Err(PollError::Transient(msg)) => {
                if tx.send(Event::PollFailed(msg)).is_err() {
                    return;
                }
            }
            Err(PollError::Fatal(msg)) => {
                let _ = tx.send(Event::PollFatal(msg));
                return;
            }
        }
    }
}

async fn run_event_loop<B: ratatui::backend::Backend>(
    terminal: &mut Terminal<B>,
    app: &mut App,
    events: &mut EventHandler,
) -> Result<()> {
    loop {
        if app.should_quit {
            return Ok(());
        }

        let Some(event) = events.next().await else {
            return Ok(());
        };

        match event {
            Event::Render => {
                terminal.draw(|f| ui::render(f, app))?;
            }
            Event::Tick => {
                app.tick_count += 1;
            }
            Event::Sample(sample) => {
                app.update(Message::SampleArrived(sample));
            }
            Event::PollFailed(reason) => {
                app.update(Message::PollFailed(reason));
            }
            Event::PollFatal(reason) => {
                app.update(Message::PollFatal(reason));
            }
            Event::Key(key) => {
                if key.kind != KeyEventKind::Press {
                    continue;
                }
                if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
                    app.update(Message::Quit);
                    continue;
                }
                handle_key(app, key.code);
            }
            Event::Resize(_, _) | Event::Mouse(_) => {}
        }
    }
}

fn handle_key(app: &mut App, code: KeyCode) {
    match app.mode {
        AppMode::Search => match code {
            KeyCode::Esc => app.update(Message::ExitSearch),
            KeyCode::Backspace => app.update(Message::SearchBackspace),
            KeyCode::Enter => app.mode = AppMode::Normal,
            KeyCode::Char(c) => app.update(Message::SearchInput(c)),
            _ => {}
        },
        AppMode::Help => match code {
            KeyCode::Esc | KeyCode::Char('?') | KeyCode::Char('q') => {
                app.update(Message::ToggleHelp);
            }
            _ => {}
        },
        AppMode::Normal => match code {
            KeyCode::Char('q') => app.update(Message::Quit),
            KeyCode::Char('j') | KeyCode::Down => app.update(Message::MoveDown),
            KeyCode::Char('k') | KeyCode::Up => app.update(Message::MoveUp),
            KeyCode::Char('g') | KeyCode::Home => app.update(Message::JumpTop),
            KeyCode::Char('G') | KeyCode::End => app.update(Message::JumpBottom),
            KeyCode::Char('p') => app.update(Message::TogglePin),
            KeyCode::Char('/') => app.update(Message::EnterSearch),
            KeyCode::Char('?') => app.update(Message::ToggleHelp),
            KeyCode::Tab => app.update(Message::ToggleFocus),
            KeyCode::Char(' ') => app.update(Message::TogglePause),
            KeyCode::Char('+') | KeyCode::Char('=') => app.update(Message::NextWindow),
            KeyCode::Char('-') | KeyCode::Char('_') => app.update(Message::PrevWindow),
            KeyCode::Char(c) if c.is_ascii_digit() && c != '0' => {
                let idx = (c as usize) - ('1' as usize);
                if idx < PANEL_COUNT {
                    app.update(Message::ExpandPanel(idx));
                }
            }
            KeyCode::Esc => {
                if app.expanded_panel.is_some() {
                    app.update(Message::CollapsePanel);
                } else if !app.filter.is_empty() {
                    app.update(Message::ExitSearch);
                }
            }
            _ => {}
        },
    }
}
