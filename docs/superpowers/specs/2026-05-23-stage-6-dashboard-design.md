# Stage 6 — Native Dashboard Window Design

Companion to the top-level [rust-tray-widget-design.md](2026-05-22-rust-tray-widget-design.md). This document fixes the details that the top-level spec left open for Stage 6.

## Goal

Add a native egui dashboard window to the existing tray app. The user left-clicks the tray icon and a real native window opens with three charts (5h cumulative-share, weekly cumulative-share, daily cost-weighted bar) and a four-KPI strip. Closing the window returns to tray-only mode; the tray keeps polling and the dashboard can be reopened from any subsequent left-click.

This is the project's CV centerpiece: it's the first piece of the app a viewer sees beyond the tray icon.

## Non-goals (Stage 6)

- ❌ Sessions table — Stage 8.
- ❌ Project/model filter sidebar — Stage 8.
- ❌ Calibration history scatter chart — Stage 8.
- ❌ Settings panel (cost weights, weekly reset config) — Stage 8.
- ❌ Window position/size persistence across launches — defer to Stage 8.
- ❌ Pro vs Max5x cap differentiation — Stage 1 known quirk; we display the single calibrated cap at 100% with no tier assertion.
- ❌ Concurrent multiple dashboard windows — exactly one allowed at any time.
- ❌ Re-rendering the icon based on dashboard state — Stage 4's icon logic is unchanged.
- ❌ Auto-refresh of charts independent of the polling cadence — charts redraw on egui's repaint schedule, reading whatever the polling thread last wrote.

## Locked-in design decisions

Settled during the Stage 6 brainstorm:

| Decision | Value |
|---|---|
| Charts in v0.6.0 | 5h cumulative-share + weekly cumulative-share + daily cost-weighted bar |
| KPI strip | 4 metrics: peak 5h share, peak weekly share, total cost-weighted, daily avg cost-weighted |
| Burn aggregate | Cost-weighted: `input×1 + cache_creation×1.25 + cache_read×0.1 + output×5` |
| Cap line | Single horizontal at 100% of calibrated cap; no Pro/Max5x split |
| Hour-of-day overlay | YES — faint dashed cap-vs-time curve on the 5h chart |
| Calendar bands | YES — light grey shading for weekend days + nights (22:00–06:00 local) |
| Range selector | YES — 1D / 5D / 14D / 1M / All buttons above each chart |
| Window lifecycle | Close button destroys; one window allowed; re-click raises if open |
| Threading | Dashboard runs on its own thread; eframe::run_native blocks that thread; `winit::EventLoopBuilder::with_any_thread(true)` opts into non-main-thread EventLoop creation on Windows |
| HWND raise-to-front | `EnumWindows`-by-title (`"Claude usage tracker"`), stored in `Arc<Mutex<Option<HWND>>>` — non-blocking handoff from dashboard thread to tray UI thread |
| Dashboard shutdown | On Quit, tray posts `WM_CLOSE` to the dashboard HWND, then joins the thread |
| Shared data | `Arc<RwLock<AppSnapshot>>` written by polling thread, read by tray + dashboard |
| Vec<Turn> sharing | Wrapped in `Arc<Vec<Turn>>` inside the snapshot to avoid 22 MB clones |
| Uncalibrated handling | If caps are None, charts show raw `output_tokens` on y-axis with no 100% line and a banner: "Uncalibrated — chart shows raw output tokens until first ≥95% anchor is observed" |

## Data flow

