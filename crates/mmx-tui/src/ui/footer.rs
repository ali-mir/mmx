use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::app::{App, AppMode};
use crate::theme;

pub fn render(f: &mut Frame, area: Rect, app: &App) {
    let line = match app.mode {
        AppMode::Search => {
            let filter_display = if app.filter.is_empty() {
                String::new()
            } else {
                app.filter.clone()
            };
            Line::from(vec![
                Span::styled(" /", theme::FOOTER_KEY),
                Span::styled(format!("{filter_display}_ "), theme::SEARCH_STYLE),
                Span::styled(" Esc", theme::FOOTER_KEY),
                Span::styled(":clear ", theme::FOOTER_STYLE),
            ])
        }
        _ => Line::from(vec![
            Span::styled(" q", theme::FOOTER_KEY),
            Span::styled(":quit ", theme::FOOTER_STYLE),
            Span::styled("j/k", theme::FOOTER_KEY),
            Span::styled(":nav ", theme::FOOTER_STYLE),
            Span::styled("p", theme::FOOTER_KEY),
            Span::styled(":pin ", theme::FOOTER_STYLE),
            Span::styled("/", theme::FOOTER_KEY),
            Span::styled(":search ", theme::FOOTER_STYLE),
            Span::styled("Tab", theme::FOOTER_KEY),
            Span::styled(":focus ", theme::FOOTER_STYLE),
            Span::styled("?", theme::FOOTER_KEY),
            Span::styled(":help ", theme::FOOTER_STYLE),
        ]),
    };

    let paragraph = Paragraph::new(line).style(theme::FOOTER_STYLE);
    f.render_widget(paragraph, area);
}
