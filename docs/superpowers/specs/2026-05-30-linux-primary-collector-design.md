# Linux Primary Collector + Multi-Machine Merge — Design

**Date:** 2026-05-30
**Status:** Approved (brainstorm complete; plan to follow)
**Scope:** Two repos — `claude-usage-tray` (this repo) and `claude-usage-tracker` (companion Python viewer, sign-off-gated).

> This design was produced from a multi-agent investigation workflow (5 investigators + an
> adversarial critic) plus direct verification of the load-bearing facts against the real code.
> Where the workflow's "Rust headless" investigator made claims that contradicted the actual
> repo (it mistakenly thought the source was a scaffold, that `main.rs` was a 13-line shim, and
> that the `windows` crate was already `cfg(windows)`-gated), the design uses the **verified**
> facts: the data/sync/api modules are real, tested code; `main.rs` is heavily Win32-entangled;
> and the `windows`/`eframe`/`egui*`/`winit` crates are **unconditional** dependencies today.

---

## 1. Goal

Run the data collection/tracking on a headless **Ubuntu 24.04 server** (where the user runs
Claude Code via `tmux new -s claude1`), in addition to the existing Windows machine, and have
the combined data visible from phone/laptop through the existing Streamlit cloud viewer
(`borgi-claude-usage-tracker.streamlit.app`), **merged into one view**.

## 2. Key decisions (made during brainstorming)

| Decision | Choice | Why |
|---|---|---|
| Same vs separate Anthropic account | **Same account** | Account-wide 5h/7d caps are identical on both machines; the genuinely-new data from each box is its **local session turns**. |
| Collector implementation | **Rust** (reuse this repo) | Matches "clone this project", portfolio value, one codebase/one schema. The collector core is already platform-agnostic. |
| Combined view | **True merge** | Viewer reads both machines' turns and concatenates them with a `machine` stamp. Requires a sign-off-gated edit to the companion repo. |
| **Who polls the live API** | **Linux server (always-on)**; **Windows stops polling** | The server is always on, so its caps/calibration time series has **no gaps**; its token stays fresh because Claude Code runs there; calibration anchors fit a continuous series better. |
| Windows live % source | **Read `caps.json` from Supabase** | The tray icon / widget / live banner keep working, sourced from the cloud copy Linux keeps fresh. |

### 2.1 The constraint that drives the role swap

`/api/oauth/usage` is rate-limited to **~1 request/minute per account** (two near-simultaneous
fetches return HTTP 429 — confirmed in `CLAUDE.md` known-quirks and `src/api/usage.rs`). With
the **same account** on both machines, only **one** machine can be the poller; a second poller
would intermittently grab the once-per-minute slot and starve the primary, re-introducing the
very gaps we are eliminating. Therefore: **Linux is the sole poller; Windows must stop hitting
the API.**

### 2.2 Why turning Windows into a non-poller is safe for its UX

Windows today polls for two purposes: (a) to drive its own tray icon / widget / live banner, and
(b) to upload account-wide `caps.json` + `calibration_log.parquet`. After the swap, Linux owns
(b), and Windows satisfies (a) by **downloading** `borgi-linux/caps.json` from Supabase instead
of calling the API. The downstream snapshot consumers (tray icon, widget, live banner) are
unchanged — only the snapshot's *source* changes from an API fetch to a cloud read.

## 3. Architecture

```
LINUX SERVER (always-on)  ── PRIMARY ────────────┐
  ~/.claude/projects/**/*.jsonl  → local turns    │ uploads to  borgi-linux/
  ~/.claude/.credentials.json    → OAuth token     │   cache.parquet           (Linux turns)
  /api/oauth/usage  (SOLE poller) → caps + calib    │   caps.json               (canonical, live)
                                                   │   calibration_log.parquet (canonical, gap-free)
                                                   ▼
                                         SUPABASE STORAGE (bucket: usage-tracker)
                                                   ▲                         │
WINDOWS LAPTOP (intermittent) ── SECONDARY ────────┤ uploads borgi/cache.parquet (Windows turns)
  ~/.claude/projects/**/*.jsonl → local turns      │ DOWNLOADS borgi-linux/caps.json → tray %/widget
  (NO API poll)                                    │
                                                   ▼
                                  STREAMLIT VIEWER (companion repo, sign-off-gated)
                                    reads caps/calibration from borgi-linux (gap-free),
                                    concats cache.parquet from BOTH prefixes + machine=<prefix>
                                    → ONE merged Windows+Linux view on phone/laptop
```

