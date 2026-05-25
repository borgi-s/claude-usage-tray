# Stage 8 (mini-project 2) — Calibration History Tab

> Part of Stage 8 ("Streamlit feature parity"). Second Stage 8 deliverable,
> after mini-project 1 (sessions table + global filters, shipped `v0.8.0`).
> Subsequent mini-projects (live API status banner, settings panel) get their
> own specs.

## Goal

Add a **Calibration** tab to the egui dashboard that visualizes how the app's
derived caps are calibrated from the calibration log — at parity with the
Python Streamlit local agent's `render_calibration_history()` expander. Four
charts:

1. **Implied 5h cap over time** — scatter, one point per qualifying calibration
   sample, points colored by local hour-of-day band.
2. **Implied weekly cap over time** — same, for the weekly window.
3. **Hour-of-day cap bins (5h)** — per-local-hour median cap, IQR band, and the
   fitted (smoothed + interpolated) curve.
4. **Hour-of-day cap bins (weekly)** — same, for the weekly window.

This is the view that answers "where do my caps come from, and are they
drifting?" — the calibration math shipped in Stage 5 finally gets a window onto
its inputs.

## Non-goals (this mini-project)

- ❌ Per-point click / drill-down to the underlying sample. YAGNI for v1.
- ❌ A continuous HSV colorbar (Python uses one). Discrete hour bands instead —
  see Charts 1 & 2. egui_plot has no native per-point continuous colorbar.
- ❌ CSV / clipboard export.
- ❌ Re-deriving the fitted hour-of-day curve. The poll thread already computes
  `hourly_5h` / `hourly_week`; this tab reuses them verbatim.
- ❌ Touching the poll cadence, calibration math, icon, or tooltip.
- ❌ Applying the global filter bar to this tab (see Filter behavior).

## Background: what exists today

- **Stage 5** shipped `calibration/hourly.rs`: `per_hour_medians` (raw 24-bin
  medians), `smooth_rolling_circular`, `interpolate_empty_circular`, and the
  public `hour_of_day_cap_series` (the fitted curve). The fitted curve is
  computed every poll and stashed on `AppSnapshot.hourly_5h` / `hourly_week`.
- **Stage 5** also shipped `calibration/anchors.rs`: `five_hour_burn_at(turns,
  ts)` and `weekly_burn_at(turns, ts)` — output-token burn in the window
  containing `ts`.
- The **calibration log** (`~/.claude-usage-tray/calibration_log.jsonl`) is a
  `Vec<CalibrationSample>`. Schema (leaner than Python's parquet — no burn
  columns, those are recomputed from `turns`):

  ```rust
  pub struct CalibrationSample {
      pub schema_version: u32,
      pub ts: DateTime<Utc>,
      pub five_hour_util: Option<f64>,
      pub five_hour_resets_at: Option<DateTime<Utc>>,
      pub seven_day_util: Option<f64>,
      pub seven_day_resets_at: Option<DateTime<Utc>>,
      pub subscription_type: String,
      pub rate_limit_tier: String,
  }
  ```

- The **dashboard** (`dashboard/app.rs`) is a single long-lived `eframe::App`.
  Mini-project 1 added a `Charts | Sessions` tab strip, a global `FilterState`,
  a `filtered_view` memo keyed on a cheap `ViewSig`, and the off-screen-parking
  lifecycle. This mini-project extends that structure.

## Locked-in design decisions

Settled during the mini-project 2 brainstorm:

| Decision | Value |
|---|---|
| Chart scope | All 4 (full parity) |
| Placement | New third tab `Calibration` (reuses the tab strip) |
| Hour-of-day coloring (scatter) | 4 discrete hour bands + legend (not a continuous colorbar) |
| Global filter bar applies? | No — tab is always account-wide / global |
| Util range for implied cap | One range across the whole tab: `0.95 ≤ util ≤ 1.01` (the existing anchor range from `config`) — **not** Python's broader `0.10–0.95` scatter range |
| Full log access for the UI | New `AppSnapshot.log: Arc<Vec<CalibrationSample>>`, stashed by the poll thread |
| Derived-series computation | Lazy, in the dashboard, memoized on `(log.len(), turns.len())` |
| Fitted curve | Reuse `snap.hourly_5h` / `hourly_week` (poll-computed) |

