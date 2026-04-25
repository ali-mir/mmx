use std::collections::VecDeque;
use std::time::Instant;

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::symbols::Marker;
use ratatui::text::Span;
use ratatui::widgets::{Axis, Block, Borders, Chart, Dataset, GraphType};

use crate::app::App;
use crate::format;
use crate::metric::TimedValue;

/// Number of panels in the default dashboard. Used by the input layer for
/// `1`..`N` panel-expansion shortcuts.
pub const PANEL_COUNT: usize = 6;

#[derive(Debug, Clone, Copy)]
pub enum SeriesKind {
    /// Plot per-second rate (counter delta over time).
    Rate,
    /// Plot raw value over time (gauge).
    Value,
}

/// Static description of one series in a panel (one curve in the chart).
#[derive(Debug, Clone, Copy)]
pub struct PanelSeries {
    pub path: &'static str,
    pub label: &'static str,
    pub color: Color,
    pub kind: SeriesKind,
}

/// How values on this panel should be formatted on the y-axis.
#[derive(Debug, Clone, Copy)]
pub enum YFormat {
    Number,
    Bytes,
    PerSecond,
    BytesPerSecond,
}

/// Static description of a dashboard panel.
#[derive(Debug, Clone, Copy)]
pub struct Panel {
    pub title: &'static str,
    pub y_format: YFormat,
    pub series: &'static [PanelSeries],
}

/// One series with concrete plot points.
struct ResolvedSeries<'a> {
    label: &'static str,
    color: Color,
    points: Vec<(f64, f64)>,
    _marker: std::marker::PhantomData<&'a ()>,
}

fn render_panel_inner(
    f: &mut Frame,
    area: Rect,
    app: &App,
    now: Instant,
    panel: &Panel,
    window_secs: f64,
    title_suffix: &str,
) {
    let resolved: Vec<ResolvedSeries> = panel
        .series
        .iter()
        .map(|ps| {
            let history = app.metrics.iter().find(|m| m.path == ps.path);
            let points = match (history, ps.kind) {
                (Some(m), SeriesKind::Rate) => rate_points(&m.history, now, window_secs),
                (Some(m), SeriesKind::Value) => value_points(&m.history, now, window_secs),
                (None, _) => Vec::new(),
            };
            ResolvedSeries {
                label: ps.label,
                color: ps.color,
                points,
                _marker: std::marker::PhantomData,
            }
        })
        .collect();

    let max_y = resolved
        .iter()
        .flat_map(|s| s.points.iter().map(|(_, y)| *y))
        .fold(0.0_f64, f64::max);
    let max_y = (max_y * 1.15).max(1.0);

    let datasets: Vec<Dataset<'_>> = resolved
        .iter()
        .map(|s| {
            Dataset::default()
                .name(s.label)
                .marker(Marker::Braille)
                .graph_type(GraphType::Line)
                .style(Style::default().fg(s.color))
                .data(&s.points)
        })
        .collect();

    let axis_style = Style::default().fg(Color::DarkGray);

    let x_axis = Axis::default()
        .style(axis_style)
        .bounds([-window_secs, 0.0])
        .labels(window_labels(window_secs));

    let y_axis = Axis::default()
        .style(axis_style)
        .bounds([0.0, max_y])
        .labels(vec![
            Span::raw(format_y(0.0, panel.y_format)),
            Span::raw(format_y(max_y / 2.0, panel.y_format)),
            Span::raw(format_y(max_y, panel.y_format)),
        ]);

    // Build a multi-span title: panel name + an inline color-coded "legend".
    // The built-in ratatui legend renders inside the plot area and the curves
    // draw through it; putting it in the border title avoids that entirely.
    let mut title_spans: Vec<Span<'static>> = vec![Span::styled(
        format!(" {}{} ", panel.title, title_suffix),
        Style::default()
            .fg(Color::White)
            .add_modifier(Modifier::BOLD),
    )];
    title_spans.push(Span::styled("│ ", Style::default().fg(Color::DarkGray)));
    for (i, s) in panel.series.iter().enumerate() {
        if i > 0 {
            title_spans.push(Span::raw(" "));
        }
        title_spans.push(Span::styled(
            s.label.to_string(),
            Style::default().fg(s.color).add_modifier(Modifier::BOLD),
        ));
    }
    title_spans.push(Span::raw(" "));

    let block = Block::default()
        .borders(Borders::ALL)
        .title(ratatui::text::Line::from(title_spans));

    let chart = Chart::new(datasets)
        .block(block)
        .x_axis(x_axis)
        .y_axis(y_axis)
        // Legend is rendered in the border title above; suppress the built-in.
        .legend_position(None);

    f.render_widget(chart, area);
}

