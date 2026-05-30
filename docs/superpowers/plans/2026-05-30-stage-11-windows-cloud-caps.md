# Windows Cloud-Caps Switchover (Phase 3) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. **Dispatch ONE implementer at a time, wait for its commit, review, then the next** — concurrent implementers on this shared checkout cause commit races. Steps use checkbox (`- [ ]`) syntax.

**Goal:** Flip the Windows tray from polling the usage API to reading the account-wide caps from `borgi-linux/caps.json` in Supabase, and make Windows upload only its local turns — so the always-on Linux server is the sole API poller (no two-poller 429 contention) while the Windows tray icon/widget/live-banner keep showing live %.

**Architecture:** Opt-in via a new env var `SUPABASE_CAPS_PREFIX`. When set (e.g. `borgi-linux`) and Supabase sync is configured, the poller thread reads `{caps_prefix}/caps.json` via a new `ObjectStore::get` instead of calling the usage API, parses it into the existing `UsageSnapshot`, and uploads cache-only (`upload_cache_only` from Phase 1). When unset, behavior is identical to today (poll API, upload all three). The switch is reversible by clearing the env var.

**Tech Stack:** Rust (stable), `ureq` (HTTP GET), `serde_json`, `chrono`, `windows` crate (poller is Win32-gated). Same crate as Phases 1–2.

---

## Background for the implementer (read once)

This repo (`claude-usage-tray`) is a Windows tray widget. Phase 1 added a Linux `collector` binary; Phase 2 (companion repo) merges both machines in the Streamlit viewer. **Phase 3 makes Windows stop polling the usage API.**

Why: the usage endpoint is rate-limited to ~1 req/min/account. With the same account on both machines, two pollers contend and 429 each other, re-introducing the gaps the Linux primary is meant to eliminate. So Windows must stop polling; it instead reads the caps the Linux box keeps fresh in `borgi-linux/caps.json`.

**What the tray live surfaces actually display:** the tray icon, taskbar widget, and dashboard live-banner show **utilization %**, which comes from `AppSnapshot.last_sample` (a `UsageSnapshot` with `five_hour`/`seven_day` buckets, each `utilization: f64` in 0.0–1.0). They do NOT need the derived-caps math for the % readout. So if we populate `last_sample` from `caps.json` instead of the API, the live surfaces keep working unchanged.

**The data we read:** `caps.json` is written by `src/sync/export.rs::caps_json`. Its relevant fields:
`sample_util_5h` / `sample_util_7d` (Option<f64>, already 0.0–1.0 fractions) and
`resets_5h_iso` / `resets_7d_iso` (Option<String>, RFC3339). That's exactly enough to rebuild a `UsageSnapshot`.

**Key types (already exist):**
- `src/api/usage.rs`: `UsageSnapshot { five_hour: Option<UsageBucket>, seven_day: Option<UsageBucket> }`, `UsageBucket { utilization: f64, resets_at: Option<DateTime<Utc>> }`, and `enum FetchError { RateLimited, Http(u16), Network(String), Parse(String) }`. `parse_usage_response` is the existing API-JSON parser; we add a sibling `parse_caps_snapshot` for the caps.json shape.
- `src/sync/storage.rs`: `trait ObjectStore { fn put(...) }` + `struct SupabaseStore` (has `agent: ureq::Agent`, `key`, `bucket`, and `pub fn object_url(&self, object_path) -> String`), `enum StorageError { Network(String), Http(u16) }`.
- `src/sync/mod.rs`: `struct Syncer<S: ObjectStore> { config, store }` with `run_once`, `upload_cache_only` (Phase 1), private `put_buffer`. `Syncer::from_env() -> Result<Option<Syncer<SupabaseStore>>>`.
- `src/tray/poller.rs` (Win32-gated): the polling thread. Its source seam is `poll_once(&creds)` (in `src/poll.rs`), which fetches the API AND appends a calibration sample. The thread builds an `AppSnapshot` and, when a `Syncer` exists, calls `syncer.run_once(&snapshot, &creds, &samples)`.
- `src/tray/mod.rs`: `run()` loads creds via `load_from_default_path()?` (bails on expiry) and passes them to the window (tooltip sub/tier) and `poller::spawn`.

