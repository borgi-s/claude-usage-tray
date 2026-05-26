# Stage 8 (mini-project 4) — Settings Panel

> Part of Stage 8 ("Streamlit feature parity"). Fourth and **final** Stage 8
> deliverable, after mini-project 1 (sessions table + global filters, shipped
> `v0.8.0`), mini-project 2 (calibration history tab, shipped `v0.9.0`), and
> mini-project 3 (live API status banner, shipped `v0.10.0`). Shipping this
> completes Stage 8 and the Streamlit replacement on Windows.

## Goal

Add a **Settings** tab to the egui dashboard that lets the user view and edit a
small set of runtime configuration values that are today compile-time `const`s
in `src/config.rs`. Changes persist to `~/.claude-usage-tray/settings.toml` and
apply **live** — both the dashboard (per-frame) and the polling thread read the
current values without a restart.

The master design spec names this mini-project as "Settings panel: cost weights
override, weekly reset config." We keep both and add two more values that are
genuinely user-specific and currently hardcoded.

## Scope — the four settings

| Setting | Today (in `config.rs`) | Why it's user-tunable |
|---|---|---|
| **Local timezone** | `LOCAL_TZ = "Europe/Copenhagen"` | Hardcoded; anyone outside CET sees wrong local times in charts/labels. A real wart for a shippable `.exe`. |
| **Weekly reset** | `WEEKLY_RESET_WEEKDAY = Sun`, `WEEKLY_RESET_HOUR_LOCAL = 7` | Named in the master spec. Feeds cap math (`derive_caps`). |
| **Poll interval** | CLI `--interval` flag (default 120) | Currently only settable on the command line; a tray user who double-clicks the `.exe` can't change it without relaunching. |
| **Cost weights** | `COST_WEIGHT_{INPUT,CACHE_CREATION,CACHE_READ,OUTPUT}` | Named in the master spec. Drives the spend / cost-weighted KPI + chart view (display only; **not** cap calibration). |

## Non-goals (this mini-project)

- ❌ **Calibration tuning** (`FIVE_HOUR_WINDOW_HOURS`, `MIN_ANCHOR_UTIL`,
  `MAX_ANCHOR_UTIL`). Easy to misconfigure and would silently distort cap
  detection. Stays a compile-time const.
- ❌ **`MODEL_CONTEXT_WINDOWS` / `DEFAULT_CONTEXT_WINDOW`** editing. Not a
  per-user preference; it's reference data that ships with the binary.
- ❌ **Subscription / tier display** and a **manual "Refresh now" button** —
  both were tentatively floated for "the settings panel" in the live-API-banner
  spec (`2026-05-25-stage-8-live-api-banner-design.md`). Neither is part of the
  agreed scope here: tier is misreported by the API (`subscriptionType` quirk)
  and a force-poll needs cooldown/debounce + a cross-thread signal into the
  poller, which is a separate feature. Left out.
- ❌ **Sync (`SUPABASE_*`) configuration in the UI.** Those are secrets read
  from `.env` (`src/sync/config.rs`) and stay there; putting a service-role key
  in a plaintext `settings.toml` edited from a GUI is the wrong place for it.
- ❌ **A separate settings window.** The egui/winit one-EventLoop-per-process
  constraint makes a second viewport risky; we use a tab in the existing strip.
- ❌ **Out-of-band cap recompute on save.** Weekly-reset changes apply on the
  next poll (≤ interval); see Propagation. Deliberately not adding a force-recompute.

## Background: what exists today

- **All config is compile-time.** `src/config.rs` exposes the values above as
  `pub const`s. They are read directly across ~13 files (calibration:
  `anchors.rs`, `hourly.rs`, `history.rs`; dashboard: `series.rs`, `axis.rs`,
  `bands.rs`, `chart_5h.rs`, `chart_daily.rs`, `filters.rs`, `sessions_table.rs`;
  shared: `snapshot.rs`).
- **Shared state already uses `Arc<RwLock<…>>`.** `SharedSnapshot =
  Arc<RwLock<AppSnapshot>>` (`src/shared/mod.rs`) threads state between the
  poller thread and the dashboard. A settings store mirrors this exactly.
- **The poller recomputes every tick.** `polling_loop` (`src/tray/poller.rs`)
  calls `compute_calibration_with_turns()` → `derive_caps(...)` and
  `compute_kpis(...)` on each iteration, then writes the `AppSnapshot`. So
  re-reading config at the top of each tick makes cap/KPI changes apply on the
  next poll for free.
- **The dashboard recomputes per frame.** `DashboardApp::filtered_view` calls
  `compute_kpis` every frame; chart renderers run every frame. Display-only
  values (cost weights, timezone labels) therefore apply on the next frame.
