# Stage 2 — Polling Daemon Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a long-running `--watch` mode to the Stage 1 CLI: poll `/api/oauth/usage` at a configurable cadence, render a redraw-in-place live view in the terminal, and append each successful sample to `~/.claude-usage-tray/calibration_log.jsonl` for Stage 5 calibration to consume.

**Architecture:** Two pure modules (`render` produces a frame string, `log::calibration` writes JSONL records) sit underneath an I/O-heavy `watch::run` loop. `clap` (derive API) replaces hand-parsed args; `tracing` writes structured logs to stderr while stdout is reserved for the live view. State (last good sample, last poll status, last frame line count) lives in-memory only — no persistence across restarts.

**Tech Stack:** Rust stable, `clap` v4 derive, `tracing` + `tracing-subscriber`, `serde` + `serde_json` (already present), `tempfile` (dev-dep for tests). Continues the Stage 1 module style: pure functions tested in `tests/`, I/O wrappers smoke-tested by running.

---

## Context for a Rust beginner

This stage introduces several new patterns. Brief one-liners:

- **`clap` derive macro:** put `#[derive(Parser)]` on a struct, annotate each field with `#[arg(long)]`, then `Cli::parse()` parses `std::env::args()`. The struct fields become the parsed values.
- **`ValueEnum`:** clap's way of accepting one of a fixed set of string values for an argument (`--interval 60|120|300`).
- **`ArgGroup`:** declarative mutual-exclusion for flags. We use it to say "exactly one of `--once` or `--watch` must be supplied."
- **`tracing` macros:** `tracing::info!("text")`, `tracing::warn!(?err, "context")`. The `?err` syntax includes `err`'s Debug representation in the structured event.
- **`tracing-subscriber`:** the thing that decides where tracing events go (stderr in our case) and what level passes the filter.
- **`std::thread::sleep` + `Instant::now()`:** blocking sleep. `Instant` is the monotonic clock for measuring elapsed time (immune to wall-clock jumps).
- **`OpenOptions::new().append(true).create(true)`:** open a file for appending; create it if it doesn't exist. One handle per call — opens, writes, closes when dropped.
- **`thiserror` `#[from]`:** auto-derives a `From<X> for MyError` so `?` can convert `Result<T, X>` to `Result<T, MyError>` transparently. Less boilerplate than hand-writing the conversion.
- **`tempfile::TempDir`:** RAII-managed temporary directory for tests. When the `TempDir` is dropped, the directory is deleted.
- **ANSI escape sequences:** `"\x1b[5A"` moves the terminal cursor up 5 lines; `"\x1b[J"` clears from cursor to end of screen. Windows Terminal supports these natively.

---

## Files this stage will create or modify

```
claude-usage-tray/
├── Cargo.toml                     (modify — new deps)
├── docs/superpowers/specs/
│   └── 2026-05-22-rust-tray-widget-design.md   (modify — .json → .jsonl)
├── src/
│   ├── main.rs                    (modify — clap dispatch + tracing init)
│   ├── lib.rs                     (modify — declare new modules)
│   ├── cli.rs                     (create — Parser struct + ValueEnum)
│   ├── paths.rs                   (create — ~/.claude-usage-tray/ helpers)
│   ├── render.rs                  (create — draw_frame pure fn + LastStatus enum)
│   ├── watch.rs                   (create — the polling loop)
│   ├── api/                       (unchanged from Stage 1)
│   └── log/
│       ├── mod.rs                 (create — declares submodules)
│       └── calibration.rs         (create — JSONL writer)
└── tests/
    ├── calibration_log_test.rs    (create)
    └── render_test.rs             (create)
```

---

## Task 1: Add dependencies

**Files:**
- Modify: `Cargo.toml`

- [ ] **Step 1: Edit `[dependencies]` and `[dev-dependencies]`**

Open `Cargo.toml`. Update the dependency sections to read:

```toml
[dependencies]
ureq = { version = "2.10", features = ["json"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
chrono = { version = "0.4", features = ["serde"] }
dirs = "5"
anyhow = "1"
thiserror = "1"
clap = { version = "4", features = ["derive"] }
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter", "fmt"] }

[dev-dependencies]
tempfile = "3"
```

Three new runtime deps (`clap`, `tracing`, `tracing-subscriber`) and one new dev-dep (`tempfile`). All other lines unchanged.

- [ ] **Step 2: Verify the build compiles with the new deps**

```powershell
cargo build
```

Expected: cargo downloads the new crates (~30s on first build), then `Finished` with no errors. The binary still runs Stage 1 behavior — we haven't wired the new deps in yet.

- [ ] **Step 3: Commit**

```powershell
git add Cargo.toml Cargo.lock
git commit -m "build: add clap, tracing, tracing-subscriber, tempfile deps"
```

(If `Cargo.lock` was previously gitignored, we want it tracked now — committing the lockfile pins exact versions for reproducible Stage 2 builds.)

---

## Task 2: CLI module with clap derive

**Files:**
- Create: `src/cli.rs`
- Modify: `src/lib.rs` (declare the new module)
- Modify: `src/main.rs` (parse args via clap; `--once` works, `--watch` errors with "not yet implemented")

This task gets clap parsing the new flags and dispatching, but the watch loop is a stub. We'll fill it in Task 7.

- [ ] **Step 1: Create `src/cli.rs`**

```rust
use clap::{ArgGroup, Parser, ValueEnum};

/// Polling interval choices for `--watch`. Values constrained to keep us
/// above the ~1 req/min rate limit of the /api/oauth/usage endpoint.
#[derive(Copy, Clone, Debug, ValueEnum)]
pub enum Interval {
    #[value(name = "60")]
    I60,
    #[value(name = "120")]
    I120,
    #[value(name = "300")]
    I300,
}

impl Interval {
    pub fn as_secs(self) -> u64 {
        match self {
            Self::I60 => 60,
            Self::I120 => 120,
            Self::I300 => 300,
        }
    }
}

#[derive(Parser, Debug)]
#[command(version, about = "Native Windows tray widget for Claude Code usage tracking.")]
#[command(group(
    ArgGroup::new("mode")
        .required(true)
        .multiple(false)
        .args(["once", "watch"])
))]
pub struct Cli {
    /// Fetch once, print, exit.
    #[arg(long)]
    pub once: bool,

    /// Loop forever with a live-redraw view in the terminal.
    #[arg(long)]
    pub watch: bool,

    /// Polling interval (only used with --watch). One of: 60, 120, 300.
    #[arg(long, value_enum, default_value_t = Interval::I120)]
    pub interval: Interval,

    /// Log level: trace | debug | info | warn | error.
    #[arg(long, default_value = "info")]
    pub log_level: String,
}
```

