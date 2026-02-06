# mmx — mongometrics

An htop-like TUI for exploring MongoDB FTDC (Full Time Diagnostic Data Capture) files.

```
┌─────────────────────────────────────────────────────┐
│ mmx | ./diagnostic.data | 487 metrics | 1024 chunks │
├─────────────────────────────────────────────────────┤
│ ┌ Pinned ─────────────────────────────────────────┐ │
│ │ serverStatus.connections.current    42       +1  │ │
│ │ serverStatus.opcounters.query       1.5K    +12  │ │
│ └─────────────────────────────────────────────────┘ │
│ ┌ Metrics (487) ──────────────────────────────────┐ │
│ │ Metric Path                     Value    Delta  │ │
│ │ serverStatus.asserts.msg        0        -      │ │
│ │ serverStatus.asserts.regular    0        -      │ │
│ │ serverStatus.asserts.user       3        +1     │ │
│ │ serverStatus.connections.avail  838      -1     │ │
│ │ ...                                             │ │
│ └─────────────────────────────────────────────────┘ │
│ q:quit j/k:nav p:pin /:search Tab:focus ?:help      │
└─────────────────────────────────────────────────────┘
```

## Prerequisites

- Rust 1.85+ (`rustup` recommended: https://rustup.rs)

## Build

```bash
git clone <repo-url> && cd mmx

# Debug build
cargo build

# Release build (optimized)
cargo build --release

# Install to ~/.cargo/bin
cargo install --path crates/mmx-tui
```

The binary is called `mmx` and will be at `target/release/mmx` (or `target/debug/mmx`).

## Usage

```bash
# Point at a single FTDC file
mmx /data/db/diagnostic.data/metrics.2024-01-01T00-00-00Z-00000

# Point at a diagnostic.data directory (reads all files)
mmx /data/db/diagnostic.data
```

## Keybindings

| Key | Action |
|-----|--------|
| `q` / `Ctrl+C` | Quit |
| `j` / `↓` | Move down |
| `k` / `↑` | Move up |
| `g` / `Home` | Jump to top |
| `G` / `End` | Jump to bottom |
| `p` | Pin/unpin metric |
| `/` | Search/filter |
| `Esc` | Clear search |
| `?` | Help overlay |
| `Tab` | Switch pinned/main focus |

## Architecture

```
mmx/
├── crates/
│   ├── mmx-ftdc/    # FTDC parser library (standalone, no TUI deps)
│   └── mmx-tui/     # TUI binary
└── test-data/       # Sample FTDC files
```

### FTDC Parser Pipeline

The `mmx-ftdc` crate implements the full FTDC binary format decoding:

1. **Read** sequential BSON documents from file
2. **Classify** by type: metadata (0), metric chunk (1), metadata delta (2)
3. **Decompress** metric chunks (zlib)
4. **Parse** reference BSON document → flatten to numeric fields with dot-separated paths
5. **Decode** varint delta stream with zero-RLE encoding
6. **Reconstruct** values via zigzag decode + cumulative sum

### TUI Architecture

Follows the Elm (TEA) pattern:

- **Model** (`app.rs`): Centralized `App` state with metrics, table state, pinned set, filter, mode
- **Update** (`app.rs`): Pure `update(Message)` state transitions
- **View** (`ui/`): Stateless rendering functions that read `App` state

The event loop runs on tokio with separate rates:
- **Tick** (1s): Reload FTDC data from disk
- **Render** (100ms): Redraw the terminal for smooth interaction

### Value Formatting

Metric values are formatted based on path heuristics:
- Byte metrics (`*.bytes*`, `*.memory*`) → `1.0 GiB`, `256 KiB`
- Duration metrics (`*millis*`, `*micros*`) → `1.5s`, `500ms`
- Counters → `1.5M`, `10.0K`

## Generating Test Data

If you don't have local FTDC files, the `scripts/` directory can spin up a
temporary mongod, run a workload against it, and copy the resulting FTDC data into `test-data/`.

**Requirements:** A local MongoDB build (or install) with `mongod` and `mongosh` binaries.

```bash
# Basic — runs a default 60s workload with 8 parallel workers
./scripts/generate_load.sh /path/to/mongo/bin

# shorter run
./scripts/generate_load.sh -d 30 /path/to/mongo/bin

# heavier workload — 2 min, 16 parallel workers, 1KB docs
./scripts/generate_load.sh -d 120 -w 16 -s 1024 /path/to/mongo/bin

# use a different port (if 27017 is taken)
./scripts/generate_load.sh -p 27018 /path/to/mongo/bin
```

| Option | Default | Description |
|--------|---------|-------------|
| `-d`, `--duration` | `60` | Workload duration in seconds |
| `-w`, `--workers` | `8` | Concurrent worker threads |
| `-s`, `--doc-size` | `512` | Approximate document payload size in bytes |
| `-p`, `--port` | `27017` | mongod listen port |

The script (`generate_load.sh`) handles the full lifecycle automatically:

1. Starts a temporary `mongod` with a replica set in a temp directory
2. Runs `workload.js` via `mongosh` — a randomized CRUD workload (inserts,
   finds, updates, deletes) with configurable concurrency, duration, and
   document size
3. Copies the resulting `diagnostic.data/` directory into `test-data/`
4. Shuts down and cleans up the mongod on exit (including on Ctrl+C)

The FTDC files land in `test-data/diagnostic.data/` and can be viewed with:

```bash
cargo run -- test-data/diagnostic.data
```

## Development

```bash
cargo build                    # Build
cargo test                     # Test (44 tests)
cargo clippy -- -D warnings    # Lint
cargo fmt                      # Format
```

## License

MIT