### Why `0.95–1.01` everywhere (divergence from Python)

Python's `implied_cap_series` uses `0.10 ≤ util ≤ 0.95` for the over-time
scatters but the narrow anchor range for the hour-of-day bins. This Rust port
uses the **single** anchor range (`config::MIN_ANCHOR_UTIL ..=
config::MAX_ANCHOR_UTIL`, i.e. `0.95..=1.01`) across all four charts. Rationale:

- One definition of "implied cap" — every point on the tab is computed the same
  way the app actually derives its caps (`global_cap_from_anchors`), so the
  scatter's median visibly *is* the cap the rest of the app uses.
- At low utilization, `burn / util` is a noisy, biased cap estimate; the broad
  Python range trades accuracy for point density. Here we prefer fidelity to
  the real derivation.

Accepted trade-off: the over-time scatters are sparse until enough
near-saturation samples accumulate. Acceptable — the hour-of-day charts carry
the early-data story, and the over-time scatter is about *drift*, which only
matters once there's history.

## Filter behavior

The global filter bar (date / project / model) stays visible above all tabs but
is **inert** for the Calibration tab. Calibration is a property of the account,
derived from anchors across all projects/models — filtering the burn by project
while the util comes from account-wide API readings would mix two populations
and distort the implied cap. This is consistent with mini-project 1, where
`caps`, `hourly_5h`, and `hourly_week` were deliberately left global.

The Calibration tab therefore reads the **unfiltered** `snap.turns` and the
full `snap.log`, never the `FilteredView`.

## Architecture

### Data plumbing: the log reaches the UI thread

The only new cross-thread data is the calibration log. The poll thread already
calls `log::calibration::read_all` (or `read_all_default`) each tick to derive
caps. We stash that `Vec` on the snapshot:

```rust
// shared/snapshot.rs — AppSnapshot gains:
pub log: Arc<Vec<CalibrationSample>>,
```

- `Arc` so the per-frame `snap.clone()` in `app.rs` stays cheap.
- Populated in `tray/poller.rs` from the value it already read — no extra file
  I/O, no extra parse.
- `Default` is an empty `Arc<Vec<_>>` (first run, before any poll).

### Derived-series computation: lazy + memoized in the dashboard

The two derived inputs the charts need beyond the fitted curve —
`implied_cap_series` (scatter points) and `per_hour_stats` (median/IQR/count) —
are computed **in the dashboard**, not the poll thread, because:

- They're only needed while the Calibration tab is the active tab.
- They depend only on `log` + `turns`, both of which change only when the poll
  thread pushes new data.

So `DashboardApp` gains a second memo, mirroring the mini-project 1
`filtered_view` cache:

```rust
// Recomputed only when the signature changes; rebuilt at most once per poll.
struct CalibSig { n_log: usize, n_turns: usize }

struct CalibData {
    implied_5h: Vec<ImpliedPoint>,
    implied_week: Vec<ImpliedPoint>,
    stats_5h: [HourStat; 24],
    stats_week: [HourStat; 24],
}

// on DashboardApp:
cached_calib: Option<(CalibSig, CalibData)>,
```

> **Rust beginner note:** `(log.len(), turns.len())` is enough to detect change
> because both vectors are append-only — new samples and new turns only ever
> grow the length. We don't need to hash contents. Same trick as the
> `ViewSig` memo already in `app.rs`.

The fitted curve is *not* recomputed here — `snap.hourly_5h` / `hourly_week`
are read straight from the snapshot.

### Rejected alternatives

