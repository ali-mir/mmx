//! Quick diagnostic: loads FTDC data and prints the formatted metrics table.
//! Usage: cargo run -p mmx-tui --example dump -- test-data/diagnostic.data

use std::io::BufReader;
use std::path::PathBuf;

use mmx_ftdc::reader;

// Need access to format module from the mmx-tui crate
// Since it's a binary crate, we inline the format logic here minimally
// by just reimporting from the binary's source
include!("../src/format.rs");

fn main() {
    let path = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .expect("Usage: dump <path-to-ftdc>");

    let files = reader::find_ftdc_files(&path).expect("find files");
    println!("Files: {}", files.len());

    let mut all_chunks = Vec::new();
    for f in &files {
        let file = std::fs::File::open(f).expect("open");
        let r = BufReader::new(file);
        match reader::read_ftdc_file(r) {
            Ok(chunks) => all_chunks.extend(chunks),
            Err(e) => eprintln!("  skip {}: {e}", f.display()),
        }
    }

    println!("Chunks: {}", all_chunks.len());

    let Some(last) = all_chunks.last() else {
        println!("No metric chunks found.");
        return;
    };

    println!("Metrics: {}", last.metrics.len());
    println!(
        "Samples in last chunk: {}",
        last.metrics.first().map(|m| m.values.len()).unwrap_or(0)
    );
    println!();
    println!("{:<70} {:>20} {:>15}", "METRIC PATH", "VALUE", "FORMATTED");
    println!("{}", "-".repeat(107));

    for m in &last.metrics {
        let current = *m.values.last().unwrap_or(&0);
        let formatted = format_value(&m.path, current);
        println!("{:<70} {:>20} {:>15}", m.path, current, formatted);
    }
}
