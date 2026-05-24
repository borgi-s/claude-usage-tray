# Stage 8 (mini-project 1) — Sessions Table + Global Filters — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a full-parity sessions table to the egui dashboard plus global date/project/model filters that drive the whole dashboard, organized under a new Charts | Sessions tab strip.

**Architecture:** The poll thread and chart-drawing code are untouched. The dashboard owns a `FilterState`, applies it to `snap.turns` to build a *filtered* `AppSnapshot` (turns replaced, KPIs recomputed; caps/hourly/live copied through), memoized so it only recomputes when filters or turn data change. Both the existing charts and the new table consume that filtered snapshot. Session aggregation, filtering, sorting, and the context-window lookup are pure, unit-tested functions; the egui widgets are smoke-tested by running.

**Tech Stack:** Rust, `egui`/`eframe` 0.29, new `egui_extras` 0.29 (`TableBuilder` + `datepicker`), `chrono`/`chrono-tz`.

**Spec:** `docs/superpowers/specs/2026-05-24-stage-8-sessions-table-design.md`

**Rust-beginner reminders used throughout this plan:**
- `&[Turn]` is a *borrowed slice* — read-only view, no ownership transfer.
- A function returning `Vec<T>` *owns* and hands back a fresh allocation.
- `#[derive(PartialEq, Eq)]` auto-generates equality so a struct can be compared with `==` and used in the memo signature.
- Integration tests live in `tests/*.rs` and import the crate as `claude_usage_tray::...`. Inline unit tests live in a `#[cfg(test)] mod tests { … }` block at the bottom of the source file (the pattern in `src/data/parser.rs`).

---

## File structure

| File | Responsibility | New? |
|---|---|---|
| `Cargo.toml` | add `egui_extras` dependency | modify |
| `src/config.rs` | `MODEL_CONTEXT_WINDOWS` + `context_window_for` | modify |
| `src/data/sessions.rs` | `prompt_tokens`, `SessionSummary`, `session_summaries`, `SortKey`, `sort_sessions`, `hide_degenerate` (pure, tested) | create |
| `src/data/mod.rs` | register `pub mod sessions;` | modify |
| `src/dashboard/filters.rs` | `FilterState`, `apply`, `distinct_projects/models`, `short_project` (pure, tested) | create |
| `src/dashboard/sessions_table.rs` | `TableBuilder` render of the 10 columns + sort/degenerate controls | create |
| `src/dashboard/filter_bar.rs` | date pickers + project/model multiselect render | create |
| `src/dashboard/mod.rs` | register the three new modules | modify |
| `src/dashboard/app.rs` | tab strip, filter state, memoized filtered view, render dispatch | modify |
| `tests/sessions_fixture_test.rs` | parser → `session_summaries` end-to-end on a fixture | create |
| `tests/fixtures/sessions_multi.jsonl` | ≥2 sessions incl. a subagent | create |

---

## Task 1: Add the `egui_extras` dependency

**Files:**
- Modify: `Cargo.toml`

- [ ] **Step 1: Add the dependency**

In `Cargo.toml`, directly below the `egui_plot = "0.29"` line, add:

```toml
egui_extras = { version = "0.29", features = ["datepicker"] }
```

- [ ] **Step 2: Verify it resolves and builds**

Run: `cargo build`
Expected: compiles successfully (downloads `egui_extras` 0.29 and its `datepicker`/`chrono` deps). No code uses it yet — that's fine.

- [ ] **Step 3: Commit**

```bash
git add Cargo.toml Cargo.lock
git commit -m "chore(stage-8): add egui_extras (TableBuilder + datepicker)"
```

---

## Task 2: `config::context_window_for`

**Files:**
- Modify: `src/config.rs`

- [ ] **Step 1: Write the failing test**

Append to the bottom of `src/config.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn context_window_prefix_match_prefers_longer_prefix() {
        // sonnet-4-6 is 1M and must win over the sonnet-4 (200k) entry.
        assert_eq!(context_window_for("claude-sonnet-4-6-20260101"), 1_000_000);
        assert_eq!(context_window_for("claude-sonnet-4-5-20251101"), 200_000);
    }

    #[test]
    fn context_window_opus_is_one_million() {
        assert_eq!(context_window_for("claude-opus-4-7"), 1_000_000);
        assert_eq!(context_window_for("claude-opus-4-6"), 1_000_000);
    }

    #[test]
    fn context_window_unknown_and_empty_fall_back_to_default() {
        assert_eq!(context_window_for("gpt-9"), DEFAULT_CONTEXT_WINDOW);
        assert_eq!(context_window_for(""), DEFAULT_CONTEXT_WINDOW);
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --lib config::tests`
Expected: FAIL — `cannot find function context_window_for` / `cannot find value DEFAULT_CONTEXT_WINDOW`.

- [ ] **Step 3: Write the implementation**

Append to `src/config.rs` *above* the `#[cfg(test)]` block:

```rust
/// Per-model context window in tokens. Prefix-matched against the model
/// string; the FIRST matching entry wins, so more specific prefixes must come
/// before shorter ones (e.g. `claude-sonnet-4-6` before `claude-sonnet-4`).
/// Mirrors the Python project's `config.MODEL_CONTEXT_WINDOWS`.
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

/// Fallback context window for empty or unrecognized model strings.
pub const DEFAULT_CONTEXT_WINDOW: u64 = 200_000;

/// Returns the context window for `model` via prefix match, or the default.
pub fn context_window_for(model: &str) -> u64 {
    if model.is_empty() {
        return DEFAULT_CONTEXT_WINDOW;
    }
    for (prefix, window) in MODEL_CONTEXT_WINDOWS {
        if model.starts_with(prefix) {
            return *window;
        }
    }
    DEFAULT_CONTEXT_WINDOW
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test --lib config::tests`
Expected: PASS (3 tests).

- [ ] **Step 5: Commit**

```bash
git add src/config.rs
git commit -m "feat(stage-8): model->context-window lookup in config"
```

---

## Task 3: `data/sessions.rs` — `prompt_tokens` + module registration

**Files:**
- Create: `src/data/sessions.rs`
- Modify: `src/data/mod.rs`

- [ ] **Step 1: Register the module**

In `src/data/mod.rs`, add below `pub mod parser;`:

```rust
pub mod sessions;
```

- [ ] **Step 2: Create the file with a failing test**

Create `src/data/sessions.rs`:

