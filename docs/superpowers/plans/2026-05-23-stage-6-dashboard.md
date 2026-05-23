# Stage 6 — Native Dashboard Window Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add an `eframe`/`egui` native dashboard window to the tray app. Left-click the tray icon → window opens with KPI strip + three charts (5h cumulative share, weekly cumulative share, daily cost-weighted bar). Close button destroys, re-click reopens; one window at a time.

**Architecture:** A new `shared/` module holds `AppSnapshot` (turns, caps, KPIs) behind `Arc<RwLock<...>>`. The polling thread writes a fresh snapshot every tick. A new `dashboard/` thread is spawned on tray-icon left-click, runs `eframe::run_native` with `EventLoopBuilderExtWindows::with_any_thread(true)`, and reads the snapshot every frame. Raise-to-front uses Win32 `EnumWindows` against a known window title. Quit posts `WM_CLOSE` to the dashboard HWND, then joins the thread.

**Tech Stack:** Rust stable, `eframe` + `egui` + `egui_plot` 0.29, existing `windows` crate (Stage 3), existing data + calibration modules (Stage 5).

**Reference spec:** `docs/superpowers/specs/2026-05-23-stage-6-dashboard-design.md`

---

## File Structure

### Created
- `src/shared/mod.rs` — module index + `SharedSnapshot` type alias
- `src/shared/snapshot.rs` — `AppSnapshot`, `DashboardKpis`, `cost_weighted`, `compute_kpis`
- `src/dashboard/mod.rs` — module index + `launch`, `find_hwnd_by_title`, `DashboardHandle`, `DASHBOARD_WINDOW_TITLE`
- `src/dashboard/app.rs` — `DashboardApp` impl `eframe::App`, top-level layout
- `src/dashboard/range.rs` — `Range` enum + `clamp_x_range`
- `src/dashboard/bands.rs` — `calendar_bands` weekend+night intervals
- `src/dashboard/series.rs` — `cumulative_share_series_5h`, `cumulative_share_series_weekly`, `daily_aggregates`
- `src/dashboard/kpi.rs` — KPI strip rendering
- `src/dashboard/chart_5h.rs` — 5h chart rendering
- `src/dashboard/chart_weekly.rs` — weekly chart rendering
- `src/dashboard/chart_daily.rs` — daily bar chart rendering
- `tests/snapshot_test.rs` — cost_weighted + compute_kpis integration tests
- `tests/range_test.rs` — range clamping tests
- `tests/bands_test.rs` — calendar bands tests
- `tests/series_test.rs` — share series + daily aggregate tests

### Modified
- `Cargo.toml` — add `eframe`, `egui`, `egui_plot`
- `src/lib.rs` — register `shared`, `dashboard` modules
- `src/config.rs` — add `COST_WEIGHT_*` constants
- `src/tray/poller.rs` — write to `SharedSnapshot` at end of tick; compute KPIs
- `src/tray/window.rs` — `TrayState` gains `shared` + `dashboard` fields; `WM_LBUTTONUP` handler; `IDM_QUIT` posts `WM_CLOSE` to dashboard
- `src/tray/mod.rs` — wire `shared` + `dashboard` into the run sequence; join dashboard at shutdown

---

## Beginner notes (read first)

You'll see a few patterns repeatedly:

- **`Arc<RwLock<T>>` for cross-thread state.** `RwLock` allows many concurrent readers OR one writer. The polling thread writes every 60s; the dashboard thread reads at ~60fps. Pattern: `*shared.write().unwrap() = new_value;` (write); `let snap = shared.read().unwrap().clone();` (read).
- **`Arc<Vec<Turn>>` inside the snapshot.** Cloning the outer `AppSnapshot` is cheap because the inner `Vec<Turn>` is itself an `Arc`. Bumping the `Arc` refcount instead of copying ~22 MB of data.
- **eframe immediate-mode UI.** Unlike retained-mode (React, Win32), egui doesn't keep widgets in a tree between frames. `App::update(&mut self, ctx, frame)` runs every frame; you describe what should be on screen this frame; egui diffs and renders. State persists in your `App` struct fields.
- **egui 0.29 API surface.** When the spec says "egui::CentralPanel" or "egui::ViewportBuilder", consult the egui 0.29 docs at https://docs.rs/egui/0.29/ for exact method names. The plan's code is correct in spirit; minor adaptations may be needed.
- **Win32 FFI through the `windows` crate.** Same `unsafe { ... }` + raw-pointer patterns from Stages 3–4. `windows::Win32::UI::WindowsAndMessaging::{EnumWindowsW, GetWindowTextW, SetForegroundWindow, ShowWindow, PostMessageW}` are the new APIs we use.
- **Win32 title strings are UTF-16.** When matching the dashboard's title, decode the GetWindowTextW buffer with `OsString::from_wide` or convert your target to UTF-16 with `.encode_utf16().collect::<Vec<u16>>()` and compare element-wise.

If you've never written egui code: `cargo run --example` against the `eframe_template` repo is a great 5-minute warmup before Task 13.

---

## Task 1: Add eframe, egui, egui_plot to Cargo.toml

**Files:**
- Modify: `Cargo.toml`

- [ ] **Step 1: Edit Cargo.toml**

Add these three lines to `[dependencies]`, placed after `clap` (alphabetically grouped with other UI/runtime deps is fine):

```toml
eframe = { version = "0.29", default-features = false, features = ["default_fonts", "glow"] }
egui = "0.29"
egui_plot = "0.29"
```

- [ ] **Step 2: Verify build**

Run: `cargo build`
Expected: builds cleanly. First build with the new deps downloads + compiles ~30 new crates (egui ecosystem); takes ~3-5 minutes the first time.

- [ ] **Step 3: Commit**

```bash
git add Cargo.toml Cargo.lock
git commit -m "chore(stage-6): add eframe + egui + egui_plot dependencies"
```

---

## Task 2: Add cost-weight constants to `src/config.rs`

**Files:**
- Modify: `src/config.rs`

- [ ] **Step 1: Append to `src/config.rs`**

```rust
/// Per-token-type weights for the "cost-weighted" KPI/chart aggregate. These
/// mirror the Python project's config.COST_WEIGHTS — they're heuristic, not
/// authoritative Anthropic pricing, and only used for the dashboard's spend
/// view (NOT for cap calibration, which uses output_tokens only — Stage 5).
pub const COST_WEIGHT_INPUT: f64 = 1.0;
pub const COST_WEIGHT_CACHE_CREATION: f64 = 1.25;
pub const COST_WEIGHT_CACHE_READ: f64 = 0.1;
pub const COST_WEIGHT_OUTPUT: f64 = 5.0;
```

- [ ] **Step 2: Verify build**

Run: `cargo build`
Expected: builds cleanly.

- [ ] **Step 3: Commit**

```bash
git add src/config.rs
git commit -m "feat(stage-6): add COST_WEIGHT_* constants to config"
```

---

## Task 3: Scaffold `src/shared/` module with `AppSnapshot` skeleton

**Files:**
- Create: `src/shared/mod.rs`
- Create: `src/shared/snapshot.rs`
- Modify: `src/lib.rs`

- [ ] **Step 1: Create `src/shared/mod.rs`**

```rust
//! State shared between the polling thread, the tray UI thread, and the
//! dashboard window thread. Wrapped in Arc<RwLock<...>> for safe concurrent
//! access.

pub mod snapshot;

use std::sync::{Arc, RwLock};

pub type SharedSnapshot = Arc<RwLock<snapshot::AppSnapshot>>;

pub fn new_shared_snapshot() -> SharedSnapshot {
    Arc::new(RwLock::new(snapshot::AppSnapshot::default()))
}
```

- [ ] **Step 2: Create `src/shared/snapshot.rs`** (types + stubs only — functions come in later tasks)

```rust
//! Cross-thread snapshot of the app's state.

use crate::api::usage::UsageSnapshot;
use crate::calibration::anchors::DerivedCaps;
use crate::calibration::live::LiveUtil;
use crate::data::parser::Turn;
use crate::render::LastStatus;
use chrono::{DateTime, Utc};
use std::sync::Arc;

/// What the polling thread writes; what the dashboard reads.
#[derive(Debug, Clone, Default)]
pub struct AppSnapshot {
    pub turns: Arc<Vec<Turn>>,
    pub caps: DerivedCaps,
    pub hourly_5h: [f64; 24],
    pub hourly_week: [f64; 24],
    pub live_util: LiveUtil,
    pub last_sample: Option<(UsageSnapshot, DateTime<Utc>)>,
    pub last_status: LastStatus,
    pub kpis: DashboardKpis,
}

/// Pre-computed KPIs so the dashboard doesn't recompute them every frame.
#[derive(Debug, Clone, Default)]
pub struct DashboardKpis {
    pub peak_5h_share: f64,
    pub peak_week_share: f64,
    pub total_cost_weighted: f64,
    pub daily_avg_cost_weighted: f64,
}
```

NOTE: `LastStatus` does not implement `Default` (check `src/render.rs`). Either add `#[derive(Default)]` to `LastStatus` (likely `LastStatus::Initial` should be the default), OR replace `Default` here with a manual impl. The simplest fix is to add `#[derive(Default)]` to `LastStatus` and mark `Initial` with `#[default]`:

Apply this fix in `src/render.rs`. Replace:
```rust
#[derive(Debug, Clone)]
pub enum LastStatus {
    /// Before the first poll completes.
    Initial,
    ...
}
```
with:
```rust
#[derive(Debug, Clone, Default)]
pub enum LastStatus {
    /// Before the first poll completes.
    #[default]
    Initial,
    ...
}
```

- [ ] **Step 3: Register the module in `src/lib.rs`**

Add `pub mod shared;` alphabetically:

```rust
pub mod api;
pub mod calibration;
pub mod cli;
pub mod config;
pub mod data;
pub mod log;
pub mod paths;
pub mod poll;
pub mod render;
pub mod shared;
pub mod tray;
pub mod watch;
```

- [ ] **Step 4: Verify build**

Run: `cargo build`
Expected: builds cleanly. UsageSnapshot needs Default — see UsageSnapshot in `src/api/usage.rs`. Add `#[derive(Default)]` there too if missing:
```rust
#[derive(Debug, Clone, Default)]
pub struct UsageSnapshot { ... }
```
Same may apply to `UsageBucket`. Add Default derives wherever the compiler complains about the AppSnapshot::default() call.

- [ ] **Step 5: Commit**

```bash
git add src/shared/ src/lib.rs src/render.rs src/api/usage.rs
git commit -m "feat(stage-6): scaffold shared/ module with AppSnapshot types"
```

---

## Task 4: Implement `cost_weighted` with unit test

**Files:**
- Modify: `src/shared/snapshot.rs`

- [ ] **Step 1: Add a failing test at the bottom of `src/shared/snapshot.rs`**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn turn(input: u64, cc: u64, cr: u64, output: u64) -> Turn {
        Turn {
            ts: chrono::Utc::now(),
            session_id: String::new(),
            subagent_id: None,
            is_subagent: false,
            project_cwd: String::new(),
            model: String::new(),
            version: String::new(),
            input_tokens: input,
            output_tokens: output,
            cache_creation_input_tokens: cc,
            cache_read_input_tokens: cr,
            source_file: PathBuf::new(),
            is_rate_limit_error: false,
        }
    }

    #[test]
    fn cost_weighted_applies_each_coefficient() {
        // Input=100, cache_create=200, cache_read=300, output=400.
        // Expected: 100*1 + 200*1.25 + 300*0.1 + 400*5 = 100 + 250 + 30 + 2000 = 2380.
        let t = turn(100, 200, 300, 400);
        assert!((cost_weighted(&t) - 2380.0).abs() < 0.001);
    }
}
```

- [ ] **Step 2: Verify the test fails**

Run: `cargo test --lib cost_weighted`
Expected: compilation error — "cannot find function `cost_weighted` in this scope".

- [ ] **Step 3: Implement `cost_weighted`**

Add to `src/shared/snapshot.rs` (above the `#[cfg(test)]` block):

