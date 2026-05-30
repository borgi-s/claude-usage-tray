# Linux Collector (Phase 1) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a headless `collector` binary that runs on a Linux server, reads local Claude Code session logs + polls the usage API, and uploads to a distinct Supabase prefix (`borgi-linux`) — without changing the Windows build's behavior.

**Architecture:** The crate's data/sync/api modules are already platform-agnostic; only `tray`, `dashboard`, `autostart`, and `main.rs` use Win32/egui. We move the GUI/Win32 dependencies behind `[target.'cfg(windows)'.dependencies]`, `#[cfg(windows)]`-gate those modules, and add a new `src/bin/collector.rs` that reuses the existing `poll`, `data::cache`, `log::calibration`, and `sync` modules through their public API.

**Tech Stack:** Rust (stable), `ureq` (rustls TLS), `serde`, `arrow`/`parquet`, `chrono`, `dirs`, `clap`, `tracing`. Target: `x86_64-unknown-linux-gnu`.

---

## Background for the implementer (read once)

This is the `claude-usage-tray` repo — a Windows tray widget. We are adding a **second binary** to the same crate so the data-collection half can run on a Linux server. You do **not** need to understand the Win32/egui GUI code; you only touch build config and a handful of platform-agnostic modules.

**Two Rust facts this plan relies on (the engineer may be new to Rust):**