```text
┌─────────────────────────────────────────────────────────────────────────┐
│  Main thread: Win32 message loop                                         │
│                                                                          │
│  HWND (message-only) ──── owns TrayState                                │
│       ▲                                                                  │
│       │ WM_APP_POLL                                                      │
│       │ WM_APP_TRAYICON (gains LMB handling — spawns dashboard thread)   │
└───────┼──────────────────────────────────────────────────────────────────┘
        │
        ├── mpsc::channel<PollEvent>          ┌────────────────────────────┐
        │       ▲                              │  Dashboard thread (NEW)    │
        │       │                              │                            │
┌───────┴──────┴────────────────┐              │  spawned on LMB if no      │
│  Polling thread (existing)    │              │  prior thread alive        │
│                               │              │                            │
│  1. cache::refresh →           │              │  eframe::run_native(...)  │
│     Arc<Vec<Turn>>            │              │  blocks until close        │
│  2. derive_caps               │              │                            │
│  3. hourly cap series         │              │  reads Arc<RwLock<...>>    │
│  4. compute live util         │              │  every frame                │
│  5. compute cost-weighted     │              │                            │
│     KPIs                      │              │  Drop guard clears the     │
│  6. write snapshot to         │──────────────┤  global Option<…> on exit  │
│     Arc<RwLock<AppSnapshot>>  │              └────────────────────────────┘
│  7. send PollEvent via mpsc   │                       ▲
│  8. PostMessageW              │                       │
└───────────────────────────────┘                       │
                                            ┌───────────┴───────────┐
                                            │  Arc<RwLock<          │
                                            │     AppSnapshot>>     │
                                            │                       │
                                            │  - turns: Arc<…>      │
                                            │  - caps: DerivedCaps  │
                                            │  - hourly_5h / week    │
                                            │  - live_util          │
                                            │  - last_sample         │
                                            │  - last_status         │
                                            │  - last_kpis          │
                                            └───────────────────────┘
```

The dashboard never blocks the polling thread and the polling thread never blocks the dashboard — they meet via the `RwLock`. Writes are fast (replace a small struct + bump an Arc refcount); reads on the dashboard side hold the read lock for the duration of one frame's data extraction (~milliseconds).

## Module layout changes from Stage 5

```text
src/
  main.rs               — unchanged
  cli.rs                — unchanged
  config.rs             — ADD: COST_WEIGHTS struct, DEFAULT_RANGE constants
  paths.rs              — unchanged
  api/                  — unchanged
  log/                  — unchanged
  data/                 — unchanged
  calibration/          — unchanged
  tray/                 — small changes:
    icon.rs             — unchanged
    poller.rs           — ADD: write to Arc<RwLock<AppSnapshot>> at end of poll
    window.rs           — ADD: WM_LBUTTONUP handling spawns dashboard thread
    mod.rs              — wire up Arc<RwLock<AppSnapshot>> + dashboard handle
  shared/               — NEW
    mod.rs
    snapshot.rs         — AppSnapshot struct + KPIs struct + cost_weighted helper
  dashboard/            — NEW
    mod.rs              — eframe entry: launch + drop guard
    app.rs              — DashboardApp impl egui::App
    range.rs            — Range enum (D1 / D5 / D14 / M1 / All) + filtering
    chart_5h.rs         — 5h cumulative-share chart rendering
    chart_weekly.rs     — weekly cumulative-share chart rendering
    chart_daily.rs      — daily cost-weighted bar chart rendering
    kpi.rs              — KPI strip layout
    bands.rs            — calendar bands helper (weekend + nights)
  render.rs             — unchanged
  watch.rs              — unchanged
```

`shared/` is the new home for state that BOTH the tray UI thread and the dashboard thread read. Keeping it separate from `tray/` (which is UI-thread-local) and `dashboard/` (which is dashboard-thread-local) makes the boundary explicit.

`dashboard/` is the new top-level concern. One file per chart keeps each focused (each chart has its own data shape, layout, and quirks).

## Data model

```rust
// shared/snapshot.rs — what the polling thread writes; what the dashboard reads.
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
    pub peak_5h_share: f64,                 // max cumulative share across any 5h window
    pub peak_week_share: f64,               // max cumulative share across any weekly window
    pub total_cost_weighted: f64,            // sum across all turns (f64 because per-turn is fractional via cache_read×0.1)
    pub daily_avg_cost_weighted: f64,        // total / days_span
}

// shared/mod.rs — the lockable container the polling thread + UI threads share.
pub type SharedSnapshot = Arc<RwLock<AppSnapshot>>;

pub fn new_shared_snapshot() -> SharedSnapshot {
    Arc::new(RwLock::new(AppSnapshot::default()))
}
```