**Cargo note:** `windows`/`eframe`/`egui*`/`winit` are `cfg(windows)`-gated (Phase 1). `src/sync/*`, `src/api/*`, `src/poll.rs` are platform-agnostic and compile on Linux too. Tasks 1–3 touch only agnostic code (so the collector binary keeps building on Linux); Tasks 4–5 touch the Win32-gated tray.

**Test commands:** `cargo test --lib <filter>` for unit tests; full gates `cargo test`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`. On an intermittent linker OOM, retry once with `-j 1`. There is a pre-existing harmless `unused import: Sender` warning in `src/tray/poller.rs` — do not "fix" unrelated files.

---

## File structure

| File | Change | Responsibility |
|---|---|---|
| `src/sync/storage.rs` | Modify | Add `get` to `ObjectStore` trait + `SupabaseStore` (HTTP GET). |
| `src/api/usage.rs` | Modify | Add `parse_caps_snapshot(bytes) -> Result<UsageSnapshot>` (caps.json → snapshot). |
| `src/sync/mod.rs` | Modify | Add `Syncer::fetch_caps(caps_prefix)` + `caps_prefix_from_env()`; update test fakes for the new trait method. |
| `src/poll.rs` | (unchanged) | Still the API path; reused when cloud mode is off. |
| `src/tray/poller.rs` | Modify | Cloud-mode source selection + cache-only upload. |
| `src/tray/mod.rs` | Modify | Tolerate missing creds when cloud mode is configured. |
| `.env.example` | Modify | Document `SUPABASE_CAPS_PREFIX`. |
| `docs/deploy-windows-secondary.md` | Create | How to flip Windows to secondary mode. |
| `tests/usage_test.rs` | Modify | Tests for `parse_caps_snapshot`. |

---

### Task 1: Add `get` to the object store

**Files:**
- Modify: `src/sync/storage.rs`

**Why:** The store can only `put` today. Reading `caps.json` from the cloud needs an HTTP GET. We add `get` to the trait and implement it on `SupabaseStore`, reusing the existing `object_url` + auth headers.

- [ ] **Step 1: Add `get` to the `ObjectStore` trait**

In `src/sync/storage.rs`, change the trait from:

```rust
pub trait ObjectStore {
    fn put(&self, object_path: &str, content_type: &str, bytes: &[u8]) -> Result<(), StorageError>;
}
```

to:

```rust
pub trait ObjectStore {
    fn put(&self, object_path: &str, content_type: &str, bytes: &[u8]) -> Result<(), StorageError>;
    /// Download an object's bytes. `object_path` is the full key including prefix,
    /// e.g. "borgi-linux/caps.json".
    fn get(&self, object_path: &str) -> Result<Vec<u8>, StorageError>;
}
```

- [ ] **Step 2: Implement `get` on `SupabaseStore`**

Add a `use std::io::Read;` at the top of `src/sync/storage.rs` (just under the existing `use` lines), then add this method inside `impl ObjectStore for SupabaseStore` (right after the `put` fn, before the closing `}` of the impl):

```rust
    fn get(&self, object_path: &str) -> Result<Vec<u8>, StorageError> {
        let url = self.object_url(object_path);
        let resp = self
            .agent
            .get(&url)
            .set("Authorization", &format!("Bearer {}", self.key))
            .set("apikey", &self.key)
            .call();

        match resp {
            Ok(r) => {
                let mut buf = Vec::new();
                r.into_reader()
                    .read_to_end(&mut buf)
                    .map_err(|e| StorageError::Network(e.to_string()))?;
                Ok(buf)
            }
            Err(ureq::Error::Status(code, _)) => Err(StorageError::Http(code)),
            Err(e) => Err(StorageError::Network(e.to_string())),
        }
    }
```

- [ ] **Step 3: Build to confirm the trait + impl compile**

Run: `cargo build`
Expected: FAIL — the test fakes in `src/sync/mod.rs` (`FakeStore`, `FailFirstStore`) no longer satisfy `ObjectStore` because they lack `get`. That's expected; Task 1 Step 4 fixes them. (If you prefer, run `cargo build --lib` to see the same errors faster.)

- [ ] **Step 4: Update the two existing test fakes in `src/sync/mod.rs`**

In `src/sync/mod.rs`, the `#[cfg(test)] mod tests` block has two fakes implementing `ObjectStore`. Add a `get` to each.

