use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::app::App;
use crate::theme;

/// Get current local time as a formatted string and epoch_ms.
fn now_local() -> (String, i64) {
    unsafe {
        let mut tv: libc::timeval = std::mem::zeroed();
        libc::gettimeofday(&mut tv, std::ptr::null_mut());
        let epoch_ms = tv.tv_sec as i64 * 1000 + tv.tv_usec as i64 / 1000;

        let mut tm: libc::tm = std::mem::zeroed();
        libc::localtime_r(&tv.tv_sec, &mut tm);

        let formatted = format!("{:02}:{:02}:{:02}", tm.tm_hour, tm.tm_min, tm.tm_sec);
        (formatted, epoch_ms)
    }
}

pub fn render(f: &mut Frame, area: Rect, app: &App) {
    let file_display = if app.file_path.len() > 60 {
        format!("...{}", &app.file_path[app.file_path.len() - 57..])
    } else {
        app.file_path.clone()
    };

    let metric_count = app.metrics.len();
    let (time_str, now_ms) = now_local();

    let mut spans = vec![
        Span::styled(" mmx ", theme::HEADER_BOLD),
        Span::styled(
            format!("| {file_display} | {metric_count} metrics | {time_str} "),
            theme::HEADER_STYLE,
        ),
    ];

    if let Some(epoch_ms) = app.sample_epoch_ms {
        let age_secs = (now_ms - epoch_ms) / 1000;
        let samples = app.sample_count;
        spans.push(Span::styled(
            format!("| {samples} samples | data: {age_secs}s ago "),
            theme::HEADER_STYLE,
        ));
    }

    let paragraph = Paragraph::new(Line::from(spans)).style(theme::HEADER_STYLE);
    f.render_widget(paragraph, area);
}
