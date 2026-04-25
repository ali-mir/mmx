# mmx — mongometrics

A real-time terminal dashboard for MongoDB metrics. Polls `serverStatus` once per second
and renders a grid of charts plus a searchable metric drawer.

```
┌ mmx │ 127.0.0.1:27017 │ Surfboard.local v8.2.7 │ 3196 metrics │ polls 47 │ last 0s ago │ 14:32:07 │ window 5m │ ● connected ─┐
│ ┌ ops/s [1] ──────────────────────┐ ┌ connections [2] ────────────────┐ ┌ network [3] ────────────────────┐ │
│ │ insert  124/s        ▁▂▃▂▁▂▃▄▄ │ │ current  87  ▁▁▂▃▃▃▄▄▄▄▄▄▄      │ │ in    12.4 MB/s ▂▃▄▅▆▆▇▇▇▆▆▅▄▃ │ │
│ │ query 2,345/s        ▁▃▅▆▇▇▇▆▅ │ │ active   42  ▁▁▁▂▃▃▃▃▃▃▄▄▄      │ │ out    3.1 MB/s ▁▂▃▄▄▄▄▄▄▄▄▃▂▁ │ │
│ └─────────────────────────────────┘ └─────────────────────────────────┘ └─────────────────────────────────┘ │
│ ┌ queues [4] ─────────────────────┐ ┌ WiredTiger cache [5] ───────────┐ ┌ memory (MiB) [6] ───────────────┐ │
│ │ readers      0                  │ │ in cache  62 MiB ▁▂▃▃▄▄▄▄▄▄▄    │ │ resident  248                   │ │
│ │ writers      2  ▁▁▁▂▃▂▁▁▁       │ │ dirty     18 MiB ▁▂▂▃▂▁▁▁▁      │ │ virtual   2.1 GiB               │ │
│ └─────────────────────────────────┘ └─────────────────────────────────┘ └─────────────────────────────────┘ │
│ ┌ Metrics (3196) ────────────────────────────────────────────────────────────────────────────────────────┐ │
│ │ Metric Path                                            Value         Rate                              │ │
│ │ * opcounters.insert                                    1,245       +124/s                              │ │
│ │   opcounters.query                                    23,401     +2,345/s                              │ │
│ │   ...                                                                                                  │ │
│ └────────────────────────────────────────────────────────────────────────────────────────────────────────┘ │
│ q:quit j/k:nav p:pin /:search Space:pause +/-:window 1-6:expand ?:help                                     │
└────────────────────────────────────────────────────────────────────────────────────────────────────────────┘
```

## Prerequisites

