# Docked Taskbar Widget Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a borderless, always-on-top widget docked over the Windows taskbar that shows two usage bars (5h + 7d) with live percentages and countdown-to-reset timers, alongside the existing tray icon.

**Architecture:** A new `src/tray/widget.rs` owns one top-level Win32 window created on the **existing** tray message loop and painted with GDI+ (same toolkit as `src/tray/icon.rs`). The window is self-driven: a 1-second `WM_TIMER` re-reads the shared snapshot + settings each tick to (a) repaint the bars/countdowns, (b) re-dock itself over the live taskbar rect, and (c) show/hide itself to match `settings.widget_enabled`. Right-click / double-click are *forwarded* to the tray window via its existing `WM_APP_TRAYICON` message, so the menu and dashboard paths are reused unchanged.

**Tech Stack:** Rust, `windows` 0.58 (`Win32_Graphics_GdiPlus`, `Win32_UI_Shell`, `Win32_UI_WindowsAndMessaging`, `Win32_Graphics_Gdi`), GDI+, `chrono`.

**Spec:** `docs/superpowers/specs/2026-05-27-taskbar-widget-design.md`

**Beginner note (this whole plan):** The user is a Rust beginner. The hard parts here are Win32 FFI and GDI+; steps explain the unfamiliar idioms inline. The two local references are `src/tray/icon.rs` (every GDI+ call we use already appears there) and `src/tray/window.rs` (window class registration, `GWLP_USERDATA` state, the `wndproc` pattern, the context menu). Read both before starting.

**Windows worktree gotcha:** Build/test in the main checkout, not a nested `.claude/worktrees/...` path (MAX_PATH link failures). If linking OOMs, use `cargo test -j 1`.

### Deviations from the spec (deliberate, to reduce risk)

These simplify the spec's mechanisms without changing the user-visible result. They are improvements found while mapping the code:

1. **Self-driven, not poll-pushed.** The spec had the poll handler `InvalidateRect` the widget. Instead the widget's own 1-second `WM_TIMER` repaints (the countdowns need a per-second tick anyway), so fresh poll data appears within ≤1 s with **zero coupling to `TrayState`**. No `widget_hwnd` field is added to `TrayState`.
2. **Re-dock every tick instead of handling `TaskbarCreated` / `WM_DPICHANGED`.** The timer recomputes position from the *live* taskbar rect each second, which inherently covers Explorer restarts, DPI changes, and taskbar moves. Sizing is derived from the taskbar's pixel height, so it is DPI-correct without querying DPI. (No `Win32_UI_HiDpi` feature needed.)
3. **Create-once-hidden + `ShowWindow`, not create/destroy.** A window may only be destroyed by its creating thread; toggling via `ShowWindow(SW_SHOW/SW_HIDE)` from the timer (on the owning thread) is race-free and lets the Settings checkbox toggle work cross-thread by just writing the setting.
4. **Forward gestures to the tray window.** Right-click and double-click `PostMessage` the existing `WM_APP_TRAYICON` to the tray HWND, reusing `show_context_menu` / `on_left_click` verbatim. The widget only needs the tray HWND, not the dashboard handle.
5. **Version bump is `v1.2.0`** (new user-facing feature on top of `v1.1.0`).

---

### Task 1: Settings fields `widget_enabled` + `widget_offset_px`

**Files:**
- Modify: `src/settings.rs:57-77` (struct + Default), `src/settings.rs` tests

- [ ] **Step 1: Add the two fields to the `Settings` struct**

In `src/settings.rs`, the struct (currently lines 57-65) gains two fields:

```rust
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub local_tz: String,
    pub weekly_reset_weekday: Weekday,
    pub weekly_reset_hour: u32,
    pub poll_interval_secs: u64,
    pub cost_weights: CostWeights,
    /// Whether the docked taskbar widget is shown. Default true.
    pub widget_enabled: bool,
    /// Pixels the widget is shifted left from its default right-anchored dock
    /// position. Drag-managed; clamped at use, so any i32 is accepted.
    pub widget_offset_px: i32,
}
```

- [ ] **Step 2: Add their defaults**

In `impl Default for Settings` (currently lines 67-77), add the two fields:

```rust
impl Default for Settings {
    fn default() -> Self {
        Self {
            local_tz: config::LOCAL_TZ.to_string(),
            weekly_reset_weekday: config::WEEKLY_RESET_WEEKDAY,
            weekly_reset_hour: config::WEEKLY_RESET_HOUR_LOCAL,
            poll_interval_secs: 120,
            cost_weights: CostWeights::default(),
            widget_enabled: true,
            widget_offset_px: 0,
        }
    }
}
```

(No `validate` change: `widget_enabled` is a bool and `widget_offset_px` is clamped when the dock rect is computed, so neither can fail validation.)

- [ ] **Step 3: Write a failing test for default + round-trip of the new fields**

Add to the `tests` module in `src/settings.rs`:

```rust
    #[test]
    fn default_widget_fields() {
        let s = Settings::default();
        assert!(s.widget_enabled);
        assert_eq!(s.widget_offset_px, 0);
    }

    #[test]
    fn save_to_then_load_from_round_trips_widget_fields() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("settings.toml");
        let s = Settings {
            widget_enabled: false,
            widget_offset_px: 120,
            ..Settings::default()
        };
        save_to(&p, &s).unwrap();
        assert_eq!(load_from(&p), s);
    }
```

- [ ] **Step 4: Run the tests**

Run: `cargo test --lib settings`
Expected: PASS, including `default_widget_fields` and `save_to_then_load_from_round_trips_widget_fields`. The pre-existing `default_settings_match_config_consts` still passes (it does not touch the new fields).

- [ ] **Step 5: Commit**

```powershell
git add src/settings.rs
git commit -m "feat(widget): add widget_enabled + widget_offset_px settings"
```

---

### Task 2: Pure widget helpers + module skeleton (TDD)

**Files:**
- Modify: `src/tray/mod.rs:7-9` (declare module)
- Create: `src/tray/widget.rs` (pure helpers + tests only; the Win32 window comes in Task 3)

**Beginner note:** This task adds only *pure* functions (no Win32) so they can be unit-tested. They decide geometry (`dock_rect`, `bar_fill_width`) and per-row visual state (`row_state`). `RECT` is the Win32 rectangle struct (`left/top/right/bottom`, i32); we import the real one so Task 3 can reuse these directly.

- [ ] **Step 1: Declare the module**

In `src/tray/mod.rs`, the module list (lines 7-9) becomes:

```rust
pub mod icon;
pub mod poller;
pub mod widget;
pub mod window;
```

- [ ] **Step 2: Create `src/tray/widget.rs` with the pure helpers**

