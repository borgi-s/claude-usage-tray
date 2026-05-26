# Settings Panel (Stage 8, mini-project 4) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a Settings tab to the egui dashboard that edits four runtime config values (local timezone, weekly reset, poll interval, cost weights), persists them to `~/.claude-usage-tray/settings.toml`, and applies them live to both the poller thread and the dashboard.

**Architecture:** Approach A — a `Settings` struct loaded at startup into `SharedSettings = Arc<RwLock<Settings>>` (mirroring the existing `SharedSnapshot`), shared with the poller and dashboard. Compile-time `const`s in `config.rs` become the *defaults*. Functions that read those consts gain a single `Copy` parameter — `CalParams { tz, reset_weekday, reset_hour }`, `CostWeights`, or `tz: Tz` — read once at the poller/dashboard boundary and threaded down. `FIVE_HOUR_WINDOW_HOURS` and the anchor-util thresholds stay consts (not settings).

**Tech Stack:** Rust, serde + `toml` (new dep), chrono/chrono-tz, eframe/egui 0.29, std `Arc<RwLock>`.

**Reference:** Spec at `docs/superpowers/specs/2026-05-26-stage-8-settings-panel-design.md`.

**Conventions (from CLAUDE.md):**
- Commit style: conventional (`feat:`, `refactor:`, `test:`, `chore:`). No `Co-Authored-By` / "Generated with" lines.
- `cargo fmt` + `cargo clippy --all-targets -- -D warnings` clean before the release tag.
- egui 0.29: use `DragValue::range(..)` (not `clamp_range`), `ComboBox::from_id_salt` (not `from_id_source`).
- Version bump must `git add Cargo.toml Cargo.lock` together.

---

## Notes for the implementer

- **Run all commands from the repo root** `C:\Users\borgi\Documents\claude-usage-tray`. Shell is PowerShell; chain with `;` and guard with `if ($?) { ... }`.
- **`cargo test <name>`** runs tests matching a substring. **`cargo test`** runs all.
- This is a Windows GUI binary (`#![windows_subsystem = "windows"]`). The dashboard/tray cannot be unit-tested headless; those tasks verify via `cargo build` + `cargo clippy` + existing tests still green, with a manual-test checklist at the end.
- Several tasks change a function signature and then must fix every caller in the same task, or the build breaks. Each task lists the call sites to update.

---

## Task 1: Add the `toml` dependency

**Files:**
- Modify: `Cargo.toml`

- [ ] **Step 1: Add the dependency**

In `Cargo.toml`, under `[dependencies]`, add after the `dotenvy` line:

```toml
toml = "0.8"
```

- [ ] **Step 2: Build to fetch + lock it**

Run: `cargo build`
Expected: compiles; `Cargo.lock` now contains a `toml` entry.

- [ ] **Step 3: Commit**

```powershell
git add Cargo.toml Cargo.lock; if ($?) { git commit -m "chore: add toml dependency for settings persistence" }
```

---

## Task 2: `Settings` types, defaults, bundles, and validation

**Files:**
- Create: `src/settings.rs`
- Modify: `src/lib.rs` (register the module)

The `Default` impls reference the existing `config.rs` consts so there is exactly one source for each default value.

- [ ] **Step 1: Write `src/settings.rs` with the types and a failing test**

Create `src/settings.rs`:

```rust
//! Runtime, user-editable configuration. Persisted to
//! `~/.claude-usage-tray/settings.toml`. Defaults mirror the compile-time
//! consts in `crate::config`, so an absent file behaves exactly like the
//! hardcoded build.

use crate::config;
use chrono::Weekday;
use chrono_tz::Tz;
use serde::{Deserialize, Serialize};

/// The four cost-weight coefficients for the spend / cost-weighted view.
/// `Copy` so it threads cheaply into hot loops. Display-only — never used for
/// cap calibration.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CostWeights {
    pub input: f64,
    pub cache_creation: f64,
    pub cache_read: f64,
    pub output: f64,
}

impl Default for CostWeights {
    fn default() -> Self {
        Self {
            input: config::COST_WEIGHT_INPUT,
            cache_creation: config::COST_WEIGHT_CACHE_CREATION,
            cache_read: config::COST_WEIGHT_CACHE_READ,
            output: config::COST_WEIGHT_OUTPUT,
        }
    }
}

/// Parsed bundle for calibration math: the local zone plus the weekly-reset
/// anchor. `Copy` so deep functions (some called per-turn) take it by value.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CalParams {
    pub tz: Tz,
    pub reset_weekday: Weekday,
    pub reset_hour: u32,
}

impl Default for CalParams {
    fn default() -> Self {
        Self {
            tz: config::LOCAL_TZ
                .parse()
                .expect("config::LOCAL_TZ must be a valid IANA name"),
            reset_weekday: config::WEEKLY_RESET_WEEKDAY,
            reset_hour: config::WEEKLY_RESET_HOUR_LOCAL,
        }
    }
}

/// All user-editable settings. `#[serde(default)]` fills any field missing from
/// the TOML file from `Default`, so partial/old files still load.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub local_tz: String,
    pub weekly_reset_weekday: Weekday,
    pub weekly_reset_hour: u32,
    pub poll_interval_secs: u64,
    pub cost_weights: CostWeights,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            local_tz: config::LOCAL_TZ.to_string(),
            weekly_reset_weekday: config::WEEKLY_RESET_WEEKDAY,
            weekly_reset_hour: config::WEEKLY_RESET_HOUR_LOCAL,
            poll_interval_secs: 120,
            cost_weights: CostWeights::default(),
        }
    }
}

impl Settings {
    /// Parse `local_tz`, falling back to the default zone if somehow invalid
    /// (defense in depth; the UI combo only offers valid zones).
    pub fn tz(&self) -> Tz {
        self.local_tz
            .parse()
            .unwrap_or_else(|_| CalParams::default().tz)
    }

    /// Bundle the calibration-relevant fields.
    pub fn cal_params(&self) -> CalParams {
        CalParams {
            tz: self.tz(),
            reset_weekday: self.weekly_reset_weekday,
            reset_hour: self.weekly_reset_hour,
        }
    }
}

/// The allowed poll intervals (seconds). Mirrors the CLI `--interval` choices;
/// constrained to stay above the ~1 req/min endpoint rate limit.
pub const POLL_INTERVAL_CHOICES: [u64; 3] = [60, 120, 300];