```rust
use crate::config;

/// Heuristic cost-weighted token count for a single turn. Used by the
/// dashboard's "total burn" KPI + daily bar chart. NOT used for cap math.
pub fn cost_weighted(turn: &Turn) -> f64 {
    turn.input_tokens as f64 * config::COST_WEIGHT_INPUT
        + turn.cache_creation_input_tokens as f64 * config::COST_WEIGHT_CACHE_CREATION
        + turn.cache_read_input_tokens as f64 * config::COST_WEIGHT_CACHE_READ
        + turn.output_tokens as f64 * config::COST_WEIGHT_OUTPUT
}
```

- [ ] **Step 4: Verify the test passes**

Run: `cargo test --lib cost_weighted`
Expected: 1 passed.

- [ ] **Step 5: Commit**

```bash
git add src/shared/snapshot.rs
git commit -m "feat(stage-6): add cost_weighted helper with unit test"
```

---

## Task 5: Implement `compute_kpis` (peak shares)

**Files:**
- Modify: `src/shared/snapshot.rs`

- [ ] **Step 1: Add a failing test inside the existing `tests` mod**

Append:

```rust
    use chrono::TimeZone;
    fn utc(y: i32, m: u32, d: u32, h: u32, mi: u32) -> chrono::DateTime<chrono::Utc> {
        chrono::Utc.with_ymd_and_hms(y, m, d, h, mi, 0).unwrap()
    }
    fn turn_at(ts: chrono::DateTime<chrono::Utc>, output: u64) -> Turn {
        let mut t = turn(0, 0, 0, output);
        t.ts = ts;
        t
    }

    #[test]
    fn compute_kpis_peak_5h_share_max_across_windows() {
        // Two 5h windows. Window 1 (10:00-12:00): 100+200+300 = 600 output.
        // Window 2 (16:00-17:00): 100 output. 6h gap so they're separate.
        // cap_5h = 1000. Peak share = 600/1000 = 0.6.
        let turns = vec![
            turn_at(utc(2026, 5, 24, 10, 0), 100),
            turn_at(utc(2026, 5, 24, 11, 0), 200),
            turn_at(utc(2026, 5, 24, 12, 0), 300),
            turn_at(utc(2026, 5, 24, 18, 0), 100),  // 6h gap → new window
        ];
        let caps = DerivedCaps { cap_5h: Some(1000.0), cap_week: None, n_anchors_5h: 1, n_anchors_week: 0 };
        let k = compute_kpis(&turns, &caps);
        assert!((k.peak_5h_share - 0.6).abs() < 0.001);
    }

    #[test]
    fn compute_kpis_peak_share_zero_when_cap_none() {
        let turns = vec![turn_at(utc(2026, 5, 24, 10, 0), 100)];
        let caps = DerivedCaps::default();  // both caps None
        let k = compute_kpis(&turns, &caps);
        assert_eq!(k.peak_5h_share, 0.0);
        assert_eq!(k.peak_week_share, 0.0);
    }
```

- [ ] **Step 2: Verify the test fails**

Run: `cargo test --lib compute_kpis`
Expected: compilation error — "cannot find function `compute_kpis`".

- [ ] **Step 3: Implement peak-share KPIs**

Add to `src/shared/snapshot.rs` (above the test block):

```rust
use crate::calibration::anchors::{five_hour_burn_at, weekly_burn_at, last_weekly_reset};
use crate::config::FIVE_HOUR_WINDOW_HOURS;
use chrono::Duration;

/// Compute all four KPIs from the turns + caps. Called once per poll.
pub fn compute_kpis(turns: &[Turn], caps: &DerivedCaps) -> DashboardKpis {
    DashboardKpis {
        peak_5h_share: peak_5h_share(turns, caps),
        peak_week_share: peak_week_share(turns, caps),
        total_cost_weighted: 0.0,        // filled in by next task
        daily_avg_cost_weighted: 0.0,    // filled in by next task
    }
}

/// Max cumulative-share across any 5h window, or 0.0 if cap_5h is None.
fn peak_5h_share(turns: &[Turn], caps: &DerivedCaps) -> f64 {
    let Some(cap) = caps.cap_5h else { return 0.0 };
    if cap <= 0.0 { return 0.0; }
    // The peak in any 5h window is exactly five_hour_burn_at(ts_at_end_of_window).
    // We don't know window boundaries without re-deriving, so iterate every turn
    // as a potential window endpoint and take the max of burn_at(turn.ts) / cap.
    turns.iter()
        .map(|t| five_hour_burn_at(turns, t.ts) as f64 / cap)
        .fold(0.0_f64, f64::max)
}

/// Max cumulative-share across any weekly window.
fn peak_week_share(turns: &[Turn], caps: &DerivedCaps) -> f64 {
    let Some(cap) = caps.cap_week else { return 0.0 };
    if cap <= 0.0 { return 0.0; }
    // Same idea — every turn is a potential endpoint.
    turns.iter()
        .map(|t| weekly_burn_at(turns, t.ts) as f64 / cap)
        .fold(0.0_f64, f64::max)
}
```

NOTE: this is O(n²) over `turns` because each `five_hour_burn_at` is O(n). For 1M turns this would be slow. We accept it for v0.6.0 (poll cadence is 60s; worst case is a few seconds CPU per poll). Optimization is listed in the spec's Open questions and deferred.

- [ ] **Step 4: Verify the tests pass**

Run: `cargo test --lib compute_kpis`
Expected: 2 passed.

- [ ] **Step 5: Commit**

```bash
git add src/shared/snapshot.rs
git commit -m "feat(stage-6): implement peak_5h_share + peak_week_share KPIs"
```

---

## Task 6: Implement total + daily-avg cost-weighted KPIs

**Files:**
- Modify: `src/shared/snapshot.rs`

- [ ] **Step 1: Append a failing test inside the existing `tests` mod**

```rust
    #[test]
    fn compute_kpis_total_and_daily_avg_cost_weighted() {
        // 3 turns on 2026-05-24, 1 turn on 2026-05-25 → span 2 days.
        // Each turn: cost_weighted = 1*1 + 1*1.25 + 1*0.1 + 1*5 = 7.35.
        let turns = vec![
            turn_at(utc(2026, 5, 24, 10, 0), 1),  // also has 1 input/cache_create/cache_read
            turn_at(utc(2026, 5, 24, 11, 0), 1),
            turn_at(utc(2026, 5, 24, 12, 0), 1),
            turn_at(utc(2026, 5, 25, 10, 0), 1),
        ];
        // Patch in input/cache_create/cache_read=1 for each.
        let turns: Vec<Turn> = turns.into_iter().map(|mut t| {
            t.input_tokens = 1;
            t.cache_creation_input_tokens = 1;
            t.cache_read_input_tokens = 1;
            t
        }).collect();
        let caps = DerivedCaps::default();
        let k = compute_kpis(&turns, &caps);
        // total = 4 * 7.35 = 29.4
        assert!((k.total_cost_weighted - 29.4).abs() < 0.01);
        // span = 1 day (last_ts - first_ts is 24h). daily_avg = 29.4 / 1.0 = 29.4
        // (We use max(1.0, span_days) to avoid div-by-zero for sub-day data.)
        assert!((k.daily_avg_cost_weighted - 29.4).abs() < 0.01);
    }

    #[test]
    fn compute_kpis_empty_turns_returns_zeros() {
        let k = compute_kpis(&[], &DerivedCaps::default());
        assert_eq!(k.peak_5h_share, 0.0);
        assert_eq!(k.total_cost_weighted, 0.0);
        assert_eq!(k.daily_avg_cost_weighted, 0.0);
    }
```

- [ ] **Step 2: Verify the test fails**

Run: `cargo test --lib compute_kpis_total_and_daily_avg_cost_weighted`
Expected: assertion fails — total_cost_weighted is 0.0 (still the stub).

- [ ] **Step 3: Fill in the total + daily_avg in `compute_kpis`**

Replace `compute_kpis` in `src/shared/snapshot.rs` with:

```rust
pub fn compute_kpis(turns: &[Turn], caps: &DerivedCaps) -> DashboardKpis {
    let total_cw: f64 = turns.iter().map(cost_weighted).sum();
    let daily_avg = if turns.len() < 2 {
        total_cw  // sub-day data: report total as daily avg
    } else {
        let first = turns.first().unwrap().ts;
        let last = turns.last().unwrap().ts;
        let span_days = ((last - first).num_seconds() as f64 / 86_400.0).max(1.0);
        total_cw / span_days
    };
    DashboardKpis {
        peak_5h_share: peak_5h_share(turns, caps),
        peak_week_share: peak_week_share(turns, caps),
        total_cost_weighted: total_cw,
        daily_avg_cost_weighted: daily_avg,
    }
}
```

- [ ] **Step 4: Verify the tests pass**

Run: `cargo test --lib compute_kpis`
Expected: 4 passed.

- [ ] **Step 5: Commit**

```bash
git add src/shared/snapshot.rs
git commit -m "feat(stage-6): implement total + daily-avg cost-weighted KPIs"
```

---

## Task 7: Wire `SharedSnapshot` into the polling thread

**Files:**
- Modify: `src/tray/poller.rs`

- [ ] **Step 1: Read `src/tray/poller.rs`** to confirm Stage 5's `compute_calibration` signature.

- [ ] **Step 2: Replace the polling loop body**

In `src/tray/poller.rs`, the `polling_loop` and `compute_calibration` need to:
1. Accept a `SharedSnapshot` parameter (cloned `Arc`).
2. Write the snapshot after computing calibration, before sending the mpsc event.

Update the function signatures in `polling_loop` and `compute_calibration`:

```rust
use crate::shared::SharedSnapshot;
use crate::shared::snapshot::{AppSnapshot, compute_kpis};
use std::sync::Arc;

fn polling_loop(
    creds: Credentials,
    interval: Duration,
    shutdown: Arc<AtomicBool>,
    hwnd: SendHwnd,
    tx: Sender<PollEvent>,
    shared: SharedSnapshot,
) {
    tracing::info!(interval_secs = interval.as_secs(), "polling thread starting");

    // Track the last good sample so it carries over for the dashboard between polls.
    let mut last_sample: Option<(crate::api::usage::UsageSnapshot, chrono::DateTime<chrono::Utc>)> = None;
    let mut last_status = crate::render::LastStatus::Initial;

    while !shutdown.load(Ordering::Relaxed) {
        let fetch_at = Instant::now();

        // Stage 5: calibration. Now also returns turns_arc so we can put it on the shared snapshot.
        let (calib, turns_arc) = compute_calibration_with_turns();

        // API fetch.
        let event = match poll_once(&creds) {
            Ok(snap) => {
                last_sample = Some((snap.clone(), chrono::Utc::now()));
                last_status = crate::render::LastStatus::Ok;
                PollEvent::Ok { snap, calib: Box::new(calib.clone()) }
            }
            Err(FetchError::RateLimited) => {
                last_status = crate::render::LastStatus::RateLimited;
                PollEvent::RateLimited
            }
            Err(other) => {
                last_status = crate::render::LastStatus::Error(other.to_string());
                PollEvent::Error(other.to_string())
            }
        };

        // Write the shared snapshot for the dashboard.
        let kpis = compute_kpis(&turns_arc, &calib.caps);
        let snapshot = AppSnapshot {
            turns: turns_arc,
            caps: calib.caps,
            hourly_5h: calib.hourly_5h,
            hourly_week: calib.hourly_week,
            live_util: calib.live,
            last_sample: last_sample.clone(),
            last_status: last_status.clone(),
            kpis,
        };
        match shared.write() {
            Ok(mut g) => *g = snapshot,
            Err(e) => tracing::warn!(error = ?e, "SharedSnapshot lock poisoned, dashboard data stale"),
        }

        let _ = tx.send(event);
        unsafe { let _ = PostMessageW(hwnd.0, WM_APP_POLL, WPARAM(0), LPARAM(0)); }
        sleep_interruptible(&shutdown, fetch_at, interval);
    }
    tracing::info!("polling thread exiting");
}
```

- [ ] **Step 3: Refactor `compute_calibration` to also return turns_arc**

Replace the existing `compute_calibration` with:

```rust
fn compute_calibration_with_turns() -> (PollCalibration, Arc<Vec<crate::data::parser::Turn>>) {
    use crate::calibration::anchors::derive_caps;
    use crate::calibration::hourly::hour_of_day_cap_series;
    use crate::calibration::live::live_util_now;
    use crate::calibration::WindowKind;
    use crate::data::cache;
    use crate::log::calibration as log_calib;

    let turns = match cache::refresh() {
        Ok(t) => t,
        Err(e) => {
            tracing::warn!(error = %e, "cache::refresh failed; skipping calibration this tick");
            return (PollCalibration::default(), Arc::new(Vec::new()));
        }
    };
    let turns_arc = Arc::new(turns);
    let log = match log_calib::read_all_default() {
        Ok(l) => l,
        Err(e) => {
            tracing::warn!(error = %e, "calibration log read failed; skipping calibration this tick");
            return (PollCalibration::default(), turns_arc);
        }
    };

    let caps = derive_caps(&log, &turns_arc);
    let hourly_5h = hour_of_day_cap_series(&log, &turns_arc, WindowKind::FiveHour);
    let hourly_week = hour_of_day_cap_series(&log, &turns_arc, WindowKind::Weekly);
    let live = live_util_now(&turns_arc, &caps);

    tracing::debug!(
        n_anchors_5h = caps.n_anchors_5h,
        n_anchors_week = caps.n_anchors_week,
        cap_5h = ?caps.cap_5h,
        cap_week = ?caps.cap_week,
        n_turns = turns_arc.len(),
        "calibration computed"
    );

    (PollCalibration { caps, live, hourly_5h, hourly_week }, turns_arc)
}
```

NOTE the new return type: `(PollCalibration, Arc<Vec<Turn>>)`. The downstream callsite uses both.

- [ ] **Step 4: Update `spawn()` to accept and pass `shared`**

Find `pub fn spawn(...)` and update its signature:

```rust
pub fn spawn(
    creds: Credentials,
    interval_secs: u64,
    shutdown: Arc<AtomicBool>,
    hwnd: SendHwnd,
    tx: Sender<PollEvent>,
    shared: SharedSnapshot,
) -> JoinHandle<()> {
    let interval = Duration::from_secs(interval_secs);
    thread::spawn(move || polling_loop(creds, interval, shutdown, hwnd, tx, shared))
}
```

- [ ] **Step 5: Update `src/tray/mod.rs::run()` to pass `shared`**

In `src/tray/mod.rs`, add at the top of `run()`:

```rust
use crate::shared::new_shared_snapshot;
let shared = new_shared_snapshot();
```

And update the `poller::spawn(...)` call to pass `shared.clone()`:

```rust
let poll_handle = poller::spawn(creds, interval_secs, shutdown.clone(), send_hwnd, tx, shared.clone());
```

(Task 9 will use `shared` further; for now we just pass it.)

- [ ] **Step 6: Verify build**

Run: `cargo build && cargo test`
Expected: builds + all existing 64 tests pass.

- [ ] **Step 7: Commit**

```bash
git add src/tray/poller.rs src/tray/mod.rs
git commit -m "feat(stage-6): poller writes AppSnapshot to SharedSnapshot each tick"
```

---

## Task 8: Add `WM_LBUTTONUP` constant and stub left-click handler

**Files:**
- Modify: `src/tray/window.rs`

- [ ] **Step 1: Add `WM_LBUTTONUP` to the existing imports in `src/tray/window.rs`**

Find the `use windows::Win32::UI::WindowsAndMessaging::{...};` block and add `WM_LBUTTONUP` to the list:

```rust
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu, DestroyWindow,
    DispatchMessageW, GetCursorPos, GetMessageW, GetWindowLongPtrW, PostQuitMessage,
    RegisterClassExW, SetForegroundWindow, SetWindowLongPtrW, TrackPopupMenu, TranslateMessage,
    CREATESTRUCTW, CW_USEDEFAULT, GWLP_USERDATA, HICON, HMENU, HWND_MESSAGE, MF_STRING, MSG,
    TPM_LEFTBUTTON, TPM_RIGHTBUTTON, WINDOW_EX_STYLE, WINDOW_STYLE, WM_APP, WM_COMMAND, WM_DESTROY,
    WM_LBUTTONUP,
    WM_NCCREATE, WM_NCDESTROY, WM_RBUTTONUP, WNDCLASSEXW,
};
```

- [ ] **Step 2: Add the LMB handler in `wndproc`**

Find the `WM_APP_TRAYICON` match arm — currently:

```rust
        WM_APP_TRAYICON => {
            // lparam.0 carries the underlying mouse event id.
            if lparam.0 as u32 == WM_RBUTTONUP {
                show_context_menu(hwnd);
            }
            LRESULT(0)
        }
```

Update to:

```rust
        WM_APP_TRAYICON => {
            // lparam.0 carries the underlying mouse event id.
            if lparam.0 as u32 == WM_RBUTTONUP {
                show_context_menu(hwnd);
            } else if lparam.0 as u32 == WM_LBUTTONUP {
                with_state(hwnd, on_left_click);
            }
            LRESULT(0)
        }
```

- [ ] **Step 3: Add `on_left_click` stub**

Append to `src/tray/window.rs` (place near the other helper functions like `show_context_menu`):

```rust
/// Handler for left-click on the tray icon. Filled in by Task 12.
fn on_left_click(_state: &mut TrayState) {
    tracing::info!("LMB on tray icon (no-op until Task 12)");
}
```

- [ ] **Step 4: Verify build**

Run: `cargo build && cargo test`
Expected: builds + all tests pass.

- [ ] **Step 5: Commit**

```bash
git add src/tray/window.rs
git commit -m "feat(stage-6): hook WM_LBUTTONUP → on_left_click stub"
```

---

## Task 9: Scaffold `src/dashboard/` with module + constants

**Files:**
- Create: `src/dashboard/mod.rs`
- Modify: `src/lib.rs`

- [ ] **Step 1: Create `src/dashboard/mod.rs`** (skeleton only — submodules + launch in later tasks)

```rust
//! Native egui dashboard window. Lives on its own thread spawned from the
//! tray UI thread; reads from SharedSnapshot.

use windows::Win32::Foundation::HWND;

pub const DASHBOARD_WINDOW_TITLE: &str = "Claude usage tracker";

pub struct DashboardHandle {
    pub hwnd: std::sync::Arc<std::sync::Mutex<Option<HWND>>>,
    pub join: std::thread::JoinHandle<()>,
}

// SAFETY: HWND is a *mut c_void newtype which Rust won't auto-Send.
// We never dereference the pointer; it's only used as an argument to Win32
// APIs that are themselves thread-safe.
unsafe impl Send for SendHwndCell {}
unsafe impl Sync for SendHwndCell {}
pub(crate) struct SendHwndCell;
```

Wait — that `SendHwndCell` approach is ugly. Simpler fix: wrap HWND in a transparent newtype just like `SendHwnd` does in `poller.rs`. Replace the above with:

```rust
//! Native egui dashboard window.

use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use windows::Win32::Foundation::HWND;

pub const DASHBOARD_WINDOW_TITLE: &str = "Claude usage tracker";

/// Thread-safe wrapper for HWND. The pointer is opaque from Rust's perspective;
/// Win32 functions that take an HWND are themselves thread-safe.
#[derive(Clone, Copy)]
pub struct SendHwnd(pub HWND);

// SAFETY: see doc comment.
unsafe impl Send for SendHwnd {}
unsafe impl Sync for SendHwnd {}

pub struct DashboardHandle {
    pub hwnd: Arc<Mutex<Option<SendHwnd>>>,
    pub join: JoinHandle<()>,
}
```

(Task 10/11/12 will add `launch`, `find_hwnd_by_title`, etc.)

- [ ] **Step 2: Register module in `src/lib.rs`**

Add `pub mod dashboard;` alphabetically:

```rust
pub mod api;
pub mod calibration;
pub mod cli;
pub mod config;
pub mod dashboard;
pub mod data;
pub mod log;
pub mod paths;
pub mod poll;
pub mod render;
pub mod shared;
pub mod tray;
pub mod watch;
```

- [ ] **Step 3: Verify build**

Run: `cargo build && cargo test`
Expected: builds + tests pass.

- [ ] **Step 4: Commit**

```bash
git add src/dashboard/ src/lib.rs
git commit -m "feat(stage-6): scaffold dashboard/ module with SendHwnd + DashboardHandle"
```

---

## Task 10: Implement `find_hwnd_by_title` Win32 helper

**Files:**
- Modify: `src/dashboard/mod.rs`

- [ ] **Step 1: Add the function (no unit test — Win32 FFI is verified by manual smoke test in Task 25)**

Append to `src/dashboard/mod.rs`:

```rust
use windows::core::BOOL;
use windows::Win32::Foundation::LPARAM;
use windows::Win32::UI::WindowsAndMessaging::{EnumWindows, GetWindowTextW, IsWindowVisible};

/// Search top-level windows for one whose title equals `target` (case-sensitive).
///
/// Returns the first match. Used to find the dashboard's HWND after eframe
/// creates it — we don't have a clean handle to it from the egui-side API.
///
/// Title is unique to this process: see DASHBOARD_WINDOW_TITLE.
pub fn find_hwnd_by_title(target: &str) -> Option<HWND> {
    let mut state = EnumState {
        target_utf16: target.encode_utf16().collect::<Vec<u16>>(),
        result: None,
    };

    // SAFETY: state lives for the duration of EnumWindows; the callback is
    // invoked synchronously from this thread; we cast a &mut to LPARAM and back.
    unsafe {
        let _ = EnumWindows(
            Some(enum_proc),
            LPARAM(&mut state as *mut _ as isize),
        );
    }
    state.result
}

struct EnumState {
    target_utf16: Vec<u16>,
    result: Option<HWND>,
}

extern "system" fn enum_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
    // SAFETY: lparam was constructed from a &mut EnumState in find_hwnd_by_title.
    let state = unsafe { &mut *(lparam.0 as *mut EnumState) };

    if unsafe { !IsWindowVisible(hwnd).as_bool() } {
        return BOOL(1); // continue
    }

    let mut buf = [0u16; 256];
    // SAFETY: buf has room for 256 UTF-16 units; GetWindowTextW handles its own length.
    let len = unsafe { GetWindowTextW(hwnd, &mut buf) } as usize;
    if len == 0 {
        return BOOL(1);
    }

    if buf[..len] == state.target_utf16[..] {
        state.result = Some(hwnd);
        return BOOL(0); // stop
    }
    BOOL(1) // continue
}
```

- [ ] **Step 2: Verify build**

Run: `cargo build`
Expected: builds cleanly. May need to add `Win32_Foundation` for `BOOL` (already in our features list).

- [ ] **Step 3: Commit**

```bash
git add src/dashboard/mod.rs
git commit -m "feat(stage-6): add find_hwnd_by_title using Win32 EnumWindows"
```

---

## Task 11: Scaffold `DashboardApp` (egui::App impl) — empty window

**Files:**
- Create: `src/dashboard/app.rs`
- Modify: `src/dashboard/mod.rs`

- [ ] **Step 1: Create `src/dashboard/app.rs` with a minimal App impl**

```rust
//! DashboardApp is the eframe::App implementation. The first frame discovers
//! its own HWND via find_hwnd_by_title and writes it into the shared slot
//! so the tray UI thread can raise the window to front on subsequent clicks.

use crate::dashboard::{find_hwnd_by_title, SendHwnd, DASHBOARD_WINDOW_TITLE};
use crate::shared::SharedSnapshot;
use std::sync::{Arc, Mutex};

pub struct DashboardApp {
    shared: SharedSnapshot,
    hwnd_slot: Arc<Mutex<Option<SendHwnd>>>,
    hwnd_found: bool,
}

impl DashboardApp {
    pub fn new(shared: SharedSnapshot, hwnd_slot: Arc<Mutex<Option<SendHwnd>>>) -> Self {
        Self { shared, hwnd_slot, hwnd_found: false }
    }

    /// Try to find our own HWND. Called every frame until found.
    fn discover_hwnd_if_needed(&mut self) {
        if self.hwnd_found {
            return;
        }
        if let Some(hwnd) = find_hwnd_by_title(DASHBOARD_WINDOW_TITLE) {
            *self.hwnd_slot.lock().unwrap() = Some(SendHwnd(hwnd));
            self.hwnd_found = true;
            tracing::debug!("dashboard HWND discovered");
        }
    }
}

impl eframe::App for DashboardApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.discover_hwnd_if_needed();

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("Claude usage tracker");
            ui.label(format!("Snapshot turns: {}", self.shared.read().unwrap().turns.len()));
            ui.label("(dashboard content coming in later tasks)");
        });

        // Request a repaint at ~30fps so the snapshot view stays fresh.
        ctx.request_repaint_after(std::time::Duration::from_millis(33));
    }
}
```

