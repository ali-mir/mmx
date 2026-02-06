use std::io::BufReader;
use std::path::PathBuf;

use mmx_ftdc::reader::{self, FtdcRecord};

fn test_data_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("test-data/diagnostic.data")
}

#[test]
fn test_parse_real_ftdc_files() {
    let dir = test_data_dir();
    if !dir.exists() {
        eprintln!("Skipping: test-data not present at {}", dir.display());
        return;
    }

    let files = reader::find_ftdc_files(&dir).expect("find ftdc files");
    assert!(!files.is_empty(), "should find at least one FTDC file");
    println!("Found {} FTDC file(s)", files.len());

    let mut total_metadata = 0;
    let mut total_chunks = 0;
    let mut total_metrics = 0;
    let mut total_samples = 0;
    let mut sample_paths: Vec<String> = Vec::new();

    for file_path in &files {
        println!("\nParsing: {}", file_path.display());
        let file = std::fs::File::open(file_path).expect("open file");
        let reader = BufReader::new(file);
        let ftdc = reader::FtdcReader::new(reader);

        for record in ftdc {
            match record {
                Ok(FtdcRecord::Metadata(doc)) => {
                    total_metadata += 1;
                    println!("  Metadata doc with {} keys", doc.len());
                }
                Ok(FtdcRecord::MetricChunk(chunk)) => {
                    total_chunks += 1;
                    let metric_count = chunk.metrics.len();
                    let sample_count = chunk.metrics.first().map(|m| m.values.len()).unwrap_or(0);
                    total_metrics = total_metrics.max(metric_count);
                    total_samples += sample_count;

                    if sample_paths.is_empty() {
                        sample_paths = chunk
                            .metrics
                            .iter()
                            .take(10)
                            .map(|m| m.path.clone())
                            .collect();
                    }
                }
                Err(e) => {
                    let fname = file_path.file_name().unwrap().to_string_lossy();
                    if fname.contains("interim") {
                        // Interim files are expected to be truncated
                        eprintln!("  Skipping truncated interim file: {e}");
                    } else {
                        eprintln!("  Warning: {e}");
                    }
                    break;
                }
            }
        }
    }

    println!("\n=== Summary ===");
    println!("Files:        {}", files.len());
    println!("Metadata:     {total_metadata}");
    println!("Chunks:       {total_chunks}");
    println!("Max metrics:  {total_metrics}");
    println!("Total samples: {total_samples}");
    println!("\nSample metric paths:");
    for path in &sample_paths {
        println!("  {path}");
    }

    assert!(total_chunks > 0, "should have at least one metric chunk");
    assert!(total_metrics > 0, "should have metrics in chunks");
    assert!(total_samples > 0, "should have samples");
}

#[test]
fn test_start_timestamp_changes_across_samples() {
    let dir = test_data_dir();
    if !dir.exists() {
        eprintln!("Skipping: test-data not present at {}", dir.display());
        return;
    }

    let files = reader::find_ftdc_files(&dir).expect("find ftdc files");

    for file_path in &files {
        let file = std::fs::File::open(file_path).expect("open file");
        let reader = BufReader::new(file);
        let ftdc = reader::FtdcReader::new(reader);

        for record in ftdc {
            let Ok(FtdcRecord::MetricChunk(chunk)) = record else {
                continue;
            };
            let Some(start_metric) = chunk.metrics.iter().find(|m| m.path == "start") else {
                continue;
            };

            if start_metric.values.len() < 2 {
                continue;
            }

            // start timestamp should increase between samples (~1000ms apart)
            let mut changed = false;
            for window in start_metric.values.windows(2) {
                if window[1] != window[0] {
                    changed = true;
                    assert!(
                        window[1] > window[0],
                        "start timestamp should increase: {} -> {}",
                        window[0],
                        window[1]
                    );
                }
            }
            assert!(changed, "start timestamp should change across samples");
            return; // Only need to verify one chunk
        }
    }
    panic!("no metric chunks found");
}

#[test]
fn test_uptime_millis_increases() {
    let dir = test_data_dir();
    if !dir.exists() {
        eprintln!("Skipping: test-data not present at {}", dir.display());
        return;
    }

    let files = reader::find_ftdc_files(&dir).expect("find ftdc files");
    let mut all_uptime_values: Vec<i64> = Vec::new();

    for file_path in &files {
        let file = std::fs::File::open(file_path).expect("open file");
        let reader = BufReader::new(file);
        let ftdc = reader::FtdcReader::new(reader);

        for record in ftdc {
            let Ok(FtdcRecord::MetricChunk(chunk)) = record else {
                continue;
            };
            let Some(uptime) = chunk
                .metrics
                .iter()
                .find(|m| m.path == "serverStatus.uptimeMillis")
            else {
                continue;
            };

            // uptimeMillis should be non-decreasing within each chunk
            for window in uptime.values.windows(2) {
                assert!(
                    window[1] >= window[0],
                    "uptimeMillis should not decrease: {} -> {}",
                    window[0],
                    window[1]
                );
            }
            all_uptime_values.extend_from_slice(&uptime.values);
        }
    }

    assert!(
        !all_uptime_values.is_empty(),
        "should find serverStatus.uptimeMillis in at least one chunk"
    );
    // Overall, uptimeMillis should increase across all chunks
    let first = all_uptime_values.first().unwrap();
    let last = all_uptime_values.last().unwrap();
    assert!(
        last > first,
        "uptimeMillis should increase across chunks: {first} -> {last}"
    );
}
