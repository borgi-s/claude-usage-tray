# Stage 7 — Supabase Sync Design

Companion to the top-level [rust-tray-widget-design.md](2026-05-22-rust-tray-widget-design.md). This document fixes the details that the top-level spec left open for Stage 7.

## Goal

The Rust agent uploads `cache.parquet`, `calibration_log.parquet`, and `caps.json` to Supabase Storage on each poll tick, under a per-user path prefix, so the existing polars-based cloud viewer (`app_cloud.py` in the `claude-usage-tracker` repo) can read them. After this stage the Python local agent (`app.py`) can be retired from the Windows machine.

## The finding that reshaped this stage

The top-level roadmap's one-liner — *"upload cache + caps + calibration_log; cloud viewer reads the files unchanged — zero changes there"* — hid a **format conflict** uncovered during the Stage 7 brainstorm:

The cloud viewer (`app_cloud.py:28`) hardcodes and `pl.read_parquet`s exactly these three filenames at the bucket root:

- `cache.parquet`
- `caps.json`
- `calibration_log.parquet`

But the Rust agent stores its data in **different formats**, and the project's tech stack **explicitly excludes parquet/polars**:

| File | Cloud viewer reads | Rust agent stores today |
|---|---|---|
| cache | `cache.parquet` | `cache.bincode` |
| calibration log | `calibration_log.parquet` | `calibration_log.jsonl` |
| caps | `caps.json` (13 fields) | not persisted — `DerivedCaps` is in-memory, 4 fields |

So the real work of Stage 7 is **format parity**, not the HTTP PUT (which is trivial). Two decisions resolved the conflict during the brainstorm (see below): keep **parquet** (data is append-heavy tabular, consumer is polars), and produce it via an **isolated sync-export module** so the parquet dependency stays quarantined.

### Secondary finding: the calibration log column gap (resolved without a Stage 5 change)

The Rust `CalibrationSample` (`src/log/calibration.rs`) persists only `ts`, `five_hour_util`, `seven_day_util`, `five_hour_resets_at`, `seven_day_resets_at`, `subscription_type`, `rate_limit_tier` — **7 fields**. The Python `calibration_log.parquet` schema has **17 columns**, additionally carrying cost-weighted burns (`burn_5h_cost_weighted`, `burn_7d_cost_weighted`) and per-window token aggregates (`input_5h` … `output_7d`) that the Python agent computes from the cache at sample time.

That looked like a second parity gap, but it collapses on inspection of what the **cloud viewer actually reads**:

- `app_cloud.py` derives caps via `caps_mod.global_cap_from_anchors(log, df, kind, …)` (`caps.py:358`), which finds anchors using `util_5h`/`util_7d` + `sampled_at` from the log, then sums **`output_tokens` from the cache `df`** for the burn. It does **not** read the log's burn/aggregate columns.
- `metrics.effective_window_hours(log, df, …)` reads `resets_5h_iso` from the log.
- The `burn_*_cost_weighted` + per-window token columns are read only by the **continuous / per-hour** functions (`implied_cap_series`, `hour_of_day_sample_counts`, `_per_hour_medians`) — these power the **Stage 8 calibration-history view**, which `app_cloud.py` does not call.

So every log column the cloud viewer reads today (`sampled_at`, `util_5h`, `util_7d`, `resets_5h_iso`, `resets_7d_iso`) already exists in the Rust `CalibrationSample`. The remaining 10 columns are emitted as **null** — the same graceful-degradation pattern as `caps.json`'s per-plan cap fields. **No Stage 5 schema change is needed for Stage 7.**

## Non-goals (Stage 7)

- ❌ **Cloud-viewer multi-user UI** (user picker / cross-user aggregation) — separate work in the `claude-usage-tracker` Python repo; not edited from the Rust project.
- ❌ **Auth hardening** — the agent uses the all-powerful `service_role` key from `.env`, mirroring the Python agent. Giving every teammate that key grants each full admin over the whole bucket. Per-user scoped credentials / RLS-with-anon-key + per-folder policies are deferred. **Documented known risk.**
- ❌ **Incremental / partitioned upload** — the whole file is re-serialized and re-uploaded each tick (exactly what the Python agent does). Accepted; flagged as future optimization once `calibration_log` grows large.
- ❌ **Switching local storage to parquet** — native `cache.bincode` + `calibration_log.jsonl` remain the canonical local format. Parquet is produced only for upload.
- ❌ **Per-plan cap differentiation** in `caps.json` (`max5x_*` / `pro_*`) — emitted as null; Rust calibration yields a single effective cap, and the viewer guards on these fields.
- ❌ **Populating `calibration_log` burn + per-window aggregate columns** — emitted as null. The cloud viewer's anchor method doesn't read them; the Stage 8 calibration-history / continuous-caps views do. Full population (compute at export time from turns, or extend `CalibrationSample` to capture at sample time) is deferred to whenever those views are ported.