```rust
//! Borderless, always-on-top widget docked over the Windows taskbar. Shows two
//! usage bars (5h + 7d) with live % and reset countdowns. Painted with GDI+
//! (mirrors `crate::tray::icon`); self-driven by a 1-second WM_TIMER that
//! repaints, re-docks over the live taskbar rect, and shows/hides to match
//! `settings.widget_enabled`.

use crate::api::usage::UsageBucket;
use crate::render::LastStatus;
use chrono::{DateTime, Utc};
use windows::Win32::Foundation::RECT;

/// Logical widget size derived from the taskbar's pixel height (so it is
/// DPI-correct without querying DPI). Margin keeps it off the taskbar edges.
const MARGIN_PX: i32 = 4;
/// Widget width as a multiple of its height (two short rows of "label bar % time").
const WIDTH_RATIO: i32 = 6;

/// Compute the widget's screen rectangle from the live taskbar rect and the
/// saved right-anchored offset. Vertically centered in the taskbar band,
/// anchored near the right edge, shifted left by `offset_px`, clamped to stay
/// within the taskbar horizontally.
pub(crate) fn dock_rect(taskbar: RECT, offset_px: i32) -> RECT {
    let tb_h = (taskbar.bottom - taskbar.top).max(1);
    let h = (tb_h - 2 * MARGIN_PX).max(8);
    let w = h * WIDTH_RATIO;

    let y = taskbar.top + (tb_h - h) / 2;

    // Default anchor: right edge minus a margin. Offset shifts it further left.
    let mut x = taskbar.right - w - MARGIN_PX - offset_px;
    // Clamp within the taskbar so it never leaves the band.
    let min_x = taskbar.left + MARGIN_PX;
    let max_x = taskbar.right - w - MARGIN_PX;
    if max_x >= min_x {
        x = x.clamp(min_x, max_x);
    } else {
        x = min_x; // taskbar narrower than the widget; pin left
    }

    RECT {
        left: x,
        top: y,
        right: x + w,
        bottom: y + h,
    }
}

/// Given a widget's current left edge and the live taskbar rect, derive the
/// offset to persist (distance from the right-anchored default position).
pub(crate) fn offset_from_left(taskbar: RECT, left: i32) -> i32 {
    let tb_h = (taskbar.bottom - taskbar.top).max(1);
    let h = (tb_h - 2 * MARGIN_PX).max(8);
    let w = h * WIDTH_RATIO;
    // left = taskbar.right - w - MARGIN - offset  =>  offset = taskbar.right - w - MARGIN - left
    (taskbar.right - w - MARGIN_PX - left).max(0)
}

/// Filled width of a bar given its track width and a utilization in [0, ∞).
pub(crate) fn bar_fill_width(track_w: i32, util: f64) -> i32 {
    let u = util.clamp(0.0, 1.0);
    (track_w as f64 * u).round() as i32
}

/// What one row (5h or 7d) should paint.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum RowState {
    /// Normal: util in [0,1], gradient color, "NN%", optional countdown text.
    Data {
        util: f64,
        pct: u8,
        countdown: Option<String>,
    },
    /// util > 100%: full red bar, "!".
    Bang { countdown: Option<String> },
    /// No usable data (rate-limited / error / initial / missing bucket).
    Question,
}

/// Decide a row's visual state, mirroring `icon::compute_visual` but per-bucket.
pub(crate) fn row_state(
    status: &LastStatus,
    bucket: Option<&UsageBucket>,
    now: DateTime<Utc>,
) -> RowState {
    match status {
        LastStatus::Initial | LastStatus::RateLimited | LastStatus::Error(_) => RowState::Question,
        LastStatus::Ok => match bucket {
            None => RowState::Question,
            Some(b) => {
                let countdown = b
                    .resets_at
                    .map(|when| crate::render::format_duration(when - now));
                if b.utilization > 1.0 {
                    RowState::Bang { countdown }
                } else {
                    let pct = (b.utilization.clamp(0.0, 1.0) * 100.0).round() as u8;
                    RowState::Data {
                        util: b.utilization,
                        pct,
                        countdown,
                    }
                }
            }
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, TimeZone};

    fn rect(l: i32, t: i32, r: i32, b: i32) -> RECT {
        RECT { left: l, top: t, right: r, bottom: b }
    }

    #[test]
    fn dock_rect_centers_vertically_and_anchors_right() {
        // taskbar 0..1920 wide, 48px tall at the bottom of a 1080 screen.
        let tb = rect(0, 1032, 1920, 1080);
        let r = dock_rect(tb, 0);
        let h = 48 - 8; // 40
        let w = h * WIDTH_RATIO; // 240
        assert_eq!(r.bottom - r.top, h);
        assert_eq!(r.right - r.left, w);
        // right edge sits a margin in from the taskbar right edge.
        assert_eq!(r.right, 1920 - MARGIN_PX);
        // vertically centered: top margin == bottom margin == 4.
        assert_eq!(r.top, 1032 + 4);
    }

    #[test]
    fn dock_rect_offset_shifts_left() {
        let tb = rect(0, 1032, 1920, 1080);
        let base = dock_rect(tb, 0);
        let shifted = dock_rect(tb, 100);
        assert_eq!(base.left - shifted.left, 100);
    }

    #[test]
    fn dock_rect_clamps_within_taskbar() {
        let tb = rect(0, 1032, 1920, 1080);
        // Absurd offset would push it off the left edge; must clamp to left margin.
        let r = dock_rect(tb, 100_000);
        assert_eq!(r.left, MARGIN_PX);
    }

    #[test]
    fn offset_from_left_is_inverse_of_dock_rect() {
        let tb = rect(0, 1032, 1920, 1080);
        let r = dock_rect(tb, 137);
        assert_eq!(offset_from_left(tb, r.left), 137);
    }

    #[test]
    fn bar_fill_width_clamps() {
        assert_eq!(bar_fill_width(200, 0.0), 0);
        assert_eq!(bar_fill_width(200, 0.5), 100);
        assert_eq!(bar_fill_width(200, 1.0), 200);
        assert_eq!(bar_fill_width(200, 1.5), 200); // clamps over 100%
    }

    fn bucket(util: f64, resets_in_secs: Option<i64>, now: DateTime<Utc>) -> UsageBucket {
        UsageBucket {
            utilization: util,
            resets_at: resets_in_secs.map(|s| now + Duration::seconds(s)),
        }
    }

    #[test]
    fn row_state_ok_data_has_pct_and_countdown() {
        let now = Utc.with_ymd_and_hms(2026, 5, 27, 12, 0, 0).unwrap();
        let b = bucket(0.57, Some(3600 + 42 * 60), now); // 1h42m
        let rs = row_state(&LastStatus::Ok, Some(&b), now);
        match rs {
            RowState::Data { pct, countdown, .. } => {
                assert_eq!(pct, 57);
                assert_eq!(countdown.as_deref(), Some("1h 42m"));
            }
            other => panic!("expected Data, got {other:?}"),
        }
    }

    #[test]
    fn row_state_over_100_is_bang() {
        let now = Utc::now();
        let b = bucket(1.2, None, now);
        assert!(matches!(row_state(&LastStatus::Ok, Some(&b), now), RowState::Bang { .. }));
    }

    #[test]
    fn row_state_rate_limited_is_question_even_with_bucket() {
        let now = Utc::now();
        let b = bucket(0.5, Some(60), now);
        assert_eq!(row_state(&LastStatus::RateLimited, Some(&b), now), RowState::Question);
    }

    #[test]
    fn row_state_missing_bucket_is_question() {
        let now = Utc::now();
        assert_eq!(row_state(&LastStatus::Ok, None, now), RowState::Question);
    }
}
```

