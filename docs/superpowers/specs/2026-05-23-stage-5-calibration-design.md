# Stage 5 — Calibration Math + Local Cache Design

Companion to the top-level [rust-tray-widget-design.md](2026-05-22-rust-tray-widget-design.md). This document fixes the details that the top-level spec left open for Stage 5.

## Goal

Replace the tooltip's API-only utilization reading with a locally-computed alternative. v0.5.0 reads `~/.claude/projects/**/*.jsonl`, caches parsed turns incrementally (mtime-diff), derives 5h and weekly caps from ≥0.95-util anchors in the existing calibration log, and shows both API and local util side-by-side in the tooltip.

The icon, its renderer, the polling cadence, the threading model, and the shutdown flow all stay unchanged. Only the tooltip surface changes and a parsing/calibration pipeline is added underneath it.

## Non-goals (Stage 5)

- ❌ Cost-weighted aggregation — Stage 6/8 dashboard concern. The cache stores the four raw token counts; cost-weighting is computed at display time later.
- ❌ Plan-tier detection (Pro vs Max5x) — the Stage 1 known quirk. Stage 5 sidesteps it entirely: we use whatever cap the user's actual util implies. No `pro_5h = max5x_5h / 5` fiction is needed because the tooltip never asserts a tier label.
- ❌ Icon changes — `compute_visual` keeps reading API util. Tooltip-only surface.
- ❌ Bootstrap fallback caps — until the first ≥0.95 anchor exists in the log, the new tooltip lines read `local 5h: (uncalibrated)` / `local 7d: (uncalibrated)`.
- ❌ `caps.json` persistence — derived caps are recomputed each poll. Math is fast (median of small N).
- ❌ Stage 8's `derive_continuous_caps` / `implied_cap_series` / `cap_series` — old Python code paths superseded by `global_cap_from_anchors`.
- ❌ Changes to the existing `--once` / `--watch` text output. Stage 2 text behavior carries over verbatim.
- ❌ Changes to the calibration log schema (Stage 2's `CalibrationSample` is sufficient as a read source).

## Locked-in design decisions

Settled during the Stage 5 brainstorm:

| Decision | Value |
|---|---|
| Where calibrated util appears | Tooltip only (new lines below the existing API lines) |
| Bootstrap behavior (no anchors yet) | Tooltip line reads `local: (uncalibrated)` |
| Where parsing runs | On the existing polling thread, before each tick's API fetch |
| Cache file format | `bincode` (rows) + `serde_json` (manifest) |
| Cache location | `~/.claude-usage-tray/cache.bincode` + `~/.claude-usage-tray/cache_manifest.json` |
| Cache schema migration | If `schema_version` doesn't match, delete both files and rebuild |
| 5h burn window | Gap-based detection — port of Python's `caps.py` algorithm (4.5h gap) |
| Weekly burn window | Since most-recent Sunday 07:00 local (per metering-facts memory) |
| Token type used for caps | `output_tokens` only (per metering-facts memory) |
| Anchor utilization range | `0.95 ≤ util ≤ 1.01` |
| Cap aggregation | Median across anchors |
| Hour-of-day cap series (24-bin) | Built ahead of Stage 6 — computed, stashed on `TrayState`, not displayed in v0.5.0 |
| Cost-weighted aggregation | Deferred to Stage 6/8 |

## Data flow

```text
~/.claude/projects/**/*.jsonl ──> data::parser ──┐
                                                  │
~/.claude-usage-tray/cache.bincode ─────> data::cache::refresh ──┐
~/.claude-usage-tray/cache_manifest.json ────────────────────────┘
                                                  │
                                                  ▼
                                          Vec<Turn>
                                                  │
~/.claude-usage-tray/calibration_log.jsonl ──> log::calibration::read_all ──┐
                                                                              │
                                                                              ▼
                                                              calibration::anchors::global_cap_from_anchors
                                                                              │
                                                                              ▼
                                                              DerivedCaps { cap_5h, cap_week }
                                                                              │
                                                                              ▼
                                                              calibration::live::live_util_now
                                                                              │
                                                                              ▼
                                                              tooltip render (new local lines)
```

## Module layout changes from Stage 4

```text
src/
  main.rs             — unchanged
  cli.rs              — unchanged
  config.rs           — NEW: constants module
                          pub const LOCAL_TZ: &str = "Europe/Copenhagen";
                          pub const WEEKLY_RESET_WEEKDAY: Weekday = Weekday::Sun;
                          pub const WEEKLY_RESET_HOUR_LOCAL: u32 = 7;
                          pub const FIVE_HOUR_WINDOW_HOURS: f64 = 4.5;
                          pub const MIN_ANCHOR_UTIL: f64 = 0.95;
                          pub const MAX_ANCHOR_UTIL: f64 = 1.01;
  paths.rs            — ADD: cache_path(), cache_manifest_path()
  api/                — unchanged
  log/                — unchanged (calibration.rs gets a tiny read_all() helper)
  data/               — NEW
    mod.rs
    parser.rs         — JSONL → Turn iterator
    cache.rs          — mtime-diff refresh + bincode (de)serialize
  calibration/        — NEW
    mod.rs
    anchors.rs        — global_cap_from_anchors (5h + weekly)
    hourly.rs         — hour-of-day 24-bin model (built ahead of Stage 6)
    live.rs           — live_util_now (current burn / cap)
  render.rs           — ADD local-util lines to tooltip
  tray/               — wiring only: TrayState gains last_caps + last_local_util,
                        poller.rs refreshes cache + recomputes caps each tick
  watch.rs            — unchanged
```

Two new top-level dirs: `data/` and `calibration/`. Nested (files-as-modules) rather than flat because each splits cleanly into 2–3 focused files; easier to navigate than a single 500-line file.

## Data model

```rust
// data/parser.rs — one row per assistant turn (mirrors Python TurnRow)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Turn {
    pub ts: DateTime<Utc>,                 // parsed from "timestamp" ISO string
    pub session_id: String,
    pub subagent_id: Option<String>,       // from path: subagents/agent-<hex>.jsonl
    pub is_subagent: bool,
    pub project_cwd: String,
    pub model: String,                     // may be ""
    pub version: String,                   // may be ""
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_creation_input_tokens: u64,
    pub cache_read_input_tokens: u64,
    pub source_file: PathBuf,
    pub is_rate_limit_error: bool,
}

// data/cache.rs — serialized to ~/.claude-usage-tray/cache.bincode
#[derive(Serialize, Deserialize)]
struct CacheFile {
    schema_version: u32,                    // start at 1
    turns: Vec<Turn>,
}

// ~/.claude-usage-tray/cache_manifest.json — JSON, not bincode (small, human-inspectable)
#[derive(Serialize, Deserialize)]
struct Manifest {
    schema_version: u32,
    mtimes: HashMap<PathBuf, i64>,          // millis-since-epoch (portable across SystemTime quirks)
}

// calibration/anchors.rs — output of the per-poll calibration step
#[derive(Debug, Clone, Default)]
pub struct DerivedCaps {
    pub cap_5h: Option<f64>,                // output tokens per 5h window at 100%
    pub cap_week: Option<f64>,              // output tokens per weekly window at 100%
    pub n_anchors_5h: usize,
    pub n_anchors_week: usize,
}

// calibration/live.rs — current util computed every poll for the tooltip
#[derive(Debug, Clone, Default)]
pub struct LiveUtil {
    pub util_5h: Option<f64>,               // None when cap_5h is None (uncalibrated)
    pub util_week: Option<f64>,
}

pub enum WindowKind { FiveHour, Weekly }
```

Beginner notes:
- `DateTime<Utc>` not `String` — parse once at JSONL ingest, store typed. `serde` handles serialization via the `chrono` crate's `serde` feature already in `Cargo.toml`.
- `u64` for token counts — Anthropic's API and the JSONL emit non-negative integers. `i64` would invite negative-burn bugs.
- `PathBuf` not `String` for paths — Windows paths can contain non-UTF-8 bytes in theory; `PathBuf` is the idiomatic Rust container.
- Different formats for different lifetimes: cache is hot per-poll (bincode wins on size + speed), manifest is rarely read but useful to debug by hand (JSON wins).

## Parser (`data/parser.rs`)

```rust
pub fn walk_jsonl(root: &Path) -> impl Iterator<Item = PathBuf>;
pub fn iter_rows(path: &Path) -> impl Iterator<Item = Turn>;
```

`iter_rows` mirrors Python `parser.iter_rows`:

1. Open file, read line-by-line, UTF-8 with replacement on invalid bytes (Python uses `errors="replace"`).
2. Skip empty lines and JSON-parse failures silently — JSONL is appended live by `claude` and partial writes happen.
3. Filter: keep rows where `message.usage` is a dict, OR where the row is a rate-limit error (`type ∈ {"api-error", "error"}` AND error.type contains "rate"/"limit" or status == 429).
4. Extract `timestamp` (ISO 8601 → `DateTime<Utc>`), `sessionId`, `cwd`, `version`, `message.model`, the four token counts (defaulting to 0 when missing).
5. Classify subagent by walking the path: `is_subagent = path.components().any(|c| c.as_os_str() == "subagents")`. If true, extract the hex ID from the filename `agent-<hex>.jsonl` via `Path::file_stem` + `strip_prefix("agent-")`. Avoids pulling in the `regex` crate for a single pattern; the Python uses regex but the Rust idiom is cheaper.

Rust idioms unfamiliar after pure Python:
- `impl Iterator<Item = Turn>` instead of returning a `Vec<Turn>` — streams rows so we never hold a whole file in memory.
- `?` on `serde_json::from_str(...)?` would propagate per-line errors; we want to swallow them. Pattern: `let v = match serde_json::from_str::<Value>(line) { Ok(v) => v, Err(_) => continue };`.
- Borrow `&str` for the line buffer inside the loop; only allocate `String`s into the `Turn` fields.

## Cache (`data/cache.rs`)

```rust
pub fn refresh(projects_root: &Path) -> Result<Vec<Turn>, CacheError>;
```

One function does it all (load existing → mtime-diff → reparse changed → write back). Returns the full sorted-by-`ts` `Vec<Turn>`.

Algorithm (port of Python `cache.refresh_cache`):

1. Load `cache.bincode` if it exists, else start with empty `Vec<Turn>`.
2. Load `cache_manifest.json` if it exists, else start with empty mtime map.
3. Walk `projects_root` for `*.jsonl`. For each path, read `metadata.modified()` → millis-since-epoch.
4. Compute three sets:
   - `new_or_changed = { path | manifest.mtimes.get(path) != current_mtime }`
   - `deleted = { path ∈ manifest.mtimes | path ∉ current paths }`
   - `unchanged = everything else`
5. If `new_or_changed ∪ deleted` is empty → return loaded turns unchanged (still sorted).
6. Drop rows from loaded turns whose `source_file ∈ (new_or_changed ∪ deleted)`.
7. Reparse `new_or_changed` files via `iter_rows`, append.
8. Sort the resulting `Vec<Turn>` by `ts` once at the end.
9. Write `CacheFile { schema_version: 1, turns }` to `cache.bincode` (write-temp-then-rename for atomicity).
10. Write updated `Manifest` to `cache_manifest.json`.
11. Return turns.

Schema migration policy: if `cache.bincode`'s `schema_version` ≠ current, delete both files and full-rebuild. Simpler than versioned conversion; cost is one full reparse per release.

## Calibration math — global cap (`calibration/anchors.rs`)

```rust
pub fn global_cap_from_anchors(
    log: &[CalibrationSample],
    turns: &[Turn],                    // sorted by ts
    kind: WindowKind,
) -> (Option<f64>, usize);              // (median_cap, n_anchors)
```

Anchors are samples where `MIN_ANCHOR_UTIL ≤ util ≤ MAX_ANCHOR_UTIL` (`0.95 ≤ util ≤ 1.01`). For each anchor we compute `implied_cap = burn_in_window(anchor.ts) / util`, summing `output_tokens`, then return the median across anchors (or `None` if zero anchors).

### 5h burn window — gap-based detection (port of Python)

The window starts at the most recent "session start", defined as the first turn after either (a) a ≥4.5h gap from the previous turn or (b) the current window has been open ≥4.5h. This matches the Python chart's window logic and the empirical observation that the 5h cap behaves like a ~4.5h window.

```rust
fn five_hour_burn_at(turns: &[Turn], anchor_ts: DateTime<Utc>) -> u64 {
    let gap = Duration::milliseconds((FIVE_HOUR_WINDOW_HOURS * 3_600_000.0) as i64);
    let mut current_start: Option<DateTime<Utc>> = None;
    let mut last_ts: Option<DateTime<Utc>> = None;
    let mut burn: u64 = 0;
    for t in turns.iter().filter(|t| t.ts <= anchor_ts) {
        match current_start {
            None => current_start = Some(t.ts),
            Some(start) => {
                let since_last  = t.ts - last_ts.unwrap();
                let since_start = t.ts - start;
                if since_last >= gap || since_start >= gap {
                    current_start = Some(t.ts);
                    burn = 0;
                }
            }
        }
        burn += t.output_tokens;
        last_ts = Some(t.ts);
    }
    burn
}
```

### Weekly burn window — fixed Sunday 07:00 local reset

Per the metering-facts memory: util_7d is a fixed weekly window that resets Sunday 07:00 local — NOT rolling 7d.

```rust
fn weekly_burn_at(turns: &[Turn], anchor_ts: DateTime<Utc>) -> u64 {
    let win_start = last_weekly_reset(anchor_ts);    // Sun 07:00 local, in UTC
    turns.iter()
        .filter(|t| t.ts >= win_start && t.ts <= anchor_ts)
        .map(|t| t.output_tokens)
        .sum()
}

fn last_weekly_reset(anchor_ts: DateTime<Utc>) -> DateTime<Utc> {
    let tz: Tz = config::LOCAL_TZ.parse().unwrap();
    let local = anchor_ts.with_timezone(&tz);
    let days_back = local.weekday().num_days_from_sunday() as i64;
    let candidate = local
        .with_hour(config::WEEKLY_RESET_HOUR_LOCAL).unwrap()
        .with_minute(0).unwrap()
        .with_second(0).unwrap()
        .with_nanosecond(0).unwrap()
        - Duration::days(days_back);
    let candidate = if candidate > local { candidate - Duration::days(7) } else { candidate };
    candidate.with_timezone(&Utc)
}
```

Beginner notes:
- `chrono_tz::Tz` parsed from `"Europe/Copenhagen"` literal each call — cheap, no need to cache.
- `with_hour` / `with_minute` etc. return `LocalResult`; in this context they can't fail (we're not setting an invalid hour) but the `unwrap()` is a flag for a future cleanup.
- DST is delegated to `chrono-tz` — no manual offset math.

