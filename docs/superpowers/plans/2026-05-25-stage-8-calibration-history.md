# Stage 8 (mini-project 2) — Calibration History Tab Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a third dashboard tab, **Calibration**, with four egui_plot charts (implied 5h/weekly cap over time as hour-banded scatters, and hour-of-day cap bins with median line + IQR band + fitted curve for 5h/weekly) at parity with the Python `render_calibration_history()`.

**Architecture:** A new pure module `calibration/history.rs` derives the chart inputs (`implied_cap_series`, `per_hour_stats`) from the calibration log + turns, reusing the Stage-5 anchor filter and burn windows. The poll thread stashes the already-read log on `AppSnapshot`; the dashboard computes the derived series lazily, memoized on `(log.len(), turns.len())`, and renders them in a new `dashboard/calibration_tab.rs`. The tab ignores the global filter bar (calibration is account-wide).

**Tech Stack:** Rust stable (MSVC), `egui`/`eframe`/`egui_plot` 0.29, `chrono` + `chrono-tz`. No new dependencies.

**Spec:** `docs/superpowers/specs/2026-05-25-stage-8-calibration-history-design.md`

---

## File structure

- **Create** `src/calibration/history.rs` — pure: `median`, `percentile`, `ImpliedPoint`, `HourStat`, `qualifying_implied` (private), `implied_cap_series`, `per_hour_stats`. Unit-tested.
- **Modify** `src/calibration/mod.rs` — add `pub mod history;`.
- **Modify** `src/calibration/hourly.rs` — refactor `per_hour_medians` to call `history::median` (DRY; one median impl).
- **Modify** `src/shared/snapshot.rs` — `AppSnapshot` gains `log: Arc<Vec<CalibrationSample>>`.
- **Modify** `src/tray/poller.rs` — `compute_calibration_with_turns` also returns the log `Arc`; the published `AppSnapshot` carries it.
- **Create** `src/dashboard/calibration_tab.rs` — `CalibData` struct + `render()` drawing the four plots.
- **Modify** `src/dashboard/mod.rs` — add `pub mod calibration_tab;`.
- **Modify** `src/dashboard/app.rs` — `Tab::Calibration`, `CalibSig`, `cached_calib`, `calib_data()` memo, tab-strip entry, central-panel arm.

---

## Task 1: `calibration/history.rs` — `median` + `percentile` helpers

**Files:**
- Create: `src/calibration/history.rs`
- Modify: `src/calibration/mod.rs`

- [ ] **Step 1: Register the module**

In `src/calibration/mod.rs`, add the new module declaration after the existing `pub mod live;` (line 6):

```rust
pub mod anchors;
pub mod history;
pub mod hourly;
pub mod live;
```

- [ ] **Step 2: Write the failing tests**

Create `src/calibration/history.rs` with just the test module and empty function stubs so it compiles-then-fails on assertions:

```rust
//! Calibration-history math: implied-cap series + per-hour statistics for the
//! dashboard's Calibration tab. Pure functions; UI lives in
//! `dashboard/calibration_tab.rs`.

/// Median of a slice (sorts in place). `None` if empty.
pub fn median(_values: &mut [f64]) -> Option<f64> {
    unimplemented!()
}

/// p-th percentile (`p` in 0.0..=1.0) via linear interpolation between order
/// statistics. Sorts in place. `None` if empty.
pub fn percentile(_values: &mut [f64], _p: f64) -> Option<f64> {
    unimplemented!()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn median_handles_odd_even_empty() {
        assert_eq!(median(&mut []), None);
        assert_eq!(median(&mut [5.0]), Some(5.0));
        assert_eq!(median(&mut [3.0, 1.0, 2.0]), Some(2.0));
        assert_eq!(median(&mut [4.0, 1.0, 3.0, 2.0]), Some(2.5));
    }

    #[test]
    fn percentile_interpolates_between_order_stats() {
        assert_eq!(percentile(&mut [], 0.5), None);
        assert_eq!(percentile(&mut [10.0], 0.25), Some(10.0));
        assert_eq!(percentile(&mut [0.0, 1.0, 2.0, 3.0, 4.0], 0.5), Some(2.0));
        assert_eq!(percentile(&mut [0.0, 1.0, 2.0, 3.0, 4.0], 0.25), Some(1.0));
        assert_eq!(percentile(&mut [0.0, 1.0, 2.0, 3.0, 4.0], 0.75), Some(3.0));
        // Two values, p25 interpolates: 0 + (10-0)*0.25 = 2.5
        assert_eq!(percentile(&mut [0.0, 10.0], 0.25), Some(2.5));
    }
}
```

> **Rust note:** `&mut [f64]` (a mutable slice) lets the function sort the caller's buffer in place — no allocation. `&mut Vec<f64>` deref-coerces to `&mut [f64]`, so callers can pass either.

- [ ] **Step 3: Run the tests to verify they fail**

Run: `cargo test --lib calibration::history`
Expected: compiles, then panics with `not implemented` (both tests fail).

- [ ] **Step 4: Implement `median` and `percentile`**

Replace the two stub bodies:

```rust
/// Median of a slice (sorts in place). `None` if empty.
pub fn median(values: &mut [f64]) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let n = values.len();
    Some(if n % 2 == 1 {
        values[n / 2]
    } else {
        (values[n / 2 - 1] + values[n / 2]) / 2.0
    })
}

/// p-th percentile (`p` in 0.0..=1.0) via linear interpolation between order
/// statistics. Sorts in place. `None` if empty.
pub fn percentile(values: &mut [f64], p: f64) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let n = values.len();
    if n == 1 {
        return Some(values[0]);
    }
    let rank = p.clamp(0.0, 1.0) * (n - 1) as f64;
    let lo = rank.floor() as usize;
    let hi = rank.ceil() as usize;
    let frac = rank - lo as f64;
    Some(values[lo] + (values[hi] - values[lo]) * frac)
}
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test --lib calibration::history`
Expected: PASS (2 tests).

- [ ] **Step 6: Commit**

```bash
git add src/calibration/history.rs src/calibration/mod.rs
git commit -m "feat(stage-8): calibration-history median + percentile helpers"
```

---

## Task 2: Refactor `hourly::per_hour_medians` onto the shared `median`

Eliminates the duplicate inline median in `hourly.rs`. Behavior is identical (`median` returns `None` for empty bins, subsuming the `is_empty` guard).

**Files:**
- Modify: `src/calibration/hourly.rs:44-57`

- [ ] **Step 1: Replace the inline median block**

In `src/calibration/hourly.rs`, find the tail of `per_hour_medians` (the block that builds `out` from `buckets`):

```rust
    let mut out: [Option<f64>; 24] = [None; 24];
    for (h, samples) in buckets.iter_mut().enumerate() {
        if samples.is_empty() {
            continue;
        }
        samples.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let n = samples.len();
        out[h] = Some(if n % 2 == 1 {
            samples[n / 2]
        } else {
            (samples[n / 2 - 1] + samples[n / 2]) / 2.0
        });
    }
    out
```

Replace it with:

```rust
    let mut out: [Option<f64>; 24] = [None; 24];
    for (h, samples) in buckets.iter_mut().enumerate() {
        out[h] = crate::calibration::history::median(samples);
    }
    out
```

- [ ] **Step 2: Run the existing hourly tests (regression)**

Run: `cargo test --lib calibration::hourly`
Expected: PASS — all existing Stage-5 tests still green (the refactor is behavior-preserving).

- [ ] **Step 3: Commit**

```bash
git add src/calibration/hourly.rs
git commit -m "refactor(stage-8): per_hour_medians reuses shared median helper"
```

---

## Task 3: `ImpliedPoint` + `implied_cap_series`

One implied-cap observation per qualifying calibration sample, sorted by ts, tagged with the local hour-of-day for the scatter bands.

**Files:**
- Modify: `src/calibration/history.rs`

- [ ] **Step 1: Add imports, the private qualify helper, `ImpliedPoint`, and a stub**

At the top of `src/calibration/history.rs` (below the module doc comment), add:

```rust
use crate::calibration::anchors::{five_hour_burn_at, weekly_burn_at};
use crate::calibration::WindowKind;
use crate::config;
use crate::data::parser::Turn;
use crate::log::calibration::CalibrationSample;
use chrono::{DateTime, Timelike, Utc};
use chrono_tz::Tz;

/// (sample ts, implied cap in raw output tokens) for every sample that
/// qualifies as an anchor for `kind`: util present, within
/// `config::MIN_ANCHOR_UTIL..=MAX_ANCHOR_UTIL`, and window burn > 0.
fn qualifying_implied(
    log: &[CalibrationSample],
    turns: &[Turn],
    kind: WindowKind,
) -> Vec<(DateTime<Utc>, f64)> {
    let mut out = Vec::new();
    for s in log {
        let util_opt = match kind {
            WindowKind::FiveHour => s.five_hour_util,
            WindowKind::Weekly => s.seven_day_util,
        };
        let Some(util) = util_opt else { continue };
        if !(config::MIN_ANCHOR_UTIL..=config::MAX_ANCHOR_UTIL).contains(&util) {
            continue;
        }
        let burn = match kind {
            WindowKind::FiveHour => five_hour_burn_at(turns, s.ts),
            WindowKind::Weekly => weekly_burn_at(turns, s.ts),
        };
        if burn == 0 || util <= 0.0 {
            continue;
        }
        out.push((s.ts, burn as f64 / util));
    }
    out
}

/// One implied-cap observation derived from a single calibration sample.
#[derive(Debug, Clone)]
pub struct ImpliedPoint {
    pub ts: DateTime<Utc>,
    pub cap: f64,        // raw output tokens
    pub local_hour: u32, // 0..=23, local-TZ hour of `ts`
}

/// Implied cap per qualifying sample, sorted by ts.
pub fn implied_cap_series(
    _log: &[CalibrationSample],
    _turns: &[Turn],
    _kind: WindowKind,
) -> Vec<ImpliedPoint> {
    unimplemented!()
}
```

> **Rust note:** `qualifying_implied` is the shared core both public functions build on (DRY). It mirrors the qualify logic already in `hourly::per_hour_medians`, so the scatter and the bins agree on which samples count.

- [ ] **Step 2: Add the tests**

Add these to the `tests` module in `history.rs` (extend the existing `mod tests`). First add the shared test builders at the top of `mod tests`:

```rust
    use crate::calibration::WindowKind;
    use crate::data::parser::Turn;
    use crate::log::calibration::CalibrationSample;
    use chrono::{DateTime, TimeZone, Utc};
    use std::path::PathBuf;

    fn utc(y: i32, m: u32, d: u32, h: u32, mi: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(y, m, d, h, mi, 0).unwrap()
    }

    fn turn(ts: DateTime<Utc>, output: u64) -> Turn {
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

    fn sample(ts: DateTime<Utc>, util: f64) -> CalibrationSample {
        CalibrationSample {
            schema_version: 1,
            ts,
            five_hour_util: Some(util),
            five_hour_resets_at: None,
            seven_day_util: Some(util),
            seven_day_resets_at: None,
            subscription_type: "pro".into(),
            rate_limit_tier: "default".into(),
        }
    }
```

Then the test functions:

```rust
    #[test]
    fn implied_filters_util_range_and_computes_cap() {
        // Anchor 2026-05-24 14:00 UTC, util 1.0, one prior turn of 100 output
        // tokens in the same 5h window → implied cap = 100 / 1.0 = 100.
        let turns = vec![turn(utc(2026, 5, 24, 13, 0), 100)];
        let log = vec![
            sample(utc(2026, 5, 24, 14, 0), 1.0),  // qualifies
            sample(utc(2026, 5, 24, 15, 0), 0.5),  // util too low → excluded
            sample(utc(2026, 5, 24, 16, 0), 1.2),  // util too high → excluded
        ];
        let pts = implied_cap_series(&log, &turns, WindowKind::FiveHour);
        assert_eq!(pts.len(), 1);
        assert!((pts[0].cap - 100.0).abs() < 1e-9);
    }

    #[test]
    fn implied_drops_zero_burn_windows() {
        // Util qualifies but there are no turns → burn 0 → dropped.
        let log = vec![sample(utc(2026, 5, 24, 14, 0), 1.0)];
        let pts = implied_cap_series(&log, &[], WindowKind::FiveHour);
        assert!(pts.is_empty());
    }

    #[test]
    fn implied_local_hour_is_local_not_utc() {
        // 14:00 UTC = 16:00 local (Europe/Copenhagen, CEST).
        let turns = vec![turn(utc(2026, 5, 24, 13, 0), 100)];
        let log = vec![sample(utc(2026, 5, 24, 14, 0), 1.0)];
        let pts = implied_cap_series(&log, &turns, WindowKind::FiveHour);
        assert_eq!(pts[0].local_hour, 16);
    }

    #[test]
    fn implied_empty_log_is_empty() {
        assert!(implied_cap_series(&[], &[], WindowKind::FiveHour).is_empty());
    }
```

- [ ] **Step 3: Run the tests to verify they fail**

Run: `cargo test --lib calibration::history`
Expected: the four `implied_*` tests panic with `not implemented`.

- [ ] **Step 4: Implement `implied_cap_series`**

Replace the stub body:

```rust
/// Implied cap per qualifying sample, sorted by ts.
pub fn implied_cap_series(
    log: &[CalibrationSample],
    turns: &[Turn],
    kind: WindowKind,
) -> Vec<ImpliedPoint> {
    let tz: Tz = config::LOCAL_TZ
        .parse()
        .expect("LOCAL_TZ must be a valid IANA name");
    let mut out: Vec<ImpliedPoint> = qualifying_implied(log, turns, kind)
        .into_iter()
        .map(|(ts, cap)| ImpliedPoint {
            ts,
            cap,
            local_hour: ts.with_timezone(&tz).hour(),
        })
        .collect();
    out.sort_by_key(|p| p.ts);
    out
}
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test --lib calibration::history`
Expected: PASS (median + percentile + 4 implied tests).

- [ ] **Step 6: Commit**

```bash
git add src/calibration/history.rs
git commit -m "feat(stage-8): implied_cap_series (hour-tagged anchor caps)"
```

---

## Task 4: `HourStat` + `per_hour_stats`

Per-local-hour median / p25 / p75 / count, for the hour-of-day charts' median line, IQR band, and count-scaled markers.

**Files:**
- Modify: `src/calibration/history.rs`

- [ ] **Step 1: Add `HourStat` + a stub**

In `src/calibration/history.rs`, below `implied_cap_series`, add:

```rust
/// Per-local-hour summary of implied caps across qualifying anchors.
#[derive(Debug, Clone, Default)]
pub struct HourStat {
    pub median: Option<f64>,
    pub p25: Option<f64>,
    pub p75: Option<f64>,
    pub n: usize,
}

/// Median / p25 / p75 / count of implied caps per local hour-of-day bin.
pub fn per_hour_stats(
    _log: &[CalibrationSample],
    _turns: &[Turn],
    _kind: WindowKind,
) -> [HourStat; 24] {
    unimplemented!()
}
```

> **Rust note:** `[HourStat; 24]: Default` works because `HourStat: Default` and std implements `Default` for arrays up to length 32. Same reason `[Vec<f64>; 24] = Default::default()` already works in `hourly.rs`.

- [ ] **Step 2: Add the tests**

Add to the `tests` module:

```rust
    #[test]
    fn per_hour_stats_percentiles_across_samples() {
        // Three separate days, each one turn before a 14:00-UTC (16:00 local)
        // anchor at util 1.0. Day gaps exceed the 4.5h window, so each window's
        // burn is that day's single turn → implied caps 100/200/300 in bin 16.
        let turns = vec![
            turn(utc(2026, 5, 18, 13, 0), 100),
            turn(utc(2026, 5, 19, 13, 0), 200),
            turn(utc(2026, 5, 20, 13, 0), 300),
        ];
        let log = vec![
            sample(utc(2026, 5, 18, 14, 0), 1.0),
            sample(utc(2026, 5, 19, 14, 0), 1.0),
            sample(utc(2026, 5, 20, 14, 0), 1.0),
        ];
        let stats = per_hour_stats(&log, &turns, WindowKind::FiveHour);
        let s = &stats[16];
        assert_eq!(s.n, 3);
        assert_eq!(s.median, Some(200.0));
        assert_eq!(s.p25, Some(150.0));
        assert_eq!(s.p75, Some(250.0));
    }

    #[test]
    fn per_hour_stats_empty_bins_are_default() {
        let stats = per_hour_stats(&[], &[], WindowKind::FiveHour);
        for s in &stats {
            assert!(s.median.is_none());
            assert!(s.p25.is_none());
            assert!(s.p75.is_none());
            assert_eq!(s.n, 0);
        }
    }

    #[test]
    fn per_hour_stats_median_agrees_with_hourly_per_hour_medians() {
        let turns = vec![
            turn(utc(2026, 5, 18, 13, 0), 100),
            turn(utc(2026, 5, 19, 13, 0), 300),
        ];
        let log = vec![
            sample(utc(2026, 5, 18, 14, 0), 1.0),
            sample(utc(2026, 5, 19, 14, 0), 1.0),
        ];
        let stats = per_hour_stats(&log, &turns, WindowKind::FiveHour);
        let raw = crate::calibration::hourly::per_hour_medians(&log, &turns, WindowKind::FiveHour);
        assert_eq!(stats[16].median, raw[16]);
    }
```

- [ ] **Step 3: Run the tests to verify they fail**

Run: `cargo test --lib calibration::history`
Expected: the three `per_hour_stats_*` tests panic with `not implemented`.

- [ ] **Step 4: Implement `per_hour_stats`**

Replace the stub body:

```rust
/// Median / p25 / p75 / count of implied caps per local hour-of-day bin.
pub fn per_hour_stats(
    log: &[CalibrationSample],
    turns: &[Turn],
    kind: WindowKind,
) -> [HourStat; 24] {
    let tz: Tz = config::LOCAL_TZ
        .parse()
        .expect("LOCAL_TZ must be a valid IANA name");
    let mut buckets: [Vec<f64>; 24] = Default::default();
    for (ts, cap) in qualifying_implied(log, turns, kind) {
        let h = ts.with_timezone(&tz).hour() as usize;
        buckets[h].push(cap);
    }
    let mut out: [HourStat; 24] = Default::default();
    for (h, vals) in buckets.iter_mut().enumerate() {
        out[h] = HourStat {
            median: median(vals),
            p25: percentile(vals, 0.25),
            p75: percentile(vals, 0.75),
            n: vals.len(),
        };
    }
    out
}
```

> **Rust note:** read `vals.len()` for `n` **before** the percentile calls if you prefer, but `len()` is unaffected by the in-place sorts, so reading it last is fine. Each of `median`/`percentile` re-sorts the same buffer; that's a few extra sorts on tiny vectors — negligible.

- [ ] **Step 5: Run the full module tests to verify they pass**

Run: `cargo test --lib calibration::history`
Expected: PASS (all median/percentile/implied/per_hour_stats tests).

- [ ] **Step 6: Commit**

```bash
git add src/calibration/history.rs
git commit -m "feat(stage-8): per_hour_stats (median/IQR/count per hour bin)"
```

---

## Task 5: Carry the calibration log on `AppSnapshot`

The dashboard needs the full log. The poll thread already reads it — stash it on the snapshot behind an `Arc`.

**Files:**
- Modify: `src/shared/snapshot.rs:1-22`
- Modify: `src/tray/poller.rs:209-260` (the helper) and `:133`, `:160-169` (call site + literal)

- [ ] **Step 1: Add the field to `AppSnapshot`**

In `src/shared/snapshot.rs`, add the import near the other `use` lines (after `use crate::data::parser::Turn;`):

```rust
use crate::log::calibration::CalibrationSample;
```

Then add the field to the `AppSnapshot` struct (after `pub turns: Arc<Vec<Turn>>,`):

```rust
#[derive(Debug, Clone, Default)]
pub struct AppSnapshot {
    pub turns: Arc<Vec<Turn>>,
    pub log: Arc<Vec<CalibrationSample>>,
    pub caps: DerivedCaps,
    pub hourly_5h: [f64; 24],
    pub hourly_week: [f64; 24],
    pub live_util: LiveUtil,
    pub last_sample: Option<(UsageSnapshot, DateTime<Utc>)>,
    pub last_status: LastStatus,
    pub kpis: DashboardKpis,
}
```