/// Validate a settings struct. `Ok(())` if usable; `Err(message)` otherwise.
/// Used by the file-load path and the UI's Save gate.
pub fn validate(s: &Settings) -> Result<(), String> {
    if s.local_tz.parse::<Tz>().is_err() {
        return Err(format!("invalid timezone: '{}'", s.local_tz));
    }
    if s.weekly_reset_hour > 23 {
        return Err(format!(
            "weekly reset hour must be 0..=23, got {}",
            s.weekly_reset_hour
        ));
    }
    if !POLL_INTERVAL_CHOICES.contains(&s.poll_interval_secs) {
        return Err(format!(
            "poll interval must be one of {:?}, got {}",
            POLL_INTERVAL_CHOICES, s.poll_interval_secs
        ));
    }
    let w = &s.cost_weights;
    for (name, v) in [
        ("input", w.input),
        ("cache_creation", w.cache_creation),
        ("cache_read", w.cache_read),
        ("output", w.output),
    ] {
        if !v.is_finite() || v < 0.0 {
            return Err(format!("cost weight '{name}' must be a finite value >= 0.0"));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_settings_match_config_consts() {
        let s = Settings::default();
        assert_eq!(s.local_tz, config::LOCAL_TZ);
        assert_eq!(s.weekly_reset_weekday, config::WEEKLY_RESET_WEEKDAY);
        assert_eq!(s.weekly_reset_hour, config::WEEKLY_RESET_HOUR_LOCAL);
        assert_eq!(s.poll_interval_secs, 120);
        assert_eq!(s.cost_weights.input, config::COST_WEIGHT_INPUT);
        assert_eq!(s.cost_weights.cache_creation, config::COST_WEIGHT_CACHE_CREATION);
        assert_eq!(s.cost_weights.cache_read, config::COST_WEIGHT_CACHE_READ);
        assert_eq!(s.cost_weights.output, config::COST_WEIGHT_OUTPUT);
    }

    #[test]
    fn validate_accepts_defaults() {
        assert!(validate(&Settings::default()).is_ok());
    }

    #[test]
    fn validate_rejects_bad_tz_hour_interval_and_weights() {
        let mut s = Settings::default();
        s.local_tz = "Not/AZone".into();
        assert!(validate(&s).is_err());

        let mut s = Settings::default();
        s.weekly_reset_hour = 24;
        assert!(validate(&s).is_err());

        let mut s = Settings::default();
        s.poll_interval_secs = 90;
        assert!(validate(&s).is_err());

        let mut s = Settings::default();
        s.cost_weights.output = -1.0;
        assert!(validate(&s).is_err());
    }

    #[test]
    fn cal_params_uses_parsed_tz() {
        let mut s = Settings::default();
        s.local_tz = "America/New_York".into();
        assert_eq!(s.cal_params().tz, chrono_tz::America::New_York);
    }
}
```

- [ ] **Step 2: Register the module**

In `src/lib.rs`, add `pub mod settings;` alongside the other `pub mod` lines (keep alphabetical-ish ordering consistent with the file).

- [ ] **Step 3: Run the tests (expect compile, then pass)**

Run: `cargo test settings::tests -v`
Expected: 4 tests pass.

- [ ] **Step 4: Commit**

```powershell
git add src/settings.rs src/lib.rs; if ($?) { git commit -m "feat(settings): Settings/CostWeights/CalParams types + validate" }
```

---

## Task 3: Persistence — `settings.toml` load/save

**Files:**
- Modify: `src/paths.rs` (add `settings_path()`)
- Modify: `src/settings.rs` (add `load`/`save` + testable `_from`/`_to` cores + tests)

- [ ] **Step 1: Add the path helper**

In `src/paths.rs`, after `state_path()`:

```rust
/// Returns ~/.claude-usage-tray/settings.toml. Does NOT create the file.
pub fn settings_path() -> Result<PathBuf> {
    Ok(app_dir()?.join("settings.toml"))
}
```

- [ ] **Step 2: Write failing persistence tests**

Append to the `tests` module in `src/settings.rs`:

```rust
    use std::path::Path;

    #[test]
    fn load_from_missing_file_returns_default() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("nope.toml");
        assert_eq!(load_from(&p), Settings::default());
    }

    #[test]
    fn load_from_corrupt_file_returns_default() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("bad.toml");
        std::fs::write(&p, "this is not = valid = toml {{{").unwrap();
        assert_eq!(load_from(&p), Settings::default());
    }

    #[test]
    fn save_to_then_load_from_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("settings.toml");
        let mut s = Settings::default();
        s.local_tz = "America/New_York".into();
        s.poll_interval_secs = 300;
        s.cost_weights.output = 9.0;
        save_to(&p, &s).unwrap();
        assert_eq!(load_from(&p), s);
    }

    #[test]
    fn load_from_partial_toml_fills_missing_with_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("partial.toml");
        // Only poll_interval_secs present; everything else must default.
        std::fs::write(&p, "poll_interval_secs = 60\n").unwrap();
        let loaded = load_from(&p);
        assert_eq!(loaded.poll_interval_secs, 60);
        assert_eq!(loaded.local_tz, config::LOCAL_TZ);
        assert_eq!(loaded.cost_weights, CostWeights::default());
    }

    fn _assert_path_arg(_: &Path) {} // keeps `Path` import used if tests trimmed
```

- [ ] **Step 3: Run to verify they fail to compile (functions missing)**

Run: `cargo test settings::tests::load_from_missing_file_returns_default`
Expected: FAIL — `cannot find function load_from`.

- [ ] **Step 4: Implement load/save**

In `src/settings.rs`, add after the `validate` function (before `#[cfg(test)]`):

```rust
use std::path::Path;

/// Load settings from the default path. Never fails: any error (missing,
/// unreadable, malformed, or failing validation) logs a warning and yields
/// defaults.
pub fn load() -> Settings {
    match crate::paths::settings_path() {
        Ok(p) => load_from(&p),
        Err(e) => {
            tracing::warn!(error = %e, "could not resolve settings path; using defaults");
            Settings::default()
        }
    }
}

/// Testable core of `load`.
pub fn load_from(path: &Path) -> Settings {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(_) => return Settings::default(), // missing file is normal
    };
    let parsed: Settings = match toml::from_str(&text) {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!(error = %e, "settings.toml is malformed; using defaults");
            return Settings::default();
        }
    };
    if let Err(msg) = validate(&parsed) {
        tracing::warn!(reason = %msg, "settings.toml failed validation; using defaults");
        return Settings::default();
    }
    parsed
}

/// Save settings to the default path. Returns the error so the UI can show it.
pub fn save(s: &Settings) -> anyhow::Result<()> {
    let p = crate::paths::settings_path()?;
    save_to(&p, s)
}

/// Testable core of `save`. Writes atomically (temp file + rename).
pub fn save_to(path: &Path, s: &Settings) -> anyhow::Result<()> {
    use anyhow::Context;
    crate::paths::ensure_parent_dir(path)?;
    let text = toml::to_string_pretty(s).context("serializing settings to TOML")?;
    let tmp = path.with_extension("toml.tmp");
    std::fs::write(&tmp, text).with_context(|| format!("writing {}", tmp.display()))?;
    std::fs::rename(&tmp, path)
        .with_context(|| format!("renaming {} -> {}", tmp.display(), path.display()))?;
    Ok(())
}
```

- [ ] **Step 5: Run the persistence tests**

Run: `cargo test settings::tests -v`
Expected: all settings tests pass (8 total).

- [ ] **Step 6: Commit**

```powershell
git add src/settings.rs src/paths.rs; if ($?) { git commit -m "feat(settings): atomic TOML load/save with default fallbacks" }
```

---

## Task 4: `SharedSettings` shared store

**Files:**
- Modify: `src/shared/mod.rs`

- [ ] **Step 1: Add the type + constructor**

In `src/shared/mod.rs`, after the `new_shared_snapshot` function:

```rust
use crate::settings::Settings;

pub type SharedSettings = Arc<RwLock<Settings>>;

/// Build the shared settings store by loading `settings.toml` (defaults on any
/// error). The sole place the file is read at startup.
pub fn new_shared_settings() -> SharedSettings {
    Arc::new(RwLock::new(crate::settings::load()))
}
```

(`Arc` and `RwLock` are already imported at the top of the file.)

- [ ] **Step 2: Build**

Run: `cargo build`
Expected: compiles.

- [ ] **Step 3: Commit**

```powershell
git add src/shared/mod.rs; if ($?) { git commit -m "feat(settings): SharedSettings = Arc<RwLock<Settings>> store" }
```

---

## Task 5: Thread `CostWeights` through the cost path

**Files:**
- Modify: `src/shared/snapshot.rs` (`cost_weighted`, `compute_kpis`, tests)
- Modify: `src/tray/poller.rs` (two `compute_kpis` callers)

`cost_weighted` and `compute_kpis` stop reading the consts and take a `CostWeights`. `series::daily_aggregates` (the other `cost_weighted` caller) is handled in Task 8.

