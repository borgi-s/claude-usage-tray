# Stage 8 (mini-project 1) — Sessions Table + Global Filters

> Part of Stage 8 ("Streamlit feature parity"). This is the first and most
> self-contained Stage 8 deliverable. Subsequent mini-projects (calibration
> history, live API status banner, settings panel) get their own specs.

## Goal

Add a **sessions table** to the egui dashboard at full parity with the Python
Streamlit local agent's table, plus the **global date / project / model
filters** that, in the Python app, drive the *entire* dashboard. This
mini-project deliberately absorbs the roadmap's separate "Filter sidebar"
item — the table and the filters were designed together in Python and porting
them as one keeps behavior consistent.

**Why these two together:** the Python sessions table reads from the same
filtered turn set that feeds the charts and KPIs. Splitting them would mean
building a table against unfiltered data now and retrofitting filtering later —
more churn than doing both at once.

## Non-goals (for this mini-project)

- ❌ Per-row drill-down / clicking a session to see its turns. YAGNI for v1.
- ❌ CSV/clipboard export of the table.
- ❌ Column resize persistence across launches.
- ❌ Touching the poll thread or calibration math.

## Background: how the dashboard renders today

The dashboard is a single `eframe::App` (`DashboardApp`) on one persistent
thread. Every frame it reads a cloned `AppSnapshot` from the shared lock and
renders a KPI strip followed by three charts inside one vertical
`ScrollArea`.

Key fact that shapes this design: **the charts already rebuild their series
from `snap.turns` every frame** (`cumulative_share_series_5h`,
`cumulative_share_series_weekly`, `daily_aggregates` all take `&[Turn]`). Only
the KPIs (`snap.kpis`), the hour-of-day overlay (`snap.hourly_5h/_week`), the
caps (`snap.caps`) and the live-util line are precomputed by the poll thread.

This means filtering is mostly a matter of feeding the existing renderers a
**different (filtered) `AppSnapshot`** — no change to chart-drawing code.

## Architecture

### Chosen approach: filtered view computed in the dashboard

The dashboard owns the filter state. Each frame, if the filter state or the
underlying turn data changed since last frame, it:

1. Applies the filters to `snap.turns` → `filtered_turns: Vec<Turn>`.
2. Recomputes KPIs from `filtered_turns` via the existing
   `snapshot::compute_kpis(&filtered_turns, &snap.caps)`.
3. Assembles a `FilteredView` — a clone of the incoming `AppSnapshot` with
   `turns` replaced by `Arc::new(filtered_turns)` and `kpis` replaced by the
   recomputed value. `caps`, `hourly_5h`, `hourly_week`, `live_util`,
   `last_sample`, `last_status` are copied through unchanged.

The existing chart renderers and the new sessions table both consume this
`FilteredView` (which *is* an `AppSnapshot`, so chart signatures are
unchanged).

**What filters do NOT affect** (and why):

- **`caps`** — the calibrated 5h / weekly caps come from the calibration log's
  anchors, independent of which projects/models/dates you're viewing. A cap is
  a property of the account, not of a filtered slice.
- **`hourly_5h` / `hourly_week`** — these are the hour-of-day *reference*
  overlay (typical burn by clock hour). They are a backdrop, not the filtered
  subject. Leaving them global keeps the overlay stable while you filter.

This matches the Python app, where the cap lines and the calibration curve are
computed globally and only the per-turn series + session table react to the
filter.

#### Rejected alternatives

- **Push filters down to the poll thread.** Filters are interactive
  (potentially changing every frame); the poll thread runs on a 60s timer.
  Coupling UI state into the poll loop mixes two unrelated clocks. Rejected.
- **Filter only the table, leave charts on the full dataset.** Violates the
  chosen whole-dashboard parity, and lets the charts and table silently
  disagree. Rejected.

### Memoization

Recomputing the filtered view every frame at 30 fps is wasteful, though not
catastrophic (the charts already do O(n) work per frame). The dashboard caches
the last `FilteredView` keyed by a cheap **signature**:

```
(FilterState, turns.len(), last_turn_ts)
```

