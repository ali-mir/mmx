# mongometrics (mmx)

A real-time terminal dashboard for MongoDB. Polls `serverStatus` once per second
and renders a Grafana-style chart grid with a searchable metric drawer below.

## Project Structure

Single crate at the repo root (no workspace).

```
mmx/
├── Cargo.toml
├── scripts/
│   ├── generate_load.sh    # spin up local mongod + CRUD workload for testing
│   └── workload.js
└── src/
    ├── main.rs             # CLI, terminal setup, event loop, key dispatch
    ├── source.rs           # MetricSource trait + ServerStatusSource (mongo driver)
    ├── bson_ext.rs         # flatten serverStatus BSON → Vec<(path, i64)>
    ├── metric.rs           # MetricKind classification, ring buffer, rate computation
    ├── app.rs              # App state (Elm) + Message + update()
    ├── event.rs            # async tick/render/key event handler
    ├── format.rs           # human-readable value / rate formatting
    ├── theme.rs            # color palette + style constants
    └── ui/
        ├── mod.rs          # top-level layout + help overlay
        ├── header.rs       # title bar (URI, host, version, polls, connection state)
        ├── chart.rs        # Chart panel + dashboard grid + default Panel definitions
        ├── pinned.rs       # pinned metrics drawer
        ├── metrics.rs      # scrollable metric drawer with rate column
        └── footer.rs       # keybinding hint bar
```

## Build & Test Commands

```bash
cargo build                                   # Build
cargo test                                    # Run all tests (33 tests)
cargo clippy --all-targets -- -D warnings     # Lint with strict warnings
cargo fmt --check                             # Check formatting
cargo run -- --uri mongodb://localhost:27017  # Run the TUI against a mongod
cargo run -- --uri mongodb://localhost:27017 --probe  # Connect, poll once, print, exit
```

## Architecture

### Metric Source (`source.rs`)

`trait MetricSource { async fn poll(&mut self) -> Result<Sample, PollError> }`.
The only implementation in v1 is `ServerStatusSource`:

1. `connect(SourceConfig)` parses the URI, builds a `mongodb::Client` with
   fail-fast timeouts (`server_selection_timeout=2s`, `connect_timeout=3s`).
2. `poll()` calls `admin.run_command({serverStatus: 1})` with the configured
   read preference (default: primary), then runs the result through
   `bson_ext::flatten_bson`.
3. Errors are classified as `Transient` (network / SDAM / pool — keep polling)
   or `Fatal` (auth — stop polling).

The driver handles SDAM and reconnection transparently; we just surface the
state to the user via the header banner.

### Data Pipeline

```
1Hz tokio interval
    └─ source.poll() ─┐
                      ▼
              Event::Sample  →  App::SampleArrived  →  merge_sample()
                                                          │
                                                          ▼
                            for each (path, value):
                            • update existing.previous = current
                            • push (now, value) into history ring (cap 900)
                            • on first sight: classify(path) → Counter | Gauge
```

### TUI (Elm / TEA)

- **Model**: the `App` struct in `app.rs`
- **Update**: pure `App::update(Message)` transitions
- **View**: stateless `ui::*` render functions

Two tokio intervals drive the event loop: 1s app tick and 100ms render. The
polling task pushes `Event::Sample` / `Event::PollFailed` / `Event::PollFatal`
into the same channel as keyboard events; everything is dispatched through
`Message`.

### Charts

- `ui::chart::default_panels()` returns the static 6-panel dashboard
  (`opcounters`, `connections`, `network`, `queues`, `WT cache`, `memory`).
- Each `Panel` is a list of `PanelSeries { path, label, color, kind }` — kind
  is `Rate` (counter delta) or `Value` (gauge).
- Per render: each series resolves its history into a `Vec<(t_ago_secs, y)>`,
  feeds it into a `ratatui::widgets::Chart` with Braille markers.
- X-axis bounds reflect the user-selected window (1m / 5m / 15m); Y-axis
  auto-scales to data.
- Pressing `1`-`6` expands a panel to the full chart area. `Esc` collapses.

### Counter vs Gauge