```rust
//! Per-session aggregation for the dashboard's sessions table. Pure functions
//! ported from the Python `metrics.session_summaries`. No egui here.

use crate::data::parser::Turn;

/// Context tokens fed into the model for one turn:
/// input + cache_creation + cache_read. Output is excluded (it's the response,
/// not the context). Mirrors the Python `prompt_tokens` derivation.
pub fn prompt_tokens(t: &Turn) -> u64 {
    t.input_tokens + t.cache_creation_input_tokens + t.cache_read_input_tokens
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use std::path::PathBuf;

    fn turn(input: u64, cc: u64, cr: u64, output: u64) -> Turn {
        Turn {
            ts: Utc::now(),
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
    fn prompt_tokens_sums_inputs_excludes_output() {
        // 100 + 200 + 300 = 600; output 999 ignored.
        let t = turn(100, 200, 300, 999);
        assert_eq!(prompt_tokens(&t), 600);
    }
}
```

- [ ] **Step 3: Run test to verify it passes**

Run: `cargo test --lib data::sessions`
Expected: PASS (1 test). (The function is trivial enough to write with its test; the next task is where TDD bites.)

- [ ] **Step 4: Commit**

```bash
git add src/data/mod.rs src/data/sessions.rs
git commit -m "feat(stage-8): sessions module + prompt_tokens helper"
```

---

## Task 4: `session_summaries` aggregation

**Files:**
- Modify: `src/data/sessions.rs`

- [ ] **Step 1: Write the failing tests**

In `src/data/sessions.rs`, extend the `tests` module with a richer builder and the aggregation tests. Add this `turn_full` helper and the tests inside `mod tests`:

```rust
    use chrono::{DateTime, TimeZone};

    fn utc(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(y, mo, d, h, mi, 0).unwrap()
    }

    /// Full turn builder for session tests.
    #[allow(clippy::too_many_arguments)]
    fn trow(
        session_id: &str,
        ts: DateTime<Utc>,
        is_subagent: bool,
        subagent_id: Option<&str>,
        project: &str,
        model: &str,
        input: u64,
        output: u64,
    ) -> Turn {
        Turn {
            ts,
            session_id: session_id.to_string(),
            subagent_id: subagent_id.map(|s| s.to_string()),
            is_subagent,
            project_cwd: project.to_string(),
            model: model.to_string(),
            version: String::new(),
            input_tokens: input,
            output_tokens: output,
            cache_creation_input_tokens: 0,
            cache_read_input_tokens: 0,
            source_file: PathBuf::new(),
            is_rate_limit_error: false,
        }
    }

    #[test]
    fn session_summaries_groups_and_aggregates_main_rows() {
        let turns = vec![
            trow("s1", utc(2026, 5, 24, 10, 0), false, None, "/home/u/proj", "claude-opus-4-7", 100, 10),
            trow("s1", utc(2026, 5, 24, 11, 0), false, None, "/home/u/proj", "claude-opus-4-7", 300, 20),
        ];
        let out = session_summaries(&turns);
        assert_eq!(out.len(), 1);
        let s = &out[0];
        assert_eq!(s.session_id, "s1");
        assert_eq!(s.start, utc(2026, 5, 24, 10, 0));
        assert_eq!(s.end, utc(2026, 5, 24, 11, 0));
        assert_eq!(s.main_turns, 2);
        assert_eq!(s.subagent_count, 0);
        assert_eq!(s.peak_prompt_tokens, 300); // max(100, 300)
        // peak ctx% = 300 / 1_000_000 (opus 4-7 window)
        assert!((s.peak_context_pct - 300.0 / 1_000_000.0).abs() < 1e-12);
        // cost_weighted: input weight 1.0, output weight 5.0 → 100+50 and 300+100
        assert!((s.main_cost_weighted - (150.0 + 400.0)).abs() < 1e-9);
        assert!((s.total_cost_weighted - s.main_cost_weighted).abs() < 1e-9);
    }

    #[test]
    fn session_summaries_joins_subagents_by_distinct_id() {
        let turns = vec![
            trow("s1", utc(2026, 5, 24, 10, 0), false, None, "/p", "claude-sonnet-4-5", 100, 0),
            trow("s1", utc(2026, 5, 24, 10, 5), true, Some("a1"), "/p", "claude-sonnet-4-5", 0, 50),
            trow("s1", utc(2026, 5, 24, 10, 6), true, Some("a1"), "/p", "claude-sonnet-4-5", 0, 50),
            trow("s1", utc(2026, 5, 24, 10, 7), true, Some("a2"), "/p", "claude-sonnet-4-5", 0, 50),
        ];
        let out = session_summaries(&turns);
        assert_eq!(out.len(), 1);
        let s = &out[0];
        assert_eq!(s.main_turns, 1);
        assert_eq!(s.subagent_count, 2); // a1, a2 distinct
        // subagent cost = 3 rows * output 50 * weight 5.0 = 750
        assert!((s.subagent_cost_weighted - 750.0).abs() < 1e-9);
        // total = main (100*1.0) + sub (750)
        assert!((s.total_cost_weighted - (100.0 + 750.0)).abs() < 1e-9);
    }

    #[test]
    fn session_summaries_drops_session_with_only_subagent_rows() {
        let turns = vec![
            trow("s1", utc(2026, 5, 24, 10, 0), true, Some("a1"), "/p", "m", 0, 50),
        ];
        assert!(session_summaries(&turns).is_empty());
    }

    #[test]
    fn session_summaries_last_main_sets_project_and_model() {
        let turns = vec![
            trow("s1", utc(2026, 5, 24, 10, 0), false, None, "/old", "claude-haiku-4-5", 10, 0),
            trow("s1", utc(2026, 5, 24, 12, 0), false, None, "/new", "claude-opus-4-7", 10, 0),
        ];
        let s = &session_summaries(&turns)[0];
        assert_eq!(s.project_cwd, "/new");
        assert_eq!(s.model, "claude-opus-4-7");
    }

    #[test]
    fn session_summaries_sorted_by_start_ascending() {
        let turns = vec![
            trow("late", utc(2026, 5, 24, 15, 0), false, None, "/p", "m", 1, 0),
            trow("early", utc(2026, 5, 24, 9, 0), false, None, "/p", "m", 1, 0),
        ];
        let out = session_summaries(&turns);
        assert_eq!(out[0].session_id, "early");
        assert_eq!(out[1].session_id, "late");
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --lib data::sessions`
Expected: FAIL — `cannot find type SessionSummary` / `cannot find function session_summaries`.

- [ ] **Step 3: Write the implementation**

In `src/data/sessions.rs`, add above the `#[cfg(test)]` block:

```rust
use crate::shared::snapshot::cost_weighted;
use chrono::{DateTime, Utc};
use std::collections::{BTreeSet, HashMap};

/// One aggregated row in the sessions table. Mirrors the Python
/// `metrics.session_summaries` output schema.
#[derive(Debug, Clone)]
pub struct SessionSummary {
    pub session_id: String,
    /// Min/max timestamp over MAIN-thread turns only.
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    /// Project + model from the last (latest-ts) main-thread turn.
    pub project_cwd: String,
    pub model: String,
    pub main_turns: usize,
    /// Count of distinct `subagent_id` within this session.
    pub subagent_count: usize,
    /// max(prompt_tokens / context_window_for(model)) over main turns.
    pub peak_context_pct: f64,
    pub peak_prompt_tokens: u64,
    pub main_cost_weighted: f64,
    pub subagent_cost_weighted: f64,
    pub total_cost_weighted: f64,
    /// (end - start) in seconds; used by the degenerate-session filter.
    pub duration_s: f64,
}

/// Group `turns` by `session_id` and aggregate. Main-thread rows drive
/// start/end/model/project/peak/main-cost; subagent rows (same session_id)
/// contribute cost + distinct count. Sessions with no main-thread rows are
/// dropped. Output is sorted by `start` ascending.
pub fn session_summaries(turns: &[Turn]) -> Vec<SessionSummary> {
    let mut groups: HashMap<&str, Vec<&Turn>> = HashMap::new();
    for t in turns {
        groups.entry(t.session_id.as_str()).or_default().push(t);
    }

    let mut out: Vec<SessionSummary> = Vec::new();
    for (sid, rows) in groups {
        let mains: Vec<&Turn> = rows.iter().copied().filter(|t| !t.is_subagent).collect();
        if mains.is_empty() {
            continue; // a session with only subagent rows is dropped
        }

        let start = mains.iter().map(|t| t.ts).min().unwrap();
        let end = mains.iter().map(|t| t.ts).max().unwrap();
        let last_main = mains.iter().max_by_key(|t| t.ts).unwrap();
        let project_cwd = last_main.project_cwd.clone();
        let model = last_main.model.clone();
        let main_turns = mains.len();
        let peak_prompt_tokens = mains.iter().map(|t| prompt_tokens(t)).max().unwrap_or(0);
        let peak_context_pct = mains
            .iter()
            .map(|t| {
                let window = crate::config::context_window_for(&t.model);
                prompt_tokens(t) as f64 / window as f64
            })
            .fold(0.0_f64, f64::max);
        let main_cost_weighted: f64 = mains.iter().map(|t| cost_weighted(t)).sum();

        let subs: Vec<&Turn> = rows.iter().copied().filter(|t| t.is_subagent).collect();
        let subagent_cost_weighted: f64 = subs.iter().map(|t| cost_weighted(t)).sum();
        let mut sub_ids: BTreeSet<&str> = BTreeSet::new();
        for t in &subs {
            if let Some(id) = &t.subagent_id {
                sub_ids.insert(id.as_str());
            }
        }
        let subagent_count = sub_ids.len();

        let total_cost_weighted = main_cost_weighted + subagent_cost_weighted;
        let duration_s = (end - start).num_milliseconds() as f64 / 1000.0;

        out.push(SessionSummary {
            session_id: sid.to_string(),
            start,
            end,
            project_cwd,
            model,
            main_turns,
            subagent_count,
            peak_context_pct,
            peak_prompt_tokens,
            main_cost_weighted,
            subagent_cost_weighted,
            total_cost_weighted,
            duration_s,
        });
    }

    out.sort_by_key(|s| s.start);
    out
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --lib data::sessions`
Expected: PASS (6 tests total in the module).

- [ ] **Step 5: Commit**

```bash
git add src/data/sessions.rs
git commit -m "feat(stage-8): session_summaries aggregation"
```

---

## Task 5: `SortKey`, `sort_sessions`, `hide_degenerate`

**Files:**
- Modify: `src/data/sessions.rs`

- [ ] **Step 1: Write the failing tests**

Add inside `mod tests` in `src/data/sessions.rs`:

```rust
    fn summary(id: &str, start: DateTime<Utc>, main_turns: usize, duration_s: f64, ctx: f64, total: f64) -> SessionSummary {
        SessionSummary {
            session_id: id.to_string(),
            start,
            end: start,
            project_cwd: String::new(),
            model: String::new(),
            main_turns,
            subagent_count: 0,
            peak_context_pct: ctx,
            peak_prompt_tokens: 0,
            main_cost_weighted: total,
            subagent_cost_weighted: 0.0,
            total_cost_weighted: total,
            duration_s,
        }
    }

    #[test]
    fn sort_sessions_peak_ctx_descending() {
        let mut v = vec![
            summary("lo", utc(2026, 5, 24, 9, 0), 10, 100.0, 0.1, 1.0),
            summary("hi", utc(2026, 5, 24, 8, 0), 10, 100.0, 0.9, 1.0),
        ];
        sort_sessions(&mut v, SortKey::PeakCtx);
        assert_eq!(v[0].session_id, "hi");
    }

    #[test]
    fn sort_sessions_total_cost_descending() {
        let mut v = vec![
            summary("cheap", utc(2026, 5, 24, 9, 0), 10, 100.0, 0.1, 1.0),
            summary("pricey", utc(2026, 5, 24, 8, 0), 10, 100.0, 0.1, 9.0),
        ];
        sort_sessions(&mut v, SortKey::TotalCost);
        assert_eq!(v[0].session_id, "pricey");
    }

    #[test]
    fn sort_sessions_chronological_ascending() {
        let mut v = vec![
            summary("b", utc(2026, 5, 24, 15, 0), 10, 100.0, 0.1, 1.0),
            summary("a", utc(2026, 5, 24, 9, 0), 10, 100.0, 0.1, 1.0),
        ];
        sort_sessions(&mut v, SortKey::Chronological);
        assert_eq!(v[0].session_id, "a");
    }

    #[test]
    fn hide_degenerate_drops_short_and_low_turn_sessions() {
        let v = vec![
            summary("keep", utc(2026, 5, 24, 9, 0), 10, 120.0, 0.1, 1.0),
            summary("few_turns", utc(2026, 5, 24, 9, 0), 2, 120.0, 0.1, 1.0),
            summary("too_short", utc(2026, 5, 24, 9, 0), 10, 30.0, 0.1, 1.0),
        ];
        let (kept, hidden) = hide_degenerate(v, 5, 60.0);
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].session_id, "keep");
        assert_eq!(hidden, 2);
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --lib data::sessions`
Expected: FAIL — `cannot find type SortKey` / functions undefined.

- [ ] **Step 3: Write the implementation**

Add above the `#[cfg(test)]` block in `src/data/sessions.rs`:

```rust
use std::cmp::Ordering;

/// How the sessions table is ordered. Selected in the table's sort control.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortKey {
    Chronological, // by start ascending
    PeakCtx,       // by peak_context_pct descending
    TotalCost,     // by total_cost_weighted descending
}

/// Sort `sessions` in place per `key`. Descending modes use a NaN-safe
/// comparison (NaN never appears here, but partial_cmp must be unwrapped).
pub fn sort_sessions(sessions: &mut [SessionSummary], key: SortKey) {
    match key {
        SortKey::Chronological => sessions.sort_by(|a, b| a.start.cmp(&b.start)),
        SortKey::PeakCtx => sessions.sort_by(|a, b| {
            b.peak_context_pct
                .partial_cmp(&a.peak_context_pct)
                .unwrap_or(Ordering::Equal)
        }),
        SortKey::TotalCost => sessions.sort_by(|a, b| {
            b.total_cost_weighted
                .partial_cmp(&a.total_cost_weighted)
                .unwrap_or(Ordering::Equal)
        }),
    }
}

/// Drop "degenerate" sessions (fewer than `min_turns` main turns OR shorter
/// than `min_duration_s`). Returns `(kept, hidden_count)`.
pub fn hide_degenerate(
    sessions: Vec<SessionSummary>,
    min_turns: usize,
    min_duration_s: f64,
) -> (Vec<SessionSummary>, usize) {
    let total = sessions.len();
    let kept: Vec<SessionSummary> = sessions
        .into_iter()
        .filter(|s| s.main_turns >= min_turns && s.duration_s >= min_duration_s)
        .collect();
    let hidden = total - kept.len();
    (kept, hidden)
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --lib data::sessions`
Expected: PASS (10 tests total in the module).

- [ ] **Step 5: Commit**

```bash
git add src/data/sessions.rs
git commit -m "feat(stage-8): session sort + degenerate filter helpers"
```

---

## Task 6: `dashboard/filters.rs` — `short_project` + module registration

**Files:**
- Create: `src/dashboard/filters.rs`
- Modify: `src/dashboard/mod.rs`

- [ ] **Step 1: Register the module**

In `src/dashboard/mod.rs`, add to the `pub mod` list (alphabetical, after `pub mod bands;`):

```rust
pub mod filters;
```

- [ ] **Step 2: Create the file with failing tests**

Create `src/dashboard/filters.rs`:

```rust
//! Global dashboard filters (date / project / model) + display helpers. Pure
//! logic; the rendering of the filter bar lives in `filter_bar.rs`.

use crate::data::parser::Turn;
use std::collections::BTreeSet;

/// Display label for a project cwd: its last path component, or "(unknown)"
/// for an empty cwd. Mirrors the Python `short_project`.
pub fn short_project(cwd: &str) -> String {
    if cwd.is_empty() {
        return "(unknown)".to_string();
    }
    let trimmed = cwd.trim_end_matches(['/', '\\']);
    let base = trimmed.rsplit(['/', '\\']).next().unwrap_or(trimmed);
    if base.is_empty() {
        cwd.to_string()
    } else {
        base.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_project_basename_unix_and_windows() {
        assert_eq!(short_project("/home/u/myproj"), "myproj");
        assert_eq!(short_project(r"C:\Users\u\widget\"), "widget");
    }

    #[test]
    fn short_project_empty_is_unknown() {
        assert_eq!(short_project(""), "(unknown)");
    }

    #[test]
    fn short_project_root_returns_input() {
        assert_eq!(short_project("/"), "/");
    }
}
```

- [ ] **Step 3: Run tests to verify they pass**

Run: `cargo test --lib dashboard::filters`
Expected: PASS (3 tests).

> Note: the unused `Turn`/`BTreeSet` imports are added now because Task 7 uses
> them; if `cargo build` warns about unused imports here, that's expected and
> resolved by Task 7. To avoid a `-D warnings` failure between tasks, the
> imports are referenced by the tests-free build only after Task 7. **If you
> are committing strictly clean between tasks, delete the two `use` lines now
> and re-add them in Task 7.** (Recommended: keep them and proceed to Task 7
> before running clippy.)

- [ ] **Step 4: Commit**

```bash
git add src/dashboard/mod.rs src/dashboard/filters.rs
git commit -m "feat(stage-8): filters module + short_project helper"
```

---

## Task 7: `FilterState` + `apply` + distinct lists

**Files:**
- Modify: `src/dashboard/filters.rs`

- [ ] **Step 1: Write the failing tests**

Add inside `mod tests` in `src/dashboard/filters.rs`:

```rust
    use chrono::{DateTime, NaiveDate, TimeZone, Utc};
    use std::path::PathBuf;

    fn turn_at(session: &str, ts: DateTime<Utc>, project: &str, model: &str) -> Turn {
        Turn {
            ts,
            session_id: session.to_string(),
            subagent_id: None,
            is_subagent: false,
            project_cwd: project.to_string(),
            model: model.to_string(),
            version: String::new(),
            input_tokens: 0,
            output_tokens: 0,
            cache_creation_input_tokens: 0,
            cache_read_input_tokens: 0,
            source_file: PathBuf::new(),
            is_rate_limit_error: false,
        }
    }

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    fn sample() -> Vec<Turn> {
        vec![
            turn_at("a", Utc.with_ymd_and_hms(2026, 5, 20, 12, 0, 0).unwrap(), "/p/alpha", "claude-opus-4-7"),
            turn_at("b", Utc.with_ymd_and_hms(2026, 5, 22, 12, 0, 0).unwrap(), "/p/beta", "claude-sonnet-4-5"),
            turn_at("c", Utc.with_ymd_and_hms(2026, 5, 24, 12, 0, 0).unwrap(), "/p/alpha", "claude-sonnet-4-5"),
        ]
    }

    #[test]
    fn apply_empty_filter_keeps_all() {
        let turns = sample();
        let f = FilterState::default();
        assert_eq!(f.apply(&turns).len(), 3);
    }

    #[test]
    fn apply_date_bounds_inclusive_local() {
        let turns = sample();
        let mut f = FilterState::default();
        f.use_date_from = true;
        f.date_from = d(2026, 5, 22);
        f.use_date_to = true;
        f.date_to = d(2026, 5, 24);
        // keeps 22nd and 24th, drops 20th.
        assert_eq!(f.apply(&turns).len(), 2);
    }

    #[test]
    fn apply_project_set_filters() {
        let turns = sample();
        let mut f = FilterState::default();
        f.projects.insert("/p/alpha".to_string());
        let kept = f.apply(&turns);
        assert_eq!(kept.len(), 2);
        assert!(kept.iter().all(|t| t.project_cwd == "/p/alpha"));
    }

    #[test]
    fn apply_model_set_filters() {
        let turns = sample();
        let mut f = FilterState::default();
        f.models.insert("claude-sonnet-4-5".to_string());
        assert_eq!(f.apply(&turns).len(), 2);
    }

    #[test]
    fn distinct_projects_and_models_dedup() {
        let turns = sample();
        assert_eq!(distinct_projects(&turns), vec!["/p/alpha".to_string(), "/p/beta".to_string()]);
        assert_eq!(
            distinct_models(&turns),
            vec!["claude-opus-4-7".to_string(), "claude-sonnet-4-5".to_string()]
        );
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --lib dashboard::filters`
Expected: FAIL — `cannot find type FilterState` / functions undefined.

