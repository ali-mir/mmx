use std::collections::{HashSet, VecDeque};
use std::time::Instant;

use ratatui::widgets::TableState;

use crate::metric::{MetricKind, TimedValue, classify, push_history, rate_per_sec};
use crate::source::Sample;

/// A single metric entry with its current state.
#[derive(Debug, Clone)]
pub struct MetricEntry {
    pub path: String,
    pub kind: MetricKind,
    pub current: i64,
    pub previous: Option<i64>,
    pub history: VecDeque<TimedValue>,
}

impl MetricEntry {
    pub fn new(path: String, value: i64, at: Instant) -> Self {
        let kind = classify(&path);
        let mut history = VecDeque::new();
        history.push_back(TimedValue { at, value });
        MetricEntry {
            path,
            kind,
            current: value,
            previous: None,
            history,
        }
    }

    pub fn delta(&self) -> Option<i64> {
        self.previous.map(|prev| self.current.wrapping_sub(prev))
    }

    /// Per-second rate based on last two history samples. Always `None` for
    /// gauges; `None` for counters until we have ≥2 samples or after a reset.
    pub fn rate_per_sec(&self) -> Option<f64> {
        match self.kind {
            MetricKind::Counter => rate_per_sec(&self.history),
            MetricKind::Gauge => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppMode {
    Normal,
    Search,
    Help,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Focus {
    Main,
    Pinned,
}

#[derive(Debug, Clone)]
pub enum ConnectionState {
    Connected,
    Reconnecting(String),
    Failed(String),
}

/// Time window the chart panels render. Cycle with `+` / `-`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowSpec {
    TenSeconds,
    ThirtySeconds,
    OneMinute,
    FiveMinutes,
    TenMinutes,
}

impl WindowSpec {
    pub fn seconds(self) -> f64 {
        match self {
            WindowSpec::TenSeconds => 10.0,
            WindowSpec::ThirtySeconds => 30.0,
            WindowSpec::OneMinute => 60.0,
            WindowSpec::FiveMinutes => 300.0,
            WindowSpec::TenMinutes => 600.0,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            WindowSpec::TenSeconds => "10s",
            WindowSpec::ThirtySeconds => "30s",
            WindowSpec::OneMinute => "1m",
            WindowSpec::FiveMinutes => "5m",
            WindowSpec::TenMinutes => "10m",
        }
    }

    pub fn next(self) -> Self {
        match self {
            WindowSpec::TenSeconds => WindowSpec::ThirtySeconds,
            WindowSpec::ThirtySeconds => WindowSpec::OneMinute,
            WindowSpec::OneMinute => WindowSpec::FiveMinutes,
            WindowSpec::FiveMinutes => WindowSpec::TenMinutes,
            WindowSpec::TenMinutes => WindowSpec::TenSeconds,
        }
    }

    pub fn prev(self) -> Self {
        match self {
            WindowSpec::TenSeconds => WindowSpec::TenMinutes,
            WindowSpec::ThirtySeconds => WindowSpec::TenSeconds,
            WindowSpec::OneMinute => WindowSpec::ThirtySeconds,
            WindowSpec::FiveMinutes => WindowSpec::OneMinute,
            WindowSpec::TenMinutes => WindowSpec::FiveMinutes,
        }
    }
}

/// Messages that drive state transitions (Elm architecture).
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub enum Message {
    MoveDown,
    MoveUp,
    JumpTop,
    JumpBottom,
    TogglePin,
    EnterSearch,
    ExitSearch,
    SearchInput(char),
    SearchBackspace,
    ToggleHelp,
    ToggleFocus,
    SampleArrived(Sample),
    PollFailed(String),
    PollFatal(String),
    TogglePause,
    NextWindow,
    PrevWindow,
    ExpandPanel(usize),
    CollapsePanel,
    UpdateMetrics(Vec<MetricEntry>),
    Tick,
    Quit,
}

/// Core application state.
pub struct App {
    pub metrics: Vec<MetricEntry>,
    pub table_state: TableState,
    pub pinned_table_state: TableState,
    pub pinned: HashSet<String>,
    pub filter: String,
    pub mode: AppMode,
    pub focus: Focus,
    pub source_label: String,
    pub host: Option<String>,
    pub version: Option<String>,
    pub last_poll: Option<Instant>,
    pub poll_count: usize,
    pub connection: ConnectionState,
    pub paused: bool,
    pub window: WindowSpec,
    pub expanded_panel: Option<usize>,
    pub tick_count: usize,
    pub should_quit: bool,
}

impl App {
    pub fn new(source_label: String) -> Self {
        let mut table_state = TableState::default();
        table_state.select(Some(0));
        App {
            metrics: Vec::new(),
            table_state,
            pinned_table_state: TableState::default(),
            pinned: HashSet::new(),
            filter: String::new(),
            mode: AppMode::Normal,
            focus: Focus::Main,
            source_label,
            host: None,
            version: None,
            last_poll: None,
            poll_count: 0,
            connection: ConnectionState::Reconnecting("connecting…".into()),
            paused: false,
            window: WindowSpec::TenSeconds,
            expanded_panel: None,
            tick_count: 0,
            should_quit: false,
        }
    }

    pub fn update(&mut self, msg: Message) {
        match msg {
            Message::MoveDown => {
                let len = self.filtered_metrics().len();
                if len == 0 {
                    return;
                }
                match self.focus {
                    Focus::Main => {
                        let i = self.table_state.selected().unwrap_or(0);
                        self.table_state.select(Some((i + 1).min(len - 1)));
                    }
                    Focus::Pinned => {
                        let pinned_len = self.pinned_metrics().len();
                        if pinned_len == 0 {
                            return;
                        }
                        let i = self.pinned_table_state.selected().unwrap_or(0);
                        self.pinned_table_state
                            .select(Some((i + 1).min(pinned_len - 1)));
                    }
                }
            }
            Message::MoveUp => match self.focus {
                Focus::Main => {
                    let i = self.table_state.selected().unwrap_or(0);
                    self.table_state.select(Some(i.saturating_sub(1)));
                }
                Focus::Pinned => {
                    let i = self.pinned_table_state.selected().unwrap_or(0);
                    self.pinned_table_state.select(Some(i.saturating_sub(1)));
                }
            },
            Message::JumpTop => match self.focus {
                Focus::Main => self.table_state.select(Some(0)),
                Focus::Pinned => self.pinned_table_state.select(Some(0)),
            },
            Message::JumpBottom => match self.focus {
                Focus::Main => {
                    let len = self.filtered_metrics().len();
                    if len > 0 {
                        self.table_state.select(Some(len - 1));
                    }
                }
                Focus::Pinned => {
                    let len = self.pinned_metrics().len();
                    if len > 0 {
                        self.pinned_table_state.select(Some(len - 1));
                    }
                }
            },
            Message::TogglePin => {
                let selected_path = match self.focus {
                    Focus::Main => {
                        let filtered = self.filtered_metrics();
                        let idx = self.table_state.selected().unwrap_or(0);
                        filtered.get(idx).map(|m| m.path.clone())
                    }
                    Focus::Pinned => {
                        let pinned = self.pinned_metrics();
                        let idx = self.pinned_table_state.selected().unwrap_or(0);
                        pinned.get(idx).map(|m| m.path.clone())
                    }
                };
                if let Some(path) = selected_path {
                    if self.pinned.contains(&path) {
                        self.pinned.remove(&path);
                    } else {
                        self.pinned.insert(path);
                    }
                }
            }
            Message::EnterSearch => {
                self.mode = AppMode::Search;
            }
            Message::ExitSearch => {
                self.mode = AppMode::Normal;
                self.filter.clear();
            }
            Message::SearchInput(c) => {
                self.filter.push(c);
                self.table_state.select(Some(0));
            }
            Message::SearchBackspace => {
                self.filter.pop();
                self.table_state.select(Some(0));
            }
            Message::ToggleHelp => {
                self.mode = if self.mode == AppMode::Help {
                    AppMode::Normal
                } else {
                    AppMode::Help
                };
            }
            Message::ToggleFocus => {
                if !self.pinned.is_empty() {
                    self.focus = match self.focus {
                        Focus::Main => Focus::Pinned,
                        Focus::Pinned => Focus::Main,
                    };
                }
            }
            Message::SampleArrived(sample) => {
                // Connection state still updates while paused so the user can
                // see when the link drops. Sample is ignored for merging only.
                self.connection = ConnectionState::Connected;
                if self.paused {
                    return;
                }
                if sample.host.is_some() {
                    self.host = sample.host;
                }
                if sample.version.is_some() {
                    self.version = sample.version;
                }
                let now = Instant::now();
                self.last_poll = Some(now);
                self.poll_count += 1;
                self.merge_sample(sample.metrics, now);
            }
            Message::PollFailed(reason) => {
                self.connection = ConnectionState::Reconnecting(reason);
            }
            Message::PollFatal(reason) => {
                self.connection = ConnectionState::Failed(reason);
            }
            Message::TogglePause => {
                self.paused = !self.paused;
            }
            Message::NextWindow => {
                self.window = self.window.next();
            }
            Message::PrevWindow => {
                self.window = self.window.prev();
            }
            Message::ExpandPanel(i) => {
                self.expanded_panel = Some(i);
            }
            Message::CollapsePanel => {
                self.expanded_panel = None;
            }
            #[cfg(test)]
            Message::UpdateMetrics(entries) => {
                self.merge_metrics(entries);
            }
            #[cfg(not(test))]
            Message::UpdateMetrics(_) => {}
            Message::Tick => {}
            Message::Quit => {
                self.should_quit = true;
            }
        }
    }

    /// Merge a new sample (from `Sample.metrics`) into existing entries:
    /// update `current`/`previous` and append a timestamped value to history.
    fn merge_sample(&mut self, new_metrics: Vec<crate::bson_ext::FlatMetric>, at: Instant) {
        use std::collections::HashMap;
        let by_path: HashMap<String, usize> = self
            .metrics
            .iter()
            .enumerate()
            .map(|(i, m)| (m.path.clone(), i))
            .collect();

        let mut additions: Vec<MetricEntry> = Vec::new();
        for new in new_metrics {
            if let Some(&i) = by_path.get(new.path.as_str()) {
                let existing = &mut self.metrics[i];
                existing.previous = Some(existing.current);
                existing.current = new.value;
                push_history(
                    &mut existing.history,
                    TimedValue {
                        at,
                        value: new.value,
                    },
                );
            } else {
                additions.push(MetricEntry::new(new.path, new.value, at));
            }
        }

        if !additions.is_empty() {
            self.metrics.extend(additions);
            self.metrics
                .sort_by(|a, b| metric_sort_key(&a.path).cmp(&metric_sort_key(&b.path)));
        }
    }

    #[cfg(test)]
    fn merge_metrics(&mut self, new_metrics: Vec<MetricEntry>) {
        for new in new_metrics {
            if let Some(existing) = self.metrics.iter_mut().find(|m| m.path == new.path) {
                existing.previous = Some(existing.current);
                existing.current = new.current;
                push_history(
                    &mut existing.history,
                    TimedValue {
                        at: Instant::now(),
                        value: new.current,
                    },
                );
            } else {
                self.metrics.push(new);
            }
        }
        self.metrics
            .sort_by(|a, b| metric_sort_key(&a.path).cmp(&metric_sort_key(&b.path)));
    }

    pub fn filtered_metrics(&self) -> Vec<&MetricEntry> {
        let filter_lower = self.filter.to_lowercase();
        self.metrics
            .iter()
            .filter(|m| {
                if filter_lower.is_empty() {
                    true
                } else {
                    m.path.to_lowercase().contains(&filter_lower)
                }
            })
            .collect()
    }

    pub fn pinned_metrics(&self) -> Vec<&MetricEntry> {
        self.metrics
            .iter()
            .filter(|m| self.pinned.contains(&m.path))
            .collect()
    }

    pub fn is_pinned(&self, path: &str) -> bool {
        self.pinned.contains(path)
    }
}

/// Sort priority: opcounters → connections → repl → everything else,
/// alphabetical within each group.
fn metric_sort_key(path: &str) -> (u8, &str) {
    let group = if path.starts_with("opcounters") {
        0
    } else if path.starts_with("connections") {
        1
    } else if path.starts_with("repl") {
        2
    } else {
        3
    };
    (group, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_metrics() -> Vec<MetricEntry> {
        let t = Instant::now();
        vec![
            MetricEntry::new("a.b.c".into(), 10, t),
            MetricEntry::new("x.y.z".into(), 20, t),
            MetricEntry::new("m.n.o".into(), 30, t),
        ]
    }

    #[test]
    fn test_move_down() {
        let mut app = App::new("test".into());
        app.update(Message::UpdateMetrics(sample_metrics()));
        assert_eq!(app.table_state.selected(), Some(0));
        app.update(Message::MoveDown);
        assert_eq!(app.table_state.selected(), Some(1));
        app.update(Message::MoveDown);
        assert_eq!(app.table_state.selected(), Some(2));
        app.update(Message::MoveDown);
        assert_eq!(app.table_state.selected(), Some(2));
    }

    #[test]
    fn test_move_up() {
        let mut app = App::new("test".into());
        app.update(Message::UpdateMetrics(sample_metrics()));
        app.update(Message::MoveDown);
        app.update(Message::MoveDown);
        assert_eq!(app.table_state.selected(), Some(2));
        app.update(Message::MoveUp);
        assert_eq!(app.table_state.selected(), Some(1));
        app.update(Message::MoveUp);
        assert_eq!(app.table_state.selected(), Some(0));
        app.update(Message::MoveUp);
        assert_eq!(app.table_state.selected(), Some(0));
    }

    #[test]
    fn test_jump_top_bottom() {
        let mut app = App::new("test".into());
        app.update(Message::UpdateMetrics(sample_metrics()));
        app.update(Message::JumpBottom);
        assert_eq!(app.table_state.selected(), Some(2));
        app.update(Message::JumpTop);
        assert_eq!(app.table_state.selected(), Some(0));
    }

    #[test]
    fn test_pin_unpin() {
        let mut app = App::new("test".into());
        app.update(Message::UpdateMetrics(sample_metrics()));
        assert!(app.pinned.is_empty());

        app.update(Message::TogglePin);
        assert!(app.pinned.contains("a.b.c"));

        app.update(Message::TogglePin);
        assert!(!app.pinned.contains("a.b.c"));
    }

    #[test]
    fn test_filter() {
        let mut app = App::new("test".into());
        app.update(Message::UpdateMetrics(sample_metrics()));
        assert_eq!(app.filtered_metrics().len(), 3);

        app.update(Message::EnterSearch);
        app.update(Message::SearchInput('x'));
        assert_eq!(app.filtered_metrics().len(), 1);
        assert_eq!(app.filtered_metrics()[0].path, "x.y.z");

        app.update(Message::ExitSearch);
        assert_eq!(app.filtered_metrics().len(), 3);
        assert_eq!(app.filter, "");
    }

    #[test]
    fn test_help_toggle() {
        let mut app = App::new("test".into());
        assert_eq!(app.mode, AppMode::Normal);
        app.update(Message::ToggleHelp);
        assert_eq!(app.mode, AppMode::Help);
        app.update(Message::ToggleHelp);
        assert_eq!(app.mode, AppMode::Normal);
    }

    #[test]
    fn test_focus_toggle() {
        let mut app = App::new("test".into());
        app.update(Message::UpdateMetrics(sample_metrics()));
        app.pinned.insert("a.b.c".into());
        assert_eq!(app.focus, Focus::Main);

        app.update(Message::ToggleFocus);
        assert_eq!(app.focus, Focus::Pinned);

        app.update(Message::ToggleFocus);
        assert_eq!(app.focus, Focus::Main);
    }

    #[test]
    fn test_focus_toggle_no_pinned() {
        let mut app = App::new("test".into());
        app.update(Message::UpdateMetrics(sample_metrics()));
        assert_eq!(app.focus, Focus::Main);

        app.update(Message::ToggleFocus);
        assert_eq!(app.focus, Focus::Main);
    }

    #[test]
    fn test_merge_tracks_previous_and_history() {
        let mut app = App::new("test".into());
        let t = Instant::now();
        app.update(Message::UpdateMetrics(vec![MetricEntry::new(
            "a".into(),
            10,
            t,
        )]));
        assert_eq!(app.metrics[0].current, 10);
        assert_eq!(app.metrics[0].previous, None);

        app.update(Message::UpdateMetrics(vec![MetricEntry::new(
            "a".into(),
            20,
            t,
        )]));
        assert_eq!(app.metrics[0].current, 20);
        assert_eq!(app.metrics[0].previous, Some(10));
        assert_eq!(app.metrics[0].delta(), Some(10));
        assert_eq!(app.metrics[0].history.len(), 2);
        assert_eq!(app.metrics[0].history.back().unwrap().value, 20);
    }
}
