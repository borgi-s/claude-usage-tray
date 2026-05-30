# Viewer Merge (Phase 2) Implementation Plan

> **For agentic workers:** This plan modifies the **companion repo** `claude-usage-tracker`
> (the Streamlit cloud viewer), NOT `claude-usage-tray`. Per `claude-usage-tray/CLAUDE.md`, the
> assistant must NOT edit the companion repo from the tray session — this plan is delivered as a
> **diff for the user to apply** in `C:\Users\borgi\Documents\claude-usage-tracker`. Every code
> block below is complete and paste-ready. The "commit" steps are commands the user runs **in the
> companion repo**. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the Streamlit cloud viewer read **multiple** Supabase prefixes (one per machine), concatenate each machine's `cache.parquet` into one merged view with a `machine` column, and source the account-wide caps/calibration from a single designated (poller) prefix — so Windows + Linux usage appears combined on phone/laptop without clobbering.

**Architecture:** The viewer already redirects `config.CACHE_PATH → DATA_DIR/cache.parquet` and flows everything from `cache.load_cache()` reading that one file. Phase 2 keeps that seam: instead of downloading one prefix's `cache.parquet`, it downloads each prefix's to a distinct local name, stamps a `machine` column, concatenates them, and writes the **merged** result to `config.CACHE_PATH`. All downstream code (metrics, charts, sessions) is untouched. caps.json + calibration_log come from one canonical prefix (the always-on poller, `borgi-linux`).

**Tech Stack:** Python 3.12, `polars==1.40.1`, `supabase==2.6.0`, `streamlit==1.57.0`, `pytest`. Companion repo at `C:\Users\borgi\Documents\claude-usage-tracker`.

---

## Background for the implementer (read once)

The companion repo is a Streamlit app deployed on Streamlit Community Cloud. The relevant files:

- **`app_cloud.py`** — the cloud entry point. On each 5-minute refresh it: reads a prefix from
  `CLOUD_USER_PREFIX`, downloads that prefix's `cache.parquet` + `caps.json` +
  `calibration_log.parquet` into `DATA_DIR`, then `load_data()` calls `cache.load_cache()` (which
  reads `config.CACHE_PATH`, redirected to `DATA_DIR/cache.parquet`) → `metrics.add_derived`.
- **`supabase_sync.py`** — transport. `download_files(client, bucket, names, target_dir, prefix)`
  downloads `{prefix}/{name}` → `target_dir/{name}` (bare local name). `last_modified_at(...)`
  returns an object's `updated_at`.
- **`cache.py`** — parquet data logic. `ROW_SCHEMA` (the 13 columns), `load_cache()` =
  `pl.read_parquet(config.CACHE_PATH)`.
- **`metrics.py`** — `add_derived(df)` parses `timestamp` → `ts`, adds `cost_weighted_tokens` etc.
  It only ADDS columns and references known ones, so an extra `machine` column passes through
  untouched. Confirmed by reading: it never does `select` on a fixed column set that would drop it.

**Why a `machine` column instead of changing the parquet schema:** stamping `machine=<prefix>` at
read time (in the viewer) needs zero change to either Rust writer and sidesteps polars'
`concat(how="vertical")` "identical columns" requirement. We use `how="diagonal"` for extra
robustness if two machines ever run different agent versions.

**Prefixes for this deployment:** Windows writes `borgi/*` (unchanged), Linux writes
`borgi-linux/*` (Phase 1). So the viewer will be configured with
`CLOUD_USER_PREFIXES="borgi,borgi-linux"` and `CLOUD_CAPS_PREFIX="borgi-linux"` (the always-on
poller). Both are account-equivalent for caps during Phase 2 (Windows still polls until Phase 3),
so either prefix's caps are valid; we pick the poller for forward-consistency.

**Test setup:** the repo uses `pytest` with tests in `tests/` (`conftest.py` adds the repo root to
`sys.path`). `tests/test_supabase_sync.py` mocks the Supabase client with `MagicMock`
(`client.storage.from_.return_value = client.storage`). Run tests with `python -m pytest` from the
repo root (activate `.venv` first).

---

## File structure