## Locked-in design decisions

Settled during the Stage 7 brainstorm:

| Decision | Value | Rationale |
|---|---|---|
| Upload format | **parquet** for cache + calibration_log; **JSON** for caps | Two files are append-heavy tabular time-series; consumer is polars. Parquet is 5–10× smaller than JSON for numeric tables, typed, and natively/fast-read by polars. Cloud viewer re-downloads the whole file each refresh, so size directly affects egress + load time. |
| Parquet crate | arrow-rs `parquet` (+ `arrow`) | Pure Rust, builds clean on MSVC. Revisits the "no parquet" stack exclusion — accepted, scoped narrowly to the sync path only. |
| Parquet source | **In-memory `AppSnapshot` → buffers** (sync-export module) | Isolated; zero changes to Stage 5/6 storage code; parquet dep quarantined; best-effort failures don't crash the agent. |
| Multi-user scope | **Rust agent + per-user bucket layout only** | Viewer UI + auth hardening are separate follow-ups. Ships a real Rust deliverable without dragging in the other repo. |
| User identity | Arbitrary anon handle in `.env` (`SUPABASE_USER_PREFIX`) | Users are anonymous; the handle is just a label to distinguish them. Validated path-safe. |
| Sync enablement | Opt-in: skipped (with one log line) if required env vars absent | Agent behaves exactly as today when unconfigured. |
| Credential | `service_role` key from `.env` via `dotenvy` | Parity with Python agent; hardening deferred. |

## Module layout

New `src/sync/` module, fully isolated from existing storage:

- `sync/mod.rs` — orchestrates a sync pass: build the three buffers, upload each, best-effort.
- `sync/export.rs` — serializes into parquet/json byte buffers: cache + caps from the in-memory `AppSnapshot`, the calibration log from disk via `log::calibration::read_all_default()`.
- `sync/storage.rs` — Supabase Storage HTTP client (ureq PUT), behind a small trait so `mod.rs` is testable with a fake.
- `sync/config.rs` — reads + validates `.env` settings.

## Configuration

Read from `.env` via `dotenvy`:

| Var | Meaning | Default |
|---|---|---|
| `SUPABASE_URL` | project URL | — (required) |
| `SUPABASE_SERVICE_ROLE_KEY` | upload credential | — (required) |
| `SUPABASE_BUCKET` | bucket name | `usage-tracker` |
| `SUPABASE_USER_PREFIX` | arbitrary anon handle | — (required) |

Sync is **opt-in**: if any required var is missing, sync is skipped and the agent logs one `tracing` info line, otherwise running exactly as today. `SUPABASE_USER_PREFIX` is validated path-safe (alphanumeric, `-`, `_`; reject slashes, spaces, dots-only) to prevent path injection into the object key.

## Bucket layout

```
{bucket}/{prefix}/cache.parquet
{bucket}/{prefix}/caps.json
{bucket}/{prefix}/calibration_log.parquet
```

Per-user prefix from the start → no collisions when teammates onboard, and no later path migration. (The current Python agent writes to the bucket root; teammates adopting the Rust agent move to prefixed paths.)

## Parquet schemas

Match the Python polars schemas **exactly** — column names, types, and order — so the viewer reads them with no change.

### cache.parquet (from `AppSnapshot.turns: Vec<Turn>`)

| Column | Type |
|---|---|
| `timestamp` | Utf8 |
| `session_id` | Utf8 |
| `subagent_id` | Utf8 (nullable) |
| `is_subagent` | Boolean |
| `project_cwd` | Utf8 |
| `model` | Utf8 |
| `version` | Utf8 |
| `input_tokens` | Int64 |
| `output_tokens` | Int64 |
| `cache_creation_input_tokens` | Int64 |
| `cache_read_input_tokens` | Int64 |
| `source_file` | Utf8 |
| `is_rate_limit_error` | Boolean |

