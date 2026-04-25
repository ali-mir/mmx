use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::app::{App, ConnectionState};
use crate::theme;

fn now_local() -> String {
    unsafe {
        let mut tv: libc::timeval = std::mem::zeroed();
        libc::gettimeofday(&mut tv, std::ptr::null_mut());
        let mut tm: libc::tm = std::mem::zeroed();
        libc::localtime_r(&tv.tv_sec, &mut tm);
        format!("{:02}:{:02}:{:02}", tm.tm_hour, tm.tm_min, tm.tm_sec)
    }
}

pub fn render(f: &mut Frame, area: Rect, app: &App) {
    let label = if app.source_label.len() > 50 {
        format!("...{}", &app.source_label[app.source_label.len() - 47..])
    } else {
        app.source_label.clone()
    };

    let metric_count = app.metrics.len();
    let time_str = now_local();

    let mut spans = vec![
        Span::styled(" mmx ", theme::HEADER_BOLD),
        Span::styled(format!("│ {label} "), theme::HEADER_STYLE),
    ];

    if let Some(host) = &app.host {
        spans.push(Span::styled(format!("│ {host} "), theme::HEADER_STYLE));
    }
    if let Some(version) = &app.version {
        spans.push(Span::styled(format!("v{version} "), theme::HEADER_STYLE));
    }

    spans.push(Span::styled(
        format!("│ {metric_count} metrics │ polls {} ", app.poll_count),
        theme::HEADER_STYLE,
    ));

    let age = app
        .last_poll
        .map(|t| t.elapsed().as_secs())
        .map(|s| format!("│ last {s}s ago "))
        .unwrap_or_else(|| "│ no data ".into());
    spans.push(Span::styled(age, theme::HEADER_STYLE));

    spans.push(Span::styled(format!("│ {time_str} "), theme::HEADER_STYLE));
    spans.push(Span::styled(
        format!("│ window {} ", app.window.label()),
        theme::HEADER_STYLE,
    ));

    if app.paused {
        spans.push(Span::styled(
            "│ ⏸ PAUSED ".to_string(),
            Style::new()
                .fg(Color::Yellow)
                .bg(Color::Blue)
                .add_modifier(Modifier::BOLD),
        ));
    }

    spans.push(connection_indicator(&app.connection));

    let paragraph = Paragraph::new(Line::from(spans)).style(theme::HEADER_STYLE);
    f.render_widget(paragraph, area);
}

fn connection_indicator(state: &ConnectionState) -> Span<'static> {
    let bold = Modifier::BOLD;
    match state {
        ConnectionState::Connected => Span::styled(
            "│ ● connected ".to_string(),
            Style::new()
                .fg(Color::Green)
                .bg(Color::Blue)
                .add_modifier(bold),
        ),
        ConnectionState::Reconnecting(reason) => Span::styled(
            format!("│ ● reconnecting ({reason}) "),
            Style::new()
                .fg(Color::Yellow)
                .bg(Color::Blue)
                .add_modifier(bold),
        ),
        ConnectionState::Failed(reason) => Span::styled(
            format!("│ ● failed ({reason}) "),
            Style::new()
                .fg(Color::Red)
                .bg(Color::Blue)
                .add_modifier(bold),
        ),
    }
}