## Calibration math — hour-of-day cap series (`calibration/hourly.rs`)

Built ahead of Stage 6. Computed and stashed on `TrayState`; not displayed in v0.5.0.

```rust
pub fn hour_of_day_cap_series(
    log: &[CalibrationSample],
    turns: &[Turn],
    kind: WindowKind,
) -> [f64; 24] {
    let raw = per_hour_medians(log, turns, kind);          // [Option<f64>; 24]
    let smoothed = smooth_rolling_circular(&raw, 3);       // [Option<f64>; 24]
    interpolate_empty_circular(&smoothed)                  // [f64; 24]
}
```

Port of Python's `_per_hour_medians` / `_smooth_rolling_circular` / `_interpolate_empty_circular`. All three are pure-Rust array math — no Win32, no I/O, easy to unit-test.

Bin assignment uses local-hour-of-day (`config::LOCAL_TZ`) on the anchor timestamp. `interpolate_empty_circular` handles empty bins via linear interpolation that wraps over midnight.

When zero valid anchors exist, returns `[0.0; 24]`.

## Live util (`calibration/live.rs`)

```rust
pub fn live_util_now(turns: &[Turn], caps: &DerivedCaps) -> LiveUtil {
    let now = Utc::now();
    LiveUtil {
        util_5h:   caps.cap_5h.map(|c|   five_hour_burn_at(turns, now) as f64 / c),
        util_week: caps.cap_week.map(|c| weekly_burn_at(turns, now) as f64 / c),
    }
}
```

