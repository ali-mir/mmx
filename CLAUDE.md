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
│       ├── examples/
│       │   └── dump.rs        # Diagnostic: dump formatted metrics to stdout
│       └── src/
│           ├── main.rs         # Entry point, clap CLI, Timeline, event loop
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
cargo test                     # Run all tests (49 tests)
cargo clippy -- -D warnings    # Lint with strict warnings
cargo fmt --check              # Check formatting
cargo run --bin mmx -- <path>  # Run the TUI with an FTDC file/directory
```

## Architecture

### FTDC Parser (`mmx-ftdc`)

Standalone library crate. No TUI dependencies. Decoding pipeline:

1. **Varint** (`varint.rs`): LEB128 unsigned varint decoder
2. **BSON Flatten** (`bson_ext.rs`): Depth-first traversal extracting numeric fields with dot-separated paths. Handles Bool, Int32, Int64, Double, DateTime, Timestamp (split into .t/.i), Decimal128, Array
3. **Chunk Decode** (`chunk.rs`): Parse type-1 FTDC docs: read uncompressed_size → zlib decompress → parse reference BSON → read metric_count/sample_count → decode varint delta stream with zero-RLE → zigzag decode → cumulative sum
4. **File Reader** (`reader.rs`): Read sequential BSON documents, classify by type field (0=metadata, 1=metric, 2=metadata delta), decode metric chunks. Supports file tailing via `TailingReader`

### TUI (`mmx-tui`)

Elm architecture (TEA pattern):
- **Model**: `App` struct in `app.rs` — metrics, table state, pinned set, filter, mode
- **Update**: `App::update(msg)` — pure state transitions driven by `Message` enum
- **View**: `ui/mod.rs` — dispatches to header/pinned/metrics/footer renderers

Data pipeline: `Timeline` struct flattens all FTDC samples across chunks into a linear sequence. On each tick, the cursor advances one sample, wrapping for replay.

Event loop: tokio async with separate tick (1s, advance sample) and render (100ms, screen draw) intervals.

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

## Done

- [x] Phase 1: Project scaffolding (workspace, deps, .gitignore)
- [x] Phase 2a: Varint decoder (LEB128)
- [x] Phase 2b: BSON flattening (Bool, Int32, Int64, Double, DateTime, Timestamp, Decimal128, Document, Array)
- [x] Phase 2c: Chunk decoder (zlib + delta + zigzag + zero-RLE)
- [x] Phase 2d: File reader (BSON doc iteration, type classification, directory scan, tailing)
- [x] Phase 2e: Integration test with real FTDC data
- [x] Phase 3a: Terminal setup (clap CLI, alternate screen, panic hook, clean shutdown)
- [x] Phase 3b: Event handler (async tick/render/key via tokio)
- [x] Phase 3c: App state (Elm architecture, state transitions, pin/unpin, filter)
- [x] Phase 3d: Layout (header, pinned, metrics, footer)
- [x] Phase 4a: Metric table (scrollable, sorted, scrollbar, delta column)
- [x] Phase 4b: Pin/unpin with pinned panel
- [x] Phase 4c: Search/filter mode
- [x] Phase 4d: Data pipeline (load FTDC, tick refresh)
- [x] Phase 5a: Theme (htop-inspired color palette)
- [x] Phase 5b: Metric formatting (bytes, durations, numbers)
- [x] Phase 5c: Help overlay

## TODO

- [ ] Hide zero-valued metrics: omit metrics whose current value is 0 to reduce noise
- [ ] Section pinning: `/pin <prefix>` to pin all metrics under a section (e.g. `/pin replication`) to the top
- [ ] Sparkline charts: inline mini charts for metric history (data model stores last 300 samples)
- [ ] Live server polling: `mmx --uri mongodb://localhost:27017` via `serverStatus` polling
- [ ] Metric source trait: abstract data source behind `trait MetricSource`
- [ ] TUI snapshot tests: use ratatui `TestBackend` for rendered output assertions
- [ ] File watcher: use `notify` crate to detect new/rotated FTDC files instead of polling

## Testing with Live FTDC Data

The test data in `test-data/diagnostic.data/` is **static** — no live mongod writes to it. To test with real, continuously-updating FTDC data:

```bash
# Start a temporary mongod with a CRUD workload (120s duration)
./scripts/generate_load.sh -d 120 /Users/ali/dev/mongodb/bin

# Script output will show the dbpath, e.g.:
#   dbpath: /tmp/mongo-ftdc-XXXXXX
# Use that path's diagnostic.data/ directory:
./target/release/mmx /tmp/mongo-ftdc-XXXXXX/diagnostic.data/
```

**Important notes:**
- The binary name is `mmx` (NOT `mmx-tui`) — see `[[bin]]` in `crates/mmx-tui/Cargo.toml`
- The script requires `mongod` and `mongosh` in the provided bin directory
- FTDC interim file is rewritten by mongod every ~10 seconds with accumulated samples
- Use `--dump` flag for non-TUI diagnostic output: `./target/release/mmx --dump <path>`
- The script runs in foreground; use `&` or a separate terminal

## Known Bugs

None currently.