| File | Change | Responsibility |
|---|---|---|
| `cache.py` | Modify (add fn) | `merge_cache_parquets(prefix_paths, out_path)` — read each machine's cache, stamp `machine`, concat, write merged. |
| `supabase_sync.py` | Modify (add fn) | `download_cache_per_prefix(client, bucket, prefixes, target_dir)` — download each prefix's `cache.parquet` to a distinct local path. |
| `app_cloud.py` | Modify | Read a prefix LIST + caps prefix; in the refresh fragment, download-per-prefix → merge → write `config.CACHE_PATH`; download caps/calib from the caps prefix. |
| `tests/test_cache_merge.py` | Create | Unit tests for `merge_cache_parquets`. |
| `tests/test_supabase_sync.py` | Modify (add tests) | Unit tests for `download_cache_per_prefix`. |

---

### Task 1: `merge_cache_parquets` in `cache.py`

**Files:**
- Modify: `cache.py`
- Test: `tests/test_cache_merge.py` (create)

**Why:** This is the pure data-merge step: given each machine's downloaded `cache.parquet`, stamp a
`machine` column and concatenate into one frame written at `config.CACHE_PATH`, which
`load_cache()` then reads unchanged.

- [ ] **Step 1: Write the failing test**

Create `tests/test_cache_merge.py` with exactly this:

```python
"""Unit tests for cache.merge_cache_parquets — multi-machine concat + machine stamp."""
from __future__ import annotations

from pathlib import Path

import polars as pl

import cache


def _write_cache(path: Path, session_ids: list[str]) -> None:
    """Write a minimal cache.parquet with the real ROW_SCHEMA columns."""
    n = len(session_ids)
    df = pl.DataFrame(
        {
            "timestamp": [f"2026-05-30T10:0{i}:00.000Z" for i in range(n)],
            "session_id": session_ids,
            "subagent_id": [None] * n,
            "is_subagent": [False] * n,
            "project_cwd": ["/p"] * n,
            "model": ["claude-opus-4-7"] * n,
            "version": ["1.0"] * n,
            "input_tokens": [1] * n,
            "output_tokens": [2] * n,
            "cache_creation_input_tokens": [0] * n,
            "cache_read_input_tokens": [0] * n,
            "source_file": ["a.jsonl"] * n,
            "is_rate_limit_error": [False] * n,
        },
        schema=cache.ROW_SCHEMA,
    )
    df.write_parquet(path)


def test_merge_stamps_machine_and_unions_rows(tmp_path: Path):
    win = tmp_path / "cache__borgi.parquet"
    lin = tmp_path / "cache__borgi-linux.parquet"
    _write_cache(win, ["w1", "w2"])
    _write_cache(lin, ["l1"])
    out = tmp_path / "cache.parquet"

    rows = cache.merge_cache_parquets({"borgi": win, "borgi-linux": lin}, out)

    assert rows == 3
    merged = pl.read_parquet(out)
    assert merged.height == 3
    assert "machine" in merged.columns
    assert set(merged["machine"].to_list()) == {"borgi", "borgi-linux"}
    # original columns preserved
    for col in cache.ROW_SCHEMA:
        assert col in merged.columns
    # machine value tracks the source prefix
    win_rows = merged.filter(pl.col("machine") == "borgi")
    assert set(win_rows["session_id"].to_list()) == {"w1", "w2"}


def test_merge_empty_writes_schema_only_cache(tmp_path: Path):
    out = tmp_path / "cache.parquet"
    rows = cache.merge_cache_parquets({}, out)
    assert rows == 0
    merged = pl.read_parquet(out)
    assert merged.height == 0
    assert "machine" in merged.columns
    for col in cache.ROW_SCHEMA:
        assert col in merged.columns
```

- [ ] **Step 2: Run it to confirm it fails**

Run (from companion repo root, venv active): `python -m pytest tests/test_cache_merge.py -v`
Expected: FAIL — `AttributeError: module 'cache' has no attribute 'merge_cache_parquets'`.

- [ ] **Step 3: Implement `merge_cache_parquets`**

In `cache.py`, add this function (e.g. right after `load_cache`, before the `if __name__` block):

