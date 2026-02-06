# mongometrics (mmx)

An htop-like CLI tool that parses MongoDB FTDC (Full Time Diagnostic Data Capture) files and presents metrics in a real-time, colorful TUI.

## Project Structure

Cargo workspace with two crates:

```
mmx/
├── Cargo.toml              # workspace root (resolver = "3", edition 2024)
├── crates/
│   ├── mmx-ftdc/           # FTDC parser library (no TUI dependency)
│   │   └── src/
│   │       ├── lib.rs          # Re-exports all modules
│   │       ├── varint.rs       # Unsigned LEB128 varint decoder
│   │       ├── bson_ext.rs     # BSON flattening (extract numeric fields with dot paths)
│   │       ├── chunk.rs        # Metric chunk decoding (decompress + delta + zigzag)
│   │       └── reader.rs       # File-level FTDC reader (iterate documents, tailing)
│   └── mmx-tui/            # TUI application binary
│       └── src/
│           ├── main.rs         # Entry point, clap CLI, terminal setup, event loop
│           ├── app.rs          # App state (Elm architecture) + update logic
│           ├── event.rs        # Async event handler (tick/render/key via tokio)
│           ├── format.rs       # Human-readable value formatting (bytes, durations, numbers)
│           ├── theme.rs        # Color palette + style constants
│           └── ui/
│               ├── mod.rs      # Top-level layout + help overlay
│               ├── header.rs   # Title bar (file path, metric count, chunk count)
│               ├── pinned.rs   # Pinned metrics panel
│               ├── metrics.rs  # Scrollable metric table with scrollbar
│               └── footer.rs   # Keybinding help bar
└── test-data/              # Sample FTDC files for testing
```

## Build & Test Commands

```bash
cargo build                    # Build all crates
cargo test                     # Run all tests (40 tests)
cargo clippy -- -D warnings    # Lint with strict warnings
cargo fmt --check              # Check formatting
cargo run --bin mmx -- <path>  # Run the TUI with an FTDC file/directory
```

## Architecture

### FTDC Parser (`mmx-ftdc`)

Standalone library crate. No TUI dependencies. Decoding pipeline:

1. **Varint** (`varint.rs`): LEB128 unsigned varint decoder
2. **BSON Flatten** (`bson_ext.rs`): Depth-first traversal extracting numeric fields with dot-separated paths. Handles Bool, Int32, Int64, Double, DateTime, Timestamp (split into .t/.i)
3. **Chunk Decode** (`chunk.rs`): Parse type-1 FTDC docs: read uncompressed_size → zlib decompress → parse reference BSON → read metric_count/sample_count → decode varint delta stream with zero-RLE → zigzag decode → cumulative sum
4. **File Reader** (`reader.rs`): Read sequential BSON documents, classify by type field (0=metadata, 1=metric, 2=metadata delta), decode metric chunks. Supports file tailing via `TailingReader`

### TUI (`mmx-tui`)

Elm architecture (TEA pattern):
- **Model**: `App` struct in `app.rs` — metrics, table state, pinned set, filter, mode
- **Update**: `App::update(msg)` — pure state transitions driven by `Message` enum
- **View**: `ui/mod.rs` — dispatches to header/pinned/metrics/footer renderers

Event loop: tokio async with separate tick (1s, data refresh) and render (100ms, screen draw) intervals.

### Key Design Decisions

- **Workspace split**: Parser is independently testable and reusable
- **Separate tick/render rates**: Data at 1s (FTDC sample rate), UI at 100ms (smooth interaction)
- **TestBackend for TUI tests**: Ratatui's `TestBackend` enables snapshot testing

## Keybindings

| Key | Action |
|-----|--------|
| `q` / `Ctrl+C` | Quit |
| `j` / `Down` | Move selection down |
| `k` / `Up` | Move selection up |
| `g` / `Home` | Jump to top |
| `G` / `End` | Jump to bottom |
| `p` | Pin/unpin selected metric |
| `/` | Enter search/filter mode |
| `Esc` | Clear search / close overlay |
| `?` | Toggle help overlay |
| `Tab` | Switch focus between pinned and main |

## Dependencies

### mmx-ftdc
- `bson` — BSON document parsing
- `flate2` — Zlib decompression

### mmx-tui
- `ratatui` + `crossterm` — TUI rendering
- `tokio` — Async runtime
- `clap` — CLI argument parsing
- `color-eyre` — Error handling + panic recovery
- `notify` — File watching for FTDC rotation

## Future Work

- **Sparkline charts**: Inline mini charts for metric history (data model already stores last 300 samples)
- **Live server polling**: `mmx --uri mongodb://localhost:27017` via `serverStatus` polling
- **Metric source trait**: Abstract data source behind `trait MetricSource`