- `FilterState` is `PartialEq` (`Eq`/`Hash` on its fields), so equality is a
  direct comparison.
- `turns.len()` + the timestamp of the last turn detect that the poll thread
  pushed new data (turns only ever get appended).

When the signature is unchanged, reuse the cached `FilteredView`. When it
changes, recompute and cache.

> **Rust beginner note:** the cache lives as `Option<(Signature, FilteredView)>`
> on `DashboardApp`. We compare the freshly-computed signature against the
> stored one; on mismatch we rebuild and overwrite. No interior mutability or
> `RefCell` needed — `update(&mut self, …)` already gives us `&mut` access to
> the app struct.

## Components

### 1. `config.rs` — model → context-window map

Port of Python `config.MODEL_CONTEXT_WINDOWS` + `context_window_for`.

```rust
/// Per-model context window in tokens. Prefix-matched against the model
/// string. Mirrors the Python config.py table verbatim.
pub const MODEL_CONTEXT_WINDOWS: &[(&str, u64)] = &[
    ("claude-opus-4-7", 1_000_000),
    ("claude-opus-4-6", 1_000_000),
    ("claude-sonnet-4-6", 1_000_000),
    ("claude-sonnet-4-5", 200_000),
    ("claude-sonnet-4", 200_000),
    ("claude-haiku-4-5", 200_000),
    ("claude-3-7-sonnet", 200_000),
    ("claude-3-5-sonnet", 200_000),
    ("claude-3-5-haiku", 200_000),
    ("claude-3-opus", 200_000),
];
pub const DEFAULT_CONTEXT_WINDOW: u64 = 200_000;

/// Prefix match (`model.starts_with(key)`), first hit wins; fallback to
/// DEFAULT_CONTEXT_WINDOW when empty or unmatched.
pub fn context_window_for(model: &str) -> u64 { /* … */ }
```

Order matters because of prefix matching: `claude-sonnet-4-6` must be checked
before `claude-sonnet-4` so the 1M entry wins. The slice is iterated top to
bottom, so list specific prefixes first. (This is why a `&[(…)]` slice is used
rather than a `HashMap` — insertion order is the match order.)

### 2. `data/sessions.rs` (new — pure, tested)

```rust
pub struct SessionSummary {
    pub session_id: String,
    pub start: DateTime<Utc>,        // min ts over main-thread turns
    pub end: DateTime<Utc>,          // max ts over main-thread turns
    pub project_cwd: String,         // last main-thread turn's cwd
    pub model: String,               // last main-thread turn's model
    pub main_turns: usize,
    pub subagent_count: usize,       // distinct subagent_id within session
    pub peak_context_pct: f64,       // max(prompt_tokens / context_window), main only
    pub peak_prompt_tokens: u64,     // max prompt_tokens, main only
    pub main_cost_weighted: f64,
    pub subagent_cost_weighted: f64,
    pub total_cost_weighted: f64,    // main + subagent
    pub duration_s: f64,             // (end - start), for the degenerate filter
}

/// prompt_tokens = input + cache_creation + cache_read (output excluded).
pub fn prompt_tokens(t: &Turn) -> u64;

/// Port of metrics.session_summaries. Groups by session_id.
pub fn session_summaries(turns: &[Turn]) -> Vec<SessionSummary>;
```

**Aggregation rules (exact parity with Python `metrics.session_summaries`):**

- Group all turns by `session_id`.
- **Main-thread rows only** (`!is_subagent`) drive: `start` (min ts), `end`
  (max ts), `project_cwd` (last), `model` (last), `main_turns` (count),
  `peak_prompt_tokens` (max), `peak_context_pct`
  (`max(prompt_tokens / context_window_for(model))`), `main_cost_weighted`
  (sum of `cost_weighted`).
- A session with **no main-thread rows is dropped** (Python returns empty for
  a main-less group).
- **Subagent rows** within the same `session_id` contribute
  `subagent_cost_weighted` (sum) and `subagent_count` (distinct `subagent_id`,
  ignoring `None`). Joined back onto the main aggregate; sessions with no
  subagents get 0 / 0.