> **Rust note:** `Arc<Vec<CalibrationSample>>` is `Default` (empty), `Clone` (cheap — bumps a refcount), and `Debug` (since `CalibrationSample` derives `Debug`). The per-frame `snap.clone()` in the dashboard stays cheap because the log lives behind the `Arc`.

- [ ] **Step 2: Make the poller helper return the log Arc**

In `src/tray/poller.rs`, change the signature of `compute_calibration_with_turns` (currently returns a 2-tuple) to a 3-tuple:

```rust
fn compute_calibration_with_turns() -> (
    PollCalibration,
    Arc<Vec<Turn>>,
    Arc<Vec<crate::log::calibration::CalibrationSample>>,
) {
```

Update the two early-return paths inside it:

```rust
    let turns = match cache::refresh() {
        Ok(t) => t,
        Err(e) => {
            tracing::warn!(error = %e, "cache::refresh failed; skipping calibration this tick");
            return (
                PollCalibration::default(),
                Arc::new(Vec::new()),
                Arc::new(Vec::new()),
            );
        }
    };
    let turns_arc = Arc::new(turns);
    let log = match log_calib::read_all_default() {
        Ok(l) => l,
        Err(e) => {
            tracing::warn!(error = %e, "calibration log read failed; skipping calibration this tick");
            return (PollCalibration::default(), turns_arc, Arc::new(Vec::new()));
        }
    };
```

And the success return (the final expression of the function), wrap `log` in an `Arc` *after* it has been borrowed by `derive_caps` / `hour_of_day_cap_series`:

```rust
    (
        PollCalibration {
            caps,
            live,
            hourly_5h,
            hourly_week,
        },
        turns_arc,
        Arc::new(log),
    )
```

- [ ] **Step 3: Update the call site + snapshot literal**

In `polling_loop`, change the destructuring (was `let (calib, turns_arc) = ...`):

```rust
        let (calib, turns_arc, log_arc) = compute_calibration_with_turns();
```

Then add `log` to the `AppSnapshot` literal (after `turns: turns_arc,`):

```rust
        let snapshot = AppSnapshot {
            turns: turns_arc,
            log: log_arc,
            caps: calib.caps,
            hourly_5h: calib.hourly_5h,
            hourly_week: calib.hourly_week,
            live_util: calib.live,
            last_sample: last_sample.clone(),
            last_status: last_status.clone(),
            kpis,
        };
```

> **Note:** the Stage-7 sync path below the literal still does its own `read_all_default()` — leave it; it's harmless and keeps the upload path independent. (A future cleanup could feed `log_arc` to it, but that's out of scope here.)

- [ ] **Step 4: Build + run existing tests**

Run: `cargo build`
Expected: compiles clean (no other `AppSnapshot { .. }` literals exist; any that did would now error and must add `log:`).

Run: `cargo test --lib`
Expected: PASS — existing tests unaffected (the new field defaults).

- [ ] **Step 5: Commit**

```bash
git add src/shared/snapshot.rs src/tray/poller.rs
git commit -m "feat(stage-8): carry calibration log on AppSnapshot"
```

---

## Task 6: `dashboard/calibration_tab.rs` — `CalibData` + render

The four plots. UI code is verified by building + running (no unit test — pure functions were tested in Tasks 1–4).

**Files:**
- Create: `src/dashboard/calibration_tab.rs`
- Modify: `src/dashboard/mod.rs`

- [ ] **Step 1: Register the module**

In `src/dashboard/mod.rs`, add `pub mod calibration_tab;` alongside the other `pub mod` declarations (keep alphabetical with the existing ones — e.g. right after `pub mod bands;` / before `pub mod chart_5h;`, wherever it fits the existing ordering).

- [ ] **Step 2: Write the module**

Create `src/dashboard/calibration_tab.rs`:

```rust
//! Calibration tab: how the app's caps are derived from the calibration log.
//! Four plots — implied 5h/weekly cap over time (hour-banded scatter) and
//! hour-of-day cap bins (median line + IQR band + fitted curve) for 5h/weekly.
//! Always account-wide: this tab ignores the global filter bar.

use crate::calibration::history::{median, HourStat, ImpliedPoint};
use crate::shared::snapshot::AppSnapshot;
use egui::{Color32, RichText, Stroke, Ui};
use egui_plot::{HLine, Legend, Line, LineStyle, Plot, PlotPoints, PlotUi, Points, Polygon};
use std::sync::Arc;

/// Derived chart inputs, memoized by the dashboard. Vecs sit behind `Arc` so
/// the per-frame clone of the memo is cheap (matches the `AppSnapshot` pattern).
#[derive(Clone)]
pub struct CalibData {
    pub implied_5h: Arc<Vec<ImpliedPoint>>,
    pub implied_week: Arc<Vec<ImpliedPoint>>,
    pub stats_5h: [HourStat; 24],
    pub stats_week: [HourStat; 24],
}

// Hour-band scatter colors.
const C_NIGHT: Color32 = Color32::from_rgb(120, 110, 220); // 0–6  indigo
const C_MORNING: Color32 = Color32::from_rgb(60, 190, 180); // 6–12 teal
const C_AFTERNOON: Color32 = Color32::from_rgb(240, 180, 70); // 12–18 amber
const C_EVENING: Color32 = Color32::from_rgb(220, 90, 180); // 18–24 magenta
// Hour-of-day chart colors.
const C_MEDIAN: Color32 = Color32::from_rgb(79, 140, 255); // blue (matches chart_5h)
const C_FITTED: Color32 = Color32::from_rgb(255, 165, 79); // orange
const C_MEDIAN_HLINE: Color32 = Color32::from_rgb(120, 120, 120);
// Soft blue IQR fill at low opacity (premultiplied: rgb already * alpha/255).
const C_IQR: Color32 = Color32::from_rgba_premultiplied(20, 35, 64, 90);

const M: f64 = 1_000_000.0; // tokens → millions

const UNCALIBRATED: &str = "(uncalibrated — no ≥95% anchors observed yet)";

pub fn render(ui: &mut Ui, snap: &AppSnapshot, calib: &CalibData) {
    egui::ScrollArea::vertical().show(ui, |ui| {
        ui.add_space(8.0);
        ui.label(RichText::new("Implied 5h cap over time").strong());
        scatter_over_time(ui, "calib_implied_5h", &calib.implied_5h);
        ui.add_space(16.0);
        ui.separator();

        ui.add_space(8.0);
        ui.label(RichText::new("Implied weekly cap over time").strong());
        scatter_over_time(ui, "calib_implied_week", &calib.implied_week);
        ui.add_space(16.0);
        ui.separator();

        ui.add_space(8.0);
        ui.label(RichText::new("Hour-of-day cap — 5h").strong());
        hour_of_day(ui, "calib_hod_5h", &calib.stats_5h, &snap.hourly_5h);
        ui.add_space(16.0);
        ui.separator();

        ui.add_space(8.0);
        ui.label(RichText::new("Hour-of-day cap — weekly").strong());
        hour_of_day(ui, "calib_hod_week", &calib.stats_week, &snap.hourly_week);
        ui.add_space(8.0);
    });
}

/// Scatter of implied cap (M tokens) vs time, points split into 4 hour bands
/// with a legend, plus a dashed line at the median implied cap.
fn scatter_over_time(ui: &mut Ui, id: &str, points: &[ImpliedPoint]) {
    if points.is_empty() {
        ui.label(RichText::new(UNCALIBRATED).color(Color32::from_rgb(220, 200, 120)));
        return;
    }

    // (color, legend name, points) for each of the four bands.
    let mut bands: [(Color32, &str, Vec<[f64; 2]>); 4] = [
        (C_NIGHT, "night 0–6", Vec::new()),
        (C_MORNING, "morning 6–12", Vec::new()),
        (C_AFTERNOON, "afternoon 12–18", Vec::new()),
        (C_EVENING, "evening 18–24", Vec::new()),
    ];
    for p in points {
        let idx = match p.local_hour {
            0..=5 => 0,
            6..=11 => 1,
            12..=17 => 2,
            _ => 3,
        };
        bands[idx].2.push([p.ts.timestamp() as f64, p.cap / M]);
    }

    let mut caps_m: Vec<f64> = points.iter().map(|p| p.cap / M).collect();
    let median_m = median(&mut caps_m);

    Plot::new(id)
        .height(240.0)
        .show_x(true)
        .show_y(true)
        .y_axis_label("M tokens")
        .legend(Legend::default())
        .x_axis_formatter(
            |mark: egui_plot::GridMark, _range: &std::ops::RangeInclusive<f64>| {
                crate::dashboard::axis::format_x_tick(mark.value)
            },
        )
        .show(ui, |plot_ui| {
            for (color, name, pts) in &bands {
                if pts.is_empty() {
                    continue;
                }
                plot_ui.points(
                    Points::new(PlotPoints::from(pts.clone()))
                        .color(*color)
                        .radius(3.0)
                        .name(*name),
                );
            }
            if let Some(m) = median_m {
                plot_ui.hline(
                    HLine::new(m)
                        .color(C_MEDIAN_HLINE)
                        .style(LineStyle::dashed_dense())
                        .name("median cap"),
                );
            }
        });
}

/// Hour-of-day chart: IQR band (p25–p75) + median line with count-scaled
/// markers + the fitted (smoothed/interpolated) curve.
fn hour_of_day(ui: &mut Ui, id: &str, stats: &[HourStat; 24], fitted: &[f64; 24]) {
    if !stats.iter().any(|s| s.median.is_some()) {
        ui.label(RichText::new(UNCALIBRATED).color(Color32::from_rgb(220, 200, 120)));
        return;
    }

    Plot::new(id)
        .height(240.0)
        .show_x(true)
        .show_y(true)
        .x_axis_label("hour of day (local)")
        .y_axis_label("M tokens")
        .legend(Legend::default())
        .show(ui, |plot_ui| {
            // IQR band: one filled polygon per contiguous run of populated hours.
            let mut run: Vec<(f64, f64, f64)> = Vec::new(); // (hour, p25_M, p75_M)
            for (h, s) in stats.iter().enumerate() {
                match (s.p25, s.p75) {
                    (Some(lo), Some(hi)) => run.push((h as f64, lo / M, hi / M)),
                    _ => {
                        draw_iqr_run(plot_ui, &run);
                        run.clear();
                    }
                }
            }
            draw_iqr_run(plot_ui, &run);

            // Median line through populated hours.
            let med_line: Vec<[f64; 2]> = stats
                .iter()
                .enumerate()
                .filter_map(|(h, s)| s.median.map(|m| [h as f64, m / M]))
                .collect();
            if med_line.len() >= 2 {
                plot_ui.line(
                    Line::new(PlotPoints::from(med_line))
                        .color(C_MEDIAN)
                        .name("median"),
                );
            }

            // Count-scaled markers on the median.
            for (h, s) in stats.iter().enumerate() {
                if let Some(m) = s.median {
                    let radius = (2.0 + s.n as f64).min(12.0) as f32;
                    plot_ui.points(
                        Points::new(PlotPoints::from(vec![[h as f64, m / M]]))
                            .color(C_MEDIAN)
                            .radius(radius),
                    );
                }
            }

            // Fitted curve (dense, 24 hours) — only if it carries signal.
            if fitted.iter().any(|&v| v > 0.0) {
                let curve: Vec<[f64; 2]> =
                    (0..24).map(|h| [h as f64, fitted[h] / M]).collect();
                plot_ui.line(
                    Line::new(PlotPoints::from(curve))
                        .color(C_FITTED)
                        .style(LineStyle::dotted_dense())
                        .name("fitted"),
                );
            }
        });
}

/// Draw one IQR polygon: p25 left→right, then p75 right→left, closing the band.
fn draw_iqr_run(plot_ui: &mut PlotUi, run: &[(f64, f64, f64)]) {
    if run.len() < 2 {
        return;
    }
    let mut poly: Vec<[f64; 2]> = Vec::with_capacity(run.len() * 2);
    for &(h, lo, _hi) in run.iter() {
        poly.push([h, lo]);
    }
    for &(h, _lo, hi) in run.iter().rev() {
        poly.push([h, hi]);
    }
    plot_ui.polygon(
        Polygon::new(PlotPoints::from(poly))
            .fill_color(C_IQR)
            .stroke(Stroke::NONE),
    );
}
```