- **Precompute the derived series on the poll thread** (like the fitted curve).
  Rejected: it would run every poll regardless of which tab is open, and the
  scatter point list grows unbounded with log size. Lazy + memoized confines
  the cost to when the tab is actually viewed.
- **Read the calibration log directly from the UI thread.** Rejected: adds file
  I/O to the render loop and a second source of truth for log freshness; the
  poll thread already has the parsed log in hand.
- **Match Python's broader util range for the scatter.** Rejected — see "Why
  `0.95–1.01` everywhere".

## Components

### 1. `calibration/history.rs` (new — pure, tested)

```rust
use crate::calibration::WindowKind;
use crate::data::parser::Turn;
use crate::log::calibration::CalibrationSample;
use chrono::{DateTime, Utc};

/// One implied-cap observation derived from a single calibration sample.
pub struct ImpliedPoint {
    pub ts: DateTime<Utc>,
    pub cap: f64,        // burn_in_window(ts) / util, in raw output tokens
    pub local_hour: u32, // 0..=23, local-TZ hour of `ts` — drives the scatter band
}

/// Per-local-hour summary of implied caps across qualifying anchors.
#[derive(Clone, Default)]
pub struct HourStat {
    pub median: Option<f64>,
    pub p25: Option<f64>,
    pub p75: Option<f64>,
    pub n: usize,
}

/// Implied cap per qualifying sample, sorted by ts. A sample qualifies when its
/// util for `kind` is Some and within config::MIN_ANCHOR_UTIL..=MAX_ANCHOR_UTIL
/// and the window burn at its ts is > 0. cap = burn / util.
pub fn implied_cap_series(
    log: &[CalibrationSample],
    turns: &[Turn],
    kind: WindowKind,
) -> Vec<ImpliedPoint>;

/// Per-hour median / p25 / p75 / count of implied caps. Superset of the
/// existing per_hour_medians; bins by local-TZ hour of the sample ts.
pub fn per_hour_stats(
    log: &[CalibrationSample],
    turns: &[Turn],
    kind: WindowKind,
) -> [HourStat; 24];

/// Median of a slice (sorts in place). None if empty. Shared helper —
/// hourly.rs's inline median is refactored to call this.
pub fn median(values: &mut [f64]) -> Option<f64>;

/// p-th percentile (0.0..=1.0) via linear interpolation between order
/// statistics. Sorts in place. None if empty.
pub fn percentile(values: &mut [f64], p: f64) -> Option<f64>;
```

- `implied_cap_series` and `per_hour_stats` share the same per-sample qualify +
  `burn = match kind { FiveHour => five_hour_burn_at, Weekly => weekly_burn_at }`
  / `util` computation already used by `per_hour_medians` — factor that into a
  private `qualifying_implied(log, turns, kind) -> Vec<(DateTime<Utc>, f64)>`
  helper the public functions build on.
- `local_hour` uses `config::LOCAL_TZ`, exactly as `per_hour_medians` does.
- The existing `per_hour_medians` in `hourly.rs` is refactored to call the new
  shared `median` helper (so there is one median implementation). Its public
  signature and behavior are unchanged; `hour_of_day_cap_series` and the poll
  thread keep working as-is.

> **Subtlety:** `per_hour_stats` recomputes the per-hour medians independently
> of `hourly.rs::per_hour_medians`. That's fine — both consume the same anchor
> set and the same `median` helper, so `stats.median` and the raw medians that
> feed the fitted curve agree. The fitted curve adds smoothing + interpolation
> on top; the raw `stats.median` is plotted as the un-smoothed marker series.

### 2. `dashboard/calibration_tab.rs` (new — render)

`pub fn render(ui: &mut Ui, snap: &AppSnapshot, calib: &CalibData)` draws the
four plots inside a vertical `ScrollArea`. If `snap.log` has zero qualifying
anchors for a window (empty `implied_*`), that window's charts show an
"(uncalibrated — no ≥95% anchors observed yet)" notice instead of an empty
plot, mirroring the Charts-tab uncalibrated banner.