- `total_cost_weighted = main_cost_weighted + subagent_cost_weighted`.

> **Subagent → session linkage:** this relies on subagent JSONL rows carrying
> the parent session's `sessionId`, exactly as the Python app assumes. The
> existing `parser.rs` already reads `sessionId` per row regardless of whether
> the file is a subagent transcript, so the linkage ports unchanged.

`cost_weighted` is the existing `snapshot::cost_weighted(&Turn)` — reused, not
reimplemented.

### 3. `dashboard/filters.rs` (new — pure, tested)

```rust
#[derive(Clone, PartialEq, Eq)]
pub struct FilterState {
    pub date_from: Option<NaiveDate>,   // inclusive, local date
    pub date_to: Option<NaiveDate>,     // inclusive, local date
    pub projects: BTreeSet<String>,     // empty = all; matches raw project_cwd
    pub models: BTreeSet<String>,       // empty = all; matches raw model
}

impl FilterState {
    /// Keep turns whose local-date is within [date_from, date_to] (when set)
    /// and whose project_cwd / model are in the selected sets (empty = all).
    pub fn apply(&self, turns: &[Turn]) -> Vec<Turn>;
}

/// Distinct project_cwd values present in `turns`, sorted by basename label.
pub fn distinct_projects(turns: &[Turn]) -> Vec<String>;
/// Distinct model values present in `turns`, sorted.
pub fn distinct_models(turns: &[Turn]) -> Vec<String>;

/// Basename for display: last path component of cwd, or "(unknown)" if empty.
/// Mirrors Python short_project().
pub fn short_project(cwd: &str) -> String;
```

Filter semantics:
- **Empty selection set = "all"** (no filtering on that dimension). This is the
  natural default and avoids a "nothing selected shows nothing" footgun.
- **Date bounds** are compared on the *local* date of each turn (consistent
  with how the daily chart buckets by local date). Either bound may be `None`
  (open-ended).
- `FilterState` derives `Eq` so it plugs straight into the memoization
  signature.

> **`BTreeSet` over `HashSet`:** gives deterministic iteration (stable filter
> bar ordering) and `Eq`/`Ord` for free. The sets are tiny (handful of
> projects/models), so ordered-set cost is irrelevant.

### 4. `dashboard/sessions_table.rs` (new — render)

Renders with `egui_extras::TableBuilder`. Ten columns, parity order:

| Column | Source | Format |
|---|---|---|
| start | `start` | local datetime, `YYYY-MM-DD HH:MM` |
| project | `short_project(project_cwd)` | basename |
| model | `model` | as-is |
| main_turns | `main_turns` | integer |
| subagents | `subagent_count` | integer |
| peak ctx% | `peak_context_pct * 100` | 1 dp, e.g. `87.3` |
| peak prompt | `peak_prompt_tokens` | integer (raw or SI — see open question) |
| main M | `main_cost_weighted / 1e6` | 2 dp |
| sub M | `subagent_cost_weighted / 1e6` | 2 dp |
| total M | `total_cost_weighted / 1e6` | 2 dp |

**Table-local controls** (above the table, parity with Python sidebar):
- **Sort selector** (`egui::ComboBox` or selectable labels): `Chronological`
  (start asc, default) · `Peak ctx%` (desc) · `Total cost` (desc).
- **Degenerate-session filter**: `min_turns` (`DragValue`, default 5) hides
  sessions with `main_turns < min_turns`; `min_duration_s` (`DragValue`,
  default 60) hides sessions with `duration_s < min_duration_s`. A caption
  shows "*N degenerate session(s) hidden*".

> The sort + degenerate filter operate on the already-globally-filtered
> session list. They are table-presentation concerns, so they live with the
> table, not in `FilterState`.

