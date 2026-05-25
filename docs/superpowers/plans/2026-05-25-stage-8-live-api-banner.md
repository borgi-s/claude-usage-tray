# Live API Status Banner Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a persistent, read-only status strip at the top of the egui dashboard window showing poll freshness (status badge + last-poll age + next-poll ETA) and live 5h/7d utilization, visible on all three tabs.

**Architecture:** A new `src/dashboard/status_banner.rs` module exposes a pure `render(...)` entry point with unit-tested formatting helpers (`format_age`, `format_eta`, `util_line`, `badge_label`, `severity`). It's mounted as a `TopBottomPanel::top` above the existing filter bar in `app.rs`, reading the unfiltered shared snapshot. A new `interval_secs` field on `AppSnapshot` (set by the poller) feeds the next-poll ETA. Pure addition on the read path — no new threads, channels, or signals.

**Tech Stack:** Rust (stable, MSVC), egui/eframe 0.29, chrono. Reuses `render::format_duration`.

**Spec:** `docs/superpowers/specs/2026-05-25-stage-8-live-api-banner-design.md`

---

## Background for the implementer

You're working in a native Windows tray app. A background poller thread fetches Claude usage every 60–300s and writes an `AppSnapshot` into an `Arc<RwLock<AppSnapshot>>` (the "shared snapshot"). The egui dashboard thread reads that snapshot each frame and draws charts. This plan adds one read-only strip; you will not touch the poller's fetch logic, the calibration math, or the tray icon.

Rust idioms you'll meet (the project owner is a Rust beginner, so the code stays simple):
- `Option<&T>` — a borrowed optional value; `match` on it or use `if let Some(x) = ...`.
- `chrono::Duration` — a signed time span. `(a - b)` of two `DateTime`s yields one. `.num_seconds()` gives an `i64`; `.max(0)` clamps negatives to zero.
- `egui` immediate-mode UI: you call `ui.label(...)` / `ui.colored_label(color, text)` every frame to paint. There's no retained widget tree.
- Tests live in the same file under `#[cfg(test)] mod tests { ... }` and run with `cargo test`.

**TDD note for Rust:** a test that calls a function which doesn't exist yet fails by *failing to compile*. That counts as a red test. You then write the function to make it compile and pass.

---

## File structure

- **Create** `src/dashboard/status_banner.rs` — the banner: pure formatting helpers + tests + the egui `render` paint function. One responsibility: present poll status.
- **Modify** `src/dashboard/mod.rs` — declare the new module.
- **Modify** `src/dashboard/app.rs` — mount the banner panel above the filter bar.
- **Modify** `src/shared/snapshot.rs` — add `interval_secs: u64` to `AppSnapshot`.
- **Modify** `src/tray/poller.rs` — populate `interval_secs` in the snapshot writes.
- **Modify** `Cargo.toml`, `CLAUDE.md` — version bump + doc update (final task).

---

## Task 1: Add `interval_secs` to the shared snapshot

**Files:**
- Modify: `src/shared/snapshot.rs` (the `AppSnapshot` struct, ~line 14-24)
- Modify: `src/tray/poller.rs` (initial write ~line 106-108; snapshot literal ~line 160-170)

This is plumbing, not a new unit of behavior, so it's verified by compilation + the existing test suite rather than a new test. The banner (later tasks) needs the poll cadence to compute "next in Ns".

- [ ] **Step 1: Add the field to `AppSnapshot`**

In `src/shared/snapshot.rs`, add the field at the end of the struct:

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
    /// Poll cadence in seconds, set by the poller. Lets the dashboard banner
    /// show the next-poll ETA. Defaults to 0 (only observable pre-first-poll,
    /// when last_sample is None and the ETA is hidden anyway).
    pub interval_secs: u64,
}
```

`u64` defaults to `0` under `#[derive(Default)]`, so no other `Default` wiring is needed. The struct literal at `src/sync/export.rs:413` uses `..Default::default()`, so it keeps compiling untouched.

- [ ] **Step 2: Set it in the poller's initial write**

In `src/tray/poller.rs`, find the early publish block (~line 106):