> **egui_plot 0.29 notes:** `Points`, `Polygon`, `Line`, `HLine`, `Legend`, `LineStyle` are all in `egui_plot`. `PlotUi` has no lifetime parameter in 0.29 (if a future bump complains, write `&mut PlotUi<'_>`). `LineStyle::dotted_dense()` and `dashed_dense()` both exist (the latter is used in `chart_5h.rs`). The `x_axis_formatter`/`y_axis_label`/`legend`/`height` builders mirror `chart_5h.rs`.

- [ ] **Step 3: Build (clippy-clean)**

Run: `cargo clippy --lib -- -D warnings`
Expected: compiles with no warnings. (`render`/`CalibData` are not yet referenced — Rust won't warn on a `pub` item, so this is clean. They get wired in Task 7.)

- [ ] **Step 4: Commit**

```bash
git add src/dashboard/calibration_tab.rs src/dashboard/mod.rs
git commit -m "feat(stage-8): calibration tab render (4 charts)"
```

---

## Task 7: Wire the Calibration tab into `DashboardApp`

Add the third tab, the memoized `CalibData`, the tab-strip entry, and the central-panel arm.

**Files:**
- Modify: `src/dashboard/app.rs`

- [ ] **Step 1: Add imports + `Tab` variant + `CalibSig`**

In `src/dashboard/app.rs`, add to the `use` block near the top:

```rust
use crate::calibration::history;
use crate::calibration::WindowKind;
use crate::dashboard::calibration_tab::CalibData;
```

Add the `Calibration` variant to the `Tab` enum:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tab {
    Charts,
    Sessions,
    Calibration,
}
```

Add the calib memo signature next to `ViewSig`:

```rust
/// Cheap signature for the calib-data memo. Equal signature ⇒ reuse cache.
#[derive(Debug, Clone, PartialEq)]
struct CalibSig {
    n_log: usize,
    n_turns: usize,
}
```

- [ ] **Step 2: Add the cache field + initialize it**

Add the field to `DashboardApp` (after `cached_view: Option<(ViewSig, AppSnapshot)>,`):

```rust
    cached_view: Option<(ViewSig, AppSnapshot)>,
    cached_calib: Option<(CalibSig, CalibData)>,
}
```

And in `DashboardApp::new`, after `cached_view: None,`:

```rust
            cached_view: None,
            cached_calib: None,
        }
    }
```

- [ ] **Step 3: Add the `calib_data` memo method**

Add this method to `impl DashboardApp` (right after `filtered_view`):

```rust
    /// Build (or reuse) the Calibration tab's derived series. Always uses the
    /// UNFILTERED snapshot — calibration is account-wide. Memoized on the log +
    /// turn lengths (both append-only, so length change ⇒ new data).
    fn calib_data(&mut self, snap: &AppSnapshot) -> CalibData {
        let sig = CalibSig {
            n_log: snap.log.len(),
            n_turns: snap.turns.len(),
        };
        if let Some((cached_sig, data)) = &self.cached_calib {
            if *cached_sig == sig {
                return data.clone();
            }
        }
        let data = CalibData {
            implied_5h: Arc::new(history::implied_cap_series(
                &snap.log,
                &snap.turns,
                WindowKind::FiveHour,
            )),
            implied_week: Arc::new(history::implied_cap_series(
                &snap.log,
                &snap.turns,
                WindowKind::Weekly,
            )),
            stats_5h: history::per_hour_stats(&snap.log, &snap.turns, WindowKind::FiveHour),
            stats_week: history::per_hour_stats(&snap.log, &snap.turns, WindowKind::Weekly),
        };
        self.cached_calib = Some((sig, data.clone()));
        data
    }