- [ ] **Step 1: Update `cost_weighted` + `compute_kpis` signatures and bodies**

In `src/shared/snapshot.rs`:

Replace the `cost_weighted` function (lines ~41-48):

```rust
use crate::settings::CostWeights;

/// Heuristic cost-weighted token count for a single turn. Used by the
/// dashboard's "total burn" KPI + daily bar chart. NOT used for cap math.
pub fn cost_weighted(turn: &Turn, w: &CostWeights) -> f64 {
    turn.input_tokens as f64 * w.input
        + turn.cache_creation_input_tokens as f64 * w.cache_creation
        + turn.cache_read_input_tokens as f64 * w.cache_read
        + turn.output_tokens as f64 * w.output
}
```

(The `use crate::config;` line at snapshot.rs:39 may now be unused in this file — if `cargo build` warns, remove it. Clippy with `-D warnings` will catch it in Task 14 regardless.)

Replace the `compute_kpis` signature + the `total_cw` line:

```rust
/// Compute all four KPIs from the turns + caps. Called once per poll.
pub fn compute_kpis(turns: &[Turn], caps: &DerivedCaps, w: &CostWeights) -> DashboardKpis {
    let total_cw: f64 = turns.iter().map(|t| cost_weighted(t, w)).sum();
```

(Leave the rest of `compute_kpis` unchanged.)

- [ ] **Step 2: Update the tests in `snapshot.rs`**

In the `tests` module:
- `cost_weighted_applies_each_coefficient`: change the call to
  `cost_weighted(&t, &CostWeights::default())`.
- Every `compute_kpis(&turns, &caps)` / `compute_kpis(&[], &DerivedCaps::default())`
  call: add `, &CostWeights::default()` as the third argument.
- Add `use crate::settings::CostWeights;` to the test module's `use super::*;`
  block (or rely on `super::*` re-export — `CostWeights` is imported at module
  scope in Step 1, so `super::*` already brings it in).

Add a new test proving non-default weights drive output:

```rust
    #[test]
    fn cost_weighted_uses_passed_weights_not_consts() {
        // All weights = 2.0 → cost = 2*(input+cc+cr+output).
        let w = CostWeights { input: 2.0, cache_creation: 2.0, cache_read: 2.0, output: 2.0 };
        let t = turn(10, 10, 10, 10);
        assert!((cost_weighted(&t, &w) - 80.0).abs() < 1e-9);
    }
```

- [ ] **Step 3: Fix the poller callers**

In `src/tray/poller.rs`, the two `compute_kpis` calls (currently
`compute_kpis(&turns_arc, &calib.caps)` at ~line 160). For now pass the default
weights so the build stays green; Task 10 replaces this with the live settings:

```rust
let kpis = compute_kpis(&turns_arc, &calib.caps, &crate::settings::CostWeights::default());
```

- [ ] **Step 4: Build + test**

Run: `cargo test snapshot::tests -v`
Expected: all pass, including the new `cost_weighted_uses_passed_weights_not_consts`.

Run: `cargo build`
Expected: compiles (poller updated).

- [ ] **Step 5: Commit**

```powershell
git add src/shared/snapshot.rs src/tray/poller.rs; if ($?) { git commit -m "refactor(settings): compute_kpis/cost_weighted take CostWeights" }
```

---

## Task 6: Thread `CalParams` through `anchors.rs`

**Files:**
- Modify: `src/calibration/anchors.rs` (5 functions + tests)

Functions changed: `last_weekly_reset`, `weekly_burn_at`, `peak_weekly_burn`,
`global_cap_from_anchors`, `derive_caps`. `five_hour_burn_at` and
`peak_five_hour_burn` are **unchanged** (they use only the 5h-window const).

- [ ] **Step 1: Update the five function signatures + bodies**

In `src/calibration/anchors.rs`:

Add the import near the top (after `use chrono_tz::Tz;`):

```rust
use crate::settings::CalParams;
```

`last_weekly_reset` — replace its signature and the tz/weekday/hour reads:

```rust
pub fn last_weekly_reset(anchor_ts: DateTime<Utc>, cp: CalParams) -> DateTime<Utc> {
    let tz = cp.tz;
    let local = anchor_ts.with_timezone(&tz);

    let target = cp.reset_weekday;
    let days_back = ((local.weekday().num_days_from_monday() as i64)
        - (target.num_days_from_monday() as i64))
        .rem_euclid(7);

    let candidate_date = local.date_naive() - Duration::days(days_back);
    let candidate_naive = candidate_date
        .and_hms_opt(cp.reset_hour, 0, 0)
        .expect("reset hour 0..=23 is valid");
    // ...rest of the function body is unchanged...
```

(Keep everything from `let candidate_local = ...` onward identical.)

`weekly_burn_at` — add `cp` and forward it:

```rust
pub fn weekly_burn_at(turns: &[Turn], anchor_ts: DateTime<Utc>, cp: CalParams) -> u64 {
    let win_start = last_weekly_reset(anchor_ts, cp);
    // ...unchanged filter/sum...
```

`peak_weekly_burn` — add `cp` and forward it:

```rust
pub fn peak_weekly_burn(turns: &[Turn], cp: CalParams) -> u64 {
    // ...unchanged until the loop body...
        let reset = last_weekly_reset(t.ts, cp);
    // ...unchanged...
```

`global_cap_from_anchors` — add `cp`; only the `Weekly` burn call uses it:

```rust
pub fn global_cap_from_anchors(
    log: &[CalibrationSample],
    turns: &[Turn],
    kind: WindowKind,
    cp: CalParams,
) -> (Option<f64>, usize) {
    // ...unchanged until the burn match...
        let burn = match kind {
            WindowKind::FiveHour => five_hour_burn_at(turns, s.ts),
            WindowKind::Weekly => weekly_burn_at(turns, s.ts, cp),
        };
    // ...unchanged...
```

`derive_caps` — add `cp`; forward to both calls:

```rust
pub fn derive_caps(log: &[CalibrationSample], turns: &[Turn], cp: CalParams) -> DerivedCaps {
    let (cap_5h, n5) = global_cap_from_anchors(log, turns, WindowKind::FiveHour, cp);
    let (cap_week, n7) = global_cap_from_anchors(log, turns, WindowKind::Weekly, cp);
    // ...unchanged DerivedCaps { ... }...
```

Remove the now-unused `use crate::config;` only if the file no longer references
`config` (it still uses `config::FIVE_HOUR_WINDOW_HOURS` and the anchor-util
consts — so **keep** the `use crate::config;`).

- [ ] **Step 2: Update existing tests + add a non-default test**

In the `tests` module of `anchors.rs`, add `use crate::settings::CalParams;`.
Then mechanically update every call:
- `last_weekly_reset(anchor)` → `last_weekly_reset(anchor, CalParams::default())`
- `weekly_burn_at(&turns, anchor)` → `weekly_burn_at(&turns, anchor, CalParams::default())`
- `peak_weekly_burn(&turns)` / `peak_weekly_burn(&[])` → add `, CalParams::default()`
- `global_cap_from_anchors(&log, &turns, WindowKind::FiveHour)` → add `, CalParams::default()`

Add this test proving a non-default reset weekday changes the window boundary:

```rust
    #[test]
    fn last_weekly_reset_honors_non_default_weekday() {
        use chrono::Weekday;
        // Anchor: Wed 2026-05-27 12:00 UTC. With a Monday-07:00-local reset,
        // the prior reset is Mon 2026-05-25 07:00 local (CEST = 05:00 UTC).
        let cp = CalParams {
            tz: chrono_tz::Europe::Copenhagen,
            reset_weekday: Weekday::Mon,
            reset_hour: 7,
        };
        let anchor = utc(2026, 5, 27, 12, 0);
        assert_eq!(last_weekly_reset(anchor, cp), utc(2026, 5, 25, 5, 0));
    }
```

