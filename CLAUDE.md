# claude-usage-tray — Project Memory

## What this project is

Native Windows tray widget for monitoring Claude Code token usage. Eventually replaces the Python+Streamlit local agent in [`claude-usage-tracker`](https://github.com/borgi-s/claude-usage-tracker) on Windows. The cloud viewer (`app_cloud.py` in that repo) stays Python — only the local agent gets ported.

**Primary motivation:** CV/portfolio piece. Native Rust + Win32 + GUI work is a rare combo and signals systems-programming skills. Secondary motivation: shippable .exe with no Python runtime requirement.

## User context

User is a **Rust beginner** (zero prior Rust experience as of 2026-05-22). Explain ownership/borrowing/lifetimes/idioms as they come up. Don't assume familiarity with cargo, modules, serde, Result/?, anyhow, etc. The Stage 1 plan has inline beginner notes — future stages should keep that style.

## Active design + plans

- **Design spec:** `docs/superpowers/specs/2026-05-22-rust-tray-widget-design.md` — full 8-stage roadmap, dependency rationale, scope cuts, non-goals. Read first.
- **Stage 1 plan:** `docs/superpowers/plans/2026-05-22-stage-1-cli.md` — single-shot CLI. **Shipped 2026-05-22 (tag `v0.1.0`).**
- **Stage 2 spec:** `docs/superpowers/specs/2026-05-22-stage-2-watch-design.md` — polling daemon design details (CLI, calibration log schema, render approach).
- **Stage 2 plan:** `docs/superpowers/plans/2026-05-22-stage-2-watch.md` — bite-sized task plan for `--watch`. **Shipped 2026-05-22 (tag `v0.2.0`).**
- **Stage 3 spec:** `docs/superpowers/specs/2026-05-22-stage-3-tray-design.md` — Win32 tray icon design details.
- **Stage 3 plan:** `docs/superpowers/plans/2026-05-22-stage-3-tray.md` — task plan. **Shipped 2026-05-22 (tag `v0.3.0`).**
- **Stage 4 spec:** `docs/superpowers/specs/2026-05-23-stage-4-gdi-icon-design.md` — GDI+ rendered percentage icon design details.
- **Stage 4 plan:** `docs/superpowers/plans/2026-05-23-stage-4-gdi-icon.md` — bite-sized task plan, **executed locally; pending final tag and push.**

Stages 5-8 will get their own implementation plans when each is ready to start.

## Stage roadmap (summary — see spec for details)

| Stage | Deliverable | Status |
|---|---|---|
| 1 | Single-shot CLI: read OAuth, hit `/api/oauth/usage`, print util | ✅ Shipped — tag `v0.1.0`, pushed to GitHub |
| 2 | Polling daemon (`--watch`) | ✅ Shipped — tag `v0.2.0`, pushed to GitHub |
| 3 | Win32 tray icon (basic, solid color) | ✅ Shipped — tag `v0.3.0`, pushed to GitHub |
| 4 | GDI-rendered percentage icon | ✅ Built locally — pending tag/push |
| 5 | Calibration math (port from Python's `caps.global_cap_from_anchors`) | Pending |
| 6 | egui dashboard window | Pending |
| 6.5 | Update notifier (GitHub Releases API) | Pending |
| 7 | Supabase Storage upload | Pending |
| 8 | Streamlit feature parity (sessions table, filters, calibration history) | Pending |

## Tech stack (locked in design)

- Rust stable, Windows MSVC toolchain, x86_64 only
- HTTP: `ureq` (blocking, no async)
- JSON: `serde` + `serde_json`
- Time: `chrono` + `chrono-tz`
- Paths: `dirs`
- Errors: `anyhow` (top-level) + `thiserror` (library modules)
- Logging: `tracing` + `tracing-subscriber` (added Stage 2)
- Win32: `windows` crate (added Stage 3)
- GUI: `eframe` + `egui` + `egui_plot` (added Stage 6)

**Excluded by design:** tokio, polars equivalents, parquet, iced/Slint/other GUI libs, cross-compilation, Mac/Linux ports.

## Conventions

- All timestamps UTC internally. Local TZ via `chrono-tz` when displaying.
- No commits with `Co-Authored-By: Claude` or "Generated with Claude Code" attribution.
- Commit style: conventional (`feat:`, `fix:`, `style:`, `chore:`). Match what the plan's commit messages use.
- `cargo fmt` + `cargo clippy --all-targets -- -D warnings` clean before each release tag.

## Known quirks (discovered during Stage 1, defer to a later stage)

- **`subscriptionType` field misreports Max plan as `"pro"`.** Confirmed 2026-05-22 against borgi's real Max account — the JSON returned by `~/.claude/.credentials.json` says `"subscriptionType": "pro"`. Whatever Anthropic populates that field with doesn't distinguish Pro from Max. Stage 5 (calibration) will need a different signal to detect Max — possibly the `rateLimitTier` field, or hardcoded inference from observed util_5h ceilings. Don't trust `Credentials.subscription_type` for plan-tier decisions.
- **`/api/oauth/usage` rate-limits to ~1 request/minute per token.** Two back-to-back fetches return HTTP 429. The Stage 2 polling daemon's minimum poll interval must respect this. Details in [the corresponding memory file](~/.claude/projects/C--Users-borgi-Documents-claude-usage-tray/memory/reference_usage_endpoint_ratelimit.md).

## Companion project (separate repo, separate concern)

`C:\Users\borgi\Documents\claude-usage-tracker\` is the Python+Streamlit project this widget will eventually displace on the local Windows machine. Live cloud viewer at https://borgi-claude-usage-tracker.streamlit.app/. Don't edit files in that repo from this session; they're separate codebases.

Key facts about that project useful to know:
- Anthropic's util_5h is metered on **output tokens**, not cost-weighted (verified empirically 2026-05-22).
- Anthropic's util_7d is **fixed weekly window** that resets Sunday 07:00 local — NOT rolling 7d despite the field name.
- These two facts MUST carry over into the Rust calibration math (Stage 5). The Python `caps.py` is the reference implementation.

## Cold-start checklist

When resuming work in this repo:

1. Read this file, then the spec at `docs/superpowers/specs/2026-05-22-rust-tray-widget-design.md`.
2. Check `~/.claude/projects/C--Users-borgi-Documents-claude-usage-tray/memory/MEMORY.md` for session-level memory pointers.
3. Verify state: `cargo --version` works? Repo initialized? What stage tag is on HEAD (`git describe --tags`)?
4. Pick the right stage plan in `docs/superpowers/plans/`. Stages execute sequentially — finish one before planning the next.
5. The Stage 1 plan has inline notes on Rust idioms for a beginner; keep that style for Stages 2-8 plans when written.
