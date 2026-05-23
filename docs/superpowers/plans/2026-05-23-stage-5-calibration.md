# Stage 5 — Calibration Math + Local Cache Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the tray tooltip's API-only utilization reading with a locally-computed alternative — built on top of an incremental mtime-diff JSONL cache and median-of-anchors cap derivation, with the calibrated 5h/7d util displayed alongside the API values.

**Architecture:** Add a `data/` module (parser + cache) and a `calibration/` module (anchors + hourly + live). The existing polling thread gains five new steps before its API fetch: refresh cache, read calibration log, derive global caps, compute hour-of-day cap series, compute live util. Results are stashed on `TrayState` and surfaced via two new lines in the tooltip. Icon rendering and threading stay untouched.

**Tech Stack:** Rust stable, `bincode` 1.x for the row cache, `chrono-tz` for local-tz Sunday-07:00 math, existing `serde`/`serde_json`/`chrono`/`tracing`/`anyhow`/`thiserror`.

**Reference spec:** `docs/superpowers/specs/2026-05-23-stage-5-calibration-design.md`

---

## File Structure

### Created
- `src/config.rs` — TZ + reset + anchor-util + 5h-window constants
- `src/data/mod.rs` — module index
- `src/data/parser.rs` — `Turn` type + `walk_jsonl` + `iter_rows`
- `src/data/cache.rs` — `refresh` + `CacheFile` + `Manifest`
- `src/calibration/mod.rs` — module index
- `src/calibration/anchors.rs` — `global_cap_from_anchors` + window math
- `src/calibration/hourly.rs` — 24-bin cap series
- `src/calibration/live.rs` — `live_util_now`
- `tests/fixtures/sample_session.jsonl` — anonymized parser fixture
- `tests/parser_test.rs` — fixture-driven parser tests
- `tests/cache_test.rs` — tmpdir-driven cache tests

### Modified
- `Cargo.toml` — add `bincode = "1.3"` and `chrono-tz = "0.9"`
- `src/lib.rs` — register `config`, `data`, `calibration` modules
- `src/paths.rs` — add `cache_path()` + `cache_manifest_path()`
- `src/log/calibration.rs` — add `read_all()` helper
- `src/tray/poller.rs` — refresh cache + compute caps before fetch; extend `PollEvent::Ok`
- `src/tray/window.rs` — `TrayState` gains `last_caps` / `last_local_util` / `last_hourly_*`; `drain_and_redraw` passes them to `format_tooltip`
- `src/render.rs` — (none) — *tooltip formatting lives in `tray::window`, see Task 25*

---

## Beginner notes (read first if you're new to Rust)

A few patterns you'll see repeatedly:

- **Module declaration:** A new directory `src/foo/` becomes a Rust module by adding `pub mod foo;` to `src/lib.rs` AND creating `src/foo/mod.rs` that declares its own children (`pub mod bar;`).
- **`#[cfg(test)]` blocks:** Inline unit tests for pure functions go at the bottom of the same file, inside `#[cfg(test)] mod tests { use super::*; ... }`. Integration tests that use the crate as a library go in `tests/foo_test.rs` and import via `use claude_usage_tray::foo::...`.
- **`bincode` 1.x basics:** `bincode::serialize(&value)` returns `Vec<u8>`; `bincode::deserialize::<T>(&bytes)` returns `Result<T, bincode::Error>`. No async, no derive of its own — relies on `serde`.
- **`chrono-tz` lookup:** `let tz: chrono_tz::Tz = "Europe/Copenhagen".parse().unwrap();` — parsing a static name string never fails at runtime, so `unwrap()` is fine here. To convert `DateTime<Utc>` to local: `utc.with_timezone(&tz)`.
- **Atomic file writes:** "Write temp + rename" pattern: `std::fs::write("file.bincode.tmp", bytes)?; std::fs::rename("file.bincode.tmp", "file.bincode")?;`. On Windows the rename is atomic at the filesystem level — a crash mid-write leaves the prior good file in place.
- **`SystemTime` → millis:** `t.duration_since(UNIX_EPOCH)?.as_millis() as i64`. We store mtimes as `i64` millis because `SystemTime` doesn't serialize portably and we don't need sub-millisecond precision for change detection.

If anything below uses an API you don't recognize, `cargo doc --open` after Task 1 has clickable links to the relevant crate docs.

---

## Task 1: Add `bincode` and `chrono-tz` to Cargo.toml

**Files:**
- Modify: `Cargo.toml`

- [ ] **Step 1: Read the current Cargo.toml**

Confirm `[dependencies]` ends with the `windows = { ... }` block and `[dev-dependencies]` has only `tempfile = "3"`.

- [ ] **Step 2: Edit `Cargo.toml`**

Add these two lines under `[dependencies]` (alphabetical order; place after `anyhow`):

```toml
bincode = "1.3"
chrono-tz = "0.9"
```

The full `[dependencies]` block now reads:

```toml
[dependencies]
ureq = { version = "2.10", features = ["json"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
chrono = { version = "0.4", features = ["serde"] }
chrono-tz = "0.9"
bincode = "1.3"
dirs = "5"
anyhow = "1"
thiserror = "1"
clap = { version = "4", features = ["derive"] }
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter", "fmt"] }
tracing-appender = "0.2"
windows = { version = "0.58", features = [
    "Win32_Foundation",
    "Win32_Graphics_Gdi",
    "Win32_Graphics_GdiPlus",
    "Win32_System_Console",
    "Win32_System_LibraryLoader",
    "Win32_UI_Shell",
    "Win32_UI_WindowsAndMessaging",
] }
```

- [ ] **Step 3: Verify it builds**

Run: `cargo build`
Expected: builds cleanly. First build with the new deps downloads `bincode`, `chrono-tz`, and their transitive deps — that's ~30 seconds the first time.

- [ ] **Step 4: Commit**

```bash
git add Cargo.toml Cargo.lock
git commit -m "chore(stage-5): add bincode and chrono-tz dependencies"
```

---

## Task 2: Create `src/config.rs` with calibration constants

**Files:**
- Create: `src/config.rs`
- Modify: `src/lib.rs`

- [ ] **Step 1: Create `src/config.rs`**

```rust
//! Calibration + display constants. Centralized so future overrides
//! (e.g., `~/.claude-usage-tray/config.toml`) have a single source of truth.

use chrono::Weekday;

/// IANA timezone name for local-time displays and weekly-reset math.
pub const LOCAL_TZ: &str = "Europe/Copenhagen";

/// Weekday on which Anthropic's weekly window resets (verified empirically).
pub const WEEKLY_RESET_WEEKDAY: Weekday = Weekday::Sun;

/// Hour (in local time) at which the weekly window resets. 07:00 local.
pub const WEEKLY_RESET_HOUR_LOCAL: u32 = 7;

/// Effective 5h burn-window length. Anthropic publishes 5h but observation
/// suggests the cap behaves like a ~4.5h window.
pub const FIVE_HOUR_WINDOW_HOURS: f64 = 4.5;

/// Minimum API utilization for a sample to be considered an anchor.
pub const MIN_ANCHOR_UTIL: f64 = 0.95;

/// Maximum API utilization for a sample to be considered an anchor.
/// Allows a small overshoot above 1.0 since the API can briefly report >100%.
pub const MAX_ANCHOR_UTIL: f64 = 1.01;
```

- [ ] **Step 2: Register the module in `src/lib.rs`**

Add `pub mod config;` to `src/lib.rs`. Place it alphabetically, just after `pub mod cli;`. The full `src/lib.rs` becomes:

```rust
pub mod api;
pub mod cli;
pub mod config;
pub mod log;
pub mod paths;
pub mod poll;
pub mod render;
pub mod tray;
pub mod watch;
```

- [ ] **Step 3: Verify it compiles**

Run: `cargo build`
Expected: builds cleanly.

- [ ] **Step 4: Commit**

```bash
git add src/config.rs src/lib.rs
git commit -m "feat(stage-5): add config module with calibration constants"
```

---

## Task 3: Add `cache_path` and `cache_manifest_path` helpers

**Files:**
- Modify: `src/paths.rs`

- [ ] **Step 1: Add the two helpers**

Append to `src/paths.rs`:

```rust
/// Returns ~/.claude-usage-tray/cache.bincode. Does NOT create the file.
pub fn cache_path() -> Result<PathBuf> {
    Ok(app_dir()?.join("cache.bincode"))
}

/// Returns ~/.claude-usage-tray/cache_manifest.json. Does NOT create the file.
pub fn cache_manifest_path() -> Result<PathBuf> {
    Ok(app_dir()?.join("cache_manifest.json"))
}
```

- [ ] **Step 2: Verify it compiles**

Run: `cargo build`
Expected: builds cleanly.

- [ ] **Step 3: Commit**

```bash
git add src/paths.rs
git commit -m "feat(stage-5): add cache_path and cache_manifest_path helpers"
```

---

## Task 4: Create `src/data/` module skeleton + `Turn` struct

**Files:**
- Create: `src/data/mod.rs`
- Create: `src/data/parser.rs`
- Modify: `src/lib.rs`

- [ ] **Step 1: Create `src/data/mod.rs`**

```rust
//! Local JSONL data layer — parser + mtime-diff cache.

pub mod parser;
```

- [ ] **Step 2: Create `src/data/parser.rs` with the `Turn` struct**

```rust
//! JSONL parser for `~/.claude/projects/**/*.jsonl`. One [`Turn`] per
//! assistant turn (or per rate-limit error row).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// One row per assistant turn. Mirrors the Python `parser.TurnRow`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Turn {
    pub ts: DateTime<Utc>,
    pub session_id: String,
    pub subagent_id: Option<String>,
    pub is_subagent: bool,
    pub project_cwd: String,
    pub model: String,
    pub version: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_creation_input_tokens: u64,
    pub cache_read_input_tokens: u64,
    pub source_file: PathBuf,
    pub is_rate_limit_error: bool,
}
```

- [ ] **Step 3: Register the module in `src/lib.rs`**

Add `pub mod data;` alphabetically:

```rust
pub mod api;
pub mod cli;
pub mod config;
pub mod data;
pub mod log;
pub mod paths;
pub mod poll;
pub mod render;
pub mod tray;
pub mod watch;
```

- [ ] **Step 4: Verify it compiles**

Run: `cargo build`
Expected: builds cleanly.

- [ ] **Step 5: Commit**

```bash
git add src/data/ src/lib.rs
git commit -m "feat(stage-5): scaffold data module and Turn struct"
```

---

## Task 5: Implement `classify_subagent` path helper with unit tests

**Files:**
- Modify: `src/data/parser.rs`