- [ ] **Step 3: Write the implementation**

Add above the `#[cfg(test)]` block in `src/dashboard/filters.rs` (the `use` lines from Task 6 stay; add the chrono imports):

```rust
use chrono::{NaiveDate, TimeZone};
use chrono_tz::Tz;

/// Global filter applied to the whole dashboard. Empty project/model sets mean
/// "all". Date bounds are gated by their `use_*` flag so the date pickers can
/// hold a buffered date even while the bound is inactive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilterState {
    pub use_date_from: bool,
    pub date_from: NaiveDate,
    pub use_date_to: bool,
    pub date_to: NaiveDate,
    pub projects: BTreeSet<String>,
    pub models: BTreeSet<String>,
}

impl Default for FilterState {
    fn default() -> Self {
        // Buffer the date pickers at "today" (local) but inactive by default.
        let today = chrono::Local::now().date_naive();
        Self {
            use_date_from: false,
            date_from: today,
            use_date_to: false,
            date_to: today,
            projects: BTreeSet::new(),
            models: BTreeSet::new(),
        }
    }
}

impl FilterState {
    /// Returns the subset of `turns` matching every active filter dimension.
    pub fn apply(&self, turns: &[Turn]) -> Vec<Turn> {
        let tz: Tz = crate::config::LOCAL_TZ.parse().expect("LOCAL_TZ");
        turns
            .iter()
            .filter(|t| {
                let local_date = t.ts.with_timezone(&tz).date_naive();
                if self.use_date_from && local_date < self.date_from {
                    return false;
                }
                if self.use_date_to && local_date > self.date_to {
                    return false;
                }
                if !self.projects.is_empty() && !self.projects.contains(&t.project_cwd) {
                    return false;
                }
                if !self.models.is_empty() && !self.models.contains(&t.model) {
                    return false;
                }
                true
            })
            .cloned()
            .collect()
    }
}

/// Distinct project cwds present in `turns`, sorted by display label (basename).
pub fn distinct_projects(turns: &[Turn]) -> Vec<String> {
    let set: BTreeSet<&str> = turns.iter().map(|t| t.project_cwd.as_str()).collect();
    let mut v: Vec<String> = set.into_iter().map(|s| s.to_string()).collect();
    v.sort_by(|a, b| short_project(a).cmp(&short_project(b)));
    v
}

/// Distinct model strings present in `turns`, sorted lexically.
pub fn distinct_models(turns: &[Turn]) -> Vec<String> {
    let set: BTreeSet<&str> = turns.iter().map(|t| t.model.as_str()).collect();
    set.into_iter().map(|s| s.to_string()).collect()
}
```

> The `TimeZone` import is needed for `with_timezone`'s trait bound usage via
> `chrono_tz::Tz`. The unused-import warning from Task 6 is now resolved because
> `Turn` and `BTreeSet` are used by `apply`/`distinct_*`.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --lib dashboard::filters`
Expected: PASS (8 tests total in the module).

- [ ] **Step 5: Commit**

```bash
git add src/dashboard/filters.rs
git commit -m "feat(stage-8): FilterState apply + distinct project/model lists"
```

---

## Task 8: Fixture-based end-to-end test (parser → session_summaries)

**Files:**
- Create: `tests/fixtures/sessions_multi.jsonl`
- Create: `tests/sessions_fixture_test.rs`

- [ ] **Step 1: Create the fixture**

Create `tests/fixtures/sessions_multi.jsonl` (one JSON object per line — keep these exact lines):

```jsonl
{"timestamp":"2026-05-24T08:00:00Z","sessionId":"sess-A","cwd":"/home/u/proj-a","version":"1.0","type":"assistant","message":{"model":"claude-opus-4-7","usage":{"input_tokens":1000,"output_tokens":200,"cache_creation_input_tokens":0,"cache_read_input_tokens":500}}}
{"timestamp":"2026-05-24T08:05:00Z","sessionId":"sess-A","cwd":"/home/u/proj-a","version":"1.0","type":"assistant","message":{"model":"claude-opus-4-7","usage":{"input_tokens":2000,"output_tokens":300,"cache_creation_input_tokens":0,"cache_read_input_tokens":1000}}}
{"timestamp":"2026-05-24T09:00:00Z","sessionId":"sess-B","cwd":"/home/u/proj-b","version":"1.0","type":"assistant","message":{"model":"claude-sonnet-4-5","usage":{"input_tokens":50,"output_tokens":10,"cache_creation_input_tokens":0,"cache_read_input_tokens":0}}}
```

Create the subagent fixture line in a separate file under a `subagents` path so the parser classifies it as a subagent. Create `tests/fixtures/subagents/agent-deadbeef.jsonl`:

```jsonl
{"timestamp":"2026-05-24T08:06:00Z","sessionId":"sess-A","cwd":"/home/u/proj-a","version":"1.0","type":"assistant","message":{"model":"claude-opus-4-7","usage":{"input_tokens":100,"output_tokens":80,"cache_creation_input_tokens":0,"cache_read_input_tokens":0}}}
```

- [ ] **Step 2: Write the failing test**

Create `tests/sessions_fixture_test.rs`:

```rust
use claude_usage_tray::data::parser::iter_rows;
use claude_usage_tray::data::sessions::session_summaries;
use std::path::Path;