- [ ] **Step 3: Run anchors tests**

Run: `cargo test calibration::anchors::tests -v`
Expected: all pass (existing + the new one). The build will still fail in
`history.rs`/`hourly.rs`/`series.rs`/`poller.rs` which call these functions —
that's fixed in Tasks 7-10. To run just this module's tests use the filter above
(it compiles the lib; if the lib doesn't compile yet because of downstream
callers, proceed to Task 7 first and run the combined tests at Step 3 there).

> **Implementer note:** Tasks 6-9 form one compile unit — each changes a callee
> signature that later tasks' callers depend on. Commit per task, but expect the
> *full* `cargo test` to go green only after Task 9. If you prefer a green build
> at each commit, do Steps "update callers" eagerly as noted; otherwise run the
> targeted module tests and accept transient downstream breakage until Task 9.

- [ ] **Step 4: Commit**

```powershell
git add src/calibration/anchors.rs; if ($?) { git commit -m "refactor(settings): anchors.rs weekly math takes CalParams" }
```

---

## Task 7: Thread `CalParams` through `history.rs` + `hourly.rs`

**Files:**
- Modify: `src/calibration/history.rs` (`qualifying_implied`, `implied_cap_series`, `per_hour_stats`, tests)
- Modify: `src/calibration/hourly.rs` (`per_hour_medians`, `hour_of_day_cap_series`, tests)

- [ ] **Step 1: Update `history.rs`**

Add `use crate::settings::CalParams;` near the top.

`qualifying_implied` — add `cp`, forward to `weekly_burn_at`:

```rust
fn qualifying_implied(
    log: &[CalibrationSample],
    turns: &[Turn],
    kind: WindowKind,
    cp: CalParams,
) -> Vec<(DateTime<Utc>, f64)> {
    // ...unchanged until the burn match...
        let burn = match kind {
            WindowKind::FiveHour => five_hour_burn_at(turns, s.ts),
            WindowKind::Weekly => weekly_burn_at(turns, s.ts, cp),
        };
    // ...unchanged...
```

`implied_cap_series` — add `cp`; use `cp.tz` instead of parsing the const; pass
`cp` to `qualifying_implied`:

```rust
pub fn implied_cap_series(
    log: &[CalibrationSample],
    turns: &[Turn],
    kind: WindowKind,
    cp: CalParams,
) -> Vec<ImpliedPoint> {
    let tz = cp.tz;
    let mut out: Vec<ImpliedPoint> = qualifying_implied(log, turns, kind, cp)
        .into_iter()
        // ...unchanged map/sort...
```

`per_hour_stats` — same treatment:

```rust
pub fn per_hour_stats(
    log: &[CalibrationSample],
    turns: &[Turn],
    kind: WindowKind,
    cp: CalParams,
) -> [HourStat; 24] {
    let tz = cp.tz;
    let mut buckets: [Vec<f64>; 24] = Default::default();
    for (ts, cap) in qualifying_implied(log, turns, kind, cp) {
    // ...unchanged...
```

Remove the `use crate::config;` line in `history.rs` **only if** nothing else
uses it — `qualifying_implied` still references `config::MIN_ANCHOR_UTIL` /
`MAX_ANCHOR_UTIL`, so **keep** the import.

- [ ] **Step 2: Update `hourly.rs`**

Add `use crate::settings::CalParams;`.

`per_hour_medians` — add `cp`; use `cp.tz`; forward to `weekly_burn_at`:

```rust
pub fn per_hour_medians(
    log: &[CalibrationSample],
    turns: &[Turn],
    kind: WindowKind,
    cp: CalParams,
) -> [Option<f64>; 24] {
    let tz = cp.tz;
    let mut buckets: [Vec<f64>; 24] = Default::default();

    for s in log {
        // ...unchanged until the burn match...
        let burn = match kind {
            WindowKind::FiveHour => five_hour_burn_at(turns, s.ts),
            WindowKind::Weekly => weekly_burn_at(turns, s.ts, cp),
        };
    // ...unchanged...
```

`hour_of_day_cap_series` — add `cp`, forward:

```rust
pub fn hour_of_day_cap_series(
    log: &[CalibrationSample],
    turns: &[Turn],
    kind: WindowKind,
    cp: CalParams,
) -> [f64; 24] {
    let raw = per_hour_medians(log, turns, kind, cp);
    let smoothed = smooth_rolling_circular(&raw, 3);
    interpolate_empty_circular(&smoothed)
}
```

Keep `use crate::config;` (still used for anchor-util consts).

- [ ] **Step 3: Update tests in both files**

`history.rs` tests: add `use crate::settings::CalParams;`. Append
`, CalParams::default()` to every `implied_cap_series(...)`, `per_hour_stats(...)`
call. In `per_hour_stats_median_agrees_with_hourly_per_hour_medians`, also append
`, CalParams::default()` to the `hourly::per_hour_medians(...)` call.

`hourly.rs` tests: add `use crate::settings::CalParams;`. Append
`, CalParams::default()` to every `per_hour_medians(...)` and
`hour_of_day_cap_series(...)` call.

- [ ] **Step 4: Run both modules' tests**

Run: `cargo test calibration:: -v`
Expected: anchors + history + hourly tests pass. (Build may still fail in
`series.rs`/`dashboard`/`poller` — fixed next. If the lib won't compile, continue
to Task 8 and run the combined suite at Task 9.)

- [ ] **Step 5: Commit**

```powershell
git add src/calibration/history.rs src/calibration/hourly.rs; if ($?) { git commit -m "refactor(settings): history.rs + hourly.rs take CalParams" }
```

---

## Task 8: Thread `tz`/`CostWeights`/`CalParams` through `series.rs` + `filters.rs`

**Files:**
- Modify: `src/dashboard/series.rs` (`cumulative_share_series_weekly`, `daily_aggregates`, tests)
- Modify: `src/dashboard/filters.rs` (`FilterState::apply`, tests)

- [ ] **Step 1: Update `series.rs`**

Add `use crate::settings::{CalParams, CostWeights};` near the top.

`cumulative_share_series_weekly` — add `cp`, forward to `last_weekly_reset`:

```rust
pub fn cumulative_share_series_weekly(
    turns: &[Turn],
    cap: Option<f64>,
    cp: CalParams,
) -> Vec<WindowedTurn> {
    // ...unchanged until the loop...
        let this_reset = last_weekly_reset(t.ts, cp);
    // ...unchanged...
```

`daily_aggregates` — add `w` + `tz`, drop the const parse, pass `w` to
`cost_weighted`:

```rust
pub fn daily_aggregates(turns: &[Turn], w: &CostWeights, tz: Tz) -> Vec<(NaiveDate, f64)> {
    use crate::shared::snapshot::cost_weighted;

    let mut map: BTreeMap<NaiveDate, f64> = BTreeMap::new();
    for t in turns {
        let local_date = t.ts.with_timezone(&tz).date_naive();
        *map.entry(local_date).or_default() += cost_weighted(t, w);
    }
    map.into_iter().collect()
}
```

`cumulative_share_series_5h` is **unchanged** (5h const only).

- [ ] **Step 2: Update `filters.rs` `apply`**

In `src/dashboard/filters.rs`, change `FilterState::apply` to take `tz` and use
it instead of parsing the const (filters.rs:55):

```rust
    pub fn apply(&self, turns: &[Turn], tz: Tz) -> Vec<Turn> {
        // remove: let tz: Tz = crate::config::LOCAL_TZ.parse().expect("LOCAL_TZ");
        // ...rest unchanged, using the passed-in `tz`...
```