- [ ] **Step 3: Run the tests**

Run: `cargo test --lib widget`
Expected: PASS — all 9 tests. (`format_duration` produces `"1h 42m"`; confirm against `src/render.rs:104`.)

- [ ] **Step 4: Commit**

```powershell
git add src/tray/mod.rs src/tray/widget.rs
git commit -m "feat(widget): pure geometry + row-state helpers (TDD)"
```

---

### Task 3: The Win32 window — class, creation, GDI+ paint, timer

**Files:**
- Modify: `src/tray/widget.rs` (add the window code above the `#[cfg(test)]` module)

**Beginner notes:**
- The window state lives in a `Box<WidgetState>` whose raw pointer is stashed in `GWLP_USERDATA` and reclaimed on `WM_NCDESTROY` — identical to `TrayState` in `window.rs`.
- We paint **off-screen** into a GDI+ bitmap (exactly like `icon.rs`), then blit it to the window in one `GdipDrawImageRectI` call so there is no flicker on the 1-second repaint.
- `SendHwnd` (from `crate::tray::poller`) is the thread-safe HWND wrapper; we keep the tray HWND in it for gesture forwarding (used in Task 5).
- All GDI+ functions used here already appear in `src/tray/icon.rs` except `GdipCreateFromHDC`, `GdipDrawImageRectI`, `GdipFillRectangleI`, and `GdipCreatePen1`/`GdipDrawRectangleI` (border). If any import name differs in `windows` 0.58, check the crate's `Win32::Graphics::GdiPlus` module — `icon.rs` is the working reference for the spelling of `Status`, `Unit*`, brushes, fonts, and string formats.

- [ ] **Step 1: Add imports at the top of `src/tray/widget.rs`**

Below the existing `use windows::Win32::Foundation::RECT;`:

```rust
use crate::shared::{SharedSettings, SharedSnapshot};
use crate::tray::poller::SendHwnd;
use anyhow::{anyhow, Result};
use windows::core::PCWSTR;
use windows::Win32::Foundation::{HMODULE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, CreateRoundRectRgn, EndPaint, HDC, PAINTSTRUCT,
};
use windows::Win32::Graphics::GdiPlus::{
    FontStyleBold, GdipCreateFont, GdipCreateFontFamilyFromName, GdipCreateFromHDC,
    GdipCreateSolidFill, GdipCreateStringFormat, GdipDeleteBrush, GdipDeleteFont,
    GdipDeleteFontFamily, GdipDeleteGraphics, GdipDeleteStringFormat, GdipDisposeImage,
    GdipDrawImageRectI, GdipDrawString, GdipFillRectangleI, GdipGetImageGraphicsContext,
    GdipGraphicsClear, GdipSetStringFormatAlign, GdipSetStringFormatLineAlign,
    GdipSetTextRenderingHint, GpBitmap, GpBrush, GpFont, GpFontFamily, GpGraphics, GpImage,
    GpSolidFill, GpStringFormat, RectF, Status, StringAlignmentCenter, StringAlignmentNear,
    TextRenderingHintSingleBitPerPixelGridFit, UnitPixel,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, GetWindowLongPtrW, RegisterClassExW, SetWindowLongPtrW,
    CREATESTRUCTW, GWLP_USERDATA, HMENU, WINDOW_EX_STYLE, WINDOW_STYLE, WM_NCCREATE, WM_NCDESTROY,
    WM_PAINT, WM_TIMER, WNDCLASSEXW, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};

// 0xFF000000 | 32bpp ARGB — same constant icon.rs documents.
const PIXEL_FORMAT_32BPP_ARGB: i32 = 0x0026_200A;

/// Timer id for the 1-second repaint/redock/visibility tick.
const TIMER_ID: usize = 1;

/// Window class name (UTF-16, null-terminated).
const WIDGET_CLASS: PCWSTR = windows::core::w!("claude-usage-tray.widget");
```

(`windows::core::w!` builds a static null-terminated UTF-16 literal — simpler than the hand-rolled array in `window.rs`.)

- [ ] **Step 2: Add the `WidgetState` struct + Drop**

```rust
/// Per-window state stored via `GWLP_USERDATA`. The widget reads `shared` on
/// paint and `settings` for the dock offset + enabled flag.
pub struct WidgetState {
    pub tray_hwnd: SendHwnd,
    pub shared: SharedSnapshot,
    pub settings: SharedSettings,
    /// Last computed dock rect, so we only call SetWindowPos when it changes.
    pub last_rect: Option<RECT>,
    /// Whether the window is currently shown (mirrors settings.widget_enabled).
    pub shown: bool,
}
```

- [ ] **Step 3: Add class registration + `create`**

```rust
fn register_class(hinst: HMODULE) -> Result<()> {
    let wc = WNDCLASSEXW {
        cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
        lpfnWndProc: Some(wndproc),
        hInstance: hinst.into(),
        lpszClassName: WIDGET_CLASS,
        ..Default::default()
    };
    // SAFETY: wc lives for the call.
    let atom = unsafe { RegisterClassExW(&wc) };
    if atom == 0 {
        let err = unsafe { windows::Win32::Foundation::GetLastError() };
        if err.0 == 1410 {
            return Ok(()); // ERROR_CLASS_ALREADY_EXISTS — fine on repeat runs
        }
        return Err(anyhow!("RegisterClassExW(widget) failed: {err:?}"));
    }
    Ok(())
}

/// Create the widget window (initially hidden; the first timer tick shows it if
/// enabled). Returns the HWND. State is leaked into GWLP_USERDATA and reclaimed
/// on WM_NCDESTROY.
pub fn create(
    hinst: HMODULE,
    tray_hwnd: HWND,
    shared: SharedSnapshot,
    settings: SharedSettings,
) -> Result<HWND> {
    register_class(hinst)?;

    let state = Box::new(WidgetState {
        tray_hwnd: SendHwnd(tray_hwnd),
        shared,
        settings,
        last_rect: None,
        shown: false,
    });
    let state_ptr = Box::into_raw(state);

    // WS_POPUP: no title/border. WS_EX_TOPMOST: always on top. WS_EX_TOOLWINDOW:
    // no taskbar button / Alt-Tab entry. Position/size are set by the timer.
    // SAFETY: class is registered; params valid; null parent => top-level.
    let hwnd = unsafe {
        CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
            WIDGET_CLASS,
            WIDGET_CLASS,
            WS_POPUP,
            0,
            0,
            10,
            10,
            None,
            HMENU::default(),
            hinst,
            Some(state_ptr.cast()),
        )
    };

    match hwnd {
        Ok(h) => {
            // 1-second tick for repaint + redock + show/hide.
            // SAFETY: h is a valid window we just created.
            unsafe {
                windows::Win32::UI::WindowsAndMessaging::SetTimer(h, TIMER_ID, 1000, None);
            }
            Ok(h)
        }
        Err(e) => {
            // Reclaim the leaked Box on the error path.
            unsafe { drop(Box::from_raw(state_ptr)) };
            Err(anyhow!("CreateWindowExW(widget) failed: {e}"))
        }
    }
}

fn with_state<F: FnOnce(&mut WidgetState)>(hwnd: HWND, f: F) {
    let ptr = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *mut WidgetState;
    if !ptr.is_null() {
        // SAFETY: pointer set in `create`; window is single-threaded.
        let state = unsafe { &mut *ptr };
        f(state);
    }
}
```