In `impl ObjectStore for FakeStore { ... }`, after its `put` method, add:

```rust
        fn get(&self, _object_path: &str) -> Result<Vec<u8>, StorageError> {
            Ok(Vec::new())
        }
```

In `impl ObjectStore for FailFirstStore { ... }`, after its `put` method, add:

```rust
        fn get(&self, _object_path: &str) -> Result<Vec<u8>, StorageError> {
            Ok(Vec::new())
        }
```

- [ ] **Step 5: Build + run sync tests**

Run: `cargo build` then `cargo test --lib sync::`
Expected: both PASS (existing sync tests green; the crate compiles with the new trait method).

- [ ] **Step 6: Commit**

```bash
git add src/sync/storage.rs src/sync/mod.rs
git commit -m "feat(sync): add ObjectStore::get for reading cloud objects"
```

---

### Task 2: `parse_caps_snapshot` — caps.json → UsageSnapshot

**Files:**
- Modify: `src/api/usage.rs`
- Test: `tests/usage_test.rs`

**Why:** The pure parser that turns `caps.json` bytes into the same `UsageSnapshot` the API path produces. Account-wide `sample_util_*` are already 0.0–1.0 fractions; `resets_*_iso` are RFC3339 strings.

- [ ] **Step 1: Write the failing tests**

Add to the end of `tests/usage_test.rs`:

```rust
#[test]
fn parse_caps_snapshot_full_builds_both_buckets() {
    let body = br#"{
        "sample_util_5h": 0.42,
        "sample_util_7d": 0.1,
        "resets_5h_iso": "2026-05-23T12:00:00+00:00",
        "resets_7d_iso": "2026-05-25T07:00:00+00:00"
    }"#;
    let snap = claude_usage_tray::api::usage::parse_caps_snapshot(body).unwrap();
    let five = snap.five_hour.expect("five_hour present");
    assert!((five.utilization - 0.42).abs() < 1e-9);
    assert!(five.resets_at.is_some());
    let seven = snap.seven_day.expect("seven_day present");
    assert!((seven.utilization - 0.1).abs() < 1e-9);
}

#[test]
fn parse_caps_snapshot_null_resets_keeps_util_drops_reset() {
    let body = br#"{"sample_util_5h": 0.5, "sample_util_7d": null, "resets_5h_iso": null, "resets_7d_iso": null}"#;
    let snap = claude_usage_tray::api::usage::parse_caps_snapshot(body).unwrap();
    let five = snap.five_hour.expect("five_hour present");
    assert!((five.utilization - 0.5).abs() < 1e-9);
    assert!(five.resets_at.is_none());
    // null util => no bucket at all.
    assert!(snap.seven_day.is_none());
}

#[test]
fn parse_caps_snapshot_missing_fields_yields_empty_snapshot() {
    // caps.json with no sample (the "no data yet" case) => both buckets None.
    let body = br#"{"subscription_type": "pro", "rate_limit_tier": "default"}"#;
    let snap = claude_usage_tray::api::usage::parse_caps_snapshot(body).unwrap();
    assert!(snap.five_hour.is_none());
    assert!(snap.seven_day.is_none());
}
```

- [ ] **Step 2: Run them to confirm they fail**

Run: `cargo test --test usage_test parse_caps_snapshot`
Expected: FAIL — `parse_caps_snapshot` does not exist (compile error / unresolved import).

- [ ] **Step 3: Implement `parse_caps_snapshot`**

In `src/api/usage.rs`, add this function (e.g. right after `parse_usage_response`). It reuses the already-imported `serde::Deserialize`, `DateTime`, `Utc`, and the `UsageBucket`/`UsageSnapshot` types:

```rust
/// Parse `caps.json` (written by `sync::export::caps_json`) into a `UsageSnapshot`.
/// Account-wide `sample_util_*` are already 0.0–1.0 fractions; `resets_*_iso` are
/// RFC3339 strings. A missing/null `sample_util_*` yields no bucket for that window.
pub fn parse_caps_snapshot(bytes: &[u8]) -> Result<UsageSnapshot> {
    #[derive(Deserialize)]
    struct CapsIn {
        #[serde(default)]
        sample_util_5h: Option<f64>,
        #[serde(default)]
        sample_util_7d: Option<f64>,
        #[serde(default)]
        resets_5h_iso: Option<String>,
        #[serde(default)]
        resets_7d_iso: Option<String>,
    }

    fn bucket(util: Option<f64>, resets_iso: Option<String>) -> Option<UsageBucket> {
        util.map(|utilization| UsageBucket {
            utilization,
            resets_at: resets_iso.and_then(|s| {
                DateTime::parse_from_rfc3339(&s)
                    .ok()
                    .map(|d| d.with_timezone(&Utc))
            }),
        })
    }

    let c: CapsIn = serde_json::from_slice(bytes).context("invalid caps.json")?;
    Ok(UsageSnapshot {
        five_hour: bucket(c.sample_util_5h, c.resets_5h_iso),
        seven_day: bucket(c.sample_util_7d, c.resets_7d_iso),
    })
}
```

> `Result` here is `anyhow::Result` (already imported at the top of `usage.rs` as `use anyhow::...`; `parse_usage_response` returns the same `Result`). `context` comes from `anyhow::Context` — confirm it's imported; the top of the file already has `use anyhow::{Context, Result};` (the same import `parse_usage_response` uses with `.context(...)`). If for some reason only `Result` is imported, add `Context` to that `use`.

- [ ] **Step 4: Run the tests to confirm they pass**

Run: `cargo test --test usage_test parse_caps_snapshot`
Expected: PASS (all three).

- [ ] **Step 5: Commit**

```bash
git add src/api/usage.rs tests/usage_test.rs
git commit -m "feat(api): parse_caps_snapshot to rebuild UsageSnapshot from caps.json"
```

---

### Task 3: `Syncer::fetch_caps` + `caps_prefix_from_env`

**Files:**
- Modify: `src/sync/mod.rs`

**Why:** Glue: `fetch_caps` does `store.get("{caps_prefix}/caps.json")` then `parse_caps_snapshot`. `caps_prefix_from_env` is the single source of the opt-in toggle, callable from both `tray::run` and the poller.

- [ ] **Step 1: Write the failing test**

In `src/sync/mod.rs`'s `#[cfg(test)] mod tests` block, add a dedicated fake + a test:

```rust
    struct CapsFake {
        body: Vec<u8>,
    }
    impl ObjectStore for CapsFake {
        fn put(&self, _: &str, _: &str, _: &[u8]) -> Result<(), StorageError> {
            Ok(())
        }
        fn get(&self, object_path: &str) -> Result<Vec<u8>, StorageError> {
            assert_eq!(object_path, "borgi-linux/caps.json");
            Ok(self.body.clone())
        }
    }

    #[test]
    fn fetch_caps_reads_prefixed_caps_json_and_parses_it() {
        let body = br#"{"sample_util_5h":0.42,"sample_util_7d":0.1,"resets_5h_iso":null,"resets_7d_iso":null}"#.to_vec();
        let syncer = Syncer {
            config: cfg(),
            store: CapsFake { body },
        };
        let snap = syncer.fetch_caps("borgi-linux").unwrap();
        assert!((snap.five_hour.unwrap().utilization - 0.42).abs() < 1e-9);
        assert!((snap.seven_day.unwrap().utilization - 0.1).abs() < 1e-9);
    }
```

> `cfg()` (the test helper) sets `prefix: "borgi"`, but `fetch_caps` uses the **caps_prefix argument** (`"borgi-linux"`), not `config.prefix` — the `CapsFake::get` assertion verifies the requested key is `borgi-linux/caps.json`.

- [ ] **Step 2: Run it to confirm it fails**

Run: `cargo test --lib fetch_caps_reads_prefixed_caps_json_and_parses_it`
Expected: FAIL — `no method named fetch_caps`.

- [ ] **Step 3: Implement `fetch_caps`**

In `src/sync/mod.rs`, inside `impl<S: ObjectStore> Syncer<S>` (after `upload_cache_only`), add:

```rust
    /// Read `{caps_prefix}/caps.json` from the store and parse it into a
    /// `UsageSnapshot`. Used by the Windows tray's cloud-caps (secondary) mode to
    /// display live utilization without polling the rate-limited usage API.
    pub fn fetch_caps(
        &self,
        caps_prefix: &str,
    ) -> anyhow::Result<crate::api::usage::UsageSnapshot> {
        let object_path = format!("{}/caps.json", caps_prefix);
        let bytes = self.store.get(&object_path)?;
        crate::api::usage::parse_caps_snapshot(&bytes)
    }
```