- [ ] **Step 2: Add the `app` submodule declaration in `src/dashboard/mod.rs`**

At the top of `src/dashboard/mod.rs`, after the doc comment, add:

```rust
pub mod app;
```

- [ ] **Step 3: Verify build**

Run: `cargo build`
Expected: builds cleanly.

- [ ] **Step 4: Commit**

```bash
git add src/dashboard/
git commit -m "feat(stage-6): scaffold DashboardApp with HWND self-discovery"
```

---

## Task 12: Implement `dashboard::launch` and wire LMB handler

**Files:**
- Modify: `src/dashboard/mod.rs`
- Modify: `src/tray/window.rs`

- [ ] **Step 1: Add `launch` to `src/dashboard/mod.rs`**

Append:

```rust
use crate::dashboard::app::DashboardApp;
use crate::shared::SharedSnapshot;

/// Spawn the dashboard window on a fresh thread. Returns immediately;
/// the HWND inside the returned handle is populated asynchronously by the
/// dashboard thread once eframe builds the window.
pub fn launch(shared: SharedSnapshot) -> DashboardHandle {
    let hwnd_slot: Arc<Mutex<Option<SendHwnd>>> = Arc::new(Mutex::new(None));
    let hwnd_slot_for_thread = hwnd_slot.clone();

    let join = std::thread::spawn(move || {
        let app = DashboardApp::new(shared, hwnd_slot_for_thread);
        let native_options = eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default()
                .with_inner_size([1100.0, 720.0])
                .with_min_inner_size([700.0, 480.0])
                .with_title(DASHBOARD_WINDOW_TITLE),
            // Opt into non-main-thread EventLoop creation. Without this,
            // winit panics on Windows when constructed off the main thread.
            event_loop_builder: Some(Box::new(|builder| {
                use eframe::egui_winit::winit::platform::windows::EventLoopBuilderExtWindows;
                builder.with_any_thread(true);
            })),
            ..Default::default()
        };

        if let Err(e) = eframe::run_native(
            "claude-usage-tray-dashboard",
            native_options,
            Box::new(|_cc| Ok(Box::new(app))),
        ) {
            tracing::warn!(error = ?e, "eframe::run_native failed");
        }
        // run_native returned → window closed → thread is about to exit.
    });

    DashboardHandle { hwnd: hwnd_slot, join }
}
```

NOTE: the exact path to `EventLoopBuilderExtWindows` may differ in your eframe 0.29 build. Try in order:
1. `eframe::egui_winit::winit::platform::windows::EventLoopBuilderExtWindows`
2. `winit::platform::windows::EventLoopBuilderExtWindows` (after `cargo add winit`)
3. Consult `cargo doc --open` from the eframe crate.

- [ ] **Step 2: Add `dashboard` field to `TrayState` in `src/tray/window.rs`**

Find the `TrayState` struct and update it. Add the imports first:

```rust
use crate::dashboard::DashboardHandle;
use crate::shared::SharedSnapshot;
```

Update the struct:

```rust
pub struct TrayState {
    pub last_sample: Option<(UsageSnapshot, DateTime<Utc>)>,
    pub last_status: LastStatus,
    pub renderer: IconRenderer,
    pub current_hicon: Option<HICON>,
    pub rx: Receiver<PollEvent>,
    pub shutdown: Arc<AtomicBool>,
    pub last_caps: Option<DerivedCaps>,
    pub last_local_util: Option<LiveUtil>,
    pub last_hourly_5h: Option<[f64; 24]>,
    pub last_hourly_week: Option<[f64; 24]>,
    pub shared: SharedSnapshot,
    pub dashboard: Arc<Mutex<Option<DashboardHandle>>>,
}
```

Add `use std::sync::Mutex;` if not already imported.

- [ ] **Step 3: Replace the stub `on_left_click` in `src/tray/window.rs`**

```rust
fn on_left_click(state: &mut TrayState) {
    let mut guard = state.dashboard.lock().unwrap();
    match guard.as_ref() {
        Some(handle) if !handle.join.is_finished() => {
            // Dashboard alive — try to raise. If HWND not yet known, no-op.
            if let Some(hwnd) = *handle.hwnd.lock().unwrap() {
                use windows::Win32::UI::WindowsAndMessaging::{
                    SetForegroundWindow, ShowWindow, SW_RESTORE,
                };
                unsafe {
                    let _ = SetForegroundWindow(hwnd.0);
                    let _ = ShowWindow(hwnd.0, SW_RESTORE);
                }
            } else {
                tracing::debug!("LMB while dashboard HWND not yet populated");
            }
        }
        _ => {
            // No window, or thread has finished. Spawn fresh.
            tracing::info!("spawning dashboard window");
            *guard = Some(crate::dashboard::launch(state.shared.clone()));
        }
    }
}
```

- [ ] **Step 4: Update `src/tray/mod.rs::run()` to initialize and pass `dashboard`**

In `src/tray/mod.rs`, after the existing `shared` variable, add:

```rust
let dashboard: Arc<std::sync::Mutex<Option<crate::dashboard::DashboardHandle>>> =
    Arc::new(std::sync::Mutex::new(None));
```

Then in the `Box::new(window::TrayState { ... })` construction, add:

```rust
shared: shared.clone(),
dashboard: dashboard.clone(),
```

- [ ] **Step 5: Verify build**

Run: `cargo build`
Expected: builds cleanly.

- [ ] **Step 6: Manual smoke test**

Run: `cargo build --release` then launch `target\release\claude-usage-tray.exe`. Left-click the tray icon — a new window titled "Claude usage tracker" should appear within ~500 ms showing the placeholder text. Close it via the X. Left-click the tray icon again — a fresh window should appear. Left-click while the window is open — the existing window should come to the front (test by minimizing it first).

If everything works, proceed to Step 7. If not, the most likely cause is the `EventLoopBuilderExtWindows` import path — see Task 12 Step 1's note.

- [ ] **Step 7: Commit**

```bash
git add src/dashboard/mod.rs src/tray/window.rs src/tray/mod.rs
git commit -m "feat(stage-6): launch dashboard on LMB; raise existing via SetForegroundWindow"
```

---

## Task 13: Shutdown coordination — WM_CLOSE to dashboard on Quit

**Files:**
- Modify: `src/tray/window.rs`
- Modify: `src/tray/mod.rs`

- [ ] **Step 1: Update the `IDM_QUIT` arm in `wndproc`**

Add `WM_CLOSE` + `PostMessageW` to the existing imports:

```rust
use windows::Win32::UI::WindowsAndMessaging::{
    // ...existing items...
    PostMessageW, WM_CLOSE,
};
```

Replace the existing `WM_COMMAND` arm:

```rust
        WM_COMMAND => {
            if (wparam.0 & 0xFFFF) == IDM_QUIT {
                with_state(hwnd, |state| {
                    state.shutdown.store(true, Ordering::Relaxed);

                    // Tell the dashboard to close, if it's open. The dashboard
                    // thread's eframe::run_native returns naturally when its
                    // window is closed, allowing the JoinHandle to complete.
                    if let Some(handle) = state.dashboard.lock().unwrap().as_ref() {
                        if let Some(hwnd_d) = *handle.hwnd.lock().unwrap() {
                            unsafe {
                                let _ = PostMessageW(hwnd_d.0, WM_CLOSE, WPARAM(0), LPARAM(0));
                            }
                        }
                    }
                });
                icon::delete(hwnd);
                unsafe { let _ = DestroyWindow(hwnd); }
            }
            LRESULT(0)
        }
```

- [ ] **Step 2: Join the dashboard thread in `src/tray/mod.rs::run()`**

After the existing `if let Err(e) = poll_handle.join() { ... }` block, append:

```rust
    // Take + join the dashboard handle if one was ever created.
    let dash = dashboard.lock().unwrap().take();
    if let Some(handle) = dash {
        if let Err(e) = handle.join.join() {
            tracing::warn!(error = ?e, "dashboard thread panicked");
        }
    }
```

(Recall the `dashboard` Arc was cloned into both `TrayState` and the local variable in Task 12 Step 4. The local `dashboard` here is the same Arc; we lock + take to extract the handle.)

- [ ] **Step 3: Verify build**

Run: `cargo build && cargo test`
Expected: builds + 64 tests pass.

- [ ] **Step 4: Manual smoke test**

Build + run the release exe. Left-click → dashboard opens. Right-click tray → Quit. The dashboard window should close, then the tray icon disappears within ~1 second. No orphaned process. Check Task Manager.

- [ ] **Step 5: Commit**

```bash
git add src/tray/window.rs src/tray/mod.rs
git commit -m "feat(stage-6): post WM_CLOSE to dashboard on Quit; join thread at shutdown"
```

---

## Task 14: Implement `Range` enum + `clamp_x_range`

**Files:**
- Create: `src/dashboard/range.rs`
- Create: `tests/range_test.rs`
- Modify: `src/dashboard/mod.rs`

- [ ] **Step 1: Create `tests/range_test.rs` with failing tests**

```rust
use chrono::{TimeZone, Utc};
use claude_usage_tray::dashboard::range::{clamp_x_range, Range};

#[test]
fn d1_clamps_to_24h_back() {
    let now = Utc.with_ymd_and_hms(2026, 5, 24, 12, 0, 0).unwrap();
    let (start, end) = clamp_x_range(now, Range::D1);
    assert_eq!(end, now);
    assert_eq!(start, Utc.with_ymd_and_hms(2026, 5, 23, 12, 0, 0).unwrap());
}

#[test]
fn d5_clamps_to_5_days_back() {
    let now = Utc.with_ymd_and_hms(2026, 5, 24, 12, 0, 0).unwrap();
    let (start, _) = clamp_x_range(now, Range::D5);
    assert_eq!(start, Utc.with_ymd_and_hms(2026, 5, 19, 12, 0, 0).unwrap());
}

#[test]
fn d14_clamps_to_14_days_back() {
    let now = Utc.with_ymd_and_hms(2026, 5, 24, 12, 0, 0).unwrap();
    let (start, _) = clamp_x_range(now, Range::D14);
    assert_eq!(start, Utc.with_ymd_and_hms(2026, 5, 10, 12, 0, 0).unwrap());
}

#[test]
fn m1_clamps_to_30_days_back() {
    let now = Utc.with_ymd_and_hms(2026, 5, 24, 12, 0, 0).unwrap();
    let (start, _) = clamp_x_range(now, Range::M1);
    assert_eq!(start, Utc.with_ymd_and_hms(2026, 4, 24, 12, 0, 0).unwrap());
}

#[test]
fn all_returns_now_for_start() {
    // For All, the caller is expected to use turns.first().ts; clamp_x_range
    // returns (now, now) and the chart code substitutes the actual data start.
    let now = Utc.with_ymd_and_hms(2026, 5, 24, 12, 0, 0).unwrap();
    let (start, end) = clamp_x_range(now, Range::All);
    assert_eq!(start, now);
    assert_eq!(end, now);
}

#[test]
fn range_label_round_trip() {
    assert_eq!(Range::D1.label(), "1D");
    assert_eq!(Range::D5.label(), "5D");
    assert_eq!(Range::D14.label(), "14D");
    assert_eq!(Range::M1.label(), "1M");
    assert_eq!(Range::All.label(), "All");
}
```

- [ ] **Step 2: Verify the tests fail**

Run: `cargo test --test range_test`
Expected: compilation error — "could not find `range` in module".

- [ ] **Step 3: Create `src/dashboard/range.rs`**

```rust
//! Range selector buttons (1D / 5D / 14D / 1M / All) above each chart.

use chrono::{DateTime, Duration, Utc};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Range {
    D1,
    D5,
    D14,
    M1,
    All,
}

impl Range {
    pub fn label(&self) -> &'static str {
        match self {
            Range::D1 => "1D",
            Range::D5 => "5D",
            Range::D14 => "14D",
            Range::M1 => "1M",
            Range::All => "All",
        }
    }

    pub fn duration(&self) -> Option<Duration> {
        match self {
            Range::D1 => Some(Duration::days(1)),
            Range::D5 => Some(Duration::days(5)),
            Range::D14 => Some(Duration::days(14)),
            Range::M1 => Some(Duration::days(30)),
            Range::All => None,
        }
    }

    pub const VARIANTS: &'static [Range] =
        &[Range::D1, Range::D5, Range::D14, Range::M1, Range::All];
}

/// Returns (start, end). For `Range::All`, returns (now, now); caller substitutes
/// turns.first().ts for the actual data-start time.
pub fn clamp_x_range(now: DateTime<Utc>, range: Range) -> (DateTime<Utc>, DateTime<Utc>) {
    let end = now;
    let start = match range.duration() {
        Some(d) => end - d,
        None => end,
    };
    (start, end)
}
```