Ensure `use chrono_tz::Tz;` is present at the top of `filters.rs` (add if absent).

- [ ] **Step 3: Update tests in both files**

`series.rs` tests (if any call the changed fns): append the new args —
`cumulative_share_series_weekly(&turns, cap, CalParams::default())`,
`daily_aggregates(&turns, &CostWeights::default(), CalParams::default().tz)`.
Add `use crate::settings::{CalParams, CostWeights};` to the test module.

`filters.rs` tests: add `use chrono_tz::Tz;` and a helper
`fn tz() -> Tz { crate::settings::CalParams::default().tz }`; change every
`self_or_state.apply(&turns)` to `.apply(&turns, tz())`.

- [ ] **Step 4: Run**

Run: `cargo test dashboard::series dashboard::filters -v`
Expected: pass (or compiles cleanly if no tests in series).

- [ ] **Step 5: Commit**

```powershell
git add src/dashboard/series.rs src/dashboard/filters.rs; if ($?) { git commit -m "refactor(settings): series + filters take tz/weights/CalParams" }
```

---

## Task 9: Thread `tz`/`CalParams` through axis, bands, and the chart renderers

**Files:**
- Modify: `src/dashboard/axis.rs` (`format_x_tick`)
- Modify: `src/dashboard/bands.rs` (`calendar_bands`)
- Modify: `src/dashboard/chart_5h.rs` (`render`, `hourly_overlay_points`)
- Modify: `src/dashboard/chart_weekly.rs` (`render`)
- Modify: `src/dashboard/chart_daily.rs` (`render`)
- Modify: `src/dashboard/sessions_table.rs` (`render`)

These are render functions (no unit tests); the gate is `cargo build` +
`cargo clippy`. Callers in `app.rs` are fixed in Task 11 — after this task the
lib won't fully build until Task 11; run the targeted check noted in Step 7.

- [ ] **Step 1: `axis.rs`**

```rust
use chrono::{DateTime, Utc};
use chrono_tz::Tz;

pub fn format_x_tick(secs: f64, tz: Tz) -> String {
    match DateTime::<Utc>::from_timestamp(secs as i64, 0) {
        Some(dt) => dt.with_timezone(&tz).format("%b %d").to_string(),
        None => String::new(),
    }
}
```

- [ ] **Step 2: `bands.rs`**

Change `calendar_bands` to take `tz` and drop the const parse:

```rust
pub fn calendar_bands(
    range_start: DateTime<Utc>,
    range_end: DateTime<Utc>,
    tz: Tz,
) -> Vec<(DateTime<Utc>, DateTime<Utc>, BandKind)> {
    if range_end <= range_start {
        return Vec::new();
    }
    // remove: let tz: Tz = config::LOCAL_TZ.parse()...
    // ...rest unchanged, using passed-in `tz`...
```

Remove `use crate::config;` from `bands.rs` if it becomes unused (it likely does
— `bands.rs` only referenced `config` for `LOCAL_TZ`).

- [ ] **Step 3: `chart_5h.rs`**

Change `render` to take `tz: Tz`; pass it into `calendar_bands`, the axis
closure, and `hourly_overlay_points`:

```rust
pub fn render(ui: &mut Ui, snap: &AppSnapshot, range: &mut Range, tz: chrono_tz::Tz) {
    // ...unchanged until calendar_bands...
            for (s, e, _kind) in calendar_bands(x_start, x_end, tz) {
    // ...
            // axis formatter closure:
        .x_axis_formatter(
            move |mark: egui_plot::GridMark, _range: &std::ops::RangeInclusive<f64>| {
                crate::dashboard::axis::format_x_tick(mark.value, tz)
            },
        )
    // ...
            // hourly overlay call:
                let overlay = hourly_overlay_points(x_start, x_end, snap.hourly_5h, cap, tz);
```

Change `hourly_overlay_points` to take `tz` and drop its local parse:

```rust
fn hourly_overlay_points(
    x_start: DateTime<Utc>,
    x_end: DateTime<Utc>,
    hourly: [f64; 24],
    cap: f64,
    tz: chrono_tz::Tz,
) -> Vec<[f64; 2]> {
    use chrono::Timelike;
    // remove: let tz: Tz = crate::config::LOCAL_TZ.parse()...
    // ...rest unchanged...
```

> Note: the axis formatter closure must be `move` so it captures `tz` (a `Copy`
> value). If `chart_weekly.rs`/`chart_daily.rs` use the same closure, apply the
> same `move` + `tz` capture there.

- [ ] **Step 4: `chart_weekly.rs`**

Inspect its body. It calls `cumulative_share_series_weekly` (needs `CalParams`)
and `calendar_bands` + the axis closure (need `tz`). Change `render` to take
`cp: chrono-aware CalParams`:

```rust
pub fn render(ui: &mut Ui, snap: &AppSnapshot, range: &mut Range, cp: crate::settings::CalParams) {
    // series call:
    let series = cumulative_share_series_weekly(&snap.turns, cap_week, cp);
    // bands:
    for (s, e, _kind) in calendar_bands(x_start, x_end, cp.tz) { ... }
    // axis closure: move |mark, _| crate::dashboard::axis::format_x_tick(mark.value, cp.tz)
}
```

(Match the exact variable names already in `chart_weekly.rs`; only the three call
sites change. Add the `move` keyword to the axis closure.)

- [ ] **Step 5: `chart_daily.rs`**

It calls `daily_aggregates` (now needs `&CostWeights` + `tz`) and the axis
closure (`tz`). Change `render`:

```rust
pub fn render(
    ui: &mut Ui,
    snap: &AppSnapshot,
    range: &mut Range,
    w: &crate::settings::CostWeights,
    tz: chrono_tz::Tz,
) {
    // aggregates call:
    let daily = daily_aggregates(&snap.turns, w, tz);
    // axis closure uses tz (move closure)
    // remove the local `let tz: Tz = ...config::LOCAL_TZ...` parse at chart_daily.rs:32
}
```

- [ ] **Step 6: `sessions_table.rs`**

`render` parses tz at sessions_table.rs:64. Change the signature to take `tz`:

```rust
pub fn render(ui: &mut Ui, turns: &[Turn], controls: &mut TableControls, tz: chrono_tz::Tz) {
    // remove the local LOCAL_TZ parse; use the passed-in `tz`
    // ...rest unchanged...
```

- [ ] **Step 7: Targeted compile check**

Run: `cargo build 2>&1 | Select-String -Pattern "dashboard|chart|axis|bands|sessions"`
Expected: the only remaining errors point at `app.rs` call sites (fixed in
Task 11). No errors *inside* axis/bands/chart_*/sessions_table themselves.

- [ ] **Step 8: Commit**

```powershell
git add src/dashboard/axis.rs src/dashboard/bands.rs src/dashboard/chart_5h.rs src/dashboard/chart_weekly.rs src/dashboard/chart_daily.rs src/dashboard/sessions_table.rs; if ($?) { git commit -m "refactor(settings): axis/bands/charts/sessions take tz/CalParams" }
```

---

## Task 10: Wire the poller to `SharedSettings`

**Files:**
- Modify: `src/tray/poller.rs` (`spawn`, `polling_loop`, `compute_calibration_with_turns`)

The poller reads the shared settings once per loop iteration: drives the poll
interval, calibration `CalParams`, and KPI `CostWeights` from it.

- [ ] **Step 1: Add `SharedSettings` to `spawn` + `polling_loop`**

In `src/tray/poller.rs`:

`spawn` — add a `settings: SharedSettings` parameter and forward it. The
`interval_secs` parameter stays (used for the very first iteration before the
loop re-reads settings) — but the loop now recomputes the interval each tick:

```rust
use crate::shared::{SharedSettings, SharedSnapshot};

pub fn spawn(
    creds: Credentials,
    shutdown: Arc<AtomicBool>,
    hwnd: SendHwnd,
    tx: Sender<PollEvent>,
    update_tx: Sender<UpdateEvent>,
    shared: SharedSnapshot,
    settings: SharedSettings,
) -> JoinHandle<()> {
    thread::spawn(move || polling_loop(creds, shutdown, hwnd, tx, update_tx, shared, settings))
}
```

(We drop the `interval_secs`/`interval` parameter entirely; the interval comes
from settings now.)

- [ ] **Step 2: Update `polling_loop`**

Change its signature to match (`settings: SharedSettings`, no `interval`).
Inside, at the **top of each `while` iteration**, read settings once:

```rust
    while !shutdown.load(Ordering::Relaxed) {
        let fetch_at = Instant::now();

        // Read live settings once per tick.
        let settings_snap = settings.read().map(|g| g.clone()).unwrap_or_default();
        let cp = settings_snap.cal_params();
        let weights = settings_snap.cost_weights;
        let interval = Duration::from_secs(settings_snap.poll_interval_secs);

        let (calib, turns_arc, log_arc) = compute_calibration_with_turns(cp);
        // ...
        let kpis = compute_kpis(&turns_arc, &calib.caps, &weights);
        let snapshot = AppSnapshot {
            // ...
            interval_secs: interval.as_secs(),
        };
        // ...
        sleep_interruptible(&shutdown, fetch_at, interval);
    }
```

Also update the pre-loop publish of `interval_secs` (currently uses the captured
`interval`): read it from settings before the loop, e.g.

```rust
    let initial_interval = settings.read().map(|g| g.poll_interval_secs).unwrap_or(120);
    if let Ok(mut g) = shared.write() {
        g.last_status = last_status.clone();
        g.interval_secs = initial_interval;
    }
```

(`Settings` implements `Default`, so `unwrap_or_default()` is valid; `Duration`
import already present.)

- [ ] **Step 3: Update `compute_calibration_with_turns`**

Add `cp: CalParams`; forward to `derive_caps` and `hour_of_day_cap_series`:

```rust
fn compute_calibration_with_turns(
    cp: crate::settings::CalParams,
) -> (
    PollCalibration,
    Arc<Vec<Turn>>,
    Arc<Vec<crate::log::calibration::CalibrationSample>>,
) {
    // ...unchanged until derive_caps...
    let caps = derive_caps(&log, &turns_arc, cp);
    let hourly_5h = hour_of_day_cap_series(&log, &turns_arc, WindowKind::FiveHour, cp);
    let hourly_week = hour_of_day_cap_series(&log, &turns_arc, WindowKind::Weekly, cp);
    let live = live_util_now(&turns_arc, &caps);
    // ...unchanged...
```

The two early-return `PollCalibration::default()` paths are unchanged.

- [ ] **Step 4: Build**