```python
def merge_cache_parquets(prefix_paths: dict[str, Path], out_path: Path) -> int:
    """Merge per-machine cache.parquet files into one frame at out_path.

    For each (prefix, path) in prefix_paths, read the parquet and add a `machine`
    column equal to the prefix, then diagonal-concat all of them (diagonal so two
    machines on different agent versions can't break the concat). Writes the merged
    frame to out_path and returns its row count. With no inputs, writes a
    schema-only (empty) cache carrying the `machine` column so load_cache() and the
    downstream pipeline still work.
    """
    frames = []
    for prefix, path in prefix_paths.items():
        df = pl.read_parquet(path).with_columns(pl.lit(prefix).alias("machine"))
        frames.append(df)
    if frames:
        merged = pl.concat(frames, how="diagonal")
    else:
        merged = pl.DataFrame(schema={**ROW_SCHEMA, "machine": pl.Utf8})
    merged.write_parquet(out_path)
    return merged.height
```

> `Path` is already imported in `cache.py` (`from pathlib import Path`), and `pl`/`ROW_SCHEMA` are
> already in scope. No new imports needed.

- [ ] **Step 4: Run the test to confirm it passes**

Run: `python -m pytest tests/test_cache_merge.py -v`
Expected: PASS (both tests).

- [ ] **Step 5: Commit (in the companion repo)**

```bash
git add cache.py tests/test_cache_merge.py
git commit -m "feat(cache): merge_cache_parquets for multi-machine viewer merge"
```

---

### Task 2: `download_cache_per_prefix` in `supabase_sync.py`

**Files:**
- Modify: `supabase_sync.py`
- Test: `tests/test_supabase_sync.py` (add tests)

**Why:** Transport step. The existing `download_files` writes every prefix's `cache.parquet` to the
same bare local name (`target_dir/cache.parquet`), so two prefixes would overwrite each other
locally. This new function downloads each prefix's `cache.parquet` to a **distinct** local path,
returning a `{prefix: Path}` map for the merge step. Missing objects (a machine that hasn't uploaded
yet) are skipped, not fatal.

- [ ] **Step 1: Write the failing tests**

Add these tests to the end of `tests/test_supabase_sync.py` (the `fake_client` fixture already
exists at the top of that file):

```python
def test_download_cache_per_prefix_writes_distinct_local_files(tmp_path, fake_client):
    result = supabase_sync.download_cache_per_prefix(
        fake_client, "usage-tracker", ["borgi", "borgi-linux"], target_dir=tmp_path
    )
    assert set(result.keys()) == {"borgi", "borgi-linux"}
    # distinct local filenames per prefix (no collision)
    assert result["borgi"] != result["borgi-linux"]
    for p in result.values():
        assert p.exists()
        assert p.read_bytes() == b"binary content"
    # downloaded cache.parquet under each prefix
    downloaded = [c.args[0] for c in fake_client.storage.download.call_args_list]
    assert "borgi/cache.parquet" in downloaded
    assert "borgi-linux/cache.parquet" in downloaded


def test_download_cache_per_prefix_skips_missing_object(tmp_path, fake_client):
    # First prefix downloads fine; second raises (no object yet) and is skipped.
    def _dl(remote):
        if remote.startswith("borgi-linux/"):
            raise Exception("Object not found")
        return b"binary content"
    fake_client.storage.download = MagicMock(side_effect=_dl)

    result = supabase_sync.download_cache_per_prefix(
        fake_client, "usage-tracker", ["borgi", "borgi-linux"], target_dir=tmp_path
    )
    assert set(result.keys()) == {"borgi"}
    assert result["borgi"].exists()
```

- [ ] **Step 2: Run them to confirm they fail**

Run: `python -m pytest tests/test_supabase_sync.py -v`
Expected: the two new tests FAIL — `AttributeError: module 'supabase_sync' has no attribute 'download_cache_per_prefix'`. (Existing tests still pass.)

- [ ] **Step 3: Implement `download_cache_per_prefix`**

In `supabase_sync.py`, add this function (e.g. right after `download_files`):

```python
def download_cache_per_prefix(
    client, bucket: str, prefixes: Iterable[str], target_dir: Path
) -> dict[str, Path]:
    """Download each prefix's `cache.parquet` to a DISTINCT local file so per-machine
    caches don't overwrite each other locally. Returns {prefix: local_path} for the
    prefixes whose object downloaded successfully; a prefix with no object yet (e.g. a
    machine that hasn't uploaded) is skipped silently. The local name encodes the
    prefix; an empty-string prefix (bucket root) maps to 'root'."""
    target_dir = Path(target_dir)
    target_dir.mkdir(parents=True, exist_ok=True)
    out: dict[str, Path] = {}
    for prefix in prefixes:
        remote = f"{prefix}/cache.parquet" if prefix else "cache.parquet"
        try:
            data = client.storage.from_(bucket).download(remote)
        except Exception:
            # Machine hasn't uploaded a cache.parquet yet — skip it.
            continue
        local = target_dir / f"cache__{prefix or 'root'}.parquet"
        local.write_bytes(data)
        out[prefix] = local
    return out
```