When the cap is `None` (no anchors yet), `util_*` is `None` and the tooltip prints `(uncalibrated)`.

## Threading & wiring

No new threads, no new locks, no new channels. The polling thread (Stage 3) gains five steps before its existing API fetch:

```text
poller loop {
    1. let turns = cache::refresh(...)               <- new
    2. let log = log::calibration::read_all()        <- new
    3. let caps = global_cap_from_anchors(...)       <- new
    4. let hourly_5h   = hour_of_day_cap_series(..)  <- new (computed, stashed for Stage 6)
    5. let hourly_week = hour_of_day_cap_series(..)  <- new (computed, stashed for Stage 6)
    6. (existing) api::usage::fetch()
    7. (existing) log::calibration::append() if fetch succeeded
    8. let local = live_util_now(turns, caps)         <- new
    9. send PollEvent { snap, caps, local, hourly_5h, hourly_week, … } over the existing mpsc
   10. PostMessageW(WM_APP_POLL) — existing
   sleep(poll_interval)
}
```

`TrayState` (UI thread) gains:

```rust
pub struct TrayState {
    // ...existing fields...
    pub last_caps: Option<DerivedCaps>,
    pub last_local_util: Option<LiveUtil>,
    pub last_hourly_5h: Option<[f64; 24]>,    // stashed for Stage 6
    pub last_hourly_week: Option<[f64; 24]>,
}
```

