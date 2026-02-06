use std::collections::HashSet;

use ratatui::widgets::TableState;

/// A single metric entry with its current state.
#[derive(Debug, Clone)]
pub struct MetricEntry {
    pub path: String,
    pub current: i64,
    pub previous: Option<i64>,
    pub history: Vec<i64>,
}

impl MetricEntry {
    pub fn delta(&self) -> Option<i64> {
        self.previous.map(|prev| self.current - prev)
    }
}

/// The mode the application is in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppMode {
    Normal,
    Search,
    Help,
}

/// Which panel has focus.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Focus {
    Main,
    Pinned,
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
    pub file_path: String,
    pub sample_timestamp: String,
    pub should_quit: bool,
}

impl App {
    pub fn new(file_path: String) -> Self {
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
            file_path,
            sample_timestamp: String::new(),
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
                // Reset selection when filter changes
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
            Message::UpdateMetrics(new_metrics) => {
                // Merge with existing: update current, shift previous
                for new in &new_metrics {
                    if let Some(existing) = self.metrics.iter_mut().find(|m| m.path == new.path) {
                        existing.previous = Some(existing.current);
                        existing.current = new.current;
                        existing.history.push(new.current);
                        // Keep last 300 samples
                        if existing.history.len() > 300 {
                            existing.history.remove(0);
                        }
                    }
                }
                // Add any new metrics not yet seen
                for new in new_metrics {
                    if !self.metrics.iter().any(|m| m.path == new.path) {
                        self.metrics.push(new);
                    }
                }
                // Sort alphabetically
                self.metrics.sort_by(|a, b| a.path.cmp(&b.path));
            }
            Message::Tick => {
                // Tick is handled externally; this is a no-op in state
            }
            Message::Quit => {
                self.should_quit = true;
            }
        }
    }

    /// Return metrics filtered by the current search string.
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

    /// Return pinned metrics in sorted order.
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

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_metrics() -> Vec<MetricEntry> {
        vec![
            MetricEntry {
                path: "a.b.c".into(),
                current: 10,
                previous: None,
                history: vec![10],
            },
            MetricEntry {
                path: "x.y.z".into(),
                current: 20,
                previous: None,
                history: vec![20],
            },
            MetricEntry {
                path: "m.n.o".into(),
                current: 30,
                previous: None,
                history: vec![30],
            },
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
        app.update(Message::MoveDown); // should not go past end
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
        app.update(Message::MoveUp); // should not go negative
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
        // Should pin the first metric (selected=0, sorted: a.b.c)
        assert!(app.pinned.contains("a.b.c"));

        app.update(Message::TogglePin);
        // Toggle again should unpin
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

        // Should not change focus if nothing is pinned
        app.update(Message::ToggleFocus);
        assert_eq!(app.focus, Focus::Main);
    }

    #[test]
    fn test_update_metrics_preserves_history() {
        let mut app = App::new("test".into());
        app.update(Message::UpdateMetrics(vec![MetricEntry {
            path: "a".into(),
            current: 10,
            previous: None,
            history: vec![10],
        }]));
        assert_eq!(app.metrics[0].current, 10);
        assert_eq!(app.metrics[0].previous, None);

        app.update(Message::UpdateMetrics(vec![MetricEntry {
            path: "a".into(),
            current: 20,
            previous: None,
            history: vec![20],
        }]));
        assert_eq!(app.metrics[0].current, 20);
        assert_eq!(app.metrics[0].previous, Some(10));
        assert_eq!(app.metrics[0].history, vec![10, 20]);
    }
}