**Charts 1 & 2 — implied cap over time** (`egui_plot::Points`):

- Points partitioned into 4 hour-band series, each its own `Points` with a
  distinct color and `.name(...)` for the legend:

  | Band | Hours (local) | Color (suggested) |
  |---|---|---|
  | night | 0–5 | indigo |
  | morning | 6–11 | teal |
  | afternoon | 12–17 | amber |
  | evening | 18–23 | magenta |

- X = `ts.timestamp()` (seconds), formatted with the existing
  `dashboard::axis::format_x_tick`. Y = `cap / 1e6` (millions of tokens).
- Dashed `HLine` at the **median implied cap** (`median` of all points' caps,
  in millions) — this is the value the app's `global_cap_from_anchors` would
  land on. Labeled "median cap".
- Plot height ~240px to match Python.

**Charts 3 & 4 — hour-of-day cap bins** (`Polygon` + `Line` + `Points`):

- **IQR band**: a single filled `Polygon` tracing p25 left→right along hours
  0–23 then p75 right→left, over the hours where both are `Some`. Soft fill,
  no stroke — same construction as the existing calendar-band polygons in
  `chart_5h.rs`. Where a contiguous run of hours has stats, draw one polygon;
  break the polygon across hour gaps (bins with `n == 0`).
- **Median line + markers**: `Line` through `(hour, median/1e6)` for hours with
  `Some` median, plus a `Points` overlay with per-marker radius `2.0 + n as
  f64` (bigger marker = more samples), reproducing Python's count-scaled
  markers.
- **Fitted curve**: dashed `Line` through `(hour, snap.hourly_5h[h] / 1e6)` for
  all 24 hours (the fitted series is dense — interpolated). Labeled "fitted".
- X-axis 0–24 with `dtick`-style 3-hour grid (egui_plot auto-grid is fine;
  optional explicit formatter). Y in millions. Optional night-hours shading via
  the existing `dashboard::bands` if it generalizes cheaply; otherwise skip
  (non-goal-adjacent polish).

Colors live as `const Color32` at the top of the module, matching the
`chart_5h.rs` convention.

### 3. `dashboard/app.rs` — third tab + calib memo

- `enum Tab { Charts, Sessions, Calibration }` (add the variant).
- `DashboardApp` gains `cached_calib: Option<(CalibSig, CalibData)>`.
- New private method `calib_data(&mut self, snap: &AppSnapshot) -> CalibData`
  mirroring `filtered_view`: build `CalibSig { n_log: snap.log.len(), n_turns:
  snap.turns.len() }`; reuse cache on match, else recompute all four series via
  `calibration::history` (using **unfiltered** `snap.turns` + `snap.log`) and
  store.
- Tab strip gains a third `selectable_value(&mut self.tab, Tab::Calibration,
  "Calibration")`.
- `CentralPanel` match gains the `Tab::Calibration =>` arm:
  `calibration_tab::render(ui, &snap, &self.calib_data(&snap))`. Note it passes
  `&snap` (the full snapshot), **not** `&view` (the filtered one).

> The borrow dance: compute `let calib = self.calib_data(&snap);` before the
> `CentralPanel` closure (it needs `&mut self`), then move `calib` into the
> closure — same pattern the existing code uses for `view`.

### 4. `shared/snapshot.rs` + `tray/poller.rs`

- `AppSnapshot` gains `pub log: Arc<Vec<CalibrationSample>>` (with the import of
  `CalibrationSample`). `#[derive(Default)]` covers the empty case.
- `poller.rs`: wrap the already-read log in `Arc::new(...)` and set it on the
  `AppSnapshot` it publishes. If a given tick skips the calibration step (the
  Stage 5 error path), carry the previous log forward or publish empty — pick
  whichever the existing code does for `caps` (consistency over novelty).

### 5. `Cargo.toml`