- [ ] **Step 1: Add a failing test at the bottom of `src/data/parser.rs`**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn classify_subagent_recognizes_subagent_path() {
        let p = Path::new("/home/u/.claude/projects/foo/subagents/agent-deadbeef.jsonl");
        let (is_sub, id) = classify_subagent(p);
        assert!(is_sub);
        assert_eq!(id, Some("deadbeef".to_string()));
    }

    #[test]
    fn classify_subagent_recognizes_windows_path() {
        let p = Path::new(r"C:\Users\u\.claude\projects\foo\subagents\agent-cafe1234.jsonl");
        let (is_sub, id) = classify_subagent(p);
        assert!(is_sub);
        assert_eq!(id, Some("cafe1234".to_string()));
    }

    #[test]
    fn classify_subagent_rejects_main_session_path() {
        let p = Path::new("/home/u/.claude/projects/foo/sess-1234.jsonl");
        let (is_sub, id) = classify_subagent(p);
        assert!(!is_sub);
        assert_eq!(id, None);
    }

    #[test]
    fn classify_subagent_rejects_subagents_dir_without_agent_prefix() {
        // Path is in subagents/ but the filename doesn't match agent-<hex>.jsonl.
        // We mark is_subagent=true (the path classification) but id=None.
        let p = Path::new("/home/u/subagents/garbage.jsonl");
        let (is_sub, id) = classify_subagent(p);
        assert!(is_sub);
        assert_eq!(id, None);
    }
}
```

- [ ] **Step 2: Verify the test fails**

Run: `cargo test --lib classify_subagent`
Expected: compilation errors — "cannot find function `classify_subagent` in this scope".

- [ ] **Step 3: Implement `classify_subagent`**

Add to `src/data/parser.rs` (above the `#[cfg(test)]` block):

```rust
/// Returns `(is_subagent, subagent_id)` for a JSONL file path.
///
/// `is_subagent` is true if any path component is literally "subagents".
/// `subagent_id` is `Some(hex)` only when the filename is `agent-<hex>.jsonl`.
pub fn classify_subagent(path: &std::path::Path) -> (bool, Option<String>) {
    let is_sub = path
        .components()
        .any(|c| c.as_os_str().eq_ignore_ascii_case("subagents"));
    if !is_sub {
        return (false, None);
    }
    let id = path
        .file_stem()
        .and_then(|s| s.to_str())
        .and_then(|s| s.strip_prefix("agent-"))
        .map(|s| s.to_string());
    (true, id)
}
```

- [ ] **Step 4: Verify the tests pass**

Run: `cargo test --lib classify_subagent`
Expected: 4 passed.

- [ ] **Step 5: Commit**

```bash
git add src/data/parser.rs
git commit -m "feat(stage-5): add classify_subagent path helper"
```

---

## Task 6: Implement `walk_jsonl` directory walker

**Files:**
- Modify: `src/data/parser.rs`

- [ ] **Step 1: Add a failing test**

Inside the existing `#[cfg(test)] mod tests` block, append:

```rust
    use tempfile::TempDir;

    fn touch(dir: &Path, rel: &str) {
        let p = dir.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, "").unwrap();
    }

    #[test]
    fn walk_jsonl_recurses_and_filters_by_extension() {
        let td = TempDir::new().unwrap();
        let root = td.path();
        touch(root, "sess-a.jsonl");
        touch(root, "proj1/sess-b.jsonl");
        touch(root, "proj1/subagents/agent-1.jsonl");
        touch(root, "proj1/notes.txt");    // should be filtered
        touch(root, "proj2/sub/sub/c.jsonl");

        let mut found: Vec<_> = walk_jsonl(root).collect();
        found.sort();

        assert_eq!(found.len(), 4);
        assert!(found.iter().all(|p| p.extension().and_then(|s| s.to_str()) == Some("jsonl")));
    }

    #[test]
    fn walk_jsonl_returns_empty_for_missing_root() {
        let td = TempDir::new().unwrap();
        let missing = td.path().join("does-not-exist");
        let found: Vec<_> = walk_jsonl(&missing).collect();
        assert!(found.is_empty());
    }
```

Also add `use std::path::Path;` if not already in the `tests` module imports (it is via `use super::*` + the earlier test).

- [ ] **Step 2: Verify the tests fail**

Run: `cargo test --lib walk_jsonl`
Expected: compilation error — "cannot find function `walk_jsonl` in this scope".

- [ ] **Step 3: Implement `walk_jsonl`**

Add to `src/data/parser.rs` (above the tests block):

```rust
/// Recursively yields every `*.jsonl` file under `root` (any depth).
///
/// Returns an empty iterator if `root` doesn't exist or can't be read —
/// callers don't need to special-case the first-run case.
pub fn walk_jsonl(root: &std::path::Path) -> impl Iterator<Item = PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    walk_inner(root, &mut out);
    out.into_iter()
}

fn walk_inner(dir: &std::path::Path, out: &mut Vec<PathBuf>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let p = entry.path();
        if p.is_dir() {
            walk_inner(&p, out);
        } else if p.extension().and_then(|s| s.to_str()) == Some("jsonl") {
            out.push(p);
        }
    }
}
```

Beginner note: we materialize the result into a `Vec` rather than streaming because recursive directory walking with lazy iteration in Rust requires more setup (a state machine or the `walkdir` crate). For our ~thousands-of-files scale, eager collection is fine.

- [ ] **Step 4: Verify the tests pass**

Run: `cargo test --lib walk_jsonl`
Expected: 2 passed.

- [ ] **Step 5: Commit**

```bash
git add src/data/parser.rs
git commit -m "feat(stage-5): add walk_jsonl recursive directory walker"
```

---

## Task 7: Implement `iter_rows` for normal usage rows

**Files:**
- Modify: `src/data/parser.rs`

- [ ] **Step 1: Add the fixture file at `tests/fixtures/sample_session.jsonl`**

Create the file `tests/fixtures/sample_session.jsonl` with this content (each line is its own JSONL record):

```jsonl
{"timestamp":"2026-05-22T10:00:00Z","sessionId":"sess-abc","cwd":"/proj/foo","version":"1.2.3","type":"assistant","message":{"model":"claude-opus-4-7","usage":{"input_tokens":100,"output_tokens":2000,"cache_creation_input_tokens":50,"cache_read_input_tokens":10}}}
{"timestamp":"2026-05-22T10:01:00Z","sessionId":"sess-abc","cwd":"/proj/foo","version":"1.2.3","type":"assistant","message":{"model":"claude-opus-4-7","usage":{"input_tokens":200,"output_tokens":3000,"cache_creation_input_tokens":0,"cache_read_input_tokens":100}}}
this line is corrupt and should be silently skipped
{"timestamp":"2026-05-22T10:02:00Z","sessionId":"sess-abc","cwd":"/proj/foo","version":"1.2.3","type":"user","message":{"role":"user","content":"hi"}}
{"timestamp":"2026-05-22T10:03:00Z","sessionId":"sess-abc","cwd":"/proj/foo","version":"1.2.3","type":"api-error","error":{"type":"rate_limit_error","status":429}}
```