The table gets its own scroll region (it's on its own tab — see below), so
nested-scrollbar issues do not arise.

### 5. `dashboard/filter_bar.rs` (new — render)

A horizontal bar rendered above the tab content (so it governs both tabs):

- **Date range**: two `egui_extras::DatePickerButton`s (from / to). A "Clear"
  button resets both to `None`.
- **Projects**: a multiselect — a `ComboBox`/menu listing
  `distinct_projects(all_turns)` by basename with checkboxes; toggling updates
  `FilterState.projects`.
- **Models**: same pattern over `distinct_models(all_turns)`.
- A summary label, e.g. "Showing 1,240 of 5,003 turns", for orientation.

Distinct project/model lists are derived from the **unfiltered** snapshot turns
(so de-selecting everything still lets you re-add options).

### 6. `dashboard/app.rs` — tabs + filter state + memoized view

- New `#[derive(PartialEq)] enum Tab { Charts, Sessions }`, default `Charts`.
- `DashboardApp` gains: `tab: Tab`, `filters: FilterState`, sort/degenerate
  state for the table, and the memoization cache
  `cached_view: Option<(Signature, AppSnapshot)>`.
- Render order when visible:
  1. `filter_bar::render(...)` (always visible).
  2. Tab strip (two `selectable_label`s).
  3. Compute/reuse the `FilteredView`.
  4. `Charts` tab → existing KPI strip + 3 charts, fed the filtered view.
     `Sessions` tab → `sessions_table::render(...)`.
- The off-screen-parking lifecycle (close = park, show, quit) is unchanged.

### 7. `Cargo.toml`

Add:
```toml
egui_extras = { version = "0.29", features = ["datepicker"] }
```
`datepicker` pulls `chrono` (already present). `egui_extras` provides
`TableBuilder`. Version pinned to 0.29 to match `egui`/`eframe`.

## Data flow (this mini-project)

```
AppSnapshot (full, from poll thread)
        │  snap.turns (all)
        ▼
filter_bar  ──sets──► FilterState ──┐
                                    ▼
            FilterState::apply(&snap.turns) ──► filtered_turns
                                    │
              compute_kpis(filtered_turns, snap.caps) ──► kpis'
                                    │
        FilteredView = snap with {turns: filtered, kpis: kpis'}
        (caps / hourly / live copied through)
                 │                               │
                 ▼                               ▼
        Charts tab (existing renderers)   Sessions tab
                                          session_summaries(filtered_turns)
                                            → sort → degenerate-hide → table
```

## Testing

`cargo test` (pure functions only — UI is smoke-tested by running):

- `context_window_for`: prefix match picks the longer prefix first
  (`claude-sonnet-4-6` → 1M, `claude-sonnet-4-5` → 200k); empty/unknown →
  200k default.
- `prompt_tokens`: sums the three input fields, excludes output.
- `session_summaries`:
  - groups by `session_id`; main-only rows set start/end/model/project;
  - `peak_context_pct` = max ratio across main turns using per-turn model
    window;
  - subagent rows add cost + distinct count, joined onto the main aggregate;
  - a session with only subagent rows is dropped;
  - `total = main + subagent`.
- `FilterState::apply`: date bounds (inclusive, local date), project set,
  model set, empty-set-means-all.
- sort ordering for each of the three modes.
- degenerate filtering by `min_turns` and `min_duration_s`.
- `short_project`: basename of a path, `(unknown)` for empty.

**Fixture:** add a JSONL under `tests/fixtures/` with ≥2 sessions, one of which
has a subagent transcript, covering: multiple models in one session, a
sub-minute degenerate session, and a session with no subagents.

## Out-of-scope follow-ups (future Stage 8 mini-projects)

- Calibration history expander (per-hour scatter + fitted curve).
- Live API status banner.
- Settings panel (cost-weight / weekly-reset overrides).

These will reuse the new tab strip introduced here.

## Open questions deferred to implementation

- Exact column widths / which columns are resizable vs fixed in
  `TableBuilder` — tune visually during implementation.
- Whether the "peak prompt" column shows raw integer or SI-abbreviated
  (`format_si`) — decide by eye; raw integers can be wide.
- Whether to persist filter + tab state across dashboard hide/show within a
  run (almost certainly yes, since `DashboardApp` is long-lived; it falls out
  for free as struct fields).