Notes for a Rust beginner:
- `#[command(group(...))]` declares a clap `ArgGroup` over the struct. `required = true` means clap rejects calls with neither flag; `multiple = false` rejects calls with both.
- `default_value_t = Interval::I120` lets clap use a typed default for the `ValueEnum` (vs `default_value` which takes a string).
- Each `#[arg(long)]` makes the field a `--snake_case` flag based on its name (Rust convention: `log_level` → `--log-level`).

- [ ] **Step 2: Declare the module in `src/lib.rs`**

Open `src/lib.rs`. Currently it reads:

```rust
pub mod api;
```

Add a line so it reads:

```rust
pub mod api;
pub mod cli;
```

- [ ] **Step 3: Replace `src/main.rs` to dispatch via clap**

Replace the entire contents of `src/main.rs` with:

```rust
use anyhow::Result;
use chrono::{DateTime, Duration, Utc};
use clap::Parser;
use claude_usage_tray::api::credentials::{load_from_default_path, Credentials};
use claude_usage_tray::api::usage::{fetch_usage, UsageBucket, UsageSnapshot};
use claude_usage_tray::cli::Cli;

fn main() -> Result<()> {
    let cli = Cli::parse();

    if cli.once {
        run_once()?;
    } else if cli.watch {
        anyhow::bail!("--watch not yet implemented (Task 7 wires this up)");
    }
    Ok(())
}

fn run_once() -> Result<()> {
    let creds = load_from_default_path()?;
    let snap = fetch_usage(&creds)?;
    print_snapshot(&snap, &creds);
    Ok(())
}

fn print_snapshot(snap: &UsageSnapshot, creds: &Credentials) {
    let now = Utc::now();
    if let Some(b) = &snap.five_hour {
        println!("5h: {}", format_bucket(b, now));
    } else {
        println!("5h: (no data)");
    }
    if let Some(b) = &snap.seven_day {
        println!("7d: {}", format_bucket(b, now));
    } else {
        println!("7d: (no data)");
    }
    println!(
        "sub: {} / tier: {}",
        creds.subscription_type, creds.rate_limit_tier
    );
}

fn format_bucket(b: &UsageBucket, now: DateTime<Utc>) -> String {
    let pct = (b.utilization * 100.0).round() as i64;
    match b.resets_at {
        Some(when) => format!("{}% (resets in {})", pct, format_duration(when - now)),
        None => format!("{}% (no reset time)", pct),
    }
}

fn format_duration(d: Duration) -> String {
    let secs = d.num_seconds().max(0);
    let days = secs / 86_400;
    let hours = (secs % 86_400) / 3600;
    let mins = (secs % 3600) / 60;
    if days > 0 {
        format!("{}d {}h", days, hours)
    } else if hours > 0 {
        format!("{}h {}m", hours, mins)
    } else {
        format!("{}m", mins)
    }
}
```