- [ ] **Step 4: Add `caps_prefix_from_env` as a module-level function**

In `src/sync/mod.rs`, add this near the top of the file (after the `use` statements, before the `Syncer` struct):

```rust
/// The opt-in toggle for the Windows tray's cloud-caps (secondary) mode. Returns
/// the prefix to read account-wide caps from (e.g. "borgi-linux") when
/// `SUPABASE_CAPS_PREFIX` is set and non-empty, else `None` (poll the API as before).
/// Loads `.env` first so it works the same whether or not the syncer has run yet.
pub fn caps_prefix_from_env() -> Option<String> {
    let _ = dotenvy::dotenv();
    std::env::var("SUPABASE_CAPS_PREFIX")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}
```

- [ ] **Step 5: Run the test + full sync module**

Run: `cargo test --lib sync::`
Expected: PASS (new `fetch_caps` test + all existing sync tests).

- [ ] **Step 6: Confirm Linux build still compiles (agnostic code only so far)**

Run: `cargo build --bin collector`
Expected: PASS (Tasks 1–3 touched only platform-agnostic modules).

- [ ] **Step 7: Commit**

```bash
git add src/sync/mod.rs
git commit -m "feat(sync): fetch_caps + caps_prefix_from_env for cloud-caps mode"
```

---

### Task 4: Wire cloud-caps mode into the poller

**Files:**
- Modify: `src/tray/poller.rs`

**Why:** The behavioral switch. When cloud mode is on, the poller reads caps from the cloud instead of the API and uploads cache-only; otherwise it behaves exactly as today. This file is Win32-gated (Windows-only), so it builds on Windows; the change is three localized edits to `polling_loop`.

- [ ] **Step 1: Determine the mode once, after the syncer is built**

In `src/tray/poller.rs`, find the block that builds `syncer` (the `let syncer = match crate::sync::Syncer::from_env() { ... };` near the start of `polling_loop`). Immediately AFTER that block, add:

```rust
    // Phase 3: opt-in cloud-caps (secondary) mode. When SUPABASE_CAPS_PREFIX is set
    // and Supabase sync is configured, read account-wide caps from the cloud instead
    // of polling the rate-limited usage API, and upload only our local turns.
    let caps_prefix = crate::sync::caps_prefix_from_env();
    let cloud_caps_mode = caps_prefix.is_some() && syncer.is_some();
    match (&caps_prefix, &syncer) {
        (Some(p), Some(_)) => tracing::info!(
            caps_prefix = %p,
            "live caps source: cloud (secondary mode; NOT polling the usage API)"
        ),
        (Some(p), None) => tracing::warn!(
            caps_prefix = %p,
            "SUPABASE_CAPS_PREFIX set but Supabase sync is not configured; falling back to API polling"
        ),
        (None, _) => tracing::info!("live caps source: usage API (primary mode)"),
    }
```

- [ ] **Step 2: Select the snapshot source in the loop**

In `polling_loop`, find:

```rust
        let event = match poll_once(&creds) {
```

Replace ONLY that line with the following (the `match` body below it stays exactly as-is):

```rust
        // Cloud mode reads caps.json from Supabase; primary mode polls the API
        // (which also appends a calibration sample). Both yield a UsageSnapshot or
        // a FetchError, so the match arms below are unchanged.
        let fetch_result = if cloud_caps_mode {
            syncer
                .as_ref()
                .expect("cloud_caps_mode implies syncer.is_some()")
                .fetch_caps(caps_prefix.as_ref().expect("cloud_caps_mode implies caps_prefix.is_some()"))
                .map_err(|e| FetchError::Network(e.to_string()))
        } else {
            poll_once(&creds)
        };
        let event = match fetch_result {
```

- [ ] **Step 3: Upload cache-only in cloud mode**

In `polling_loop`, find the sync block:

```rust
        if let Some(syncer) = &syncer {
            let samples = match crate::log::calibration::read_all_default() {
                Ok(s) => s,
                Err(e) => {
                    tracing::warn!(error = %e, "calibration log read failed; uploading empty samples this tick");
                    Vec::new()
                }
            };
            syncer.run_once(&snapshot, &creds, &samples);
        }
```

Replace it entirely with:

```rust
        if let Some(syncer) = &syncer {
            if cloud_caps_mode {
                // Secondary mode: upload only our local turns. caps.json and the
                // calibration log are the primary (Linux) poller's responsibility,
                // so we must not overwrite them from here.
                syncer.upload_cache_only(&snapshot);
            } else {
                let samples = match crate::log::calibration::read_all_default() {
                    Ok(s) => s,
                    Err(e) => {
                        tracing::warn!(error = %e, "calibration log read failed; uploading empty samples this tick");
                        Vec::new()
                    }
                };
                syncer.run_once(&snapshot, &creds, &samples);
            }
        }
```

- [ ] **Step 4: Build + clippy + fmt**

Run: `cargo build`
Expected: PASS.
Run: `cargo clippy --all-targets -- -D warnings`
Expected: PASS (ignore the pre-existing `unused import: Sender` note if it appears; do not edit other files).
Run: `cargo fmt --check`
Expected: PASS (run `cargo fmt` first if it complains, then re-check, and include the formatting in this commit).

- [ ] **Step 5: Commit**

```bash
git add src/tray/poller.rs
git commit -m "feat(tray): cloud-caps secondary mode in the poller"
```

---

### Task 5: Tolerate a missing token when cloud mode is on

**Files:**
- Modify: `src/tray/mod.rs`

**Why:** `tray::run` currently does `load_from_default_path()?` and aborts if the OAuth token is missing/expired. On an intermittent laptop the token can be stale at launch — but in cloud mode we don't need it (we read caps from the cloud and upload cache-only). So when `SUPABASE_CAPS_PREFIX` is configured, fall back to placeholder creds instead of aborting.

- [ ] **Step 1: Broaden the creds import**

In `src/tray/mod.rs`, change:

```rust
use crate::api::credentials::load_from_default_path;
```

to:

```rust
use crate::api::credentials::{load_from_default_path, Credentials};
```

- [ ] **Step 2: Replace the creds load with a cloud-mode-aware fallback**

In `src/tray/mod.rs`'s `run()`, replace:

```rust
    let creds = load_from_default_path()?;
    tracing::info!(
        subscription = %creds.subscription_type,
        tier = %creds.rate_limit_tier,
        "loaded credentials"
    );
```

with:

```rust
    // In cloud-caps (secondary) mode we don't poll the API, so a missing/expired
    // token must not stop the tray from starting — we read live caps from the cloud
    // and upload cache-only. In primary mode the token is required as before.
    let cloud_caps_mode = crate::sync::caps_prefix_from_env().is_some();
    let creds = match load_from_default_path() {
        Ok(c) => {
            tracing::info!(
                subscription = %c.subscription_type,
                tier = %c.rate_limit_tier,
                "loaded credentials"
            );
            c
        }
        Err(e) if cloud_caps_mode => {
            tracing::warn!(
                error = %e,
                "credentials unavailable; starting in cloud-caps mode without a live token"
            );
            Credentials {
                access_token: String::new(),
                subscription_type: "unknown".to_string(),
                rate_limit_tier: "unknown".to_string(),
            }
        }
        Err(e) => return Err(e),
    };
```

> `Credentials` fields (`access_token`, `subscription_type`, `rate_limit_tier`) are all public. In cloud mode the empty `access_token` is never used (no API poll); the window tooltip will show "unknown / unknown" sub/tier, which is acceptable for a secondary machine.

- [ ] **Step 3: Build**

