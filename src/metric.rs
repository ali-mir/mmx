use std::collections::VecDeque;
use std::time::Instant;

/// Max samples retained per metric in the ring buffer. At 1 Hz this is 15 min,
/// matching the longest selectable chart window.
pub const HISTORY_CAP: usize = 900;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MetricKind {
    /// Monotonically-increasing value; rendered as a per-second rate.
    Counter,
    /// Instantaneous value; rendered as-is.
    Gauge,
}

#[derive(Debug, Clone, Copy)]
pub struct TimedValue {
    pub at: Instant,
    pub value: i64,
}

/// Classify a flattened metric path. Heuristic-only — no schema is loaded
/// from mongod. Misclassifications surface as weird rate numbers, never crashes.
pub fn classify(path: &str) -> MetricKind {
    if is_gauge(path) {
        MetricKind::Gauge
    } else {
        MetricKind::Counter
    }
}

fn is_gauge(path: &str) -> bool {
    // Top-level instance scalars.
    matches!(
        path,
        "uptime" | "uptimeMillis" | "uptimeEstimate" | "localTime" | "pid"
    ) || ends_with_any(
        path,
        &[
            ".current",
            ".available",
            ".resident",
            ".virtual",
            ".mapped",
            ".mappedWithJournal",
            ".active",
            ".activeReaders",
            ".activeWriters",
            ".activeClients",
            ".activeReads",
            ".activeWrites",
            ".currentActive",
            ".currentInactive",
            ".currentOpen",
            ".currentPrepared",
            ".queueLength",
            ".lag",
            ".heapUsageBytes",
            ".size",
            ".freeBytes",
            ".used",
            ".usedBytes",
            ".dirty",
            ".dirtyBytes",
        ],
    ) || path.contains("currentQueue.")
        || path.contains("activeClients.")
        || path.contains("currently in")
        || path.starts_with("mem.")
        || path.starts_with("connections.")
        || path.starts_with("globalLock.")
        || path.starts_with("logicalSessionRecordCache.activeSessionsCount")
}

fn ends_with_any(path: &str, suffixes: &[&str]) -> bool {
    suffixes.iter().any(|s| path.ends_with(s))
}

/// Push a new sample into a fixed-capacity ring buffer.
pub fn push_history(history: &mut VecDeque<TimedValue>, sample: TimedValue) {
    if history.len() == HISTORY_CAP {
        history.pop_front();
    }
    history.push_back(sample);
}

/// Per-second rate from the last two samples in the history. Returns `None`
/// if there's only one sample, dt is too small to be meaningful, or the delta
/// went negative (treated as a counter reset / mongod restart).
pub fn rate_per_sec(history: &VecDeque<TimedValue>) -> Option<f64> {
    if history.len() < 2 {
        return None;
    }
    let last = history[history.len() - 1];
    let prev = history[history.len() - 2];
    let dt = last.at.duration_since(prev.at).as_secs_f64();
    if dt < 0.05 {
        return None;
    }
    let dv = last.value.checked_sub(prev.value)?;
    if dv < 0 {
        return None;
    }
    Some(dv as f64 / dt)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn known_gauges_classify_as_gauge() {
        for path in [
            "connections.current",
            "mem.resident",
            "globalLock.currentQueue.readers",
            "wiredTiger.cache.bytes currently in the cache",
            "transactions.currentActive",
            "uptimeMillis",
        ] {
            assert_eq!(classify(path), MetricKind::Gauge, "{path}");
        }
    }

    #[test]
    fn typical_counters_classify_as_counter() {
        for path in [
            "opcounters.insert",
            "network.bytesIn",
            "metrics.document.deleted",
            "asserts.user",
        ] {
            assert_eq!(classify(path), MetricKind::Counter, "{path}");
        }
    }

    #[test]
    fn rate_needs_two_samples() {
        let mut h = VecDeque::new();
        let t = Instant::now();
        push_history(&mut h, TimedValue { at: t, value: 10 });
        assert_eq!(rate_per_sec(&h), None);
    }

    #[test]
    fn rate_computes_per_second() {
        let mut h = VecDeque::new();
        let t = Instant::now();
        push_history(&mut h, TimedValue { at: t, value: 100 });
        push_history(
            &mut h,
            TimedValue {
                at: t + Duration::from_secs(2),
                value: 300,
            },
        );
        let rate = rate_per_sec(&h).unwrap();
        assert!((rate - 100.0).abs() < 0.01, "rate was {rate}");
    }

    #[test]
    fn negative_delta_means_reset() {
        let mut h = VecDeque::new();
        let t = Instant::now();
        push_history(&mut h, TimedValue { at: t, value: 1000 });
        push_history(
            &mut h,
            TimedValue {
                at: t + Duration::from_secs(1),
                value: 5,
            },
        );
        assert_eq!(rate_per_sec(&h), None);
    }

    #[test]
    fn history_caps_at_capacity() {
        let mut h = VecDeque::new();
        let t = Instant::now();
        for i in 0..(HISTORY_CAP + 50) {
            push_history(
                &mut h,
                TimedValue {
                    at: t + Duration::from_millis(i as u64),
                    value: i as i64,
                },
            );
        }
        assert_eq!(h.len(), HISTORY_CAP);
        assert_eq!(h.front().unwrap().value, 50);
        assert_eq!(h.back().unwrap().value, (HISTORY_CAP + 49) as i64);
    }
}
