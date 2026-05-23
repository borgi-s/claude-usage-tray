# Stage 7 — Supabase Sync Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** The tray agent uploads `cache.parquet`, `calibration_log.parquet`, and `caps.json` to Supabase Storage under a per-user prefix on each poll tick, so the existing polars cloud viewer reads them with no change.

**Architecture:** A new isolated `src/sync/` module. It serializes the in-memory `AppSnapshot` (turns → cache, last_sample+caps+creds → caps) plus the on-disk calibration log (→ calibration_log) into parquet/JSON byte buffers, then PUTs each to Supabase Storage. Native `bincode`/`jsonl` local storage is untouched. Sync is opt-in (skipped unless `.env` is configured) and best-effort (upload errors are logged, never crash the poll loop). The uploader sits behind a trait so the orchestration is unit-testable with a fake.

**Tech Stack:** Rust, `arrow` + `parquet` (arrow-rs, new), `dotenvy` (new), `ureq` (existing), `serde_json` (existing), `chrono` (existing).

---

## Beginner orientation (read once before starting)

A few concepts this plan leans on, since this is early Rust:

- **Parquet / Arrow.** Parquet is a compressed, columnar file format for tables. The Rust way to write it is: build **Arrow arrays** (one typed array per column, e.g. all the `output_tokens` together), bundle them into a **`RecordBatch`** (arrays + a schema describing column names/types), then stream the batch through an **`ArrowWriter`** into a byte buffer. "Columnar" = values of one column stored together, which compresses well and is fast for the polars consumer to read.
- **Why a `trait` for the uploader.** A `trait` is like an interface. We define `ObjectStore` with one method `put(...)`. The real implementation talks to Supabase over HTTP; a fake implementation in tests just records what it was asked to upload. Code that depends on the trait (our orchestration) can be tested without a network.
- **`Option<T>` → nullable column.** Many fields are `Option` (maybe-absent). Arrow arrays built from an iterator of `Option` produce SQL-style nulls. We emit several columns as all-null on purpose (the cloud viewer doesn't read them yet — see the spec).
- **Best-effort.** The sync functions return `Result`, but the *caller in the poll loop* logs any error and moves on. A failed upload must never stop polling.
- **`cargo add`** edits `Cargo.toml` for you and picks a compatible version. `cargo build` then downloads + compiles. The arrow/parquet crates are large; the first build will be slow — that's expected.

Reference (do not edit): the Python schemas this must match live in the companion repo —
`claude-usage-tracker/cache.py` (`ROW_SCHEMA`), `calibration_log.py` (`SCHEMA`), `caps.py` (`DerivedCaps`).

---

## File structure

| File | Responsibility | Created/Modified |
|---|---|---|
| `src/sync/mod.rs` | Module root + `Syncer` orchestration (build 3 buffers, upload each, best-effort) | Create |
| `src/sync/config.rs` | `SyncConfig` — read + validate `.env` (`from_env`), prefix sanitization | Create |
| `src/sync/export.rs` | Serialize to bytes: `cache_parquet`, `calibration_log_parquet`, `caps_json` (+ `CapsJson` struct) | Create |
| `src/sync/storage.rs` | `ObjectStore` trait + `SupabaseStore` (ureq PUT) + `StorageError` | Create |
| `src/lib.rs` | Register `pub mod sync;` | Modify |
| `src/tray/poller.rs` | Construct `Syncer` once; call it each tick after the snapshot is built | Modify |
| `Cargo.toml` | Add `arrow`, `parquet`, `dotenvy` | Modify |
| `.env.example` | Document the four Supabase env vars | Create |
| `.gitignore` | Ensure `.env` is ignored | Modify |
| `CLAUDE.md` | Add Stage 7 spec+plan pointers; mark progress | Modify |

**Interfaces locked here (used across tasks — keep names exact):**

```rust
// config.rs
pub struct SyncConfig { pub url: String, pub service_role_key: String, pub bucket: String, pub prefix: String }
pub fn from_env() -> anyhow::Result<Option<SyncConfig>>;   // Ok(None) = not configured
fn is_valid_prefix(s: &str) -> bool;

// storage.rs
pub trait ObjectStore { fn put(&self, object_path: &str, content_type: &str, bytes: &[u8]) -> Result<(), StorageError>; }
pub struct SupabaseStore { /* agent, url, key, bucket */ }

// export.rs
pub fn cache_parquet(turns: &[crate::data::parser::Turn]) -> anyhow::Result<Vec<u8>>;
pub fn calibration_log_parquet(samples: &[crate::log::calibration::CalibrationSample]) -> anyhow::Result<Vec<u8>>;
pub fn caps_json(snapshot: &crate::shared::snapshot::AppSnapshot, creds: &crate::api::credentials::Credentials) -> anyhow::Result<Vec<u8>>;

// mod.rs
pub struct Syncer<S: ObjectStore> { config: SyncConfig, store: S }
impl Syncer<SupabaseStore> { pub fn from_env() -> anyhow::Result<Option<Self>>; }
impl<S: ObjectStore> Syncer<S> {
    pub fn run_once(&self, snapshot: &AppSnapshot, creds: &Credentials, samples: &[CalibrationSample]); // best-effort, logs
}
```

---

## Task 1: Add dependencies and register the module skeleton

**Files:**
- Modify: `Cargo.toml`
- Create: `src/sync/mod.rs`
- Modify: `src/lib.rs:1-13`

- [ ] **Step 1: Add the crates**

Run:
```bash
cargo add arrow parquet dotenvy
```
Beginner note: this adds three lines under `[dependencies]` in `Cargo.toml`. `arrow` + `parquet` must share the same major version — `cargo add` resolves a matching pair. If they resolve to different majors, pin them equal (e.g. `cargo add arrow@53 parquet@53`).

- [ ] **Step 2: Create the module root**

Create `src/sync/mod.rs`:
```rust
//! Stage 7: best-effort upload of cache + calibration log + caps to Supabase
//! Storage, so the polars cloud viewer reads them unchanged. See
//! docs/superpowers/specs/2026-05-23-stage-7-supabase-sync-design.md.

pub mod config;
pub mod export;
pub mod storage;
```

- [ ] **Step 3: Register the module**

In `src/lib.rs`, add `pub mod sync;` in alphabetical position (after `pub mod shared;`):
```rust
pub mod shared;
pub mod sync;
pub mod tray;
```

- [ ] **Step 4: Create empty submodule files so it compiles**

Create `src/sync/config.rs`, `src/sync/export.rs`, `src/sync/storage.rs` each containing only:
```rust
// implemented in later tasks
```

- [ ] **Step 5: Build to verify deps resolve and compile**

Run: `cargo build`
Expected: PASS (slow first build while arrow/parquet compile). If arrow/parquet versions conflict, pin them to the same major and rebuild.

- [ ] **Step 6: Commit**

```bash
git add Cargo.toml Cargo.lock src/sync/mod.rs src/sync/config.rs src/sync/export.rs src/sync/storage.rs src/lib.rs
git commit -m "feat(stage-7): add arrow/parquet/dotenvy deps + sync module skeleton"
```

---

## Task 2: `sync::config` — read and validate `.env`

**Files:**
- Modify: `src/sync/config.rs`
- Test: in-file `#[cfg(test)]` module

- [ ] **Step 1: Write the failing tests**

Replace `src/sync/config.rs` with:
```rust
//! Reads Supabase sync settings from environment (.env via dotenvy).

/// Validated Supabase sync configuration. Absent => sync disabled.
#[derive(Debug, Clone, PartialEq)]
pub struct SyncConfig {
    pub url: String,
    pub service_role_key: String,
    pub bucket: String,
    pub prefix: String,
}

const DEFAULT_BUCKET: &str = "usage-tracker";

/// A prefix is path-safe if non-empty and only ASCII alphanumerics, `-`, `_`.
/// This is the per-user object-key segment, so it must not contain slashes,
/// spaces, or `.` runs that could escape the prefix or confuse the storage API.
fn is_valid_prefix(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_prefixes_accepted() {
        assert!(is_valid_prefix("borgi"));
        assert!(is_valid_prefix("user-1_x"));
    }

    #[test]
    fn invalid_prefixes_rejected() {
        assert!(!is_valid_prefix(""));
        assert!(!is_valid_prefix("a/b"));
        assert!(!is_valid_prefix("has space"));
        assert!(!is_valid_prefix("dots.here"));
    }
}
```

- [ ] **Step 2: Run tests to verify they pass**

Run: `cargo test --lib sync::config`
Expected: PASS (these test the helper directly).

Beginner note: we wrote the helper and its tests together because the helper is trivial and pure. The next step adds the env-reading function, which we test via a serial test that sets process env vars.

- [ ] **Step 3: Write the failing test for `from_env`**

Add to the `tests` module in `src/sync/config.rs`:
```rust
    // These tests mutate process-global env vars, so they must not run in
    // parallel with each other. We serialize them by running everything inside
    // one #[test] fn.
    fn clear_env() {
        for k in ["SUPABASE_URL", "SUPABASE_SERVICE_ROLE_KEY", "SUPABASE_BUCKET", "SUPABASE_USER_PREFIX"] {
            std::env::remove_var(k);
        }
    }

    #[test]
    fn from_env_behaviors() {
        // 1. Missing required vars => Ok(None).
        clear_env();
        assert_eq!(from_env_inner().unwrap(), None);

        // 2. All required present, default bucket.
        clear_env();
        std::env::set_var("SUPABASE_URL", "https://x.supabase.co");
        std::env::set_var("SUPABASE_SERVICE_ROLE_KEY", "key123");
        std::env::set_var("SUPABASE_USER_PREFIX", "borgi");
        let cfg = from_env_inner().unwrap().unwrap();
        assert_eq!(cfg.url, "https://x.supabase.co");
        assert_eq!(cfg.bucket, "usage-tracker");
        assert_eq!(cfg.prefix, "borgi");

        // 3. Custom bucket honored.
        std::env::set_var("SUPABASE_BUCKET", "team-bucket");
        assert_eq!(from_env_inner().unwrap().unwrap().bucket, "team-bucket");

        // 4. Invalid prefix => Err.
        clear_env();
        std::env::set_var("SUPABASE_URL", "https://x.supabase.co");
        std::env::set_var("SUPABASE_SERVICE_ROLE_KEY", "key123");
        std::env::set_var("SUPABASE_USER_PREFIX", "bad/prefix");
        assert!(from_env_inner().is_err());

        clear_env();
    }
```

Run: `cargo test --lib sync::config`
Expected: FAIL — `from_env_inner` not found.

- [ ] **Step 4: Implement `from_env` + `from_env_inner`**

Add to `src/sync/config.rs` (above the tests module):
```rust
use anyhow::{bail, Result};

/// Public entry: load `.env` (best-effort) then read from the environment.
/// Returns `Ok(None)` when sync is not configured (a required var is absent),
/// `Err` when configured but invalid (bad prefix).
pub fn from_env() -> Result<Option<SyncConfig>> {
    // Loads ./.env into the process environment if present. Missing file is fine.
    let _ = dotenvy::dotenv();
    from_env_inner()
}

/// Pure-ish core: reads only from `std::env`, so tests can set vars directly
/// without touching the filesystem.
fn from_env_inner() -> Result<Option<SyncConfig>> {
    let (url, key, prefix) = match (
        std::env::var("SUPABASE_URL").ok(),
        std::env::var("SUPABASE_SERVICE_ROLE_KEY").ok(),
        std::env::var("SUPABASE_USER_PREFIX").ok(),
    ) {
        (Some(u), Some(k), Some(p)) if !u.is_empty() && !k.is_empty() => (u, k, p),
        _ => return Ok(None),
    };

    if !is_valid_prefix(&prefix) {
        bail!("SUPABASE_USER_PREFIX '{prefix}' is invalid: use only letters, digits, '-', '_'");
    }

    let bucket = std::env::var("SUPABASE_BUCKET")
        .ok()
        .filter(|b| !b.is_empty())
        .unwrap_or_else(|| DEFAULT_BUCKET.to_string());

    Ok(Some(SyncConfig { url, service_role_key: key, bucket, prefix }))
}
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test --lib sync::config`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add src/sync/config.rs
git commit -m "feat(stage-7): sync config from .env with prefix validation"
```

---

## Task 3: `sync::export::cache_parquet`

**Files:**
- Modify: `src/sync/export.rs`
- Test: in-file `#[cfg(test)]` module

Columns (must match `cache.py` `ROW_SCHEMA` exactly, in order): `timestamp` Utf8, `session_id` Utf8, `subagent_id` Utf8 (nullable), `is_subagent` Bool, `project_cwd` Utf8, `model` Utf8, `version` Utf8, `input_tokens` Int64, `output_tokens` Int64, `cache_creation_input_tokens` Int64, `cache_read_input_tokens` Int64, `source_file` Utf8, `is_rate_limit_error` Bool.

- [ ] **Step 1: Write the failing test**

Put this in `src/sync/export.rs`:
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::parser::Turn;
    use chrono::TimeZone;
    use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
    use std::io::Write;
    use std::path::PathBuf;

    fn sample_turn() -> Turn {
        Turn {
            ts: chrono::Utc.with_ymd_and_hms(2026, 5, 23, 10, 0, 0).unwrap(),
            session_id: "sess-1".into(),
            subagent_id: None,
            is_subagent: false,
            project_cwd: "C:/proj".into(),
            model: "claude-opus-4-7".into(),
            version: "1.0".into(),
            input_tokens: 100,
            output_tokens: 400,
            cache_creation_input_tokens: 200,
            cache_read_input_tokens: 300,
            source_file: PathBuf::from("a.jsonl"),
            is_rate_limit_error: false,
        }
    }

    /// Write bytes to a temp file and read the parquet back into RecordBatches.
    fn read_back(bytes: &[u8]) -> arrow::record_batch::RecordBatch {
        let mut f = tempfile::NamedTempFile::new().unwrap();
        f.write_all(bytes).unwrap();
        let file = f.reopen().unwrap();
        let mut reader = ParquetRecordBatchReaderBuilder::try_new(file).unwrap().build().unwrap();
        reader.next().unwrap().unwrap()
    }

    #[test]
    fn cache_parquet_roundtrips_schema_and_values() {
        let bytes = cache_parquet(&[sample_turn()]).unwrap();
        let batch = read_back(&bytes);

        // 13 columns, exact names + order.
        let names: Vec<&str> = batch.schema().fields().iter().map(|f| f.name().as_str()).collect();
        assert_eq!(names, vec![
            "timestamp", "session_id", "subagent_id", "is_subagent", "project_cwd",
            "model", "version", "input_tokens", "output_tokens",
            "cache_creation_input_tokens", "cache_read_input_tokens",
            "source_file", "is_rate_limit_error",
        ]);
        assert_eq!(batch.num_rows(), 1);

        use arrow::array::{Int64Array, StringArray};
        let out = batch.column(8).as_any().downcast_ref::<Int64Array>().unwrap();
        assert_eq!(out.value(0), 400);
        let sess = batch.column(1).as_any().downcast_ref::<StringArray>().unwrap();
        assert_eq!(sess.value(0), "sess-1");
    }

    #[test]
    fn cache_parquet_handles_empty() {
        let bytes = cache_parquet(&[]).unwrap();
        let batch = read_back(&bytes);
        assert_eq!(batch.num_rows(), 0);
        assert_eq!(batch.schema().fields().len(), 13);
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --lib sync::export`
Expected: FAIL — `cache_parquet` not found.

- [ ] **Step 3: Implement `cache_parquet`**

Add to the top of `src/sync/export.rs`:
```rust
//! Serialize in-memory/on-disk state into parquet + JSON byte buffers that the
//! polars cloud viewer reads. Schemas mirror the Python project exactly.

use crate::data::parser::Turn;
use anyhow::Result;
use arrow::array::{ArrayRef, BooleanArray, Int64Array, StringArray};
use arrow::datatypes::{DataType, Field, Schema};
use arrow::record_batch::RecordBatch;
use parquet::arrow::ArrowWriter;
use std::sync::Arc;

/// Serialize the cache (one row per turn) to parquet bytes.
pub fn cache_parquet(turns: &[Turn]) -> Result<Vec<u8>> {
    let schema = Arc::new(Schema::new(vec![
        Field::new("timestamp", DataType::Utf8, false),
        Field::new("session_id", DataType::Utf8, false),
        Field::new("subagent_id", DataType::Utf8, true),
        Field::new("is_subagent", DataType::Boolean, false),
        Field::new("project_cwd", DataType::Utf8, false),
        Field::new("model", DataType::Utf8, false),
        Field::new("version", DataType::Utf8, false),
        Field::new("input_tokens", DataType::Int64, false),
        Field::new("output_tokens", DataType::Int64, false),
        Field::new("cache_creation_input_tokens", DataType::Int64, false),
        Field::new("cache_read_input_tokens", DataType::Int64, false),
        Field::new("source_file", DataType::Utf8, false),
        Field::new("is_rate_limit_error", DataType::Boolean, false),
    ]));

    let columns: Vec<ArrayRef> = vec![
        Arc::new(StringArray::from_iter_values(turns.iter().map(|t| t.ts.to_rfc3339()))),
        Arc::new(StringArray::from_iter_values(turns.iter().map(|t| t.session_id.clone()))),
        Arc::new(StringArray::from(turns.iter().map(|t| t.subagent_id.clone()).collect::<Vec<Option<String>>>())),
        Arc::new(BooleanArray::from(turns.iter().map(|t| t.is_subagent).collect::<Vec<bool>>())),
        Arc::new(StringArray::from_iter_values(turns.iter().map(|t| t.project_cwd.clone()))),
        Arc::new(StringArray::from_iter_values(turns.iter().map(|t| t.model.clone()))),
        Arc::new(StringArray::from_iter_values(turns.iter().map(|t| t.version.clone()))),
        Arc::new(Int64Array::from_iter_values(turns.iter().map(|t| t.input_tokens as i64))),
        Arc::new(Int64Array::from_iter_values(turns.iter().map(|t| t.output_tokens as i64))),
        Arc::new(Int64Array::from_iter_values(turns.iter().map(|t| t.cache_creation_input_tokens as i64))),
        Arc::new(Int64Array::from_iter_values(turns.iter().map(|t| t.cache_read_input_tokens as i64))),
        Arc::new(StringArray::from_iter_values(turns.iter().map(|t| t.source_file.to_string_lossy().into_owned()))),
        Arc::new(BooleanArray::from(turns.iter().map(|t| t.is_rate_limit_error).collect::<Vec<bool>>())),
    ];

    let batch = RecordBatch::try_new(schema.clone(), columns)?;
    write_parquet(schema, &batch)
}

/// Stream a single RecordBatch into an in-memory parquet buffer.
fn write_parquet(schema: Arc<Schema>, batch: &RecordBatch) -> Result<Vec<u8>> {
    let mut buf: Vec<u8> = Vec::new();
    {
        let mut writer = ArrowWriter::try_new(&mut buf, schema, None)?;
        writer.write(batch)?;
        writer.close()?;
    }
    Ok(buf)
}
```

Beginner note: `from_iter_values` builds a non-null array from plain values; `StringArray::from(Vec<Option<String>>)` builds a nullable array (used for `subagent_id`). `ArrayRef` is `Arc<dyn Array>` — the column type a `RecordBatch` holds.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test --lib sync::export`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/sync/export.rs
git commit -m "feat(stage-7): cache.parquet writer matching python ROW_SCHEMA"
```

---

## Task 4: `sync::export::calibration_log_parquet`

**Files:**
- Modify: `src/sync/export.rs`
- Test: in-file `#[cfg(test)]` module

Columns (match `calibration_log.py` `SCHEMA` exactly, in order): `sampled_at` Timestamp(ms, "UTC"), `util_5h` F64, `util_7d` F64, `burn_5h_cost_weighted` F64 (null), `burn_7d_cost_weighted` F64 (null), `input_5h`/`cache_creation_5h`/`cache_read_5h`/`output_5h` Int64 (null), `input_7d`/`cache_creation_7d`/`cache_read_7d`/`output_7d` Int64 (null), `subscription_type` Utf8, `rate_limit_tier` Utf8, `resets_5h_iso` Utf8, `resets_7d_iso` Utf8.

- [ ] **Step 1: Write the failing test**

Add to the `tests` module in `src/sync/export.rs`:
```rust
    use crate::log::calibration::CalibrationSample;

    fn sample_calib() -> CalibrationSample {
        CalibrationSample {
            schema_version: 1,
            ts: chrono::Utc.with_ymd_and_hms(2026, 5, 23, 9, 30, 0).unwrap(),
            five_hour_util: Some(0.42),
            five_hour_resets_at: Some(chrono::Utc.with_ymd_and_hms(2026, 5, 23, 12, 0, 0).unwrap()),
            seven_day_util: Some(0.10),
            seven_day_resets_at: None,
            subscription_type: "pro".into(),
            rate_limit_tier: "default".into(),
        }
    }

    #[test]
    fn calib_log_parquet_schema_values_and_nulls() {
        let bytes = calibration_log_parquet(&[sample_calib()]).unwrap();
        let batch = read_back(&bytes);

        let names: Vec<&str> = batch.schema().fields().iter().map(|f| f.name().as_str()).collect();
        assert_eq!(names, vec![
            "sampled_at", "util_5h", "util_7d",
            "burn_5h_cost_weighted", "burn_7d_cost_weighted",
            "input_5h", "cache_creation_5h", "cache_read_5h", "output_5h",
            "input_7d", "cache_creation_7d", "cache_read_7d", "output_7d",
            "subscription_type", "rate_limit_tier", "resets_5h_iso", "resets_7d_iso",
        ]);
        assert_eq!(batch.num_rows(), 1);

        // sampled_at is a UTC-tagged millisecond timestamp.
        use arrow::datatypes::{DataType, TimeUnit};
        assert_eq!(
            batch.schema().field(0).data_type(),
            &DataType::Timestamp(TimeUnit::Millisecond, Some("UTC".into()))
        );

        use arrow::array::{Float64Array, StringArray};
        let u5 = batch.column(1).as_any().downcast_ref::<Float64Array>().unwrap();
        assert!((u5.value(0) - 0.42).abs() < 1e-9);

        // burn columns are entirely null.
        let burn5 = batch.column(3).as_any().downcast_ref::<Float64Array>().unwrap();
        assert!(burn5.is_null(0));

        // seven_day_resets_at was None -> resets_7d_iso null.
        let r7 = batch.column(16).as_any().downcast_ref::<StringArray>().unwrap();
        assert!(r7.is_null(0));

        let sub = batch.column(13).as_any().downcast_ref::<StringArray>().unwrap();
        assert_eq!(sub.value(0), "pro");
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --lib sync::export::tests::calib_log_parquet_schema_values_and_nulls`
Expected: FAIL — `calibration_log_parquet` not found.

- [ ] **Step 3: Implement `calibration_log_parquet`**

Add to `src/sync/export.rs` (extend the `use arrow::array::...` line to include `Float64Array, TimestampMillisecondArray`, and the `use arrow::datatypes::...` line to include `TimeUnit`):
```rust
use crate::log::calibration::CalibrationSample;

/// Serialize the calibration log (one row per sample) to parquet bytes. Columns
/// the cloud viewer does not read (burns + per-window token aggregates) are
/// emitted as all-null; see the Stage 7 spec.
pub fn calibration_log_parquet(samples: &[CalibrationSample]) -> Result<Vec<u8>> {
    let n = samples.len();

    let schema = Arc::new(Schema::new(vec![
        Field::new("sampled_at", DataType::Timestamp(TimeUnit::Millisecond, Some("UTC".into())), false),
        Field::new("util_5h", DataType::Float64, true),
        Field::new("util_7d", DataType::Float64, true),
        Field::new("burn_5h_cost_weighted", DataType::Float64, true),
        Field::new("burn_7d_cost_weighted", DataType::Float64, true),
        Field::new("input_5h", DataType::Int64, true),
        Field::new("cache_creation_5h", DataType::Int64, true),
        Field::new("cache_read_5h", DataType::Int64, true),
        Field::new("output_5h", DataType::Int64, true),
        Field::new("input_7d", DataType::Int64, true),
        Field::new("cache_creation_7d", DataType::Int64, true),
        Field::new("cache_read_7d", DataType::Int64, true),
        Field::new("output_7d", DataType::Int64, true),
        Field::new("subscription_type", DataType::Utf8, false),
        Field::new("rate_limit_tier", DataType::Utf8, false),
        Field::new("resets_5h_iso", DataType::Utf8, true),
        Field::new("resets_7d_iso", DataType::Utf8, true),
    ]));

    let sampled_at = TimestampMillisecondArray::from(
        samples.iter().map(|s| s.ts.timestamp_millis()).collect::<Vec<i64>>(),
    )
    .with_timezone("UTC");

    // Helper closures for the all-null columns of length n.
    let null_f64 = || Arc::new(Float64Array::from(vec![None::<f64>; n])) as ArrayRef;
    let null_i64 = || Arc::new(Int64Array::from(vec![None::<i64>; n])) as ArrayRef;

    let columns: Vec<ArrayRef> = vec![
        Arc::new(sampled_at),
        Arc::new(Float64Array::from(samples.iter().map(|s| s.five_hour_util).collect::<Vec<Option<f64>>>())),
        Arc::new(Float64Array::from(samples.iter().map(|s| s.seven_day_util).collect::<Vec<Option<f64>>>())),
        null_f64(), // burn_5h_cost_weighted
        null_f64(), // burn_7d_cost_weighted
        null_i64(), null_i64(), null_i64(), null_i64(), // *_5h
        null_i64(), null_i64(), null_i64(), null_i64(), // *_7d
        Arc::new(StringArray::from_iter_values(samples.iter().map(|s| s.subscription_type.clone()))),
        Arc::new(StringArray::from_iter_values(samples.iter().map(|s| s.rate_limit_tier.clone()))),
        Arc::new(StringArray::from(samples.iter().map(|s| s.five_hour_resets_at.map(|d| d.to_rfc3339())).collect::<Vec<Option<String>>>())),
        Arc::new(StringArray::from(samples.iter().map(|s| s.seven_day_resets_at.map(|d| d.to_rfc3339())).collect::<Vec<Option<String>>>())),
    ];

    let batch = RecordBatch::try_new(schema.clone(), columns)?;
    write_parquet(schema, &batch)
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test --lib sync::export`
Expected: PASS (both export tests).

- [ ] **Step 5: Commit**

```bash
git add src/sync/export.rs
git commit -m "feat(stage-7): calibration_log.parquet writer with null-padded unused columns"
```

---

## Task 5: `sync::export::caps_json`

**Files:**
- Modify: `src/sync/export.rs`
- Test: in-file `#[cfg(test)]` module

`CapsJson` mirrors `caps.py` `DerivedCaps` (13 fields, same order). Populate `sampled_at`/`sample_util_*`/`resets_*_iso` from `AppSnapshot.last_sample`, `subscription_type`/`rate_limit_tier` from `Credentials`; everything else null.

- [ ] **Step 1: Write the failing test**

Add to the `tests` module in `src/sync/export.rs`:
```rust
    use crate::api::credentials::Credentials;
    use crate::api::usage::{UsageBucket, UsageSnapshot};
    use crate::shared::snapshot::AppSnapshot;

    #[test]
    fn caps_json_populates_from_snapshot_and_nulls_the_rest() {
        let usage = UsageSnapshot {
            five_hour: Some(UsageBucket {
                utilization: 0.42,
                resets_at: Some(chrono::Utc.with_ymd_and_hms(2026, 5, 23, 12, 0, 0).unwrap()),
            }),
            seven_day: Some(UsageBucket { utilization: 0.1, resets_at: None }),
        };
        let sampled = chrono::Utc.with_ymd_and_hms(2026, 5, 23, 9, 30, 0).unwrap();
        let snapshot = AppSnapshot {
            last_sample: Some((usage, sampled)),
            ..Default::default()
        };
        let creds = Credentials {
            access_token: "t".into(),
            subscription_type: "pro".into(),
            rate_limit_tier: "default".into(),
        };

        let bytes = caps_json(&snapshot, &creds).unwrap();
        let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();

        assert_eq!(v["sample_util_5h"], 0.42);
        assert_eq!(v["sample_util_7d"], 0.1);
        assert_eq!(v["subscription_type"], "pro");
        assert_eq!(v["rate_limit_tier"], "default");
        assert_eq!(v["sampled_at"], sampled.to_rfc3339());
        assert_eq!(v["resets_5h_iso"], "2026-05-23T12:00:00+00:00");
        assert!(v["resets_7d_iso"].is_null());
        // Per-plan caps + burns are null (not derived in Stage 7).
        assert!(v["max5x_5h"].is_null());
        assert!(v["pro_weekly"].is_null());
        assert!(v["sample_burn_5h"].is_null());
        // All 13 keys present.
        assert_eq!(v.as_object().unwrap().len(), 13);
    }

    #[test]
    fn caps_json_handles_no_sample() {
        let creds = Credentials { access_token: "t".into(), subscription_type: "pro".into(), rate_limit_tier: "default".into() };
        let bytes = caps_json(&AppSnapshot::default(), &creds).unwrap();
        let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert!(v["sampled_at"].is_null());
        assert!(v["sample_util_5h"].is_null());
        assert_eq!(v["subscription_type"], "pro");
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --lib sync::export::tests::caps_json_populates_from_snapshot_and_nulls_the_rest`
Expected: FAIL — `caps_json` not found.

- [ ] **Step 3: Implement `caps_json` + `CapsJson`**

Add to `src/sync/export.rs`:
```rust
use crate::api::credentials::Credentials;
use crate::shared::snapshot::AppSnapshot;
use serde::Serialize;

/// Mirrors caps.py `DerivedCaps`. Field order matches the Python dataclass.
/// `Option::None` serializes to JSON `null`; `serde_json` always emits the key.
#[derive(Debug, Serialize)]
struct CapsJson {
    max5x_5h: Option<f64>,
    max5x_weekly: Option<f64>,
    pro_5h: Option<f64>,
    pro_weekly: Option<f64>,
    sampled_at: Option<String>,
    sample_burn_5h: Option<f64>,
    sample_burn_7d: Option<f64>,
    sample_util_5h: Option<f64>,
    sample_util_7d: Option<f64>,
    subscription_type: Option<String>,
    resets_5h_iso: Option<String>,
    resets_7d_iso: Option<String>,
    rate_limit_tier: Option<String>,
}

/// Build caps.json bytes (pretty-printed, like the Python agent).
pub fn caps_json(snapshot: &AppSnapshot, creds: &Credentials) -> Result<Vec<u8>> {
    let (sampled_at, util_5h, util_7d, resets_5h, resets_7d) = match &snapshot.last_sample {
        Some((usage, at)) => (
            Some(at.to_rfc3339()),
            usage.five_hour.as_ref().map(|b| b.utilization),
            usage.seven_day.as_ref().map(|b| b.utilization),
            usage.five_hour.as_ref().and_then(|b| b.resets_at).map(|d| d.to_rfc3339()),
            usage.seven_day.as_ref().and_then(|b| b.resets_at).map(|d| d.to_rfc3339()),
        ),
        None => (None, None, None, None, None),
    };

    let caps = CapsJson {
        max5x_5h: None,
        max5x_weekly: None,
        pro_5h: None,
        pro_weekly: None,
        sampled_at,
        sample_burn_5h: None,
        sample_burn_7d: None,
        sample_util_5h: util_5h,
        sample_util_7d: util_7d,
        subscription_type: Some(creds.subscription_type.clone()),
        resets_5h_iso: resets_5h,
        resets_7d_iso: resets_7d,
        rate_limit_tier: Some(creds.rate_limit_tier.clone()),
    };

    Ok(serde_json::to_vec_pretty(&caps)?)
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test --lib sync::export`
Expected: PASS (all export tests).

- [ ] **Step 5: Commit**

```bash
git add src/sync/export.rs
git commit -m "feat(stage-7): caps.json builder mirroring python DerivedCaps schema"
```

---

## Task 6: `sync::storage` — `ObjectStore` trait + `SupabaseStore`

**Files:**
- Modify: `src/sync/storage.rs`
- Test: in-file `#[cfg(test)]` module (constructs the store; no network call)

- [ ] **Step 1: Write the failing test**

Put this in `src/sync/storage.rs`:
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::sync::config::SyncConfig;

    #[test]
    fn supabase_store_builds_object_url() {
        let cfg = SyncConfig {
            url: "https://x.supabase.co".into(),
            service_role_key: "key123".into(),
            bucket: "usage-tracker".into(),
            prefix: "borgi".into(),
        };
        let store = SupabaseStore::new(&cfg);
        assert_eq!(
            store.object_url("borgi/cache.parquet"),
            "https://x.supabase.co/storage/v1/object/usage-tracker/borgi/cache.parquet"
        );
    }

    #[test]
    fn trailing_slash_in_url_is_trimmed() {
        let cfg = SyncConfig {
            url: "https://x.supabase.co/".into(),
            service_role_key: "k".into(),
            bucket: "b".into(),
            prefix: "p".into(),
        };
        let store = SupabaseStore::new(&cfg);
        assert_eq!(store.object_url("p/caps.json"), "https://x.supabase.co/storage/v1/object/b/p/caps.json");
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --lib sync::storage`
Expected: FAIL — `SupabaseStore` not found.

- [ ] **Step 3: Implement the trait + Supabase client**

Add to the top of `src/sync/storage.rs`:
```rust
//! Uploads objects to Supabase Storage over HTTP. The `ObjectStore` trait lets
//! the orchestration in `sync::mod` be tested with a fake (no network).

use crate::sync::config::SyncConfig;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum StorageError {
    #[error("network error: {0}")]
    Network(String),
    #[error("storage returned HTTP {0}")]
    Http(u16),
}

/// Abstract object sink. `object_path` is the full key including the user
/// prefix, e.g. "borgi/cache.parquet".
pub trait ObjectStore {
    fn put(&self, object_path: &str, content_type: &str, bytes: &[u8]) -> Result<(), StorageError>;
}

/// Supabase Storage REST client. Uploads via `PUT /storage/v1/object/{bucket}/{key}`
/// with `x-upsert: true` so existing objects are overwritten.
pub struct SupabaseStore {
    agent: ureq::Agent,
    base_url: String, // trimmed, no trailing slash
    key: String,
    bucket: String,
}

impl SupabaseStore {
    pub fn new(cfg: &SyncConfig) -> Self {
        let agent = ureq::AgentBuilder::new()
            .timeout(std::time::Duration::from_secs(30))
            .build();
        SupabaseStore {
            agent,
            base_url: cfg.url.trim_end_matches('/').to_string(),
            key: cfg.service_role_key.clone(),
            bucket: cfg.bucket.clone(),
        }
    }

    pub fn object_url(&self, object_path: &str) -> String {
        format!("{}/storage/v1/object/{}/{}", self.base_url, self.bucket, object_path)
    }
}

impl ObjectStore for SupabaseStore {
    fn put(&self, object_path: &str, content_type: &str, bytes: &[u8]) -> Result<(), StorageError> {
        let url = self.object_url(object_path);
        let resp = self
            .agent
            .put(&url)
            .set("Authorization", &format!("Bearer {}", self.key))
            .set("apikey", &self.key)
            .set("x-upsert", "true")
            .set("Content-Type", content_type)
            .send_bytes(bytes);

        match resp {
            Ok(_) => Ok(()),
            Err(ureq::Error::Status(code, _)) => Err(StorageError::Http(code)),
            Err(e) => Err(StorageError::Network(e.to_string())),
        }
    }
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test --lib sync::storage`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/sync/storage.rs
git commit -m "feat(stage-7): supabase storage uploader behind ObjectStore trait"
```

---

## Task 7: `sync::Syncer` orchestration

**Files:**
- Modify: `src/sync/mod.rs`
- Test: in-file `#[cfg(test)]` module with a fake `ObjectStore`

- [ ] **Step 1: Write the failing test**

Append to `src/sync/mod.rs`:
```rust
use crate::api::credentials::Credentials;
use crate::log::calibration::CalibrationSample;
use crate::shared::snapshot::AppSnapshot;
use crate::sync::config::SyncConfig;
use crate::sync::storage::{ObjectStore, StorageError, SupabaseStore};

/// Builds the three buffers and uploads them under the configured prefix.
pub struct Syncer<S: ObjectStore> {
    config: SyncConfig,
    store: S,
}

impl Syncer<SupabaseStore> {
    /// Construct from `.env`. `Ok(None)` means sync is not configured.
    pub fn from_env() -> anyhow::Result<Option<Self>> {
        match crate::sync::config::from_env()? {
            Some(config) => {
                let store = SupabaseStore::new(&config);
                Ok(Some(Syncer { config, store }))
            }
            None => Ok(None),
        }
    }
}

impl<S: ObjectStore> Syncer<S> {
    /// Best-effort: build + upload all three objects. Per-object errors are
    /// logged and skipped; this never returns an error to the poll loop.
    pub fn run_once(&self, snapshot: &AppSnapshot, creds: &Credentials, samples: &[CalibrationSample]) {
        self.put_buffer("cache.parquet", "application/octet-stream", crate::sync::export::cache_parquet(&snapshot.turns));
        self.put_buffer("calibration_log.parquet", "application/octet-stream", crate::sync::export::calibration_log_parquet(samples));
        self.put_buffer("caps.json", "application/json", crate::sync::export::caps_json(snapshot, creds));
    }

    fn put_buffer(&self, name: &str, content_type: &str, built: anyhow::Result<Vec<u8>>) {
        let object_path = format!("{}/{}", self.config.prefix, name);
        match built {
            Ok(bytes) => match self.store.put(&object_path, content_type, &bytes) {
                Ok(()) => tracing::debug!(object = %object_path, bytes = bytes.len(), "synced"),
                Err(e) => tracing::warn!(object = %object_path, error = %e, "upload failed"),
            },
            Err(e) => tracing::warn!(object = %object_path, error = %e, "serialization failed"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[derive(Default)]
    struct FakeStore {
        puts: Mutex<Vec<(String, String, usize)>>, // (object_path, content_type, byte_len)
    }
    impl ObjectStore for FakeStore {
        fn put(&self, object_path: &str, content_type: &str, bytes: &[u8]) -> Result<(), StorageError> {
            self.puts.lock().unwrap().push((object_path.into(), content_type.into(), bytes.len()));
            Ok(())
        }
    }

    fn cfg() -> SyncConfig {
        SyncConfig { url: "https://x.supabase.co".into(), service_role_key: "k".into(), bucket: "b".into(), prefix: "borgi".into() }
    }

    #[test]
    fn run_once_uploads_three_prefixed_objects() {
        let syncer = Syncer { config: cfg(), store: FakeStore::default() };
        let creds = Credentials { access_token: "t".into(), subscription_type: "pro".into(), rate_limit_tier: "default".into() };

        syncer.run_once(&AppSnapshot::default(), &creds, &[]);

        let puts = syncer.store.puts.lock().unwrap();
        let paths: Vec<&str> = puts.iter().map(|(p, _, _)| p.as_str()).collect();
        assert_eq!(paths, vec![
            "borgi/cache.parquet",
            "borgi/calibration_log.parquet",
            "borgi/caps.json",
        ]);
        assert_eq!(puts[0].1, "application/octet-stream");
        assert_eq!(puts[2].1, "application/json");
        // All three produced non-empty buffers (even empty tables have a parquet footer).
        assert!(puts.iter().all(|(_, _, len)| *len > 0));
    }
}
```

- [ ] **Step 2: Replace the module declarations note**

`src/sync/mod.rs` currently starts with the doc comment + `pub mod` lines from Task 1. Keep those at the top; the code above goes below them. Confirm the file begins with:
```rust
pub mod config;
pub mod export;
pub mod storage;
```
and the `use` + `Syncer` code follows.

- [ ] **Step 3: Run test to verify it passes**

Run: `cargo test --lib sync`
Expected: PASS (config, export, storage, and the new orchestration test).

Beginner note: the fake records each `put` instead of hitting the network, so we assert exactly which objects would be uploaded, in order, with the right content types.

- [ ] **Step 4: Commit**

```bash
git add src/sync/mod.rs
git commit -m "feat(stage-7): Syncer orchestration with best-effort per-object upload"
```

---

## Task 8: Wire `Syncer` into the tray polling loop

**Files:**
- Modify: `src/tray/poller.rs:76-164`

The poller builds `AppSnapshot` at lines ~136-145 and reads the calibration log inside `compute_calibration_with_turns`. We construct the `Syncer` once before the loop, and call it each tick after the snapshot is written. We re-read the calibration log for the sync (a cheap file read) to keep the change local.

- [ ] **Step 1: Construct the Syncer before the loop**

In `src/tray/poller.rs`, inside `polling_loop`, after the existing "publish Initial status" block (around line 102) and before `while !shutdown...`, add:
```rust
    // Stage 7: best-effort Supabase sync. `None` when unconfigured (no .env) —
    // the agent then behaves exactly as before.
    let syncer = match crate::sync::Syncer::from_env() {
        Ok(s) => {
            if s.is_some() {
                tracing::info!("supabase sync enabled");
            } else {
                tracing::info!("supabase sync disabled (no .env config)");
            }
            s
        }
        Err(e) => {
            tracing::warn!(error = %e, "supabase sync config invalid; disabled");
            None
        }
    };
```

- [ ] **Step 2: Call sync each tick using the freshly-built snapshot**

The loop builds the local `snapshot` value (around line 136-145) and then *moves* it into the shared lock with `*g = snapshot` (around line 146-151). Insert the sync call **between** building `snapshot` and the `match shared.write()` block, so we borrow the local value before it's moved (no re-locking needed):

```rust
        // Stage 7: best-effort upload of the snapshot we just built. Re-read the
        // calibration log (cheap file read) so the parquet matches this tick.
        if let Some(syncer) = &syncer {
            let samples = crate::log::calibration::read_all_default().unwrap_or_default();
            syncer.run_once(&snapshot, &creds, &samples);
        }

        match shared.write() {
```

i.e. the existing `match shared.write() { ... }` block stays exactly as-is, immediately after this new block. The new code goes right before it.

Beginner note: `&snapshot` borrows the value; the borrow ends when `run_once` returns, so the following `*g = snapshot` move still compiles. `run_once` is best-effort and logs its own errors, so there is nothing to handle here.

- [ ] **Step 3: Build + run the existing test suite**

Run: `cargo build && cargo test`
Expected: PASS. (No new automated test — this is glue around a visual/long-running loop; the orchestration is already covered in Task 7.)

- [ ] **Step 4: Clippy + fmt clean (release gate)**

Run: `cargo fmt && cargo clippy --all-targets -- -D warnings`
Expected: no warnings. Fix any that appear (e.g. unused imports).

- [ ] **Step 5: Commit**

```bash
git add src/tray/poller.rs
git commit -m "feat(stage-7): wire supabase sync into the tray polling loop"
```

---

## Task 9: Manual smoke test against a real bucket

**Files:** none (manual verification)

This is the feature-correctness check the automated tests can't give. Requires a Supabase project with a Storage bucket (default name `usage-tracker`).

- [ ] **Step 1: Create `.env` in the repo root**

```
SUPABASE_URL=https://YOUR_PROJECT_REF.supabase.co
SUPABASE_SERVICE_ROLE_KEY=eyJhbGciOi....
SUPABASE_BUCKET=usage-tracker
SUPABASE_USER_PREFIX=borgi
```

- [ ] **Step 2: Run the tray agent**

Run: `cargo run` (the default tray mode that runs the polling loop).
Watch the log for `supabase sync enabled` then, after the first tick, `synced` debug lines for the three objects. (Use `RUST_LOG=debug` if debug lines aren't visible.)

- [ ] **Step 3: Verify in Supabase**

In the Supabase dashboard → Storage → `usage-tracker`, confirm `borgi/cache.parquet`, `borgi/calibration_log.parquet`, `borgi/caps.json` exist and update on subsequent ticks.

- [ ] **Step 4: Verify the cloud viewer reads them**

Point the cloud viewer (`claude-usage-tracker/app_cloud.py`, separate repo) at this bucket/prefix and confirm it renders without schema errors. If the viewer still reads bucket-root paths, that prefix change is the documented follow-up — note it and move on; it is out of scope for this stage.

- [ ] **Step 5: Confirm opt-out**

Rename `.env` to `.env.off`, run `cargo run` again, confirm the log shows `supabase sync disabled (no .env config)` and the agent otherwise behaves normally. Restore `.env` afterward (or leave off).

---

## Task 10: Docs + ignore + project memory

**Files:**
- Create: `.env.example`
- Modify: `.gitignore`
- Modify: `CLAUDE.md`

- [ ] **Step 1: Add `.env.example`**

Create `.env.example`:
```
# Supabase sync (Stage 7). Copy to .env and fill in. NEVER commit .env.
# service_role key: Supabase Dashboard > Project Settings > API > service_role.
# This key has full admin access to the bucket — treat as a secret.
SUPABASE_URL=https://YOUR_PROJECT_REF.supabase.co
SUPABASE_SERVICE_ROLE_KEY=eyJhbGciOi....
SUPABASE_BUCKET=usage-tracker
# Arbitrary anonymous handle that namespaces this machine's files in the bucket.
# Letters, digits, '-', '_' only.
SUPABASE_USER_PREFIX=changeme
```

- [ ] **Step 2: Ignore `.env`**

Confirm `.gitignore` contains a line `.env` (add it if missing). Verify `git status` does NOT list your local `.env`.

- [ ] **Step 3: Update `CLAUDE.md`**

In the "Active design + plans" list, add:
```markdown
- **Stage 7 spec:** `docs/superpowers/specs/2026-05-23-stage-7-supabase-sync-design.md` — Supabase Storage upload, parquet/format-parity findings, per-user layout.
- **Stage 7 plan:** `docs/superpowers/plans/2026-05-23-stage-7-supabase-sync.md` — task plan.
```
In the stage roadmap table, change the Stage 7 status cell from `Pending` to `✅ Shipped — tag v0.7.0` only **after** the release tag is cut (leave as Pending / "In progress" until then). Add `arrow`, `parquet`, `dotenvy` to the Tech stack list (note: parquet revisits the original "no parquet" exclusion — scoped to the sync path only).

- [ ] **Step 4: Commit**

```bash
git add .env.example .gitignore CLAUDE.md
git commit -m "docs(stage-7): .env.example, gitignore .env, update CLAUDE.md pointers"
```

---

## Release (after all tasks pass)

- [ ] `cargo fmt && cargo clippy --all-targets -- -D warnings` clean.
- [ ] `cargo test` green.
- [ ] Manual smoke test (Task 9) confirmed against a real bucket.
- [ ] Bump `version` in `Cargo.toml` to `0.7.0`; commit `chore: bump to v0.7.0`.
- [ ] Tag `v0.7.0` and push (per existing release convention).
- [ ] Flip the Stage 7 roadmap cell in `CLAUDE.md` to shipped.

---

## Notes / known limitations (carried from the spec)

- **Auth:** the agent uses the admin `service_role` key from `.env`. Sharing it with teammates grants each full bucket admin. Per-user scoped credentials / RLS are a deferred follow-up.
- **Full-file re-upload each tick** — accepted for now; incremental upload is future work as `calibration_log` grows.
- **Null columns** in `calibration_log.parquet` (burns + per-window aggregates) and `caps.json` (`max5x_*`/`pro_*`/`sample_burn_*`): the cloud viewer's current anchor method doesn't read them; populating them is deferred to the Stage 8 calibration-history port.
- **Cloud-viewer multi-user UI** (per-user picker) lives in the separate Python repo — not built here.