#[test]
fn fixture_sessions_aggregate_with_subagent() {
    let mut turns = Vec::new();
    turns.extend(iter_rows(Path::new("tests/fixtures/sessions_multi.jsonl")));
    turns.extend(iter_rows(Path::new(
        "tests/fixtures/subagents/agent-deadbeef.jsonl",
    )));

    let summaries = session_summaries(&turns);
    // sess-A (2 main + 1 subagent) and sess-B (1 main), sorted by start.
    assert_eq!(summaries.len(), 2);
    assert_eq!(summaries[0].session_id, "sess-A");
    assert_eq!(summaries[1].session_id, "sess-B");

    let a = &summaries[0];
    assert_eq!(a.main_turns, 2);
    assert_eq!(a.subagent_count, 1); // agent-deadbeef
    // peak_prompt_tokens = max(1000+0+500, 2000+0+1000) = 3000
    assert_eq!(a.peak_prompt_tokens, 3000);
    // opus-4-7 window 1_000_000 → peak ctx = 3000 / 1_000_000
    assert!((a.peak_context_pct - 3000.0 / 1_000_000.0).abs() < 1e-12);

    let b = &summaries[1];
    assert_eq!(b.main_turns, 1);
    assert_eq!(b.subagent_count, 0);
}
```

- [ ] **Step 3: Run test to verify it passes**

Run: `cargo test --test sessions_fixture_test`
Expected: PASS. (If it fails on `subagent_count`, confirm the subagent fixture path contains a `subagents` directory component — that's what `classify_subagent` keys on.)

- [ ] **Step 4: Commit**

```bash
git add tests/fixtures/sessions_multi.jsonl tests/fixtures/subagents/agent-deadbeef.jsonl tests/sessions_fixture_test.rs
git commit -m "test(stage-8): fixture covering multi-session + subagent aggregation"
```

---

## Task 9: `dashboard/sessions_table.rs` — table render

**Files:**
- Create: `src/dashboard/sessions_table.rs`
- Modify: `src/dashboard/mod.rs`

This task has no unit test — egui rendering is visual and smoke-tested by running the app in Task 11. Verification here is a clean build + clippy.

- [ ] **Step 1: Register the module**

In `src/dashboard/mod.rs`, add (after `pub mod range;`):

```rust
pub mod sessions_table;
```

- [ ] **Step 2: Create the render module**

Create `src/dashboard/sessions_table.rs`:

```rust
//! Sessions table tab: TableBuilder over per-session summaries, with a sort
//! selector and a degenerate-session filter. Pure aggregation lives in
//! `crate::data::sessions`.

use crate::data::parser::Turn;
use crate::data::sessions::{hide_degenerate, session_summaries, sort_sessions, SortKey};
use crate::dashboard::filters::short_project;
use crate::dashboard::kpi::format_si;
use chrono_tz::Tz;
use egui::Ui;
use egui_extras::{Column, TableBuilder};

/// Persistent table-local controls (live on DashboardApp).
pub struct TableControls {
    pub sort: SortKey,
    pub min_turns: usize,
    pub min_duration_s: f64,
}

impl Default for TableControls {
    fn default() -> Self {
        Self {
            sort: SortKey::Chronological,
            min_turns: 5,
            min_duration_s: 60.0,
        }
    }
}

pub fn render(ui: &mut Ui, turns: &[Turn], controls: &mut TableControls) {
    // Controls row.
    ui.horizontal(|ui| {
        ui.label("Sort:");
        egui::ComboBox::from_id_source("sessions_sort")
            .selected_text(sort_label(controls.sort))
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut controls.sort, SortKey::Chronological, "Chronological");
                ui.selectable_value(&mut controls.sort, SortKey::PeakCtx, "Peak ctx%");
                ui.selectable_value(&mut controls.sort, SortKey::TotalCost, "Total cost");
            });
        ui.separator();
        ui.label("Min turns:");
        ui.add(egui::DragValue::new(&mut controls.min_turns).range(0..=1000));
        ui.label("Min duration (s):");
        ui.add(egui::DragValue::new(&mut controls.min_duration_s).range(0.0..=86_400.0));
    });

    let mut summaries = session_summaries(turns);
    sort_sessions(&mut summaries, controls.sort);
    let (summaries, hidden) = hide_degenerate(summaries, controls.min_turns, controls.min_duration_s);

    ui.label(
        egui::RichText::new(format!(
            "{} session(s) · {} degenerate hidden",
            summaries.len(),
            hidden
        ))
        .size(11.0)
        .color(egui::Color32::GRAY),
    );
    ui.add_space(4.0);

    let tz: Tz = crate::config::LOCAL_TZ.parse().expect("LOCAL_TZ");

    TableBuilder::new(ui)
        .striped(true)
        .resizable(true)
        .column(Column::initial(140.0)) // start
        .column(Column::initial(120.0)) // project
        .column(Column::initial(150.0)) // model
        .column(Column::initial(70.0)) // main_turns
        .column(Column::initial(70.0)) // subagents
        .column(Column::initial(80.0)) // peak ctx%
        .column(Column::initial(90.0)) // peak prompt
        .column(Column::initial(70.0)) // main M
        .column(Column::initial(70.0)) // sub M
        .column(Column::remainder()) // total M
        .header(20.0, |mut header| {
            for title in [
                "start", "project", "model", "main", "subs", "ctx%", "peak prompt", "main M",
                "sub M", "total M",
            ] {
                header.col(|ui| {
                    ui.strong(title);
                });
            }
        })
        .body(|mut body| {
            for s in &summaries {
                body.row(18.0, |mut row| {
                    row.col(|ui| {
                        ui.label(s.start.with_timezone(&tz).format("%Y-%m-%d %H:%M").to_string());
                    });
                    row.col(|ui| {
                        ui.label(short_project(&s.project_cwd));
                    });
                    row.col(|ui| {
                        ui.label(&s.model);
                    });
                    row.col(|ui| {
                        ui.label(s.main_turns.to_string());
                    });
                    row.col(|ui| {
                        ui.label(s.subagent_count.to_string());
                    });
                    row.col(|ui| {
                        ui.label(format!("{:.1}", s.peak_context_pct * 100.0));
                    });
                    row.col(|ui| {
                        ui.label(format_si(s.peak_prompt_tokens as f64));
                    });
                    row.col(|ui| {
                        ui.label(format!("{:.2}", s.main_cost_weighted / 1e6));
                    });
                    row.col(|ui| {
                        ui.label(format!("{:.2}", s.subagent_cost_weighted / 1e6));
                    });
                    row.col(|ui| {
                        ui.label(format!("{:.2}", s.total_cost_weighted / 1e6));
                    });
                });
            }
        });
}

