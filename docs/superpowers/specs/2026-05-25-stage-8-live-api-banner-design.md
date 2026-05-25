# Stage 8 (mini-project 3) — Live API Status Banner

> Part of Stage 8 ("Streamlit feature parity"). Third Stage 8 deliverable,
> after mini-project 1 (sessions table + global filters, shipped `v0.8.0`) and
> mini-project 2 (calibration history tab, shipped `v0.9.0`). The remaining
> mini-project (settings panel) gets its own spec.

## Goal

Add a persistent, read-only status strip at the top of the egui dashboard
window — visible on all three tabs (Charts / Sessions / Calibration) — that
shows poll freshness and current usage at a glance. It mirrors the `--watch`
CLI footer (`render::format_footer`) in egui form, answering "is the data live,
and what's my usage right now?" from anywhere in the window.

The master design spec describes this as matching the Streamlit agent's
`Updated Ns ago · sub pro tier …` line. We keep the freshness + util parts and
deliberately drop the subscription/tier part (see Non-goals).

## Non-goals (this mini-project)

- ❌ Subscription / tier display. Credentials aren't in the shared snapshot, and
  the known quirk (`subscriptionType` misreports Max as `"pro"`) makes the value
  misleading anyway. Deferred to the settings panel mini-project, which already
  touches config/creds.
- ❌ Manual "Refresh now" button. The `/api/oauth/usage` endpoint rate-limits to
  ~1 req/min; a force-poll needs cooldown/debounce + a cross-thread signal into
  the poller. Deferred — it overlaps with the future settings panel.
- ❌ Touching the poll cadence, calibration math, icon, or tooltip.
- ❌ Applying the global filter bar to the banner. Poll status is account-wide;
  the banner reads the unfiltered snapshot.

## Background: what exists today

- **The shared snapshot already carries the data.** `AppSnapshot`
  (`src/shared/snapshot.rs`) has `last_sample: Option<(UsageSnapshot,
  DateTime<Utc>)>` and `last_status: LastStatus`, both written every poll by the
  tray poller (`src/tray/poller.rs`). The dashboard already reads this snapshot
  each frame.
- **`LastStatus`** (`src/render.rs`) is the status enum:
  `Initial` (pre-first-poll), `Ok`, `RateLimited`, `Error(String)`.
- **`UsageSnapshot`** (`src/api/usage.rs`) holds
  `five_hour: Option<UsageBucket>` and `seven_day: Option<UsageBucket>`; each
  `UsageBucket` has `utilization: f64` (0.0–1.0) and `resets_at:
  Option<DateTime<Utc>>`.
- **`render::format_duration(Duration) -> String`** is already `pub` and formats
  a span as `2d 3h` / `4h 12m` / `7m`. The `--watch` footer
  (`render::format_footer`) uses it for the "last poll / next / badge" line; we
  reuse the same formatter so the dashboard and CLI agree.
- **Dashboard layout** (`src/dashboard/app.rs::update`): in the visible branch it
  reads the snapshot, builds the filtered view, then shows a
  `TopBottomPanel::top("filter_bar_panel")` (filter bar + tab strip) followed by
  a `CentralPanel` that matches on the active `Tab`. egui stacks top panels in
  declaration order, so a banner panel declared *before* the filter-bar panel
  renders above it.
- **The existing "Uncalibrated" banner** (Charts tab) shows the in-repo pattern
  for a colored `egui::Frame` with `fill` + `inner_margin` + a `RichText` label.

## What the banner shows

A single horizontal line, left → right:

1. **Status dot + badge.**
   - `Ok` → small green dot, no text label (the dot + fresh age is enough).
   - `RateLimited` → amber badge text `rate-limited`.
   - `Error(msg)` → red badge text `error: <msg>`.
   - `Initial` → neutral badge text `fetching…`.
2. **Last-poll age** — `updated 12s ago`, computed from `last_sample`'s
   timestamp vs `now`. Before the first successful poll (`last_sample == None`)
   shows `updated never`.
3. **Next-poll ETA** — `next in 48s`, from last-poll time + `interval_secs` −
   `now`. Omitted entirely when `last_sample == None` (no basis to compute it).
   Clamped to `0s` if the computed next-poll time is already in the past (a poll
   is due / in flight).