- [ ] **Step 4: Register `range` submodule in `src/dashboard/mod.rs`**

At the top of `src/dashboard/mod.rs`, after `pub mod app;`, add:

```rust
pub mod range;
```

- [ ] **Step 5: Verify the tests pass**

Run: `cargo test --test range_test`
Expected: 6 passed.

- [ ] **Step 6: Commit**

```bash
git add src/dashboard/range.rs src/dashboard/mod.rs tests/range_test.rs
git commit -m "feat(stage-6): add Range enum + clamp_x_range with tests"
```

---

## Task 15: Implement `calendar_bands`

**Files:**
- Create: `src/dashboard/bands.rs`
- Create: `tests/bands_test.rs`
- Modify: `src/dashboard/mod.rs`

- [ ] **Step 1: Create failing tests at `tests/bands_test.rs`**

```rust
use chrono::{TimeZone, Utc};
use claude_usage_tray::dashboard::bands::{calendar_bands, BandKind};

#[test]
fn weekend_band_starts_saturday_0000_local_ends_monday_0000() {
    // Sat 2026-05-23 00:00 CEST (UTC+2 in May) = Fri 2026-05-22 22:00 UTC.
    // Mon 2026-05-25 00:00 CEST = Sun 2026-05-24 22:00 UTC.
    let range_start = Utc.with_ymd_and_hms(2026, 5, 20, 0, 0, 0).unwrap();
    let range_end = Utc.with_ymd_and_hms(2026, 5, 27, 0, 0, 0).unwrap();
    let bands = calendar_bands(range_start, range_end);
    let weekends: Vec<_> = bands.iter().filter(|(_, _, k)| *k == BandKind::Weekend).collect();
    assert_eq!(weekends.len(), 1);
    let (s, e, _) = weekends[0];
    assert_eq!(s, Utc.with_ymd_and_hms(2026, 5, 22, 22, 0, 0).unwrap());
    assert_eq!(e, Utc.with_ymd_and_hms(2026, 5, 24, 22, 0, 0).unwrap());
}

#[test]
fn night_bands_one_per_calendar_day_in_range() {
    // Range covers exactly 3 calendar days (Mon 5/18, Tue 5/19, Wed 5/20).
    // Should produce 3 night bands: Mon→Tue, Tue→Wed, Wed→Thu.
    let range_start = Utc.with_ymd_and_hms(2026, 5, 18, 0, 0, 0).unwrap();
    let range_end = Utc.with_ymd_and_hms(2026, 5, 21, 0, 0, 0).unwrap();
    let bands = calendar_bands(range_start, range_end);
    let nights: Vec<_> = bands.iter().filter(|(_, _, k)| *k == BandKind::Night).collect();
    assert_eq!(nights.len(), 3);
}

#[test]
fn empty_range_returns_empty_vec() {
    let t = Utc.with_ymd_and_hms(2026, 5, 24, 12, 0, 0).unwrap();
    let bands = calendar_bands(t, t);
    assert!(bands.is_empty());
}
```

- [ ] **Step 2: Verify the tests fail**

Run: `cargo test --test bands_test`
Expected: compilation error.

- [ ] **Step 3: Create `src/dashboard/bands.rs`**

```rust
//! Calendar shading bands — weekends (Sat+Sun in local TZ) and nights (22:00-06:00 local).

use crate::config;
use chrono::{DateTime, Datelike, Duration, TimeZone, Timelike, Utc, Weekday};
use chrono_tz::Tz;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BandKind {
    Weekend,
    Night,
}

/// Yields (start_utc, end_utc, kind) for every weekend + night band that
/// intersects `[range_start, range_end]`.
pub fn calendar_bands(
    range_start: DateTime<Utc>,
    range_end: DateTime<Utc>,
) -> Vec<(DateTime<Utc>, DateTime<Utc>, BandKind)> {
    if range_end <= range_start {
        return Vec::new();
    }
    let tz: Tz = config::LOCAL_TZ.parse().expect("LOCAL_TZ must be valid IANA name");
    let mut out = Vec::new();

    // Weekend: every Saturday 00:00 local → Monday 00:00 local.
    let mut cur = range_start.with_timezone(&tz);
    // Back up to most recent Saturday 00:00.
    let days_back = (cur.weekday().num_days_from_monday() + 7 - Weekday::Sat.num_days_from_monday()) % 7;
    let mut weekend_start_local = cur.date_naive() - Duration::days(days_back as i64);
    loop {
        let local_start_naive = weekend_start_local.and_hms_opt(0, 0, 0).unwrap();
        let local_end_naive = (weekend_start_local + Duration::days(2)).and_hms_opt(0, 0, 0).unwrap();
        let local_start = tz.from_local_datetime(&local_start_naive).single().unwrap();
        let local_end = tz.from_local_datetime(&local_end_naive).single().unwrap();
        let utc_start = local_start.with_timezone(&Utc);
        let utc_end = local_end.with_timezone(&Utc);

        if utc_start >= range_end { break; }
        if utc_end > range_start {
            out.push((utc_start.max(range_start), utc_end.min(range_end), BandKind::Weekend));
        }
        weekend_start_local += Duration::days(7);
    }

    // Night: every day 22:00 local → next-day 06:00 local.
    let mut night_local_date = range_start.with_timezone(&tz).date_naive();
    loop {
        let local_start_naive = night_local_date.and_hms_opt(22, 0, 0).unwrap();
        let local_end_naive = (night_local_date + Duration::days(1)).and_hms_opt(6, 0, 0).unwrap();
        let local_start = tz.from_local_datetime(&local_start_naive).single().unwrap();
        let local_end = tz.from_local_datetime(&local_end_naive).single().unwrap();
        let utc_start = local_start.with_timezone(&Utc);
        let utc_end = local_end.with_timezone(&Utc);

        if utc_start >= range_end { break; }
        if utc_end > range_start {
            out.push((utc_start.max(range_start), utc_end.min(range_end), BandKind::Night));
        }
        night_local_date += Duration::days(1);
    }

    out
}

// Silence unused import on platforms without Timelike methods used.
#[allow(unused_imports)]
use chrono::Timelike as _;
```

- [ ] **Step 4: Register the submodule**

In `src/dashboard/mod.rs`, add `pub mod bands;` near the other submodules:

```rust
pub mod app;
pub mod bands;
pub mod range;
```

- [ ] **Step 5: Verify the tests pass**

Run: `cargo test --test bands_test`
Expected: 3 passed.

- [ ] **Step 6: Commit**

```bash
git add src/dashboard/bands.rs src/dashboard/mod.rs tests/bands_test.rs
git commit -m "feat(stage-6): add calendar_bands for weekend + night shading"
```

---

## Task 16: Implement `cumulative_share_series_5h`

**Files:**
- Create: `src/dashboard/series.rs`
- Create: `tests/series_test.rs`
- Modify: `src/dashboard/mod.rs`

- [ ] **Step 1: Create failing tests at `tests/series_test.rs`**

```rust
use chrono::{TimeZone, Utc};
use claude_usage_tray::data::parser::Turn;
use claude_usage_tray::dashboard::series::{cumulative_share_series_5h, WindowedTurn};
use std::path::PathBuf;

fn utc(y: i32, m: u32, d: u32, h: u32, mi: u32) -> chrono::DateTime<chrono::Utc> {
    Utc.with_ymd_and_hms(y, m, d, h, mi, 0).unwrap()
}

fn turn(ts: chrono::DateTime<chrono::Utc>, output: u64) -> Turn {
    Turn {
        ts,
        session_id: String::new(),
        subagent_id: None,
        is_subagent: false,
        project_cwd: String::new(),
        model: String::new(),
        version: String::new(),
        input_tokens: 0,
        output_tokens: output,
        cache_creation_input_tokens: 0,
        cache_read_input_tokens: 0,
        source_file: PathBuf::new(),
        is_rate_limit_error: false,
    }
}

#[test]
fn cumulative_share_5h_single_window_stepped_growth() {
    let turns = vec![
        turn(utc(2026, 5, 24, 10, 0), 100),
        turn(utc(2026, 5, 24, 11, 0), 200),
        turn(utc(2026, 5, 24, 12, 0), 300),
    ];
    // cap = 1000 → shares: 0.1, 0.3, 0.6
    let series = cumulative_share_series_5h(&turns, Some(1000.0));
    assert_eq!(series.len(), 3);
    assert!((series[0].cumulative_share - 0.1).abs() < 0.001);
    assert!((series[1].cumulative_share - 0.3).abs() < 0.001);
    assert!((series[2].cumulative_share - 0.6).abs() < 0.001);
    assert_eq!(series[0].window_idx, 0);
    assert_eq!(series[1].window_idx, 0);
    assert_eq!(series[2].window_idx, 0);
}

#[test]
fn cumulative_share_5h_new_window_after_gap_resets_share() {
    let turns = vec![
        turn(utc(2026, 5, 24, 8, 0), 500),    // window 0
        turn(utc(2026, 5, 24, 14, 0), 200),   // 6h gap → window 1
        turn(utc(2026, 5, 24, 15, 0), 300),   // still window 1
    ];
    let series = cumulative_share_series_5h(&turns, Some(1000.0));
    assert_eq!(series.len(), 3);
    assert_eq!(series[0].window_idx, 0);
    assert_eq!(series[1].window_idx, 1);
    assert_eq!(series[2].window_idx, 1);
    assert!((series[0].cumulative_share - 0.5).abs() < 0.001);
    // Window 1 cumulative resets, so turn at 14:00 = 200/1000 = 0.2.
    assert!((series[1].cumulative_share - 0.2).abs() < 0.001);
    // Turn at 15:00 cumulates within window 1: (200+300)/1000 = 0.5
    assert!((series[2].cumulative_share - 0.5).abs() < 0.001);
}

#[test]
fn cumulative_share_5h_no_cap_uses_raw_output() {
    let turns = vec![turn(utc(2026, 5, 24, 10, 0), 100)];
    let series = cumulative_share_series_5h(&turns, None);
    assert_eq!(series.len(), 1);
    // cumulative_share is raw output tokens when cap is None.
    assert_eq!(series[0].cumulative_share, 100.0);
}
```

- [ ] **Step 2: Verify the tests fail**

Run: `cargo test --test series_test`
Expected: compilation error.

- [ ] **Step 3: Create `src/dashboard/series.rs`** with the 5h function

```rust
//! Per-turn cumulative-share series for the dashboard charts.

use crate::config::FIVE_HOUR_WINDOW_HOURS;
use crate::data::parser::Turn;
use chrono::{DateTime, Duration, Utc};

#[derive(Debug, Clone, PartialEq)]
pub struct WindowedTurn {
    pub ts: DateTime<Utc>,
    pub cumulative_share: f64,
    pub window_idx: usize,
}

/// Per-turn cumulative share across gap-detected 5h windows. If `cap` is None,
/// returns raw cumulative output tokens (not normalized).
pub fn cumulative_share_series_5h(turns: &[Turn], cap: Option<f64>) -> Vec<WindowedTurn> {
    let gap = Duration::milliseconds((FIVE_HOUR_WINDOW_HOURS * 3_600_000.0) as i64);
    let mut out: Vec<WindowedTurn> = Vec::with_capacity(turns.len());
    let mut current_start: Option<DateTime<Utc>> = None;
    let mut last_ts: Option<DateTime<Utc>> = None;
    let mut window_idx: usize = 0;
    let mut burn_in_window: u64 = 0;
    let mut first_in_window = true;

    for t in turns {
        match (current_start, last_ts) {
            (None, _) => {
                current_start = Some(t.ts);
            }
            (Some(start), Some(prev)) => {
                let since_last = t.ts - prev;
                let since_start = t.ts - start;
                if since_last >= gap || since_start >= gap {
                    current_start = Some(t.ts);
                    burn_in_window = 0;
                    window_idx += 1;
                    first_in_window = true;
                }
            }
            (Some(_), None) => unreachable!("current_start implies last_ts"),
        }
        burn_in_window += t.output_tokens;
        let share = match cap {
            Some(c) if c > 0.0 => burn_in_window as f64 / c,
            _ => burn_in_window as f64,
        };
        out.push(WindowedTurn {
            ts: t.ts,
            cumulative_share: share,
            window_idx,
        });
        last_ts = Some(t.ts);
        let _ = first_in_window;  // reserved for future visual hints
        first_in_window = false;
    }
    out
}
```