```rust
    if let Ok(mut g) = shared.write() {
        g.last_status = last_status.clone();
    }
```

Change it to also set the interval (the `interval: Duration` parameter is already in scope):

```rust
    if let Ok(mut g) = shared.write() {
        g.last_status = last_status.clone();
        g.interval_secs = interval.as_secs();
    }
```

- [ ] **Step 3: Set it in the poller's per-poll snapshot literal**

In `src/tray/poller.rs`, find the `AppSnapshot { ... }` literal (~line 160) and add the field:

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
            interval_secs: interval.as_secs(),
        };
```

- [ ] **Step 4: Verify it compiles and existing tests pass**

Run: `cargo test`
Expected: builds cleanly; all existing tests PASS (no test referenced the new field).

- [ ] **Step 5: Commit**

```bash
git add src/shared/snapshot.rs src/tray/poller.rs
git commit -m "feat(stage-8): add interval_secs to shared snapshot for status banner"
```

---

## Task 2: Create the banner module + `format_age` helper (TDD)

**Files:**
- Create: `src/dashboard/status_banner.rs`
- Modify: `src/dashboard/mod.rs`

`format_age` renders a poll's age at *seconds* resolution so "live" actually reads as live (`render::format_duration` only has minute granularity, which is right for reset countdowns but too coarse here).

- [ ] **Step 1: Declare the module**

In `src/dashboard/mod.rs`, add the declaration in alphabetical order, right after `pub mod sessions_table;`:

```rust
pub mod sessions_table;
pub mod status_banner;
```

- [ ] **Step 2: Write the failing test (creates the file)**

Create `src/dashboard/status_banner.rs` with exactly this content:

```rust
//! Live API status banner: a persistent read-only strip at the top of the
//! dashboard window. Shows the poll status badge, last-poll age, next-poll ETA,
//! and live 5h/7d utilization. Mirrors the `--watch` CLI footer in egui form.

use chrono::Duration;