### 3.1 Storage layout (Supabase Storage, bucket `usage-tracker`)

| Object | Producer | Notes |
|---|---|---|
| `borgi/cache.parquet` | Windows | Windows local turns (unchanged key; Windows keeps its existing prefix). |
| `borgi/caps.json`, `borgi/calibration_log.parquet` | Windows (legacy) | Become **stale** after switchover. Viewer ignores them for live data; may optionally concat the old `calibration_log` for pre-switchover history (see §6.3). |
| `borgi-linux/cache.parquet` | Linux | Linux local turns. |
| `borgi-linux/caps.json` | Linux | **Canonical** live caps (account-wide). |
| `borgi-linux/calibration_log.parquet` | Linux | **Canonical** calibration series (gap-free going forward). |

Prefixes are disjoint, so the `x-upsert: true` overwrite each machine performs only ever touches
its **own** keys — nothing is clobbered (verified: `src/sync/storage.rs`, `src/sync/mod.rs` build
the key as `{prefix}/{name}`, and `src/sync/config.rs` validates the prefix to a single slash-free
path segment).

## 4. Component A — Linux collector + build portability (`claude-usage-tray`)

Verified coupling map (authoritative grep, not the workflow's): **all** Win32/GUI code lives in
`src/tray/*`, `src/dashboard/*`, `src/autostart.rs`, and `src/main.rs`. Every collector-relevant
module — `src/data/*`, `src/sync/*`, `src/calibration/*`, `src/api/*`, `src/poll.rs`,
`src/config.rs`, `src/paths.rs`, `src/settings.rs`, `src/shared/*` — is already free of
`windows`/`eframe`/`egui`. So this is **packaging work, not a rewrite**.

### 4.1 `Cargo.toml`

- Move the GUI/Win32 dependency stack — `windows`, `eframe`, `egui`, `egui_plot`, `egui_extras`,
  `winit` — under `[target.'cfg(windows)'.dependencies]`. They are inherently Windows-only in this
  project, so target-gating (rather than a feature flag) is the simplest correct lever.
- Add a second binary target:
  ```toml
  [[bin]]
  name = "collector"
  path = "src/bin/collector.rs"
  ```

> **Rust note (target-gated deps):** `[target.'cfg(windows)'.dependencies]` means "only pull these
> crates when compiling for Windows." On `x86_64-unknown-linux-gnu` Cargo never downloads or
> compiles them, so the heavy egui/glow/winit/Win32 stack simply isn't part of the Linux build.

### 4.2 `src/lib.rs`

- `#[cfg(windows)] pub mod tray;`
- `#[cfg(windows)] pub mod dashboard;`
- `#[cfg(windows)] pub mod autostart;`
- Confirm no platform-agnostic module references these three. (`main.rs` does, but it is a binary,
  not part of the library — handled in §4.3.)

> **Rust note (`#[cfg(windows)]`):** a compile-time switch. On Linux these `mod` lines vanish, so
> the library exposes only the portable modules and compiles clean.

### 4.3 `src/main.rs`

`main.rs` is the **Windows GUI binary** (it imports `windows::Win32::Graphics::GdiPlus`, defines a
`GdiplusGuard`, calls `AttachConsole`, and dispatches to the tray). It will not compile on Linux.
Two options; the plan will pick one:

- **(Preferred)** Wrap the existing body in `#[cfg(windows)]` and add a tiny `#[cfg(not(windows))] fn main()`
  stub that prints "GUI build is Windows-only; run the `collector` binary." Keeps `cargo build`
  green on Linux even without `--bin collector`.
- **(Fallback)** Leave `main.rs` as-is and always build the server with `cargo build --bin collector`.

### 4.4 `src/bin/collector.rs` (new, ~40–60 lines) — the **full/primary** loop

Because Linux is primary, the collector does the *whole* job (poll + turns), reusing the
**portable** modules only: `poll::poll_once`, `data::cache::refresh`, `log::calibration`, and
`sync`. It must **not** reuse `tray::poller`'s `compute_calibration_with_turns` helper — that lives
inside the Win32-gated `src/tray/poller.rs` (it imports `windows::…PostMessageW`) and is not
available on Linux. The collector also doesn't need it: the uploaded artifacts require only `turns`
(for `cache.parquet`), the raw calibration-log samples (for `calibration_log.parquet`), and the
poll's `UsageSnapshot` + creds (for `caps.json`). None of those need the derived-caps math. The
collector therefore builds a **minimal** `AppSnapshot { turns, last_sample, ..Default::default() }`
and calls the sync path. Per cycle:

1. **Best-effort poll:** `poll::poll_once()` → fetch usage (Bearer token from
   `~/.claude/.credentials.json`) and append a calibration sample to the server-local
   `~/.claude-usage-tray/calibration_log.jsonl`. On `RateLimited`/`Unauthorized`/network error:
   **log and continue** — do not crash, do not abort the cycle.
2. **Always refresh turns:** `data::cache::refresh()` → `Vec<Turn>` from `~/.claude/projects`.
   This needs no token, so it runs even when the poll failed.
3. **Upload:** build an `AppSnapshot` and call the sync path against prefix `borgi-linux`:
   - always upload `cache.parquet` (turns);
   - upload `caps.json` + `calibration_log.parquet` only when this cycle's poll succeeded (so a
     stale token never overwrites good caps with empty data — leave the last good copies in place).