- [ ] **Step 4: Register the submodule**

In `src/dashboard/mod.rs`:

```rust
pub mod app;
pub mod bands;
pub mod range;
pub mod series;
```

- [ ] **Step 5: Verify the tests pass**

Run: `cargo test --test series_test`
Expected: 3 passed.

- [ ] **Step 6: Commit**

```bash
git add src/dashboard/series.rs src/dashboard/mod.rs tests/series_test.rs
git commit -m "feat(stage-6): add cumulative_share_series_5h with tests"
```

---

## Task 17: Implement `cumulative_share_series_weekly`

**Files:**
- Modify: `src/dashboard/series.rs`
- Modify: `tests/series_test.rs`

- [ ] **Step 1: Append failing tests**

To `tests/series_test.rs`:

```rust
use claude_usage_tray::dashboard::series::cumulative_share_series_weekly;

#[test]
fn cumulative_share_weekly_resets_at_sunday_0700_local() {
    // 2026-05-17 is a Sunday. CEST (May) = UTC+2. Sun 07:00 CEST = Sun 05:00 UTC.
    let turns = vec![
        turn(utc(2026, 5, 17, 4, 0), 999),  // before reset → excluded from window
        turn(utc(2026, 5, 17, 6, 0), 100),  // after reset → window 0
        turn(utc(2026, 5, 23, 12, 0), 200), // still window 0
        turn(utc(2026, 5, 24, 6, 0), 50),   // after next reset → window 1
    ];
    let series = cumulative_share_series_weekly(&turns, Some(1000.0));
    assert_eq!(series.len(), 4);
    // The pre-reset turn has its own "preceding" window_idx (we still emit it,
    // just with window_idx=0 and cumulative=999). The new week is window_idx=1+.
    // Implementation choice: window_idx increments at every reset, so pre-first-reset
    // turns are window_idx=0; turns after first reset are window_idx=1; etc.
    assert!(series[0].window_idx < series[1].window_idx
            || series[0].window_idx == 0 && series[1].window_idx > 0);

    // turn at 23 12:00 is same week as turn at 17 06:00 → cumulative continues
    let mid_share = series.iter().find(|w| w.ts == utc(2026, 5, 23, 12, 0)).unwrap();
    assert!((mid_share.cumulative_share - 0.3).abs() < 0.001);  // 100+200=300/1000

    // turn at 24 06:00 → fresh week → cumulative resets to 50/1000 = 0.05
    let next_share = series.iter().find(|w| w.ts == utc(2026, 5, 24, 6, 0)).unwrap();
    assert!((next_share.cumulative_share - 0.05).abs() < 0.001);
}
```

- [ ] **Step 2: Verify the tests fail**

Run: `cargo test --test series_test cumulative_share_weekly`
Expected: compilation error.

- [ ] **Step 3: Append to `src/dashboard/series.rs`**

```rust
use crate::calibration::anchors::last_weekly_reset;

/// Per-turn cumulative share within each fixed Sunday-07:00-local week.
pub fn cumulative_share_series_weekly(turns: &[Turn], cap: Option<f64>) -> Vec<WindowedTurn> {
    let mut out: Vec<WindowedTurn> = Vec::with_capacity(turns.len());
    let mut current_reset: Option<DateTime<Utc>> = None;
    let mut window_idx: usize = 0;
    let mut burn_in_window: u64 = 0;

    for t in turns {
        let this_reset = last_weekly_reset(t.ts);
        match current_reset {
            None => {
                current_reset = Some(this_reset);
            }
            Some(prev) if prev != this_reset => {
                current_reset = Some(this_reset);
                burn_in_window = 0;
                window_idx += 1;
            }
            _ => {}
        }
        burn_in_window += t.output_tokens;
        let share = match cap {
            Some(c) if c > 0.0 => burn_in_window as f64 / c,
            _ => burn_in_window as f64,
        };
        out.push(WindowedTurn {
            ts: t.ts,
            cumulative_share: share,
            window_idx,
        });
    }
    out
}
```

- [ ] **Step 4: Verify the tests pass**

Run: `cargo test --test series_test`
Expected: 4 passed.

- [ ] **Step 5: Commit**

```bash
git add src/dashboard/series.rs tests/series_test.rs
git commit -m "feat(stage-6): add cumulative_share_series_weekly"
```

---

## Task 18: Implement `daily_aggregates`

**Files:**
- Modify: `src/dashboard/series.rs`
- Modify: `tests/series_test.rs`

- [ ] **Step 1: Append failing tests**

To `tests/series_test.rs`:

```rust
use claude_usage_tray::dashboard::series::daily_aggregates;

#[test]
fn daily_aggregates_groups_by_local_date() {
    // 2 turns on 2026-05-24 (CEST), 1 turn on 2026-05-25 (CEST).
    // Each turn input=10, cache_create=10, cache_read=10, output=10.
    // cost_weighted per turn: 10*1 + 10*1.25 + 10*0.1 + 10*5 = 73.5.
    let mk = |ts: chrono::DateTime<chrono::Utc>| {
        let mut t = turn(ts, 10);
        t.input_tokens = 10;
        t.cache_creation_input_tokens = 10;
        t.cache_read_input_tokens = 10;
        t
    };
    // 2026-05-24 10:00 UTC = 12:00 CEST = May 24 local.
    // 2026-05-24 23:30 UTC = 01:30 CEST May 25 → May 25 local.
    // 2026-05-25 10:00 UTC = 12:00 CEST May 25 → May 25 local.
    let turns = vec![
        mk(utc(2026, 5, 24, 10, 0)),
        mk(utc(2026, 5, 24, 23, 30)),
        mk(utc(2026, 5, 25, 10, 0)),
    ];
    let daily = daily_aggregates(&turns);
    // May 24 local: 1 turn = 73.5. May 25 local: 2 turns = 147.0.
    assert_eq!(daily.len(), 2);
    // Find by date.
    let may24 = daily.iter().find(|(d, _)| d.day() == 24).unwrap();
    let may25 = daily.iter().find(|(d, _)| d.day() == 25).unwrap();
    assert!((may24.1 - 73.5).abs() < 0.01);
    assert!((may25.1 - 147.0).abs() < 0.01);
}

#[test]
fn daily_aggregates_empty_returns_empty() {
    let out = daily_aggregates(&[]);
    assert!(out.is_empty());
}
```

Note: the test needs `chrono::Datelike` in scope. Add at the top of `tests/series_test.rs`:

```rust
use chrono::Datelike;
```

- [ ] **Step 2: Verify the tests fail**

Run: `cargo test --test series_test daily_aggregates`
Expected: compilation error.

- [ ] **Step 3: Append to `src/dashboard/series.rs`**

```rust
use crate::config;
use crate::shared::snapshot::cost_weighted;
use chrono::NaiveDate;
use chrono_tz::Tz;
use std::collections::BTreeMap;

/// Sum cost-weighted tokens per local-date, returned in ascending date order.
pub fn daily_aggregates(turns: &[Turn]) -> Vec<(NaiveDate, f64)> {
    let tz: Tz = config::LOCAL_TZ.parse().expect("LOCAL_TZ must be valid IANA name");
    let mut map: BTreeMap<NaiveDate, f64> = BTreeMap::new();
    for t in turns {
        let local_date = t.ts.with_timezone(&tz).date_naive();
        *map.entry(local_date).or_default() += cost_weighted(t);
    }
    map.into_iter().collect()
}
```

- [ ] **Step 4: Verify the tests pass**

Run: `cargo test --test series_test`
Expected: 6 passed.

- [ ] **Step 5: Commit**

```bash
git add src/dashboard/series.rs tests/series_test.rs
git commit -m "feat(stage-6): add daily_aggregates with cost-weighted sum"
```

---

## Task 19: Implement KPI strip rendering

**Files:**
- Create: `src/dashboard/kpi.rs`
- Modify: `src/dashboard/mod.rs`
- Modify: `src/dashboard/app.rs`

- [ ] **Step 1: Create `src/dashboard/kpi.rs`**

```rust
//! KPI strip layout. Four equal-width columns above the charts.

use crate::shared::snapshot::DashboardKpis;
use egui::{Ui, Color32};

pub fn render(ui: &mut Ui, kpis: &DashboardKpis, caps_available: bool) {
    ui.columns(4, |cols| {
        kpi_share(&mut cols[0], "Peak 5h share", kpis.peak_5h_share, caps_available);
        kpi_share(&mut cols[1], "Peak weekly share", kpis.peak_week_share, caps_available);
        kpi_total(&mut cols[2], "Total burn", kpis.total_cost_weighted, "cost-weighted");
        kpi_rate(&mut cols[3], "Daily avg", kpis.daily_avg_cost_weighted, "/ day");
    });
}

fn kpi_share(ui: &mut Ui, label: &str, share: f64, caps_available: bool) {
    ui.label(egui::RichText::new(label).size(11.0).color(Color32::GRAY));
    if caps_available {
        ui.label(egui::RichText::new(format!("{}%", (share * 100.0).round() as i64)).size(22.0));
        let pct = share.clamp(0.0, 1.0);
        let (rect, _) = ui.allocate_exact_size(
            egui::vec2(ui.available_width(), 4.0),
            egui::Sense::hover(),
        );
        let painter = ui.painter();
        painter.rect_filled(rect, 1.0, Color32::from_gray(60));
        let mut fill_rect = rect;
        fill_rect.max.x = rect.min.x + rect.width() * pct as f32;
        painter.rect_filled(fill_rect, 1.0, Color32::from_rgb(79, 140, 255));
    } else {
        ui.label(egui::RichText::new("—").size(22.0).color(Color32::GRAY));
        let (rect, _) = ui.allocate_exact_size(
            egui::vec2(ui.available_width(), 4.0),
            egui::Sense::hover(),
        );
        ui.painter().rect_filled(rect, 1.0, Color32::from_gray(40));
    }
}

fn kpi_total(ui: &mut Ui, label: &str, value: f64, suffix: &str) {
    ui.label(egui::RichText::new(label).size(11.0).color(Color32::GRAY));
    ui.label(egui::RichText::new(format_si(value)).size(22.0));
    ui.label(egui::RichText::new(suffix).size(11.0).color(Color32::GRAY));
}

fn kpi_rate(ui: &mut Ui, label: &str, value: f64, suffix: &str) {
    ui.label(egui::RichText::new(label).size(11.0).color(Color32::GRAY));
    ui.label(egui::RichText::new(format_si(value)).size(22.0));
    ui.label(egui::RichText::new(suffix).size(11.0).color(Color32::GRAY));
}

/// Format a number with SI suffixes (e.g., 42_500_000 → "42.5M").
pub fn format_si(v: f64) -> String {
    let abs = v.abs();
    if abs >= 1e9 {
        format!("{:.1}B", v / 1e9)
    } else if abs >= 1e6 {
        format!("{:.1}M", v / 1e6)
    } else if abs >= 1e3 {
        format!("{:.1}K", v / 1e3)
    } else {
        format!("{:.0}", v)
    }
}
```

- [ ] **Step 2: Register the submodule in `src/dashboard/mod.rs`**

```rust
pub mod app;
pub mod bands;
pub mod kpi;
pub mod range;
pub mod series;
```

- [ ] **Step 3: Call the KPI strip from `DashboardApp::update`**

Edit `src/dashboard/app.rs`. Replace the placeholder `CentralPanel` body:

```rust
        egui::CentralPanel::default().show(ctx, |ui| {
            let snap = self.shared.read().unwrap().clone();
            let caps_available = snap.caps.cap_5h.is_some() || snap.caps.cap_week.is_some();
            ui.add_space(8.0);
            crate::dashboard::kpi::render(ui, &snap.kpis, caps_available);
            ui.add_space(16.0);
            ui.separator();
            ui.label(format!("Snapshot turns: {} (charts coming in next task)", snap.turns.len()));
        });
```

- [ ] **Step 4: Verify build + smoke test**

Run: `cargo build` then run the .exe. Left-click tray → dashboard window shows the KPI strip (likely zeroes if no anchors yet). Numbers should be rendered with SI suffixes.

Add a unit test for `format_si` in `src/dashboard/kpi.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn format_si_picks_suffix() {
        assert_eq!(format_si(42_500_000.0), "42.5M");
        assert_eq!(format_si(8_100.0), "8.1K");
        assert_eq!(format_si(950.0), "950");
        assert_eq!(format_si(0.0), "0");
        assert_eq!(format_si(1_200_000_000.0), "1.2B");
    }
}
```

Run: `cargo test --lib format_si`
Expected: 1 passed.

- [ ] **Step 5: Commit**

```bash
git add src/dashboard/kpi.rs src/dashboard/mod.rs src/dashboard/app.rs
git commit -m "feat(stage-6): render KPI strip with 4 metrics"
```

---

## Task 20: Render the 5h chart

**Files:**
- Create: `src/dashboard/chart_5h.rs`
- Modify: `src/dashboard/mod.rs`
- Modify: `src/dashboard/app.rs`

- [ ] **Step 1: Create `src/dashboard/chart_5h.rs`**

```rust
//! 5h cumulative-share chart: stepped line + cap line + hour-of-day overlay +
//! calendar bands + range selector.

use crate::dashboard::bands::{calendar_bands, BandKind};
use crate::dashboard::range::{clamp_x_range, Range};
use crate::dashboard::series::cumulative_share_series_5h;
use crate::shared::snapshot::AppSnapshot;
use chrono::{DateTime, Utc};
use egui::{Color32, Stroke, Ui};
use egui_plot::{Line, Plot, PlotPoints, Polygon, VLine};

const COLOR_LINE: Color32 = Color32::from_rgb(79, 140, 255);
const COLOR_BAND: Color32 = Color32::from_rgba_premultiplied(136, 136, 136, 22);
const COLOR_CAP: Color32 = Color32::from_rgb(120, 120, 120);
const COLOR_HOURLY: Color32 = Color32::from_rgba_premultiplied(180, 180, 180, 80);

pub fn render(ui: &mut Ui, snap: &AppSnapshot, range: &mut Range) {
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("5h cumulative share").strong());
        ui.separator();
        for &r in Range::VARIANTS {
            if ui.selectable_label(*range == r, r.label()).clicked() {
                *range = r;
            }
        }
    });

    let now = Utc::now();
    let (mut x_start, x_end) = clamp_x_range(now, *range);
    if *range == Range::All {
        if let Some(first) = snap.turns.first() {
            x_start = first.ts;
        }
    }

    let cap_5h = snap.caps.cap_5h;
    let series = cumulative_share_series_5h(&snap.turns, cap_5h);

    // X-coordinate helper: seconds-since-epoch as f64 for egui_plot.
    let x = |t: DateTime<Utc>| t.timestamp() as f64;

    // Filter series to the visible range.
    let visible: Vec<_> = series.iter()
        .filter(|w| w.ts >= x_start && w.ts <= x_end)
        .collect();

    // Group by window_idx to draw separate stepped segments.
    let mut segments: Vec<Vec<[f64; 2]>> = Vec::new();
    let mut current_win: isize = -1;
    for w in &visible {
        let pct = w.cumulative_share * 100.0;
        if w.window_idx as isize != current_win {
            segments.push(Vec::new());
            current_win = w.window_idx as isize;
        }
        segments.last_mut().unwrap().push([x(w.ts), pct]);
    }

    Plot::new("chart_5h")
        .height(280.0)
        .show_x(true)
        .show_y(true)
        .y_axis_label("% of cap")
        .show(ui, |plot_ui| {
            // Calendar bands.
            for (s, e, _kind) in calendar_bands(x_start, x_end) {
                plot_ui.polygon(
                    Polygon::new(PlotPoints::from(vec![
                        [x(s), 0.0],
                        [x(e), 0.0],
                        [x(e), 200.0],
                        [x(s), 200.0],
                    ]))
                    .fill_color(COLOR_BAND)
                    .stroke(Stroke::NONE),
                );
            }

            // Hour-of-day overlay (if cap_5h available).
            if let Some(cap) = cap_5h {
                let overlay = hourly_overlay_points(x_start, x_end, snap.hourly_5h, cap);
                plot_ui.line(
                    Line::new(PlotPoints::from(overlay))
                        .color(COLOR_HOURLY)
                        .style(egui_plot::LineStyle::dashed_loose())
                        .name("hourly cap"),
                );
            }

            // Cap line at 100% (if cap exists).
            if cap_5h.is_some() {
                plot_ui.hline(
                    egui_plot::HLine::new(100.0)
                        .color(COLOR_CAP)
                        .style(egui_plot::LineStyle::dashed_dense()),
                );
            }

            // Cumulative share segments.
            for (i, seg) in segments.iter().enumerate() {
                if seg.is_empty() { continue; }
                plot_ui.line(
                    Line::new(PlotPoints::from(seg.clone()))
                        .color(COLOR_LINE)
                        .name(if i == 0 { "5h share" } else { "" }),
                );
            }
        });
}

/// Sample the hour-of-day cap curve at each hour boundary in [x_start, x_end],
/// converting to (timestamp_seconds, percent-of-cap).
fn hourly_overlay_points(
    x_start: chrono::DateTime<Utc>,
    x_end: chrono::DateTime<Utc>,
    hourly: [f64; 24],
    cap: f64,
) -> Vec<[f64; 2]> {
    use chrono::Timelike;
    use chrono_tz::Tz;
    let tz: Tz = crate::config::LOCAL_TZ.parse().expect("LOCAL_TZ");

    // Step every hour.
    let mut out = Vec::new();
    let mut cur = x_start;
    while cur < x_end {
        let local = cur.with_timezone(&tz);
        let h = local.hour() as usize;
        let pct = (hourly[h] / cap) * 100.0;
        out.push([cur.timestamp() as f64, pct]);
        cur += chrono::Duration::hours(1);
    }
    out
}
```

- [ ] **Step 2: Register the submodule in `src/dashboard/mod.rs`**

```rust
pub mod app;
pub mod bands;
pub mod chart_5h;
pub mod kpi;
pub mod range;
pub mod series;
```

- [ ] **Step 3: Wire into `DashboardApp::update`**

Edit `src/dashboard/app.rs`:

```rust
use crate::dashboard::range::Range;

pub struct DashboardApp {
    shared: SharedSnapshot,
    hwnd_slot: Arc<Mutex<Option<SendHwnd>>>,
    hwnd_found: bool,
    range_5h: Range,
}

impl DashboardApp {
    pub fn new(shared: SharedSnapshot, hwnd_slot: Arc<Mutex<Option<SendHwnd>>>) -> Self {
        Self { shared, hwnd_slot, hwnd_found: false, range_5h: Range::D5 }
    }
    // ...existing discover_hwnd_if_needed unchanged...
}

impl eframe::App for DashboardApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.discover_hwnd_if_needed();

        egui::CentralPanel::default().show(ctx, |ui| {
            let snap = self.shared.read().unwrap().clone();
            let caps_available = snap.caps.cap_5h.is_some() || snap.caps.cap_week.is_some();
            ui.add_space(8.0);
            crate::dashboard::kpi::render(ui, &snap.kpis, caps_available);
            ui.add_space(16.0);
            ui.separator();
            ui.add_space(8.0);
            crate::dashboard::chart_5h::render(ui, &snap, &mut self.range_5h);
        });

        ctx.request_repaint_after(std::time::Duration::from_millis(33));
    }
}
```

- [ ] **Step 4: Verify build + smoke test**

Run: `cargo build --release` then launch. Left-click tray → dashboard shows KPI strip + 5h chart with stepped line, calendar bands (you should see weekend shading), and (if calibrated) a 100% cap line + hourly overlay. Click the range buttons (1D / 5D / 14D / 1M / All) — the chart x-range should adjust.

If anything is visually broken or doesn't compile, the most likely cause is a mismatch with egui_plot 0.29's API. The functions used (`Plot::new`, `Line::new`, `Polygon::new`, `HLine`, `LineStyle::dashed_loose`/`dashed_dense`) should all exist in 0.29 — verify against `cargo doc --open --package egui_plot`.

- [ ] **Step 5: Commit**

```bash
git add src/dashboard/chart_5h.rs src/dashboard/mod.rs src/dashboard/app.rs
git commit -m "feat(stage-6): render 5h chart with bands + cap line + hour overlay + range"
```

---

## Task 21: Render the weekly chart

**Files:**
- Create: `src/dashboard/chart_weekly.rs`
- Modify: `src/dashboard/mod.rs`
- Modify: `src/dashboard/app.rs`

- [ ] **Step 1: Create `src/dashboard/chart_weekly.rs`**

```rust
//! Weekly cumulative-share chart: per-week (Sun 07:00 local reset) stepped line.

use crate::dashboard::bands::calendar_bands;
use crate::dashboard::range::{clamp_x_range, Range};
use crate::dashboard::series::cumulative_share_series_weekly;
use crate::shared::snapshot::AppSnapshot;
use chrono::Utc;
use egui::{Color32, Stroke, Ui};
use egui_plot::{HLine, Line, LineStyle, Plot, PlotPoints, Polygon};

const COLOR_LINE: Color32 = Color32::from_rgb(79, 140, 255);
const COLOR_BAND: Color32 = Color32::from_rgba_premultiplied(136, 136, 136, 22);
const COLOR_CAP: Color32 = Color32::from_rgb(120, 120, 120);

pub fn render(ui: &mut Ui, snap: &AppSnapshot, range: &mut Range) {
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("Weekly cumulative share").strong());
        ui.separator();
        for &r in Range::VARIANTS {
            if ui.selectable_label(*range == r, r.label()).clicked() {
                *range = r;
            }
        }
    });

    let now = Utc::now();
    let (mut x_start, x_end) = clamp_x_range(now, *range);
    if *range == Range::All {
        if let Some(first) = snap.turns.first() {
            x_start = first.ts;
        }
    }

    let cap_week = snap.caps.cap_week;
    let series = cumulative_share_series_weekly(&snap.turns, cap_week);

    let x = |t: chrono::DateTime<Utc>| t.timestamp() as f64;

    let visible: Vec<_> = series.iter()
        .filter(|w| w.ts >= x_start && w.ts <= x_end)
        .collect();

    let mut segments: Vec<Vec<[f64; 2]>> = Vec::new();
    let mut current_win: isize = -1;
    for w in &visible {
        let pct = w.cumulative_share * 100.0;
        if w.window_idx as isize != current_win {
            segments.push(Vec::new());
            current_win = w.window_idx as isize;
        }
        segments.last_mut().unwrap().push([x(w.ts), pct]);
    }

    Plot::new("chart_weekly")
        .height(280.0)
        .show_x(true)
        .show_y(true)
        .y_axis_label("% of cap")
        .show(ui, |plot_ui| {
            for (s, e, _) in calendar_bands(x_start, x_end) {
                plot_ui.polygon(
                    Polygon::new(PlotPoints::from(vec![
                        [x(s), 0.0],
                        [x(e), 0.0],
                        [x(e), 200.0],
                        [x(s), 200.0],
                    ]))
                    .fill_color(COLOR_BAND)
                    .stroke(Stroke::NONE),
                );
            }
            if cap_week.is_some() {
                plot_ui.hline(
                    HLine::new(100.0).color(COLOR_CAP).style(LineStyle::dashed_dense()),
                );
            }
            for (i, seg) in segments.iter().enumerate() {
                if seg.is_empty() { continue; }
                plot_ui.line(
                    Line::new(PlotPoints::from(seg.clone()))
                        .color(COLOR_LINE)
                        .name(if i == 0 { "Weekly share" } else { "" }),
                );
            }
        });
}
```

- [ ] **Step 2: Register the submodule**

```rust
pub mod app;
pub mod bands;
pub mod chart_5h;
pub mod chart_weekly;
pub mod kpi;
pub mod range;
pub mod series;
```