### calibration_log.parquet (from the Rust `CalibrationSample` records)

Source the rows from `log::calibration::read_all_default()` (the existing JSONL log). Map column names + types; null-pad the columns the cloud viewer doesn't read.

| Column | Type | Source |
|---|---|---|
| `sampled_at` | `timestamp(ms, tz="UTC")` — **must** match polars `Datetime("ms","UTC")` so it round-trips | `CalibrationSample.ts` |
| `util_5h`, `util_7d` | Float64 | `five_hour_util`, `seven_day_util` |
| `resets_5h_iso`, `resets_7d_iso` | Utf8 | `five_hour_resets_at`, `seven_day_resets_at` → ISO 8601 |
| `subscription_type`, `rate_limit_tier` | Utf8 | `subscription_type`, `rate_limit_tier` |
| `burn_5h_cost_weighted`, `burn_7d_cost_weighted` | Float64 | **null** — read only by Stage 8 continuous/per-hour analysis |
| `input_5h`, `cache_creation_5h`, `cache_read_5h`, `output_5h` | Int64 | **null** — same |
| `input_7d`, `cache_creation_7d`, `cache_read_7d`, `output_7d` | Int64 | **null** — same |

The cloud viewer's anchor-based cap derivation reads only the populated columns; the nulls degrade gracefully (the Stage 8 calibration-history view isn't ported yet).

## caps.json

Widen the in-memory `DerivedCaps` (4 fields) into a 13-field serde struct matching the Python schema. Populate from `AppSnapshot`:

| Field | Source | Notes |
|---|---|---|
| `sampled_at` | `last_sample` timestamp | ISO 8601 |
| `sample_util_5h`, `sample_util_7d` | `UsageSnapshot` `utilization` (0–1) | drives the viewer's live panel |
| `resets_5h_iso`, `resets_7d_iso` | `UsageSnapshot` `resets_at` | ISO 8601 |
| `subscription_type` | `Credentials` | (known to misreport Max as `pro`; carried as-is) |
| `sample_burn_5h`, `sample_burn_7d` | computed cost-weighted window sums from turns | |
| `max5x_5h`, `max5x_weekly`, `pro_5h`, `pro_weekly`, `rate_limit_tier` | **null** | Rust yields a single effective cap, not per-plan caps; the viewer guards on these (`if prev.max5x_5h`), so only the optional cap-caption line is hidden. Graceful degradation. |

## Upload

`PUT /storage/v1/object/{bucket}/{prefix}/{name}` via `ureq`, headers:

- `Authorization: Bearer {service_role_key}`
- `apikey: {service_role_key}`
- `x-upsert: true` (overwrite existing object)
- `Content-Type: application/octet-stream` for parquet, `application/json` for caps

**Best-effort**: any upload error (network, HTTP non-2xx) is logged via `tracing` and the watch loop continues. Sync never crashes the agent.

## Integration

The sync pass is called from the existing `--watch` poll loop after a successful poll + calibration tick. All three files upload each tick (parity with the Python agent's behavior).

## Data flow

```
AppSnapshot.turns          ──────────────> sync::export::cache_parquet      ──┐
AppSnapshot.last_sample+caps ────────────> sync::export::caps_json          ──┤──> sync::storage::put ──> Supabase
log::calibration::read_all_default() ────> sync::export::calib_log_parquet  ──┘     {bucket}/{prefix}/...
```

## Testing

- **Unit:**
  - Parquet round-trip: write a buffer, read it back with arrow, assert row count + schema (column names/types) match.
  - `caps.json` serialized shape: all 13 keys present; nulls where expected; util/reset fields populated from a sample snapshot.
  - Prefix sanitization: accepts safe handles, rejects slashes/spaces/empty.
- **No live-network tests.** The `storage` uploader sits behind a small trait; `mod.rs` orchestration is tested against a fake that records what would be uploaded.

## New dependencies

- `parquet` (arrow-rs) + `arrow` — parquet writing. Adds compile time (~30–60s) and a few MB to the `.exe`; accepted, scoped to sync.
- `dotenvy` — `.env` parsing (already anticipated in the top-level spec).

## Deliverable

Cloud-syncing Rust agent. Tag `v0.7.0`.
