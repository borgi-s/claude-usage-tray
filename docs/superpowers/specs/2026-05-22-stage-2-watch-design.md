# Stage 2 — Polling Daemon Design

Companion to the top-level [rust-tray-widget-design.md](2026-05-22-rust-tray-widget-design.md). This document fixes the details that the top-level spec left open for Stage 2.

## Goal

Add a `--watch` mode to the existing Stage 1 CLI: a long-running loop that polls Anthropic's `/api/oauth/usage` endpoint on a fixed cadence, renders the result as a single-screen live view in the terminal, and persists each successful sample to disk for Stage 5 calibration to consume later.

## Non-goals (Stage 2)

- ❌ Tray icon, GUI window — Stage 3+.
- ❌ File-based log rotation — `tracing` writes to stderr only; file logging arrives in Stage 3 when the daemon goes background.
- ❌ Config file (`config.toml`) — interval is set via CLI flag with a small fixed set of choices.
- ❌ Custom Ctrl-C / signal handler — default OS termination is fine; nothing in-memory needs cleanup beyond what append-then-flush already gives us.
- ❌ Persisted cached state — on restart the daemon comes up cold and waits for the first poll.

## CLI surface

`clap` v4 with the derive API replaces the hand-parsed `std::env::args()` from Stage 1.

```text
USAGE:
  claude-usage-tray <MODE>

MODES:
  --once                    Single fetch + print, then exit (Stage 1 behavior preserved)
  --watch [--interval S]    Long-running loop with a redraw-in-place live view

OPTIONS:
  --interval <SECS>         Polling interval. One of: 60, 120, 300. Default: 120.
  --log-level <LEVEL>       trace | debug | info | warn | error. Default: info.
  -h, --help
  -V, --version
```

`--interval` is a clap `ValueEnum` with exactly three variants (e.g. `I60 = 60`, `I120 = 120`, `I300 = 300`). No free-form integers, no runtime range validation. Default `120` balances responsiveness against the `/api/oauth/usage` ~1 req/min rate limit — at 60s some users will see 429s in normal operation; 120s gives breathing room. 300s is a "background and forget" cadence for low-impact monitoring.

`--once` and `--watch` are mutually exclusive and at least one must be supplied. Modeled as a clap `ArgGroup` (required = true, multiple = false).

## Module layout (after Stage 2)

```
src/
  main.rs              — parse CLI, install tracing subscriber, dispatch to once or watch
  cli.rs               — #[derive(Parser)] struct + ValueEnum for --interval
  api/                 — unchanged from Stage 1 (credentials, usage)
  watch.rs             — the polling loop + state machine; no I/O of its own beyond api::* and log::*
  render.rs            — pure function: (WatchState, Credentials, IntervalSecs, LastStatus) -> String
                         (string is what gets written to stdout; ANSI cursor escapes included)
  log/
    mod.rs
    calibration.rs     — append a JSONL sample to ~/.claude-usage-tray/calibration_log.jsonl
  paths.rs             — resolve ~/.claude-usage-tray/, lazy-create on first write
```

`render.rs` is split from `watch.rs` so the renderer is a pure function over inputs — unit-testable with a frozen `now` and synthetic snapshots, no need to stand up the loop.

## Polling loop control flow

```text
watch::run(interval) -> anyhow::Result<()>:
  creds = api::credentials::load_from_default_path()?
  state = WatchState { last_success: None, last_status: Initial }
  loop:
    fetch_at = Instant::now()
    match api::usage::fetch_usage(&creds):
      Ok(snap):
        state.last_success = Some((snap.clone(), Utc::now()))
        state.last_status  = Ok
        log::calibration::append(&snap, &creds)
          .unwrap_or_else(|e| tracing::warn!(?e, "calibration log write failed"))
        render_to_stdout(&state, &creds, interval)
      Err(FetchError::RateLimited):
        state.last_status = RateLimited
        tracing::warn!("rate limited; keeping last sample on screen")
        render_to_stdout(&state, &creds, interval)
      Err(other):
        state.last_status = Error(other.to_string())
        tracing::warn!(?other, "poll failed")
        render_to_stdout(&state, &creds, interval)
    sleep_until(fetch_at + interval)   // anchors cadence to start-of-fetch
```