/// Format a poll's age at seconds resolution: `12s ago`, `1m 5s ago`,
/// `1h 1m ago`. Negative spans clamp to `0s ago`.
fn format_age(d: Duration) -> String {
    let secs = d.num_seconds().max(0);
    if secs < 60 {
        format!("{}s ago", secs)
    } else if secs < 3600 {
        format!("{}m {}s ago", secs / 60, secs % 60)
    } else {
        format!("{}h {}m ago", secs / 3600, (secs % 3600) / 60)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_age_buckets() {
        assert_eq!(format_age(Duration::seconds(5)), "5s ago");
        assert_eq!(format_age(Duration::seconds(0)), "0s ago");
        assert_eq!(format_age(Duration::seconds(65)), "1m 5s ago");
        assert_eq!(format_age(Duration::seconds(3700)), "1h 1m ago");
        // Negative (clock skew / future timestamp) clamps to zero.
        assert_eq!(format_age(Duration::seconds(-10)), "0s ago");
    }
}
```

- [ ] **Step 3: Run the test to verify it passes**

Run: `cargo test --lib status_banner`
Expected: `format_age_buckets` PASSES. (Here the function is written alongside the test; the helper is small and total, so we assert correctness directly rather than staging a separate compile-failure step.)

- [ ] **Step 4: Commit**

```bash
git add src/dashboard/status_banner.rs src/dashboard/mod.rs
git commit -m "feat(stage-8): status_banner module + format_age helper"
```

---

## Task 3: Add the `format_eta` helper (TDD)

**Files:**
- Modify: `src/dashboard/status_banner.rs`

`format_eta` renders the time until the next poll. It's always ≤ the interval (≤ 300s), so no hours branch is needed; negatives (a poll is due/in flight) clamp to `0s`.

- [ ] **Step 1: Write the failing test**

In `src/dashboard/status_banner.rs`, add this test inside the existing `mod tests` block:

```rust
    #[test]
    fn format_eta_buckets() {
        assert_eq!(format_eta(Duration::seconds(48)), "48s");
        assert_eq!(format_eta(Duration::seconds(125)), "2m 5s");
        // Next-poll time already past → clamp to zero.
        assert_eq!(format_eta(Duration::seconds(-3)), "0s");
    }
```

- [ ] **Step 2: Run the test to verify it fails (does not compile)**

Run: `cargo test --lib status_banner`
Expected: FAIL — compile error, `cannot find function format_eta in this scope`.

- [ ] **Step 3: Implement `format_eta`**

In `src/dashboard/status_banner.rs`, add this function right after `format_age`:

```rust
/// Format the time until the next poll at seconds resolution: `48s`, `2m 5s`.
/// Always less than the poll interval; negative spans clamp to `0s`.
fn format_eta(d: Duration) -> String {
    let secs = d.num_seconds().max(0);
    if secs < 60 {
        format!("{}s", secs)
    } else {
        format!("{}m {}s", secs / 60, secs % 60)
    }
}
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `cargo test --lib status_banner`
Expected: `format_eta_buckets` PASSES (and `format_age_buckets` still PASSES).

- [ ] **Step 5: Commit**

```bash
git add src/dashboard/status_banner.rs
git commit -m "feat(stage-8): format_eta helper for next-poll countdown"
```

---

## Task 4: Add the `util_line` helper (TDD)

**Files:**
- Modify: `src/dashboard/status_banner.rs`

`util_line` builds the `5h … · 7d …` string from the last sample, appending a reset countdown per bucket via `render::format_duration`. Missing data falls back to an em-dash (`—`). The middle dot (`·`) separator and em-dash use `\u{...}` escapes to match the existing `render.rs` style.

- [ ] **Step 1: Update imports**

In `src/dashboard/status_banner.rs`, replace the import line:

```rust
use chrono::Duration;
```

with:

```rust
use crate::api::usage::{UsageBucket, UsageSnapshot};
use chrono::{DateTime, Duration, Utc};
```

- [ ] **Step 2: Write the failing test**

In `src/dashboard/status_banner.rs`, add to the `mod tests` block. Also add the `chrono::TimeZone` import the test needs at the top of `mod tests` (right under `use super::*;`):

```rust
    use chrono::TimeZone;

    fn now_fixed() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 5, 25, 12, 0, 0).unwrap()
    }

    #[test]
    fn util_line_no_sample_shows_dashes() {
        assert_eq!(util_line(None, now_fixed()), "5h \u{2014} \u{00B7} 7d \u{2014}");
    }

    #[test]
    fn util_line_both_buckets_with_and_without_reset() {
        let snap = UsageSnapshot {
            // 43%, resets in 2h 10m.
            five_hour: Some(UsageBucket {
                utilization: 0.43,
                resets_at: Some(now_fixed() + Duration::minutes(130)),
            }),
            // 71%, no reset time.
            seven_day: Some(UsageBucket {
                utilization: 0.71,
                resets_at: None,
            }),
        };
        assert_eq!(
            util_line(Some(&snap), now_fixed()),
            "5h 43% (resets 2h 10m) \u{00B7} 7d 71%"
        );
    }

    #[test]
    fn util_line_missing_bucket_shows_dash() {
        let snap = UsageSnapshot {
            five_hour: Some(UsageBucket {
                utilization: 0.50,
                resets_at: None,
            }),
            seven_day: None,
        };
        assert_eq!(
            util_line(Some(&snap), now_fixed()),
            "5h 50% \u{00B7} 7d \u{2014}"
        );
    }
```

- [ ] **Step 3: Run the test to verify it fails (does not compile)**

Run: `cargo test --lib status_banner`
Expected: FAIL — compile error, `cannot find function util_line in this scope`.

- [ ] **Step 4: Implement `util_line` and `bucket_str`**

In `src/dashboard/status_banner.rs`, add after `format_eta`:

```rust
/// Build the `5h … · 7d …` utilization string. `None` (no poll yet) shows
/// both windows as em-dashes.
fn util_line(sample: Option<&UsageSnapshot>, now: DateTime<Utc>) -> String {
    match sample {
        None => "5h \u{2014} \u{00B7} 7d \u{2014}".to_string(),
        Some(snap) => format!(
            "5h {} \u{00B7} 7d {}",
            bucket_str(snap.five_hour.as_ref(), now),
            bucket_str(snap.seven_day.as_ref(), now),
        ),
    }
}

/// One bucket: `43%`, or `43% (resets 2h 10m)` when a reset time is known,
/// or `—` when the bucket is absent.
fn bucket_str(b: Option<&UsageBucket>, now: DateTime<Utc>) -> String {
    match b {
        None => "\u{2014}".to_string(),
        Some(bucket) => {
            let pct = (bucket.utilization * 100.0).round() as i64;
            match bucket.resets_at {
                Some(when) => {
                    format!("{}% (resets {})", pct, crate::render::format_duration(when - now))
                }
                None => format!("{}%", pct),
            }
        }
    }
}
```

- [ ] **Step 5: Run the test to verify it passes**

Run: `cargo test --lib status_banner`
Expected: all three `util_line_*` tests PASS, prior tests still PASS.

- [ ] **Step 6: Commit**

```bash
git add src/dashboard/status_banner.rs
git commit -m "feat(stage-8): util_line helper for live 5h/7d display"
```

---

## Task 5: Add `severity` + `badge_label` helpers (TDD)

**Files:**
- Modify: `src/dashboard/status_banner.rs`

`severity` maps the poll status to a visual treatment (Neutral / Warn / Error) that drives the strip's background color. `badge_label` produces the text shown next to the status dot (empty in the Ok state — the green dot says it all).

- [ ] **Step 1: Update imports**

In `src/dashboard/status_banner.rs`, add this import line under the existing `use crate::api::usage...` line:

```rust
use crate::render::LastStatus;
```

- [ ] **Step 2: Write the failing test**

In `src/dashboard/status_banner.rs`, add to the `mod tests` block:

```rust
    #[test]
    fn severity_maps_each_status() {
        assert_eq!(severity(&LastStatus::Initial), Severity::Neutral);
        assert_eq!(severity(&LastStatus::Ok), Severity::Neutral);
        assert_eq!(severity(&LastStatus::RateLimited), Severity::Warn);
        assert_eq!(severity(&LastStatus::Error("x".into())), Severity::Error);
    }

    #[test]
    fn badge_label_per_status() {
        assert_eq!(badge_label(&LastStatus::Initial), "fetching\u{2026}");
        assert_eq!(badge_label(&LastStatus::Ok), "");
        assert_eq!(badge_label(&LastStatus::RateLimited), "rate-limited");
        assert_eq!(badge_label(&LastStatus::Error("boom".into())), "error: boom");
    }
```

- [ ] **Step 3: Run the test to verify it fails (does not compile)**

Run: `cargo test --lib status_banner`
Expected: FAIL — compile error, `cannot find type Severity` / `cannot find function severity` / `badge_label`.

- [ ] **Step 4: Implement `Severity`, `severity`, and `badge_label`**

In `src/dashboard/status_banner.rs`, add after `bucket_str`:

```rust
/// Visual severity of the current poll status — drives the strip background.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Severity {
    /// Initial or Ok: neutral strip (color would be visual noise when nothing's wrong).
    Neutral,
    /// Rate-limited: amber strip.
    Warn,
    /// Error: red strip.
    Error,
}

fn severity(status: &LastStatus) -> Severity {
    match status {
        LastStatus::Initial | LastStatus::Ok => Severity::Neutral,
        LastStatus::RateLimited => Severity::Warn,
        LastStatus::Error(_) => Severity::Error,
    }
}

/// Text shown next to the status dot. Empty for `Ok` — the green dot is enough.
fn badge_label(status: &LastStatus) -> String {
    match status {
        LastStatus::Initial => "fetching\u{2026}".to_string(),
        LastStatus::Ok => String::new(),
        LastStatus::RateLimited => "rate-limited".to_string(),
        LastStatus::Error(msg) => format!("error: {}", msg),
    }
}
```

- [ ] **Step 5: Run the test to verify it passes**

Run: `cargo test --lib status_banner`
Expected: `severity_maps_each_status` and `badge_label_per_status` PASS, prior tests still PASS.

- [ ] **Step 6: Commit**

```bash
git add src/dashboard/status_banner.rs
git commit -m "feat(stage-8): severity + badge_label helpers for banner status"
```

---

## Task 6: Add the `render` paint function and mount it (compile + manual)

**Files:**
- Modify: `src/dashboard/status_banner.rs` (add `pub fn render`)
- Modify: `src/dashboard/app.rs` (mount the panel)

The paint function needs a live `egui::Ui`, so it isn't unit-tested; it composes the already-tested helpers and is verified by a clean build plus a manual visual check. egui is immediate-mode: `render` is called every frame from `app.rs`.

- [ ] **Step 1: Add the egui imports**

In `src/dashboard/status_banner.rs`, add at the top with the other `use` lines:

```rust
use egui::{Color32, Ui};
```

- [ ] **Step 2: Implement `render`**

In `src/dashboard/status_banner.rs`, add the public paint function (place it above the helpers or below them — order doesn't matter in Rust):

```rust
/// Paint the status strip. `now` is passed in (not `Utc::now()` internally) to
/// keep the helpers deterministic for tests and match `render::draw_frame`'s
/// convention. Reads the unfiltered snapshot fields — poll status is account-wide.
pub fn render(
    ui: &mut Ui,
    last_sample: Option<&(UsageSnapshot, DateTime<Utc>)>,
    last_status: &LastStatus,
    interval_secs: u64,
    now: DateTime<Utc>,
) {
    // Capture the theme's text color before we borrow `ui` mutably in the frame.
    let neutral_text = ui.visuals().text_color();
    let (fill, text_color) = match severity(last_status) {
        Severity::Neutral => (None, neutral_text),
        Severity::Warn => (
            Some(Color32::from_rgb(70, 55, 25)),
            Color32::from_rgb(230, 190, 90),
        ),
        Severity::Error => (
            Some(Color32::from_rgb(70, 30, 30)),
            Color32::from_rgb(235, 130, 120),
        ),
    };

    let mut frame = egui::Frame::default().inner_margin(egui::Margin::symmetric(8.0, 4.0));
    if let Some(c) = fill {
        frame = frame.fill(c);
    }

    frame.show(ui, |ui| {
        ui.horizontal(|ui| {
            // Status dot: green only when the last poll succeeded.
            let dot = match last_status {
                LastStatus::Ok => Color32::from_rgb(80, 200, 120),
                LastStatus::Initial => Color32::GRAY,
                LastStatus::RateLimited => Color32::from_rgb(230, 190, 90),
                LastStatus::Error(_) => Color32::from_rgb(235, 130, 120),
            };
            ui.colored_label(dot, "\u{25CF}"); // ● filled circle

            // Badge (empty in the Ok state — the green dot says it all).
            let badge = badge_label(last_status);
            if !badge.is_empty() {
                ui.colored_label(text_color, badge);
                ui.label("\u{00B7}");
            }

            // Last-poll age.
            let age = match last_sample {
                Some((_, t)) => format!("updated {}", format_age(now - *t)),
                None => "updated never".to_string(),
            };
            ui.colored_label(text_color, age);

            // Next-poll ETA (only meaningful once a poll has landed).
            if let Some((_, t)) = last_sample {
                let next = *t + Duration::seconds(interval_secs as i64);
                ui.label("\u{00B7}");
                ui.colored_label(text_color, format!("next in {}", format_eta(next - now)));
            }

            // Live util.
            ui.label("\u{00B7}");
            ui.colored_label(text_color, util_line(last_sample.map(|(s, _)| s), now));
        });
    });
}
```

- [ ] **Step 3: Mount the panel in `app.rs`**

In `src/dashboard/app.rs`, find this point in the visible branch of `update` (~line 185):

```rust
        let view = self.filtered_view(&snap);

        egui::TopBottomPanel::top("filter_bar_panel").show(ctx, |ui| {
```

Insert the banner panel between those two statements (it must be declared *before* the filter-bar panel so egui stacks it at the very top):

```rust
        let view = self.filtered_view(&snap);

        egui::TopBottomPanel::top("status_banner_panel").show(ctx, |ui| {
            crate::dashboard::status_banner::render(
                ui,
                snap.last_sample.as_ref(),
                &snap.last_status,
                snap.interval_secs,
                Utc::now(),
            );
        });

        egui::TopBottomPanel::top("filter_bar_panel").show(ctx, |ui| {
```

`Utc` is already imported in `app.rs` (`use chrono::{DateTime, Utc};`), and `snap` is the unfiltered snapshot already read just above.

- [ ] **Step 4: Verify it builds and all tests pass**

Run: `cargo build`
Expected: clean build, no errors.

Run: `cargo test`
Expected: all tests PASS (including the banner helper tests from Tasks 2-5).

- [ ] **Step 5: Manual visual check (user-driven)**

This is a GUI app on Windows; the visual confirmation is done by a human (per the project's tray manual-test convention). Run `cargo run` to start the tray, open the dashboard from the tray menu, and confirm:
- A thin strip sits above the filter bar, visible on Charts / Sessions / Calibration tabs.
- After a successful poll: a green dot, `updated Ns ago` ticking up, `next in Ns` counting down, and `5h N% · 7d N%`.
- The strip background stays neutral when Ok (only the dot is colored).

If you cannot run the GUI in this environment, note that the build + helper tests are green and defer the visual check to the user.

- [ ] **Step 6: Commit**

```bash
git add src/dashboard/status_banner.rs src/dashboard/app.rs
git commit -m "feat(stage-8): render live API status banner + mount in dashboard"
```

---

## Task 7: Lint, version bump, and docs

**Files:**
- Modify: `Cargo.toml` (version)
- Modify: `CLAUDE.md` (spec/plan pointers)

- [ ] **Step 1: Format and lint**

Run: `cargo fmt`
Run: `cargo clippy --all-targets -- -D warnings`
Expected: no warnings (the project's pre-release gate). Fix any reported issues, e.g. an unused import — there should be none if imports were added per task.

- [ ] **Step 2: Bump the version**

In `Cargo.toml`, change:

```toml
version = "0.9.0"
```

to:

```toml
version = "0.10.0"
```

- [ ] **Step 3: Update CLAUDE.md spec/plan pointers**

In `CLAUDE.md`, find the mini-project 2 plan bullet in the "Active design + plans" section:

```markdown
- **Stage 8 (mini-project 2) plan:** `docs/superpowers/plans/2026-05-25-stage-8-calibration-history.md` — task plan. **Shipped 2026-05-25 (tag `v0.9.0`).**
```

Add two new bullets immediately after it:

```markdown
- **Stage 8 (mini-project 3) spec:** `docs/superpowers/specs/2026-05-25-stage-8-live-api-banner-design.md` — live API status banner: persistent top strip showing poll status badge + last-poll age + next-poll ETA + live 5h/7d util. Reads the shared snapshot (adds `interval_secs`); account-wide (ignores the global filter bar).
- **Stage 8 (mini-project 3) plan:** `docs/superpowers/plans/2026-05-25-stage-8-live-api-banner.md` — task plan.
```

Then update the trailing sentence in that section:

```markdown
Stage 8 ships as a series of mini-projects (one per spec+plan); the remaining ones (live API status banner, settings panel) get their own specs when started.
```

to:

```markdown
Stage 8 ships as a series of mini-projects (one per spec+plan); the remaining one (settings panel) gets its own spec when started.
```

(The roadmap-table "Shipped" annotation + the plan bullets' `Shipped … (tag v0.10.0)` markers are added during the finishing-a-development-branch step, after the release tag exists.)

- [ ] **Step 4: Verify everything is still green**

Run: `cargo test`
Expected: all tests PASS.

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml CLAUDE.md
git commit -m "chore: bump to v0.10.0 (Stage 8 mini-project 3 — live API status banner)"
```

---

## Done criteria

- `cargo fmt` + `cargo clippy --all-targets -- -D warnings` clean.
- `cargo test` green, including the new `status_banner` helper tests.
- The dashboard shows a persistent status strip above the filter bar on all three tabs, neutral when Ok and color-escalating on rate-limit/error.
- No changes to poll cadence, calibration math, tray icon, or tooltip.

Release tagging (`v0.10.0`), the branch merge, and the CLAUDE.md roadmap-row update are handled by the finishing-a-development-branch skill after this plan completes.
