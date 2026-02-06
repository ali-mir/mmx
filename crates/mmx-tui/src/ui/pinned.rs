use ratatui::Frame;
use ratatui::layout::{Constraint, Rect};
use ratatui::style::Stylize;
use ratatui::text::Span;
use ratatui::widgets::{Block, Borders, Cell, Row, Table};

use crate::app::{App, Focus};
use crate::format::format_value;
use crate::theme;

pub fn render(f: &mut Frame, area: Rect, app: &mut App) {
    let is_focused = app.focus == Focus::Pinned;

    // Collect pinned metric data before borrowing table_state mutably
    let pinned_data: Vec<(String, i64, Option<i64>)> = app
        .pinned_metrics()
        .iter()
        .map(|m| (m.path.clone(), m.current, m.delta()))
        .collect();

    let rows: Vec<Row> = pinned_data
        .iter()
        .map(|(path, current, delta)| {
            let path_cell = Cell::from(Span::styled(path.as_str(), theme::METRIC_PATH_STYLE));

            let value_cell = Cell::from(Span::styled(
                format_value(path, *current),
                theme::VALUE_NORMAL,
            ));

            let delta_cell = match delta {
                Some(d) if *d > 0 => Cell::from(Span::styled(
                    format!("+{}", format_value(path, *d)),
                    theme::DELTA_POSITIVE,
                )),
                Some(d) if *d < 0 => {
                    Cell::from(Span::styled(format_value(path, *d), theme::DELTA_NEGATIVE))
                }
                Some(_) => Cell::from(Span::styled("0", theme::DELTA_ZERO)),
                None => Cell::from(Span::styled("-", theme::DELTA_ZERO)),
            };

            Row::new(vec![path_cell, value_cell, delta_cell])
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
            Cell::from("Delta".bold()),
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
