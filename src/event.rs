use std::time::Duration;

use crossterm::event::{self, Event as CrosstermEvent, KeyEvent, MouseEvent};
use tokio::sync::mpsc;

use crate::source::Sample;

/// Application events.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub enum Event {
    Key(KeyEvent),
    Mouse(MouseEvent),
    Resize(u16, u16),
    /// Periodic tick (UI age refresh; not the data poll).
    Tick,
    /// Periodic render trigger.
    Render,
    /// New sample from the metric source.
    Sample(Sample),
    /// Source poll failed transiently — keep polling.
    PollFailed(String),
    /// Source poll failed fatally — stop polling.
    PollFatal(String),
}

/// Async event handler. Crossterm input + tick/render timers run on internal
/// tasks; the polling task is spawned by `main` and pushes via [`Self::sender`].
pub struct EventHandler {
    rx: mpsc::UnboundedReceiver<Event>,
    tx: mpsc::UnboundedSender<Event>,
    _tasks: Vec<tokio::task::JoinHandle<()>>,
}

impl EventHandler {
    pub fn new(tick_rate: Duration, render_rate: Duration) -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        let mut tasks = Vec::new();

        let tx_tick = tx.clone();
        tasks.push(tokio::spawn(async move {
            let mut interval = tokio::time::interval(tick_rate);
            loop {
                interval.tick().await;
                if tx_tick.send(Event::Tick).is_err() {
                    break;
                }
            }
        }));

        let tx_render = tx.clone();
        tasks.push(tokio::spawn(async move {
            let mut interval = tokio::time::interval(render_rate);
            loop {
                interval.tick().await;
                if tx_render.send(Event::Render).is_err() {
                    break;
                }
            }
        }));

        let tx_input = tx.clone();
        tasks.push(tokio::spawn(async move {
            loop {
                let maybe_event = tokio::task::spawn_blocking(|| {
                    event::poll(Duration::from_millis(100))
                        .ok()
                        .and_then(|has_event| if has_event { event::read().ok() } else { None })
                })
                .await;

                if let Ok(Some(crossterm_event)) = maybe_event {
                    let event = match crossterm_event {
                        CrosstermEvent::Key(key) => Event::Key(key),
                        CrosstermEvent::Mouse(mouse) => Event::Mouse(mouse),
                        CrosstermEvent::Resize(w, h) => Event::Resize(w, h),
                        _ => continue,
                    };
                    if tx_input.send(event).is_err() {
                        break;
                    }
                }
            }
        }));

        EventHandler {
            rx,
            tx,
            _tasks: tasks,
        }
    }

    /// Clone of the internal sender for external producers (e.g. the metric
    /// poll task pushing `Event::Sample` / `Event::PollFailed` / `Event::PollFatal`).
    pub fn sender(&self) -> mpsc::UnboundedSender<Event> {
        self.tx.clone()
    }

    pub async fn next(&mut self) -> Option<Event> {
        self.rx.recv().await
    }
}