- Rust 1.85+ (`rustup` recommended: https://rustup.rs)
- A reachable mongod

## Build

```bash
git clone <repo-url> && cd mmx

# Debug build
cargo build

# Release build (optimized)
cargo build --release

# Install to ~/.cargo/bin
cargo install --path .
```

The binary is `mmx`; it lands at `target/release/mmx` (or `target/debug/mmx`).

## Usage

```bash
# Single host
mmx --uri mongodb://127.0.0.1:27017/?directConnection=true

# Replica set / SRV — picks a host based on the read preference (default: primary)
mmx --uri "mongodb+srv://user:pw@cluster0.mongodb.net" --read-preference secondaryPreferred

# Connect, poll once, print a summary, exit (useful for sanity-checking a URI)
mmx --uri mongodb://localhost:27017 --probe

# Slow it down or speed it up
mmx --uri mongodb://localhost:27017 --interval 2s
```

Useful flags:

| Flag | Default | Description |
|------|---------|-------------|
| `--uri` | (required) | MongoDB connection string |
| `--interval` | `1s` | Poll interval (`ms`/`s`/`m` units) |
| `--read-preference` | `primary` | `primary`, `primary-preferred`, `secondary`, `secondary-preferred`, `nearest` |
| `--connect-timeout` | `3s` | Driver TCP connect timeout |
| `--server-selection-timeout` | `2s` | Driver SDAM timeout — kept low so a missed tick fails fast |
| `--app-name` | `mmx` | Sent as `appName` so it shows up in `db.currentOp()` |
| `--tls-allow-invalid-certs` | off | Insecure TLS bypass for local dev |
| `--probe` | off | Connect, poll once, print a summary, and exit |

## Keybindings

| Key | Action |
|-----|--------|
| `q` / `Ctrl+C` | Quit |
| `j` / `↓` | Move selection down (in metric drawer) |
| `k` / `↑` | Move selection up |
| `g` / `Home` | Jump to top |
| `G` / `End` | Jump to bottom |
| `p` | Pin/unpin selected metric |
| `/` | Search/filter |
| `Esc` | Collapse panel / clear search / close help |
| `Tab` | Switch focus between Pinned and Main drawer |
| `Space` | Pause/resume polling |
| `+` / `-` | Cycle chart time window: 10s → 30s → 1m → 5m → 10m (default 10s) |
| `1`–`6` | Expand the corresponding panel to fullscreen |
| `?` | Toggle help overlay |

## Architecture

```
mmx/
├── Cargo.toml
└── src/
    ├── main.rs          # CLI, terminal setup, event loop, key handling
    ├── source.rs        # MetricSource trait + ServerStatusSource
    ├── bson_ext.rs      # Flatten serverStatus BSON → (path, i64) pairs
    ├── metric.rs        # MetricKind classification, history ring, rate computation
    ├── app.rs           # App state (Elm) + Message + update()
    ├── event.rs         # Async tick/render/key event handler
    ├── format.rs        # Human-readable value/rate formatting
    ├── theme.rs         # Color palette
    └── ui/
        ├── mod.rs       # Top-level layout + help overlay
        ├── header.rs    # Title bar (URI, host, version, polls, connection state)
        ├── chart.rs     # Chart panel + dashboard grid + Panel definitions
        ├── pinned.rs    # Pinned metrics drawer
        ├── metrics.rs   # Scrollable metric drawer with rate/delta column
        └── footer.rs    # Keybinding hint bar
```

### Polling Pipeline

1. `ServerStatusSource::connect` parses the URI and builds a `mongodb::Client`
   with fail-fast timeouts (`server_selection_timeout=2s`).
2. A tokio interval task calls `db.run_command({serverStatus: 1})` every
   `--interval`, classifying any error as `Transient` (banner + retry) or
   `Fatal` (banner + stop).
3. The returned `bson::Document` is flattened by `bson_ext::flatten_bson`
   into a flat `Vec<(path, i64)>`.
4. `App::merge_sample` updates the per-metric state and pushes a timestamped
   value into a 900-entry ring buffer (15 min @ 1 Hz).
5. The render task draws the dashboard at 100ms.

### Counter vs Gauge

`metric::classify` heuristically labels each path as a counter or gauge based
on suffix and prefix patterns (e.g. `connections.current` → gauge,
`opcounters.insert` → counter). The drawer shows counters as a per-second
**rate**, gauges as a one-tick **delta**. Negative deltas on a counter are
treated as a mongod restart and skipped.

### TUI Architecture

Standard Elm (TEA) pattern:

- **Model** (`app.rs`): the `App` struct
- **Update** (`app.rs`): pure `update(Message)` transitions
- **View** (`ui/`): stateless render functions that read `App`

Two tokio intervals drive the loop: a 1s app tick (UI age refresh) and a 100ms
render. Polling runs on a separate task and pushes `Sample` / `PollFailed` /
`PollFatal` into the same channel as keyboard input.

## Local Testing with mongod

`scripts/generate_load.sh` spins up a temporary `mongod` and runs a CRUD workload
against it — useful for exercising the dashboard with real, moving counters.

**Requirements:** local MongoDB binaries (`mongod`, `mongosh`).

```bash
# In one terminal: start mongod + a 120s workload
./scripts/generate_load.sh -d 120 /path/to/mongo/bin
# (note the printed dbpath / port)

# In another terminal: point mmx at it
./target/release/mmx --uri "mongodb://127.0.0.1:27017/?directConnection=true"
```

| Option | Default | Description |
|--------|---------|-------------|
| `-d`, `--duration` | `60` | Workload duration in seconds |
| `-w`, `--workers` | `8` | Concurrent worker threads |
| `-s`, `--doc-size` | `512` | Approximate document payload size in bytes |
| `-p`, `--port` | `27017` | mongod listen port |

## Development

```bash
cargo build                            # Build
cargo test                             # 33 tests
cargo clippy --all-targets -- -D warnings   # Lint
cargo fmt                              # Format
```

## License

MIT
