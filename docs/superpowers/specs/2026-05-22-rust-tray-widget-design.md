# Claude Usage Tray — Native Windows Rust Widget

## Goal

Replace the Python/Streamlit local agent in [claude-usage-tracker](https://github.com/borgi-s/claude-usage-tracker) with a native Windows application written in Rust. The application lives in the system tray, displays current 5h/7d Claude Code usage at a glance, and opens a native egui dashboard window on click. Primary motivations: a CV/portfolio piece demonstrating Rust + Win32 + GUI work, and a personal shipping target.

## Non-goals

- ❌ Mac/Linux ports — Windows-only is the entire scope.
- ❌ Auto-update / self-replacement — notify only; user downloads from GitHub Releases.
- ❌ Cloud viewer port — `app_cloud.py` stays in Python+Streamlit, deployed on Streamlit Community Cloud. The Rust agent uploads the same files to Supabase, the cloud viewer reads them unchanged.
- ❌ User accounts / multi-tenant — single user, single machine.
- ❌ Telemetry, crash reporting.
- ❌ Non-OAuth Anthropic auth (no manual API-key input flow). Reads `~/.claude/.credentials.json` only.

## Architecture

### Project layout

Separate GitHub repo: `github.com/borgi-s/claude-usage-tray`. Single binary Rust crate, compiled with `#![windows_subsystem = "windows"]` so launching from the tray doesn't open a console window.

```
src/
  main.rs          — entry point, init, top-level message loop
  config.rs        — local TZ, weekly reset constants, cost weights (mirrors Python config.py)
  api/             — Anthropic OAuth + /api/oauth/usage client
  data/            — JSONL parsing of ~/.claude/projects/, in-memory model, on-disk cache
  calibration/     — port of caps.global_cap_from_anchors (output_tokens based, 5h + weekly)
  tray/            — Win32 tray icon (Shell_NotifyIconW), tooltip, right-click menu, GDI icon rendering
  dashboard/       — egui window: chart panels, KPIs, sessions table (Stage 6+)
  sync/            — Supabase upload (Stage 7+)
  updater/         — GitHub Releases API version check (Stage 6.5)
tests/
  fixtures/        — anonymized sample JSONL + caps.json for calibration tests
```

### Dependencies

| Crate | Purpose | First used in stage |
|---|---|---|
| `ureq` | Blocking HTTP client | 1 |
| `serde` + `serde_json` | JSON parse/serialize | 1 |
| `dirs` | Cross-platform home dir lookup | 1 |
| `chrono` + `chrono-tz` | Timestamps + local TZ handling | 1 |
| `anyhow` | Top-level error ergonomics | 1 |
| `thiserror` | Library-internal error enums | 2 |
| `tracing` + `tracing-subscriber` | Structured logging | 2 |
| `windows` | Win32 APIs (Shell, GDI, User) | 3 |
| `semver` | Version comparison for update notifier | 6.5 |
| `eframe` + `egui` | Native window framework | 6 |
| `egui_plot` | Charts inside egui | 6 |
| `dotenvy` | Read .env for Supabase keys | 7 |

### Excluded by design

- ❌ Async runtime (`tokio`) — blocking is fine for 1 RPS polling; simpler for a Rust beginner.
- ❌ Polars equivalent — `Vec<Turn>` + manual filtering is enough.
- ❌ Parquet — bincode or JSON for the cache. Smaller deps, no native libs.
- ❌ Alternative GUI libs (`iced`, `Slint`) — `egui` chosen for ease of learning and richest docs.
- ❌ Cross-compilation — Windows x86_64 MSVC only.

## Stage roadmap

Each stage is a tagged GitHub release, shippable on its own. Beginner-pace estimates assume evenings/weekends.

### Stage 1 — "Hello Anthropic" CLI *(~2-3 weeks)*
- `cargo init`, hello world, README, LICENSE (MIT).
- Implement `api::credentials` — read `~/.claude/.credentials.json`, parse OAuth token.
- Implement `api::usage::fetch()` — hit `/api/oauth/usage`, parse JSON into a `UsageSnapshot` struct.
- Binary: `claude-usage --once` prints `5h: 56% (resets in 2h13m) · 7d: 56% (resets in 1d 21h)`.
- **Learned:** cargo, modules, ownership, serde derive, file I/O, ureq, `Result`/`?`, anyhow.
- **Deliverable:** Single-shot CLI. Tag `v0.1.0`.

### Stage 2 — Polling daemon *(~1-2 weeks)*
- `--watch` flag — loop forever, poll every 60s, print updates.
- Handle `RateLimited` (HTTP 429) gracefully with cached last-known state.
- Add `tracing` for structured logs.
- Persist samples to `~/.claude-usage-tray/calibration_log.jsonl` (append-only JSON-lines — one record per line). See [`2026-05-22-stage-2-watch-design.md`](2026-05-22-stage-2-watch-design.md) for schema and rationale.
- **Learned:** loops, `std::thread::sleep`, custom error types, file append, tracing.
- **Deliverable:** Long-running CLI. Tag `v0.2.0`.

### Stage 3 — Tray icon (basic) *(~3-4 weeks)*
- Add `windows` crate.
- Hide console (`#![windows_subsystem = "windows"]`).
- Create tray icon (`Shell_NotifyIconW` with `NIM_ADD`).
- Solid color icon (green / yellow / red by 5h util).
- Update tooltip every 60s with latest util.
- Right-click menu: "Quit".
- Polling loop on background thread; communicate to UI thread via `mpsc::channel`.
- **Learned:** `windows` crate FFI, Win32 message loop, threads + channels, `unsafe` blocks.
- **Deliverable:** Tray app v1. Tag `v0.3.0`. **First impressive demo.**

### Stage 4 — Rendered percentage icon *(~2 weeks)*
- Use GDI to draw `"56"` text into a 16×16 `HICON`.
- Re-render icon each poll.
- Color background by util threshold.
- **Learned:** GDI bitmap/HDC, Win32 graphics primitives.
- **Deliverable:** Glanceable icon. Tag `v0.4.0`.

### Stage 5 — Calibration math + cache *(~3-4 weeks)*
- Port `parser.py` → `data::parser` — walk `~/.claude/projects/`, parse JSONL into `Vec<Turn>`.
- Implement mtime-diff incremental parse (mirrors `cache.py`).
- Port `caps.global_cap_from_anchors` → `calibration::compute_cap` (output_tokens, 5h + weekly variants; weekly window is since Sun 07:00 local, NOT rolling).
- Persist parsed cache to disk (bincode).
- Tray tooltip shows util % against locally-calibrated cap (not just API util).
- **Learned:** date/time math, file walking, larger Rust modules, performance.
- **Deliverable:** Self-calibrating tray. Tag `v0.5.0`.

### Stage 6 — Dashboard window (egui) *(~4-6 weeks)*
- Add `eframe` + `egui` + `egui_plot`.
- Left-click tray icon → open window (or focus existing).
- 5h chart: cumulative output share, calendar bands (weekends/nights), Pro + Max5x cap lines.
- Weekly chart: same shape, since-Sun-07:00-local reset.
- Daily bar chart.
- KPI strip: peak 5h, peak weekly, total burn, daily avg.
- Close button = hide window (tray stays alive); Quit only from tray menu.
- **Learned:** egui immediate-mode paradigm, chart rendering, window lifecycle.
- **Deliverable:** Real native dashboard. Tag `v0.6.0`. **CV centerpiece.**

### Stage 6.5 — Update notifier *(~1 week)*
- Query `GET https://api.github.com/repos/borgi-s/claude-usage-tray/releases/latest` once per day or on app start.
- Compare returned `tag_name` against `env!("CARGO_PKG_VERSION")` using the `semver` crate.
- If newer: prepend "Update available — v1.2.3" to the tray right-click menu; click → opens release page in default browser.
- Persist last-check timestamp to `~/.claude-usage-tray/state.json` to avoid GitHub rate-limit (60 req/hr unauthenticated).
- No auto-download, no self-replacement — explicitly out of scope.
- **Learned:** GitHub API, semver comparison, simple persistent state.
- **Deliverable:** App notifies the user of new builds. Tag `v0.6.5`.

### Stage 7 — Supabase sync *(~2-3 weeks)*
- Implement multipart PUT to Supabase Storage (`PUT /storage/v1/object/{bucket}/{name}`).
- Read service-role key from `.env` via `dotenvy`.
- Upload cache + caps + calibration_log on each polling tick.
- Existing Python cloud viewer (`app_cloud.py`) reads the files unchanged — zero changes there.
- After this stage, Python local agent (`app.py`) can be retired from your machine.
- **Learned:** HTTP PUT, multipart upload, env config.
- **Deliverable:** Cloud-syncing Rust agent. Tag `v0.7.0`.

### Stage 8 — Streamlit feature parity *(~2-4 months, ongoing)*
- Session table panel (egui table widget).
- Filter sidebar: date range, project, model multiselect.
- Calibration history expander (per-hour scatter, fitted curve overlay).
- Live API status banner (matches the current "Updated Ns ago · sub `pro` tier ..." line).
- Settings panel: cost weights override, weekly reset config.
- Each is its own mini-project — ship one per week.
- **Deliverable:** Full Streamlit replacement on Windows. Tag `v1.0.0`.

## Data flow

```
~/.claude/.credentials.json ──> api::credentials ──> Bearer token
                                                          │
                                                          ▼
                                              Anthropic /api/oauth/usage
                                                          │
                                                          ▼
                                                  UsageSnapshot
                                                          │
~/.claude/projects/*.jsonl ──> data::parser ──> Vec<Turn> ─┤
                                                          │
                                                          ▼
                                        calibration::compute_cap
                                                          │
                                                          ▼
                                              effective_cap_5h, effective_cap_week
                                                          │
            ┌─────────────────────────────────────────────┼─────────────────────────┐
            ▼                                             ▼                         ▼
      tray::update_icon                          dashboard::render             sync::upload
      (tooltip, color, %)                       (egui charts, KPIs)        (Supabase Storage)
```

## Cross-cutting concerns

### Testing
- `cargo test` for pure functions: calibration math, JSONL parsing, version comparison, OAuth credential parsing, UTC ↔ local TZ conversion.
- **No** unit tests for Win32 tray code or egui UI — both are visual; smoke-test by running.
- `tests/fixtures/` holds anonymized sample JSONL + a canonical `caps.json` for calibration regressions.

### Error handling
- Top-level `main` returns `anyhow::Result<()>` — errors logged via `tracing`, app exits cleanly with non-zero code.
- Library modules expose `thiserror`-derived enums so callers can pattern-match (idiomatic Rust; good code-review optics).
- Background polling thread: errors → `tracing::warn!`, never panic. Last-known cached state is what tooltip shows during outages.

### Logging
- `tracing` to a rolling file at `~/.claude-usage-tray/logs/app.log`, size-capped, keep last 5 files.
- `--log-level debug` CLI flag for troubleshooting.
- `tracing-subscriber` with a JSON formatter — greppable, future-proof for metrics.

### Distribution & release flow
- [`cargo-dist`](https://opensource.axo.dev/cargo-dist/) for auto-generated GitHub Releases on tag push.
- Initial releases: standalone `.exe`. WiX MSI installer can be added later.
- No code signing initially (~$200/yr; defer until users complain about Windows SmartScreen).
- Versioning: semver. `0.x` through Stage 7; `v1.0.0` at Stage 8 completion.

### CI
- GitHub Actions: `cargo fmt --check`, `cargo clippy -- -D warnings`, `cargo test`.
- Windows runner, stable Rust only. No matrix testing.

### Repo hygiene
- README with screenshots and animated GIFs of the tray + dashboard.
- LICENSE: MIT.
- CONTRIBUTING.md (even minimal — signals seriousness).
- CHANGELOG.md (keepachangelog.com format).
- GitHub issue templates: bug report, feature request.
- Build/test badge from GitHub Actions in README.

### Config file
- `~/.claude-usage-tray/config.toml` for user overrides: poll interval, weekly reset day/hour, cost weights (when exposed).
- Read on startup, ignore-if-missing, sensible defaults baked in.

## Open questions deferred to implementation

- Exact GDI text rendering approach for Stage 4 — possibly use a small bitmap font baked in, or DirectWrite. Decide in stage.
- Whether to bundle `tracing-appender` for log rotation or roll our own. Decide in Stage 2.
- Whether the dashboard window remembers position/size between launches (probably yes via state.json — decide in Stage 6).