4. **Live util** — `5h 43% · 7d 71%` from `last_sample`. Each bucket appends a
   reset countdown when `resets_at` is present, e.g. `5h 43% (resets 2h 10m)`.
   With no sample yet, shows `5h — · 7d —`. A bucket that's `None` in the sample
   shows `—` for that window.

Separators between segments are ` · ` (middle dot), matching the calibration-tab
and util conventions already used in the codebase.

### Age string granularity

`format_duration` has minute granularity (`7m`, `4h 12m`) — fine for the
"resets in" countdowns. But last-poll age at a 60–300s cadence wants seconds
resolution (`updated 12s ago`, `updated 1m 5s ago`) so "live" actually reads as
live. The banner therefore has its own small `format_age(Duration) -> String`
helper:

- `< 60s` → `Ns ago` (e.g. `12s ago`)
- `< 60m` → `Nm Ss ago` (e.g. `1m 5s ago`)
- otherwise → `Nh Mm ago`

Reset countdowns keep using `render::format_duration` (minute granularity is
correct there). Next-poll ETA also uses seconds granularity via a tiny inline
`Ns` / `Nm Ss` format (it's always < interval, i.e. ≤ 300s).

## Color treatment

Neutral strip in the normal case; color escalates only on trouble:

| `last_status`        | Strip background        | Dot / badge                       |
|----------------------|-------------------------|-----------------------------------|
| `Initial`            | neutral (panel default) | gray dot, `fetching…`             |
| `Ok`                 | neutral (panel default) | green dot, no badge text          |
| `RateLimited`        | amber fill              | amber `rate-limited` badge        |
| `Error(msg)`         | red fill                | red `error: <msg>` badge          |

A persistent always-green bar would be visually heavy, so Ok stays neutral with
just the green dot. The amber/red fills reuse the `egui::Frame { fill,
inner_margin }` pattern from the Uncalibrated banner. Exact colors picked to sit
on egui's dark theme (amber ≈ `rgb(70, 55, 25)` bg with `rgb(230, 190, 90)`
text; red ≈ `rgb(70, 30, 30)` bg with `rgb(235, 130, 120)` text), consistent
with the existing warning banner's palette.

## Architecture

### New module: `src/dashboard/status_banner.rs`

Follows the existing one-purpose-per-module dashboard pattern (`filter_bar.rs`,
`kpi.rs`). Public entry point:

```rust
pub fn render(
    ui: &mut egui::Ui,
    last_sample: Option<&(UsageSnapshot, DateTime<Utc>)>,
    last_status: &LastStatus,
    interval_secs: u64,
    now: DateTime<Utc>,
);
```

`now` is passed in (not `Utc::now()` internally) so the formatting is
deterministic for tests, matching `render::draw_frame`'s convention.

Internally it computes the segment strings via small pure helpers, then paints:
a colored-or-neutral `egui::Frame` containing a `ui.horizontal(...)` with the
dot (a `Color32` filled circle or a colored `●` glyph), badge, age, ETA, and
util label. The pure helpers (the testable core) are kept free of any `egui`
paint calls:

- `fn format_age(d: Duration) -> String` — seconds-granularity age (see above).
- `fn format_eta(d: Duration) -> String` — seconds-granularity, clamped ≥ 0.
- `fn util_line(sample: Option<&UsageSnapshot>, now) -> String` — the
  `5h … · 7d …` string incl. reset countdowns / `—` fallbacks.
- `fn badge_text(status: &LastStatus) -> &str` / the status → color mapping.

### Placement in `app.rs`

In the visible branch of `DashboardApp::update`, before the existing
`filter_bar_panel`:

```rust
egui::TopBottomPanel::top("status_banner_panel").show(ctx, |ui| {
    crate::dashboard::status_banner::render(
        ui,
        snap.last_sample.as_ref(),
        &snap.last_status,
        snap.interval_secs,
        chrono::Utc::now(),
    );
});
```

