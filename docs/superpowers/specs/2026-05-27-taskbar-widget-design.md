# Docked taskbar widget — design

**Date:** 2026-05-27
**Status:** Approved (brainstorming) — pending spec review
**Mockups:** branch `design/taskbar-widget-mockups`, file `mockups/widget-mockups.svg`
([GitHub link](https://github.com/borgi-s/claude-usage-tray/blob/design/taskbar-widget-mockups/mockups/widget-mockups.svg))

## Motivation

Replicate the signature surface of [CodeZeno/Claude-Code-Usage-Monitor](https://github.com/CodeZeno/Claude-Code-Usage-Monitor):
a small horizontal widget docked on the Windows taskbar showing **two usage bars**
(5-hour and 7-day) with live percentages and countdown-to-reset timers. Today this
project surfaces usage only as a 16×16 notification-area icon (`src/tray/icon.rs`);
the widget becomes the primary always-visible display while the existing tray icon
stays for the right-click menu.

This is also a deliberate CV/portfolio piece: a hand-written, borderless, always-on-top
Win32 window painted with GDI+ is the kind of native systems work the project exists
to showcase.

## Decisions locked during brainstorming

- **Scope:** the full docked bar-widget (not just an icon restyle).
- **Positioning:** auto-docked over the taskbar, with drag-to-reposition along it and a
  remembered offset.
- **Tray icon:** kept. Widget is primary display; the icon retains the right-click menu.
- **Bar data:** raw API utilization (`five_hour` / `seven_day` from `/api/oauth/usage`) —
  identical to what the tray icon shows. Countdown timers come from each bucket's `resets_at`.
- **Layout:** Variant A — stacked mini-bars (two rows: label · bar · % · countdown), ~300×64 px.
- **Rendering:** Approach A — a second Win32 window on the existing message loop, painted
  with GDI+. (Approach B, a second egui/eframe window, is blocked by the one-EventLoop-per-process
  constraint; Approach C, raw winit+glow on the dashboard loop, was rejected as more complex
  and coupled to the dashboard being open.)

## Architecture

### New module: `src/tray/widget.rs`

Owns one borderless top-level window and its paint/positioning/drag logic.

**Window creation**
- Class: a new `WNDCLASSEXW` (e.g. class name `claude-usage-tray.widget`), `wndproc` in this module.
- Style: `WS_POPUP` (no title bar, no border).
- Ex-style: `WS_EX_TOPMOST | WS_EX_TOOLWINDOW` — always-on-top and excluded from the Alt-Tab
  list / taskbar button. **Not** `WS_EX_LAYERED` (see rounded corners below).
- Created on the **existing** Win32 message loop (`window::message_loop()`), in the same
  startup path that adds the tray icon. No new thread, no new event loop.
- Rounded corners: `SetWindowRgn` with a rounded-rect region (`CreateRoundRectRgn`). This
  keeps a normal opaque `WM_PAINT` path (simpler than `UpdateLayeredWindow` per-pixel alpha)
  while still giving clean rounded corners over the taskbar.

**Per-window state** (`WidgetState`, stored via `Box::into_raw` in `GWLP_USERDATA`, reclaimed
on `WM_NCDESTROY` — same idiom as `TrayState` in `window.rs`):

```rust
struct WidgetState {
    shared: crate::shared::SharedSnapshot,   // Arc clone — read on paint
    settings: crate::shared::SharedSettings, // Arc clone — read for offset/enabled
    drag: DragState,                         // see Positioning + drag
}
```

### Data flow

The widget reads the **same shared snapshot** the dashboard banner already uses. No new
pipeline: `AppSnapshot` already carries `last_sample: Option<(UsageSnapshot, DateTime<Utc>)>`
(each `UsageBucket` has `utilization` + `resets_at`) and `last_status: LastStatus`.

- `WM_PAINT`: take a read-lock on `shared`, derive the two bars' fill/colour/text and the two
  countdowns from `last_sample` + `last_status`, paint via GDI+.
- New polls: `TrayState` gains `widget_hwnd: Option<HWND>`. After `drain_and_redraw` updates
  state and the icon, it calls `InvalidateRect(widget_hwnd, …, TRUE)` so the widget repaints
  immediately with fresh data (it does **not** read `TrayState`; it re-reads `shared`).
- Countdown ticking: a 1-second `WM_TIMER` on the widget calls `InvalidateRect` so the
  remaining-time text counts down between polls. Repaint is cheap (small window, no allocation
  in the hot path beyond a few formatted strings).

### Rendering (GDI+, mirrors `icon.rs`)

For each of the two rows (5h, 7d):
- **Bar track:** rounded rect, dark (`#34363b`).
- **Bar fill:** rounded rect, width = `track_width * util.clamp(0,1)`, colour = `anchored_gradient(util)`
  (reused from `icon.rs` — make it `pub(crate)` if not already).
- **Label** (`5h` / `7d`), **percent** (`{round(util*100)}%`), **countdown** (`format_duration`
  of `resets_at - now`, reused from `render.rs`). All via the same hinted `GdipDrawString`
  path `icon.rs` uses (`TextRenderingHintSingleBitPerPixelGridFit`, Segoe UI).

**Status special-cases** (must match `compute_visual` in `icon.rs` so the widget and icon never
disagree):
- `LastStatus::Initial | RateLimited | Error(_)`, or `Ok` with no sample → gray track, no fill,
  `?` in the percent slot, countdown blank.
- A bucket with `utilization > 1.0` → red fill clamped full, `!` in that row's percent slot.
- Otherwise → gradient fill + digits.

A bucket missing from the snapshot (e.g. `seven_day == None`) renders that row as `?` / `--`.

### Positioning + drag

**Resolve taskbar geometry** on startup, and re-resolve on `TaskbarCreated` (registered message —
Explorer restart) and `WM_DPICHANGED`:
- `SHAppBarMessage(ABM_GETTASKBARPOS)` for the taskbar rect + edge; fallback `FindWindowW("Shell_TrayWnd")`
  + `GetWindowRect`.
- **Supported case:** a horizontal taskbar docked at the bottom (`ABE_BOTTOM`). Place the widget
  vertically centred within the taskbar band, anchored near the right edge (left of the clock),
  shifted left by `settings.widget_offset_px`.
- **Fallback:** non-bottom / autohide / not-found → place at a fixed bottom-right screen position
  and `tracing::warn!` once. Not a first-class target (see non-goals).

**Drag:** `WM_NCHITTEST` returns `HTCAPTION` over the widget body, so Windows handles the drag.
On `WM_EXITSIZEMOVE`, clamp the window back into the taskbar band (snap Y to the centred position,
clamp X within the taskbar width) and persist the new `widget_offset_px` to settings
(`settings::save`, same atomic temp-file+rename path used everywhere).

**DPI:** read the window's DPI (`GetDpiForWindow`) and scale the widget's logical size (300×64 @96dpi)
so it stays correct on scaled displays; recompute on `WM_DPICHANGED`.

### Interaction

- **Left-drag:** move (via `HTCAPTION`).
- **Right-click** (`WM_NCRBUTTONUP`, since the body is `HTCAPTION`): reuse the existing
  `window::show_context_menu` — the widget posts to / shares the tray's menu so there is one
  menu definition. (Implementation detail: factor `show_context_menu` so it can be invoked for
  either window, or have the widget `PostMessage` the tray window to show it.)
- **Double-click** (`WM_NCLBUTTONDBLCLK`): open/show the dashboard — reuse the same path as the
  tray icon's `on_left_click`.

### Settings + tray menu

**`Settings` (src/settings.rs) gains two fields** (`#[serde(default)]` already covers old files):
- `widget_enabled: bool` — default `true`.
- `widget_offset_px: i32` — default `0` (anchored at the default right-side spot). Drag-managed;
  not shown as an editable control.

`validate`: `widget_offset_px` accepts any `i32` (clamped at position time, so no validation
failure); `widget_enabled` is a plain bool. Round-trip + default tests extended accordingly.

**Settings tab (src/dashboard/settings_tab.rs):** add a "Show taskbar widget" checkbox bound to
`widget_enabled`. Toggling it live creates/destroys (or shows/hides) the widget window.

**Tray menu (src/tray/window.rs):** add a checkable item **"Show taskbar widget"** (new
`IDM_WIDGET`), checked = `widget_enabled`. Toggling flips the setting, persists it, and
shows/hides the widget window.

Show/hide vs create/destroy: prefer **create on enable / destroy on disable** for a clean
zero-cost-when-off model, mirroring how the dashboard thread is spawned lazily. (If destroy/recreate
proves fiddly with the message loop, `ShowWindow(SW_HIDE/SW_SHOW)` is an acceptable fallback —
the window is cheap to keep parked.)

## Module / file touch list

- **New:** `src/tray/widget.rs` — window class, `wndproc`, `WidgetState`, paint, positioning, drag,
  timer, create/destroy/show/hide.
- **`src/tray/icon.rs`:** reuse `anchored_gradient` (already `pub(crate)`); optionally lift the
  status→glyph decision into a shared helper so the widget reuses identical logic. No behavioural
  change to the icon.
- **`src/tray/window.rs`:** add `widget_hwnd: Option<HWND>` to `TrayState`; `InvalidateRect` the
  widget in `drain_and_redraw`; add `IDM_WIDGET` + menu item + toggle handler; factor
  `show_context_menu` for reuse; create the widget at startup when `widget_enabled`.
- **`src/tray/mod.rs`:** wire widget creation into the tray startup sequence; expose the module.
- **`src/settings.rs`:** two new fields + defaults + validate + tests.
- **`src/dashboard/settings_tab.rs`:** "Show taskbar widget" checkbox + live apply.
- **`src/render.rs`:** reuse `format_duration` (no change expected).

## Testing strategy

**Unit-testable pure functions** (no Win32/GDI+):
- `bar_fill_width(track_px, util) -> px` — clamps `util` to [0,1]; 0 → 0, 1 → full, mid → proportional.
- countdown text: `format_duration(resets_at - now)` already tested in `render.rs`; add a widget-level
  helper test for the "no resets_at" / past-reset (≤0) cases → blank / "0m".
- `widget_rect(taskbar_rect, dpi, offset_px) -> RECT` — vertical-centre + right-anchor + offset;
  test centring and that offset shifts X left, and that the rect stays within the taskbar width
  (clamp).
- status → row rendering decision (gradient+digits / `!` / `?` / `--`) — table tests mirroring
  `icon.rs::compute_visual`.

**Manual GUI verification** (GDI+ paint, window lifecycle, docking, drag — same approach used for
the tray icon and dashboard window). Spec includes this checklist; record results in CLAUDE.md on
ship:
1. Widget appears docked over the bottom taskbar, left of the clock, on launch (with `widget_enabled`).
2. Two bars show correct colour/percent for the current 5h/7d util; matches the tray icon's number.
3. Countdown timers decrement each second and match `resets_at`.
4. Rate-limited / error / no-data → `?` state; >100% → `!` state.
5. Drag along the taskbar works; release snaps back into the band; offset persists across restart.
6. Right-click → the tray context menu; double-click → dashboard opens/focuses.
7. Tray menu "Show taskbar widget" + Settings checkbox toggle the widget live; state persists.
8. DPI: on a 125%/150% display the widget is correctly sized and positioned.
9. Explorer restart (`TaskbarCreated`) → widget re-docks.

## Non-goals (YAGNI)

- Vertical / left / right / top taskbars and autohide taskbars as first-class targets (fallback only).
- Multi-monitor taskbar awareness (docks on the primary taskbar).
- Codex usage bars (CodeZeno has them; out of scope here).
- Per-pixel alpha / acrylic / blur backgrounds (`WS_EX_LAYERED` not used).
- Resizable widget / user-configurable layout.

## Risks / open considerations

- **`HTCAPTION` swallows clicks:** making the whole body draggable means only NC mouse messages
  fire. Interaction is therefore drag / `WM_NCRBUTTONUP` / `WM_NCLBUTTONDBLCLK`. If a single
  left-click action is later wanted, revisit (e.g. distinguish click from drag manually).
- **Always-on-top over a focused fullscreen app:** `WS_EX_TOPMOST` can overlay games/fullscreen
  video. Acceptable for v1 (the tray icon is the fallback surface); could add fullscreen detection
  later.
- **Taskbar geometry edge cases** (autohide, Explorer restart) are handled best-effort via the
  fallback + `TaskbarCreated` re-dock; documented as not-first-class.