1. **A crate can have many binaries.** `src/main.rs` is the existing GUI binary. Any file under `src/bin/` becomes its *own* additional binary. Each binary is compiled as a **separate crate that depends on the library** (`src/lib.rs`). Consequence: `src/bin/collector.rs` can only call **`pub`** items from the library — `pub(crate)` items are invisible to it. (That's why Task 4 makes one function `pub`.)

2. **`#[cfg(windows)]` is a compile-time switch.** An item marked `#[cfg(windows)]` simply does not exist when compiling for Linux. `[target.'cfg(windows)'.dependencies]` in `Cargo.toml` is the same idea for *dependencies* — those crates are never even downloaded on Linux. We use both to make the Windows-only code vanish cleanly on the server.

**What "done" looks like for Phase 1:** on the Ubuntu server, `cargo build --release --bin collector` succeeds, and running the binary uploads `borgi-linux/cache.parquet`, `borgi-linux/caps.json`, and `borgi-linux/calibration_log.parquet` to Supabase. The existing Windows build is byte-for-byte unchanged.

**Where verification runs:** every `cargo test` step runs on your Windows dev machine. The Windows `cargo build` steps prove we did not break the existing build. The collector binary also builds and runs on Windows (`cargo build --bin collector`), so you can smoke-test it locally. Only the final Linux build + systemd step happens on the server (Task 7, documented, not run from here).

---

## File structure

| File | Change | Responsibility |
|---|---|---|
| `Cargo.toml` | Modify | Move `windows`/`eframe`/`egui`/`egui_plot`/`egui_extras`/`winit` under `[target.'cfg(windows)'.dependencies]`; declare `[[bin]] collector`. |
| `src/lib.rs` | Modify | `#[cfg(windows)]`-gate `tray`, `dashboard`, `autostart` modules. |
| `src/main.rs` | Modify | `#[cfg(windows)]`-gate the whole GUI entry point; add a `#[cfg(not(windows))]` stub `main`. |
| `src/poll.rs` | Modify | Make `poll_once` `pub` so the collector binary can call it. |
| `src/sync/mod.rs` | Modify | Add `Syncer::upload_cache_only` for the poll-failed / Windows cache-only path. |
| `src/bin/collector.rs` | Create | The headless collect+upload loop. |
| `.env.example` | Modify | Document the server `.env` (distinct prefix). |
| `docs/deploy-linux.md` | Create | Server deployment steps + systemd user service. |

---

### Task 1: Move GUI/Win32 dependencies behind a Windows target gate

**Files:**
- Modify: `Cargo.toml`

**Why:** `eframe`, `egui`, `egui_plot`, `egui_extras`, `winit`, and `windows` only compile on Windows in this project. Today they are unconditional `[dependencies]`, so a Linux build would try to compile the whole egui/glow/winit/Win32 stack and fail. Moving them under `[target.'cfg(windows)'.dependencies]` means Cargo never fetches them on Linux. On Windows nothing changes — they're still included.

- [ ] **Step 1: Edit `Cargo.toml` dependencies**

Replace the current `[dependencies]` block (and the `windows = { ... }` entry) so the GUI/Win32 crates live under a Windows-only target table. The full new `[dependencies]` + new target table:

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
arrow = "58.3.0"
parquet = "58.3.0"
dotenvy = "0.15.7"
toml = "0.8"
semver = "1"

# GUI + Win32: only ever compiled for Windows. On Linux these are never fetched,
# so the headless `collector` binary builds without the egui/glow/winit/Win32 stack.
[target.'cfg(windows)'.dependencies]
eframe = { version = "0.29", default-features = false, features = ["default_fonts", "glow"] }
egui = "0.29"
egui_plot = "0.29"
egui_extras = { version = "0.29", features = ["datepicker"] }
winit = { version = "0.30", features = [] }
windows = { version = "0.58", features = [
    "Win32_Foundation",
    "Win32_Graphics_Gdi",
    "Win32_Graphics_GdiPlus",
    "Win32_System_Console",
    "Win32_System_LibraryLoader",
    "Win32_System_Registry",
    "Win32_UI_Accessibility",
    "Win32_UI_Shell",
    "Win32_UI_WindowsAndMessaging",
] }
```

- [ ] **Step 2: Declare the collector binary**

Add this near the bottom of `Cargo.toml` (after `[dependencies]`/target tables, before or after `[dev-dependencies]` — order doesn't matter):

```toml
[[bin]]
name = "collector"
path = "src/bin/collector.rs"
```

> Note: the existing GUI binary (`src/main.rs`) is still auto-detected and named after the package (`claude-usage-tray`); you do not need to declare it explicitly.

- [ ] **Step 3: Verify the Windows build is unchanged**

Run: `cargo build`
Expected: builds successfully (the collector binary doesn't exist yet, so Cargo will warn `can't find 'collector' bin` OR error that the path is missing — that's expected at this step; if it errors, that's fine, proceed to create a placeholder in the next sub-step).

If `cargo build` errors because `src/bin/collector.rs` is missing, create a one-line placeholder so the manifest is valid:

```bash
mkdir src/bin
printf 'fn main() {}\n' > src/bin/collector.rs
```

Then re-run `cargo build` — Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add Cargo.toml Cargo.lock src/bin/collector.rs
git commit -m "build: target-gate GUI/Win32 deps; declare collector binary"
```

> We commit `Cargo.lock` together with `Cargo.toml` — moving deps can change the lockfile, and `cargo build` regenerates it silently otherwise.

---

### Task 2: Gate the Windows-only modules in `lib.rs`

**Files:**
- Modify: `src/lib.rs`

**Why:** `tray`, `dashboard`, and `autostart` use the `windows`/`eframe`/`egui` crates that no longer exist on Linux. They must be excluded from the Linux build. They are referenced only by each other and by `main.rs` (a binary, gated in Task 3) — no platform-agnostic module depends on them, so gating them is safe.

- [ ] **Step 1: Edit `src/lib.rs`**

Add `#[cfg(windows)]` to exactly the three Windows-only module declarations. The full file becomes:

```rust
pub mod api;
#[cfg(windows)]
pub mod autostart;
pub mod calibration;
pub mod cli;
pub mod config;
#[cfg(windows)]
pub mod dashboard;
pub mod data;
pub mod log;
pub mod paths;
pub mod poll;
pub mod render;
pub mod settings;
pub mod shared;
pub mod state;
pub mod sync;
#[cfg(windows)]
pub mod tray;
pub mod updater;
pub mod watch;
```

- [ ] **Step 2: Verify the Windows build still works**

Run: `cargo build`
Expected: PASS (on Windows, `cfg(windows)` is true so all modules are still compiled).

- [ ] **Step 3: Commit**

```bash
git add src/lib.rs
git commit -m "build: cfg(windows)-gate tray/dashboard/autostart modules"
```

---

### Task 3: Gate the GUI entry point in `main.rs`

**Files:**
- Modify: `src/main.rs`

**Why:** `main.rs` imports `windows::Win32::Graphics::GdiPlus`, defines a `GdiplusGuard`, and calls `AttachConsole` — none of which compile on Linux. We gate the entire GUI `main` behind `#[cfg(windows)]` and add a tiny `#[cfg(not(windows))]` stub so `cargo build`/`cargo test` stay green on the server (the real Linux entry point is the separate `collector` binary).

- [ ] **Step 1: Replace `src/main.rs` with the gated version**

Full file:

```rust
#![cfg_attr(windows, windows_subsystem = "windows")]

// On Linux/macOS this binary is not the entry point — the headless `collector`
// binary is. We still provide a stub `main` so `cargo build`/`cargo test`
// succeed on those platforms.
#[cfg(not(windows))]
fn main() {
    eprintln!(
        "claude-usage-tray (GUI) is Windows-only.\n\
         On this platform, run the collector instead:\n  \
         cargo run --release --bin collector -- --once"
    );
    std::process::exit(1);
}

#[cfg(windows)]
use anyhow::Result;
#[cfg(windows)]
use chrono::Utc;
#[cfg(windows)]
use clap::Parser;
#[cfg(windows)]
use claude_usage_tray::api::credentials::{load_from_default_path, Credentials};
#[cfg(windows)]
use claude_usage_tray::api::usage::{fetch_usage, UsageBucket, UsageSnapshot};
#[cfg(windows)]
use claude_usage_tray::cli::Cli;
#[cfg(windows)]
use claude_usage_tray::render::format_duration;
#[cfg(windows)]
use tracing_subscriber::EnvFilter;
#[cfg(windows)]
use windows::Win32::Graphics::GdiPlus::{
    GdiplusShutdown, GdiplusStartup, GdiplusStartupInput, Status,
};

/// RAII guard that initializes GDI+ in `init()` and shuts it down on drop.
/// We hold one for the whole process lifetime so cleanup runs on every exit path
/// (including `?` early-returns and panic unwinding).
#[cfg(windows)]
struct GdiplusGuard(usize);

#[cfg(windows)]
impl GdiplusGuard {
    fn init() -> Result<Self> {
        let mut token: usize = 0;
        let input = GdiplusStartupInput {
            GdiplusVersion: 1,
            ..Default::default()
        };
        // SAFETY: token is on the stack and the input pointer is valid.
        // GdiplusStartup writes the token and returns a Status code.
        let status = unsafe { GdiplusStartup(&mut token, &input, std::ptr::null_mut()) };
        if status != Status(0) {
            anyhow::bail!("GdiplusStartup failed with status {:?}", status);
        }
        Ok(Self(token))
    }
}

#[cfg(windows)]
impl Drop for GdiplusGuard {
    fn drop(&mut self) {
        // SAFETY: token was obtained from a successful GdiplusStartup and we are
        // the sole owner. After shutdown, no more GDI+ calls happen.
        unsafe { GdiplusShutdown(self.0) };
    }
}

#[cfg(windows)]
fn main() -> Result<()> {
    // Attach to parent console (if any) so --once/--watch can still print to a terminal.
    // Harmlessly fails when launched from Explorer.
    let _ = unsafe {
        windows::Win32::System::Console::AttachConsole(
            windows::Win32::System::Console::ATTACH_PARENT_PROCESS,
        )
    };

    let cli = Cli::parse();

    let _gdiplus = GdiplusGuard::init()?;

    if cli.once {
        init_tracing_stderr(&cli.log_level);
        run_once()?;
    } else if cli.watch {
        init_tracing_stderr(&cli.log_level);
        claude_usage_tray::watch::run(cli.interval.as_secs())?;
    } else {
        let _guard = claude_usage_tray::log::tray::init_file_subscriber(&cli.log_level)?;
        claude_usage_tray::tray::run()?;
        // _guard drops at end of this branch → tracing-appender flushes pending events.
    }
    Ok(())
}

#[cfg(windows)]
fn init_tracing_stderr(level: &str) {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(level));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .with_target(false)
        .init();
}

#[cfg(windows)]
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

#[cfg(windows)]
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

#[cfg(windows)]
fn format_one(b: &UsageBucket, now: chrono::DateTime<Utc>) -> String {
    let pct = (b.utilization * 100.0).round() as i64;
    match b.resets_at {
        Some(when) => format!("{}% (resets in {})", pct, format_duration(when - now)),
        None => format!("{}% (no reset time)", pct),
    }
}
```

- [ ] **Step 2: Verify the Windows build still works**

Run: `cargo build`
Expected: PASS. The GUI binary behaves exactly as before (every item is `#[cfg(windows)]`, and you are on Windows).

- [ ] **Step 3: Commit**

```bash
git add src/main.rs
git commit -m "build: cfg(windows)-gate GUI main; add non-Windows stub"
```

---

### Task 4: Make `poll_once` callable from the collector binary

**Files:**
- Modify: `src/poll.rs`

**Why:** The collector binary is a separate crate and can only call `pub` items from the library. `poll_once` is currently `pub(crate)`. Widening it to `pub` lets the collector reuse the exact same poll+calibration-append logic the GUI uses (DRY), instead of duplicating it.

- [ ] **Step 1: Change the visibility**

In `src/poll.rs`, change the function signature line from:

```rust
pub(crate) fn poll_once(creds: &Credentials) -> Result<UsageSnapshot, FetchError> {
```

to:

```rust
pub fn poll_once(creds: &Credentials) -> Result<UsageSnapshot, FetchError> {
```

- [ ] **Step 2: Verify nothing broke**

Run: `cargo build`
Expected: PASS (widening visibility never breaks existing callers).

- [ ] **Step 3: Commit**

```bash
git add src/poll.rs
git commit -m "refactor: make poll_once pub for the collector binary"
```

---

### Task 5: Add a cache-only upload path to `Syncer`

**Files:**
- Modify: `src/sync/mod.rs`

**Why:** Today `Syncer::run_once` always uploads all three objects (cache + caps + calibration). The collector needs a path that uploads **only** `cache.parquet`: it uses this when a poll fails (so a stale/empty caps snapshot never overwrites the last good `caps.json`). This same method is reused by Windows in Phase 3. `run_once`'s private `put_buffer` helper already does the work; we add a small public method that calls it for just the cache object.

- [ ] **Step 1: Write the failing test**

Add this test to the `#[cfg(test)] mod tests` block at the bottom of `src/sync/mod.rs` (the `FakeStore`, `cfg()`, and imports it needs already exist in that module):

```rust
#[test]
fn upload_cache_only_uploads_just_the_cache_object() {
    let syncer = Syncer {
        config: cfg(),
        store: FakeStore::default(),
    };

    syncer.upload_cache_only(&AppSnapshot::default());

    let puts = syncer.store.puts.lock().unwrap();
    let paths: Vec<&str> = puts.iter().map(|(p, _, _)| p.as_str()).collect();
    assert_eq!(paths, vec!["borgi/cache.parquet"]);
    assert_eq!(puts[0].1, "application/octet-stream");
}
```

- [ ] **Step 2: Run it to confirm it fails**

Run: `cargo test --lib upload_cache_only_uploads_just_the_cache_object`
Expected: FAIL — compile error, `no method named upload_cache_only`.

- [ ] **Step 3: Implement `upload_cache_only`**

In `src/sync/mod.rs`, inside `impl<S: ObjectStore> Syncer<S>` (the same block that contains `run_once` and `put_buffer`), add this method right after `run_once`:

```rust
    /// Best-effort: upload ONLY `cache.parquet` (local turns) under the configured
    /// prefix. Used when a poll failed (so we don't overwrite good caps.json with
    /// an empty snapshot) and by the Windows cache-only path. Never returns an error.
    pub fn upload_cache_only(&self, snapshot: &AppSnapshot) {
        self.put_buffer(
            "cache.parquet",
            "application/octet-stream",
            crate::sync::export::cache_parquet(&snapshot.turns),
        );
    }
```

- [ ] **Step 4: Run the test to confirm it passes**

Run: `cargo test --lib upload_cache_only_uploads_just_the_cache_object`
Expected: PASS.

- [ ] **Step 5: Run the full sync test module to confirm no regression**

Run: `cargo test --lib sync::`
Expected: PASS (all existing sync tests still green).

- [ ] **Step 6: Commit**

```bash
git add src/sync/mod.rs
git commit -m "feat(sync): add upload_cache_only for poll-failed / cache-only path"
```

---

### Task 6: Implement the collector binary

**Files:**
- Modify (replace the placeholder): `src/bin/collector.rs`

**Why:** This is the headless loop that runs on the server. Per cycle it: (1) refreshes local turns from `~/.claude/projects` (no token needed); (2) attempts a usage poll; (3) uploads — all three objects on poll success, cache-only on poll failure; (4) sleeps. It supports `--once` (one cycle, exit) for testing and `--interval <secs>` (default 120) for the daemon loop.

**Design notes:**
- It reuses `poll::poll_once` (Task 4), `data::cache::refresh`, `log::calibration::read_all_default`, and `sync::Syncer` (Task 5) — all through the public API. It does **not** touch any `tray`/`dashboard` code.
- The Supabase prefix comes from the server's `.env` (`SUPABASE_USER_PREFIX=borgi-linux`), read by `Syncer::from_env()`. No prefix is hardcoded here.
- Credentials load is fallible (e.g. expired token). If it fails, we still upload turns (cache-only) and skip the poll — the turns pipeline needs no token.

- [ ] **Step 1: Write `src/bin/collector.rs`**

Full file:

```rust
//! Headless Linux collector: refresh local turns, poll the usage API, and
//! upload to Supabase under the prefix from `.env` (e.g. `borgi-linux`).
//!
//! This is a SEPARATE binary from the Windows GUI (`src/main.rs`). It only uses
//! the platform-agnostic library modules, so it builds on x86_64-unknown-linux-gnu.

use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Result;
use clap::Parser;

use claude_usage_tray::api::credentials::{load_from_default_path, Credentials};
use claude_usage_tray::data::cache;
use claude_usage_tray::log::calibration;
use claude_usage_tray::poll::poll_once;
use claude_usage_tray::shared::snapshot::AppSnapshot;
use claude_usage_tray::sync::Syncer;
use tracing_subscriber::EnvFilter;

#[derive(Parser, Debug)]
#[command(version, about = "Headless Claude Code usage collector (Linux server).")]
struct CollectorCli {
    /// Run one collect+upload cycle and exit (for testing).
    #[arg(long)]
    once: bool,

    /// Seconds between cycles in daemon mode. Keep >= 60 to respect the
    /// ~1 req/min usage-endpoint rate limit.
    #[arg(long, default_value_t = 120)]
    interval: u64,

    /// Log level: trace | debug | info | warn | error.
    #[arg(long, default_value = "info")]
    log_level: String,
}

fn main() -> Result<()> {
    let cli = CollectorCli::parse();
    init_tracing(&cli.log_level);

    let syncer = match Syncer::from_env() {
        Ok(Some(s)) => s,
        Ok(None) => {
            anyhow::bail!(
                "Supabase sync is not configured. Create a .env with SUPABASE_URL, \
                 SUPABASE_SERVICE_ROLE_KEY, and SUPABASE_USER_PREFIX in the working directory."
            );
        }
        Err(e) => return Err(e.context("invalid Supabase sync config")),
    };

    if cli.once {
        run_cycle(&syncer);
        return Ok(());
    }

    tracing::info!(interval_secs = cli.interval, "collector starting");
    let interval = Duration::from_secs(cli.interval);
    loop {
        let started = Instant::now();
        run_cycle(&syncer);
        let elapsed = started.elapsed();
        if elapsed < interval {
            std::thread::sleep(interval - elapsed);
        }
    }
}

/// One collect+upload cycle. Best-effort: every failure is logged, never fatal,
/// so the daemon keeps running.
fn run_cycle<S: claude_usage_tray::sync::storage::ObjectStore>(syncer: &Syncer<S>) {
    // 1. Refresh local turns (no token required).
    let turns = match cache::refresh() {
        Ok(t) => t,
        Err(e) => {
            tracing::warn!(error = %e, "cache refresh failed; skipping this cycle");
            return;
        }
    };
    let turns_arc = Arc::new(turns);

    // 2. Try to load creds + poll the usage API. On any failure, fall back to a
    //    cache-only upload so we still push the local turns and never overwrite a
    //    good caps.json with empty data.
    let creds: Option<Credentials> = match load_from_default_path() {
        Ok(c) => Some(c),
        Err(e) => {
            tracing::warn!(error = %e, "credentials unavailable; uploading turns only");
            None
        }
    };

    let poll_ok = match &creds {
        Some(c) => match poll_once(c) {
            Ok(snap) => Some(snap),
            Err(e) => {
                tracing::warn!(error = %e, "usage poll failed; uploading turns only");
                None
            }
        },
        None => None,
    };

    match (poll_ok, &creds) {
        // Poll succeeded: upload all three objects (cache + caps + calibration).
        (Some(snap), Some(c)) => {
            let samples = calibration::read_all_default().unwrap_or_else(|e| {
                tracing::warn!(error = %e, "calibration log read failed; uploading empty samples");
                Vec::new()
            });
            let snapshot = AppSnapshot {
                turns: turns_arc,
                last_sample: Some((snap, chrono::Utc::now())),
                ..Default::default()
            };
            syncer.run_once(&snapshot, c, &samples);
            tracing::info!("cycle complete (full upload)");
        }
        // Poll failed or no creds: upload turns only.
        _ => {
            let snapshot = AppSnapshot {
                turns: turns_arc,
                ..Default::default()
            };
            syncer.upload_cache_only(&snapshot);
            tracing::info!("cycle complete (cache-only upload)");
        }
    }
}

fn init_tracing(level: &str) {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(level));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .with_target(false)
        .init();
}
```

> **Rust note (`chrono`):** `chrono` is already a dependency, so `chrono::Utc::now()` is available without adding a `use`. **`Arc`** (atomically reference-counted pointer) is how `AppSnapshot` holds its `turns`; we wrap the `Vec<Turn>` in `Arc::new(...)` to match the field's type.

> **Visibility check:** this file references `claude_usage_tray::sync::storage::ObjectStore` in the `run_cycle` generic bound. That path is already `pub` (`pub mod storage;` in `src/sync/mod.rs`, `pub trait ObjectStore`). No change needed.

- [ ] **Step 2: Build the collector on Windows (compiles against public API only)**

Run: `cargo build --bin collector`
Expected: PASS. This proves the collector uses only `pub` items and links cleanly. (It builds on Windows too, since it uses only platform-agnostic modules.)

- [ ] **Step 3: Verify the whole workspace still builds and tests pass**

Run: `cargo build` then `cargo test`
Expected: both PASS.

- [ ] **Step 4 (optional local smoke test): run one cycle on Windows**

> Only do this if your dev-machine `.env` points at a **throwaway** prefix, or you're comfortable it writes to your own `borgi` prefix (same thing the tray already uploads — harmless). To be safe, you can temporarily set `SUPABASE_USER_PREFIX=borgi-smoketest` in `.env`.

Run: `cargo run --bin collector -- --once --log-level debug`
Expected: logs `cycle complete (full upload)` (or `cache-only upload` if the API is rate-limited), and the objects appear under that prefix in the Supabase dashboard.

- [ ] **Step 5: Run clippy (release gate)**

Run: `cargo clippy --all-targets -- -D warnings`
Expected: PASS (no warnings). Fix any clippy findings in `collector.rs` before committing.

- [ ] **Step 6: Commit**

```bash
git add src/bin/collector.rs
git commit -m "feat(collector): headless Linux collect+upload binary"
```

---

### Task 7: Server deployment docs (.env template + systemd user service)

**Files:**
- Modify: `.env.example`
- Create: `docs/deploy-linux.md`

**Why:** The server steps cannot be run from this machine (it's remote), so they are delivered as documentation. This task records the exact, safe sequence: a fresh `.env` with a **distinct** prefix (so Windows data is never clobbered), tight file permissions, native Linux build, and a systemd **user** service with linger (survives reboot/logout — unlike the `tmux` session).

- [ ] **Step 1: Update `.env.example` with a per-machine prefix note**

Replace the last two lines of `.env.example` (the `SUPABASE_USER_PREFIX` comment + value) with:

```
# Arbitrary anonymous handle that namespaces THIS machine's files in the bucket.
# Letters, digits, '-', '_' only. MUST be unique per machine so two machines do
# not overwrite each other's objects. Example: Windows uses "borgi", the Linux
# server uses "borgi-linux".
SUPABASE_USER_PREFIX=changeme
```

- [ ] **Step 2: Create `docs/deploy-linux.md`**

Write this file verbatim:

````markdown
# Deploying the collector on a Linux server (Ubuntu 24.04)

This runs the headless `collector` binary as the same user that runs Claude Code,
uploading this machine's usage data to Supabase under a **distinct** prefix so it
never overwrites the Windows machine's data.

## 0. Prerequisites

- Claude Code is already installed and logged in on the server (so
  `~/.claude/.credentials.json` exists and `~/.claude/projects/` is being written).
- You can build Rust on the box: install the toolchain and a linker:
  ```bash
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
  source "$HOME/.cargo/env"
  sudo apt-get update && sudo apt-get install -y build-essential
  ```

## 1. Clone and build

```bash
git clone <your-fork-or-repo-url> ~/claude-usage-tray
cd ~/claude-usage-tray
cargo build --release --bin collector
# binary at: ~/claude-usage-tray/target/release/collector
```

If the build fails on TLS, confirm `ureq` resolved to rustls (it does by default
with `features = ["json"]`); only if it pulled native-tls would you need
`sudo apt-get install -y libssl-dev pkg-config`.

## 2. Create the secrets file (distinct prefix!)

Put the `.env` where the service's working directory will be, NOT inside the git
work tree's tracked area (it is gitignored, but keep it isolated anyway):

```bash
mkdir -p ~/.config/claude-usage-tray
cat > ~/.config/claude-usage-tray/.env <<'EOF'
SUPABASE_URL=https://YOUR_PROJECT_REF.supabase.co
SUPABASE_SERVICE_ROLE_KEY=YOUR_KEY
SUPABASE_BUCKET=usage-tracker
SUPABASE_USER_PREFIX=borgi-linux
EOF
chmod 600 ~/.config/claude-usage-tray/.env
```

- **Do NOT copy the Windows `.env` verbatim** — it sets `SUPABASE_USER_PREFIX=borgi`
  and would make the server clobber the Windows objects every cycle.
- Prefer a Supabase **Storage-scoped** key over the full `service_role` key if you
  can create one; `service_role` bypasses Row Level Security and could delete every
  prefix in the bucket if the box is compromised.

## 3. Smoke test

```bash
cd ~/.config/claude-usage-tray
~/claude-usage-tray/target/release/collector --once --log-level debug
```

The collector loads `.env` from its **current working directory**, so run it from
`~/.config/claude-usage-tray`. Expected: a `cycle complete` log line, and
`borgi-linux/cache.parquet` (+ `caps.json`, `calibration_log.parquet`) appearing in
the Supabase Storage dashboard.

## 4. Run as a systemd user service (survives reboot)

Create `~/.config/systemd/user/claude-collector.service`:

```ini
[Unit]
Description=Claude Code usage collector
After=network-online.target

[Service]
Type=simple
WorkingDirectory=%h/.config/claude-usage-tray
ExecStart=%h/claude-usage-tray/target/release/collector --interval 120
Restart=on-failure
RestartSec=30

[Install]
WantedBy=default.target
```

Enable it and turn on **linger** so it runs without an active login session
(i.e. survives reboot and logout):

```bash
systemctl --user daemon-reload
systemctl --user enable --now claude-collector.service
loginctl enable-linger "$USER"
```

Check status / logs:

```bash
systemctl --user status claude-collector.service
journalctl --user -u claude-collector.service -f
```

## Notes

- The collector is independent of your `tmux new -s claude1` session; you do not
  need tmux open for it to run.
- The poll cadence is `--interval` (default 120s). Keep it >= 60s to respect the
  usage endpoint's ~1 request/minute limit.
- Token freshness: because you actively run Claude Code on the server, it keeps
  `~/.claude/.credentials.json` current. If the token ever expires, the collector
  logs a warning, skips the poll, and still uploads local turns that cycle.
````

- [ ] **Step 3: Verify the docs build nothing (sanity only)**

Run: `cargo build`
Expected: PASS (docs don't affect the build; this just confirms the tree is still clean).

- [ ] **Step 4: Commit**

```bash
git add .env.example docs/deploy-linux.md
git commit -m "docs: Linux server deployment guide + per-machine prefix note"
```

---

## Final verification (run all before declaring Phase 1 done)

- [ ] `cargo build` — PASS (Windows GUI build unchanged)
- [ ] `cargo build --bin collector` — PASS (collector compiles)
- [ ] `cargo test` — PASS (all tests, including new `upload_cache_only` test)
- [ ] `cargo clippy --all-targets -- -D warnings` — PASS (release gate per project conventions)
- [ ] `cargo fmt --check` — PASS (run `cargo fmt` first if needed)

> **Linux build is verified on the server**, per `docs/deploy-linux.md` step 1. That
> step is the first true `x86_64-unknown-linux-gnu` compile and is the one part of
> this plan that cannot be exercised from the Windows dev machine.

## Out of scope for Phase 1 (handled by later plans)

- **Phase 2 — viewer merge** (`claude-usage-tracker`, companion repo): make the Streamlit
  viewer read multiple prefixes and concat `cache.parquet` with a `machine` stamp; take
  caps/calibration from `borgi-linux`. Delivered as a diff for the user to apply. Until then,
  the server's data is visible by pointing `CLOUD_USER_PREFIX` at `borgi-linux`.
- **Phase 3 — Windows switch-over**: add `ObjectStore::get` + a caps.json parser + a
  caps-source-prefix config; repoint the Windows tray poller to download `borgi-linux/caps.json`
  instead of polling the API, and make Windows upload cache-only. This removes the two-poller
  overlap. (Phase 1 tolerates the brief overlap — occasional 429s are handled gracefully.)