fn sort_label(key: SortKey) -> &'static str {
    match key {
        SortKey::Chronological => "Chronological",
        SortKey::PeakCtx => "Peak ctx%",
        SortKey::TotalCost => "Total cost",
    }
}
```

- [ ] **Step 3: Verify clean build + clippy**

Run: `cargo build`
Expected: compiles. (The `render`/`TableControls` are `pub`, so no dead-code warning despite not being called yet.)

Run: `cargo clippy --all-targets -- -D warnings`
Expected: no warnings.

> **egui 0.29 API note:** if clippy reports `ComboBox::from_id_source` as
> deprecated (it was renamed to `from_id_salt` in this era), swap the call to
> `egui::ComboBox::from_id_salt("sessions_sort")`. Same signature. Re-run
> clippy until clean — `-D warnings` treats deprecation as an error.

- [ ] **Step 4: Commit**

```bash
git add src/dashboard/mod.rs src/dashboard/sessions_table.rs
git commit -m "feat(stage-8): sessions table render (TableBuilder)"
```

---

## Task 10: `dashboard/filter_bar.rs` — filter bar render

**Files:**
- Create: `src/dashboard/filter_bar.rs`
- Modify: `src/dashboard/mod.rs`

No unit test — visual, smoke-tested in Task 11.

- [ ] **Step 1: Register the module**

In `src/dashboard/mod.rs`, add (after `pub mod filters;`):

```rust
pub mod filter_bar;
```

- [ ] **Step 2: Create the render module**

Create `src/dashboard/filter_bar.rs`:

```rust
//! Global filter bar: date range pickers + project/model multiselect menus.
//! Mutates the shared `FilterState`. Distinct option lists come from the FULL
//! (unfiltered) turn set so de-selected options remain re-selectable.

use crate::data::parser::Turn;
use crate::dashboard::filters::{distinct_models, distinct_projects, short_project, FilterState};
use egui::Ui;
use egui_extras::DatePickerButton;

pub fn render(ui: &mut Ui, all_turns: &[Turn], filter: &mut FilterState, shown: usize, total: usize) {
    ui.horizontal_wrapped(|ui| {
        // Date from.
        ui.checkbox(&mut filter.use_date_from, "From");
        ui.add_enabled(
            filter.use_date_from,
            DatePickerButton::new(&mut filter.date_from).id_source("filter_from"),
        );
        ui.separator();
        // Date to.
        ui.checkbox(&mut filter.use_date_to, "To");
        ui.add_enabled(
            filter.use_date_to,
            DatePickerButton::new(&mut filter.date_to).id_source("filter_to"),
        );
        ui.separator();

        // Project multiselect.
        let projects = distinct_projects(all_turns);
        ui.menu_button(project_button_label(filter), |ui| {
            for p in &projects {
                let mut checked = filter.projects.contains(p);
                if ui.checkbox(&mut checked, short_project(p)).changed() {
                    if checked {
                        filter.projects.insert(p.clone());
                    } else {
                        filter.projects.remove(p);
                    }
                }
            }
            if ui.button("Clear").clicked() {
                filter.projects.clear();
            }
        });

        // Model multiselect.
        let models = distinct_models(all_turns);
        ui.menu_button(model_button_label(filter), |ui| {
            for m in &models {
                let mut checked = filter.models.contains(m);
                if ui.checkbox(&mut checked, m).changed() {
                    if checked {
                        filter.models.insert(m.clone());
                    } else {
                        filter.models.remove(m);
                    }
                }
            }
            if ui.button("Clear").clicked() {
                filter.models.clear();
            }
        });

        ui.separator();
        ui.label(
            egui::RichText::new(format!("Showing {shown} of {total} turns"))
                .size(11.0)
                .color(egui::Color32::GRAY),
        );
    });
}

fn project_button_label(filter: &FilterState) -> String {
    if filter.projects.is_empty() {
        "Projects: all".to_string()
    } else {
        format!("Projects: {}", filter.projects.len())
    }
}

fn model_button_label(filter: &FilterState) -> String {
    if filter.models.is_empty() {
        "Models: all".to_string()
    } else {
        format!("Models: {}", filter.models.len())
    }
}
```

- [ ] **Step 3: Verify clean build + clippy**

Run: `cargo build`
Expected: compiles.

Run: `cargo clippy --all-targets -- -D warnings`
Expected: no warnings.

> If `DatePickerButton::id_source` is reported as not found in egui_extras
> 0.29, drop the `.id_source(...)` call — it's optional; the two pickers will
> still get distinct IDs from their differing `&mut` targets. Re-run clippy.

- [ ] **Step 4: Commit**

```bash
git add src/dashboard/mod.rs src/dashboard/filter_bar.rs
git commit -m "feat(stage-8): global filter bar render"
```

---

## Task 11: Wire tabs + filter state + memoized filtered view into `app.rs`

**Files:**
- Modify: `src/dashboard/app.rs`

No unit test — this is UI integration, verified by building and running the app.

- [ ] **Step 1: Add imports and new fields**

In `src/dashboard/app.rs`, update the top `use` block to add:

```rust
use crate::dashboard::filters::FilterState;
use crate::dashboard::sessions_table::TableControls;
use crate::shared::snapshot::{compute_kpis, AppSnapshot};
use chrono::{DateTime, Utc};
```

Add this enum above `pub struct DashboardApp`:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tab {
    Charts,
    Sessions,
}

/// Cheap signature for the filtered-view memo. Equal signature ⇒ reuse cache.
#[derive(Clone, PartialEq)]
struct ViewSig {
    filter: FilterState,
    n_turns: usize,
    last_ts: Option<DateTime<Utc>>,
}
```

Extend `DashboardApp` with these fields (add to the struct definition):

```rust
    tab: Tab,
    filters: FilterState,
    table_controls: TableControls,
    cached_view: Option<(ViewSig, AppSnapshot)>,
```

- [ ] **Step 2: Initialize the new fields in `new`**

In `DashboardApp::new`, add to the returned `Self { … }`:

```rust
            tab: Tab::Charts,
            filters: FilterState::default(),
            table_controls: TableControls::default(),
            cached_view: None,
```

- [ ] **Step 3: Add the memoized filtered-view helper**

Add this method inside `impl DashboardApp` (after `new`):

```rust
    /// Build (or reuse) the filtered AppSnapshot: turns filtered + KPIs
    /// recomputed; caps/hourly/live copied through unchanged. Memoized on the
    /// filter state and the turn vector's length+last-timestamp.
    fn filtered_view(&mut self, snap: &AppSnapshot) -> AppSnapshot {
        let sig = ViewSig {
            filter: self.filters.clone(),
            n_turns: snap.turns.len(),
            last_ts: snap.turns.last().map(|t| t.ts),
        };
        if let Some((cached_sig, view)) = &self.cached_view {
            if *cached_sig == sig {
                return view.clone();
            }
        }
        let filtered = self.filters.apply(&snap.turns);
        let kpis = compute_kpis(&filtered, &snap.caps);
        let mut view = snap.clone();
        view.turns = std::sync::Arc::new(filtered);
        view.kpis = kpis;
        self.cached_view = Some((sig, view.clone()));
        view
    }
```

- [ ] **Step 4: Replace the visible-render block (step 5 of `update`)**

Replace the entire `egui::CentralPanel::default().show(ctx, |ui| { … });` block (the one that currently renders KPIs + charts) with:

```rust
        // 5. Visible: filter bar + tab strip + tab content.
        let snap = self.shared.read().unwrap().clone();
        let all_turns = snap.turns.clone();
        let view = self.filtered_view(&snap);

        egui::TopBottomPanel::top("filter_bar_panel").show(ctx, |ui| {
            ui.add_space(4.0);
            crate::dashboard::filter_bar::render(
                ui,
                &all_turns,
                &mut self.filters,
                view.turns.len(),
                all_turns.len(),
            );
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.selectable_value(&mut self.tab, Tab::Charts, "Charts");
                ui.selectable_value(&mut self.tab, Tab::Sessions, "Sessions");
            });
        });

        egui::CentralPanel::default().show(ctx, |ui| match self.tab {
            Tab::Charts => {
                let caps_available = view.caps.cap_5h.is_some() || view.caps.cap_week.is_some();
                egui::ScrollArea::vertical().show(ui, |ui| {
                    ui.add_space(8.0);
                    crate::dashboard::kpi::render(ui, &view.kpis, caps_available);
                    ui.add_space(16.0);
                    if view.caps.cap_5h.is_none() && view.caps.cap_week.is_none() {
                        egui::Frame::default()
                            .fill(egui::Color32::from_rgb(60, 50, 30))
                            .inner_margin(egui::Margin::same(8.0))
                            .show(ui, |ui| {
                                ui.label(
                                    egui::RichText::new(
                                        "Uncalibrated — charts show raw output tokens until first ≥95% anchor is observed in the calibration log.",
                                    )
                                    .color(egui::Color32::from_rgb(220, 200, 120)),
                                );
                            });
                        ui.add_space(8.0);
                    }
                    ui.separator();
                    ui.add_space(8.0);
                    crate::dashboard::chart_5h::render(ui, &view, &mut self.range_5h);
                    ui.add_space(16.0);
                    ui.separator();
                    ui.add_space(8.0);
                    crate::dashboard::chart_weekly::render(ui, &view, &mut self.range_week);
                    ui.add_space(16.0);
                    ui.separator();
                    ui.add_space(8.0);
                    crate::dashboard::chart_daily::render(ui, &view, &mut self.range_daily);
                    ui.add_space(8.0);
                });
            }
            Tab::Sessions => {
                crate::dashboard::sessions_table::render(ui, &view.turns, &mut self.table_controls);
            }
        });
```

> **Why `TopBottomPanel` for the bar:** egui panels must be added before the
> `CentralPanel`. The filter bar + tab strip go in a top panel so the central
> area is exactly the tab content — and the `Sessions` tab's `TableBuilder`
> owns the full central height for its own scrolling (no nested ScrollArea).

- [ ] **Step 5: Verify clean build + clippy**

Run: `cargo build`
Expected: compiles.

Run: `cargo clippy --all-targets -- -D warnings`
Expected: no warnings.

- [ ] **Step 6: Manual smoke test**

Run: `cargo run -- --watch` (or however the tray is normally launched), then left-click the tray icon to open the dashboard.

Verify by observation:
- A filter bar with From/To date checkboxes+pickers, Projects and Models menu buttons, and a "Showing N of M turns" label appears at the top.
- Two tabs: **Charts** (KPI strip + 3 charts, unchanged) and **Sessions** (the table).
- The Sessions table shows the 10 columns and scrolls independently.
- Toggling a project/model filter or a date bound updates BOTH the charts and the table (turn count in the label changes).
- The sort combo and min-turns/min-duration controls reorder/shrink the table.

- [ ] **Step 7: Commit**

```bash
git add src/dashboard/app.rs
git commit -m "feat(stage-8): wire tabs + global filters + memoized filtered view"
```

---

## Task 12: Release prep

**Files:**
- Modify: `Cargo.toml`
- Modify: `CLAUDE.md`

- [ ] **Step 1: Full verification suite**

Run: `cargo fmt`
Run: `cargo fmt --check`
Expected: no diff.

Run: `cargo clippy --all-targets -- -D warnings`
Expected: no warnings.

Run: `cargo test`
Expected: all tests pass (including the new `data::sessions`, `dashboard::filters`, and `sessions_fixture_test`).

- [ ] **Step 2: Bump version**

In `Cargo.toml`, change:

```toml
version = "0.7.1"
```
to
```toml
version = "0.8.0"
```

- [ ] **Step 3: Update CLAUDE.md roadmap**

In `CLAUDE.md`, under "Active design + plans", add two pointers:

```markdown
- **Stage 8 (mini-project 1) spec:** `docs/superpowers/specs/2026-05-24-stage-8-sessions-table-design.md` — sessions table + global filters (date/project/model) that drive the whole dashboard; new Charts|Sessions tab strip.
- **Stage 8 (mini-project 1) plan:** `docs/superpowers/plans/2026-05-24-stage-8-sessions-table.md` — task plan. **Shipped 2026-05-24 (tag `v0.8.0`).**
```

In the Stage roadmap table, update the Stage 8 row's status to note mini-project 1 shipped (leave the rest of Stage 8 pending):

```markdown
| 8 | Streamlit feature parity (sessions table, filters, calibration history) | 🔶 In progress — mini-project 1 (sessions table + filters) shipped `v0.8.0` |
```

- [ ] **Step 4: Commit + verify the build one more time**

```bash
git add Cargo.toml Cargo.lock CLAUDE.md
git commit -m "chore: bump to v0.8.0 (Stage 8 mini-project 1 — sessions table + filters)"
```

Run: `cargo build --release`
Expected: release build succeeds.

- [ ] **Step 5: Tag (after manual confirmation)**

Per project convention, tagging is a deliberate, user-confirmed step. Once the dashboard has been manually verified (Task 11 Step 6), create the tag:

```bash
git tag v0.8.0
git push && git push --tags
```

---

## Self-review notes (for the implementer)

- **Spec coverage:** every spec section maps to a task — config map (T2), `prompt_tokens` (T3), `session_summaries` (T4), sort/degenerate (T5), `short_project` (T6), `FilterState`/distinct (T7), fixture (T8), table render (T9), filter bar (T10), tabs + memoized whole-dashboard filtering with caps/hourly held global (T11), dependency (T1), release (T12).
- **Implementation refinement vs spec:** the spec sketched `date_from: Option<NaiveDate>`; this plan uses `use_date_from: bool` + a buffered `NaiveDate` so `DatePickerButton` can bind directly to `&mut NaiveDate`. The `apply` semantics are identical (inactive bound = open-ended). Documented in Task 7.
- **`format_si` reuse:** the table's "peak prompt" column reuses `crate::dashboard::kpi::format_si` (already `pub`) rather than reimplementing SI formatting.
