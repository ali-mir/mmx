pub mod chart;
pub mod footer;
pub mod header;
pub mod metrics;
pub mod pinned;

use std::time::Instant;

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};

use crate::app::{App, AppMode};
use crate::theme;

pub fn render(f: &mut Frame, app: &mut App) {
    let has_pinned = !app.pinned.is_empty();
    // Anchor chart "now" to the last successful poll, not the wall clock —
    // otherwise already-plotted points drift leftward every render tick (10 Hz)
    // instead of holding still and snapping over once per poll.
    let now = app.last_poll.unwrap_or_else(Instant::now);

    let mut constraints = vec![Constraint::Length(1)]; // header
    if has_pinned {
        let pinned_count = app.pinned_metrics().len();
        constraints.push(Constraint::Length((pinned_count as u16 + 3).min(12)));
    }
    // Charts: take ~60% of the remaining vertical space (user wants big charts).
    constraints.push(Constraint::Percentage(60)); // chart panel
    constraints.push(Constraint::Min(8)); // metric drawer
    constraints.push(Constraint::Length(1)); // footer

    let chunks = Layout::vertical(constraints).split(f.area());

    let mut idx = 0;
    header::render(f, chunks[idx], app);
    idx += 1;

    if has_pinned {
        pinned::render(f, chunks[idx], app);
        idx += 1;
    }

    chart::render_dashboard(
        f,
        chunks[idx],
        app,
        now,
        app.window.seconds(),
        app.expanded_panel,
    );
    idx += 1;

    metrics::render(f, chunks[idx], app);
    idx += 1;

    footer::render(f, chunks[idx], app);

    if app.mode == AppMode::Help {
        render_help_overlay(f);
    }
}

fn render_help_overlay(f: &mut Frame) {
    let area = f.area();
    let width = 50u16.min(area.width.saturating_sub(4));
    let height = 18u16.min(area.height.saturating_sub(4));
    let x = (area.width.saturating_sub(width)) / 2;
    let y = (area.height.saturating_sub(height)) / 2;
    let overlay_area = Rect::new(x, y, width, height);

    f.render_widget(Clear, overlay_area);

    let keybindings = vec![
        ("q / Ctrl+C", "Quit"),
        ("j / Down", "Move selection down"),
        ("k / Up", "Move selection up"),
        ("g / Home", "Jump to top"),
        ("G / End", "Jump to bottom"),
        ("p", "Pin/unpin selected metric"),
        ("/", "Enter search/filter mode"),
        ("Tab", "Switch focus (pinned/main)"),
        ("Space", "Pause/resume polling"),
        ("+ / -", "Cycle time window (1m/5m/15m)"),
        ("1 — 6", "Expand panel by index"),
        ("Esc", "Collapse / clear search / close help"),
        ("?", "Toggle this help"),
    ];

    let lines: Vec<Line> = keybindings
        .iter()
        .map(|(key, desc)| {
            Line::from(vec![
                Span::styled(format!("  {key:<16}"), theme::HELP_KEY_STYLE),
                Span::styled(*desc, theme::HELP_DESC_STYLE),
            ])
        })
        .collect();

    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Help ")
        .style(Style::default().fg(Color::White).bg(Color::DarkGray))
        .border_style(Style::default().fg(Color::Cyan).bg(Color::DarkGray));

    let paragraph = Paragraph::new(lines)
        .block(block)
        .wrap(Wrap { trim: false })
        .style(theme::HELP_OVERLAY_BG);

    f.render_widget(paragraph, overlay_area);
}
