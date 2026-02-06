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
    let chunk_info = format!("{metric_count} metrics | {} chunks", app.total_chunks);
    let sample_info = if app.sample_time.is_empty() {
        String::new()
    } else {
        format!(" | {}", app.sample_time)
    };

    let line = Line::from(vec![
        Span::styled(" mmx ", theme::HEADER_BOLD),
        Span::styled(format!("| {file_display} "), theme::HEADER_STYLE),
        Span::styled(format!("| {chunk_info}{sample_info} "), theme::HEADER_STYLE),
    ]);

    let paragraph = Paragraph::new(line).style(theme::HEADER_STYLE);
    f.render_widget(paragraph, area);
}
