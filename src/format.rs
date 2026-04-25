/// Format a per-second rate based on metric path. Bytes-per-second for byte
/// metrics, decimal otherwise. Values below 1/s show one decimal place; larger
/// values are rounded and run through the same K/M/B abbreviation.
pub fn format_rate(path: &str, rate: f64) -> String {
    let path_lower = path.to_lowercase();
    if rate.abs() < 0.05 {
        return "0/s".to_string();
    }
    if rate.abs() < 10.0 {
        if is_byte_metric(&path_lower) {
            return format!("{rate:.1} B/s");
        }
        return format!("{rate:.1}/s");
    }
    let rounded = rate.round() as i64;
    if is_byte_metric(&path_lower) {
        format!("{}/s", format_bytes(rounded))
    } else {
        format!("{}/s", format_number(rounded))
    }
}

/// Format a metric value with human-readable units based on the metric path.
pub fn format_value(path: &str, value: i64) -> String {
    let path_lower = path.to_lowercase();

    if is_byte_metric(&path_lower) {
        format_bytes(value)
    } else if is_millis_metric(&path_lower) {
        format_duration_ms(value)
    } else if is_micros_metric(&path_lower) {
        format_duration_us(value)
    } else if is_timestamp_metric(&path_lower, value) {
        format_epoch_ms(value)
    } else {
        format_number(value)
    }
}

fn is_byte_metric(path: &str) -> bool {
    path.contains("bytes")
        || path.contains("bytesread")
        || path.contains("byteswritten")
        || path.contains("datasize")
        || path.contains("storagesize")
        || path.contains("totalsize")
        || path.contains("cache.bytes")
        || path.contains("memory")
        || (path.contains("wiredtiger") && path.contains("cache") && path.ends_with("bytes"))
}

fn is_millis_metric(path: &str) -> bool {
    path.contains("millis")
        || path.contains("timemillis")
        || path.contains("locktimemicros") // actually micros, but we check millis first
        || path.ends_with("ms")
}

fn is_micros_metric(path: &str) -> bool {
    path.contains("micros") || path.contains("usecs")
}

fn is_timestamp_metric(path: &str, value: i64) -> bool {
    // Value must look like epoch-ms between 2000-01-01 and 2100-01-01
    if !(946_684_800_000..=4_102_444_800_000).contains(&value) {
        return false;
    }

    // Top-level start/end fields
    if path == "start" || path == "end" {
        return true;
    }

    // Path suffix patterns for timestamp fields
    path.ends_with(".start")
        || path.ends_with(".end")
        || path.ends_with(".localtime")
        || path.ends_with(".clustertime")
        || path.ends_with(".operationtime")
        || path.ends_with(".lastupdated")
        || path.ends_with(".readtimestamp")
        || path.ends_with(".oldestactivetimestamp")
        || path.ends_with(".stabletimestamp")
        || path.ends_with(".oldesttimestamp")
        || path.ends_with(".walltime")
        || path.ends_with(".date")
        || path.ends_with(".lastapplied")
        || path.ends_with(".lastdurable")
        || path.contains("timestamp") // catch-all for *Timestamp* fields
}

fn format_bytes(value: i64) -> String {
    let abs = value.unsigned_abs();
    let sign = if value < 0 { "-" } else { "" };

    if abs >= 1 << 30 {
        format!("{sign}{:.1} GiB", abs as f64 / (1u64 << 30) as f64)
    } else if abs >= 1 << 20 {
        format!("{sign}{:.1} MiB", abs as f64 / (1u64 << 20) as f64)
    } else if abs >= 1 << 10 {
        format!("{sign}{:.1} KiB", abs as f64 / (1u64 << 10) as f64)
    } else {
        format!("{sign}{abs} B")
    }
}

fn format_duration_ms(value: i64) -> String {
    let abs = value.unsigned_abs();
    let sign = if value < 0 { "-" } else { "" };

    if abs >= 60_000 {
        format!("{sign}{:.1}m", abs as f64 / 60_000.0)
    } else if abs >= 1_000 {
        format!("{sign}{:.1}s", abs as f64 / 1_000.0)
    } else {
        format!("{sign}{abs}ms")
    }
}

fn format_duration_us(value: i64) -> String {
    let abs = value.unsigned_abs();
    let sign = if value < 0 { "-" } else { "" };

    if abs >= 1_000_000 {
        format!("{sign}{:.1}s", abs as f64 / 1_000_000.0)
    } else if abs >= 1_000 {
        format!("{sign}{:.1}ms", abs as f64 / 1_000.0)
    } else {
        format!("{sign}{abs}us")
    }
}

