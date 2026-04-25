use ratatui::Frame;
use ratatui::layout::{Constraint, Rect};
use ratatui::style::Stylize;
use ratatui::text::Span;
use ratatui::widgets::{Block, Borders, Cell, Row, Table};

use crate::app::{App, Focus};
use crate::format::format_value;
use crate::theme;
use crate::ui::metrics::rate_cell;

pub fn render(f: &mut Frame, area: Rect, app: &mut App) {
    let is_focused = app.focus == Focus::Pinned;

    // Clone pinned entries up front so we can borrow `app` mutably for table state below.
    let pinned: Vec<crate::app::MetricEntry> =
        app.pinned_metrics().iter().map(|m| (*m).clone()).collect();

    let rows: Vec<Row> = pinned
        .iter()
        .map(|m| {
            let path_cell = Cell::from(Span::styled(m.path.clone(), theme::METRIC_PATH_STYLE));
            let value_cell = Cell::from(Span::styled(
                format_value(&m.path, m.current),
                theme::VALUE_NORMAL,
            ));
            Row::new(vec![path_cell, value_cell, rate_cell(m)])
        })
        .collect();

    let border_style = if is_focused {
        theme::PINNED_BORDER
    } else {
        ratatui::style::Style::default().fg(ratatui::style::Color::DarkGray)
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
            Cell::from("Pinned Metric".bold()),
            Cell::from("Value".bold()),
            Cell::from("Rate".bold()),
        ])
        .style(theme::PINNED_BORDER),
    )
    .block(
        Block::default()
            .borders(Borders::ALL)
            .title(" Pinned ")
            .border_style(border_style),
    )
    .row_highlight_style(theme::SELECTED_STYLE);

    f.render_stateful_widget(table, area, &mut app.pinned_table_state);
}