Key properties:
- **First poll is immediate.** The first `render_to_stdout` happens within ~1 second of program start, so the user sees activity without waiting a full interval.
- **Cadence anchors to fetch start, not fetch end.** A slow fetch shortens the next sleep instead of stretching the whole schedule. If a fetch ever exceeds the interval (network stall longer than 120s), the next sleep is zero and the next poll fires immediately — no negative-sleep panic.
- **Calibration log failures do not crash the loop.** Logged and swallowed — disk full or permission issues never take down the watcher.
- **The renderer always sees the latest `last_status`,** so the screen can show a "stale" or "rate-limited" footer next to the previous-good sample during outages.

## Live view (render.rs)

Single-screen, redraw-in-place. ANSI escapes only — no `crossterm` / TUI lib (the dependency footprint isn't worth it for ~5 lines of output).

```text
claude-usage-tray  watching (120s)  press Ctrl-C to quit

  5h:  57%  resets in 2h 12m
  7d:  57%  resets in 1d 21h
  sub: pro / tier: default_claude_ai
  last poll: 14:24:01  next: 14:26:01            [Ok]
```

Every visible line — including the header — is part of the redraw. There is no "print once then redraw the body" split. On each tick `render::draw` returns a `String` containing:
1. A cursor-up escape (`\x1b[<N>A`) where `N` is the number of lines drawn in the previous frame, to move back to the top of that frame.
2. A line-clear escape (`\x1b[2K`) per line in the new frame, to wipe leftover characters from longer previous lines.
3. The new frame text.

For the first frame, `N` is 0 (nothing to overwrite). The watch loop tracks `last_line_count` so subsequent frames know how far up to go.

**Terminal compatibility:** ANSI cursor escapes are supported by Windows Terminal and modern PowerShell (Win10+). Legacy `cmd.exe` may render the escape codes as literal `?[2K` etc. — Stage 2 targets Windows Terminal; cmd.exe is not a supported terminal.

`last_status` footer values:
- `[Ok]` — most recent poll succeeded.
- `[stale 2m · rate-limited]` — sample is from N minutes ago, last poll was 429.
- `[stale 2m · error: network timeout]` — sample is from N minutes ago, last poll was some other error.
- `[fetching…]` — first frame before the first poll completes.

## Cached state on 429 / errors

In-memory only. `WatchState.last_success` is `Option<(UsageSnapshot, DateTime<Utc>)>`. On restart, it's `None`; the screen shows `fetching…` until the first successful poll. Persisting the last sample across restarts adds disk I/O for no real user benefit — first successful poll arrives within seconds of startup.

## Tracing setup

- `tracing-subscriber::fmt()` with the human-readable formatter, writing to **stderr only**.
- Level controlled by `--log-level`; if `RUST_LOG` env var is set it takes precedence (standard subscriber behavior via `EnvFilter`).
- No JSON formatter, no file appender, no rolling logs — those land in Stage 3.
- Stdout is reserved exclusively for the render output. Tracing must never touch stdout or the live view garbles.

## Calibration log

**Path:** `~/.claude-usage-tray/calibration_log.jsonl`
(Spec divergence from the top-level design, which calls it `calibration_log.json` — see "Spec divergence" below.)

**Directory creation:** lazy, on first write. `std::fs::create_dir_all(parent)` is called inside `log::calibration::append` before opening the file, so the directory springs into existence on the first successful poll and not before.

**File open:** `OpenOptions::new().append(true).create(true).open(path)`. One file descriptor per call (no long-lived handle) — keeps the writer stateless and the rest of the process holds no resources between polls.

**Write:** serialize one record with `serde_json::to_string` (no pretty-printing), write the line + `\n`, `file.flush()`. Flushing after each write ensures a crash mid-loop loses at most a partially-written final line, which a JSONL reader will skip.

**Schema (per record):**

```json
{
  "schema_version": 1,
  "ts": "2026-05-22T14:23:01Z",
  "five_hour_util": 0.56,
  "five_hour_resets_at": "2026-05-22T17:00:00Z",
  "seven_day_util": 0.56,
  "seven_day_resets_at": "2026-05-24T05:00:00Z",
  "subscription_type": "pro",
  "rate_limit_tier": "default_claude_ai"
}
```

- `schema_version`: integer literal `1`. Reserved for future format evolution.
- `ts`: UTC, RFC 3339, second-precision. The moment the API response was received.
- `five_hour_util` / `seven_day_util`: 0.0–1.0 floats. May be `null` if the API omitted the bucket.
- `*_resets_at`: RFC 3339 UTC, or `null` if API omitted it.
- `subscription_type` / `rate_limit_tier`: strings from the OAuth credentials.

**Logged ticks:** only successful polls. Rate-limited and error ticks are skipped — Stage 5 detects gaps by absence and doesn't need event records to explain them.

## Error handling

- Top-level `main` returns `anyhow::Result<()>` — unchanged from Stage 1.
- `FetchError` (already defined in Stage 1) is reused. The `RateLimited` variant gets dedicated handling in the watch loop; other variants fall into the generic error case.
- New typed error: `log::calibration::LogError` derived via `thiserror`, with variants:
  - `Io(std::io::Error)` — directory create, file open, write, flush failures.
  - `Serde(serde_json::Error)` — should be unreachable in practice; included for completeness.
- All `LogError` instances are logged via `tracing::warn!` and swallowed; the watch loop never returns a `LogError` upward.

## Graceful shutdown

None. `Ctrl-C` invokes the OS default — process terminates. Justification: the only state is in-memory (`WatchState`) and an append-only log that's flushed per write. Nothing to clean up. Signal-handling lands in Stage 3 when there's a tray icon to remove via `Shell_NotifyIconW(NIM_DELETE)`.

## Testing

| Test file | What it covers | Notes |
|---|---|---|
| `tests/credentials_test.rs` | existing | unchanged from Stage 1 |
| `tests/usage_test.rs` | existing | unchanged from Stage 1 |
| `tests/calibration_log_test.rs` | new | Round-trip: append two `UsageSnapshot`s to a `TempDir`-backed file; read each line back with `serde_json::from_str`; assert field equality. |
| `tests/render_test.rs` | new | Frozen `now` + two synthetic `WatchState`s (post-success, post-429); assert key substrings (`"5h: 57%"`, `"stale"`, footer tags). No snapshot tooling — substring assertions are enough for ~5-line output. |

`watch::run` itself is not unit-tested. Loops + `thread::sleep` + real network calls don't unit-test cleanly. Smoke-test by running.

New dev-dependency: `tempfile`.

## New runtime dependencies

| Crate | Purpose |
|---|---|
| `clap` v4, features = ["derive"] | CLI parser |
| `tracing` | Structured logging facade |
| `tracing-subscriber`, features = ["env-filter", "fmt"] | Stderr subscriber + `RUST_LOG` honoring |

Stage 2 explicitly does **not** add: `tracing-appender` (file logging deferred to Stage 3), `ctrlc` (no custom signal handling), `crossterm` / `ratatui` (a few ANSI escapes are enough).

## Spec divergence

The top-level design at [2026-05-22-rust-tray-widget-design.md](2026-05-22-rust-tray-widget-design.md#stage-2--polling-daemon-1-2-weeks) says:

> Persist samples to `~/.claude-usage-tray/calibration_log.json` (append-only).

This document changes the filename to `calibration_log.jsonl`. Reason: the format is JSON-lines (one record per line), and the `.jsonl` extension labels that honestly. Append-only JSON-array files have worse failure modes (read-modify-write, fragile under kills), and the spec's "append-only" wording implies the line-oriented format anyway. The top-level spec will be amended to reference `.jsonl` and link this document.

## Stage 2 deliverable / verification

End-to-end checks before tagging `v0.2.0`:

- `cargo fmt --check` → clean.
- `cargo clippy --all-targets -- -D warnings` → clean.
- `cargo test` → 6 existing + 2 render + 1 calibration roundtrip ≈ 9 tests passing.
- `cargo run -- --watch` → live screen redraws every 120s in Windows Terminal; ANSI escapes look correct (no leftover lines, no flicker beyond the redraw).
- `cargo run -- --watch --interval 60` → 60s cadence; if the 429 path triggers, screen keeps the previous sample with `[stale … rate-limited]` footer.
- `cargo run -- --once` → unchanged Stage 1 behavior; backward compatibility check.
- After ~5 minutes of `--watch`: inspect `~/.claude-usage-tray/calibration_log.jsonl` — N lines, each parses as JSON, each has `schema_version: 1`.
- Tag `v0.2.0` and push.