/// Render the default dashboard. If `expanded` is `Some(i)`, that panel is
/// drawn full-area instead of the 3×2 grid.
pub fn render_dashboard(
    f: &mut Frame,
    area: Rect,
    app: &App,
    now: Instant,
    window_secs: f64,
    expanded: Option<usize>,
) {
    let panels = default_panels();
    if let Some(i) = expanded {
        if let Some(panel) = panels.get(i) {
            render_panel_with_index(f, area, app, now, panel, window_secs, i, true);
            return;
        }
    }

    let rows = Layout::vertical([Constraint::Ratio(1, 2), Constraint::Ratio(1, 2)]).split(area);
    let cols_per_row = panels.len().div_ceil(2);
    for row_idx in 0..2 {
        let cells = Layout::horizontal(vec![
            Constraint::Ratio(1, cols_per_row as u32);
            cols_per_row
        ])
        .split(rows[row_idx]);
        for col_idx in 0..cols_per_row {
            let panel_idx = row_idx * cols_per_row + col_idx;
            if let Some(panel) = panels.get(panel_idx) {
                render_panel_with_index(
                    f,
                    cells[col_idx],
                    app,
                    now,
                    panel,
                    window_secs,
                    panel_idx,
                    false,
                );
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn render_panel_with_index(
    f: &mut Frame,
    area: Rect,
    app: &App,
    now: Instant,
    panel: &Panel,
    window_secs: f64,
    index: usize,
    expanded: bool,
) {
    let suffix = if expanded {
        " (Esc to collapse)".to_string()
    } else {
        format!(" [{}]", index + 1)
    };
    render_panel_inner(f, area, app, now, panel, window_secs, &suffix);
}

fn window_labels(window_secs: f64) -> Vec<Span<'static>> {
    let w = window_secs as i64;
    let q2 = w * 2 / 3;
    let q1 = w / 3;
    vec![
        Span::raw(fmt_ago(w)),
        Span::raw(fmt_ago(q2)),
        Span::raw(fmt_ago(q1)),
        Span::raw("now"),
    ]
}

fn fmt_ago(secs: i64) -> String {
    if secs == 0 {
        "now".to_string()
    } else if secs < 60 || secs % 60 != 0 {
        format!("-{secs}s")
    } else {
        format!("-{}m", secs / 60)
    }
}

/// Compute pairwise rates from a history ring: each consecutive `(t, v)` pair
/// produces a `(seconds_before_now, dv/dt)` point. Negative deltas (counter
/// resets) are dropped.
pub fn rate_points(
    history: &VecDeque<TimedValue>,
    now: Instant,
    window_secs: f64,
) -> Vec<(f64, f64)> {
    if history.len() < 2 {
        return Vec::new();
    }
    let mut out = Vec::with_capacity(history.len() - 1);
    for i in 1..history.len() {
        let prev = history[i - 1];
        let cur = history[i];
        let dt = cur.at.duration_since(prev.at).as_secs_f64();
        if dt < 0.05 {
            continue;
        }
        let dv = cur.value.checked_sub(prev.value).unwrap_or(0);
        if dv < 0 {
            continue;
        }
        let rate = dv as f64 / dt;
        let t_ago = -(now.duration_since(cur.at).as_secs_f64());
        if t_ago >= -window_secs {
            out.push((t_ago, rate));
        }
    }
    out
}

/// Plot raw value over time (for gauges).
pub fn value_points(
    history: &VecDeque<TimedValue>,
    now: Instant,
    window_secs: f64,
) -> Vec<(f64, f64)> {
    history
        .iter()
        .filter_map(|tv| {
            let t_ago = -(now.duration_since(tv.at).as_secs_f64());
            if t_ago >= -window_secs {
                Some((t_ago, tv.value as f64))
            } else {
                None
            }
        })
        .collect()
}

fn format_y(v: f64, fmt: YFormat) -> String {
    match fmt {
        YFormat::Number => format::format_value("", v as i64),
        YFormat::Bytes => format::format_value("bytes", v as i64),
        YFormat::PerSecond => format::format_rate("", v),
        YFormat::BytesPerSecond => format::format_rate("bytes", v),
    }
}

/// Default Grafana-style dashboard: 6 panels in a 3×2 grid.
fn default_panels() -> &'static [Panel] {
    &[OPCOUNTERS, CONNECTIONS, NETWORK, QUEUES, WT_CACHE, MEMORY]
}

const OPCOUNTERS: Panel = Panel {
    title: "ops/s",
    y_format: YFormat::PerSecond,
    series: &[
        PanelSeries {
            path: "opcounters.insert",
            label: "insert",
            color: Color::Cyan,
            kind: SeriesKind::Rate,
        },
        PanelSeries {
            path: "opcounters.query",
            label: "query",
            color: Color::LightGreen,
            kind: SeriesKind::Rate,
        },
        PanelSeries {
            path: "opcounters.update",
            label: "update",
            color: Color::Yellow,
            kind: SeriesKind::Rate,
        },
        PanelSeries {
            path: "opcounters.delete",
            label: "delete",
            color: Color::LightRed,
            kind: SeriesKind::Rate,
        },
        PanelSeries {
            path: "opcounters.getmore",
            label: "getmore",
            color: Color::Magenta,
            kind: SeriesKind::Rate,
        },
        PanelSeries {
            path: "opcounters.command",
            label: "command",
            color: Color::LightBlue,
            kind: SeriesKind::Rate,
        },
    ],
};

const CONNECTIONS: Panel = Panel {
    title: "connections",
    y_format: YFormat::Number,
    series: &[
        PanelSeries {
            path: "connections.current",
            label: "current",
            color: Color::Cyan,
            kind: SeriesKind::Value,
        },
        PanelSeries {
            path: "connections.active",
            label: "active",
            color: Color::LightGreen,
            kind: SeriesKind::Value,
        },
        PanelSeries {
            path: "connections.totalCreated",
            label: "new/s",
            color: Color::Yellow,
            kind: SeriesKind::Rate,
        },
    ],
};

const NETWORK: Panel = Panel {
    title: "network",
    y_format: YFormat::BytesPerSecond,
    series: &[
        PanelSeries {
            path: "network.bytesIn",
            label: "in",
            color: Color::LightGreen,
            kind: SeriesKind::Rate,
        },
        PanelSeries {
            path: "network.bytesOut",
            label: "out",
            color: Color::Cyan,
            kind: SeriesKind::Rate,
        },
    ],
};

const QUEUES: Panel = Panel {
    title: "queues / active clients",
    y_format: YFormat::Number,
    series: &[
        PanelSeries {
            path: "globalLock.currentQueue.readers",
            label: "queued readers",
            color: Color::LightBlue,
            kind: SeriesKind::Value,
        },
        PanelSeries {
            path: "globalLock.currentQueue.writers",
            label: "queued writers",
            color: Color::Yellow,
            kind: SeriesKind::Value,
        },
        PanelSeries {
            path: "globalLock.activeClients.readers",
            label: "active readers",
            color: Color::LightGreen,
            kind: SeriesKind::Value,
        },
        PanelSeries {
            path: "globalLock.activeClients.writers",
            label: "active writers",
            color: Color::LightRed,
            kind: SeriesKind::Value,
        },
    ],
};

const WT_CACHE: Panel = Panel {
    title: "WiredTiger cache",
    y_format: YFormat::Bytes,
    series: &[
        PanelSeries {
            path: "wiredTiger.cache.bytes currently in the cache",
            label: "in cache",
            color: Color::Cyan,
            kind: SeriesKind::Value,
        },
        PanelSeries {
            path: "wiredTiger.cache.tracked dirty bytes in the cache",
            label: "dirty",
            color: Color::Yellow,
            kind: SeriesKind::Value,
        },
    ],
};

const MEMORY: Panel = Panel {
    title: "memory (MiB)",
    y_format: YFormat::Number,
    series: &[
        PanelSeries {
            path: "mem.resident",
            label: "resident",
            color: Color::LightGreen,
            kind: SeriesKind::Value,
        },
        PanelSeries {
            path: "mem.virtual",
            label: "virtual",
            color: Color::LightBlue,
            kind: SeriesKind::Value,
        },
        PanelSeries {
            path: "mem.mapped",
            label: "mapped",
            color: Color::Yellow,
            kind: SeriesKind::Value,
        },
    ],
};