(`SetTimer` needs to be imported; add `SetTimer` to the `WindowsAndMessaging` import list in Step 1 if your editor flags it — it lives in that module.)

- [ ] **Step 4: Add the `wndproc` with `WM_NCCREATE`, `WM_PAINT`, `WM_TIMER` stub, `WM_NCDESTROY`**

```rust
extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_NCCREATE => {
            // SAFETY: lparam is *const CREATESTRUCTW on WM_NCCREATE.
            let cs = unsafe { &*(lparam.0 as *const CREATESTRUCTW) };
            unsafe { SetWindowLongPtrW(hwnd, GWLP_USERDATA, cs.lpCreateParams as isize) };
            unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
        }
        WM_PAINT => {
            with_state(hwnd, |state| paint(hwnd, state));
            LRESULT(0)
        }
        WM_TIMER => {
            // Filled in Task 5 (redock + show/hide + repaint). For now just repaint.
            unsafe {
                let _ = windows::Win32::Graphics::Gdi::InvalidateRect(hwnd, None, false);
            }
            LRESULT(0)
        }
        WM_NCDESTROY => {
            let ptr = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *mut WidgetState;
            if !ptr.is_null() {
                // SAFETY: set by `create` via Box::into_raw.
                unsafe { drop(Box::from_raw(ptr)) };
                unsafe { SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0) };
            }
            unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}
```

(Add `InvalidateRect` to the `Win32::Graphics::Gdi` import list.)

- [ ] **Step 5: Add the GDI+ paint function**

This paints off-screen then blits. Layout: two rows; each row = label (left), bar track + fill (middle), pct (right of bar), countdown (far right). Colors reuse `icon::anchored_gradient`.