Run: `cargo build`
Expected: `poller.rs` compiles. (Callers in `tray::run` still pass the old arg
list — fixed in Task 12. If `tray/mod.rs` errors, that's expected.)

- [ ] **Step 5: Commit**

```powershell
git add src/tray/poller.rs; if ($?) { git commit -m "feat(settings): poller reads live settings each tick" }
```

---

## Task 11: Wire `DashboardApp` to `SharedSettings`

**Files:**
- Modify: `src/dashboard/app.rs`

- [ ] **Step 1: Add settings to `DashboardApp` + memo signatures**

In `src/dashboard/app.rs`:

Add imports:

```rust
use crate::settings::{CalParams, CostWeights, Settings};
use crate::shared::SharedSettings;
use chrono_tz::Tz;
```

Add fields to `DashboardApp`:

```rust
    settings: SharedSettings,
    settings_draft: Settings,
    settings_save_msg: Option<Result<(), String>>,
```

Extend `ViewSig` so the memo invalidates when tz/weights change (critical for
live-apply):

```rust
struct ViewSig {
    filter: FilterState,
    n_turns: usize,
    last_ts: Option<DateTime<Utc>>,
    tz: Tz,
    weights: CostWeights,
}
```

Extend `CalibSig` so it invalidates when `CalParams` change:

```rust
struct CalibSig {
    n_log: usize,
    n_turns: usize,
    cp: CalParams,
}
```

- [ ] **Step 2: Update `DashboardApp::new`**

```rust
    pub fn new(shared: SharedSnapshot, signals: Arc<DashboardSignals>, settings: SharedSettings) -> Self {
        let settings_draft = settings.read().map(|g| g.clone()).unwrap_or_default();
        Self {
            shared,
            signals,
            settings,
            settings_draft,
            settings_save_msg: None,
            visible: true,
            // ...rest of the existing fields unchanged...
            cached_view: None,
            cached_calib: None,
        }
    }
```

- [ ] **Step 3: Update `filtered_view` + `calib_data`**

`filtered_view` takes `tz` + `weights`:

```rust
    fn filtered_view(&mut self, snap: &AppSnapshot, tz: Tz, weights: CostWeights) -> AppSnapshot {
        let sig = ViewSig {
            filter: self.filters.clone(),
            n_turns: snap.turns.len(),
            last_ts: snap.turns.last().map(|t| t.ts),
            tz,
            weights,
        };
        if let Some((cached_sig, view)) = &self.cached_view {
            if *cached_sig == sig {
                return view.clone();
            }
        }
        let filtered = self.filters.apply(&snap.turns, tz);
        let kpis = compute_kpis(&filtered, &snap.caps, &weights);
        // ...unchanged build of `view`...
    }
```

`calib_data` takes `cp`:

```rust
    fn calib_data(&mut self, snap: &AppSnapshot, cp: CalParams) -> CalibData {
        let sig = CalibSig {
            n_log: snap.log.len(),
            n_turns: snap.turns.len(),
            cp,
        };
        // ...unchanged memo check...
        let data = CalibData {
            implied_5h: Arc::new(history::implied_cap_series(&snap.log, &snap.turns, WindowKind::FiveHour, cp)),
            implied_week: Arc::new(history::implied_cap_series(&snap.log, &snap.turns, WindowKind::Weekly, cp)),
            stats_5h: history::per_hour_stats(&snap.log, &snap.turns, WindowKind::FiveHour, cp),
            stats_week: history::per_hour_stats(&snap.log, &snap.turns, WindowKind::Weekly, cp),
        };
        // ...unchanged cache store + return...
    }
```

- [ ] **Step 4: Read settings once per frame in `update` + thread the values**

In `eframe::App::update`, right after the visible-state guard where `snap` is
read (the `let snap = self.shared.read()...` block, ~line 183), add:

```rust
        let settings_now = self.settings.read().map(|g| g.clone()).unwrap_or_default();
        let cp = settings_now.cal_params();
        let tz = cp.tz;
        let weights = settings_now.cost_weights;
        let view = self.filtered_view(&snap, tz, weights);
```

(Replace the existing `let view = self.filtered_view(&snap);` line.)

Update the tab strip to add Settings:

```rust
                ui.selectable_value(&mut self.tab, Tab::Charts, "Charts");
                ui.selectable_value(&mut self.tab, Tab::Sessions, "Sessions");
                ui.selectable_value(&mut self.tab, Tab::Calibration, "Calibration");
                ui.selectable_value(&mut self.tab, Tab::Settings, "Settings");
```

Add `Settings` to the `Tab` enum:

```rust
enum Tab {
    Charts,
    Sessions,
    Calibration,
    Settings,
}
```

Update the chart/sessions/calibration render calls in the `CentralPanel` match:

```rust
            Tab::Charts => {
                // ...unchanged until the chart render calls...
                    crate::dashboard::chart_5h::render(ui, &view, &mut self.range_5h, tz);
                    // ...
                    crate::dashboard::chart_weekly::render(ui, &view, &mut self.range_week, cp);
                    // ...
                    crate::dashboard::chart_daily::render(ui, &view, &mut self.range_daily, &weights, tz);
                    // ...
            }
            Tab::Sessions => {
                crate::dashboard::sessions_table::render(ui, &view.turns, &mut self.table_controls, tz);
            }
            Tab::Calibration => {
                let calib = self.calib_data(&snap, cp);
                crate::dashboard::calibration_tab::render(ui, &snap, &calib);
            }
            Tab::Settings => {
                crate::dashboard::settings_tab::render(
                    ui,
                    &mut self.settings_draft,
                    &self.settings,
                    &mut self.settings_save_msg,
                );
            }
```

(`settings_tab` is created in Task 13; until then this arm won't compile —
implement Task 13 before building. If doing strict per-task commits, comment the
`Tab::Settings` arm body with `unimplemented!()` temporarily, then wire it in
Task 13. Recommended: do Task 13 immediately after this task's edits, then build
once.)

- [ ] **Step 5: Build (after Task 13's file exists)**

Run: `cargo build`
Expected: `app.rs` compiles once `settings_tab.rs` exists.

- [ ] **Step 6: Commit**

```powershell
git add src/dashboard/app.rs; if ($?) { git commit -m "feat(settings): dashboard reads live settings; Settings tab wired; memo keys include tz/weights/CalParams" }
```

---

## Task 12: Wire `launch`, `TrayState`, `tray::run`, and `main`

**Files:**
- Modify: `src/dashboard/mod.rs` (`launch`)
- Modify: `src/tray/window.rs` (`TrayState` field + `launch` call)
- Modify: `src/tray/mod.rs` (`run`)
- Modify: `src/main.rs` (tray invocation)

- [ ] **Step 1: `dashboard::launch` takes `SharedSettings`**

In `src/dashboard/mod.rs`:

```rust
use crate::shared::{SharedSettings, SharedSnapshot};

pub fn launch(shared: SharedSnapshot, settings: SharedSettings) -> DashboardHandle {
    let signals = Arc::new(DashboardSignals::default());
    let signals_for_thread = signals.clone();

    let join = std::thread::spawn(move || {
        let app = DashboardApp::new(shared, signals_for_thread, settings);
        // ...unchanged NativeOptions + run_native...
    });

    DashboardHandle { signals, join }
}
```

- [ ] **Step 2: `TrayState` carries `SharedSettings`**

In `src/tray/window.rs`:
- Add a field to the `TrayState` struct (next to `pub shared: crate::shared::SharedSnapshot,`):

```rust
    pub settings: crate::shared::SharedSettings,
```

- Update the `launch` call (window.rs:541):

```rust
            *guard = Some(crate::dashboard::launch(state.shared.clone(), state.settings.clone()));
```

- [ ] **Step 3: `tray::run` builds + threads `SharedSettings`**

In `src/tray/mod.rs`, change `run` to construct the settings store, drop the
`interval_secs` parameter, and pass settings into both `TrayState` and the
poller:

```rust
pub fn run() -> Result<()> {
    let creds = load_from_default_path()?;
    use crate::shared::{new_shared_settings, new_shared_snapshot};
    let shared = new_shared_snapshot();
    let settings = new_shared_settings();
    // ...unchanged dashboard handle / hinst / renderer / shutdown / channels...

    let state = Box::new(window::TrayState {
        // ...unchanged fields...
        shared: shared.clone(),
        settings: settings.clone(),
        // ...unchanged...
    });

    // ...unchanged create/icon...

    let send_hwnd = poller::SendHwnd(hwnd);
    let poll_handle = poller::spawn(
        creds,
        shutdown.clone(),
        send_hwnd,
        tx,
        update_tx,
        shared.clone(),
        settings.clone(),
    );
    // ...unchanged message loop + joins...
}
```

- [ ] **Step 4: `main.rs` — tray mode no longer passes `--interval`**

In `src/main.rs`, the `else` branch (tray mode):

```rust
    } else {
        let _guard = claude_usage_tray::log::tray::init_file_subscriber(&cli.log_level)?;
        claude_usage_tray::tray::run()?;
    }
```

(The `--watch` branch still uses `cli.interval.as_secs()`. The `--interval` flag
now affects only `--watch`; document this in Task 14's CLAUDE.md note.)

- [ ] **Step 5: Build**

Run: `cargo build`
Expected: full workspace compiles (assuming Task 13 done).

- [ ] **Step 6: Commit**

```powershell
git add src/dashboard/mod.rs src/tray/window.rs src/tray/mod.rs src/main.rs; if ($?) { git commit -m "feat(settings): thread SharedSettings through tray run/launch; tray interval from settings" }
```

---

## Task 13: The Settings tab UI

**Files:**
- Create: `src/dashboard/settings_tab.rs`
- Modify: `src/dashboard/mod.rs` (register `pub mod settings_tab;`)

- [ ] **Step 1: Register the module**

In `src/dashboard/mod.rs`, add to the `pub mod` list:

```rust
pub mod settings_tab;
```

- [ ] **Step 2: Write the tab renderer**

Create `src/dashboard/settings_tab.rs`:

```rust
//! The Settings tab: edits a working-copy `Settings` (`draft`) and, on Save,
//! writes the shared store + persists to disk. Account-wide; ignores the global
//! filter bar (like the Calibration tab).

use crate::settings::{self, Settings, POLL_INTERVAL_CHOICES};
use crate::shared::SharedSettings;
use chrono::Weekday;
use egui::{ComboBox, DragValue, RichText, Ui};

const WEEKDAYS: [Weekday; 7] = [
    Weekday::Mon,
    Weekday::Tue,
    Weekday::Wed,
    Weekday::Thu,
    Weekday::Fri,
    Weekday::Sat,
    Weekday::Sun,
];

fn weekday_label(w: Weekday) -> &'static str {
    match w {
        Weekday::Mon => "Monday",
        Weekday::Tue => "Tuesday",
        Weekday::Wed => "Wednesday",
        Weekday::Thu => "Thursday",
        Weekday::Fri => "Friday",
        Weekday::Sat => "Saturday",
        Weekday::Sun => "Sunday",
    }
}

/// Render the Settings tab. `draft` is the editable working copy; `shared` is
/// the live store written on Save; `save_msg` shows the last save result.
pub fn render(
    ui: &mut Ui,
    draft: &mut Settings,
    shared: &SharedSettings,
    save_msg: &mut Option<Result<(), String>>,
) {
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        ui.heading("Settings");
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.button("Reset to defaults").clicked() {
                *draft = Settings::default();
            }
        });
    });
    ui.separator();
    ui.add_space(8.0);

    egui::Grid::new("settings_grid")
        .num_columns(2)
        .spacing([24.0, 12.0])
        .show(ui, |ui| {
            // Timezone
            ui.label("Timezone");
            ComboBox::from_id_salt("tz_combo")
                .selected_text(draft.local_tz.clone())
                .show_ui(ui, |ui| {
                    for tz in chrono_tz::TZ_VARIANTS {
                        let name = tz.name();
                        ui.selectable_value(&mut draft.local_tz, name.to_string(), name);
                    }
                });
            ui.end_row();

            // Weekly reset
            ui.label("Weekly reset");
            ui.horizontal(|ui| {
                ComboBox::from_id_salt("weekday_combo")
                    .selected_text(weekday_label(draft.weekly_reset_weekday))
                    .show_ui(ui, |ui| {
                        for w in WEEKDAYS {
                            ui.selectable_value(
                                &mut draft.weekly_reset_weekday,
                                w,
                                weekday_label(w),
                            );
                        }
                    });
                ui.label("at");
                ui.add(DragValue::new(&mut draft.weekly_reset_hour).range(0..=23));
                ui.label(":00 local");
            });
            ui.end_row();

            // Poll interval
            ui.label("Poll interval");
            ui.horizontal(|ui| {
                for secs in POLL_INTERVAL_CHOICES {
                    ui.selectable_value(
                        &mut draft.poll_interval_secs,
                        secs,
                        format!("{secs}s"),
                    );
                }
            });
            ui.end_row();

            // Cost weights
            ui.label("Cost weights");
            ui.horizontal(|ui| {
                weight_field(ui, "input", &mut draft.cost_weights.input);
                weight_field(ui, "cache-write", &mut draft.cost_weights.cache_creation);
                weight_field(ui, "cache-read", &mut draft.cost_weights.cache_read);
                weight_field(ui, "output", &mut draft.cost_weights.output);
            });
            ui.end_row();
        });

    ui.add_space(16.0);
    ui.separator();
    ui.add_space(8.0);

    // Dirty = draft differs from the live store.
    let current = shared.read().map(|g| g.clone()).unwrap_or_default();
    let dirty = *draft != current;
    let valid = settings::validate(draft);

    ui.horizontal(|ui| {
        let can_save = dirty && valid.is_ok();
        if ui
            .add_enabled(can_save, egui::Button::new("Save"))
            .clicked()
        {
            if let Ok(mut g) = shared.write() {
                *g = draft.clone();
            }
            *save_msg = Some(settings::save(draft).map_err(|e| e.to_string()));
        }

        match (&valid, dirty, save_msg.as_ref()) {
            (Err(msg), _, _) => {
                ui.label(RichText::new(format!("✗ {msg}")).color(egui::Color32::from_rgb(220, 120, 120)));
            }
            (Ok(()), true, _) => {
                ui.label(RichText::new("● unsaved changes").color(egui::Color32::from_rgb(220, 200, 120)));
            }
            (Ok(()), false, Some(Ok(()))) => {
                ui.label(RichText::new("✓ Saved").color(egui::Color32::from_rgb(120, 200, 120)));
            }
            (Ok(()), false, Some(Err(e))) => {
                ui.label(RichText::new(format!("✗ save failed: {e}")).color(egui::Color32::from_rgb(220, 120, 120)));
            }
            (Ok(()), false, None) => {}
        }
    });
}

fn weight_field(ui: &mut Ui, label: &str, value: &mut f64) {
    ui.vertical(|ui| {
        ui.label(RichText::new(label).small());
        ui.add(DragValue::new(value).speed(0.05).range(0.0..=f64::MAX));
    });
}
```

- [ ] **Step 3: Build**

Run: `cargo build`
Expected: compiles (this is the file `app.rs` Task 11 referenced).

- [ ] **Step 4: Commit**

```powershell
git add src/dashboard/settings_tab.rs src/dashboard/mod.rs; if ($?) { git commit -m "feat(settings): Settings tab UI (tz/weekly-reset/interval/cost-weights)" }
```

---

## Task 14: Final integration — lint, full test, manual verify, version bump

**Files:**
- Modify: `Cargo.toml` + `Cargo.lock` (version → 0.11.0)
- Modify: `CLAUDE.md` (roadmap + plans list + `--interval` behavior note)

- [ ] **Step 1: Format + lint**

Run: `cargo fmt`
Run: `cargo clippy --all-targets -- -D warnings`
Expected: no warnings. Common cleanups: remove any now-unused
`use crate::config;` in `bands.rs`/`snapshot.rs`; remove the `_assert_path_arg`
helper added in Task 3 if `Path` is otherwise used (it is — delete that helper).

- [ ] **Step 2: Full test suite**

Run: `cargo test`
Expected: all tests pass (settings module + all updated calibration/dashboard
tests).

- [ ] **Step 3: Manual verification (GUI — cannot be automated)**

Build + launch the tray binary:

Run: `cargo run`

Verify each:
- [ ] Tray icon appears; left-click opens the dashboard; a **Settings** tab is
  present in the strip after Calibration.
- [ ] **Timezone:** change from Europe/Copenhagen to e.g. America/New_York, Save.
  Chart x-axis date labels + the Sessions table timestamps shift to the new zone
  on the next frame (no restart).
- [ ] **Cost weights:** change `output` from 5 to 50, Save. The Charts tab "total
  burn" KPI + daily bar chart jump on the next frame.
- [ ] **Weekly reset:** change weekday to Monday, Save. The weekly chart's window
  boundaries shift within one poll interval (≤ the selected interval).
- [ ] **Poll interval:** change to 60s, Save. The status banner's next-poll ETA
  reflects 60s after the next poll completes.
- [ ] **Persistence:** confirm `~/.claude-usage-tray/settings.toml` was written;
  close + relaunch; the Settings tab shows the saved values.
- [ ] **Validation/dirty states:** editing shows "● unsaved changes"; Save shows
  "✓ Saved"; an out-of-range value (if reachable) disables Save.
- [ ] **Reset to defaults** repopulates the draft with Copenhagen / Sunday / 07 /
  120s / 1,1.25,0.1,5 (and Save persists them).

- [ ] **Step 4: Bump version + update CLAUDE.md**

In `Cargo.toml`, set `version = "0.11.0"`.

In `CLAUDE.md`:
- Add to the "Active design + plans" list:
  ```
  - **Stage 8 (mini-project 4) spec:** `docs/superpowers/specs/2026-05-26-stage-8-settings-panel-design.md` — Settings tab: local timezone, weekly reset, poll interval, cost weights; persisted to settings.toml, applied live via SharedSettings (Approach A). **Shipped 2026-05-26 (tag `v0.11.0`).**
  - **Stage 8 (mini-project 4) plan:** `docs/superpowers/plans/2026-05-26-stage-8-settings-panel.md` — task plan. **Shipped 2026-05-26 (tag `v0.11.0`).**
  ```
- Update the Stage 8 roadmap-table row to note mini-project 4 (settings panel)
  shipped `v0.11.0`, and mark Stage 8 complete.
- Under Conventions or a notes section, add: "Tray mode reads its poll interval
  from `settings.toml` (default 120s); the `--interval` CLI flag now governs only
  `--watch` terminal mode."

- [ ] **Step 5: Rebuild to sync `Cargo.lock` version, then commit**

Run: `cargo build`
(This rewrites `Cargo.lock`'s own `version =` line.)

```powershell
git add Cargo.toml Cargo.lock CLAUDE.md; if ($?) { git commit -m "chore: bump to v0.11.0 (Stage 8 settings panel)" }
```

- [ ] **Step 6: Tag the release**

```powershell
git tag v0.11.0
```

(Push — `git push origin main --tags` — when ready, per the user's release flow.)

---

## Self-review checklist (already applied during authoring)

- **Spec coverage:** timezone (Tasks 6-9,11), weekly reset (Tasks 6-7,9), poll
  interval (Tasks 10,12), cost weights (Tasks 5,8,11); persistence (Task 3);
  SharedSettings (Task 4); live-apply via per-frame/per-tick reads + memo-key
  invalidation (Tasks 10-11); Settings tab + filter-bar independence (Tasks
  11,13); `--interval` behavior change (Tasks 12,14). All covered.
- **Memo-invalidation gotcha:** `ViewSig`/`CalibSig` gain tz/weights/CalParams so
  cached views recompute when settings change — without this, live-apply would
  silently no-op while the filter/turns are unchanged.
- **Build-ordering note:** Tasks 6-9 change callee signatures whose callers are
  fixed in Tasks 10-13; the full build/test is green after Task 13. Per-task
  commits are fine; run targeted module tests in the interim.