No changes. `egui_plot` (0.29) and `egui_extras` (0.29) are already present;
`Points` / `Polygon` / `Line` / `HLine` are all in `egui_plot` 0.29.

## Data flow (this mini-project)

```
poll thread:
  log  = read_all(...)            (already done for caps)
  turns = cache::refresh(...)     (already done)
  ─► AppSnapshot { …, log: Arc::new(log), turns: Arc::new(turns), hourly_* }

dashboard (Calibration tab active):
  snap = shared.read().clone()
  calib = calib_data(&snap):          memo on (log.len, turns.len)
     implied_cap_series(log, turns, FiveHour) ─► implied_5h
     implied_cap_series(log, turns, Weekly)   ─► implied_week
     per_hour_stats(log, turns, FiveHour)     ─► stats_5h
     per_hour_stats(log, turns, Weekly)       ─► stats_week
  render:
     chart 1  implied_5h     (+ median HLine)
     chart 2  implied_week   (+ median HLine)
     chart 3  stats_5h IQR/median  + snap.hourly_5h fitted
     chart 4  stats_week IQR/median + snap.hourly_week fitted
```

## Testing

`cargo test` — pure functions only; the UI is smoke-tested by running.

- `median`: odd / even length, empty → None, does not panic on NaN-free input.
- `percentile`: p0 / p50 / p100, interpolated mid-rank, empty → None.
- `implied_cap_series`:
  - filters to the `0.95..=1.01` util range (a 0.5 sample and a 1.2 sample are
    excluded; a 0.97 sample is included);
  - `cap == burn_in_window(ts) / util`;
  - drops samples whose window burn is 0;
  - `local_hour` reflects `config::LOCAL_TZ`, not UTC;
  - empty log → empty Vec;
  - output sorted by ts.
- `per_hour_stats`:
  - bins by local hour; `n` counts samples per bin;
  - `median` / `p25` / `p75` correct for a bin with several samples;
  - empty bins → `HourStat::default()` (all None, n 0);
  - agrees with `hourly::per_hour_medians` on the median field for the same
    input (regression guard that the two paths share semantics).
- `hourly::per_hour_medians` regression: still passes after refactor onto the
  shared `median` helper (existing Stage 5 tests cover this).

**Fixture:** reuse / extend an existing calibration-log fixture (or build a
small `Vec<CalibrationSample>` + `Vec<Turn>` inline as the `hourly.rs` tests
already do) covering: samples across ≥3 distinct local hours, one out-of-range
util, one zero-burn window, and a multi-sample hour for percentile math.

## Verification before tagging

- `cargo fmt --check` clean.
- `cargo clippy --all-targets -- -D warnings` clean.
- `cargo test` — all pass (existing + ~10 new).
- `cargo build --release`.
- Run the .exe → open dashboard → Calibration tab shows 4 charts; if the real
  log has ≥95% anchors, the hour-of-day median markers and fitted curve appear
  and roughly agree; the over-time scatter shows hour-banded points with a
  median line. With a fresh/empty log, the tab shows the uncalibrated notice
  rather than empty plots.
- Confirm the global filter bar does **not** change the Calibration tab's
  charts (toggle a project filter; calibration charts stay put).
- Tag the release (next patch/minor per the project's versioning) and push.

## Out-of-scope follow-ups (future Stage 8 mini-projects)

- Live API status banner (last poll, 429 state, errors).
- Settings panel (cost-weight / weekly-reset / local-TZ overrides).

Both reuse the now-three-wide tab strip.

## Open questions deferred to implementation

- **Night-hours shading** on the hour-of-day charts — include if
  `dashboard::bands` generalizes to an hour axis cheaply; otherwise skip.
- **Exact band colors / legend placement** — tune by eye against the dark
  theme, matching the `chart_5h.rs` palette.
- **Carry-forward vs empty log on a skipped poll tick** — match whatever the
  existing `caps` handling does in `poller.rs`.
- **Y-axis units label** — "M tokens" vs "millions"; pick by eye.