Beginner notes:
- `Arc<RwLock<T>>` is the canonical "multiple readers, one writer" container in Rust. `RwLock` blocks writers when readers hold the lock and vice versa; for our access pattern (write every 60s, read at ~60 fps) it's essentially uncontended.
- `Arc<Vec<Turn>>` inside `AppSnapshot` looks redundant ("Arc inside RwLock?") but isn't: cloning the outer Arc<RwLock<...>> is cheap, but cloning the *inner* Vec<Turn> when extracting a frame's data would be expensive (~22 MB on a real machine). By making the inner `Arc<Vec<Turn>>`, both the polling thread's writes and the dashboard's reads share the same backing allocation.

## Cost-weighted aggregation (deferred from Stage 5, landed here)

```rust
// config.rs — ADD
pub const COST_WEIGHT_INPUT: f64 = 1.0;
pub const COST_WEIGHT_CACHE_CREATION: f64 = 1.25;
pub const COST_WEIGHT_CACHE_READ: f64 = 0.1;
pub const COST_WEIGHT_OUTPUT: f64 = 5.0;
```

```rust
// shared/snapshot.rs — ADD
pub fn cost_weighted(turn: &Turn) -> f64 {
    turn.input_tokens as f64 * config::COST_WEIGHT_INPUT
        + turn.cache_creation_input_tokens as f64 * config::COST_WEIGHT_CACHE_CREATION
        + turn.cache_read_input_tokens as f64 * config::COST_WEIGHT_CACHE_READ
        + turn.output_tokens as f64 * config::COST_WEIGHT_OUTPUT
}
```

Computed at use-time, not stored on `Turn`. Keeps the cache schema unchanged (`SCHEMA_VERSION` still 1; no migration needed).

## Window lifecycle

```rust
// tray/window.rs — gains fields on TrayState
pub struct TrayState {
    // ...existing fields...
    pub shared: SharedSnapshot,
    pub dashboard: Arc<Mutex<Option<DashboardHandle>>>,
}

// dashboard/mod.rs
pub struct DashboardHandle {
    /// HWND of the dashboard window — populated asynchronously after the egui
    /// window is built. `None` until the first frame finds the HWND; the
    /// raise-to-front path checks `.lock().take()` and skips if still None.
    pub hwnd: Arc<Mutex<Option<HWND>>>,
    pub join: JoinHandle<()>,
}
```

The HWND is `Arc<Mutex<Option<HWND>>>`, not a bare `HWND`, so that `launch()` can return immediately to the UI thread without blocking. The dashboard thread populates it after `eframe::run_native` builds the window. See "HWND extraction" below.

**LMB on the tray icon** (`WM_APP_TRAYICON` with `lparam == WM_LBUTTONUP`):

```rust
fn on_left_click(state: &mut TrayState) {
    let mut guard = state.dashboard.lock().unwrap();
    match guard.as_ref() {
        Some(handle) if !handle.join.is_finished() => {
            // Still alive — try to raise to front. If HWND not populated yet,
            // skip — the window is still being created and will appear soon.
            if let Some(hwnd) = *handle.hwnd.lock().unwrap() {
                unsafe {
                    let _ = SetForegroundWindow(hwnd);
                    let _ = ShowWindow(hwnd, SW_RESTORE);
                }
            }
        }
        _ => {
            // No window, or thread has finished — spawn a new one.
            *guard = Some(dashboard::launch(state.shared.clone()));
        }
    }
}
```

**The dashboard launch is non-blocking** — `launch()` returns the handle immediately after spawning the thread:

```rust
// dashboard/mod.rs
pub fn launch(shared: SharedSnapshot) -> DashboardHandle {
    let hwnd_slot: Arc<Mutex<Option<HWND>>> = Arc::new(Mutex::new(None));
    let hwnd_slot_for_thread = hwnd_slot.clone();

    let join = std::thread::spawn(move || {
        let app = app::DashboardApp::new(shared, hwnd_slot_for_thread);
        let native_options = eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default()
                .with_inner_size([1100.0, 720.0])
                .with_min_inner_size([700.0, 480.0])
                .with_title(DASHBOARD_WINDOW_TITLE),
            // Allow EventLoop creation on a non-main thread. Without this,
            // winit on Windows panics ("EventLoop must be created on the
            // main thread").
            event_loop_builder: Some(Box::new(|builder| {
                use winit::platform::windows::EventLoopBuilderExtWindows;
                builder.with_any_thread(true);
            })),
            ..Default::default()
        };
        let _ = eframe::run_native(
            "claude-usage-tray-dashboard",
            native_options,
            Box::new(|_cc| Ok(Box::new(app))),
        );
        // run_native returned → window was closed → thread is about to exit.
    });

    DashboardHandle { hwnd: hwnd_slot, join }
}

pub const DASHBOARD_WINDOW_TITLE: &str = "Claude usage tracker";
```

The UI thread sees `DashboardHandle.hwnd` as `None` initially. Once egui builds its window (typically <300 ms but can be longer on cold GPU init), the dashboard thread writes the HWND. Any tray click that arrives before the HWND is populated finds `*handle.hwnd.lock().unwrap() == None` and skips the raise call. The window will pop on its own anyway since the OS gives focus to newly created top-level windows by default.

### HWND extraction

eframe 0.29 does **not** expose a stable way to retrieve the underlying HWND from inside `App::update` — `Frame::raw_window_handle()` was removed and not all eframe builds offer a clean replacement. The spec uses a Win32-native approach instead: find the HWND by enumerating top-level windows and matching on the unique title.

Strategy (inside `DashboardApp::update`, run only on the first frame):

```rust
impl eframe::App for DashboardApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if !self.hwnd_found {
            // Wait one frame so the OS has actually created the window, then
            // EnumWindows looking for our known title.
            if let Some(hwnd) = find_hwnd_by_title(DASHBOARD_WINDOW_TITLE) {
                *self.hwnd_slot.lock().unwrap() = Some(hwnd);
                self.hwnd_found = true;
            }
            // If not found this frame, retry next frame. Bounded by ~10 frames
            // (≈170 ms at 60fps) in practice.
        }
        // ...render frame...
    }
}

/// Win32 EnumWindows callback that matches the supplied title and stops.
fn find_hwnd_by_title(target: &str) -> Option<HWND> {
    // Implementation: GetWindowTextW into a buffer, compare UTF-16 to target,
    // return Some(hwnd) on match. Idiomatic Rust+windows-crate FFI.
}
```

This is ugly but reliable: title-based lookup doesn't depend on private eframe internals, can't break across eframe version bumps, and the title is unique to this process (no other application uses "Claude usage tracker" as a window title — and even if another window with that title appeared, the worst case is a benign no-op).

If HWND extraction never succeeds (e.g., another process happens to have a window with the exact same title and we pick the wrong one): raise-to-front is degraded to "no-op when the dashboard window is already open." The tray's `Some(handle) if !handle.join.is_finished()` check still prevents spawning duplicate dashboards. Acceptable degraded mode.

### Shutdown coordination

The user's "Quit" path must close the dashboard window before the tray thread joins it. The existing Stage 3 Quit handler in `wndproc` becomes:

```rust
WM_COMMAND => {
    if (wparam.0 & 0xFFFF) == IDM_QUIT {
        with_state(hwnd, |state| {
            // 1. Tell the polling thread to stop.
            state.shutdown.store(true, Ordering::Relaxed);

            // 2. If a dashboard is open, send it WM_CLOSE. The dashboard
            //    thread's eframe::run_native sees the close request and
            //    returns naturally, letting the JoinHandle complete.
            if let Some(handle) = state.dashboard.lock().unwrap().as_ref() {
                if let Some(dash_hwnd) = *handle.hwnd.lock().unwrap() {
                    unsafe {
                        let _ = PostMessageW(dash_hwnd, WM_CLOSE, WPARAM(0), LPARAM(0));
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

In `tray::run()`, after `window::message_loop()` returns and we've joined the polling thread, we also join the dashboard thread (if any):

```rust
// After the message loop exits:
if let Err(e) = poll_handle.join() {
    tracing::warn!(error = ?e, "polling thread panicked");
}
// Take ownership of any dashboard handle that's still around and join it.
// At this point we've already posted WM_CLOSE; this is just waiting for the
// thread to finish unwinding.
let dash_handle = state.dashboard.lock().unwrap().take();
if let Some(handle) = dash_handle {
    if let Err(e) = handle.join.join() {
        tracing::warn!(error = ?e, "dashboard thread panicked");
    }
}
```

Note: at the point we want to join, `state` is held inside the now-destroyed window. We need an `Arc<Mutex<Option<DashboardHandle>>>` clone held by `tray::run` directly so we can join after the window goes away. Adjust the existing `run()` to clone `dashboard` before passing `state` into `Box::new(...)`.

If the user closes the dashboard manually first, the join is essentially instant (thread already exited). If they Quit with the dashboard still open, the WM_CLOSE → eframe loop exit → thread exit chain runs in <100 ms typically. The whole-app shutdown stays bounded.

## Chart 1 — 5h cumulative-share

The marquee chart. Cumulative output share within each gap-detected 5h window (gap = 4.5h per Stage 5).

**Data preparation** (per repaint, ~60fps but cached frame-to-frame):

For each `Turn` in `snapshot.turns`, determine its window via the same gap-detection algorithm Stage 5 uses (`five_hour_burn_at` logic, but tracking windows instead of summing burn). Cumulative share within each window:

```rust
share[i] = sum(output_tokens for turns 0..=i in this window) / cap_5h
```

If `caps.cap_5h` is `None`, use `output_tokens` raw on y-axis and disable normalization.

**Visual layout (1100×~280 px):**

```
   ┌──[ 1D | 5D | 14D | 1M | All ]──────────────────────[ peak: 87% ]──┐
   │                                                                    │
   │   100% ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ │
   │                                       ╱╱╱╱                          │
   │  hourly-cap ............................ ............              │
   │      curve                  ╱╱╱╱╱            ╱╱╱╱╱                  │
   │                       ╱╱╱╱╱           ╱╱╱╱╱╱                       │
   │            ╱╱╱╱╱╱╱╱╱╱╱╱╱╱╱╱╱                                       │
   │  ▓▓▓░░░░▓▓▓                ▓▓▓░░░░▓▓▓                 ▓▓▓░░░░▓▓▓  │
   │  Sat Sun     Mon    Tue    Sat Sun     Mon    Tue     Sat Sun     │
   └────────────────────────────────────────────────────────────────────┘
