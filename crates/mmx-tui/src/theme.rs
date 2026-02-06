#![allow(dead_code)]

use ratatui::style::{Color, Modifier, Style};

// Header bar
pub const HEADER_STYLE: Style = Style::new().fg(Color::White).bg(Color::Blue);
pub const HEADER_BOLD: Style = Style::new()
    .fg(Color::White)
    .bg(Color::Blue)
    .add_modifier(Modifier::BOLD);

// Footer/keybinding bar
pub const FOOTER_STYLE: Style = Style::new().fg(Color::Black).bg(Color::Cyan);
pub const FOOTER_KEY: Style = Style::new()
    .fg(Color::Black)
    .bg(Color::Cyan)
    .add_modifier(Modifier::BOLD);

// Selected row
pub const SELECTED_STYLE: Style = Style::new()
    .fg(Color::White)
    .bg(Color::DarkGray)
    .add_modifier(Modifier::BOLD);

// Metric paths
pub const METRIC_PATH_STYLE: Style = Style::new().fg(Color::Cyan);
pub const METRIC_PINNED_MARKER: Style = Style::new().fg(Color::Yellow).add_modifier(Modifier::BOLD);

// Values
pub const VALUE_NORMAL: Style = Style::new().fg(Color::White);
pub const VALUE_LOW: Style = Style::new().fg(Color::Green);
pub const VALUE_MID: Style = Style::new().fg(Color::Yellow);
pub const VALUE_HIGH: Style = Style::new().fg(Color::Red);

// Delta
pub const DELTA_POSITIVE: Style = Style::new().fg(Color::Green);
pub const DELTA_NEGATIVE: Style = Style::new().fg(Color::Red);
pub const DELTA_ZERO: Style = Style::new().fg(Color::DarkGray);

// Pinned section border
pub const PINNED_BORDER: Style = Style::new().fg(Color::Cyan);

// Search
pub const SEARCH_STYLE: Style = Style::new().fg(Color::Yellow);
pub const SEARCH_MATCH: Style = Style::new()
    .fg(Color::Black)
    .bg(Color::Yellow)
    .add_modifier(Modifier::BOLD);

// Help overlay
pub const HELP_OVERLAY_BG: Style = Style::new().fg(Color::White).bg(Color::DarkGray);
pub const HELP_KEY_STYLE: Style = Style::new()
    .fg(Color::Cyan)
    .bg(Color::DarkGray)
    .add_modifier(Modifier::BOLD);
pub const HELP_DESC_STYLE: Style = Style::new().fg(Color::White).bg(Color::DarkGray);