4. `sleep(interval)`.

Config: `--interval <secs>` (default 120, safely above the ~1/min limit) and `--once` (for a
one-shot test). Supabase config from `.env` (`SUPABASE_USER_PREFIX=borgi-linux`).

> The plan may extract the shared cycle into a reusable `fn run_collector_cycle(...)` so the
> Windows poller and the Linux collector don't duplicate logic — decided at plan time.

## 5. Component B — Windows stops polling, reads cloud caps (`claude-usage-tray`)

1. **Add a Supabase GET** to the storage layer. Today `ObjectStore`/`SupabaseStore` only `put`s;
   add a `get(object_path) -> Result<Vec<u8>, StorageError>` (ureq `GET`, `Authorization: Bearer`
   + `apikey`). A read-only/anon key suffices here if available; otherwise reuse the configured key.
2. **Repoint the Windows poller** (`src/tray/poller.rs`): instead of `api::usage::fetch_usage(...)`,
   download `borgi-linux/caps.json`, parse its `sample_util_5h`/`sample_util_7d`/reset fields into a
   `UsageSnapshot`, and update the shared snapshot's `last_sample` exactly as before. The tray icon,
   taskbar widget, and live banner display **utilization %** (which comes straight from `caps.json`),
   so they need **no** derived-caps math and keep working unchanged. The Windows poller can stop
   computing derived caps entirely (those fed only the dashboard's analytics tabs — see §5.1).
3. **Windows upload becomes cache-only:** Windows continues to `data::cache::refresh()` and upload
   `borgi/cache.parquet`, but no longer uploads `caps.json`/`calibration_log.parquet` (those are
   Linux's job now).
4. **Graceful offline behavior:** if the cloud read fails (Windows offline, or Supabase
   unreachable), the tray shows the last-known value / an "offline" state rather than erroring.

### 5.1 Scope boundary for the Windows native UI

Only the **live** surfaces (tray icon, widget, live banner) are repointed to the cloud in this
work. The Windows native dashboard's **historical** tabs (calibration history, sessions) continue
to read local files and will reflect only this machine's local data; the **merged, account-wide,
cross-machine** experience is delivered by the **Streamlit viewer** (Component C), which is the
user's stated target ("available from my phone or laptop using the streamlit app"). Wiring the
Windows native dashboard's historical tabs to the cloud is an explicit **non-goal** here (§9).

## 6. Component C — Streamlit viewer merge (`claude-usage-tracker`, SIGN-OFF-GATED)

> The companion repo is off-limits to edit from the implementation session per `CLAUDE.md`. This
> change will be delivered as a **diff/PR for the user to apply** in that repo. Without it there is
> **no merged view** — verified: `app_cloud.py` reads exactly one prefix (`CLOUD_USER_PREFIX`) and
> a single `cache.parquet`, with no concatenation anywhere.

Changes:
1. Resolve a **list** of prefixes (e.g. comma-separated `CLOUD_USER_PREFIXES="borgi,borgi-linux"`;
   dynamic folder listing is a possible later enhancement but explicit list is safer for v1).
2. For each prefix, download `{prefix}/cache.parquet`, add a `machine = <prefix>` column, and
   concatenate. Stamping the column at read time means **no parquet schema change** and sidesteps
   polars' `vertical_relaxed` "won't fill missing columns" trap; it also yields a free per-machine
   filter/label in the UI.
3. Take `caps.json` + `calibration_log.parquet` from the **designated canonical prefix**
   (`borgi-linux`, the always-on poller) — never summed (they are account-wide, so summing would be
   wrong). Tolerate a prefix that has only `cache.parquet` (the Linux prefix always has all three;
   the Windows prefix's caps/calib are ignored for live data).

### 6.1 No host column in the parquet
A self-describing `machine`/`host` column is **deliberately not added** to `cache.parquet`. It
would be a breaking, coordinated cross-repo schema change requiring both writers to add it in the
same release; the viewer-side prefix stamp achieves the same outcome with zero writer change.

### 6.2 Cosmetic note
Merged sessions will show Windows `C:\...` and Linux `/home/...` `project_cwd` paths verbatim. The
`machine` column labels them, so this is acceptable.

### 6.3 Calibration continuity across the handover (optional)
Linux's `calibration_log` begins at deployment. The pre-switchover history is frozen in
`borgi/calibration_log.parquet`. The viewer **may** concat both calibration logs (dedup by
`sampled_at`) to preserve a continuous timeline across the handover. Nice-to-have; the plan will
decide whether to include it in the first viewer change.

## 7. Safety (the four ranked priorities)

1. **Don't leak secrets.** `.env` is already gitignored (`.gitignore:12`) and `service_role_key`
   is `[redacted]` in `SyncConfig`'s `Debug`. On the server: store `.env` `chmod 600`, owned by the
   run user, in the systemd unit's `WorkingDirectory` (dotenvy loads `./.env` from the process CWD).
   **Prefer a Storage-scoped Supabase key over `service_role`** (which bypasses RLS and could delete
   *all* prefixes including Windows's). Disable the push remote on the server clone as defense in
   depth; never `git add` a key file.
2. **Don't clobber Supabase data.** Distinct prefix `borgi-linux` ⇒ disjoint keys; Windows's
   `borgi/*` is untouched. **Author a fresh `.env` on the server** — do not copy the Windows `.env`
   verbatim, or it silently reuses `borgi` and clobbers.
3. **Low-maintenance.** Run the collector as a **systemd `--user` service** with
   `loginctl enable-linger borgi` (survives reboot and logout), `Restart=on-failure`, and an
   internal sleep loop for cadence. This is independent of the `tmux -s claude1` session (a tmux
   pane dies on reboot).
4. **Don't break Windows.** Phased rollout (§8) and single-poller-after-switchover. During the brief
   overlap in Phase 1, occasional 429s are handled gracefully by the existing `RateLimited` path.

### 7.1 Credentials on Linux (resolved)
Confirmed on the server: `~/.claude/.credentials.json` exists, mode `600`, content
`{"claudeAiOauth":{"accessToken":...}}` — the **plaintext-file** case the loader reads directly
(`src/api/credentials.rs`), **no keyring involved**. The token is kept fresh because Claude Code
runs on the server; the repo does **not** auto-refresh, so if the token ever goes stale the poll
errors with an actionable message and that cycle is skipped — the turns upload still proceeds
(§4.4). No interactive login is needed (the server is already authenticated).

## 8. Rollout phasing (so nothing breaks mid-flight)

1. **Phase 1 — Linux collector live.** Build + deploy the collector; it uploads to `borgi-linux/`.
   Windows is untouched (still polls + uploads `borgi/`). Brief two-poller overlap is tolerated.
   *Verify:* `borgi-linux/cache.parquet` + `caps.json` appear in Supabase.
2. **Phase 2 — Viewer merge.** Apply the Component C change. *Verify:* phone/laptop shows the
   gap-free merged Windows+Linux view with a `machine` breakdown.
3. **Phase 3 — Windows switch-over.** Ship Component B: Windows stops polling and reads caps from
   `borgi-linux/caps.json`. Linux is now the sole poller; contention gone. *Verify:* Windows tray %
   still updates (from cloud); no 429s on the server poller.

## 9. Non-goals / scope cuts

- **Cross-compilation.** Build the collector **natively on Ubuntu** (install `rustup` +
  `build-essential`), or build in WSL/a Linux container and copy the binary. Cross-compiling from
  Windows stays excluded per `CLAUDE.md`.
- **Linux GUI.** The collector is headless; no tray/dashboard/widget on Linux.
- **Windows native dashboard reading cloud history.** Out of scope (§5.1); Streamlit is the merged
  viewer.
- **Dynamic prefix discovery in the viewer.** v1 uses an explicit prefix list.
- **A per-row `machine`/`host` column in the parquet.** Use the viewer-side prefix stamp instead.
- **Conflict-resolving merge / read-modify-write to a shared key.** Rejected: unsafe with two
  concurrent writers; the additive concat of disjoint prefixes is correct and simple.

## 10. Testing

- **Compiles headless:** `cargo build --release --bin collector` on `x86_64-unknown-linux-gnu`
  (first real cross-platform build; confirm `ureq` resolves to **rustls** so no system OpenSSL is
  needed — if it resolves to native-tls, install `libssl-dev`/`pkg-config` or pin `ureq` to rustls).
- **Windows unchanged:** existing `cargo test` + `cargo clippy --all-targets -- -D warnings` stay
  green on Windows; the default Windows build is byte-for-byte the same (gated deps/modules are
  exactly today's behavior under `cfg(windows)`).
- **New `get` path:** unit-test `SupabaseStore::get` via the existing fake-`ObjectStore` pattern
  (mirror the `put` tests in `src/sync/mod.rs`).
- **Manual end-to-end:** Phase verifications in §8.

## 11. Risks & open items

- **First Linux build friction** (TLS backend / OpenSSL) — see §10; low and contained.
- **Token staleness during long idle** on the server — mitigated by active Claude Code use +
  graceful skip; turns upload is decoupled from the poll.
- **Operator discipline:** the server `.env` must use a *distinct* prefix; there is no runtime guard
  against prefix reuse.
- **Companion-repo change requires sign-off** before anyone edits `claude-usage-tracker`.

## 12. Affected files (summary)

**`claude-usage-tray` (this repo):**
- `Cargo.toml` — target-gate GUI/Win32 deps; add `[[bin]] collector`.
- `src/lib.rs` — `#[cfg(windows)]`-gate `tray`/`dashboard`/`autostart`.
- `src/main.rs` — `#[cfg(windows)]` body + non-Windows stub.
- `src/bin/collector.rs` — **new** headless primary loop.
- `src/sync/storage.rs` (+ `ObjectStore` trait) — add `get`.
- `src/sync/mod.rs` — add a **cache-only / conditional** upload path (today `Syncer::run_once`
  uploads all three objects unconditionally). Used by Windows always, and by Linux on cycles where
  the poll failed (so a stale token never overwrites good `caps.json`/`calibration_log.parquet`).
- `src/tray/poller.rs` — read caps from cloud instead of polling the API; cache-only upload.
- `.env.example` / docs — server `.env` template + systemd unit + deployment steps.

**`claude-usage-tracker` (companion, sign-off-gated, delivered as a diff):**
- `app_cloud.py` + `supabase_sync.py` (+ `cache.py`) — multi-prefix download, `machine`-stamped
  concat, canonical caps/calibration from `borgi-linux`.