```rust
/// Paint the widget: off-screen GDI+ bitmap, then one blit to the window DC.
fn paint(hwnd: HWND, state: &WidgetState) {
    // Window client size.
    let mut rc = RECT::default();
    // SAFETY: hwnd valid; rc out-param.
    unsafe {
        let _ = windows::Win32::UI::WindowsAndMessaging::GetClientRect(hwnd, &mut rc);
    }
    let w = (rc.right - rc.left).max(1);
    let h = (rc.bottom - rc.top).max(1);

    let mut ps = PAINTSTRUCT::default();
    // SAFETY: hwnd valid; ps out-param.
    let hdc = unsafe { BeginPaint(hwnd, &mut ps) };
    if hdc.is_invalid() {
        return;
    }

    // Best-effort: any GDI+ failure just leaves the window blank this frame.
    let _ = paint_offscreen(hdc, w, h, state);

    // SAFETY: matched BeginPaint/EndPaint pair.
    unsafe {
        let _ = EndPaint(hwnd, &ps);
    }
}

fn paint_offscreen(hdc: HDC, w: i32, h: i32, state: &WidgetState) -> Result<()> {
    // 1) Off-screen ARGB bitmap + graphics (same as icon.rs).
    let mut bitmap: *mut GpBitmap = std::ptr::null_mut();
    let s = unsafe {
        windows::Win32::Graphics::GdiPlus::GdipCreateBitmapFromScan0(
            w, h, 0, PIXEL_FORMAT_32BPP_ARGB, None, &mut bitmap,
        )
    };
    if s != Status(0) {
        anyhow::bail!("GdipCreateBitmapFromScan0 failed: {s:?}");
    }
    let mut g: *mut GpGraphics = std::ptr::null_mut();
    let s = unsafe { GdipGetImageGraphicsContext(bitmap as *mut GpImage, &mut g) };
    if s != Status(0) {
        unsafe { GdipDisposeImage(bitmap as *mut GpImage) };
        anyhow::bail!("GdipGetImageGraphicsContext failed: {s:?}");
    }

    // 2) Panel background (#26282b opaque) and a subtle border (#3a3c40).
    let _ = unsafe { GdipGraphicsClear(g, 0xFF26_282Bu32) };
    unsafe { GdipSetTextRenderingHint(g, TextRenderingHintSingleBitPerPixelGridFit) };

    // 3) Read the snapshot once.
    let snap = state.shared.read().ok();
    let (sample, status) = match snap.as_ref() {
        Some(s) => (
            s.last_sample.as_ref().map(|(snap, _)| snap.clone()),
            s.last_status.clone(),
        ),
        None => (None, LastStatus::Initial),
    };
    let now = Utc::now();

    // 4) Layout metrics.
    let pad = (h / 8).max(2);
    let row_h = (h - 2 * pad) / 2;
    let label_w = row_h * 2; // room for "5h"/"7d"
    let time_w = (w as f32 * 0.28) as i32;
    let pct_w = (w as f32 * 0.16) as i32;
    let bar_x = pad + label_w;
    let bar_w = (w - pad - bar_x - pct_w - time_w).max(4);
    let bar_h = (row_h / 3).max(3);

    let five = sample.as_ref().and_then(|s| s.five_hour.clone());
    let seven = sample.as_ref().and_then(|s| s.seven_day.clone());
    let rows = [
        ("5h", crate::tray::widget::row_state(&status, five.as_ref(), now)),
        ("7d", crate::tray::widget::row_state(&status, seven.as_ref(), now)),
    ];

    // Reusable white text brush + font family.
    let font_name: Vec<u16> = "Segoe UI".encode_utf16().chain(std::iter::once(0)).collect();
    let mut family: *mut GpFontFamily = std::ptr::null_mut();
    unsafe {
        GdipCreateFontFamilyFromName(PCWSTR(font_name.as_ptr()), std::ptr::null_mut(), &mut family)
    };
    let em = (row_h as f32 * 0.6).clamp(8.0, 16.0);
    let mut font: *mut GpFont = std::ptr::null_mut();
    unsafe { GdipCreateFont(family, em, FontStyleBold.0, UnitPixel, &mut font) };
    let mut white: *mut GpSolidFill = std::ptr::null_mut();
    unsafe { GdipCreateSolidFill(0xFFFF_FFFFu32, &mut white) };
    let mut gray: *mut GpSolidFill = std::ptr::null_mut();
    unsafe { GdipCreateSolidFill(0xFF9A_A0A6u32, &mut gray) };

    for (i, (label, rs)) in rows.iter().enumerate() {
        let row_y = pad + i as i32 * row_h;
        let bar_y = row_y + (row_h - bar_h) / 2;

        // Label (left, gray).
        draw_text(g, label, gray as *mut GpBrush, font, pad, row_y, label_w, row_h, false);

        // Track (dark) — always drawn.
        let mut track: *mut GpSolidFill = std::ptr::null_mut();
        unsafe { GdipCreateSolidFill(0xFF34_363Bu32, &mut track) };
        unsafe { GdipFillRectangleI(g, track as *mut GpBrush, bar_x, bar_y, bar_w, bar_h) };
        unsafe { GdipDeleteBrush(track as *mut GpBrush) };

        match rs {
            crate::tray::widget::RowState::Data { util, pct, countdown } => {
                let (r, gg, b) = crate::tray::icon::anchored_gradient(*util);
                let argb = 0xFF00_0000u32 | (u32::from(r) << 16) | (u32::from(gg) << 8) | u32::from(b);
                let fill_w = crate::tray::widget::bar_fill_width(bar_w, *util);
                let mut fill: *mut GpSolidFill = std::ptr::null_mut();
                unsafe { GdipCreateSolidFill(argb, &mut fill) };
                unsafe { GdipFillRectangleI(g, fill as *mut GpBrush, bar_x, bar_y, fill_w, bar_h) };
                unsafe { GdipDeleteBrush(fill as *mut GpBrush) };
                draw_text(g, &format!("{pct}%"), white as *mut GpBrush, font,
                    bar_x + bar_w, row_y, pct_w, row_h, false);
                if let Some(cd) = countdown {
                    draw_text(g, cd, gray as *mut GpBrush, font,
                        bar_x + bar_w + pct_w, row_y, time_w, row_h, false);
                }
            }
            crate::tray::widget::RowState::Bang { countdown } => {
                let mut fill: *mut GpSolidFill = std::ptr::null_mut();
                unsafe { GdipCreateSolidFill(0xFFCC_2929u32, &mut fill) };
                unsafe { GdipFillRectangleI(g, fill as *mut GpBrush, bar_x, bar_y, bar_w, bar_h) };
                unsafe { GdipDeleteBrush(fill as *mut GpBrush) };
                draw_text(g, "!", white as *mut GpBrush, font,
                    bar_x + bar_w, row_y, pct_w, row_h, false);
                if let Some(cd) = countdown {
                    draw_text(g, cd, gray as *mut GpBrush, font,
                        bar_x + bar_w + pct_w, row_y, time_w, row_h, false);
                }
            }
            crate::tray::widget::RowState::Question => {
                draw_text(g, "?", gray as *mut GpBrush, font,
                    bar_x + bar_w, row_y, pct_w, row_h, false);
            }
        }
    }

    // Clean up shared GDI+ objects.
    unsafe {
        GdipDeleteBrush(white as *mut GpBrush);
        GdipDeleteBrush(gray as *mut GpBrush);
        GdipDeleteFont(font);
        GdipDeleteFontFamily(family);
    }

    // 5) Blit the off-screen bitmap to the window DC in one shot (no flicker).
    let mut gd: *mut GpGraphics = std::ptr::null_mut();
    let s = unsafe { GdipCreateFromHDC(hdc, &mut gd) };
    if s == Status(0) {
        unsafe { GdipDrawImageRectI(gd, bitmap as *mut GpImage, 0, 0, w, h) };
        unsafe { GdipDeleteGraphics(gd) };
    }

    // 6) Dispose off-screen objects.
    unsafe {
        GdipDeleteGraphics(g);
        GdipDisposeImage(bitmap as *mut GpImage);
    }
    Ok(())
}

/// Draw a single string into a layout rect with the given brush. `center` picks
/// horizontal centering vs left alignment; vertical is always centered.
#[allow(clippy::too_many_arguments)]
fn draw_text(
    g: *mut GpGraphics,
    text: &str,
    brush: *mut GpBrush,
    font: *mut GpFont,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    center: bool,
) {
    let utf16: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
    let len = text.encode_utf16().count() as i32;
    let mut fmt: *mut GpStringFormat = std::ptr::null_mut();
    unsafe { GdipCreateStringFormat(0, 0u16, &mut fmt) };
    let align = if center { StringAlignmentCenter } else { StringAlignmentNear };
    unsafe { GdipSetStringFormatAlign(fmt, align) };
    unsafe { GdipSetStringFormatLineAlign(fmt, StringAlignmentCenter) };
    let layout = RectF { X: x as f32, Y: y as f32, Width: w as f32, Height: h as f32 };
    unsafe { GdipDrawString(g, PCWSTR(utf16.as_ptr()), len, font, &layout, fmt, brush) };
    unsafe { GdipDeleteStringFormat(fmt) };
}
```

(`GetClientRect` lives in `Win32::UI::WindowsAndMessaging`; `GdipCreateBitmapFromScan0` in `GdiPlus`. Add them to the imports as the compiler directs. Reference `icon.rs` for exact module paths.)

- [ ] **Step 6: Verify it compiles clean**

Run: `cargo build` then `cargo clippy --all-targets -- -D warnings`
Expected: both succeed, no warnings. The widget is not created yet (Task 5 wires it in), so nothing is shown — this is a compile-only checkpoint. Fix any import-name mismatches against `icon.rs` now.

- [ ] **Step 7: Commit**

```powershell
git add src/tray/widget.rs
git commit -m "feat(widget): Win32 window + GDI+ off-screen paint of the two bars"
```

---

### Task 4: Rounded corners helper + timer redock/show-hide/drag/gestures

**Files:**
- Modify: `src/tray/widget.rs` (flesh out `WM_TIMER`; add `WM_NCHITTEST`, `WM_NCRBUTTONUP`, `WM_NCLBUTTONDBLCLK`, `WM_EXITSIZEMOVE`; add taskbar geometry + apply-rect helpers)

**Beginner notes:**
- `SHAppBarMessage(ABM_GETTASKBARPOS, &mut abd)` fills `abd.rc` (taskbar rect, screen coords) and `abd.uEdge` (which screen edge). We only first-class the bottom edge (`ABE_BOTTOM = 3`).
- Returning `HTCAPTION` from `WM_NCHITTEST` makes the whole body behave like a title bar, so Windows handles left-drag for free. The cost: only *non-client* mouse messages fire — hence right-click is `WM_NCRBUTTONUP` and double-click is `WM_NCLBUTTONDBLCLK`.
- We forward those gestures to the tray window via its existing `WM_APP_TRAYICON` message, whose handler already shows the menu (on right) or opens the dashboard (on left). No new logic.

- [ ] **Step 1: Add taskbar geometry + apply helpers**

Add near the other helpers in `widget.rs`:

```rust
use windows::Win32::UI::Shell::{SHAppBarMessage, APPBARDATA, ABM_GETTASKBARPOS};

const ABE_BOTTOM: u32 = 3;

/// Query the live taskbar rect + edge. Returns None if the call fails.
fn taskbar_rect() -> Option<(RECT, u32)> {
    let mut abd = APPBARDATA {
        cbSize: std::mem::size_of::<APPBARDATA>() as u32,
        ..Default::default()
    };
    // SAFETY: abd is a valid out-param for the documented message.
    let ok = unsafe { SHAppBarMessage(ABM_GETTASKBARPOS, &mut abd) };
    if ok == 0 {
        return None;
    }
    Some((abd.rc, abd.uEdge))
}

/// Move/resize the window to `r` only if it changed (avoids needless repaints
/// and fighting an in-progress drag).
fn apply_rect(hwnd: HWND, state: &mut WidgetState, r: RECT) {
    if state.last_rect == Some(r) {
        return;
    }
    let w = r.right - r.left;
    let h = r.bottom - r.top;
    // SAFETY: hwnd valid. SWP_NOACTIVATE keeps focus off the widget; HWND_TOPMOST
    // keeps it above normal windows.
    unsafe {
        let _ = windows::Win32::UI::WindowsAndMessaging::SetWindowPos(
            hwnd,
            windows::Win32::UI::WindowsAndMessaging::HWND_TOPMOST,
            r.left,
            r.top,
            w,
            h,
            windows::Win32::UI::WindowsAndMessaging::SWP_NOACTIVATE,
        );
        // Rounded corners: region radius scales with height.
        let radius = (h / 3).max(4);
        let rgn = CreateRoundRectRgn(0, 0, w + 1, h + 1, radius, radius);
        // SetWindowRgn takes ownership of the region.
        let _ = windows::Win32::UI::WindowsAndMessaging::SetWindowRgn(hwnd, rgn, true);
    }
    state.last_rect = Some(r);
}
```

(Add `SetWindowPos`, `HWND_TOPMOST`, `SWP_NOACTIVATE`, `SetWindowRgn`, `ShowWindow`, `SW_SHOW`, `SW_HIDE` to the `WindowsAndMessaging` imports; `CreateRoundRectRgn` is already imported from `Graphics::Gdi`.)

- [ ] **Step 2: Replace the `WM_TIMER` arm with full reconcile logic**

In `wndproc`, replace the Task-3 stub:

```rust
        WM_TIMER => {
            with_state(hwnd, |state| tick(hwnd, state));
            LRESULT(0)
        }
```

And add the `tick` function:

```rust
/// One timer tick: reconcile visibility, re-dock, repaint.
fn tick(hwnd: HWND, state: &mut WidgetState) {
    let want_visible = state
        .settings
        .read()
        .map(|g| g.widget_enabled)
        .unwrap_or(true);

    // Show/hide to match the setting.
    if want_visible != state.shown {
        let cmd = if want_visible {
            windows::Win32::UI::WindowsAndMessaging::SW_SHOW
        } else {
            windows::Win32::UI::WindowsAndMessaging::SW_HIDE
        };
        // SAFETY: hwnd valid. ShowWindow is safe to call here (owning thread).
        unsafe {
            let _ = windows::Win32::UI::WindowsAndMessaging::ShowWindow(hwnd, cmd);
        }
        state.shown = want_visible;
    }
    if !want_visible {
        return; // nothing to dock/paint while hidden
    }

    // Re-dock over the live taskbar (bottom edge only; else leave last position).
    if let Some((tb, edge)) = taskbar_rect() {
        if edge == ABE_BOTTOM {
            let offset = state.settings.read().map(|g| g.widget_offset_px).unwrap_or(0);
            let r = dock_rect(tb, offset);
            apply_rect(hwnd, state, r);
        }
    }

    // Repaint (updates countdown text + any fresh poll data).
    // SAFETY: hwnd valid.
    unsafe {
        let _ = windows::Win32::Graphics::Gdi::InvalidateRect(hwnd, None, false);
    }
}
```

- [ ] **Step 3: Add the gesture + drag arms to `wndproc`**

Add these arms before the `_ =>` catch-all:

```rust
        WM_NCHITTEST => {
            // Whole body acts as a drag handle.
            LRESULT(windows::Win32::UI::WindowsAndMessaging::HTCAPTION as isize)
        }
        WM_NCRBUTTONUP => {
            // Forward to the tray window's existing right-click handler (shows menu).
            with_state(hwnd, |state| {
                unsafe {
                    let _ = windows::Win32::UI::WindowsAndMessaging::PostMessageW(
                        state.tray_hwnd.0,
                        crate::tray::window::WM_APP_TRAYICON,
                        WPARAM(0),
                        LPARAM(windows::Win32::UI::WindowsAndMessaging::WM_RBUTTONUP as isize),
                    );
                }
            });
            LRESULT(0)
        }
        WM_NCLBUTTONDBLCLK => {
            // Forward to the tray window's left-click handler (opens dashboard).
            with_state(hwnd, |state| {
                unsafe {
                    let _ = windows::Win32::UI::WindowsAndMessaging::PostMessageW(
                        state.tray_hwnd.0,
                        crate::tray::window::WM_APP_TRAYICON,
                        WPARAM(0),
                        LPARAM(windows::Win32::UI::WindowsAndMessaging::WM_LBUTTONUP as isize),
                    );
                }
            });
            LRESULT(0)
        }
        WM_EXITSIZEMOVE => {
            // Drag finished: persist the new offset, then snap back into the band.
            on_drag_end(hwnd);
            LRESULT(0)
        }
```

And the `on_drag_end` helper (takes the HWND so it can read the final window position):

```rust
fn on_drag_end(hwnd: HWND) {
    // Current window position (screen coords).
    let mut r = RECT::default();
    // SAFETY: hwnd valid; r out-param.
    unsafe {
        let _ = windows::Win32::UI::WindowsAndMessaging::GetWindowRect(hwnd, &mut r);
    }
    let Some((tb, edge)) = taskbar_rect() else { return };
    if edge != ABE_BOTTOM {
        return;
    }
    let new_offset = offset_from_left(tb, r.left);
    with_state(hwnd, |state| {
        // Persist offset to shared settings + disk. Re-dock happens next tick.
        if let Ok(mut g) = state.settings.write() {
            g.widget_offset_px = new_offset;
            let to_save = g.clone();
            drop(g);
            if let Err(e) = crate::settings::save(&to_save) {
                tracing::warn!(error = %e, "failed to persist widget offset");
            }
        }
        // Force a redock next tick by clearing the cached rect.
        state.last_rect = None;
    });
}
```

(Add `WM_NCRBUTTONUP`, `WM_NCLBUTTONDBLCLK`, `WM_EXITSIZEMOVE`, `WM_RBUTTONUP`, `WM_LBUTTONUP`, `HTCAPTION`, `PostMessageW`, `GetWindowRect` to the imports. The borrow in the `settings.write()` block clones before `save` so the lock is released first.)