```

- Stepped (shape=hv) cumulative-share line, blue (#4f8cff).
- Horizontal cap reference line at y=100%, dashed grey.
- Hour-of-day cap-curve overlay: a faint dashed line whose y-value at time t is `hourly_5h[local_hour(t)] / cap_5h`. Provides a hint of "your typical cap at this hour."
- Calendar bands: light grey rectangle behind weekend days (Sat 00:00 → Mon 00:00 local) and night hours (22:00 → 06:00 local) every day.
- Range selector buttons above the chart, clamping x-range. Default: 5D.
- Peak indicator shown in the header — `peak_5h_share` from `DashboardKpis`.

## Chart 2 — Weekly cumulative-share

Same shape as chart 1 but the window is the fixed Sunday-07:00-local week (per Stage 5 `last_weekly_reset`). Each week's cumulative share is plotted as a separate visual segment with line breaks at reset moments.

**Visual differences from chart 1:**
- No hour-of-day overlay (per the spec answer: "Stage 5 already builds the hour-of-day cap series" → 5h chart only).
- The peak indicator shows `peak_week_share`.
- Default range: 14D.

## Chart 3 — Daily cost-weighted bar

A simpler chart: vertical bars showing total cost-weighted tokens per UTC calendar day.

**Data preparation:**

```rust
// Group turns by date (local-tz date), sum cost_weighted per day.
let mut daily: BTreeMap<NaiveDate, f64> = BTreeMap::new();
for turn in turns.iter() {
    let local_date = turn.ts.with_timezone(&tz).date_naive();
    *daily.entry(local_date).or_default() += cost_weighted(turn);
}
```

**Visual layout:**

- Bars filled blue (#4f8cff).
- Y-axis: cost-weighted tokens (raw count, not normalized).
- Default range: 14D.
- No cap line (the daily bar is per-day, caps are per-window).
- Calendar bands omitted (the bars themselves communicate the date pattern).

## KPI strip

Top of the dashboard, above the charts. Four equal-width columns (`egui::Grid` or `columns`):

```
┌────────────────────┬────────────────────┬────────────────────┬────────────────────┐
│ Peak 5h share      │ Peak weekly share  │ Total burn         │ Daily avg          │
│ 87%                │ 64%                │ 42.5M cost-weighted│ 8.1M / day         │
│ ░░░░░░░░░░░░░░░░░░ │ ░░░░░░░░░░░░░░░░░░ │                    │                    │
└────────────────────┴────────────────────┴────────────────────┴────────────────────┘
```

- "Peak 5h share" / "Peak weekly share" show a percentage + a thin horizontal bar (progressbar at the same percentage, max 100%).
- "Total burn" shows `total_cost_weighted` formatted with SI suffixes (`42.5M`).
- "Daily avg" shows total / days-in-data.

If caps are None, the share KPIs show `—` and grey out the bar. The total + daily-avg KPIs always work since they don't depend on caps.

## Calendar bands

Module: `dashboard/bands.rs`.

```rust
/// Yields (start_utc, end_utc) for every weekend interval and night interval
/// inside `[range_start, range_end]`.
pub fn calendar_bands(
    range_start: DateTime<Utc>,
    range_end: DateTime<Utc>,
) -> Vec<(DateTime<Utc>, DateTime<Utc>, BandKind)> { /* ... */ }

pub enum BandKind { Weekend, Night }
```

Drawing: each band becomes an `egui_plot::Polygon` filled with a low-alpha grey (#888888 at 12% opacity).

Computation is straightforward iteration in local time (Europe/Copenhagen):
- Find first Sat in range → 48h interval → repeat every 7d.
- Find first day boundary in range → for each day, add 22:00→06:00 next-day band.

## Range selector

Module: `dashboard/range.rs`.

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Range {
    D1,
    D5,
    D14,
    M1,
    All,
}

impl Range {
    pub fn label(&self) -> &'static str { /* "1D" | "5D" | ... */ }
    pub fn duration(&self) -> Option<chrono::Duration> { /* None for All */ }
}

pub fn clamp_x_range(turns: &[Turn], now: DateTime<Utc>, range: Range)
    -> (DateTime<Utc>, DateTime<Utc>)
{
    let end = now;
    let start = match range.duration() {
        Some(d) => end - d,
        None => turns.first().map(|t| t.ts).unwrap_or(end),
    };
    (start, end)
}
```

Each chart owns its own `Range` field on `DashboardApp` (independent selection per chart). Defaults: 5h chart → 5D, weekly → 14D, daily bar → 14D.

Range buttons render as `egui::SelectableLabel`s in a horizontal row above each chart.

## Dependencies

```toml
# Cargo.toml additions
eframe = { version = "0.29", default-features = false, features = ["default_fonts", "glow"] }
egui = "0.29"
egui_plot = "0.29"
```

