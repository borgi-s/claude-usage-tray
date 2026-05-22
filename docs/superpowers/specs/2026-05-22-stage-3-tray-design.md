# Stage 3 — Win32 Tray Icon Design

Companion to the top-level [rust-tray-widget-design.md](2026-05-22-rust-tray-widget-design.md). This document fixes the details that the top-level spec left open for Stage 3.

## Goal

Add a tray-icon mode that runs as the default when the .exe is double-clicked: hidden console, polling thread, solid-color status icon updated on each successful poll, hover tooltip with current util, and right-click "Quit" menu. Stage 2's `--once` and `--watch` modes continue to work from a terminal.

This is the "first impressive demo" stage of the project — the smallest possible thing that looks and behaves like a real Windows tray app.

## Non-goals (Stage 3)

- ❌ GDI-rendered numeric icon (the "57%" digits) — Stage 4.
- ❌ Calibration math against local cache — Stage 5.
- ❌ Dashboard window (egui) — Stage 6.
- ❌ Update checker — Stage 6.5.
- ❌ Cloud sync — Stage 7.
- ❌ Single-instance enforcement — multiple icons appear if the user runs the .exe twice. Acceptable for v0.3.0; revisit when there's a settings UI.
- ❌ Auto-start on Windows login — deferred. User launches the .exe manually for now.
- ❌ Settings UI / config file — interval still set via `--interval` flag.
- ❌ Notification toasts — Stage 6+.
- ❌ TaskbarCreated recovery — if Explorer.exe restarts the icon is lost until next app restart. Tracked for Stage 6.

## CLI surface (revised from Stage 2)

The Stage 2 ArgGroup requiring exactly one of `--once`/`--watch` is relaxed: with no flags, the binary enters tray mode. The `--once` and `--watch` flags remain as opt-in alternatives.

```text
USAGE:
  claude-usage-tray [OPTIONS]

MODES (mutually exclusive; if none, tray mode):
  --once                    Single fetch + print, then exit
  --watch                   Long-running loop with redraw-in-place live view

OPTIONS:
  --interval <SECS>         Polling interval (used by --watch and tray). 60 | 120 | 300. Default: 120.
  --log-level <LEVEL>       trace | debug | info | warn | error. Default: info.
  -h, --help
  -V, --version
```

Implementation: change the clap `ArgGroup` from `required = true` to `required = false`. Add `else { tray::run(cli.interval.as_secs())? }` after the existing `if cli.once / else if cli.watch` chain in `main.rs`.

### `#![windows_subsystem = "windows"]`

Add this at the top of `main.rs`. When launched from Explorer, no console window appears (this is what makes the tray app look polished). When launched from a terminal with `--once` or `--watch`, stdio output would normally go to a black hole — to preserve Stage 2 behavior, call `AttachConsole(ATTACH_PARENT_PROCESS)` early in `main()` and ignore the error case (no parent = launched from Explorer = fine, no stdio needed). This is the standard idiom for Rust binaries that are both a GUI app and a CLI.

## Module layout (after Stage 3)

```
src/
  main.rs                — windows_subsystem attribute + AttachConsole + tray dispatch
  cli.rs                 — ArgGroup relaxed (no mode → tray)
  poll.rs                — NEW: pub(crate) fn poll_once(creds) -> Result<UsageSnapshot, FetchError>
                           (extracts fetch + calibration-log step from watch::tick)
  watch.rs               — refactored: tick() now calls poll::poll_once
  render.rs              — unchanged
  paths.rs               — unchanged (tray log path = app_dir())
  tray/                  — NEW module
    mod.rs               — pub fn run(interval_secs: u64) -> Result<()>
    window.rs            — hidden window, message loop, WndProc
    icon.rs              — Shell_NotifyIcon wrappers, IconSet, color → HICON helper
    poller.rs            — polling thread body + interruptible sleep
  log/
    calibration.rs       — unchanged
    tray.rs              — NEW: pub fn init_file_subscriber(level) -> WorkerGuard
                           (tracing-appender daily rotation to ~/.claude-usage-tray/tray.YYYY-MM-DD.log)
  api/                   — unchanged
```