It reads the **unfiltered** `snap` (already read at the top of the visible
branch), not the filtered `view` — poll status is account-wide. Declared first
⇒ renders at the very top of the window, above the filter bar + tab strip, on
every tab.

### Interval plumbing

The banner needs the poll cadence for the next-poll ETA. Add a field to the
shared snapshot:

```rust
// src/shared/snapshot.rs — AppSnapshot
pub interval_secs: u64,
```

Set once by the poller when it first writes the snapshot
(`src/tray/poller.rs`), so it's correct from the first write rather than
defaulting to `0`. `interval` is already in scope there (the poller is
constructed with `interval_secs`); write `interval.as_secs()` into the snapshot
alongside the existing fields.

**Why the snapshot and not a constructor arg:** every other banner input
(`last_sample`, `last_status`) already comes from the snapshot the dashboard
reads each frame, so this keeps all banner inputs in one place and is the
smallest threading diff. *Alternative considered:* thread `interval_secs`
through `dashboard::launch` → `DashboardApp::new` (interval is a process
constant, arguably not "live state"), but that touches three signatures plus
`TrayState`. Rejected as more invasive for no real benefit.

`AppSnapshot::default()` gives `interval_secs == 0`; that's only observable
before the first poll write, during which `last_sample == None` and the ETA
segment is omitted anyway, so a `0` interval is never rendered.

## Data flow

```
tray poller ──writes every poll──▶ AppSnapshot {
                                      last_sample, last_status, interval_secs, …
                                    }  (Arc<RwLock<…>>)
                                         │ read each frame (unfiltered)
                                         ▼
                          status_banner::render(ui, …)  ──paints top strip
```

No new threads, channels, or signals. Pure addition on the read path.

## Error / edge handling

| Situation                          | Banner behavior                                   |
|------------------------------------|---------------------------------------------------|
| No poll yet (`last_sample = None`) | `updated never`, ETA omitted, `5h — · 7d —`, gray dot, `fetching…` |
| `Ok` but a bucket is `None`        | That window shows `—`; the other shows its %      |
| `resets_at = None` on a bucket     | Show `5h 43%` with no `(resets …)` suffix         |
| `RateLimited`                      | Amber strip; age keeps counting up from last good sample (data is stale-but-shown) |
| `Error(msg)`                       | Red strip; `error: <msg>`; age counts up from last good sample |
| Next-poll time already past        | `next in 0s` (poll due / in flight)               |
| Poisoned snapshot lock             | Handled upstream exactly as today (`app.rs` already `.unwrap()`s the read; no change) |

The banner never panics on missing data — every field has a `—` / omitted
fallback.

## Testing

Unit tests on the pure helpers in `status_banner.rs` (no egui context needed):

- `format_age`: `5s` → `5s ago`; `65s` → `1m 5s ago`; `3700s` → `1h 1m ago`;
  zero → `0s ago`.
- `format_eta`: sub-minute, multi-minute, and negative (past) → clamped `0s`.
- `util_line`: both buckets present with resets; one bucket `None`; sample
  `None` → `5h — · 7d —`; bucket present but `resets_at None` → no countdown.
- `badge_text` / status→color mapping: one assertion per `LastStatus` variant.

Manual smoke test (per the Windows tray testing notes): launch the tray, open
the dashboard, confirm the strip shows above the filter bar on all three tabs,
the age ticks up between polls, the dot is green after a successful poll, and
the strip is unobtrusive in the Ok state.

No new integration tests; this is a read-path presentation layer over data
that's already exercised by the poller + tray tests.

## Files touched

- **New:** `src/dashboard/status_banner.rs` (render + pure helpers + tests).
- `src/dashboard/mod.rs` — `pub mod status_banner;`.
- `src/dashboard/app.rs` — add the `status_banner_panel` top panel in the
  visible branch.
- `src/shared/snapshot.rs` — add `interval_secs: u64` to `AppSnapshot`.
- `src/tray/poller.rs` — populate `interval_secs` in the snapshot write.

## Out-of-scope follow-ups (future Stage 8 mini-project)

- Settings panel (cost-weight / weekly-reset / local-TZ overrides), which is
  also the natural home for subscription/tier display and a guarded manual
  refresh.