> `Path` and `Iterable` are already imported at the top of `supabase_sync.py`. No new imports.

- [ ] **Step 4: Run the tests to confirm they pass**

Run: `python -m pytest tests/test_supabase_sync.py -v`
Expected: PASS (new tests + all existing supabase_sync tests).

- [ ] **Step 5: Commit (in the companion repo)**

```bash
git add supabase_sync.py tests/test_supabase_sync.py
git commit -m "feat(sync): download_cache_per_prefix for multi-machine viewer merge"
```

---

### Task 3: Wire the multi-prefix merge into `app_cloud.py`

**Files:**
- Modify: `app_cloud.py`

**Why:** This connects Tasks 1+2 into the live refresh path: resolve the prefix list + caps prefix
from env, download each machine's cache, merge to `config.CACHE_PATH`, and download caps/calibration
from the canonical prefix. Everything after `load_data()` is unchanged.

- [ ] **Step 1: Replace the single-prefix constant block**

Find this block near the top of `app_cloud.py`:

```python
DATA_DIR = Path(os.environ.get("CLOUD_DATA_DIR", "/tmp/usage-tracker"))
FILE_NAMES = ["cache.parquet", "caps.json", "calibration_log.parquet"]
# Per-user folder in the bucket (the Rust agent's SUPABASE_USER_PREFIX). Empty =
# read from bucket root, matching the legacy Python agent's upload location.
USER_PREFIX = os.environ.get("CLOUD_USER_PREFIX", "").strip("/")
```

Replace it with:

```python
DATA_DIR = Path(os.environ.get("CLOUD_DATA_DIR", "/tmp/usage-tracker"))
# Account-wide files (caps + calibration) come from ONE canonical prefix — the
# always-on poller. The merged cache.parquet is built from all machine prefixes.
CAPS_FILE_NAMES = ["caps.json", "calibration_log.parquet"]


def _prefixes() -> list[str]:
    """Machine prefixes to merge. CLOUD_USER_PREFIXES (comma-separated) wins;
    falls back to the legacy single CLOUD_USER_PREFIX ('' = bucket root)."""
    multi = os.environ.get("CLOUD_USER_PREFIXES", "").strip()
    if multi:
        return [p.strip().strip("/") for p in multi.split(",") if p.strip()]
    return [os.environ.get("CLOUD_USER_PREFIX", "").strip("/")]


PREFIXES = _prefixes()
# Caps/calibration are account-wide (same on every machine), so read them from one
# prefix — the always-on poller. Defaults to the first prefix if unset.
CAPS_PREFIX = os.environ.get("CLOUD_CAPS_PREFIX", PREFIXES[0]).strip("/")
```

- [ ] **Step 2: Replace the refresh fragment**

Find the whole `refresh_data_panel` function:

```python
@st.fragment(run_every=300)
def refresh_data_panel():
    client, bucket = _client()
    try:
        mtime = supabase_sync.last_modified_at(client, bucket, "cache.parquet", prefix=USER_PREFIX)
        mtime_key = mtime.isoformat() if mtime else None
        # Only re-download (and invalidate the chart cache) when the agent has
        # actually written new data. Skipping the download on no-op polls keeps
        # the fragment's stale-fade brief.
        if mtime_key != st.session_state.get("last_cache_mtime"):
            supabase_sync.download_files(client, bucket, FILE_NAMES, target_dir=DATA_DIR, prefix=USER_PREFIX)
            st.session_state["last_cache_mtime"] = mtime_key
            load_data.clear()
        seconds_old = None
        if mtime is not None:
            seconds_old = (datetime.now(tz=timezone.utc) - mtime).total_seconds()
        render.render_live_panel_from_cache(agent_seconds_old=seconds_old)
    except Exception as e:
        st.error(f"Could not fetch latest from Supabase: {e}")
```

