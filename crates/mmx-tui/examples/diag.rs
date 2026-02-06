//! Diagnostic: show chunk/sample structure of FTDC data.
//! Usage: cargo run -p mmx-tui --example diag -- /path/to/diagnostic.data

use std::io::BufReader;
use std::path::PathBuf;

fn main() {
    let path = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .expect("Usage: diag <path-to-ftdc>");

    let files = mmx_ftdc::reader::find_ftdc_files(&path).expect("find files");
    println!("Files: {}", files.len());

    let mut total_samples = 0usize;
    for f in &files {
        println!("  {}", f.display());
        let file = std::fs::File::open(f).expect("open");
        let r = BufReader::new(file);
        match mmx_ftdc::reader::read_ftdc_file(r) {
            Ok(chunks) => {
                for (i, c) in chunks.iter().enumerate() {
                    let nsamp = if c.metrics.is_empty() {
                        0
                    } else {
                        c.metrics[0].values.len()
                    };
                    total_samples += nsamp;
                    println!(
                        "    chunk[{i}]: {nsamp} samples x {} metrics",
                        c.metrics.len()
                    );
                }
            }
            Err(e) => println!("    skip: {e}"),
        }
    }
    println!("\nTotal samples: {total_samples}");
}
