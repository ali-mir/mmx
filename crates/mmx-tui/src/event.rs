use std::time::Duration;

use crossterm::event::{self, Event as CrosstermEvent, KeyEvent, MouseEvent};
use tokio::sync::mpsc;

/// Application events.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub enum Event {
    /// Terminal key press.
    Key(KeyEvent),
    /// Terminal mouse event.
    Mouse(MouseEvent),
    /// Terminal resize.
    Resize(u16, u16),
    /// Periodic tick for data refresh.
    Tick,
    /// Periodic render trigger.
    Render,
}

/// Async event handler that polls crossterm events and emits ticks/renders.
pub struct EventHandler {
    rx: mpsc::UnboundedReceiver<Event>,
    _task: tokio::task::JoinHandle<()>,
}

impl EventHandler {
    /// Create a new EventHandler with the given tick and render intervals.
    pub fn new(tick_rate: Duration, render_rate: Duration) -> Self {
        let (tx, rx) = mpsc::unbounded_channel();

        let task = tokio::spawn(async move {
            let mut tick_interval = tokio::time::interval(tick_rate);
            let mut render_interval = tokio::time::interval(render_rate);

            loop {
                let event = tokio::select! {
                    _ = tick_interval.tick() => Event::Tick,
                    _ = render_interval.tick() => Event::Render,
                    maybe_event = tokio::task::spawn_blocking(|| {
                        event::poll(Duration::from_millis(50))
                            .ok()
                            .and_then(|has_event| {
                                if has_event {
                                    event::read().ok()
                                } else {
                                    None
                                }
                            })
                    }) => {
                        if let Ok(Some(crossterm_event)) = maybe_event {
                            match crossterm_event {
                                CrosstermEvent::Key(key) => Event::Key(key),
                                CrosstermEvent::Mouse(mouse) => Event::Mouse(mouse),
                                CrosstermEvent::Resize(w, h) => Event::Resize(w, h),
                                _ => continue,
                            }
                        } else {
                            continue;
                        }
                    }
                };

                if tx.send(event).is_err() {
                    break;
                }
            }
        });

        EventHandler { rx, _task: task }
    }

    /// Receive the next event.
    pub async fn next(&mut self) -> Option<Event> {
        self.rx.recv().await
    }
}