- **Poll interval is captured once.** `poller::spawn(interval_secs, …)` turns it
  into a local `Duration` used by `sleep_interruptible`; the loop never re-reads
  it. Tray mode passes `cli.interval.as_secs()` (default 120) via
  `tray::run` (`src/main.rs:66`).

## Architecture (Approach A — shared lock, read at boundaries)

### Data model — `src/settings.rs` (new)

```rust
#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]                  // missing/partial fields fall back to defaults
pub struct Settings {
    pub local_tz: String,          // IANA name, e.g. "Europe/Copenhagen"
    pub weekly_reset_weekday: Weekday,
    pub weekly_reset_hour: u32,    // 0..=23, local
    pub poll_interval_secs: u64,   // one of {60, 120, 300}
    pub cost_weights: CostWeights,
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CostWeights {
    pub input: f64,
    pub cache_creation: f64,
    pub cache_read: f64,
    pub output: f64,
}
```

- `impl Default for Settings` / `CostWeights` returns **exactly** the current
  `config.rs` const values, so a fresh install with no file behaves identically
  to today. `config.rs` keeps the consts as the single source of those defaults
  (the `Default` impl references them), so there is one place to change a default.
- `local_tz` validated by parsing to `chrono_tz::Tz`; a parse helper
  `fn tz(&self) -> Tz` returns the parsed zone, falling back to the default zone
  if somehow invalid (defense in depth; the UI combo prevents bad values).

### Persistence

- New `paths::settings_path()` → `~/.claude-usage-tray/settings.toml`.
- **Format: TOML** via a new `toml` dependency (small, pure-Rust, no async).
  Chosen for human-editability and parity with `Cargo.toml` ergonomics. (The
  repo otherwise uses `serde_json` for `state.json` / `cache_manifest.json`;
  TOML is the deliberate choice for a user-facing config file.)
- `settings::load() -> Settings`: read file → parse → on **any** error
  (missing / corrupt / invalid), `tracing::warn!` and return
  `Settings::default()`. Never fails the app.
- `settings::save(&Settings) -> Result<()>`: serialize → write **atomically**
  (write to `settings.toml.tmp`, then rename over `settings.toml`) →
  `ensure_parent_dir` first. Returns the error so the UI can show it.

### Shared store

- `SharedSettings = Arc<RwLock<Settings>>` added to `src/shared/mod.rs`, with
  `new_shared_settings()` calling `settings::load()`.
- Constructed in `tray::run`, cloned into (a) the poller thread and (b)
  `DashboardApp::new`.

### Reading at boundaries (the Approach-A contract)

Deep functions keep taking **plain values**, not the lock, so they stay
unit-testable and never lock inside hot loops. The boundary reads the lock once,
snapshots into locals, and passes them down:

- **Poller** (`polling_loop`): at the top of each iteration, read the lock once
  into a local `Settings`. Pass `weekly_reset` + `tz` into the calibration path
  and `cost_weights` into `compute_kpis`. Use `poll_interval_secs` for
  `sleep_interruptible` and `snapshot.interval_secs`.
- **Dashboard** (`DashboardApp::update`): read the lock once per frame into a
  local; pass `cost_weights` into `compute_kpis` (`filtered_view`) and `tz` into
  chart axis/label formatting.

Functions whose signatures gain parameters (the boundary set, ~6):
`derive_caps`, `hour_of_day_cap_series`, `implied_cap_series`,
`per_hour_stats` (calibration: take weekly-reset + tz where they currently read
the const); `cost_weighted(&Turn, &CostWeights)` (the sole reader of the cost
consts, `snapshot.rs:43`) and its callers `compute_kpis` + the daily chart;
chart-axis / series formatters (take `Tz`). Each deep call site changes from
reading a `config::` const to
using the passed-in value. This is mechanical churn confined to the boundary
functions and their immediate callees.

### UI — `src/dashboard/settings_tab.rs` (new) + `Tab::Settings`

- New `Tab::Settings` variant; `selectable_value` added to the tab strip in
  `app.rs` (`Charts | Sessions | Calibration | Settings`).
- Like Calibration, the tab **ignores the global filter bar** (it's account-/
  app-wide config). The filter bar still renders above it (consistent with the
  current Calibration behavior); no change to the panel layout.
- `DashboardApp` gains `settings_draft: Settings` — a **working copy** the tab
  edits, so edits aren't applied keystroke-by-keystroke. Initialized from the
  shared lock on first show.

Layout:

```
Settings                                      [ Reset to defaults ]

Timezone        [ Europe/Copenhagen        ▼ ]   (combo: chrono_tz::TZ_VARIANTS)
Weekly reset    [ Sunday ▼ ]  at  [ 07 ] : 00  local
Poll interval   ( ) 60s    (•) 120s    ( ) 300s
Cost weights    input [1.00]  cache-write [1.25]  cache-read [0.10]  output [5.00]

                                [ Save ]    ● unsaved changes / ✓ Saved / ✗ <err>
```

- **Save** enabled only when `draft != *shared.read()` **and** the draft
  validates. On click: `*shared.write() = draft.clone()`, then
  `settings::save(&draft)`; show `✓ Saved` or `✗ <error>` from the save result.
- **Reset to defaults** sets `draft = Settings::default()` (not saved until Save).
- Widgets enforce validity: timezone combo (only real `Tz` values); weekday
  combo; hour `DragValue` clamped `0..=23`; interval radio (3 choices);
  weight `DragValue`s clamped `>= 0.0`. A `settings::validate(&Settings)`
  predicate exists too, for the file-load path and tests.

### Poll interval ↔ CLI flag

- **Tray mode** (the default, no `--once`/`--watch`) takes its interval from
  `settings.toml` (default 120 when absent), **not** from `--interval`.
  `main.rs` passes the loaded `Settings` (or the shared store) into `tray::run`
  instead of `cli.interval.as_secs()`.
- The `--interval` CLI flag continues to govern **`--watch`** terminal mode only.
- This is a small, documented behavior change: `--interval` no longer affects
  the tray. (Rejected alternative: have an explicitly-passed `--interval`
  override settings for that launch — detecting "explicitly passed" in clap adds
  fiddle for negligible benefit.)

## Propagation — what applies when

| Setting | Read at | Latency |
|---|---|---|
| Cost weights | `compute_kpis` (poller per tick **and** dashboard per frame) | Charts/KPIs next frame (~instant) |
| Local timezone | chart axis/label formatting (dashboard per frame) + display cap math | Next frame (~instant) |
| Weekly reset | `derive_caps` via `compute_calibration_with_turns` (poller) | **Next poll** (≤ interval) |
| Poll interval | top of `polling_loop` iteration + `snapshot.interval_secs` | Next iteration |

The weekly-reset "next poll" lag (≤ 5 min) is acceptable and deliberate — no
out-of-band recompute is added. Noted as a known latency.

## Testing

- `settings.rs`: round-trip serialize/deserialize; `load()` on a missing file →
  defaults; `load()` on corrupt TOML → defaults (+ warn); partial TOML fills
  gaps via `#[serde(default)]`; `save()` writes atomically and round-trips.
- `validate()` predicate: rejects bad tz string, hour `> 23`, negative weights,
  interval not in `{60,120,300}`; accepts the defaults.
- Boundary fns: add cases passing **non-default** `CostWeights` / weekly-reset /
  `Tz` to `compute_kpis` and `derive_caps` to prove the params actually drive
  output (guards against a missed const reference).
- `Default for Settings` equals the `config.rs` consts (a test asserting the two
  agree, so they can't silently drift).
- UI rendering stays manual-test, consistent with prior Stage 8 mini-projects.

## File-by-file change summary

**New**
- `src/settings.rs` — `Settings`, `CostWeights`, `Default`, `load`, `save`,
  `validate`, `tz()`.
- `src/dashboard/settings_tab.rs` — the tab renderer.

**Modified**
- `src/config.rs` — consts referenced by `Default` impls (kept as the default
  source); no consts removed.
- `src/paths.rs` — `settings_path()`.
- `src/shared/mod.rs` — `SharedSettings` + `new_shared_settings()`.
- `src/lib.rs` — `pub mod settings;`.
- `src/dashboard/mod.rs`, `src/dashboard/app.rs` — `Tab::Settings`,
  `settings_draft`, read shared settings per frame, pass values into boundaries.
- `src/tray/mod.rs` (`run`) + `src/tray/poller.rs` — construct/clone
  `SharedSettings`, read it per tick, drive interval + calibration + KPIs from it.
- `src/main.rs` — tray mode interval comes from settings, not `--interval`.
- Calibration boundary fns (`anchors.rs`, `hourly.rs`, `history.rs`) and
  dashboard formatters (`series.rs`, `axis.rs`, `compute_kpis` in `snapshot.rs`)
  — take values as params instead of reading consts.
- `Cargo.toml` / `Cargo.lock` — add `toml`.

## Version

Ships as `v0.11.0`, completing Stage 8. Update the CLAUDE.md roadmap row + the
Active plans list when shipped.