What changed vs Stage 1:
- The hand-parsed `args.iter().any(...)` block is gone — clap handles `--help`, `--version`, and validation.
- `print_help` is gone — clap auto-generates help from the doc comments and `#[command(about = ...)]`.
- The Stage-1 `print_snapshot` / `format_bucket` / `format_duration` functions are unchanged (they'll be reused by `render::draw_frame` in Task 6, so don't delete them yet — we'll refactor in Task 6).

- [ ] **Step 4: Smoke-test `--once`**

```powershell
cargo run -- --once
```

Expected: live 5h/7d output, same as Stage 1.

- [ ] **Step 5: Smoke-test `--help` (auto-generated by clap)**

```powershell
cargo run -- --help
```

Expected: clap-generated help. Should mention `--once`, `--watch`, `--interval`, `--log-level`, `--help`, `--version`. The `--interval` line should show `[default: 120] [possible values: 60, 120, 300]`.

- [ ] **Step 6: Smoke-test missing-mode rejection**

```powershell
cargo run
```

Expected: clap prints `error: the following required arguments were not provided:` and exits non-zero. The mutual-exclusion group is enforcing "must specify one of --once or --watch".

- [ ] **Step 7: Smoke-test `--watch` stub**

```powershell
cargo run -- --watch
```

Expected: prints `Error: --watch not yet implemented (Task 7 wires this up)` and exits non-zero. Confirms the dispatch reached the right branch.

- [ ] **Step 8: Commit**

```powershell
git add Cargo.toml src/cli.rs src/lib.rs src/main.rs
git commit -m "feat(cli): clap derive parser with --once/--watch dispatch"
```

---

## Task 3: Install the tracing subscriber

**Files:**
- Modify: `src/main.rs` (add `init_tracing` called from `main`)

`tracing` events are no-ops until a subscriber is registered. We register one early in `main` so any module that uses `tracing::warn!` later has somewhere for events to go.

- [ ] **Step 1: Add an `init_tracing` function to `src/main.rs`**

At the top of `src/main.rs`, add an import:

```rust
use tracing_subscriber::EnvFilter;
```

Then add this function below `main`:

```rust
fn init_tracing(level: &str) {
    // `RUST_LOG` env var (if set) takes precedence over --log-level.
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(level));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .with_target(false)
        .init();
}
```

What this does:
- `EnvFilter::try_from_default_env()` reads `RUST_LOG`; if unset, we fall back to the CLI flag value.
- `.with_writer(std::io::stderr)` sends events to stderr — explicit and important; the default is stdout, which would collide with the live view.
- `.with_target(false)` drops the `claude_usage_tray::watch:` module path from each log line — less visual noise for a CLI.

- [ ] **Step 2: Call `init_tracing` at the top of `main`**

Modify `main` so the first statement after `Cli::parse()` is the tracing init:

```rust
fn main() -> Result<()> {
    let cli = Cli::parse();
    init_tracing(&cli.log_level);

    if cli.once {
        run_once()?;
    } else if cli.watch {
        anyhow::bail!("--watch not yet implemented (Task 7 wires this up)");
    }
    Ok(())
}
```

- [ ] **Step 3: Sprinkle a trace event in `run_once` to confirm wiring**

In `src/main.rs`, modify `run_once` to add a `tracing::info!` after fetching:

```rust
fn run_once() -> Result<()> {
    let creds = load_from_default_path()?;
    let snap = fetch_usage(&creds)?;
    tracing::info!(
        five_hour = ?snap.five_hour.as_ref().map(|b| b.utilization),
        seven_day = ?snap.seven_day.as_ref().map(|b| b.utilization),
        "fetched usage snapshot"
    );
    print_snapshot(&snap, &creds);
    Ok(())
}
```

The `?` prefix on a field name tells `tracing` to capture the Debug form of the value into the structured event.

- [ ] **Step 4: Smoke-test default log level**

```powershell
cargo run -- --once
```

Expected: stderr shows one line like
```
2026-05-22T14:23:01.123456Z  INFO fetched usage snapshot five_hour=Some(0.57) seven_day=Some(0.57)
```
followed by the existing 3-line snapshot output on stdout. The two streams interleave in the terminal but go through separate pipes — you can redirect them independently (`cargo run -- --once 2>tracing.log` keeps stdout clean).

- [ ] **Step 5: Smoke-test `--log-level debug`**

```powershell
cargo run -- --once --log-level debug
```

Expected: same INFO line at minimum. (We don't have any DEBUG events yet, but the filter is now accepting them.)

- [ ] **Step 6: Smoke-test `--log-level warn` suppresses the INFO event**

```powershell
cargo run -- --once --log-level warn
```

Expected: only the stdout output; no tracing lines on stderr.

- [ ] **Step 7: Commit**

```powershell
git add src/main.rs
git commit -m "feat(tracing): stderr subscriber with EnvFilter + --log-level"
```

---

## Task 4: paths module — ~/.claude-usage-tray/ helpers

**Files:**
- Create: `src/paths.rs`
- Modify: `src/lib.rs` (declare module)

A tiny module that resolves the per-user data directory and ensures it exists before writing. Splitting this out keeps later modules from each rolling their own `dirs::home_dir()` calls.

- [ ] **Step 1: Create `src/paths.rs`**

```rust
use anyhow::{Context, Result};
use std::path::PathBuf;

const APP_DIR_NAME: &str = ".claude-usage-tray";

/// Returns ~/.claude-usage-tray/. Does NOT create the directory.
pub fn app_dir() -> Result<PathBuf> {
    let home = dirs::home_dir().context("could not determine home directory")?;
    Ok(home.join(APP_DIR_NAME))
}

/// Returns ~/.claude-usage-tray/calibration_log.jsonl. Does NOT create the file.
pub fn calibration_log_path() -> Result<PathBuf> {
    Ok(app_dir()?.join("calibration_log.jsonl"))
}

/// Ensures the parent directory of `path` exists, creating it (and ancestors) if needed.
/// Idempotent: no-op if already present.
pub fn ensure_parent_dir(path: &std::path::Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("could not create directory {}", parent.display()))?;
    }
    Ok(())
}
```

Notes for a Rust beginner:
- `&str` is a borrowed string slice; `const APP_DIR_NAME: &str = ...` is the idiomatic way to define a compile-time string constant.
- `.with_context(|| format!(...))` defers the `format!` call until an error actually happens — saves work in the happy path.
- We split `app_dir()` from `calibration_log_path()` so future stages can ask for `app_dir().join("logs/app.log")`, `app_dir().join("state.json")`, etc., without each redefining the home-dir lookup.

- [ ] **Step 2: Declare the module in `src/lib.rs`**

Update `src/lib.rs` to read:

```rust
pub mod api;
pub mod cli;
pub mod paths;
```

- [ ] **Step 3: Confirm it compiles**

```powershell
cargo build
```

Expected: `Finished` with no errors. We won't wire `paths` in until Task 5.

- [ ] **Step 4: Commit**

```powershell
git add src/paths.rs src/lib.rs
git commit -m "feat(paths): ~/.claude-usage-tray/ resolution + lazy mkdir helper"
```

---

## Task 5: Calibration log writer (TDD)

**Files:**
- Create: `src/log/mod.rs`
- Create: `src/log/calibration.rs`
- Create: `tests/calibration_log_test.rs`
- Modify: `src/lib.rs` (declare `log` module)

The writer is two pure functions (build a sample, write a record to a path) plus a glue function (default path). Tests cover the first two; the glue is one line and exercised by smoke-testing the watch loop later.

- [ ] **Step 1: Declare the `log` module in `src/lib.rs`**

```rust
pub mod api;
pub mod cli;
pub mod log;
pub mod paths;
```

Note: `log` is a real Rust crate name (different ecosystem) but our project doesn't depend on it, so the name collision is harmless. If you ever pull in the `log` crate later, you'll need to rename this module.

- [ ] **Step 2: Create `src/log/mod.rs`**

```rust
pub mod calibration;
```

- [ ] **Step 3: Create the test fixture-free test file `tests/calibration_log_test.rs`**

```rust
use chrono::{TimeZone, Utc};
use claude_usage_tray::api::credentials::Credentials;
use claude_usage_tray::api::usage::{UsageBucket, UsageSnapshot};
use claude_usage_tray::log::calibration::{append, sample_from, CalibrationSample};
use tempfile::TempDir;

fn fake_creds() -> Credentials {
    Credentials {
        access_token: "irrelevant".to_string(),
        subscription_type: "pro".to_string(),
        rate_limit_tier: "default_claude_ai".to_string(),
    }
}

fn fake_snapshot() -> UsageSnapshot {
    UsageSnapshot {
        five_hour: Some(UsageBucket {
            utilization: 0.56,
            resets_at: Some(Utc.with_ymd_and_hms(2026, 5, 22, 17, 0, 0).unwrap()),
        }),
        seven_day: Some(UsageBucket {
            utilization: 0.42,
            resets_at: Some(Utc.with_ymd_and_hms(2026, 5, 24, 5, 0, 0).unwrap()),
        }),
    }
}

#[test]
fn sample_from_maps_all_fields() {
    let snap = fake_snapshot();
    let creds = fake_creds();
    let s = sample_from(&snap, &creds);

    assert_eq!(s.schema_version, 1);
    assert!((s.five_hour_util.unwrap() - 0.56).abs() < 1e-9);
    assert!((s.seven_day_util.unwrap() - 0.42).abs() < 1e-9);
    assert!(s.five_hour_resets_at.is_some());
    assert!(s.seven_day_resets_at.is_some());
    assert_eq!(s.subscription_type, "pro");
    assert_eq!(s.rate_limit_tier, "default_claude_ai");
}

#[test]
fn sample_from_handles_missing_buckets() {
    let snap = UsageSnapshot {
        five_hour: None,
        seven_day: None,
    };
    let s = sample_from(&snap, &fake_creds());

    assert!(s.five_hour_util.is_none());
    assert!(s.five_hour_resets_at.is_none());
    assert!(s.seven_day_util.is_none());
    assert!(s.seven_day_resets_at.is_none());
}

#[test]
fn append_writes_one_line_per_call_and_round_trips() {
    let dir = TempDir::new().expect("tempdir");
    let path = dir.path().join("log.jsonl");

    let s1 = sample_from(&fake_snapshot(), &fake_creds());
    let s2 = sample_from(&fake_snapshot(), &fake_creds());

    append(&path, &s1).expect("first append");
    append(&path, &s2).expect("second append");

    let raw = std::fs::read_to_string(&path).expect("read back");
    let lines: Vec<&str> = raw.lines().collect();
    assert_eq!(lines.len(), 2, "expected exactly 2 lines, got: {raw}");

    let parsed: CalibrationSample =
        serde_json::from_str(lines[0]).expect("first line parses");
    assert_eq!(parsed.schema_version, 1);
    assert_eq!(parsed.subscription_type, "pro");
    assert!((parsed.five_hour_util.unwrap() - 0.56).abs() < 1e-9);
}

#[test]
fn append_creates_parent_directory_lazily() {
    let dir = TempDir::new().expect("tempdir");
    let nested = dir.path().join("does/not/exist/yet/log.jsonl");

    let s = sample_from(&fake_snapshot(), &fake_creds());
    append(&nested, &s).expect("should create dirs and write");

    assert!(nested.exists(), "expected file to exist at {}", nested.display());
}
```

- [ ] **Step 4: Run tests — confirm they fail to compile**

```powershell
cargo test --test calibration_log_test
```

Expected: `unresolved import 'claude_usage_tray::log'` or `cannot find ... in module 'log'`. The implementation isn't written yet.

- [ ] **Step 5: Create `src/log/calibration.rs` with the implementation**

```rust
use crate::api::credentials::Credentials;
use crate::api::usage::UsageSnapshot;
use crate::paths;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;
use thiserror::Error;

const SCHEMA_VERSION: u32 = 1;

/// One record per JSONL line in ~/.claude-usage-tray/calibration_log.jsonl.
/// Field naming mirrors Anthropic's API (`five_hour`/`seven_day`) for clarity.
#[derive(Debug, Clone, Serialize, Deserialize)]
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

#[derive(Debug, Error)]
pub enum LogError {
    #[error("io error writing calibration log: {0}")]
    Io(#[from] std::io::Error),

    #[error("serialization failed: {0}")]
    Serde(#[from] serde_json::Error),
}

/// Pure: build a sample record from a fresh snapshot + the active credentials.
pub fn sample_from(snap: &UsageSnapshot, creds: &Credentials) -> CalibrationSample {
    CalibrationSample {
        schema_version: SCHEMA_VERSION,
        ts: Utc::now(),
        five_hour_util: snap.five_hour.as_ref().map(|b| b.utilization),
        five_hour_resets_at: snap.five_hour.as_ref().and_then(|b| b.resets_at),
        seven_day_util: snap.seven_day.as_ref().map(|b| b.utilization),
        seven_day_resets_at: snap.seven_day.as_ref().and_then(|b| b.resets_at),
        subscription_type: creds.subscription_type.clone(),
        rate_limit_tier: creds.rate_limit_tier.clone(),
    }
}

/// I/O: append one JSONL record to `path`. Creates parent dirs and the file
/// if needed. Each call serializes, writes one line + `\n`, then flushes.
pub fn append(path: &Path, sample: &CalibrationSample) -> Result<(), LogError> {
    paths::ensure_parent_dir(path).map_err(|e| {
        LogError::Io(std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))
    })?;

    let line = serde_json::to_string(sample)?;

    let mut file = OpenOptions::new()
        .append(true)
        .create(true)
        .open(path)?;

    file.write_all(line.as_bytes())?;
    file.write_all(b"\n")?;
    file.flush()?;
    Ok(())
}

/// Glue: convenience wrapper for the watch loop. Resolves the default path,
/// builds a sample, and appends in one call.
pub fn append_to_default_path(
    snap: &UsageSnapshot,
    creds: &Credentials,
) -> Result<(), LogError> {
    let path = paths::calibration_log_path().map_err(|e| {
        LogError::Io(std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))
    })?;
    let sample = sample_from(snap, creds);
    append(&path, &sample)
}
```

Notes for a Rust beginner:
- `#[from]` on the enum variants auto-generates `From<std::io::Error> for LogError` and `From<serde_json::Error> for LogError`. That's why we can write `?` on `serde_json::to_string(sample)?` and the `?` operator silently converts the error into a `LogError::Serde`.
- We wrap `paths::ensure_parent_dir`'s `anyhow::Error` in a `LogError::Io` by stringifying it. Slightly clunky but keeps `LogError` self-contained — callers don't need to depend on `anyhow` to pattern-match on it.
- `snap.five_hour.as_ref().map(|b| b.utilization)` reads as: borrow the `Option<UsageBucket>`, and if it's `Some`, project the borrowed bucket through `.utilization` (a `Copy`-able `f64`).

- [ ] **Step 6: Run tests — all four should pass**

```powershell
cargo test --test calibration_log_test
```

Expected:
```
running 4 tests
test sample_from_handles_missing_buckets ... ok
test sample_from_maps_all_fields ... ok
test append_creates_parent_directory_lazily ... ok
test append_writes_one_line_per_call_and_round_trips ... ok

test result: ok. 4 passed
```

- [ ] **Step 7: Commit**

```powershell
git add src/log/ src/lib.rs tests/calibration_log_test.rs
git commit -m "feat(log): calibration JSONL writer with TDD coverage"
```

---

## Task 6: Render module (TDD)

**Files:**
- Create: `src/render.rs`
- Create: `tests/render_test.rs`
- Modify: `src/lib.rs` (declare module)
- Modify: `src/main.rs` (Stage 1's `format_bucket` / `format_duration` move into `render.rs`)

The renderer is pure: given current state, produce a `Frame { body, line_count }`. The watch loop will handle ANSI cursor repositioning around the body separately.

- [ ] **Step 1: Declare `render` in `src/lib.rs`**

```rust
pub mod api;
pub mod cli;
pub mod log;
pub mod paths;
pub mod render;
```

- [ ] **Step 2: Create `tests/render_test.rs`**

```rust
use chrono::{TimeZone, Utc};
use claude_usage_tray::api::credentials::Credentials;
use claude_usage_tray::api::usage::{UsageBucket, UsageSnapshot};
use claude_usage_tray::render::{draw_frame, LastStatus};

fn fake_creds() -> Credentials {
    Credentials {
        access_token: "x".to_string(),
        subscription_type: "pro".to_string(),
        rate_limit_tier: "default_claude_ai".to_string(),
    }
}

fn fake_snapshot() -> UsageSnapshot {
    UsageSnapshot {
        five_hour: Some(UsageBucket {
            utilization: 0.57,
            // 2 hours, 12 minutes from `now` below
            resets_at: Some(Utc.with_ymd_and_hms(2026, 5, 22, 16, 36, 1).unwrap()),
        }),
        seven_day: Some(UsageBucket {
            utilization: 0.57,
            // 1 day, 21 hours from `now` below
            resets_at: Some(Utc.with_ymd_and_hms(2026, 5, 24, 11, 24, 1).unwrap()),
        }),
    }
}

#[test]
fn ok_frame_includes_percent_and_reset_countdown_and_status_tag() {
    let creds = fake_creds();
    let snap = fake_snapshot();
    let now = Utc.with_ymd_and_hms(2026, 5, 22, 14, 24, 1).unwrap();
    let f = draw_frame(
        Some(&(snap, now)),
        &creds,
        120,
        &LastStatus::Ok,
        now,
    );

    assert!(f.body.contains("5h: 57%"), "body was:\n{}", f.body);
    assert!(f.body.contains("2h 12m"), "body was:\n{}", f.body);
    assert!(f.body.contains("7d: 57%"), "body was:\n{}", f.body);
    assert!(f.body.contains("1d 21h"), "body was:\n{}", f.body);
    assert!(f.body.contains("sub: pro / tier: default_claude_ai"));
    assert!(f.body.contains("[Ok]"));
    assert!(f.line_count >= 5);
}

#[test]
fn initial_frame_shows_fetching_placeholder() {
    let creds = fake_creds();
    let now = Utc.with_ymd_and_hms(2026, 5, 22, 14, 24, 1).unwrap();
    let f = draw_frame(None, &creds, 120, &LastStatus::Initial, now);

    assert!(f.body.contains("fetching"));
    // Even when no sample is available, header + footer + sub line should print.
    assert!(f.line_count >= 3);
}

#[test]
fn rate_limited_status_shows_stale_footer_with_last_good_sample() {
    let creds = fake_creds();
    let snap = fake_snapshot();
    let sample_taken_at = Utc.with_ymd_and_hms(2026, 5, 22, 14, 22, 1).unwrap();
    let now = Utc.with_ymd_and_hms(2026, 5, 22, 14, 24, 1).unwrap(); // 2 min later
    let f = draw_frame(
        Some(&(snap, sample_taken_at)),
        &creds,
        120,
        &LastStatus::RateLimited,
        now,
    );

    assert!(f.body.contains("5h: 57%"), "should still show the cached sample");
    assert!(f.body.contains("stale"), "footer should indicate staleness");
    assert!(f.body.contains("rate-limited"), "footer should explain why");
}
```

- [ ] **Step 3: Run tests — confirm they fail to compile**

```powershell
cargo test --test render_test
```

Expected: `unresolved import claude_usage_tray::render`. The module doesn't exist yet.

- [ ] **Step 4: Create `src/render.rs` with the implementation**

```rust
use crate::api::credentials::Credentials;
use crate::api::usage::{UsageBucket, UsageSnapshot};
use chrono::{DateTime, Duration, Utc};
use std::fmt::Write;

/// Result of running one render pass. `body` is the printable text;
/// `line_count` is the number of lines `body` occupies (= number of `\n` chars).
/// The watch loop uses `line_count` to compute the ANSI cursor-up escape for
/// the next frame.
#[derive(Debug, Clone)]
pub struct Frame {
    pub body: String,
    pub line_count: u16,
}

/// Status badge shown in the footer of each frame.
#[derive(Debug, Clone)]
pub enum LastStatus {
    /// Before the first poll completes.
    Initial,
    /// Most recent poll succeeded.
    Ok,
    /// Most recent poll was HTTP 429.
    RateLimited,
    /// Most recent poll failed with some other error.
    Error(String),
}

/// Pure: build the on-screen frame for one tick.
/// - `last_success`: the most recent successful sample + when it was received, or None if no poll has succeeded yet.
/// - `interval_secs`: polling cadence, shown in the header.
/// - `status`: badge for the footer.
/// - `now`: current time (passed in for testability — production passes `Utc::now()`).
pub fn draw_frame(
    last_success: Option<&(UsageSnapshot, DateTime<Utc>)>,
    creds: &Credentials,
    interval_secs: u64,
    status: &LastStatus,
    now: DateTime<Utc>,
) -> Frame {
    let mut body = String::new();
    let mut lines: u16 = 0;

    // Header.
    writeln!(
        body,
        "claude-usage-tray  watching ({}s)  press Ctrl-C to quit",
        interval_secs
    )
    .unwrap();
    lines += 1;
    writeln!(body).unwrap();
    lines += 1;

    // Body.
    match last_success {
        Some((snap, _)) => {
            writeln!(body, "  5h: {}", format_bucket_opt(snap.five_hour.as_ref(), now)).unwrap();
            writeln!(body, "  7d: {}", format_bucket_opt(snap.seven_day.as_ref(), now)).unwrap();
            lines += 2;
        }
        None => {
            writeln!(body, "  5h: (fetching…)").unwrap();
            writeln!(body, "  7d: (fetching…)").unwrap();
            lines += 2;
        }
    }
    writeln!(
        body,
        "  sub: {} / tier: {}",
        creds.subscription_type, creds.rate_limit_tier
    )
    .unwrap();
    lines += 1;

    // Footer.
    let footer = format_footer(last_success.map(|(_, t)| *t), interval_secs, status, now);
    writeln!(body, "  {}", footer).unwrap();
    lines += 1;

    Frame { body, line_count: lines }
}

fn format_bucket_opt(b: Option<&UsageBucket>, now: DateTime<Utc>) -> String {
    match b {
        Some(bucket) => format_bucket(bucket, now),
        None => "(no data)".to_string(),
    }
}

fn format_bucket(b: &UsageBucket, now: DateTime<Utc>) -> String {
    let pct = (b.utilization * 100.0).round() as i64;
    match b.resets_at {
        Some(when) => format!("{}% resets in {}", pct, format_duration(when - now)),
        None => format!("{}% (no reset time)", pct),
    }
}

pub(crate) fn format_duration(d: Duration) -> String {
    let secs = d.num_seconds().max(0);
    let days = secs / 86_400;
    let hours = (secs % 86_400) / 3600;
    let mins = (secs % 3600) / 60;
    if days > 0 {
        format!("{}d {}h", days, hours)
    } else if hours > 0 {
        format!("{}h {}m", hours, mins)
    } else {
        format!("{}m", mins)
    }
}

fn format_footer(
    last_sample_at: Option<DateTime<Utc>>,
    interval_secs: u64,
    status: &LastStatus,
    now: DateTime<Utc>,
) -> String {
    let last_str = match last_sample_at {
        Some(t) => format!("last poll: {}", t.format("%H:%M:%S")),
        None => "last poll: —".to_string(),
    };
    let next_str = match last_sample_at {
        Some(t) => {
            let next = t + Duration::seconds(interval_secs as i64);
            format!("next: {}", next.format("%H:%M:%S"))
        }
        None => "next: —".to_string(),
    };

    let badge = match status {
        LastStatus::Initial => "[fetching…]".to_string(),
        LastStatus::Ok => "[Ok]".to_string(),
        LastStatus::RateLimited => {
            let age = last_sample_at
                .map(|t| format_duration(now - t))
                .unwrap_or_else(|| "?".to_string());
            format!("[stale {} · rate-limited]", age)
        }
        LastStatus::Error(msg) => {
            let age = last_sample_at
                .map(|t| format_duration(now - t))
                .unwrap_or_else(|| "?".to_string());
            format!("[stale {} · error: {}]", age, msg)
        }
    };

    format!("{}  {}  {}", last_str, next_str, badge)
}
```

Notes for a Rust beginner:
- `std::fmt::Write` lets us `writeln!(body, ...)` into a `String`. The `.unwrap()` is safe because writing to a `String` cannot fail.
- `pub(crate)` makes `format_duration` visible to other modules in this crate but not to external users — Stage 1's `main.rs` will import it.
- `LastStatus::Error(msg)` carries a description; the watch loop passes `other.to_string()` from the matched `FetchError`.

- [ ] **Step 5: Simplify `src/main.rs` by importing the shared formatters from `render`**

In `src/main.rs`:
- Delete the `format_bucket` and `format_duration` functions (they now live in `render`).
- Delete the `print_snapshot` function and inline a small one-shot formatter that reuses `render::format_duration`.

Replace the body of `src/main.rs` with:

```rust
use anyhow::Result;
use chrono::Utc;
use clap::Parser;
use claude_usage_tray::api::credentials::{load_from_default_path, Credentials};
use claude_usage_tray::api::usage::{fetch_usage, UsageBucket, UsageSnapshot};
use claude_usage_tray::cli::Cli;
use claude_usage_tray::render::format_duration;
use tracing_subscriber::EnvFilter;

fn main() -> Result<()> {
    let cli = Cli::parse();
    init_tracing(&cli.log_level);

    if cli.once {
        run_once()?;
    } else if cli.watch {
        anyhow::bail!("--watch not yet implemented (Task 7 wires this up)");
    }
    Ok(())
}

fn init_tracing(level: &str) {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(level));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .with_target(false)
        .init();
}

fn run_once() -> Result<()> {
    let creds = load_from_default_path()?;
    let snap = fetch_usage(&creds)?;
    tracing::info!(
        five_hour = ?snap.five_hour.as_ref().map(|b| b.utilization),
        seven_day = ?snap.seven_day.as_ref().map(|b| b.utilization),
        "fetched usage snapshot"
    );
    print_snapshot(&snap, &creds);
    Ok(())
}

fn print_snapshot(snap: &UsageSnapshot, creds: &Credentials) {
    let now = Utc::now();
    if let Some(b) = &snap.five_hour {
        println!("5h: {}", format_one(b, now));
    } else {
        println!("5h: (no data)");
    }
    if let Some(b) = &snap.seven_day {
        println!("7d: {}", format_one(b, now));
    } else {
        println!("7d: (no data)");
    }
    println!(
        "sub: {} / tier: {}",
        creds.subscription_type, creds.rate_limit_tier
    );
}

fn format_one(b: &UsageBucket, now: chrono::DateTime<Utc>) -> String {
    let pct = (b.utilization * 100.0).round() as i64;
    match b.resets_at {
        Some(when) => format!("{}% (resets in {})", pct, format_duration(when - now)),
        None => format!("{}% (no reset time)", pct),
    }
}
```

- [ ] **Step 6: Run all tests — render + everything else passes**

```powershell
cargo test
```

Expected: 6 Stage-1 tests + 4 calibration tests + 3 render tests = 13 passing. Order may vary.

- [ ] **Step 7: Smoke-test that `--once` still produces Stage-1-shaped output**

```powershell
cargo run -- --once
```

Expected: same 3-line snapshot on stdout, optional INFO on stderr. (We refactored but didn't change behavior.)

- [ ] **Step 8: Commit**

```powershell
git add src/render.rs src/lib.rs src/main.rs tests/render_test.rs
git commit -m "feat(render): pure draw_frame + LastStatus, shared with --once"
```

---

## Task 7: Watch loop

**Files:**
- Create: `src/watch.rs`
- Modify: `src/lib.rs` (declare module)
- Modify: `src/main.rs` (dispatch `--watch` to `watch::run`)

The loop has no unit tests (spec calls this out). We smoke-test by running.

- [ ] **Step 1: Declare `watch` in `src/lib.rs`**

```rust
pub mod api;
pub mod cli;
pub mod log;
pub mod paths;
pub mod render;
pub mod watch;
```

- [ ] **Step 2: Create `src/watch.rs`**

```rust
use crate::api::credentials::{load_from_default_path, Credentials};
use crate::api::usage::{fetch_usage, FetchError, UsageSnapshot};
use crate::log::calibration::append_to_default_path;
use crate::render::{draw_frame, Frame, LastStatus};
use anyhow::Result;
use chrono::{DateTime, Utc};
use std::io::Write;
use std::thread;
use std::time::{Duration, Instant};

struct WatchState {
    last_success: Option<(UsageSnapshot, DateTime<Utc>)>,
    last_status: LastStatus,
    last_line_count: u16,
}

pub fn run(interval_secs: u64) -> Result<()> {
    let creds = load_from_default_path()?;
    let mut state = WatchState {
        last_success: None,
        last_status: LastStatus::Initial,
        last_line_count: 0,
    };

    tracing::info!(interval_secs, "watch loop starting");

    loop {
        let fetch_at = Instant::now();
        tick(&creds, &mut state);
        // `redraw` returns the new line_count so the next frame's cursor-up math is correct.
        // If the write fails (broken pipe, etc.), keep the prior count.
        state.last_line_count =
            redraw(&state, &creds, interval_secs).unwrap_or(state.last_line_count);
        sleep_until_next(fetch_at, interval_secs);
    }
}

fn tick(creds: &Credentials, state: &mut WatchState) {
    match fetch_usage(creds) {
        Ok(snap) => {
            // Append BEFORE updating in-memory state so a log failure doesn't
            // make us forget the fresh sample (we never propagate log errors).
            if let Err(e) = append_to_default_path(&snap, creds) {
                tracing::warn!(error = %e, "calibration log write failed");
            }
            state.last_success = Some((snap, Utc::now()));
            state.last_status = LastStatus::Ok;
        }
        Err(FetchError::RateLimited) => {
            tracing::warn!("rate limited; keeping last sample on screen");
            state.last_status = LastStatus::RateLimited;
        }
        Err(other) => {
            tracing::warn!(error = ?other, "poll failed");
            state.last_status = LastStatus::Error(other.to_string());
        }
    }
}

fn redraw(state: &WatchState, creds: &Credentials, interval_secs: u64) -> std::io::Result<u16> {
    let frame: Frame = draw_frame(
        state.last_success.as_ref(),
        creds,
        interval_secs,
        &state.last_status,
        Utc::now(),
    );

    let prefix = if state.last_line_count == 0 {
        String::new()
    } else {
        // Move cursor up N lines, then clear from cursor to end of screen.
        format!("\x1b[{}A\x1b[J", state.last_line_count)
    };

    let mut stdout = std::io::stdout().lock();
    write!(stdout, "{}{}", prefix, frame.body)?;
    stdout.flush()?;

    Ok(frame.line_count)
}

fn sleep_until_next(fetch_at: Instant, interval_secs: u64) {
    let target = fetch_at + Duration::from_secs(interval_secs);
    let now = Instant::now();
    if target > now {
        thread::sleep(target - now);
    }
    // If `target <= now` (fetch took longer than the interval), don't sleep — loop again immediately.
}
```

Notes for a Rust beginner:
- `&mut state` in `tick` lets the function mutate the state struct in place.
- `std::io::stdout().lock()` acquires a buffered handle; writing through it is buffered until `flush()` so the whole frame paints atomically.
- `.unwrap_or(state.last_line_count)` says: if the draw failed (broken pipe, etc.), keep the prior count so the next frame still tries to reposition correctly.
- The loop is `loop { ... }` — infinite. Termination is `Ctrl-C` from the user, which the OS translates to a process kill. No graceful shutdown logic (deferred to Stage 3).

- [ ] **Step 3: Wire `--watch` in `src/main.rs`**

Update the `if/else` in `main`:

```rust
if cli.once {
    run_once()?;
} else if cli.watch {
    claude_usage_tray::watch::run(cli.interval.as_secs())?;
}
```

(You're replacing the `anyhow::bail!("--watch not yet implemented...")` line.)

- [ ] **Step 4: Smoke-test `--watch` for one tick + Ctrl-C**

```powershell
cargo run -- --watch
```

Expected:
- Within ~1 second, the live view appears with current 5h/7d numbers and `[Ok]` badge.
- Wait ~120 seconds. The screen refreshes in place — no new lines scroll, the existing lines update.
- Press `Ctrl-C`. Process exits.

If the screen scrolls instead of redrawing in place, your terminal might not support ANSI escapes (Windows Terminal does; legacy `cmd.exe` may not).

- [ ] **Step 5: Smoke-test the calibration log appeared**

After at least one full tick:

```powershell
Get-Content $HOME\.claude-usage-tray\calibration_log.jsonl
```

Expected: one or more JSON lines, each with `schema_version`, `ts`, `five_hour_util`, etc. Parse one manually:

```powershell
Get-Content $HOME\.claude-usage-tray\calibration_log.jsonl | Select-Object -First 1 | ConvertFrom-Json
```

You should see a structured object printed.

- [ ] **Step 6: Smoke-test `--watch --interval 60`**

```powershell
cargo run -- --watch --interval 60
```

Expected: header now says `watching (60s)`. Two consecutive 60s ticks against the real endpoint may hit the rate limiter — that's a chance to verify the rate-limited footer. The screen should show `[stale 1m · rate-limited]` next to the prior good sample, NOT crash or scroll.

(If you don't naturally trigger 429, you can simulate it: temporarily edit `src/api/usage.rs::fetch_usage` to return `Err(FetchError::RateLimited)` after the first call, smoke-test, then revert. Don't commit the simulation code.)

- [ ] **Step 7: Smoke-test `--watch --interval 300`**

```powershell
cargo run -- --watch --interval 300
```

Expected: header says `watching (300s)`. Verify one tick then `Ctrl-C`. (No need to wait 5 minutes for the second tick.)

- [ ] **Step 8: Commit**

```powershell
git add src/watch.rs src/lib.rs src/main.rs
git commit -m "feat(watch): polling loop with redraw-in-place live view"
```

---

## Task 8: Release polish — clippy, fmt, top-level spec update, tag v0.2.0

**Files:**
- Modify: `docs/superpowers/specs/2026-05-22-rust-tray-widget-design.md` (`.json` → `.jsonl`)
- (no source changes beyond fmt/clippy fixes)

- [ ] **Step 1: Run `cargo fmt`**

```powershell
cargo fmt
git diff
```

If fmt made changes:

```powershell
git add -u
git commit -m "style: cargo fmt"
```

Otherwise skip the commit.

- [ ] **Step 2: Run clippy and address every warning**

```powershell
cargo clippy --all-targets -- -D warnings
```

Expected: `Finished` with no warnings. If clippy complains, read the suggestion and apply. Common findings on this stage are likely to be:
- "this `match` could be written as `if let`" — accept the suggestion if it reads cleanly.
- "unused imports" — remove them.
- "needless `clone`" — replace with a borrow if the lifetime works.

If you fix anything:

```powershell
git add -u
git commit -m "style: address clippy suggestions"
```

- [ ] **Step 3: Update the top-level spec to reference `.jsonl`**

Open `docs/superpowers/specs/2026-05-22-rust-tray-widget-design.md`.

Find the Stage 2 bullet that reads:

> Persist samples to `~/.claude-usage-tray/calibration_log.json` (append-only).

Replace with:

> Persist samples to `~/.claude-usage-tray/calibration_log.jsonl` (append-only JSON-lines — one record per line). See [`2026-05-22-stage-2-watch-design.md`](2026-05-22-stage-2-watch-design.md) for schema and rationale.

- [ ] **Step 4: Commit the spec update**

```powershell
git add docs/superpowers/specs/2026-05-22-rust-tray-widget-design.md
git commit -m "docs: top-level spec mirrors Stage 2 .jsonl + schema cross-link"
```

- [ ] **Step 5: Full test run + release build**

```powershell
cargo test
cargo build --release
```

Expected:
- All 13 tests pass.
- `target\release\claude-usage-tray.exe` is built (~3 MB).

- [ ] **Step 6: Smoke-test the release binary**

```powershell
.\target\release\claude-usage-tray.exe --once
.\target\release\claude-usage-tray.exe --watch --interval 120
```

(For the second command, observe one tick then `Ctrl-C`.)

Expected: same behavior as `cargo run`, just snappier startup.

- [ ] **Step 7: Tag v0.2.0**

```powershell
git tag -a v0.2.0 -m "Stage 2: polling daemon (--watch, calibration log, tracing)"
git tag --list
```

Expected: both `v0.1.0` and `v0.2.0` appear.

- [ ] **Step 8: Push tag to GitHub**

```powershell
git push origin main
git push origin v0.2.0
```

- [ ] **Step 9: Verify on github.com**

Open https://github.com/borgi-s/claude-usage-tray/tags. The `v0.2.0` tag should appear with its annotation message. The Tags tab shows both v0.1.0 and v0.2.0.

Stage 2 complete. The Stage 3 plan (tray icon) is a separate document written when you're ready.

---

## Verification summary

End-to-end:
- `cargo test` → 13 tests passing (6 from Stage 1 + 4 calibration + 3 render).
- `cargo clippy --all-targets -- -D warnings` → clean.
- `cargo fmt --check` → clean.
- `cargo build --release` → ~3 MB `claude-usage-tray.exe`.
- `.\target\release\claude-usage-tray.exe --once` → Stage-1 behavior preserved.
- `.\target\release\claude-usage-tray.exe --watch` → live redraw, calibration log written.
- `~/.claude-usage-tray/calibration_log.jsonl` exists with at least one parseable JSON line per successful tick.
- `git push origin v0.2.0` → tag visible on GitHub.

---

## Self-review notes

Quick check against the spec:

| Spec Stage 2 requirement | Task |
|---|---|
| `--watch` flag, polls every 60s (configurable 60/120/300, default 120) | Task 2 + Task 7 |
| Handle `RateLimited` (HTTP 429) gracefully with cached last-known state | Task 7 (tick + redraw paths) |
| Add `tracing` for structured logs (stderr only) | Task 3 |
| Persist samples to `~/.claude-usage-tray/calibration_log.jsonl` (append-only) | Task 5 |
| Custom error types — `LogError` via thiserror | Task 5 |
| Tag `v0.2.0` | Task 8 |
| Top-level spec updated to mirror the .jsonl filename | Task 8 |
| Render is unit-tested (pure function) | Task 6 |
| Calibration writer is unit-tested with round-trip | Task 5 |
| Watch loop is smoke-tested, not unit-tested | Task 7 |
| First poll is immediate; cadence anchors to fetch start | Task 7 (`sleep_until_next`) |
| No `crossterm` / `ctrlc` / `tracing-appender` | confirmed by Cargo.toml deltas in Task 1 |
| Calibration log failures logged and swallowed | Task 7 (`if let Err(e) = ...`) |

All spec requirements have at least one task. No placeholders, no `TBD`, no "similar to Task N" references. Types/method signatures consistent across tasks (`CalibrationSample`, `LastStatus`, `Frame`, `WatchState`, `Interval::as_secs`).