Communicated via the existing `mpsc::channel<PollEvent>` — `PollEvent` gains the new fields. The poller does all the heavy work (parse + median) before sending; the UI thread only reads.

## Tooltip integration (`render.rs`)

The Stage 3/4 tooltip currently shows:

```text
5h: 57%   7d: 42%
```

After Stage 5:

```text
5h: 57%        7d: 42%
local 5h: 54%  local 7d: 40%
```

When `caps.cap_5h` is `None`, the local 5h line reads `local 5h: (uncalibrated)`. Same shape for weekly. Tooltip stays under the Windows 127-char limit comfortably.

The existing "(stale Nm)" footer from Stage 3 carries over unchanged.

## Error handling per layer

| Layer | Failure | Behavior |
|---|---|---|
| `parser::iter_rows` | Bad JSON line | Skip, `tracing::trace!` |
| `parser::iter_rows` | Unreadable file | `tracing::warn!`, skip file |
| `cache::refresh` | bincode decode fail | Delete cache + manifest, full reparse, `tracing::warn!` |
| `cache::refresh` | Write fail | Return turns anyway, `tracing::warn!` (next poll retries write) |
| `cache::refresh` | I/O fail walking root | Return `Err`; poller skips Stage-5 work this tick |
| `log::read_all` | Bad JSON line | Skip, `tracing::trace!` (matches append's tolerant model) |
| `log::read_all` | File missing | Return empty Vec (first run) |
| `anchors::global_cap_from_anchors` | Zero anchors | Return `(None, 0)` |
| `hourly::hour_of_day_cap_series` | Zero anchors | Return `[0.0; 24]` |
| Poller | Any Stage-5 step returns `Err` | Skip Stage-5 work this tick, fall through to existing API fetch, tooltip's local line stays at its previous value (or `(uncalibrated)` if never set) |

Stage 5 never aborts the polling loop. The tray icon and API tooltip always work even if local calibration is broken.

## New runtime dependencies

```toml
bincode = "1.3"          # row cache serialization
```

`chrono` already has the `serde` feature enabled. `chrono-tz` and `serde_json` already in `Cargo.toml`. Subagent path classification uses `Path::components` rather than the `regex` crate (see Parser section).

## Testing

| Module | Test type | Notes |
|---|---|---|
| `parser::iter_rows` | Unit + fixture | `tests/fixtures/sample_session.jsonl` (anonymized) — turn with usage, corrupt line, rate-limit error row, subagent file. ~6 tests. |
| `parser::walk_jsonl` | Unit | tmpdir with nested *.jsonl files. |
| `cache::refresh` | Unit + tmpdir | First-run builds cache. Touched mtime reparses only that file. Deleted file's rows are dropped. Corrupt cache.bincode triggers rebuild. ~5 tests. |
| `calibration::anchors::last_weekly_reset` | Unit | Mon/Sat/Sun-06:00/Sun-08:00 local in Europe/Copenhagen; verify wrap-around. ~4 tests. |
| `calibration::anchors::five_hour_burn_at` | Unit | Single window, multi-window-with-gap, window-rollover-by-duration. ~3 tests. |
| `calibration::anchors::global_cap_from_anchors` | Unit | Zero anchors, single anchor, multi-anchor median. ~3 tests. |
| `calibration::hourly` | Unit | Pure array math — empty-bin / single-bin / all-bins boundary cases. ~4 tests. |
| `calibration::live::live_util_now` | Unit | Caps None → util None. Caps Some → util = burn/cap. ~2 tests. |
| Existing 15 Stage 1–4 tests | Regression | Must continue to pass. |

Target: ~27 new tests, ~42 total. No visual smoke tests for tooltip changes — manual verification only.

## Stage 5 deliverable / verification

End-to-end checks before tagging `v0.5.0`:

- `cargo fmt --check` clean.
- `cargo clippy --all-targets -- -D warnings` clean.
- `cargo test` — all tests pass.
- `cargo build --release` → `target\release\claude-usage-tray.exe`.
- Run the .exe → tooltip shows API util immediately, local lines appear within 1 poll interval (or `(uncalibrated)` if the calibration log has no ≥0.95 anchors yet).
- Inspect `~/.claude-usage-tray/cache.bincode` exists; `cache_manifest.json` lists every JSONL file under `~/.claude/projects/`.
- Truncate `cache.bincode` to 4 bytes → next poll logs a warning + rebuilds cleanly.
- Touch one JSONL file → next poll reparses only that file (verified via `tracing::debug!` line counts).
- Run for ≥1 hour → handle count stable, memory stable, no leaks.
- Tag `v0.5.0` and push.

## Carry-overs from Stage 4 (unchanged)

- Polling cadence (60/120/300 s; default 120 s).
- Threading model (UI thread + polling thread + mpsc + `PostMessageW`).
- Icon rendering pipeline (`IconRenderer`, `compute_visual`, HICON lifecycle).
- Right-click menu and shutdown sequence.
- API util feeding the icon glyph.

## Stage 5 enabling Stage 6

Stage 6 (dashboard window) will consume:
- `Vec<Turn>` from `data::cache::refresh` — for the 5h cumulative-share chart and the weekly chart and the daily bar chart.
- `DerivedCaps` from `calibration::anchors` — for the cap lines drawn on the charts.
- The 24-bin hour-of-day cap series from `calibration::hourly` — for the smooth cap-vs-time curve overlaid on the 5h chart.

By shipping the hour-of-day model in Stage 5 even though it isn't displayed yet, Stage 6 becomes a pure rendering exercise on top of an already-tested data layer.

## Open questions deferred to implementation

- **Exact tooltip layout.** Two extra lines (as shown) vs single-line "5h: 57% (local 54%)" vs a `─` separator. Tune in implementation; current Stage 3 tooltip already uses multi-line, so adding two more is the path of least resistance.
- **`bincode` 1.x vs 2.x.** 1.x is serde-only (simpler for a beginner); 2.x has its own derive macros and a faster format. Likely settle on 1.x in the plan unless 2.x's migration story is trivial.
- **Cache size bound.** Over a year of usage the cache could reach tens of MB. Probably fine; revisit if startup load gets noticeable in profiling.
- **`tests/fixtures/sample_session.jsonl` content.** Will be hand-anonymized from a real session during implementation. Goal: ≤200 lines, covers all parser branches (usage rows, rate-limit rows, corrupt lines, subagent path classification).