Run: `cargo build`
Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add src/tray/mod.rs
git commit -m "feat(tray): start without a token when cloud-caps mode is configured"
```

---

### Task 6: Docs + final gates

**Files:**
- Modify: `.env.example`
- Create: `docs/deploy-windows-secondary.md`

**Why:** Record the opt-in env var and the one-step switchover.

- [ ] **Step 1: Document `SUPABASE_CAPS_PREFIX` in `.env.example`**

Append to `.env.example`:

```
# Phase 3 (optional, Windows secondary mode): when set, the Windows tray STOPS
# polling the usage API and instead reads account-wide caps from
# {SUPABASE_CAPS_PREFIX}/caps.json (the always-on Linux poller's prefix), and
# uploads only its own local turns. Leave UNSET on the Linux server / primary
# poller. Example on the Windows machine: SUPABASE_CAPS_PREFIX=borgi-linux
#SUPABASE_CAPS_PREFIX=borgi-linux
```

- [ ] **Step 2: Create `docs/deploy-windows-secondary.md`**

Write this file verbatim:

````markdown
# Flipping the Windows tray to secondary mode (Phase 3)

By default the Windows tray polls Anthropic's usage API every ~2 minutes. Once the
Linux server is the always-on primary poller (Phase 1) and the Streamlit viewer
merges both machines (Phase 2), the Windows machine should STOP polling so the two
machines don't contend on the ~1 req/min/account rate limit.

In secondary mode the Windows tray:
- reads account-wide caps from `borgi-linux/caps.json` in Supabase (kept fresh by
  the Linux poller) to drive the tray icon / widget / live-banner %,
- uploads only its own local turns (`borgi/cache.parquet`),
- never calls the usage API.

## How to switch

Add ONE line to the Windows `.env` (next to the existing `SUPABASE_*` vars):

```
SUPABASE_CAPS_PREFIX=borgi-linux
```

Then restart the tray. On startup the log shows:

```
live caps source: cloud (secondary mode; NOT polling the usage API)
```

## How to switch back

Remove (or comment out) `SUPABASE_CAPS_PREFIX` and restart the tray. It returns to
polling the usage API and uploading all three objects under its own prefix.

## Prerequisites / notes

- The Linux collector must be running and writing `borgi-linux/caps.json` (Phase 1),
  otherwise the tray's % readout will be empty (it shows the gray "no data" icon).
- `SUPABASE_URL` / `SUPABASE_SERVICE_ROLE_KEY` / `SUPABASE_BUCKET` must still be set
  (the tray needs the Supabase client to read caps and to upload its turns).
- Secondary mode does not need a fresh OAuth token: if the laptop's token is expired
  at launch, the tray still starts (it logs a warning and shows "unknown" plan/tier
  in the tooltip). It will pick up real creds again automatically once you run Claude
  Code on the laptop and restart, or whenever you switch back to primary mode.
- Keep `SUPABASE_USER_PREFIX=borgi` on Windows unchanged — that's still where Windows
  uploads its own turns.
````

- [ ] **Step 3: Build sanity**

Run: `cargo build`
Expected: PASS (docs don't affect the build; confirms the tree is clean).

- [ ] **Step 4: Commit**

```bash
git add .env.example docs/deploy-windows-secondary.md
git commit -m "docs: SUPABASE_CAPS_PREFIX + Windows secondary-mode guide"
```

---

## Final verification (run all before declaring Phase 3 done)

- [ ] `cargo build` — PASS (Windows build)
- [ ] `cargo build --bin collector` — PASS (Linux collector still compiles; Tasks 1–3 are agnostic, Tasks 4–5 are Win32-gated)
- [ ] `cargo test` — PASS (new `parse_caps_snapshot`, `fetch_caps`, and updated fakes; nothing regressed)
- [ ] `cargo clippy --all-targets -- -D warnings` — PASS
- [ ] `cargo fmt --check` — PASS

## Manual end-to-end verification (acceptance gate)

- [ ] **Prereq:** Phase 1 deployed — `borgi-linux/caps.json` exists in Supabase and is being refreshed by the Linux poller.
- [ ] On Windows, add `SUPABASE_CAPS_PREFIX=borgi-linux` to `.env`, restart the tray.
- [ ] Tray log shows `live caps source: cloud (secondary mode; NOT polling the usage API)`.
- [ ] Tray icon / widget show a live % that matches the Linux-sourced caps (and updates over a few minutes as the Linux poller refreshes caps.json).
- [ ] Supabase shows `borgi/cache.parquet` still being updated (Windows turns), while `borgi/caps.json` and `borgi/calibration_log.parquet` STOP changing (Windows no longer writes them).
- [ ] No HTTP 429s on the Linux poller (the two-poller contention is gone).
- [ ] Remove the env var, restart → log shows `live caps source: usage API (primary mode)` (switch-back works).

## Out of scope

- Wiring the Windows native dashboard's historical tabs to cloud/merged data — the
  merged history lives in the Streamlit viewer (Phase 2). Windows dashboard history
  stays local-only by design (see design spec §5.1).
- A version bump / release tag — handle separately once all three phases are verified
  end-to-end on real machines.
