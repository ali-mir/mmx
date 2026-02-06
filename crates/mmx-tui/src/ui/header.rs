use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::app::App;
use crate::theme;

pub fn render(f: &mut Frame, area: Rect, app: &App) {
    let file_display = if app.file_path.len() > 60 {
        format!("...{}", &app.file_path[app.file_path.len() - 57..])
    } else {
        app.file_path.clone()
    };

    let metric_count = app.metrics.len();

    let mut spans = vec![
        Span::styled(" mmx ", theme::HEADER_BOLD),
        Span::styled(
            format!("| {file_display} | {metric_count} metrics "),
            theme::HEADER_STYLE,
        ),
    ];

    if !app.sample_timestamp.is_empty() {
        spans.push(Span::styled(
            format!("| {} ", app.sample_timestamp),
            theme::HEADER_STYLE,
        ));
    }

    spans.push(Span::styled(
        format!("| tick:{} ", app.tick_count),
        theme::HEADER_STYLE,
    ));

    let paragraph = Paragraph::new(Line::from(spans)).style(theme::HEADER_STYLE);
    f.render_widget(paragraph, area);
}
