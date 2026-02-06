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