Replace it entirely with:

```python
@st.fragment(run_every=300)
def refresh_data_panel():
    client, bucket = _client()
    try:
        # Freshness across ALL machines: re-download only when any prefix's
        # cache.parquet changed. Compute each prefix's mtime once; reuse for both
        # the change-key and the "newest activity" age shown in the live panel.
        mtimes = {
            p: supabase_sync.last_modified_at(client, bucket, "cache.parquet", prefix=p)
            for p in PREFIXES
        }
        mtime_key = "|".join(
            f"{p}:{mt.isoformat() if mt else 'none'}" for p, mt in mtimes.items()
        )
        if mtime_key != st.session_state.get("last_cache_mtime"):
            # 1. Download each machine's cache.parquet to a distinct local file and
            #    merge into config.CACHE_PATH (with a `machine` column per row).
            prefix_paths = supabase_sync.download_cache_per_prefix(
                client, bucket, PREFIXES, target_dir=DATA_DIR
            )
            cache.merge_cache_parquets(prefix_paths, config.CACHE_PATH)
            # 2. caps.json + calibration_log are account-wide — take them from the
            #    canonical (poller) prefix only. Best-effort: a missing caps file
            #    must not block the merged cache view (the viewer has fallback caps).
            try:
                supabase_sync.download_files(
                    client, bucket, CAPS_FILE_NAMES, target_dir=DATA_DIR, prefix=CAPS_PREFIX
                )
            except Exception as caps_err:
                st.warning(f"Caps/calibration unavailable from '{CAPS_PREFIX}': {caps_err}")
            st.session_state["last_cache_mtime"] = mtime_key
            load_data.clear()
        # Live-panel age = newest activity across all machines.
        newest = max((mt for mt in mtimes.values() if mt is not None), default=None)
        seconds_old = (
            (datetime.now(tz=timezone.utc) - newest).total_seconds() if newest else None
        )
        render.render_live_panel_from_cache(agent_seconds_old=seconds_old)
    except Exception as e:
        st.error(f"Could not fetch latest from Supabase: {e}")
```

- [ ] **Step 3: Update the data-flows caption (cosmetic accuracy)**

Find:

```python
st.caption("Read-only cloud view · refreshes every 5 min · data flows from your Windows agent → Supabase → here.")
```

Replace with:

```python
st.caption("Read-only cloud view · refreshes every 5 min · data flows from your machines → Supabase → here.")
```

- [ ] **Step 4: Verify imports are satisfied**

Confirm `app_cloud.py` already imports `cache` and `config` at the top (it does — lines 17 and 21
in the current file). No new imports are needed: `download_cache_per_prefix` and
`merge_cache_parquets` are accessed via the already-imported `supabase_sync` and `cache` modules.

- [ ] **Step 5: Syntax + import smoke check**

Run (from companion repo root, venv active):
`python -c "import ast; ast.parse(open('app_cloud.py').read()); print('app_cloud.py parses')"`
Expected: prints `app_cloud.py parses` (no SyntaxError).

> A full `import app_cloud` would execute Streamlit page setup and try to read secrets, so the AST
> parse is the right non-interactive check here. The behavioral check is the manual run in Task 5.

- [ ] **Step 6: Commit (in the companion repo)**

```bash
git add app_cloud.py
git commit -m "feat(cloud): merge multiple machine prefixes into one view"
```

---

### Task 4: Full test run + Streamlit secrets/env documentation

**Files:**
- Modify: `README.md` (or wherever the companion repo documents Streamlit Cloud secrets) — OPTIONAL,
  see note.

**Why:** Confirm nothing regressed, and record the new env vars the Streamlit Cloud deployment needs.

- [ ] **Step 1: Run the whole test suite**

Run: `python -m pytest -q`
Expected: all tests pass (the new cache-merge + sync tests plus the pre-existing
`test_metrics`, `test_calibration_characterization`, `test_session_cost_attribution`,
`test_supabase_sync` suites).

- [ ] **Step 2: Record the new deployment env vars**

The Streamlit Community Cloud app needs these set (App → Settings → Secrets, or environment). Add a
short note to the companion repo's README (or wherever deployment is documented). The exact values
for borgi's deployment:

```
CLOUD_USER_PREFIXES = "borgi,borgi-linux"
CLOUD_CAPS_PREFIX   = "borgi-linux"
```