Notes on `eframe` features:
- `default_fonts` — bundles a default font so we don't depend on system Segoe UI.
- `glow` — uses OpenGL for rendering. Available on Windows out of the box.

`winit` is accessed transitively through `eframe::egui_winit::winit` — no direct `winit` entry in `Cargo.toml`. The `EventLoopBuilderExtWindows` trait we need is at `eframe::egui_winit::winit::platform::windows`.

`raw-window-handle` is no longer needed: HWND extraction goes through Win32 `EnumWindows`/`GetWindowTextW` instead, accessed via the existing `windows` crate (Stage 3 dependency).

Added binary size (release): ~3 MB (mostly eframe + egui_plot). Acceptable for the CV-piece value.

## Polling-thread changes

```rust
// tray/poller.rs — at the end of every tick, BEFORE sending the mpsc event:
let snapshot = AppSnapshot {
    turns: turns_arc.clone(),
    caps: caps.clone(),
    hourly_5h,
    hourly_week,
    live_util: live.clone(),
    last_sample: prior_last_sample.clone(),  // carried over from last good poll
    last_status: status.clone(),
    kpis: compute_kpis(&turns_arc, &caps),
};
*shared.write().unwrap() = snapshot;
```

`compute_kpis` is a pure function in `shared/snapshot.rs` that iterates `turns` to derive `peak_5h_share`, `peak_week_share`, `total_cost_weighted`, `daily_avg_cost_weighted`. It runs once per poll (every 60s default); the dashboard reads the pre-computed result every frame.

## Error handling

| Layer | Failure | Behavior |
|---|---|---|
| `eframe::run_native` | Returns Err (window init failed) | Log error, thread exits, dashboard handle becomes `is_finished() == true`, next click spawns fresh attempt |
| `EnumWindows` HWND lookup | Doesn't find a matching window in any frame | `hwnd_slot` stays `None`; raise-to-front becomes a no-op; the thread-alive check prevents duplicate spawns. Dashboard still fully functional. |
| Tray click before HWND is populated | First few frames after launch | `*handle.hwnd.lock() == None` → skip raise. The OS gives focus to newly created windows by default, so the user sees the dashboard appear anyway. |
| `SharedSnapshot` write/read | Poisoned lock (writer panicked) | Recover: replace with a fresh `AppSnapshot::default()` and `tracing::warn!` |
| Empty turns vec | No JSONL parsed yet | Charts show "No data yet — first poll in progress" placeholder text |
| `caps.cap_5h == None` | No anchors observed | Chart 1 shows raw output tokens + banner; cap line hidden; peak KPIs show "—" |
| `caps.cap_week == None` | No weekly anchors yet | Same for chart 2 |
| Dashboard thread panics | (e.g., out-of-memory in egui) | join.is_finished() detects it; next click spawns fresh thread; main process keeps running |

The tray and polling thread are NEVER blocked by dashboard failures. Closing or crashing the dashboard does not affect tray behavior.

## Testing

| Module | Test type | Notes |
|---|---|---|
| `shared::snapshot::cost_weighted` | Unit | Per-coefficient math; 1 test. |
| `shared::snapshot::compute_kpis` | Unit | Synthetic 5-turn vec, verify all 4 KPIs computed correctly. ~3 tests. |
| `dashboard::range::clamp_x_range` | Unit | Each `Range` variant + "All" edge case. ~5 tests. |
| `dashboard::bands::calendar_bands` | Unit | A known-week's weekends + nights, fixed local TZ. ~3 tests. |
| `dashboard::chart_5h::cumulative_share_series` | Unit | Synthetic turns + cap; verify stepped output. ~3 tests. |
| `dashboard::chart_weekly::cumulative_share_series` | Unit | Per-week reset behavior. ~3 tests. |
| `dashboard::chart_daily::daily_aggregates` | Unit | Group-by-day correctness. ~2 tests. |
| `dashboard::find_hwnd_by_title` | Manual / smoke | Win32 FFI — verify by running the .exe + checking that left-click-while-open raises the window. No automated test (would require spawning a real window in a test harness). |
| Existing 64 Stage 1–5 tests | Regression | Must continue to pass. |