## Threading model

Two threads, no shared mutable state besides one atomic and one mpsc channel.

- **UI thread** = `main()`. Parses CLI, dispatches. If tray mode: initializes file-backed tracing, loads credentials, creates the hidden window, creates the tray icon, spawns the polling thread, runs the message loop until WM_QUIT, joins the polling thread, removes the tray icon, returns. The `WorkerGuard` returned by `init_file_subscriber` is bound to a local in `main()` so it lives the whole process — dropping it flushes the appender.
- **Polling thread.** Owns `Credentials` (moved in). Holds an `Arc<AtomicBool>` (shutdown flag), an `mpsc::Sender<PollEvent>`, and a `Send`-able wrapper around `HWND`. Body: `while !shutdown.load(Relaxed) { fetch + log + send + PostMessage + interruptible-sleep }`.

### `HWND` Send-ability

The `windows` crate defines `HWND` as `HWND(pub *mut core::ffi::c_void)`. Raw pointers are not `Send` by default. The polling thread needs the HWND to call `PostMessageW`. Standard workaround:

```rust
#[derive(Clone, Copy)]
struct SendHwnd(HWND);
unsafe impl Send for SendHwnd {}
```

This is safe because `PostMessageW` is itself thread-safe (it posts to the target thread's message queue) — we never dereference the HWND from the polling thread; we only pass it back through the Win32 API.

## IPC

```rust
pub enum PollEvent {
    Ok(UsageSnapshot),
    RateLimited,
    Error(String),
}
```

Polling thread sends one event through `mpsc::channel<PollEvent>()` after every poll attempt. The send is followed by `unsafe { PostMessageW(hwnd, WM_APP_POLL, WPARAM(0), LPARAM(0)) }` where `WM_APP_POLL = WM_APP + 1`. This wakes the UI thread which then drains the receiver.

The UI thread's WndProc handler for `WM_APP_POLL` calls `rx.try_iter()` to collect all events queued since the last wake (in case multiple polls happened between wakes — rare but possible if the UI thread was busy). It keeps the last event, updates `TrayState`, then calls `Shell_NotifyIconW(NIM_MODIFY)` with the new icon and tooltip.

The channel is bounded only by memory — `mpsc::channel()` (unbounded). Cap-and-drop semantics aren't needed because the UI thread drains promptly and polls are >=60s apart.

## Window + WndProc

Tray apps need an HWND to receive Shell_NotifyIcon callbacks and our custom `WM_APP_POLL`. Window is message-only (never visible):

```text
RegisterClassExW with class name "claude-usage-tray.tray"
CreateWindowExW(0, class, ..., HWND_MESSAGE, ...)
```

`HWND_MESSAGE` parent makes the window invisible, non-enumerable, and excluded from `EnumWindows` — exactly what a tray-only app wants.

`RegisterClassExW` failure path: if it returns 0 and `GetLastError() == ERROR_CLASS_ALREADY_EXISTS`, proceed normally (the class was registered by a prior invocation of this same binary — common when running twice in dev). Other errors are fatal.

WndProc handles:

| Message | Action |
|---|---|
| `WM_APP_POLL` (WM_APP+1) | Drain receiver, update TrayState, call `tray::icon::modify(hwnd, &state)` |
| `WM_APP_TRAYICON` (WM_APP+2) | Shell_NotifyIcon callback. `lparam` carries the mouse event; on `WM_RBUTTONUP` show context menu via `TrackPopupMenu` |
| `WM_COMMAND` with command ID `IDM_QUIT` (1) | `shutdown.store(true)`, `Shell_NotifyIcon(NIM_DELETE)`, `PostQuitMessage(0)` |
| `WM_DESTROY` | `PostQuitMessage(0)` |
| anything else | `DefWindowProcW` |

### Carrying state into WndProc

`TrayState` lives in a `Box<TrayState>` whose raw pointer is stored in the window's `GWLP_USERDATA` slot during `WM_NCCREATE` (passed in via `lpCreateParams` of `CreateWindowExW`). WndProc retrieves it via `GetWindowLongPtrW(hwnd, GWLP_USERDATA)`. This is the canonical Win32 idiom for per-instance state. On `WM_NCDESTROY`, the Box is reclaimed and dropped.

```rust
struct TrayState {
    last_sample: Option<(UsageSnapshot, Instant)>,
    last_status: PollStatus,
    icons: IconSet,
    rx: mpsc::Receiver<PollEvent>,
    shutdown: Arc<AtomicBool>,
}
```

## Tray icon

`Shell_NotifyIconW` with `NIM_ADD` on startup, `NIM_MODIFY` on each update, `NIM_DELETE` on shutdown. `NOTIFYICONDATAW` configured with:

- `uID = 1` (single icon per window)
- `uCallbackMessage = WM_APP_TRAYICON`
- `hIcon` = current state icon (see Icon state machine below)
- `szTip` = current tooltip (≤127 chars wchar buffer)
- `uFlags = NIF_ICON | NIF_TIP | NIF_MESSAGE`

## Icon state machine

Five logical states, four distinct icons:

| State | Icon |
|---|---|
| `Initial` (before first poll completes) | Gray |
| `Ok` with `max(5h, 7d) < 0.60` | Green |
| `Ok` with `0.60 ≤ max(5h, 7d) < 0.85` | Yellow |
| `Ok` with `max(5h, 7d) ≥ 0.85` | Red |
| `RateLimited` or `Error(_)` (regardless of util) | Gray |

The "stale" state collapses into Gray so users immediately notice a problem rather than seeing a stale green icon. The tooltip explains the specific status.

Icons are generated once at startup:

- 16×16 ARGB buffer, all pixels solid color.
- `CreateIcon(hinst, 16, 16, planes=1, bpp=32, mask=&[0u8; 32], color=&color_buf)` returns an `HICON`.
- The four HICONs are kept in `IconSet { gray, green, yellow, red }` and released via `DestroyIcon` before the IconSet is dropped.

Fixed color palette:

| Color | RGB | Hex (B-G-R-A bytes) |
|---|---|---|
| Gray | (128, 128, 128) | `0x80 0x80 0x80 0xFF` |
| Green | (46, 184, 46) | `0x2E 0xB8 0x2E 0xFF` |
| Yellow | (230, 184, 0) | `0x00 0xB8 0xE6 0xFF` |
| Red | (204, 41, 41) | `0x29 0x29 0xCC 0xFF` |

(`CreateIcon` expects bottom-up B-G-R-A row order in the color buffer.)

## Tooltip

`NOTIFYICONDATAW.szTip` is a `[u16; 128]` UTF-16 buffer. Format with sample state:

```text
5h: 57%   7d: 42%
updated 14:24 (Ok)
```

Status footer variants:

- `(Ok)` — last poll succeeded within one interval ago.
- `(stale Xm)` — X is minutes since last successful poll; shown for both rate-limited and error states.
- `(error)` — only used in the initial-error case where no successful poll has ever happened.

When no poll has been attempted yet (the few hundred ms between window creation and first WM_APP_POLL):

```text
Claude usage tray
fetching…
```

When the first poll attempt failed (rate-limited or error) and no good sample has ever been recorded, the standard two-line layout is used with `--` placeholders:

```text
5h: --   7d: --
no data yet (rate-limited)
```

Updates happen on every WM_APP_POLL (i.e., every poll attempt, success or failure).

## Right-click menu

Created lazily on `WM_APP_TRAYICON` with `lparam == WM_RBUTTONUP`:

```text
+----------+
| Quit     |
+----------+
```

Single item with command ID `IDM_QUIT = 1`. Implementation:

```text
GetCursorPos → POINT
SetForegroundWindow(hwnd)               — required Win32 idiom so menu dismisses on click-away
hmenu = CreatePopupMenu()
AppendMenuW(hmenu, MF_STRING, IDM_QUIT, "Quit")
TrackPopupMenu(hmenu, TPM_RIGHTBUTTON, x, y, 0, hwnd, null)
PostMessage(hwnd, WM_NULL, 0, 0)        — companion to SetForegroundWindow
DestroyMenu(hmenu)
```

No reason to cache the HMENU across right-clicks; rebuilding it costs nothing.

## Logging in tray mode

Tray mode hides the console, so tracing must go to a file. New module `src/log/tray.rs`:

```rust
pub fn init_file_subscriber(level: &str) -> WorkerGuard {
    let dir = paths::app_dir().expect("home dir");
    std::fs::create_dir_all(&dir).ok();
    let appender = tracing_appender::rolling::Builder::new()
        .rotation(Rotation::DAILY)
        .filename_prefix("tray")
        .filename_suffix("log")
        .max_log_files(7)
        .build(&dir)
        .expect("appender");
    let (non_blocking, guard) = tracing_appender::non_blocking(appender);
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(level));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(non_blocking)
        .with_target(false)
        .with_ansi(false)
        .init();
    guard
}
```

Produces files like `~/.claude-usage-tray/tray.2026-05-22.log`. The `.max_log_files(7)` keeps the last 7 days of logs and auto-deletes older ones.

The returned `WorkerGuard` is held by `main()` for the lifetime of the process — dropping it flushes the appender at exit.

`--once` and `--watch` keep their existing `tracing_subscriber::fmt().with_writer(std::io::stderr)` subscribers from Stage 2 — no change.

## Shutdown sequence

1. User right-clicks tray icon → context menu shown.
2. User clicks "Quit" → `WM_COMMAND` with `IDM_QUIT`.
3. WndProc:
   - `shutdown.store(true, Ordering::Relaxed)`
   - `Shell_NotifyIconW(NIM_DELETE)` — user sees the icon disappear immediately.
   - `PostQuitMessage(0)` — posts WM_QUIT to the queue.
4. Message loop's `GetMessageW` returns 0 (WM_QUIT) and the loop exits.
5. `tray::run` calls `polling_thread_handle.join().unwrap()` — the polling thread has been checking the shutdown flag between sleeps and exits cleanly.
6. `tray::run` returns to `main()`.
7. `main()` returns; `WorkerGuard` drops → tracing-appender flushes pending events.
8. Process exits.

Polling thread side:

```rust
fn sleep_interruptible(shutdown: &Arc<AtomicBool>, fetch_at: Instant, interval: Duration) {
    let target = fetch_at + interval;
    while Instant::now() < target {
        if shutdown.load(Ordering::Relaxed) {
            return;
        }
        let remaining = target.saturating_duration_since(Instant::now());
        std::thread::sleep(remaining.min(Duration::from_millis(500)));
    }
}
```

Worst-case quit latency: ~500 ms (sleep granularity) + the residual of any in-flight HTTP request (`ureq` default 30s read timeout, in practice a poll completes in <200ms when not rate-limited). The HTTP request itself is not cancelled — `ureq` doesn't support cancellation. The Quit is responsive because the icon disappears immediately (step 3); the actual process exit may lag by up to ~500ms behind that.

## Error handling

- `tray::run` returns `anyhow::Result<()>`. Errors only escape from `load_from_default_path()` and the various Win32 creation calls (register class, create window, create icons, NIM_ADD). All are fatal and surface to `main()` which exits non-zero — Windows will not show a dialog for these failures since stderr goes to the file log; the user sees no tray icon appear, can check `tray.<date>.log` for the cause.
- Polling thread errors are sent through the channel as `PollEvent::Error(msg)` or `RateLimited`. They never crash the thread or propagate up.
- `CreateIcon` returning null causes `tray::run` to return `anyhow::bail!("could not create tray icon")` at startup.
- `Shell_NotifyIconW` failures during NIM_MODIFY are logged as warnings and the loop continues. A common case is Explorer.exe being restarted; Stage 3 does not recover from this (see TaskbarCreated note in Non-goals).

## Calibration log

Unchanged. `poll::poll_once` calls `append_to_default_path` on every `Ok` snapshot — same as Stage 2's `tick`. Tray mode produces identical calibration log entries to watch mode.

## New runtime dependencies

| Crate | Purpose |
|---|---|
| `windows` v0.58+, features = ["Win32_Foundation", "Win32_UI_Shell", "Win32_UI_WindowsAndMessaging", "Win32_System_LibraryLoader", "Win32_Graphics_Gdi"] | Win32 FFI |
| `tracing-appender` v0.2.3+, features = ["non-blocking"] | Daily-rotating file log + non-blocking writer |

No new dev-dependencies. No new test files.

## Testing

- **No unit tests for tray code.** Win32 FFI doesn't unit-test cleanly — the calls require an active message-pump thread and the OS shell. Validation is by manual smoke test: build, run, verify icon appears with right color, hover for tooltip, right-click for menu, click Quit.
- **`poll::poll_once`** is a small extracted helper (~10 lines). The existing `watch.rs` integration is its smoke test; the existing calibration_log tests verify the file-write side effect.
- **No new test files.** The existing 13 tests (Stage 1 + Stage 2) must continue to pass after the `watch::tick` refactor.

## `windows` crate ergonomics

The `windows` crate's auto-generated bindings use UTF-16 strings via `PCWSTR`. A few private helpers in `src/tray/window.rs` reduce verbosity:

```rust
fn wstr(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn pcwstr(buf: &[u16]) -> PCWSTR {
    PCWSTR(buf.as_ptr())
}
```

Call sites store the `Vec<u16>` in a local so the `PCWSTR` doesn't dangle.

## Stage 3 deliverable / verification

End-to-end checks before tagging `v0.3.0`:

- `cargo fmt --check` clean.
- `cargo clippy --all-targets -- -D warnings` clean.
- `cargo test` — 13 Stage 1 + Stage 2 tests still pass.
- `cargo build --release` → `target\release\claude-usage-tray.exe` (~4 MB; the `windows` crate adds 500-700 KB).
- Double-click the .exe in Explorer → tray icon appears within ~1 second showing gray (initial), then transitions to green/yellow/red on first successful poll.
- Hover the icon → tooltip shows two-line content with current 5h/7d and "(Ok)" footer.
- Right-click → menu with "Quit" appears at the cursor.
- Click Quit → icon disappears immediately, process exits within ~1 second.
- `~/.claude-usage-tray/tray.<today>.log` exists with INFO/WARN events.
- `~/.claude-usage-tray/calibration_log.jsonl` continues to grow on each Ok poll.
- `claude-usage-tray.exe --once` and `--watch` still produce Stage-2 output on the terminal (verifies AttachConsole works).
- Force a rate-limit (run `--once` then start tray within 60s): tray icon goes gray, tooltip shows "(stale Nm)".
- Tag `v0.3.0` and push.

## Carry-overs from Stage 2 (unchanged)

- Polling interval default = 120s; 60s and 300s also supported via `--interval`.
- RateLimited 429s do NOT write to calibration log.
- Cadence anchors to fetch start (`fetch_at` Instant captured before fetch).
- No persisted state across restarts. On launch, `last_sample = None` and icon is gray until first Ok poll.

## Stage 3 enabling Stage 4

Stage 4 (rendered percentage icon) replaces `tray::icon::IconSet` with a function that builds a fresh HICON per poll containing the rendered "57" text on a colored background via GDI. The icon state machine and color thresholds carry over unchanged; only the icon-creation step changes. The current `IconSet` struct can stay as-is (covers the "no sample yet" and error states); the Ok branch grows a "render with text" path.