`metric::classify(path)` is heuristic: a curated list of gauge suffixes
(`.current`, `.available`, `.resident`, `.activeReaders`, …) plus prefixes
(`mem.`, `connections.`, `globalLock.`) labels gauges; everything else is a
counter. Misclassifications surface as weird rate numbers (never crashes) —
adjust the lists in `metric.rs` if you spot one.

Rates are computed from the last two samples in the ring buffer:
`(v[n] - v[n-1]) / (t[n] - t[n-1]).as_secs_f64()`. Negative deltas (mongod
restart) → skip the rate this tick.

### Key Design Decisions

- **Single crate**: the FTDC parser was deleted; no consumers, the only
  reusable piece (`bson_ext::flatten_bson`) lives in the binary. Code is
  preserved in git history if needed.
- **Polling on a separate task**: a slow poll doesn't stall the UI. The main
  event loop just receives `Sample` events.
- **Fail-fast driver timeouts**: a missed tick should surface in ~2s, not 30s.
- **Static dashboard**: the panel list is a `&'static [Panel]`; runtime config
  (TOML, etc.) is intentionally deferred.

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
| `Tab` | Switch focus between pinned and main drawer |
| `Space` | Pause/resume polling |
| `+` / `-` | Cycle chart time window (1m / 5m / 15m) |
| `1`–`6` | Expand the corresponding panel to fullscreen |
| `Esc` | Collapse panel / clear search / close help |
| `?` | Toggle help overlay |

## Dependencies

- `mongodb` 3 — official Rust driver (rustls TLS, snappy + zstd compression)
- `bson` 2 — BSON document parsing
- `ratatui` 0.29 + `crossterm` 0.28 — TUI rendering and terminal control
- `tokio` 1 — async runtime
- `clap` 4 — CLI argument parsing
- `color-eyre` 0.6 — error handling + panic recovery
- `libc` — `gettimeofday`/`localtime_r` for the header timestamp

## Done

- [x] Phase 1: Single-crate restructure; mongo driver wired; live polling end-to-end via `--probe` and TUI
- [x] Phase 2: Per-metric history ring (900 samples = 15 min) and rate computation; counter/gauge classification
- [x] Phase 3: Single Chart widget validated for opcounters
- [x] Phase 4: 3×2 panel grid (opcounters / connections / network / queues / WT cache / memory)
- [x] Phase 5: Pause (`Space`), time-window cycle (`+`/`-`), panel expand (`1`-`6` / `Esc`)
- [x] Phase 6: Polish — panel index hints, fail-fast driver timeouts, banner connection states
- [x] Phase 7: Docs

## TODO

- [ ] User-defined dashboards (TOML config) so the panel list isn't `&'static`
- [ ] Per-frame point caching: rebuild `(t,v)` vectors only when a sample arrives, not every render
- [ ] Smarter counter/gauge classification: monotonicity heuristic for unknowns; learn from data
- [ ] `r` to force-reconnect (dropping the driver pool)
- [ ] Replica set member list panel (poll `replSetGetStatus` as a second `MetricSource`)
- [ ] Average latency (`opLatencies.{reads,writes,commands}.latency / .ops`) — needs a "ratio" series type
- [ ] FTDC archive replay mode (load a `diagnostic.data` directory and step through it offline)

## Testing with Live mongod

The static `test-data/` directory is gone — there is no on-disk fixture. To
exercise the dashboard end-to-end, run a local mongod and point mmx at it.

```bash
# In one terminal: spin up mongod + a CRUD workload (script blocks for the duration)
./scripts/generate_load.sh -d 120 /Users/ali/dev/mongodb/bin

# In another terminal: connect mmx
./target/release/mmx --uri "mongodb://127.0.0.1:27017/?directConnection=true"
```

For a no-mongosh smoke check (the binary path used during development):

```bash
DBPATH=$(mktemp -d /tmp/mongo-mmx-XXXXXX)
/Users/ali/dev/mongodb/bin/mongod --port 27117 --dbpath "$DBPATH" --bind_ip 127.0.0.1 --quiet > "$DBPATH/mongod.log" 2>&1 &
./target/release/mmx --uri "mongodb://127.0.0.1:27117/?directConnection=true" --probe
```

`--probe` prints metric counts and a few well-known values, then exits — no TTY
required.

## Known Bugs

None currently.