```

> **Borrow note:** `calib_data` takes `&mut self` (to write the cache) and `&snap` (an owned local in `update`). It's called from inside the `CentralPanel` closure's `Tab::Calibration` arm; that's fine because only one match arm runs, so its `&mut self` use doesn't overlap the other arms' `&mut self.range_*` borrows.

- [ ] **Step 4: Add the tab-strip button**

In `update`, find the tab strip inside the filter-bar panel:

```rust
            ui.horizontal(|ui| {
                ui.selectable_value(&mut self.tab, Tab::Charts, "Charts");
                ui.selectable_value(&mut self.tab, Tab::Sessions, "Sessions");
            });
```

Add the third entry:

```rust
            ui.horizontal(|ui| {
                ui.selectable_value(&mut self.tab, Tab::Charts, "Charts");
                ui.selectable_value(&mut self.tab, Tab::Sessions, "Sessions");
                ui.selectable_value(&mut self.tab, Tab::Calibration, "Calibration");
            });
```

- [ ] **Step 5: Add the central-panel arm**

In the `CentralPanel` `match self.tab { … }`, add a third arm after the `Tab::Sessions` arm. Note it passes the **unfiltered** `&snap` (not `&view`):

```rust
            Tab::Sessions => {
                crate::dashboard::sessions_table::render(ui, &view.turns, &mut self.table_controls);
            }
            Tab::Calibration => {
                let calib = self.calib_data(&snap);
                crate::dashboard::calibration_tab::render(ui, &snap, &calib);
            }
```

- [ ] **Step 6: Build + clippy**

Run: `cargo clippy --all-targets -- -D warnings`
Expected: compiles, no warnings.

Run: `cargo test --lib`
Expected: PASS (all existing + new pure-function tests).

- [ ] **Step 7: Commit**

```bash
git add src/dashboard/app.rs
git commit -m "feat(stage-8): wire Calibration tab + memoized calib data"
```

---

## Task 8: Final verification + release

**Files:** none (verification + tag).

- [ ] **Step 1: Format + lint + test gate**

Run each, expecting clean output / all-pass:

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

- [ ] **Step 2: Release build**

Run: `cargo build --release`
Expected: produces `target\release\claude-usage-tray.exe`.

- [ ] **Step 3: Manual smoke test**

Launch the .exe, open the dashboard from the tray, click the **Calibration** tab. Verify:
- Four chart sections render. With a real log that has ≥95% anchors: the over-time scatters show hour-banded points + a dashed median-cap line; the hour-of-day charts show the median line, IQR band, count-scaled markers, and the dashed orange fitted curve. With a fresh/empty log: each section shows the "(uncalibrated …)" notice instead of an empty plot.
- **Filter independence:** on the Charts or Sessions tab, set a project or model filter, then switch to Calibration — its charts must be unchanged (account-wide, filter-inert).
- Switching tabs and closing/reopening the dashboard behaves normally (off-screen parking still works).

- [ ] **Step 4: Update CLAUDE.md + tag**

Update the Stage 8 row / mini-project notes in `CLAUDE.md` to record mini-project 2 (calibration history tab) as shipped, add the spec + plan paths to the "Active design + plans" list, then bump the version and tag per the project's versioning convention (follow the `v0.8.0` mini-project-1 pattern — e.g. a `v0.8.x` minor/patch bump in `Cargo.toml`). Commit the version bump, tag, and push.

```bash
git add CLAUDE.md Cargo.toml Cargo.lock
git commit -m "chore: bump version (Stage 8 mini-project 2 — calibration history tab)"
git tag v0.8.x   # replace with the chosen version
git push && git push --tags
```

---

## Self-review notes (for the executor)

- **Spec coverage:** all 4 charts (Tasks 3,4,6), new Calibration tab (Task 7), `0.95–1.01` util range everywhere (`qualifying_implied`, Task 3), `AppSnapshot.log` plumbing (Task 5), lazy+memoized derived series (Task 7), filter-inert behavior (Task 7 passes `&snap` not `&view`), uncalibrated notice (Task 6), median/IQR/fitted/count markers (Task 6). Fitted curve reuses `snap.hourly_*` (Task 6) — not recomputed.
- **Type consistency:** `median(&mut [f64]) -> Option<f64>`, `percentile(&mut [f64], f64)`, `ImpliedPoint { ts, cap, local_hour }`, `HourStat { median, p25, p75, n }`, `CalibData { implied_5h, implied_week, stats_5h, stats_week }`, `CalibSig { n_log, n_turns }` — names are used identically across Tasks 3,4,6,7.
- **Deferred (spec open questions):** night-hours shading on the hour-of-day charts is omitted (kept simple); revisit only if desired. Y-axis label is "M tokens". Carry-forward-on-skipped-tick: the helper returns empty `Arc`s on error, matching the existing `caps`/`turns` default-on-error behavior.