Target: ~20 new tests, ~84 total.

**No unit tests for egui rendering itself.** Native UI doesn't unit-test cleanly. Manual smoke test verifies the visual output.

## Stage 6 verification (before tagging v0.6.0)

- `cargo fmt --check` clean.
- `cargo clippy --all-targets -- -D warnings` clean.
- `cargo test` — all tests pass.
- `cargo build --release` → exe size ~7.5 MB (Stage 5 ~4.5 MB + eframe ~3 MB).
- Run the .exe → tray icon appears as in Stage 5.
- Left-click the tray icon → dashboard window opens within ~500 ms, populated with charts.
- Verify each chart:
  - 5h chart shows the cumulative-share line with calendar bands and hour-of-day overlay.
  - Weekly chart shows the same shape, reset at each Sunday 07:00 local.
  - Daily bar shows last 14 days of cost-weighted totals.
- Verify KPI strip shows 4 metrics; sanity-check the values against the Streamlit dashboard.
- Click the range buttons (1D/5D/14D/1M/All) — chart x-range updates.
- Close the dashboard window → tray icon and tooltip continue updating.
- Left-click tray icon again → fresh dashboard opens.
- Minimize the dashboard, then left-click tray icon → window restores to front.
- Right-click tray + Quit → dashboard window closes if open, then the app exits cleanly within ~1 s.
- Run for ≥1 h with the dashboard open → memory usage stable, no GDI/handle leaks.
- Tag `v0.6.0` and push.

## Carry-overs from Stage 5 (unchanged)

- Calibration log schema.
- Cache.bincode + cache_manifest.json layout.
- Calibration math (`global_cap_from_anchors`, `derive_caps`, hour-of-day series).
- Tooltip format with local-util lines.
- Polling cadence (60/120/300 s; default 120 s).
- Icon rendering pipeline.
- Tray right-click menu.

## Stage 6 enabling Stage 8

Stage 8 (Streamlit feature parity) adds on top of Stage 6:
- Sessions table panel — reads `turns` from the shared snapshot, groups by `session_id`.
- Filter sidebar — adds filter state to `DashboardApp`, applies before chart rendering.
- Calibration history scatter — reads `log` (calibration log entries) and overlays implied-cap vs time.
- Live API status banner — reads `last_sample` + `last_status` from the shared snapshot.
- Settings panel — modifies `~/.claude-usage-tray/config.toml` (Stage 8 also adds the config file).

Stage 6's shared-snapshot + dashboard module structure should not require redesign for Stage 8 — each new feature is an additional read-only consumer of the snapshot plus a new file under `dashboard/`.

## Open questions deferred to implementation

- **Exact font** — egui's default font on Windows is a generic sans-serif via the bundled `default_fonts`. We may swap to Segoe UI Variable for visual polish; decide after first usable build.
- **Exact color palette** — using approximate Plotly defaults for now (#4f8cff blue, #ff8a4f orange, #888 grey). A more polished palette is a follow-up tweak.
- **Whether `compute_kpis` is fast enough on a 1M-turn cache** — first profile on real data after wiring up. If it's slow, cache the result on the snapshot and only recompute when `turns` Arc changes identity. Probably fine without optimization.
- **Whether to use `egui::CentralPanel` or `egui::SidePanel` layout for the KPI+3-charts arrangement** — visual decision; mockup in implementation.
- **Initial window size** — proposed 1100×720, may tune.
- **Eframe version pinning** — currently proposes 0.29. After Stage 6 ships, treat any eframe upgrade as a "verify the non-main-thread + EnumWindows-by-title contract still holds" task, since both depend on internals that aren't guaranteed stable across major versions.