Notes on the content:
- Two assistant rows with `message.usage` (parser must yield these).
- One garbage line (parser must skip).
- One user row without `message.usage` (parser must skip).
- One rate-limit error row (Task 8 will yield this; for this task it's still skipped because `is_rate_limit_error` handling is the next task).

- [ ] **Step 2: Add a failing integration test at `tests/parser_test.rs`**

```rust
use claude_usage_tray::data::parser::iter_rows;
use std::path::Path;

#[test]
fn iter_rows_yields_two_usage_rows_from_fixture() {
    let p = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("sample_session.jsonl");
    let rows: Vec<_> = iter_rows(&p).collect();

    // For now we expect exactly 2 — the two assistant-with-usage rows.
    // (The rate-limit row will be added in the next task.)
    let usage_rows: Vec<_> = rows.iter().filter(|r| !r.is_rate_limit_error).collect();
    assert_eq!(usage_rows.len(), 2);

    let r0 = &usage_rows[0];
    assert_eq!(r0.session_id, "sess-abc");
    assert_eq!(r0.project_cwd, "/proj/foo");
    assert_eq!(r0.model, "claude-opus-4-7");
    assert_eq!(r0.version, "1.2.3");
    assert_eq!(r0.input_tokens, 100);
    assert_eq!(r0.output_tokens, 2000);
    assert_eq!(r0.cache_creation_input_tokens, 50);
    assert_eq!(r0.cache_read_input_tokens, 10);
    assert!(!r0.is_subagent);
    assert_eq!(r0.subagent_id, None);

    let r1 = &usage_rows[1];
    assert_eq!(r1.output_tokens, 3000);
}
```

- [ ] **Step 3: Verify the test fails**

Run: `cargo test --test parser_test`
Expected: compilation error — "cannot find function `iter_rows` in module `parser`".

- [ ] **Step 4: Implement `iter_rows`**

Add to `src/data/parser.rs` (above the tests block):

```rust
/// Yields one `Turn` per JSONL line that contains usage telemetry or a
/// rate-limit error. Bad JSON lines, empty lines, and rows without a
/// `message.usage` field (and not rate-limit errors) are silently skipped.
pub fn iter_rows(path: &std::path::Path) -> impl Iterator<Item = Turn> {
    let (is_sub, sub_id) = classify_subagent(path);
    let path_owned = path.to_path_buf();
    let content = std::fs::read_to_string(path).unwrap_or_default();
    let lines: Vec<String> = content.lines().map(|s| s.to_string()).collect();

    lines.into_iter().filter_map(move |raw_line| {
        let line = raw_line.trim();
        if line.is_empty() {
            return None;
        }
        let obj: serde_json::Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => return None,
        };
        let obj = obj.as_object()?;

        let rate_limited = is_rate_limit_error(obj);
        let usage = obj.get("message").and_then(|m| m.get("usage"));
        if usage.is_none() && !rate_limited {
            return None;
        }

        let ts_raw = obj.get("timestamp").and_then(|v| v.as_str())?;
        let ts = chrono::DateTime::parse_from_rfc3339(ts_raw)
            .ok()?
            .with_timezone(&chrono::Utc);

        let session_id = obj
            .get("sessionId")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let project_cwd = obj
            .get("cwd")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let version = obj
            .get("version")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let model = obj
            .get("message")
            .and_then(|m| m.get("model"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let usage_obj = usage.and_then(|u| u.as_object());
        let get_u64 = |key: &str| -> u64 {
            usage_obj
                .and_then(|u| u.get(key))
                .and_then(|v| v.as_u64())
                .unwrap_or(0)
        };

        Some(Turn {
            ts,
            session_id,
            subagent_id: sub_id.clone(),
            is_subagent: is_sub,
            project_cwd,
            model,
            version,
            input_tokens: get_u64("input_tokens"),
            output_tokens: get_u64("output_tokens"),
            cache_creation_input_tokens: get_u64("cache_creation_input_tokens"),
            cache_read_input_tokens: get_u64("cache_read_input_tokens"),
            source_file: path_owned.clone(),
            is_rate_limit_error: rate_limited,
        })
    })
}

/// Stub — full implementation arrives in the next task.
fn is_rate_limit_error(_obj: &serde_json::Map<String, serde_json::Value>) -> bool {
    false
}
```

Beginner notes:
- We `read_to_string` the whole file then iterate `lines()`. For our scale (~MB per file) this is fine and simpler than streaming.
- `serde_json::Value` is the dynamic-JSON type — handy when we don't want to declare a full struct for every possible row shape.
- `move` on the outer closure captures `path_owned`, `is_sub`, `sub_id` by value so the returned iterator owns them and outlives the function.

- [ ] **Step 5: Verify the test passes**

Run: `cargo test --test parser_test`
Expected: 1 passed.

- [ ] **Step 6: Commit**

```bash
git add src/data/parser.rs tests/fixtures/sample_session.jsonl tests/parser_test.rs
git commit -m "feat(stage-5): implement iter_rows for usage rows + fixture"
```

---

## Task 8: Detect rate-limit error rows in `iter_rows`

**Files:**
- Modify: `src/data/parser.rs`
- Modify: `tests/parser_test.rs`

- [ ] **Step 1: Add a failing test**

Append to `tests/parser_test.rs`:

```rust
#[test]
fn iter_rows_yields_rate_limit_error_row() {
    let p = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("sample_session.jsonl");
    let rows: Vec<_> = iter_rows(&p).collect();

    let rl_rows: Vec<_> = rows.iter().filter(|r| r.is_rate_limit_error).collect();
    assert_eq!(rl_rows.len(), 1);
    assert_eq!(rl_rows[0].session_id, "sess-abc");
    assert_eq!(rl_rows[0].input_tokens, 0); // no usage on error rows
}
```

- [ ] **Step 2: Verify the test fails**

Run: `cargo test --test parser_test iter_rows_yields_rate_limit_error_row`
Expected: assertion fails — `rl_rows.len()` is 0 (the stub returns `false`).

- [ ] **Step 3: Replace the stub `is_rate_limit_error`**

In `src/data/parser.rs`, replace the stub with:

```rust
/// Returns true if `obj` represents an API error caused by rate limiting.
fn is_rate_limit_error(obj: &serde_json::Map<String, serde_json::Value>) -> bool {
    // Outer "type" must be an error variant.
    let outer_type = obj.get("type").and_then(|v| v.as_str()).unwrap_or("");
    if outer_type != "api-error" && outer_type != "error" {
        return false;
    }
    // Check obj.error directly.
    if let Some(err) = obj.get("error").and_then(|v| v.as_object()) {
        if error_indicates_rate_limit(err) {
            return true;
        }
    }
    // Check obj.message.error (some shapes nest it inside message).
    if let Some(err) = obj
        .get("message")
        .and_then(|m| m.get("error"))
        .and_then(|v| v.as_object())
    {
        if error_indicates_rate_limit(err) {
            return true;
        }
    }
    false
}

fn error_indicates_rate_limit(err: &serde_json::Map<String, serde_json::Value>) -> bool {
    if err.get("status").and_then(|v| v.as_u64()) == Some(429) {
        return true;
    }
    let t = err
        .get("type")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_lowercase();
    t.contains("rate") || t.contains("limit")
}
```

- [ ] **Step 4: Verify all parser tests pass**

Run: `cargo test --test parser_test`
Expected: 2 passed.

- [ ] **Step 5: Commit**

```bash
git add src/data/parser.rs tests/parser_test.rs
git commit -m "feat(stage-5): detect rate-limit error rows in iter_rows"
```

---

## Task 9: Scaffold `src/data/cache.rs` with serializable types

**Files:**
- Create: `src/data/cache.rs`
- Modify: `src/data/mod.rs`

- [ ] **Step 1: Create `src/data/cache.rs`**

```rust
//! Mtime-diff incremental cache for `Vec<Turn>`. Persists to
//! `~/.claude-usage-tray/cache.bincode` + `cache_manifest.json`.

use crate::data::parser::Turn;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use thiserror::Error;

pub const SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct CacheFile {
    pub schema_version: u32,
    pub turns: Vec<Turn>,
}

#[derive(Debug, Serialize, Deserialize, Default)]
pub(crate) struct Manifest {
    pub schema_version: u32,
    pub mtimes: HashMap<PathBuf, i64>,
}

#[derive(Debug, Error)]
pub enum CacheError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("bincode error: {0}")]
    Bincode(#[from] bincode::Error),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("anyhow error: {0}")]
    Anyhow(#[from] anyhow::Error),
}
```

- [ ] **Step 2: Register the new submodule in `src/data/mod.rs`**

```rust
//! Local JSONL data layer — parser + mtime-diff cache.

pub mod cache;
pub mod parser;
```

- [ ] **Step 3: Verify it compiles**

Run: `cargo build`
Expected: builds cleanly. The `pub(crate)` types are unused-warning-suppressed by being `pub(crate)` (Rust doesn't warn on these the same way as private items).

- [ ] **Step 4: Commit**

```bash
git add src/data/cache.rs src/data/mod.rs
git commit -m "feat(stage-5): scaffold cache module with CacheFile + Manifest types"
```

---

## Task 10: Implement `cache::refresh` — first-run full parse

**Files:**
- Modify: `src/data/cache.rs`
- Create: `tests/cache_test.rs`

- [ ] **Step 1: Add a failing integration test at `tests/cache_test.rs`**

```rust
use claude_usage_tray::data::cache;
use tempfile::TempDir;

fn write_jsonl(dir: &std::path::Path, rel: &str, content: &str) {
    let p = dir.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, content).unwrap();
}

const SAMPLE_USAGE_LINE: &str = r#"{"timestamp":"2026-05-22T10:00:00Z","sessionId":"s1","cwd":"/proj","version":"1","type":"assistant","message":{"model":"opus","usage":{"input_tokens":1,"output_tokens":100}}}"#;

#[test]
fn refresh_first_run_parses_all_files() {
    let projects = TempDir::new().unwrap();
    let app_dir = TempDir::new().unwrap();
    write_jsonl(projects.path(), "a/sess-1.jsonl", SAMPLE_USAGE_LINE);
    write_jsonl(projects.path(), "b/sess-2.jsonl", SAMPLE_USAGE_LINE);

    let turns = cache::refresh_at(projects.path(), app_dir.path()).unwrap();
    assert_eq!(turns.len(), 2);

    // Cache files should now exist.
    assert!(app_dir.path().join("cache.bincode").exists());
    assert!(app_dir.path().join("cache_manifest.json").exists());
}
```

- [ ] **Step 2: Verify the test fails**

Run: `cargo test --test cache_test`
Expected: compilation error — "cannot find function `refresh_at` in module `cache`".

- [ ] **Step 3: Implement `refresh_at` (takes paths for testability)**

Append to `src/data/cache.rs`:

```rust
use crate::data::parser::{iter_rows, walk_jsonl};
use std::path::Path;
use std::time::UNIX_EPOCH;

/// Refresh the cache against `projects_root`, writing cache + manifest into `app_dir`.
///
/// Returns the full sorted-by-`ts` `Vec<Turn>` covering every JSONL file under root.
/// First run reads everything; subsequent runs reparse only files whose mtime changed
/// (and drop rows from files that have been deleted).
pub fn refresh_at(projects_root: &Path, app_dir: &Path) -> Result<Vec<Turn>, CacheError> {
    std::fs::create_dir_all(app_dir)?;

    // 1. Load prior cache + manifest, tolerating any failure (treat as empty).
    let mut prior_turns: Vec<Turn> = load_cache(app_dir).unwrap_or_default();
    let mut prior_mtimes: HashMap<PathBuf, i64> = load_manifest(app_dir).unwrap_or_default();

    // 2. Walk root for *.jsonl and read current mtimes.
    let current: Vec<PathBuf> = walk_jsonl(projects_root).collect();
    let mut current_mtimes: HashMap<PathBuf, i64> = HashMap::new();
    for p in &current {
        let mt = mtime_millis(p).unwrap_or(0);
        current_mtimes.insert(p.clone(), mt);
    }

    // 3. Compute diff sets.
    let new_or_changed: Vec<PathBuf> = current
        .iter()
        .filter(|p| prior_mtimes.get(*p) != current_mtimes.get(*p))
        .cloned()
        .collect();
    let deleted: Vec<PathBuf> = prior_mtimes
        .keys()
        .filter(|p| !current_mtimes.contains_key(*p))
        .cloned()
        .collect();

    // 4. Fast path: nothing changed → return prior turns.
    if new_or_changed.is_empty() && deleted.is_empty() {
        prior_turns.sort_by_key(|t| t.ts);
        return Ok(prior_turns);
    }

    // 5. Drop stale rows.
    let stale: std::collections::HashSet<PathBuf> = new_or_changed
        .iter()
        .chain(deleted.iter())
        .cloned()
        .collect();
    prior_turns.retain(|t| !stale.contains(&t.source_file));

    // 6. Reparse changed files, append.
    for p in &new_or_changed {
        for row in iter_rows(p) {
            prior_turns.push(row);
        }
    }

    // 7. Sort by ts.
    prior_turns.sort_by_key(|t| t.ts);

    // 8. Write out (atomic).
    prior_mtimes = current_mtimes;
    write_cache(app_dir, &prior_turns)?;
    write_manifest(app_dir, &prior_mtimes)?;

    Ok(prior_turns)
}

fn mtime_millis(p: &Path) -> Option<i64> {
    let meta = std::fs::metadata(p).ok()?;
    let modified = meta.modified().ok()?;
    let dur = modified.duration_since(UNIX_EPOCH).ok()?;
    Some(dur.as_millis() as i64)
}

fn load_cache(app_dir: &Path) -> Result<Vec<Turn>, CacheError> {
    let path = app_dir.join("cache.bincode");
    if !path.exists() {
        return Ok(Vec::new());
    }
    let bytes = std::fs::read(&path)?;
    let file: CacheFile = bincode::deserialize(&bytes)?;
    if file.schema_version != SCHEMA_VERSION {
        return Ok(Vec::new());
    }
    Ok(file.turns)
}

fn load_manifest(app_dir: &Path) -> Result<HashMap<PathBuf, i64>, CacheError> {
    let path = app_dir.join("cache_manifest.json");
    if !path.exists() {
        return Ok(HashMap::new());
    }
    let bytes = std::fs::read(&path)?;
    let m: Manifest = serde_json::from_slice(&bytes)?;
    if m.schema_version != SCHEMA_VERSION {
        return Ok(HashMap::new());
    }
    Ok(m.mtimes)
}

fn write_cache(app_dir: &Path, turns: &[Turn]) -> Result<(), CacheError> {
    let file = CacheFile {
        schema_version: SCHEMA_VERSION,
        turns: turns.to_vec(),
    };
    let bytes = bincode::serialize(&file)?;
    let final_path = app_dir.join("cache.bincode");
    let tmp_path = app_dir.join("cache.bincode.tmp");
    std::fs::write(&tmp_path, &bytes)?;
    std::fs::rename(&tmp_path, &final_path)?;
    Ok(())
}

fn write_manifest(app_dir: &Path, mtimes: &HashMap<PathBuf, i64>) -> Result<(), CacheError> {
    let m = Manifest {
        schema_version: SCHEMA_VERSION,
        mtimes: mtimes.clone(),
    };
    let bytes = serde_json::to_vec_pretty(&m)?;
    let final_path = app_dir.join("cache_manifest.json");
    let tmp_path = app_dir.join("cache_manifest.json.tmp");
    std::fs::write(&tmp_path, &bytes)?;
    std::fs::rename(&tmp_path, &final_path)?;
    Ok(())
}
```

- [ ] **Step 4: Verify the test passes**

Run: `cargo test --test cache_test`
Expected: 1 passed.

- [ ] **Step 5: Commit**

```bash
git add src/data/cache.rs tests/cache_test.rs
git commit -m "feat(stage-5): implement cache::refresh_at first-run full parse"
```

---

## Task 11: Mtime-diff incremental — only reparse changed files

**Files:**
- Modify: `tests/cache_test.rs`

- [ ] **Step 1: Add a failing test**

Append to `tests/cache_test.rs`:

```rust
#[test]
fn refresh_second_run_reparses_only_changed_files() {
    let projects = TempDir::new().unwrap();
    let app_dir = TempDir::new().unwrap();
    write_jsonl(projects.path(), "a/sess-1.jsonl", SAMPLE_USAGE_LINE);
    write_jsonl(projects.path(), "b/sess-2.jsonl", SAMPLE_USAGE_LINE);

    // First run primes the cache.
    let turns_1 = cache::refresh_at(projects.path(), app_dir.path()).unwrap();
    assert_eq!(turns_1.len(), 2);

    // Modify sess-1 to add a second row. Set its mtime to "now" explicitly so
    // the change is detectable even on filesystems with coarse mtime resolution.
    let extra = format!("\n{}", SAMPLE_USAGE_LINE);
    let p1 = projects.path().join("a").join("sess-1.jsonl");
    let mut existing = std::fs::read_to_string(&p1).unwrap();
    existing.push_str(&extra);
    std::fs::write(&p1, existing).unwrap();
    let now = std::time::SystemTime::now() + std::time::Duration::from_secs(2);
    filetime::set_file_mtime(&p1, filetime::FileTime::from_system_time(now)).unwrap();

    let turns_2 = cache::refresh_at(projects.path(), app_dir.path()).unwrap();
    // sess-1 now has 2 rows; sess-2 still has 1.
    assert_eq!(turns_2.len(), 3);
}
```

- [ ] **Step 2: Add `filetime` to `[dev-dependencies]` in `Cargo.toml`**

```toml
[dev-dependencies]
tempfile = "3"
filetime = "0.2"
```

- [ ] **Step 3: Verify the test passes**

Run: `cargo test --test cache_test refresh_second_run_reparses_only_changed_files`
Expected: 1 passed. (The incremental logic from Task 10 already handles this — this test is verification that the diff-set computation works.)

- [ ] **Step 4: Add a no-op fast-path test to ensure the unchanged case bypasses reparse**

Append:

```rust
#[test]
fn refresh_no_changes_returns_quickly_with_same_count() {
    let projects = TempDir::new().unwrap();
    let app_dir = TempDir::new().unwrap();
    write_jsonl(projects.path(), "a/sess.jsonl", SAMPLE_USAGE_LINE);

    let turns_1 = cache::refresh_at(projects.path(), app_dir.path()).unwrap();
    let turns_2 = cache::refresh_at(projects.path(), app_dir.path()).unwrap();
    assert_eq!(turns_1, turns_2);
}
```

- [ ] **Step 5: Verify the test passes**

Run: `cargo test --test cache_test`
Expected: 3 passed.

- [ ] **Step 6: Commit**

```bash
git add tests/cache_test.rs Cargo.toml Cargo.lock
git commit -m "test(stage-5): verify mtime-diff incremental + no-op fast path"
```

---

## Task 12: Handle file deletion + corrupt-cache rebuild

**Files:**
- Modify: `tests/cache_test.rs`

- [ ] **Step 1: Add two failing tests**

Append to `tests/cache_test.rs`:

```rust
#[test]
fn refresh_drops_rows_from_deleted_files() {
    let projects = TempDir::new().unwrap();
    let app_dir = TempDir::new().unwrap();
    write_jsonl(projects.path(), "a.jsonl", SAMPLE_USAGE_LINE);
    write_jsonl(projects.path(), "b.jsonl", SAMPLE_USAGE_LINE);

    let turns_1 = cache::refresh_at(projects.path(), app_dir.path()).unwrap();
    assert_eq!(turns_1.len(), 2);

    std::fs::remove_file(projects.path().join("a.jsonl")).unwrap();

    let turns_2 = cache::refresh_at(projects.path(), app_dir.path()).unwrap();
    assert_eq!(turns_2.len(), 1);
}

#[test]
fn refresh_recovers_from_corrupt_cache() {
    let projects = TempDir::new().unwrap();
    let app_dir = TempDir::new().unwrap();
    write_jsonl(projects.path(), "a.jsonl", SAMPLE_USAGE_LINE);

    // Prime the cache.
    let _ = cache::refresh_at(projects.path(), app_dir.path()).unwrap();

    // Corrupt the cache file.
    std::fs::write(app_dir.path().join("cache.bincode"), b"not bincode").unwrap();

    // Refresh should silently rebuild and return the correct rows.
    let turns = cache::refresh_at(projects.path(), app_dir.path()).unwrap();
    assert_eq!(turns.len(), 1);

    // A second refresh on an unchanged tree must succeed (proves the rebuild
    // produced a deserializable cache).
    let turns2 = cache::refresh_at(projects.path(), app_dir.path()).unwrap();
    assert_eq!(turns2.len(), 1);
}
```

- [ ] **Step 2: Verify the tests pass**

Run: `cargo test --test cache_test`
Expected: 5 passed. (The deletion test passes because Task 10's diff sets already include `deleted`. The corruption-recovery test passes because `load_cache` returns `Err`, and `unwrap_or_default` falls back to `Vec::new()`.)

- [ ] **Step 3: Commit**

```bash
git add tests/cache_test.rs
git commit -m "test(stage-5): verify file-deletion handling and corrupt-cache rebuild"
```

---

## Task 13: Add `paths-resolved` wrapper `cache::refresh`

**Files:**
- Modify: `src/data/cache.rs`

- [ ] **Step 1: Add the convenience wrapper**

The polling thread will need a wrapper that resolves the default paths. Append to `src/data/cache.rs`:

```rust
/// Convenience wrapper: refreshes against `~/.claude/projects/` and writes the
/// cache under `~/.claude-usage-tray/`. Used by the polling thread.
pub fn refresh() -> Result<Vec<Turn>, CacheError> {
    let projects_root = dirs::home_dir()
        .ok_or_else(|| anyhow::anyhow!("could not determine home directory"))?
        .join(".claude")
        .join("projects");
    let app_dir = crate::paths::app_dir()?;
    refresh_at(&projects_root, &app_dir)
}
```

- [ ] **Step 2: Verify it compiles**

Run: `cargo build`
Expected: builds cleanly.

- [ ] **Step 3: Commit**

```bash
git add src/data/cache.rs
git commit -m "feat(stage-5): add cache::refresh wrapper with default paths"
```

---

## Task 14: Add `log::calibration::read_all` helper

**Files:**
- Modify: `src/log/calibration.rs`

- [ ] **Step 1: Add the helper at the bottom of `src/log/calibration.rs`**

```rust
/// Read every record from a calibration log file. Bad lines are silently
/// skipped (matches the append-side tolerance). Returns empty Vec if the
/// file doesn't exist.
pub fn read_all(path: &Path) -> Result<Vec<CalibrationSample>, LogError> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let content = std::fs::read_to_string(path)?;
    let mut out = Vec::new();
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        match serde_json::from_str::<CalibrationSample>(line) {
            Ok(s) => out.push(s),
            Err(_) => {
                tracing::trace!(line = %line, "skipping malformed calibration log line");
            }
        }
    }
    Ok(out)
}

/// Convenience wrapper: read from the default path.
pub fn read_all_default() -> Result<Vec<CalibrationSample>, LogError> {
    let path = paths::calibration_log_path()
        .map_err(|e| LogError::Io(std::io::Error::other(e.to_string())))?;
    read_all(&path)
}
```

- [ ] **Step 2: Verify it compiles**

Run: `cargo build`
Expected: builds cleanly.

- [ ] **Step 3: Add a tiny round-trip test in `tests/calibration_log_test.rs`**

Append:

```rust
#[test]
fn read_all_round_trips_through_append() {
    use claude_usage_tray::log::calibration::read_all;
    let td = TempDir::new().unwrap();
    let path = td.path().join("log.jsonl");

    let sample = sample_from(&fake_snapshot(), &fake_creds());
    append(&path, &sample).unwrap();
    append(&path, &sample).unwrap();

    let rows = read_all(&path).unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].subscription_type, "pro");
}
```

- [ ] **Step 4: Verify the test passes**

Run: `cargo test --test calibration_log_test read_all_round_trips_through_append`
Expected: 1 passed.

- [ ] **Step 5: Commit**

```bash
git add src/log/calibration.rs tests/calibration_log_test.rs
git commit -m "feat(stage-5): add log::calibration::read_all helper"
```

---

## Task 15: Scaffold `src/calibration/` module + shared types

**Files:**
- Create: `src/calibration/mod.rs`
- Create: `src/calibration/anchors.rs`
- Modify: `src/lib.rs`

- [ ] **Step 1: Create `src/calibration/mod.rs`**

```rust
//! Local calibration math: derives 5h + weekly caps from the calibration log,
//! computes the current live util, and (ahead of Stage 6) a per-hour cap series.

pub mod anchors;
pub mod hourly;
pub mod live;

/// Which window kind to compute against.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowKind {
    FiveHour,
    Weekly,
}
```

- [ ] **Step 2: Create stub `src/calibration/anchors.rs`**

```rust
//! Median-of-anchors cap derivation.

use chrono::{DateTime, Utc};

/// Caps derived from the latest calibration log + cache.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DerivedCaps {
    pub cap_5h: Option<f64>,
    pub cap_week: Option<f64>,
    pub n_anchors_5h: usize,
    pub n_anchors_week: usize,
}

// Subsequent tasks add `last_weekly_reset`, `five_hour_burn_at`,
// `weekly_burn_at`, `global_cap_from_anchors`.
#[allow(dead_code)]
fn _placeholder(_t: DateTime<Utc>) {}
```

- [ ] **Step 3: Create stub `src/calibration/hourly.rs`**

```rust
//! 24-bin hour-of-day cap series. Built ahead of Stage 6; not displayed in v0.5.0.

// Filled in by later tasks.
```

- [ ] **Step 4: Create stub `src/calibration/live.rs`**

```rust
//! Live util for the tooltip.

// Filled in by Task 22.
```

- [ ] **Step 5: Register the module in `src/lib.rs`**

Add `pub mod calibration;` alphabetically:

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
pub mod tray;
pub mod watch;
```

- [ ] **Step 6: Verify it compiles**

Run: `cargo build`
Expected: builds cleanly with a warning about the unused `_placeholder` (acceptable; it disappears in Task 16).

- [ ] **Step 7: Commit**

```bash
git add src/calibration/ src/lib.rs
git commit -m "feat(stage-5): scaffold calibration module with DerivedCaps + WindowKind"
```

---

## Task 16: Implement `last_weekly_reset` with unit tests

**Files:**
- Modify: `src/calibration/anchors.rs`

- [ ] **Step 1: Add failing tests at the bottom of `src/calibration/anchors.rs`**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn utc(y: i32, m: u32, d: u32, h: u32, mi: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(y, m, d, h, mi, 0).unwrap()
    }

    #[test]
    fn last_weekly_reset_when_anchor_is_monday_picks_prior_sunday_0700() {
        // Mon 2026-05-25 14:30 UTC = Mon 16:30 local (CEST = UTC+2 in May).
        let anchor = utc(2026, 5, 25, 14, 30);
        let reset = last_weekly_reset(anchor);
        // Prior Sun 2026-05-24 07:00 local CEST = 2026-05-24 05:00 UTC.
        assert_eq!(reset, utc(2026, 5, 24, 5, 0));
    }

    #[test]
    fn last_weekly_reset_when_anchor_is_sunday_after_0700_picks_today() {
        let anchor = utc(2026, 5, 24, 8, 0);  // Sun 10:00 local CEST
        let reset = last_weekly_reset(anchor);
        assert_eq!(reset, utc(2026, 5, 24, 5, 0));
    }

    #[test]
    fn last_weekly_reset_when_anchor_is_sunday_before_0700_picks_prior_sunday() {
        let anchor = utc(2026, 5, 24, 4, 0);  // Sun 06:00 local CEST
        let reset = last_weekly_reset(anchor);
        // Prior Sun: 2026-05-17 05:00 UTC.
        assert_eq!(reset, utc(2026, 5, 17, 5, 0));
    }

    #[test]
    fn last_weekly_reset_handles_saturday() {
        let anchor = utc(2026, 5, 23, 10, 0);  // Sat 12:00 local
        let reset = last_weekly_reset(anchor);
        // Prior Sun = 2026-05-17 05:00 UTC.
        assert_eq!(reset, utc(2026, 5, 17, 5, 0));
    }
}
```

- [ ] **Step 2: Verify the tests fail**

Run: `cargo test --lib last_weekly_reset`
Expected: compilation errors — "cannot find function `last_weekly_reset`".

- [ ] **Step 3: Implement `last_weekly_reset`**

Replace the `_placeholder` line in `src/calibration/anchors.rs` with:

```rust
use crate::config;
use chrono::{Datelike, Duration, TimeZone, Timelike, Weekday};
use chrono_tz::Tz;

/// Returns the most-recent weekly-reset moment (Sun 07:00 local) at or before
/// `anchor_ts`, expressed in UTC.
pub fn last_weekly_reset(anchor_ts: DateTime<Utc>) -> DateTime<Utc> {
    let tz: Tz = config::LOCAL_TZ.parse().expect("LOCAL_TZ must be a valid IANA name");
    let local = anchor_ts.with_timezone(&tz);

    // days_back: how many days from `local`'s weekday back to Sunday (0..=6).
    let target = config::WEEKLY_RESET_WEEKDAY;
    let days_back = ((local.weekday().num_days_from_monday() as i64)
        - (target.num_days_from_monday() as i64))
        .rem_euclid(7);

    // Sun of the same week at 07:00 local.
    let candidate_date = local.date_naive() - Duration::days(days_back);
    let candidate_naive = candidate_date
        .and_hms_opt(config::WEEKLY_RESET_HOUR_LOCAL, 0, 0)
        .expect("07:00 is always valid");
    let candidate_local = tz
        .from_local_datetime(&candidate_naive)
        .single()
        .or_else(|| tz.from_local_datetime(&candidate_naive).earliest())
        .expect("Sun 07:00 should resolve unambiguously");

    let candidate = if candidate_local > local {
        candidate_local - Duration::days(7)
    } else {
        candidate_local
    };

    candidate.with_timezone(&Utc)
}

#[allow(dead_code)]
fn _silence_imports(_: Weekday) {}
```

Beginner notes:
- `num_days_from_monday`: chrono's `Weekday` returns 0 for Mon ... 6 for Sun. We compute distance from `local.weekday` back to `target = Sunday`. The `rem_euclid(7)` keeps it non-negative when target weekday is "earlier in the week" than current.
- `from_local_datetime` returns a `LocalResult` because DST can cause an hour to be ambiguous or skipped. 07:00 local on a Sunday is far from any DST transition, so `single()` always works — but we keep the `earliest()` fallback as a defensive measure.

- [ ] **Step 4: Verify the tests pass**

Run: `cargo test --lib last_weekly_reset`
Expected: 4 passed.

- [ ] **Step 5: Commit**

```bash
git add src/calibration/anchors.rs
git commit -m "feat(stage-5): implement last_weekly_reset with DST-safe local-tz math"
```

---

## Task 17: Implement `five_hour_burn_at` with gap-based window detection

**Files:**
- Modify: `src/calibration/anchors.rs`

- [ ] **Step 1: Add failing tests**

Inside the existing `#[cfg(test)] mod tests` block in `src/calibration/anchors.rs`, append:

```rust
    use crate::data::parser::Turn;
    use std::path::PathBuf;

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

    #[test]
    fn five_hour_burn_at_single_window_sums_all() {
        let turns = vec![
            turn(utc(2026, 5, 24, 10, 0), 100),
            turn(utc(2026, 5, 24, 11, 0), 200),
            turn(utc(2026, 5, 24, 12, 0), 300),
        ];
        let anchor = utc(2026, 5, 24, 13, 0);
        assert_eq!(five_hour_burn_at(&turns, anchor), 600);
    }

    #[test]
    fn five_hour_burn_at_drops_pre_gap_turns() {
        // First turn at 04:00, big gap, then a session 10:00-12:00 totalling 500.
        // Anchor at 12:00 should include only the 10:00+ turns.
        let turns = vec![
            turn(utc(2026, 5, 24, 4, 0), 999),   // pre-gap — should be excluded
            turn(utc(2026, 5, 24, 10, 0), 100),
            turn(utc(2026, 5, 24, 11, 0), 200),
            turn(utc(2026, 5, 24, 12, 0), 200),
        ];
        let anchor = utc(2026, 5, 24, 12, 0);
        assert_eq!(five_hour_burn_at(&turns, anchor), 500);
    }

    #[test]
    fn five_hour_burn_at_window_rollover_by_duration() {
        // Continuous activity over >4.5 hours triggers rollover at the 4.5h mark.
        let turns = vec![
            turn(utc(2026, 5, 24, 8, 0), 100),
            turn(utc(2026, 5, 24, 10, 0), 200),
            turn(utc(2026, 5, 24, 12, 30), 300),  // 4.5h after 08:00 → new window starts here
            turn(utc(2026, 5, 24, 13, 0), 400),
        ];
        let anchor = utc(2026, 5, 24, 13, 0);
        // New window starts at 12:30. Sums 300 + 400 = 700.
        assert_eq!(five_hour_burn_at(&turns, anchor), 700);
    }
```

- [ ] **Step 2: Verify the tests fail**

Run: `cargo test --lib five_hour_burn_at`
Expected: compilation errors — "cannot find function `five_hour_burn_at`".

- [ ] **Step 3: Implement `five_hour_burn_at`**

Add to `src/calibration/anchors.rs` (above the tests block, after `last_weekly_reset`):

```rust
use crate::data::parser::Turn;

/// Sum `output_tokens` for the gap-based 5h window containing `anchor_ts`.
///
/// `turns` is assumed sorted by `ts` ascending. The window resets to start at
/// the current turn whenever:
///   - the gap from the previous turn is `>= FIVE_HOUR_WINDOW_HOURS`, OR
///   - the window has been open for `>= FIVE_HOUR_WINDOW_HOURS`.
pub fn five_hour_burn_at(turns: &[Turn], anchor_ts: DateTime<Utc>) -> u64 {
    let gap = Duration::milliseconds((config::FIVE_HOUR_WINDOW_HOURS * 3_600_000.0) as i64);
    let mut current_start: Option<DateTime<Utc>> = None;
    let mut last_ts: Option<DateTime<Utc>> = None;
    let mut burn: u64 = 0;

    for t in turns.iter().filter(|t| t.ts <= anchor_ts) {
        match (current_start, last_ts) {
            (None, _) => {
                current_start = Some(t.ts);
            }
            (Some(start), Some(prev)) => {
                let since_last = t.ts - prev;
                let since_start = t.ts - start;
                if since_last >= gap || since_start >= gap {
                    current_start = Some(t.ts);
                    burn = 0;
                }
            }
            (Some(_), None) => unreachable!("current_start implies last_ts"),
        }
        burn += t.output_tokens;
        last_ts = Some(t.ts);
    }

    burn
}
```

- [ ] **Step 4: Verify the tests pass**

Run: `cargo test --lib five_hour_burn_at`
Expected: 3 passed.

- [ ] **Step 5: Commit**

```bash
git add src/calibration/anchors.rs
git commit -m "feat(stage-5): implement five_hour_burn_at with gap-based window detection"
```

---

## Task 18: Implement `weekly_burn_at`

**Files:**
- Modify: `src/calibration/anchors.rs`

- [ ] **Step 1: Add failing tests**

Inside the existing `mod tests` block in `src/calibration/anchors.rs`, append:

```rust
    #[test]
    fn weekly_burn_at_sums_since_last_reset() {
        let turns = vec![
            turn(utc(2026, 5, 17, 4, 0), 999),   // before Sun 05:00 UTC reset — excluded
            turn(utc(2026, 5, 17, 6, 0), 100),   // after reset
            turn(utc(2026, 5, 19, 12, 0), 200),
            turn(utc(2026, 5, 23, 8, 0), 300),
        ];
        let anchor = utc(2026, 5, 23, 12, 0);  // Sat — last reset was Sun 17 05:00 UTC
        assert_eq!(weekly_burn_at(&turns, anchor), 600);
    }

    #[test]
    fn weekly_burn_at_after_reset_excludes_prior_week() {
        let turns = vec![
            turn(utc(2026, 5, 23, 12, 0), 500),
            turn(utc(2026, 5, 24, 6, 0), 100),  // after Sun 05:00 UTC reset
        ];
        let anchor = utc(2026, 5, 24, 8, 0);
        // Only the 100 token row falls within the new week.
        assert_eq!(weekly_burn_at(&turns, anchor), 100);
    }
```

- [ ] **Step 2: Verify the tests fail**

Run: `cargo test --lib weekly_burn_at`
Expected: compilation errors — "cannot find function `weekly_burn_at`".

- [ ] **Step 3: Implement `weekly_burn_at`**

Add to `src/calibration/anchors.rs` (after `five_hour_burn_at`):

```rust
/// Sum `output_tokens` since the most-recent Sun 07:00-local reset.
/// `turns` may be in any order; we filter, not iterate-in-order.
pub fn weekly_burn_at(turns: &[Turn], anchor_ts: DateTime<Utc>) -> u64 {
    let win_start = last_weekly_reset(anchor_ts);
    turns
        .iter()
        .filter(|t| t.ts >= win_start && t.ts <= anchor_ts)
        .map(|t| t.output_tokens)
        .sum()
}
```

- [ ] **Step 4: Verify the tests pass**

Run: `cargo test --lib weekly_burn_at`
Expected: 2 passed.

- [ ] **Step 5: Commit**

```bash
git add src/calibration/anchors.rs
git commit -m "feat(stage-5): implement weekly_burn_at since-Sun-0700-local reset"
```

---

## Task 19: Implement `global_cap_from_anchors`

**Files:**
- Modify: `src/calibration/anchors.rs`

- [ ] **Step 1: Add failing tests**

Inside the existing `mod tests` block, append:

```rust
    use crate::log::calibration::CalibrationSample;
    use crate::calibration::WindowKind;

    fn sample(ts: DateTime<Utc>, util_5h: f64, util_7d: f64) -> CalibrationSample {
        CalibrationSample {
            schema_version: 1,
            ts,
            five_hour_util: Some(util_5h),
            five_hour_resets_at: None,
            seven_day_util: Some(util_7d),
            seven_day_resets_at: None,
            subscription_type: "pro".to_string(),
            rate_limit_tier: "default_claude_ai".to_string(),
        }
    }

    #[test]
    fn global_cap_zero_anchors_returns_none() {
        let log = vec![sample(utc(2026, 5, 24, 10, 0), 0.5, 0.4)];
        let turns = vec![turn(utc(2026, 5, 24, 9, 0), 100)];
        let (cap, n) = global_cap_from_anchors(&log, &turns, WindowKind::FiveHour);
        assert!(cap.is_none());
        assert_eq!(n, 0);
    }

    #[test]
    fn global_cap_single_anchor_returns_burn_over_util() {
        // burn at anchor = 1000; util = 1.00 → cap = 1000.
        let log = vec![sample(utc(2026, 5, 24, 10, 0), 1.00, 0.5)];
        let turns = vec![
            turn(utc(2026, 5, 24, 8, 0), 400),
            turn(utc(2026, 5, 24, 9, 0), 600),
        ];
        let (cap, n) = global_cap_from_anchors(&log, &turns, WindowKind::FiveHour);
        assert_eq!(cap, Some(1000.0));
        assert_eq!(n, 1);
    }

    #[test]
    fn global_cap_multi_anchor_returns_median() {
        // Three anchors, all util=1.00, but bigger windows for some.
        // Implied caps: 100, 200, 300 → median 200.
        let log = vec![
            sample(utc(2026, 5, 24, 10, 0), 1.00, 0.5),
            sample(utc(2026, 5, 24, 16, 0), 1.00, 0.5),
            sample(utc(2026, 5, 24, 22, 0), 1.00, 0.5),
        ];
        let turns = vec![
            turn(utc(2026, 5, 24, 9, 30), 100),    // anchor 1: burn 100, util 1 → cap 100
            // Anchor 1's window ends, anchor 2 starts a new window. 6h gap > 4.5h.
            turn(utc(2026, 5, 24, 15, 30), 200),   // anchor 2: burn 200, util 1 → cap 200
            // Anchor 2's window ends, anchor 3 starts a new window.
            turn(utc(2026, 5, 24, 21, 30), 300),   // anchor 3: burn 300, util 1 → cap 300
        ];
        let (cap, n) = global_cap_from_anchors(&log, &turns, WindowKind::FiveHour);
        assert_eq!(cap, Some(200.0));
        assert_eq!(n, 3);
    }
```

- [ ] **Step 2: Verify the tests fail**

Run: `cargo test --lib global_cap_from_anchors`
Expected: compilation errors — "cannot find function `global_cap_from_anchors`".

- [ ] **Step 3: Implement `global_cap_from_anchors`**

Add to `src/calibration/anchors.rs`:

```rust
use crate::calibration::WindowKind;
use crate::log::calibration::CalibrationSample;

/// Median implied cap across all valid anchors. Returns (None, 0) if no anchors.
///
/// An anchor is a `CalibrationSample` where the relevant util (5h or weekly)
/// falls in `[MIN_ANCHOR_UTIL, MAX_ANCHOR_UTIL]`. For each anchor we compute
/// `burn_in_window(anchor.ts) / util` summing `output_tokens`, then take the
/// median across anchors.
pub fn global_cap_from_anchors(
    log: &[CalibrationSample],
    turns: &[Turn],
    kind: WindowKind,
) -> (Option<f64>, usize) {
    let mut implied: Vec<f64> = Vec::new();
    for s in log {
        let util_opt = match kind {
            WindowKind::FiveHour => s.five_hour_util,
            WindowKind::Weekly => s.seven_day_util,
        };
        let Some(util) = util_opt else { continue };
        if util < config::MIN_ANCHOR_UTIL || util > config::MAX_ANCHOR_UTIL {
            continue;
        }
        let burn = match kind {
            WindowKind::FiveHour => five_hour_burn_at(turns, s.ts),
            WindowKind::Weekly => weekly_burn_at(turns, s.ts),
        };
        if burn == 0 || util <= 0.0 {
            continue;
        }
        implied.push(burn as f64 / util);
    }
    if implied.is_empty() {
        return (None, 0);
    }
    let n = implied.len();
    implied.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let median = if n % 2 == 1 {
        implied[n / 2]
    } else {
        (implied[n / 2 - 1] + implied[n / 2]) / 2.0
    };
    (Some(median), n)
}
```

- [ ] **Step 4: Verify the tests pass**

Run: `cargo test --lib global_cap_from_anchors`
Expected: 3 passed.

- [ ] **Step 5: Add a public composite helper for the polling thread**

Append to `src/calibration/anchors.rs`:

```rust
/// Compute both 5h and weekly caps in one call.
pub fn derive_caps(log: &[CalibrationSample], turns: &[Turn]) -> DerivedCaps {
    let (cap_5h, n5) = global_cap_from_anchors(log, turns, WindowKind::FiveHour);
    let (cap_week, n7) = global_cap_from_anchors(log, turns, WindowKind::Weekly);
    DerivedCaps {
        cap_5h,
        cap_week,
        n_anchors_5h: n5,
        n_anchors_week: n7,
    }
}
```

- [ ] **Step 6: Verify it still compiles**

Run: `cargo build && cargo test --lib calibration`
Expected: builds + all calibration tests pass.

- [ ] **Step 7: Commit**

```bash
git add src/calibration/anchors.rs
git commit -m "feat(stage-5): implement global_cap_from_anchors + derive_caps"
```

---

## Task 20: Implement `per_hour_medians` in hourly.rs

**Files:**
- Modify: `src/calibration/hourly.rs`

- [ ] **Step 1: Add failing tests**

Add at the bottom of `src/calibration/hourly.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
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

    fn sample(ts: DateTime<Utc>, util_5h: f64) -> CalibrationSample {
        CalibrationSample {
            schema_version: 1,
            ts,
            five_hour_util: Some(util_5h),
            five_hour_resets_at: None,
            seven_day_util: Some(0.0),
            seven_day_resets_at: None,
            subscription_type: "pro".to_string(),
            rate_limit_tier: "default_claude_ai".to_string(),
        }
    }

    #[test]
    fn per_hour_medians_empty_log_returns_all_none() {
        let raw = per_hour_medians(&[], &[], WindowKind::FiveHour);
        assert_eq!(raw.len(), 24);
        assert!(raw.iter().all(|v| v.is_none()));
    }

    #[test]
    fn per_hour_medians_bins_by_local_hour() {
        // Anchor at 2026-05-24 14:00 UTC = 16:00 local CEST.
        // Burn = 100, util = 1.0 → implied cap = 100.
        let log = vec![sample(utc(2026, 5, 24, 14, 0), 1.0)];
        let turns = vec![turn(utc(2026, 5, 24, 13, 0), 100)];
        let raw = per_hour_medians(&log, &turns, WindowKind::FiveHour);
        // Bin 16 (local) should be Some(100.0).
        assert_eq!(raw[16], Some(100.0));
        // All other bins should be None.
        for (h, v) in raw.iter().enumerate() {
            if h != 16 {
                assert_eq!(*v, None);
            }
        }
    }
}
```

- [ ] **Step 2: Verify the tests fail**

Run: `cargo test --lib per_hour_medians`
Expected: compilation errors — "cannot find function `per_hour_medians`".

- [ ] **Step 3: Implement `per_hour_medians`**

Replace the empty body of `src/calibration/hourly.rs` (above the test block) with:

```rust
//! 24-bin hour-of-day cap series. Built ahead of Stage 6; not displayed in v0.5.0.

use crate::calibration::anchors::{five_hour_burn_at, weekly_burn_at};
use crate::calibration::WindowKind;
use crate::config;
use crate::data::parser::Turn;
use crate::log::calibration::CalibrationSample;
use chrono::{Datelike, Timelike};
use chrono_tz::Tz;

/// One implied cap per local hour-of-day, computed as median across anchors
/// whose timestamp falls in that bin. Bins with no anchors are `None`.
pub fn per_hour_medians(
    log: &[CalibrationSample],
    turns: &[Turn],
    kind: WindowKind,
) -> [Option<f64>; 24] {
    let tz: Tz = config::LOCAL_TZ
        .parse()
        .expect("LOCAL_TZ must be a valid IANA name");
    let mut buckets: [Vec<f64>; 24] = Default::default();

    for s in log {
        let util_opt = match kind {
            WindowKind::FiveHour => s.five_hour_util,
            WindowKind::Weekly => s.seven_day_util,
        };
        let Some(util) = util_opt else { continue };
        if util < config::MIN_ANCHOR_UTIL || util > config::MAX_ANCHOR_UTIL {
            continue;
        }
        let burn = match kind {
            WindowKind::FiveHour => five_hour_burn_at(turns, s.ts),
            WindowKind::Weekly => weekly_burn_at(turns, s.ts),
        };
        if burn == 0 || util <= 0.0 {
            continue;
        }
        let implied = burn as f64 / util;
        let local_hour = s.ts.with_timezone(&tz).hour() as usize;
        buckets[local_hour].push(implied);
    }

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
}

#[allow(dead_code)]
fn _silence(_: chrono::DateTime<chrono::Utc>) {} // keep imports tidy
```

Beginner note: `[Vec<f64>; 24]: Default` works because Rust derives `Default` for arrays-of-T where T: Default and the length is small (24 is fine).

- [ ] **Step 4: Verify the tests pass**

Run: `cargo test --lib per_hour_medians`
Expected: 2 passed.

- [ ] **Step 5: Commit**

```bash
git add src/calibration/hourly.rs
git commit -m "feat(stage-5): implement per_hour_medians for hourly cap bins"
```

---

## Task 21: Implement `smooth_rolling_circular`

**Files:**
- Modify: `src/calibration/hourly.rs`

- [ ] **Step 1: Add failing tests**

Inside the existing `mod tests` block in `src/calibration/hourly.rs`, append:

```rust
    #[test]
    fn smooth_rolling_circular_passes_through_dense_data() {
        let mut raw = [Some(100.0); 24];
        raw[12] = Some(200.0);
        // 3-bin median at hour 12: median(100, 200, 100) = 100.
        // At hour 11: median(100, 100, 200) = 100. Identical.
        let out = smooth_rolling_circular(&raw, 3);
        assert_eq!(out[12], Some(100.0));
        assert_eq!(out[11], Some(100.0));
    }

    #[test]
    fn smooth_rolling_circular_handles_nones_by_skipping() {
        let mut raw: [Option<f64>; 24] = [None; 24];
        raw[10] = Some(50.0);
        raw[11] = Some(100.0);
        raw[12] = Some(150.0);
        let out = smooth_rolling_circular(&raw, 3);
        // At 11: median(50, 100, 150) = 100.
        assert_eq!(out[11], Some(100.0));
        // At 12: median(100, 150) = 125.
        assert_eq!(out[12], Some(125.0));
        // At 13: only 150 contributes from neighbor at 12. median(150) = 150.
        assert_eq!(out[13], Some(150.0));
    }
```

- [ ] **Step 2: Verify the tests fail**

Run: `cargo test --lib smooth_rolling_circular`
Expected: compilation errors — "cannot find function `smooth_rolling_circular`".

- [ ] **Step 3: Implement `smooth_rolling_circular`**

Add to `src/calibration/hourly.rs`:

```rust
/// Circular rolling median over a 24-bin array. Window size 3 means each bin
/// gets the median of itself and its two neighbors (with wrap). None values
/// are skipped.
pub fn smooth_rolling_circular(raw: &[Option<f64>; 24], window: usize) -> [Option<f64>; 24] {
    let half = (window / 2) as isize;
    let n = 24isize;
    let mut out: [Option<f64>; 24] = [None; 24];
    for i in 0..24isize {
        let mut neighbors: Vec<f64> = Vec::new();
        for offset in -half..=half {
            let j = ((i + offset) % n + n) % n;
            if let Some(v) = raw[j as usize] {
                neighbors.push(v);
            }
        }
        if neighbors.is_empty() {
            continue;
        }
        neighbors.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let m = neighbors.len();
        out[i as usize] = Some(if m % 2 == 1 {
            neighbors[m / 2]
        } else {
            (neighbors[m / 2 - 1] + neighbors[m / 2]) / 2.0
        });
    }
    out
}
```

- [ ] **Step 4: Verify the tests pass**

Run: `cargo test --lib smooth_rolling_circular`
Expected: 2 passed.

- [ ] **Step 5: Commit**

```bash
git add src/calibration/hourly.rs
git commit -m "feat(stage-5): implement smooth_rolling_circular for hourly bins"
```

---

## Task 22: Implement `interpolate_empty_circular` + `hour_of_day_cap_series`

**Files:**
- Modify: `src/calibration/hourly.rs`

- [ ] **Step 1: Add failing tests**

Inside the existing `mod tests` block, append:

```rust
    #[test]
    fn interpolate_empty_circular_all_none_returns_zeros() {
        let raw: [Option<f64>; 24] = [None; 24];
        let out = interpolate_empty_circular(&raw);
        assert_eq!(out, [0.0; 24]);
    }

    #[test]
    fn interpolate_empty_circular_fills_gaps_linearly() {
        let mut raw: [Option<f64>; 24] = [None; 24];
        raw[0] = Some(100.0);
        raw[6] = Some(700.0);
        let out = interpolate_empty_circular(&raw);
        // Between bin 0 (100) and bin 6 (700), bin 3 should be exactly halfway = 400.
        assert_eq!(out[0], 100.0);
        assert_eq!(out[6], 700.0);
        assert!((out[3] - 400.0).abs() < 0.001);
    }

    #[test]
    fn hour_of_day_cap_series_empty_returns_zeros() {
        let out = hour_of_day_cap_series(&[], &[], WindowKind::FiveHour);
        assert_eq!(out, [0.0; 24]);
    }
```

- [ ] **Step 2: Verify the tests fail**

Run: `cargo test --lib interpolate_empty_circular`
Expected: compilation errors.

- [ ] **Step 3: Implement both functions**

Add to `src/calibration/hourly.rs`:

```rust
/// Linear-interpolate `None` bins across the array, with circular wrap.
/// If all bins are `None`, returns `[0.0; 24]`.
pub fn interpolate_empty_circular(smoothed: &[Option<f64>; 24]) -> [f64; 24] {
    let n: isize = 24;
    let any = smoothed.iter().any(|v| v.is_some());
    if !any {
        return [0.0; 24];
    }
    let mut out = [0.0f64; 24];
    for h in 0..24usize {
        if let Some(v) = smoothed[h] {
            out[h] = v;
            continue;
        }
        // Search backward for nearest non-None.
        let mut prev: Option<(usize, isize)> = None;
        for off in 1..=n {
            let j = ((h as isize - off) % n + n) % n;
            if smoothed[j as usize].is_some() {
                prev = Some((j as usize, off));
                break;
            }
        }
        // Search forward for nearest non-None.
        let mut next: Option<(usize, isize)> = None;
        for off in 1..=n {
            let j = ((h as isize + off) % n + n) % n;
            if smoothed[j as usize].is_some() {
                next = Some((j as usize, off));
                break;
            }
        }
        out[h] = match (prev, next) {
            (Some((pi, pd)), Some((ni, nd))) => {
                let pv = smoothed[pi].unwrap();
                let nv = smoothed[ni].unwrap();
                let total = (pd + nd) as f64;
                pv * (nd as f64 / total) + nv * (pd as f64 / total)
            }
            (Some((pi, _)), None) => smoothed[pi].unwrap(),
            (None, Some((ni, _))) => smoothed[ni].unwrap(),
            (None, None) => 0.0,
        };
    }
    out
}

/// Public entry point: per-hour median → 3-bin circular smoothing → interpolation.
/// Returns `[0.0; 24]` if no valid anchors exist.
pub fn hour_of_day_cap_series(
    log: &[CalibrationSample],
    turns: &[Turn],
    kind: WindowKind,
) -> [f64; 24] {
    let raw = per_hour_medians(log, turns, kind);
    let smoothed = smooth_rolling_circular(&raw, 3);
    interpolate_empty_circular(&smoothed)
}
```

- [ ] **Step 4: Verify the tests pass**

Run: `cargo test --lib calibration::hourly`
Expected: all hourly tests pass.

- [ ] **Step 5: Commit**

```bash
git add src/calibration/hourly.rs
git commit -m "feat(stage-5): finalize hourly cap series with interpolation + public API"
```

---

## Task 23: Implement `live_util_now`

**Files:**
- Modify: `src/calibration/live.rs`

- [ ] **Step 1: Add failing tests**

Replace the empty body of `src/calibration/live.rs` with:

```rust
//! Live util for the tooltip.

use crate::calibration::anchors::{five_hour_burn_at, weekly_burn_at, DerivedCaps};
use crate::data::parser::Turn;
use chrono::{DateTime, Utc};

/// Current local utilization, in [0.0, ∞). `None` means "uncalibrated" — i.e.
/// the corresponding cap in `DerivedCaps` is `None`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LiveUtil {
    pub util_5h: Option<f64>,
    pub util_week: Option<f64>,
}

/// Compute the current util against the supplied caps. `now` is passed in for
/// testability — production callers should use `live_util_now`.
pub fn live_util_at(turns: &[Turn], caps: &DerivedCaps, now: DateTime<Utc>) -> LiveUtil {
    LiveUtil {
        util_5h: caps
            .cap_5h
            .map(|c| five_hour_burn_at(turns, now) as f64 / c),
        util_week: caps
            .cap_week
            .map(|c| weekly_burn_at(turns, now) as f64 / c),
    }
}

/// Convenience wrapper using `Utc::now()`.
pub fn live_util_now(turns: &[Turn], caps: &DerivedCaps) -> LiveUtil {
    live_util_at(turns, caps, Utc::now())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::parser::Turn;
    use chrono::{TimeZone, Utc};
    use std::path::PathBuf;

    fn turn(ts: chrono::DateTime<Utc>, output: u64) -> Turn {
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
    fn live_util_at_no_caps_returns_no_util() {
        let caps = DerivedCaps::default();
        let now = Utc.with_ymd_and_hms(2026, 5, 24, 12, 0, 0).unwrap();
        let live = live_util_at(&[], &caps, now);
        assert_eq!(live.util_5h, None);
        assert_eq!(live.util_week, None);
    }

    #[test]
    fn live_util_at_with_caps_returns_burn_over_cap() {
        let caps = DerivedCaps {
            cap_5h: Some(1000.0),
            cap_week: Some(10_000.0),
            n_anchors_5h: 1,
            n_anchors_week: 1,
        };
        let now = Utc.with_ymd_and_hms(2026, 5, 24, 12, 0, 0).unwrap();
        let turns = vec![turn(Utc.with_ymd_and_hms(2026, 5, 24, 11, 0, 0).unwrap(), 250)];
        let live = live_util_at(&turns, &caps, now);
        assert_eq!(live.util_5h, Some(0.25));
        // Weekly window includes the same turn.
        assert_eq!(live.util_week, Some(250.0 / 10_000.0));
    }
}
```

- [ ] **Step 2: Verify the tests pass**

Run: `cargo test --lib calibration::live`
Expected: 2 passed.

- [ ] **Step 3: Commit**

```bash
git add src/calibration/live.rs
git commit -m "feat(stage-5): implement live_util_now/live_util_at"
```

---

## Task 24: Extend `PollEvent::Ok` with calibration fields

**Files:**
- Modify: `src/tray/poller.rs`

- [ ] **Step 1: Update the `PollEvent` enum**

Replace the `PollEvent` definition in `src/tray/poller.rs` with:

```rust
use crate::calibration::anchors::DerivedCaps;
use crate::calibration::live::LiveUtil;

/// Calibration outputs attached to a successful poll.
#[derive(Debug, Clone, Default)]
pub struct PollCalibration {
    pub caps: DerivedCaps,
    pub live: LiveUtil,
    pub hourly_5h: [f64; 24],
    pub hourly_week: [f64; 24],
}

/// One outcome of a single poll attempt. Sent from the polling thread to the
/// UI thread via mpsc.
#[derive(Debug)]
pub enum PollEvent {
    Ok {
        snap: UsageSnapshot,
        calib: PollCalibration,
    },
    RateLimited,
    Error(String),
}
```

- [ ] **Step 2: Update existing usages**

The match arm in `polling_loop` previously did `Ok(snap) => PollEvent::Ok(snap)`. Replace it with:

```rust
        let event = match poll_once(&creds) {
            Ok(snap) => PollEvent::Ok {
                snap,
                calib: PollCalibration::default(),
            },
            Err(FetchError::RateLimited) => PollEvent::RateLimited,
            Err(other) => PollEvent::Error(other.to_string()),
        };
```

(Task 25 will fill in `calib` with real data; for now we send default.)

- [ ] **Step 3: Update `tray::window::drain_and_redraw`**

In `src/tray/window.rs`, find the `PollEvent::Ok(snap)` match arm in `drain_and_redraw` and change it to:

```rust
            PollEvent::Ok { snap, calib: _ } => {
                state.last_sample = Some((snap, Utc::now()));
                state.last_status = LastStatus::Ok;
            }
```

(Task 26 wires up the `calib` field; for now we destructure-and-ignore.)

- [ ] **Step 4: Verify it compiles**

Run: `cargo build && cargo test`
Expected: builds + all existing tests still pass.

- [ ] **Step 5: Commit**

```bash
git add src/tray/poller.rs src/tray/window.rs
git commit -m "refactor(stage-5): extend PollEvent::Ok with PollCalibration field"
```

---

## Task 25: Wire cache + caps + live util into the polling loop

**Files:**
- Modify: `src/tray/poller.rs`

- [ ] **Step 1: Add the calibration step before the API fetch**

Replace the body of `polling_loop` in `src/tray/poller.rs` with:

```rust
fn polling_loop(
    creds: Credentials,
    interval: Duration,
    shutdown: Arc<AtomicBool>,
    hwnd: SendHwnd,
    tx: Sender<PollEvent>,
) {
    tracing::info!(
        interval_secs = interval.as_secs(),
        "polling thread starting"
    );

    while !shutdown.load(Ordering::Relaxed) {
        let fetch_at = Instant::now();

        // Stage 5: refresh local cache + derive caps + live util.
        let calib = compute_calibration();

        // API fetch.
        let event = match poll_once(&creds) {
            Ok(snap) => PollEvent::Ok { snap, calib },
            Err(FetchError::RateLimited) => PollEvent::RateLimited,
            Err(other) => PollEvent::Error(other.to_string()),
        };

        // If the UI thread has already dropped the receiver, send fails — we
        // simply exit the loop on the next shutdown check.
        let _ = tx.send(event);

        // Wake the UI thread to drain the channel.
        // SAFETY: PostMessageW is thread-safe; the HWND is valid until shutdown.
        unsafe {
            let _ = PostMessageW(hwnd.0, WM_APP_POLL, WPARAM(0), LPARAM(0));
        }

        sleep_interruptible(&shutdown, fetch_at, interval);
    }

    tracing::info!("polling thread exiting");
}

/// Refresh cache, read calibration log, derive caps, compute live util + hourly.
/// On any error returns `PollCalibration::default()` so the poll itself still proceeds.
fn compute_calibration() -> PollCalibration {
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
            return PollCalibration::default();
        }
    };
    let log = match log_calib::read_all_default() {
        Ok(l) => l,
        Err(e) => {
            tracing::warn!(error = %e, "calibration log read failed; skipping calibration this tick");
            return PollCalibration::default();
        }
    };

    let caps = derive_caps(&log, &turns);
    let hourly_5h = hour_of_day_cap_series(&log, &turns, WindowKind::FiveHour);
    let hourly_week = hour_of_day_cap_series(&log, &turns, WindowKind::Weekly);
    let live = live_util_now(&turns, &caps);

    tracing::debug!(
        n_anchors_5h = caps.n_anchors_5h,
        n_anchors_week = caps.n_anchors_week,
        cap_5h = ?caps.cap_5h,
        cap_week = ?caps.cap_week,
        "calibration computed"
    );

    PollCalibration {
        caps,
        live,
        hourly_5h,
        hourly_week,
    }
}
```

- [ ] **Step 2: Verify it compiles**

Run: `cargo build && cargo test`
Expected: builds + all tests pass.

- [ ] **Step 3: Commit**

```bash
git add src/tray/poller.rs
git commit -m "feat(stage-5): wire cache + caps + live util into polling loop"
```

---

## Task 26: Add calibration fields to `TrayState` + plumb into tooltip

**Files:**
- Modify: `src/tray/window.rs`

- [ ] **Step 1: Add fields to `TrayState`**

Update the `TrayState` struct in `src/tray/window.rs`:

```rust
use crate::calibration::anchors::DerivedCaps;
use crate::calibration::live::LiveUtil;

/// State carried inside the window via GWLP_USERDATA.
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
}
```

- [ ] **Step 2: Initialize the new fields in `src/tray/mod.rs`**

Update the `Box::new(window::TrayState { ... })` construction in `src/tray/mod.rs::run`:

```rust
    let state = Box::new(window::TrayState {
        last_sample: None,
        last_status: LastStatus::Initial,
        renderer,
        current_hicon: None,
        rx,
        shutdown: shutdown.clone(),
        last_caps: None,
        last_local_util: None,
        last_hourly_5h: None,
        last_hourly_week: None,
    });
```

- [ ] **Step 3: Update the `PollEvent::Ok` arm in `drain_and_redraw`**

In `src/tray/window.rs`, the match arm now reads:

```rust
            PollEvent::Ok { snap, calib } => {
                state.last_sample = Some((snap, Utc::now()));
                state.last_status = LastStatus::Ok;
                state.last_caps = Some(calib.caps);
                state.last_local_util = Some(calib.live);
                state.last_hourly_5h = Some(calib.hourly_5h);
                state.last_hourly_week = Some(calib.hourly_week);
            }
```

- [ ] **Step 4: Verify it compiles**

Run: `cargo build && cargo test`
Expected: builds + all tests pass.

- [ ] **Step 5: Commit**

```bash
git add src/tray/window.rs src/tray/mod.rs
git commit -m "feat(stage-5): add calibration fields to TrayState"
```

---

## Task 27: Render local util lines in the tooltip

**Files:**
- Modify: `src/tray/window.rs`

- [ ] **Step 1: Update `format_tooltip` signature**

Find `format_tooltip` in `src/tray/window.rs` and update its signature + body. Replace the existing function with:

```rust
/// Format the tooltip text (UTF-16, null-terminated, <=127 chars per szTip cap).
pub(crate) fn format_tooltip(
    status: &LastStatus,
    last_sample: Option<&(UsageSnapshot, DateTime<Utc>)>,
    local: Option<&LiveUtil>,
    now: DateTime<Utc>,
) -> Vec<u16> {
    let text = match (last_sample, status) {
        (None, LastStatus::Initial) => "Claude usage tray\nfetching\u{2026}".to_string(),
        (None, LastStatus::RateLimited) => {
            "5h: --   7d: --\nno data yet (rate-limited)".to_string()
        }
        (None, LastStatus::Error(msg)) => {
            format!("5h: --   7d: --\nno data yet ({})", short(msg))
        }
        (None, LastStatus::Ok) => "Claude usage tray\nfetching\u{2026}".to_string(),
        (Some((snap, sample_at)), st) => {
            let h5 = snap
                .five_hour
                .as_ref()
                .map(|b| format!("{}%", (b.utilization * 100.0).round() as i64))
                .unwrap_or_else(|| "--".to_string());
            let d7 = snap
                .seven_day
                .as_ref()
                .map(|b| format!("{}%", (b.utilization * 100.0).round() as i64))
                .unwrap_or_else(|| "--".to_string());
            let updated = sample_at.with_timezone(&Local).format("%H:%M");
            let footer = match st {
                LastStatus::Ok => "(Ok)".to_string(),
                LastStatus::Initial => "(fetching)".to_string(),
                LastStatus::RateLimited => format!(
                    "(stale {})",
                    format_duration(ChronoDuration::seconds(
                        (now - *sample_at).num_seconds().max(0)
                    ))
                ),
                LastStatus::Error(msg) => format!("(error: {})", short(msg)),
            };
            let local_line = format_local_line(local);
            format!("5h: {h5}   7d: {d7}\n{local_line}\nupdated {updated} {footer}")
        }
    };
    encode_utf16(&text)
}

fn format_local_line(local: Option<&LiveUtil>) -> String {
    match local {
        None => "local: (uncalibrated)".to_string(),
        Some(l) => {
            let f = |u: Option<f64>| match u {
                Some(v) => format!("{}%", (v * 100.0).round() as i64),
                None => "(uncalibrated)".to_string(),
            };
            format!("local 5h: {}   local 7d: {}", f(l.util_5h), f(l.util_week))
        }
    }
}
```

- [ ] **Step 2: Update all callers of `format_tooltip`**

There are three call sites in `src/tray/window.rs`. Update each to pass `state.last_local_util.as_ref()`:

In `drain_and_redraw`, the early-return path (renderer failed):
```rust
            let tooltip = format_tooltip(
                &state.last_status,
                state.last_sample.as_ref(),
                state.last_local_util.as_ref(),
                Utc::now(),
            );
```

And the main path:
```rust
    let tooltip = format_tooltip(
        &state.last_status,
        state.last_sample.as_ref(),
        state.last_local_util.as_ref(),
        Utc::now(),
    );
```

There's also a call in `src/tray/mod.rs::run`. Update it:

```rust
    let initial_tooltip =
        window::format_tooltip(&LastStatus::Initial, None, None, chrono::Utc::now());
```

- [ ] **Step 3: Add an import of `LiveUtil` at the top of `src/tray/window.rs`**

(Already added in Task 26; verify it's there: `use crate::calibration::live::LiveUtil;`.)

- [ ] **Step 4: Verify it compiles + tests pass**

Run: `cargo build && cargo test`
Expected: builds + all tests pass.

- [ ] **Step 5: Add a unit test for `format_local_line`**

At the bottom of `src/tray/window.rs`, add:

```rust
#[cfg(test)]
mod tooltip_tests {
    use super::*;

    #[test]
    fn format_local_line_none_says_uncalibrated() {
        assert_eq!(format_local_line(None), "local: (uncalibrated)");
    }

    #[test]
    fn format_local_line_both_caps_prints_both_pcts() {
        let live = LiveUtil {
            util_5h: Some(0.54),
            util_week: Some(0.40),
        };
        assert_eq!(
            format_local_line(Some(&live)),
            "local 5h: 54%   local 7d: 40%"
        );
    }

    #[test]
    fn format_local_line_partial_caps_prints_uncalibrated_per_window() {
        let live = LiveUtil {
            util_5h: Some(0.54),
            util_week: None,
        };
        assert_eq!(
            format_local_line(Some(&live)),
            "local 5h: 54%   local 7d: (uncalibrated)"
        );
    }
}
```

- [ ] **Step 6: Verify the new tests pass**

Run: `cargo test --lib format_local_line`
Expected: 3 passed.

- [ ] **Step 7: Commit**

```bash
git add src/tray/window.rs src/tray/mod.rs
git commit -m "feat(stage-5): render local util lines in tooltip"
```

---

## Task 28: Final verification, version bump, manual smoke test

**Files:**
- Modify: `Cargo.toml`

- [ ] **Step 1: Run fmt + clippy + tests cleanly**

```bash
cargo fmt --all
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

Expected: all clean. If clippy complains about anything Stage 5-introduced, fix in place and re-run.

- [ ] **Step 2: Build release binary**

```bash
cargo build --release
```

Expected: `target/release/claude-usage-tray.exe` exists.

- [ ] **Step 3: Manual smoke test**

```bash
.\target\release\claude-usage-tray.exe
```

Verify by hovering the tray icon:
- Tooltip shows `5h: NN%   7d: NN%` (API values, as before).
- Tooltip's second line shows either `local 5h: NN%   local 7d: NN%` OR `local 5h: (uncalibrated)   local 7d: (uncalibrated)` depending on whether your calibration log has ≥0.95 anchors yet.
- Footer line still shows `updated HH:MM (Ok)`.
- Icon glyph + color still matches Stage 4 (uses API util, not local — by design).

Verify files were created:
```powershell
ls $env:USERPROFILE\.claude-usage-tray\
```

Expected: `cache.bincode`, `cache_manifest.json`, `calibration_log.jsonl`, `logs/`.

Force a cache rebuild:
```powershell
# Truncate cache.bincode to 4 bytes.
$null | Set-Content -Encoding Byte -Path "$env:USERPROFILE\.claude-usage-tray\cache.bincode"
```

Wait for the next poll (≤2 minutes), then check the log file:
```powershell
Get-Content "$env:USERPROFILE\.claude-usage-tray\logs\*.log" -Tail 30
```

Expected: a `tracing::warn` line about the corrupt cache + a `tracing::debug` line about calibration recomputed.

Quit the app via tray right-click → Quit. Verify it exits cleanly (no orphan process in Task Manager).

- [ ] **Step 4: Bump version to 0.5.0**

Edit `Cargo.toml`:

```toml
version = "0.5.0"
```

Run `cargo build` once to update `Cargo.lock`.

- [ ] **Step 5: Commit the version bump**

```bash
git add Cargo.toml Cargo.lock
git commit -m "release: bump version to 0.5.0"
```

- [ ] **Step 6: Tag the release**

```bash
git tag -a v0.5.0 -m "Stage 5 — calibration math + local cache"
git push origin main
git push origin v0.5.0
```

- [ ] **Step 7: Update `CLAUDE.md`**

Update the stage roadmap table in `CLAUDE.md` to mark Stage 5 as shipped:

```markdown
| 5 | Calibration math (port from Python's `caps.global_cap_from_anchors`) | ✅ Shipped — tag `v0.5.0`, pushed to GitHub |
```

And add the Stage 5 spec + plan rows to the "Active design + plans" list:

```markdown
- **Stage 5 spec:** `docs/superpowers/specs/2026-05-23-stage-5-calibration-design.md` — calibration math + local cache design details.
- **Stage 5 plan:** `docs/superpowers/plans/2026-05-23-stage-5-calibration.md` — task plan. **Shipped 2026-05-23 (tag `v0.5.0`).**
```

Commit:

```bash
git add CLAUDE.md
git commit -m "docs: mark Stage 5 shipped in CLAUDE.md"
git push origin main
```

---

## Summary of test counts

Before Stage 5: 15 tests (Stages 1–4).

Added by Stage 5:
- `classify_subagent` — 4
- `walk_jsonl` — 2
- `iter_rows` — 2 (integration)
- `cache::refresh_at` — 5 (integration)
- `read_all` — 1 (integration)
- `last_weekly_reset` — 4
- `five_hour_burn_at` — 3
- `weekly_burn_at` — 2
- `global_cap_from_anchors` — 3
- `per_hour_medians` — 2
- `smooth_rolling_circular` — 2
- `interpolate_empty_circular` + `hour_of_day_cap_series` — 3
- `live_util_at` — 2
- `format_local_line` — 3

Total Stage 5: 38 new tests. Grand total: 53 tests.