- [ ] **Step 3: Wire into `DashboardApp`**

In `src/dashboard/app.rs`, add a `range_week: Range` field and initialize to `Range::D14`. Then update `update()`:

```rust
pub struct DashboardApp {
    shared: SharedSnapshot,
    hwnd_slot: Arc<Mutex<Option<SendHwnd>>>,
    hwnd_found: bool,
    range_5h: Range,
    range_week: Range,
}

impl DashboardApp {
    pub fn new(shared: SharedSnapshot, hwnd_slot: Arc<Mutex<Option<SendHwnd>>>) -> Self {
        Self {
            shared, hwnd_slot, hwnd_found: false,
            range_5h: Range::D5,
            range_week: Range::D14,
        }
    }
    // ...
}
```

In `update()`'s `CentralPanel`, after the 5h chart:

```rust
            ui.add_space(16.0);
            ui.separator();
            ui.add_space(8.0);
            crate::dashboard::chart_weekly::render(ui, &snap, &mut self.range_week);
```

- [ ] **Step 4: Smoke test**

Build + run + open dashboard. Verify weekly chart appears below 5h, with similar style + range buttons.

- [ ] **Step 5: Commit**

```bash
git add src/dashboard/chart_weekly.rs src/dashboard/mod.rs src/dashboard/app.rs
git commit -m "feat(stage-6): render weekly chart with per-week-reset cumulative share"
```

---

## Task 22: Render the daily bar chart

**Files:**
- Create: `src/dashboard/chart_daily.rs`
- Modify: `src/dashboard/mod.rs`
- Modify: `src/dashboard/app.rs`

- [ ] **Step 1: Create `src/dashboard/chart_daily.rs`**

```rust
//! Daily cost-weighted bar chart.

use crate::dashboard::range::{clamp_x_range, Range};
use crate::dashboard::series::daily_aggregates;
use crate::shared::snapshot::AppSnapshot;
use chrono::{NaiveDate, NaiveTime, TimeZone, Utc};
use chrono_tz::Tz;
use egui::{Color32, Ui};
use egui_plot::{Bar, BarChart, Plot};

const COLOR_BAR: Color32 = Color32::from_rgb(79, 140, 255);

pub fn render(ui: &mut Ui, snap: &AppSnapshot, range: &mut Range) {
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("Daily burn").strong());
        ui.separator();
        for &r in Range::VARIANTS {
            if ui.selectable_label(*range == r, r.label()).clicked() {
                *range = r;
            }
        }
    });

    let now = Utc::now();
    let (mut x_start, x_end) = clamp_x_range(now, *range);
    if *range == Range::All {
        if let Some(first) = snap.turns.first() {
            x_start = first.ts;
        }
    }

    let tz: Tz = crate::config::LOCAL_TZ.parse().expect("LOCAL_TZ");
    let aggregates = daily_aggregates(&snap.turns);

    // Filter to visible date range and convert to (timestamp, value) bars.
    let bars: Vec<Bar> = aggregates.iter()
        .filter_map(|(date, val)| {
            let date_naive = date.and_time(NaiveTime::from_hms_opt(12, 0, 0).unwrap());
            let local_dt = tz.from_local_datetime(&date_naive).single()?;
            let utc_dt = local_dt.with_timezone(&Utc);
            if utc_dt < x_start || utc_dt > x_end {
                return None;
            }
            Some(Bar::new(utc_dt.timestamp() as f64, *val).width(60_000.0))  // 60_000s ≈ ~17h wide
        })
        .collect();

    Plot::new("chart_daily")
        .height(220.0)
        .show_x(true)
        .show_y(true)
        .y_axis_label("cost-weighted")
        .show(ui, |plot_ui| {
            plot_ui.bar_chart(BarChart::new(bars).color(COLOR_BAR));
        });
}
```

- [ ] **Step 2: Register the submodule**

```rust
pub mod app;
pub mod bands;
pub mod chart_5h;
pub mod chart_daily;
pub mod chart_weekly;
pub mod kpi;
pub mod range;
pub mod series;
```

- [ ] **Step 3: Wire into `DashboardApp`**

Add `range_daily: Range` field, default `Range::D14`. After the weekly chart in `update()`:

```rust
            ui.add_space(16.0);
            ui.separator();
            ui.add_space(8.0);
            crate::dashboard::chart_daily::render(ui, &snap, &mut self.range_daily);
```

- [ ] **Step 4: Smoke test**

Build + run + open dashboard. All 3 charts should now appear; bars on the daily chart.

- [ ] **Step 5: Commit**

```bash
git add src/dashboard/chart_daily.rs src/dashboard/mod.rs src/dashboard/app.rs
git commit -m "feat(stage-6): render daily cost-weighted bar chart"
```

---

## Task 23: Use a scroll area so all 3 charts fit small windows

**Files:**
- Modify: `src/dashboard/app.rs`

- [ ] **Step 1: Wrap the central panel contents in a ScrollArea**

Edit `src/dashboard/app.rs::update`. Replace the body of `CentralPanel::default().show(ctx, |ui| { ... })` with:

```rust
        egui::CentralPanel::default().show(ctx, |ui| {
            let snap = self.shared.read().unwrap().clone();
            let caps_available = snap.caps.cap_5h.is_some() || snap.caps.cap_week.is_some();
            egui::ScrollArea::vertical().show(ui, |ui| {
                ui.add_space(8.0);
                crate::dashboard::kpi::render(ui, &snap.kpis, caps_available);
                ui.add_space(16.0);
                ui.separator();
                ui.add_space(8.0);
                crate::dashboard::chart_5h::render(ui, &snap, &mut self.range_5h);
                ui.add_space(16.0);
                ui.separator();
                ui.add_space(8.0);
                crate::dashboard::chart_weekly::render(ui, &snap, &mut self.range_week);
                ui.add_space(16.0);
                ui.separator();
                ui.add_space(8.0);
                crate::dashboard::chart_daily::render(ui, &snap, &mut self.range_daily);
                ui.add_space(8.0);
            });
        });
```

- [ ] **Step 2: Verify build + smoke test**

Run. Resize the window smaller — scrolling should let you see all 3 charts.

- [ ] **Step 3: Commit**

```bash
git add src/dashboard/app.rs
git commit -m "feat(stage-6): wrap dashboard in ScrollArea for small windows"
```

---

## Task 24: Add "uncalibrated" banner above charts when caps are None

**Files:**
- Modify: `src/dashboard/app.rs`

- [ ] **Step 1: Add a banner conditional**

In `src/dashboard/app.rs::update`, between the KPI strip and the first chart, add:

```rust
                if snap.caps.cap_5h.is_none() && snap.caps.cap_week.is_none() {
                    egui::Frame::default()
                        .fill(egui::Color32::from_rgb(60, 50, 30))
                        .inner_margin(egui::Margin::same(8.0))
                        .show(ui, |ui| {
                            ui.label(
                                egui::RichText::new(
                                    "Uncalibrated — charts show raw output tokens until first ≥95% anchor is observed in the calibration log.",
                                )
                                .color(egui::Color32::from_rgb(220, 200, 120))
                            );
                        });
                    ui.add_space(8.0);
                }
```

- [ ] **Step 2: Smoke test**

If your calibration log has no anchors yet (`tracing::debug! n_anchors_5h=0 n_anchors_week=0`), the banner appears. If anchors exist, it doesn't.

- [ ] **Step 3: Commit**

```bash
git add src/dashboard/app.rs
git commit -m "feat(stage-6): show uncalibrated banner when caps are None"
```

---

## Task 25: Final verification, version bump, manual smoke test, tag v0.6.0

**Files:**
- Modify: `Cargo.toml`
- Modify: `CLAUDE.md`

- [ ] **Step 1: Run fmt + clippy + tests cleanly**

```bash
cargo fmt --all
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

Fix any clippy issues introduced by Stage 6. Tests should show ~84 total.

- [ ] **Step 2: Release build**

```bash
cargo build --release
```

Expected: `target/release/claude-usage-tray.exe`, ~7-8 MB.

- [ ] **Step 3: Manual smoke test (full Stage 6 verification)**

```powershell
.\target\release\claude-usage-tray.exe
```

Verify each item:

1. **Tray icon appears** with the Stage 4 GDI+ rendered glyph.
2. **Tooltip on hover** shows the Stage 5 three-line format (`5h: NN%`, `local 5h: NN%`, `updated HH:MM (Ok)`) — unchanged from Stage 5.
3. **Left-click tray** → dashboard window opens within ~500 ms.
4. **KPI strip** shows 4 values with SI suffixes.
5. **5h chart** — stepped line, weekend bands, night bands, hour-of-day overlay (if calibrated), 100% cap line (if calibrated), range buttons work.
6. **Weekly chart** — stepped line per-week, range buttons work.
7. **Daily bar chart** — bars at each calendar day, range buttons work.
8. **Resize the window** → ScrollArea lets all charts be reachable in a small window.
9. **Click the X** to close → window closes; tray icon continues polling.
10. **Left-click tray again** → fresh dashboard appears.
11. **Minimize dashboard, left-click tray** → window restores to front.
12. **Right-click tray → Quit** → dashboard closes (if open) AND tray exits within ~1 s.
13. **Run for ≥30 minutes** with dashboard open, polling continues, no leaks.

If anything is broken, fix in place and re-test.

- [ ] **Step 4: Bump version to 0.6.0**

Edit `Cargo.toml`:

```toml
version = "0.6.0"
```

Run `cargo build` to update `Cargo.lock`.

- [ ] **Step 5: Update CLAUDE.md**

Update the stage roadmap table:

```markdown
| 6 | egui dashboard window | ✅ Shipped — tag `v0.6.0`, pushed to GitHub |
```

Add the Stage 6 spec + plan to "Active design + plans":

```markdown
- **Stage 6 spec:** `docs/superpowers/specs/2026-05-23-stage-6-dashboard-design.md` — native egui dashboard window design.
- **Stage 6 plan:** `docs/superpowers/plans/2026-05-23-stage-6-dashboard.md` — task plan. **Shipped 2026-05-23 (tag `v0.6.0`).**
```

Update the "Stages 5-8" line in the spec-and-plans intro to "Stages 6.5-8".

- [ ] **Step 6: Commit version bump + CLAUDE.md**

```bash
git add Cargo.toml Cargo.lock CLAUDE.md
git commit -m "release: bump version to 0.6.0 and update CLAUDE.md"
```

- [ ] **Step 7: Merge to main + tag**

```bash
git checkout main
git merge stage-6-dashboard --no-ff -m "Merge branch 'stage-6-dashboard' - Stage 6 native dashboard window (v0.6.0)"
git tag -a v0.6.0 -m "Stage 6 — native egui dashboard window

Adds left-click tray → dashboard window with three charts (5h cumulative
share, weekly cumulative share, daily cost-weighted bar) and a four-KPI strip.
Window lives on its own thread via eframe::run_native with
EventLoopBuilderExtWindows::with_any_thread(true). Quit cleanly posts
WM_CLOSE to the dashboard before joining its thread. Raise-to-front uses
Win32 EnumWindows against the window title."
git push origin main
git push origin v0.6.0
git branch -d stage-6-dashboard
```

---

## Summary

**Tasks:** 25 total. Most are TDD (test first → fail → implement → pass → commit). Tasks 11–13, 19–24 are integration/visual and verified via manual smoke test in addition to compiles.

**Test count target:** 64 prior + ~17 new = ~81 total. (Stage 6 adds: 1 cost_weighted, 4 compute_kpis, 6 range, 3 bands, 4 series, 1 format_si — and the chart-rendering modules themselves are visual-only.)

**Manual smoke checkpoints:** Tasks 12, 13, 20, 21, 22, 24 — each adds a user-visible piece that benefits from a quick "does it look right?" check before moving on.

**The riskiest tasks:**
- Task 12 (`launch` + LMB wiring) — first time `eframe::run_native` is called; non-main-thread EventLoop opt-in must work.
- Task 13 (shutdown) — WM_CLOSE must actually unblock the dashboard thread.
- Task 20 (5h chart) — first egui_plot rendering; API mismatches against egui_plot 0.29 may need small adaptations.

If any of these break, the most common fix is consulting `cargo doc --open --package eframe` (or `egui_plot`) to verify the exact API surface in 0.29.