fn format_epoch_ms(value: i64) -> String {
    let secs = value / 1_000;
    let millis = (value % 1_000) as u32;

    // Manual UTC breakdown (no chrono dependency needed)
    let days_since_epoch = secs / 86_400;
    let time_of_day = secs % 86_400;
    let hour = time_of_day / 3_600;
    let minute = (time_of_day % 3_600) / 60;
    let second = time_of_day % 60;

    // Convert days since 1970-01-01 to Y-M-D
    // Using a simplified algorithm
    let (year, month, day) = days_to_ymd(days_since_epoch);

    if millis == 0 {
        format!("{year:04}-{month:02}-{day:02} {hour:02}:{minute:02}:{second:02}")
    } else {
        format!("{year:04}-{month:02}-{day:02} {hour:02}:{minute:02}:{second:02}.{millis:03}")
    }
}

pub fn days_to_ymd(days: i64) -> (i64, i64, i64) {
    // Civil days-from-epoch to (y, m, d) — Howard Hinnant's algorithm
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}

fn format_number(value: i64) -> String {
    let abs = value.unsigned_abs();
    let sign = if value < 0 { "-" } else { "" };

    if abs >= 1_000_000_000 {
        format!("{sign}{:.2}B", abs as f64 / 1_000_000_000.0)
    } else if abs >= 1_000_000 {
        format!("{sign}{:.2}M", abs as f64 / 1_000_000.0)
    } else if abs >= 10_000 {
        format!("{sign}{:.1}K", abs as f64 / 1_000.0)
    } else {
        format!("{sign}{abs}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_bytes() {
        assert_eq!(format_bytes(0), "0 B");
        assert_eq!(format_bytes(512), "512 B");
        assert_eq!(format_bytes(1024), "1.0 KiB");
        assert_eq!(format_bytes(1_048_576), "1.0 MiB");
        assert_eq!(format_bytes(1_073_741_824), "1.0 GiB");
        assert_eq!(format_bytes(-1024), "-1.0 KiB");
    }

    #[test]
    fn test_format_duration_ms() {
        assert_eq!(format_duration_ms(0), "0ms");
        assert_eq!(format_duration_ms(500), "500ms");
        assert_eq!(format_duration_ms(1500), "1.5s");
        assert_eq!(format_duration_ms(90_000), "1.5m");
    }

    #[test]
    fn test_format_duration_us() {
        assert_eq!(format_duration_us(0), "0us");
        assert_eq!(format_duration_us(500), "500us");
        assert_eq!(format_duration_us(1500), "1.5ms");
        assert_eq!(format_duration_us(1_500_000), "1.5s");
    }

    #[test]
    fn test_format_number() {
        assert_eq!(format_number(0), "0");
        assert_eq!(format_number(42), "42");
        assert_eq!(format_number(9999), "9999");
        assert_eq!(format_number(10_000), "10.0K");
        assert_eq!(format_number(1_500_000), "1.50M");
        assert_eq!(format_number(2_500_000_000), "2.50B");
        assert_eq!(format_number(-42), "-42");
    }

    #[test]
    fn test_format_epoch_ms() {
        // 2024-01-01 00:00:00.000 UTC
        assert_eq!(format_epoch_ms(1_704_067_200_000), "2024-01-01 00:00:00");
        // With millis
        assert_eq!(
            format_epoch_ms(1_704_067_200_123),
            "2024-01-01 00:00:00.123"
        );
    }

    #[test]
    fn test_format_value_heuristic() {
        // Byte metrics
        assert!(format_value("wiredTiger.cache.bytes", 1_073_741_824).contains("GiB"));
        assert!(format_value("serverStatus.mem.bytes", 1024).contains("KiB"));

        // Millis metrics
        assert!(format_value("opLatencies.reads.latencyMillis", 1500).contains('s'));

        // Timestamp metrics — should NOT show as "B"
        let ts = format_value("config.image_collection.stats.end", 1_704_067_200_000);
        assert!(ts.contains("2024"), "expected datetime, got: {ts}");
        assert!(!ts.contains('B'), "should not format as billions: {ts}");

        let ts2 = format_value("local.oplog.rs.stats.start", 1_704_067_200_000);
        assert!(ts2.contains("2024"), "expected datetime, got: {ts2}");

        // Regular numbers
        assert_eq!(format_value("connections.current", 42), "42");
    }
}