- [ ] **Step 4: Verify it compiles clean**

Run: `cargo build` then `cargo clippy --all-targets -- -D warnings`
Expected: both succeed, no warnings. Still not created/shown — wired in Task 5.

- [ ] **Step 5: Commit**

```powershell
git add src/tray/widget.rs
git commit -m "feat(widget): timer redock/show-hide, drag-offset persist, gesture forwarding"
```

---

### Task 5: Wire the widget into tray startup

**Files:**
- Modify: `src/tray/mod.rs:54-70` (create the widget after the tray window + icon)

- [ ] **Step 1: Create the widget after the tray icon is registered**

In `src/tray/mod.rs`, after `render_and_store_initial_icon(hwnd, &initial_tooltip)?;` (line 59) and before the `send_hwnd`/poller block (line 61), insert:

```rust
    // Create the docked taskbar widget (hidden until its first timer tick, which
    // shows it iff settings.widget_enabled). Failure is non-fatal — the tray icon
    // is the fallback surface.
    match widget::create(hinst, hwnd, shared.clone(), settings.clone()) {
        Ok(_whwnd) => tracing::info!("taskbar widget created"),
        Err(e) => tracing::warn!(error = %e, "failed to create taskbar widget; continuing"),
    }
```

(We don't keep the returned HWND: the widget owns its own lifecycle via its timer and is destroyed automatically when the message loop ends / process exits. `shared` and `settings` are already in scope as the `SharedSnapshot` / `SharedSettings` created earlier in `run`.)

- [ ] **Step 2: Build + first manual smoke check**

Run: `cargo build` then `cargo run`
Expected: the tray icon appears AND a small two-bar widget docks at the right of the taskbar within ~1 second, showing the current 5h/7d values. Quit via the tray menu. (Full verification is Task 8; this is a sanity check that creation + paint + dock work end-to-end.)

- [ ] **Step 3: Commit**

```powershell
git add src/tray/mod.rs
git commit -m "feat(widget): create the docked widget during tray startup"
```

---

### Task 6: Tray menu "Show taskbar widget" toggle

**Files:**
- Modify: `src/tray/window.rs` — `IDM_*` consts (~line 41), `WM_COMMAND` arm (~line 209-218), menu build (~line 515-541)

**Beginner note:** This item flips `settings.widget_enabled`, persists settings, and returns — the widget's own timer notices the change within ≤1 s and shows/hides itself. The checkmark is computed at menu-build time from the live setting.

- [ ] **Step 1: Add the command id**

After `pub const IDM_AUTOSTART: usize = 4;` (line 41):

```rust
/// Tray menu command id: toggle "show taskbar widget".
pub const IDM_WIDGET: usize = 5;
```

- [ ] **Step 2: Handle the command in `WM_COMMAND`**

After the `IDM_AUTOSTART` arm (ends ~line 218), add:

```rust
                id if id == IDM_WIDGET => {
                    with_state(hwnd, |state| {
                        if let Ok(mut g) = state.settings.write() {
                            g.widget_enabled = !g.widget_enabled;
                            let to_save = g.clone();
                            drop(g);
                            if let Err(e) = crate::settings::save(&to_save) {
                                tracing::warn!(error = %e, "failed to persist widget_enabled");
                            }
                        }
                    });
                }
```

- [ ] **Step 3: Add the checkable menu item**

In `show_context_menu`, add the label next to the others (after `let autostart_label = …`, ~line 516):

```rust
    let widget_label = encode_utf16("Show taskbar widget");
```

Then, inside the `unsafe` block, after the autostart `AppendMenuW` and before the `IDM_QUIT` append (~line 541):

```rust
        let widget_flags = MF_STRING
            | {
                let enabled = with_state_value(hwnd, |s| {
                    s.settings.read().map(|g| g.widget_enabled).unwrap_or(true)
                });
                if enabled { MF_CHECKED } else { MF_UNCHECKED }
            };
        let _ = AppendMenuW(hmenu, widget_flags, IDM_WIDGET, PCWSTR(widget_label.as_ptr()));
```

This needs a small read-only `with_state` that returns a value. Add near `with_state` (~line 243):

```rust
fn with_state_value<T, F: FnOnce(&TrayState) -> T>(hwnd: HWND, f: F, default: T) -> T {
    let state_ptr = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *mut TrayState;
    if state_ptr.is_null() {
        return default;
    }
    // SAFETY: pointer set by `create`; window is single-threaded.
    let state = unsafe { &*state_ptr };
    f(state)
}
```

And adjust the call site to pass the default:

```rust
        let widget_flags = MF_STRING
            | if with_state_value(hwnd, |s| s.settings.read().map(|g| g.widget_enabled).unwrap_or(true), true) {
                MF_CHECKED
            } else {
                MF_UNCHECKED
            };
```

- [ ] **Step 4: Verify it compiles clean**

Run: `cargo build` then `cargo clippy --all-targets -- -D warnings`
Expected: both succeed, no warnings.

- [ ] **Step 5: Commit**

```powershell
git add src/tray/window.rs
git commit -m "feat(widget): checkable Show-taskbar-widget item in tray menu"
```

---

### Task 7: Settings-tab "Show taskbar widget" checkbox

**Files:**
- Modify: `src/dashboard/settings_tab.rs` — insert a row near the auto-start row (~line 109-124) and preserve widget fields at Save (~line 141-144)

**Design note (matches the autostart checkbox):** This checkbox lives OUTSIDE the draft/Save grid. It reads + writes the **shared** settings directly and persists immediately, so a toggle takes effect within ≤1 s (the widget's timer). Because `widget_enabled`/`widget_offset_px` are also `Settings` fields, we preserve them from the shared store at Save time so a normal tz/weights Save never clobbers a value changed via this checkbox or via drag.

- [ ] **Step 1: Insert the widget checkbox after the auto-start row**

In `settings_tab.rs`, after the auto-start `ui.horizontal(|ui| { … });` block (ends ~line 124) and before `ui.add_space(16.0);` (line 126), insert:

```rust
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        // Reads + writes the shared store directly (outside draft/Save); the
        // widget's timer applies the change within ~1s.
        let mut enabled = shared.read().map(|g| g.widget_enabled).unwrap_or(true);
        if ui.checkbox(&mut enabled, "Show taskbar widget").changed() {
            if let Ok(mut g) = shared.write() {
                g.widget_enabled = enabled;
                let to_save = g.clone();
                drop(g);
                let _ = settings::save(&to_save);
            }
            // Keep the draft consistent so the Save button doesn't show it as dirty.
            draft.widget_enabled = enabled;
        }
    });
```

- [ ] **Step 2: Preserve widget fields when Save writes the draft**

In the Save `clicked()` block (lines 141-144), replace:

```rust
            if let Ok(mut g) = shared.write() {
                *g = draft.clone();
            }
            *save_msg = Some(settings::save(draft).map_err(|e| e.to_string()));
```

with a version that keeps the live widget fields:

```rust
            // Preserve the live widget fields (changed via the checkbox above or
            // via drag) so a tz/weights Save doesn't overwrite them with a stale draft.
            let mut to_save = draft.clone();
            if let Ok(g) = shared.read() {
                to_save.widget_enabled = g.widget_enabled;
                to_save.widget_offset_px = g.widget_offset_px;
            }
            // Mirror into the draft so the dirty check settles.
            draft.widget_enabled = to_save.widget_enabled;
            draft.widget_offset_px = to_save.widget_offset_px;
            if let Ok(mut g) = shared.write() {
                *g = to_save.clone();
            }
            *save_msg = Some(settings::save(&to_save).map_err(|e| e.to_string()));
```

- [ ] **Step 3: Verify it compiles clean**

Run: `cargo build` then `cargo clippy --all-targets -- -D warnings`
Expected: both succeed, no warnings. (`settings` and `shared`/`draft` are already in scope in `render`.)

- [ ] **Step 4: Commit**

```powershell
git add src/dashboard/settings_tab.rs
git commit -m "feat(widget): Show-taskbar-widget checkbox in Settings tab"
```

---

### Task 8: Manual GUI verification, version bump, docs, tag

**Files:**
- Modify: `Cargo.toml:3` (version), `Cargo.lock` (regenerated), `CLAUDE.md`

- [ ] **Step 1: Full verification build**

Run: `cargo fmt` then `cargo clippy --all-targets -- -D warnings` then `cargo test`
Expected: fmt idempotent; clippy clean; all tests pass (incl. the widget helper tests from Task 2 and the settings tests from Task 1).

- [ ] **Step 2: Manual GUI verification**

Run `cargo run` and verify each (record results in CLAUDE.md):
  1. Widget docks over the bottom taskbar, right side, on launch; two bars show the current 5h/7d util.
  2. The widget's percentages match the tray icon's number (icon shows the max of the two).
  3. Countdown timers decrement each second and match each bucket's reset.
  4. Force the `?` state (e.g. immediately after launch, before first poll) — both rows show `?`. If you can induce a 429, confirm `?` persists.
  5. Drag the widget left along the taskbar; release → it stays put and snaps within the band. Quit and relaunch → it returns to the dragged position (offset persisted).
  6. Right-click the widget → the tray context menu appears. Double-click → the dashboard opens/focuses.
  7. Tray menu "Show taskbar widget": untick → widget disappears within ~1 s; tick → reappears. The checkmark reflects state when the menu is reopened.
  8. Settings tab "Show taskbar widget" checkbox toggles the widget within ~1 s and agrees with the tray menu.
  9. On a 125%/150% display the widget is sized to the taskbar height and positioned correctly.
  10. Restart Explorer (`Stop-Process -Name explorer` then it auto-restarts) → the widget re-docks within ~1 s.

Expected: all pass. If the bars look cramped, tune `WIDTH_RATIO` / `pct_w` / `time_w` in `widget.rs` and rebuild (cosmetic only).

- [ ] **Step 3: Bump the version**

In `Cargo.toml`, line 3:

```toml
version = "1.2.0"
```

- [ ] **Step 4: Regenerate Cargo.lock and update CLAUDE.md**

Run `cargo build` so `Cargo.lock`'s own `version =` line updates. Then add a roadmap note in `CLAUDE.md` under "Post-1.0 polish": a `v1.2.0` entry referencing this spec + plan, summarizing the docked taskbar widget (new `src/tray/widget.rs`; self-driven 1s timer; reads the shared snapshot; drag-docked with persisted `widget_offset_px`; toggle from tray menu + Settings; tray icon kept). Note manual GUI verification done.

- [ ] **Step 5: Commit and tag**

```powershell
git add Cargo.toml Cargo.lock CLAUDE.md
git commit -m "chore: bump to v1.2.0 (docked taskbar widget)"
git tag v1.2.0
```

(Push is left to the user, consistent with prior releases. The `design/taskbar-widget-mockups` branch can be deleted once merged/abandoned.)

---

## Self-Review

**Spec coverage:**
- New `src/tray/widget.rs`, borderless `WS_POPUP` + `WS_EX_TOPMOST|WS_EX_TOOLWINDOW`, rounded via `SetWindowRgn`, on the existing message loop → Tasks 2-5. ✓
- Two stacked rows (5h/7d): label · bar · % · countdown; Variant A layout → Task 3 paint. ✓
- Raw API util via shared snapshot; countdowns from `resets_at` → Task 3 (`row_state`, paint). ✓
- Color via `anchored_gradient`; `?`/`!` status special-cases matching `compute_visual` → Task 2 `row_state` + Task 3 paint. ✓
- Auto-dock over the bottom taskbar via `SHAppBarMessage`; drag (`HTCAPTION`) + clamp + persisted offset → Task 4. ✓ (Re-dock-every-tick replaces explicit `TaskbarCreated`/`WM_DPICHANGED`; DPI handled by taskbar-relative sizing — see Deviations.)
- Right-click → tray menu; double-click → dashboard (reused via forwarding) → Task 4. ✓
- Settings `widget_enabled` (default true) + `widget_offset_px`, validate/round-trip → Task 1. ✓
- Tray menu checkable "Show taskbar widget" → Task 6. ✓
- Settings-tab checkbox, live-apply, Save preserves widget fields → Task 7. ✓
- Tray icon kept (no change to `icon.rs` behavior; widget created alongside) → Task 5. ✓
- Unit tests (fill width, dock rect, offset inverse, row state) + manual GUI checklist → Tasks 2, 8. ✓
- Non-goals (vertical/multi-monitor taskbars, Codex bars, layered alpha) → not implemented; bottom-edge-only guard in `tick`/`on_drag_end`. ✓

**Placeholder scan:** No TBD/TODO/"handle edge cases" — every step has concrete code or exact commands.

**Type consistency:** `dock_rect(RECT, i32) -> RECT`, `offset_from_left(RECT, i32) -> i32`, `bar_fill_width(i32, f64) -> i32`, `row_state(&LastStatus, Option<&UsageBucket>, DateTime<Utc>) -> RowState`, `RowState::{Data{util,pct,countdown}, Bang{countdown}, Question}`, `WidgetState{tray_hwnd, shared, settings, last_rect, shown}`, `widget::create(HMODULE, HWND, SharedSnapshot, SharedSettings) -> Result<HWND>`, `IDM_WIDGET: usize = 5`, settings fields `widget_enabled: bool` / `widget_offset_px: i32` — names used identically across tasks. `anchored_gradient` and `compute_visual` are the existing `icon.rs` names.

**Note for the implementer:** the GDI+ import lists in Task 3/4 are best-effort against `windows` 0.58; if a symbol's module path differs, `src/tray/icon.rs` is the compiling reference for every GDI+ name reused here. Resolve such mismatches at the Task-3/4 compile checkpoints.
