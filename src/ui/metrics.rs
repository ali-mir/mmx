use ratatui::Frame;
use ratatui::layout::{Constraint, Rect};
use ratatui::style::Stylize;
use ratatui::text::Span;
use ratatui::widgets::{
    Block, Borders, Cell, Row, Scrollbar, ScrollbarOrientation, ScrollbarState, Table,
};

use crate::app::{App, Focus, MetricEntry};
use crate::format::{format_rate, format_value};
use crate::metric::MetricKind;
use crate::theme;

pub fn render(f: &mut Frame, area: Rect, app: &mut App) {
    let filtered = app.filtered_metrics();
    let is_focused = app.focus == Focus::Main;

    let rows: Vec<Row> = filtered
        .iter()
        .map(|m| {
            let pin_marker = if app.is_pinned(&m.path) { "* " } else { "  " };
            let path_cell = Cell::from(Span::styled(
                format!("{pin_marker}{}", m.path),
                theme::METRIC_PATH_STYLE,
            ));
            let value_cell = Cell::from(Span::styled(
                format_value(&m.path, m.current),
                theme::VALUE_NORMAL,
            ));
            Row::new(vec![path_cell, value_cell, rate_cell(m)])
        })
        .collect();

    let row_count = rows.len();

    let border_style = if is_focused {
        ratatui::style::Style::default().fg(ratatui::style::Color::Cyan)
    } else {
        ratatui::style::Style::default().fg(ratatui::style::Color::DarkGray)
    };

    let title = if app.filter.is_empty() {
        format!(" Metrics ({row_count}) ")
    } else {
        format!(" Metrics ({row_count} matching) ")
    };

    let table = Table::new(
        rows,
        [
            Constraint::Percentage(60),
            Constraint::Percentage(20),
            Constraint::Percentage(20),
        ],
    )
    .header(
        Row::new(vec![
            Cell::from("Metric Path".bold()),
            Cell::from("Value".bold()),
            Cell::from("Rate".bold()),
        ])
        .style(theme::HEADER_STYLE),
    )
    .block(
        Block::default()
            .borders(Borders::ALL)
            .title(title)
            .border_style(border_style),
    )
    .row_highlight_style(theme::SELECTED_STYLE);

    f.render_stateful_widget(table, area, &mut app.table_state);

    if row_count > 0 {
        let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight);
        let mut scrollbar_state =
            ScrollbarState::new(row_count).position(app.table_state.selected().unwrap_or(0));
        f.render_stateful_widget(scrollbar, area, &mut scrollbar_state);
    }
}

/// Rate column. Counters show a per-second rate; gauges show a one-tick delta
/// (since rates of instantaneous values are nonsensical).
pub(crate) fn rate_cell(m: &MetricEntry) -> Cell<'static> {
    match m.kind {
        MetricKind::Counter => match m.rate_per_sec() {
            Some(r) if r >= 0.05 => Cell::from(Span::styled(
                format!("+{}", format_rate(&m.path, r)),
                theme::DELTA_POSITIVE,
            )),
            Some(_) => Cell::from(Span::styled("0/s", theme::DELTA_ZERO)),
            None => Cell::from(Span::styled("—", theme::DELTA_ZERO)),
        },
        MetricKind::Gauge => match m.delta() {
            Some(d) if d > 0 => Cell::from(Span::styled(
                format!("+{}", format_value(&m.path, d)),
                theme::DELTA_POSITIVE,
            )),
            Some(d) if d < 0 => Cell::from(Span::styled(
                format_value(&m.path, d),
                theme::DELTA_NEGATIVE,
            )),
            _ => Cell::from(Span::styled("—", theme::DELTA_ZERO)),
        },
    }
}