- `CLOUD_USER_PREFIXES` — comma-separated machine prefixes to merge. The legacy single
  `CLOUD_USER_PREFIX` still works as a fallback when `CLOUD_USER_PREFIXES` is unset (so the app keeps
  working if you forget to set the new var — it just shows one machine).
- `CLOUD_CAPS_PREFIX` — which prefix supplies the account-wide caps/calibration. Set to the
  always-on poller (`borgi-linux`). Defaults to the first entry of `CLOUD_USER_PREFIXES` if unset.

> If the companion repo has no deployment doc, skip the README edit — the values above are recorded
> in this plan and in the `claude-usage-tray` design spec (§6). This task's hard requirement is just
> Step 1 (tests green) + setting the secrets in the Streamlit dashboard.

- [ ] **Step 3: Commit any doc change (in the companion repo, if you made one)**

```bash
git add README.md
git commit -m "docs: CLOUD_USER_PREFIXES / CLOUD_CAPS_PREFIX for multi-machine merge"
```

---

### Task 5: Manual end-to-end verification

**Why:** The merge logic is unit-tested, but the live Supabase→merge→render path can only be
confirmed against real data. No code change — this is the acceptance gate.

- [ ] **Step 1: Prerequisite**

Phase 1's Linux collector has run at least once, so `borgi-linux/cache.parquet` exists in Supabase
(alongside the existing `borgi/cache.parquet` from Windows). Confirm both objects exist in the
Supabase Storage dashboard.

- [ ] **Step 2: Set the secrets and reboot the app**

In Streamlit Community Cloud, set `CLOUD_USER_PREFIXES="borgi,borgi-linux"` and
`CLOUD_CAPS_PREFIX="borgi-linux"`, then reboot the app (or wait for the next 5-min refresh).

- [ ] **Step 3: Verify the merged view**

Open the app on phone/laptop. Confirm:
- The sessions table / charts now include rows from BOTH machines (e.g. Linux `project_cwd` values
  like `/home/...` appear alongside Windows `C:\...`).
- The live caps panel still renders (caps came from `borgi-linux`).
- No error banner.

- [ ] **Step 4 (optional sanity): confirm row union locally**

If you want a quick offline check before deploying, point a local run at the two prefixes:
```bash
# with the venv active and Supabase env vars set for download
python -c "import polars as pl; df = pl.read_parquet('/tmp/usage-tracker/cache.parquet'); print(df['machine'].value_counts())"
```
Expected: both `borgi` and `borgi-linux` appear with non-zero counts.

---

## Self-review (done by the plan author against the spec)

- **Spec §6.1 (resolve a list of prefixes):** Task 3 Step 1 — `CLOUD_USER_PREFIXES`. ✅
- **Spec §6.2 (per-prefix download + `machine` stamp + concat):** Tasks 1+2, wired in Task 3. ✅
- **Spec §6.3 (caps/calibration from canonical prefix, never summed):** Task 3 — `CAPS_PREFIX`,
  caps downloaded from one prefix only. ✅
- **Spec §6.1-bis (no parquet schema change):** the `machine` column is added at read time in the
  viewer; neither Rust writer changes. ✅
- **Spec safety (no clobber):** read-only viewer change; no upload path touched. ✅
- **Backward compatibility:** legacy `CLOUD_USER_PREFIX` still honored as a fallback; default
  `CAPS_PREFIX` = first prefix. ✅
- **Placeholder scan:** none. **Type consistency:** `merge_cache_parquets(dict[str, Path], Path)`
  defined in Task 1 and called identically in Task 3; `download_cache_per_prefix(...) -> dict[str, Path]`
  defined in Task 2 and consumed in Task 3. ✅

## Out of scope for Phase 2 (handled elsewhere)

- **Phase 3** (Windows stops polling, reads caps from `borgi-linux/caps.json`) — separate plan in
  the `claude-usage-tray` repo.
- **A per-machine UI filter** in the viewer sidebar — the `machine` column makes this trivial later,
  but it is not required for the merged view and is deliberately deferred (YAGNI).
- **Calibration-log continuity across the Phase-3 handover** (concat both prefixes' calibration
  logs) — optional future enhancement noted in the design spec §6.3; not needed while both machines
  still poll.
