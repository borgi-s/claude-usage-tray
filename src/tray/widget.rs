//! Borderless, always-on-top widget docked over the Windows taskbar. Shows two
//! usage bars (5h + 7d) with live % and reset countdowns. Painted with GDI+
//! (mirrors `crate::tray::icon`); self-driven by a 1-second WM_TIMER that
//! repaints, re-docks over the live taskbar rect, and shows/hides to match
//! `settings.widget_enabled`.

use crate::api::usage::UsageBucket;
use crate::render::LastStatus;
use chrono::{DateTime, Utc};
use windows::Win32::Foundation::RECT;

use crate::shared::{SharedSettings, SharedSnapshot};
use crate::tray::poller::SendHwnd;
use anyhow::{anyhow, Result};
use windows::core::PCWSTR;
use windows::Win32::Foundation::{HMODULE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, CreateRoundRectRgn, EndPaint, InvalidateRect, SetWindowRgn, HDC, PAINTSTRUCT,
};
use windows::Win32::Graphics::GdiPlus::{
    FontStyleBold, GdipCreateBitmapFromScan0, GdipCreateFont, GdipCreateFontFamilyFromName,
    GdipCreateFromHDC, GdipCreateSolidFill, GdipCreateStringFormat, GdipDeleteBrush,
    GdipDeleteFont, GdipDeleteFontFamily, GdipDeleteGraphics, GdipDeleteStringFormat,
    GdipDisposeImage, GdipDrawImageRectI, GdipDrawString, GdipFillRectangleI,
    GdipGetImageGraphicsContext, GdipGraphicsClear, GdipSetStringFormatAlign,
    GdipSetStringFormatLineAlign, GdipSetTextRenderingHint, GpBitmap, GpBrush, GpFont,
    GpFontFamily, GpGraphics, GpImage, GpSolidFill, GpStringFormat, RectF, Status,
    StringAlignmentCenter, StringAlignmentNear, TextRenderingHintSingleBitPerPixelGridFit,
    UnitPixel,
};
use windows::Win32::UI::Shell::{SHAppBarMessage, ABM_GETTASKBARPOS, APPBARDATA};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, GetClientRect, GetWindowLongPtrW, GetWindowRect,
    PostMessageW, RegisterClassExW, SetTimer, SetWindowLongPtrW, SetWindowPos, ShowWindow,
    CREATESTRUCTW, GWLP_USERDATA, HMENU, HTCAPTION, HWND_TOPMOST, SWP_NOACTIVATE, SW_HIDE, SW_SHOW,
    WM_EXITSIZEMOVE, WM_LBUTTONUP, WM_NCCREATE, WM_NCDESTROY, WM_NCHITTEST, WM_NCLBUTTONDBLCLK,
    WM_NCRBUTTONUP, WM_PAINT, WM_RBUTTONUP, WM_TIMER, WNDCLASSEXW, WS_EX_TOOLWINDOW, WS_EX_TOPMOST,
    WS_POPUP,
};

// 0xFF000000 | 32bpp ARGB — same constant icon.rs documents.
const PIXEL_FORMAT_32BPP_ARGB: i32 = 0x0026_200A;

/// Timer id for the 1-second repaint/redock/visibility tick.
const TIMER_ID: usize = 1;

/// Window class name (UTF-16, null-terminated).
const WIDGET_CLASS: PCWSTR = windows::core::w!("claude-usage-tray.widget");

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
                SetTimer(h, TIMER_ID, 1000, None);
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

/// `uEdge` value returned by `ABM_GETTASKBARPOS` when the taskbar sits on the
/// bottom edge of the screen. We only first-class this case; vertical/top/left
/// taskbars are explicit non-goals (the widget keeps its last position).
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
        let _ = SetWindowPos(hwnd, HWND_TOPMOST, r.left, r.top, w, h, SWP_NOACTIVATE);
        // Rounded corners: region radius scales with height.
        let radius = (h / 3).max(4);
        let rgn = CreateRoundRectRgn(0, 0, w + 1, h + 1, radius, radius);
        // SetWindowRgn takes ownership of the region.
        let _ = SetWindowRgn(hwnd, rgn, true);
    }
    state.last_rect = Some(r);
}

/// One timer tick: reconcile visibility, re-dock, repaint.
fn tick(hwnd: HWND, state: &mut WidgetState) {
    let want_visible = state
        .settings
        .read()
        .map(|g| g.widget_enabled)
        .unwrap_or(true);

    // Show/hide to match the setting.
    if want_visible != state.shown {
        let cmd = if want_visible { SW_SHOW } else { SW_HIDE };
        // SAFETY: hwnd valid. ShowWindow is safe to call here (owning thread).
        unsafe {
            let _ = ShowWindow(hwnd, cmd);
        }
        state.shown = want_visible;
    }
    if !want_visible {
        return; // nothing to dock/paint while hidden
    }

    // Re-dock over the live taskbar (bottom edge only; else leave last position).
    if let Some((tb, edge)) = taskbar_rect() {
        if edge == ABE_BOTTOM {
            let offset = state
                .settings
                .read()
                .map(|g| g.widget_offset_px)
                .unwrap_or(0);
            let r = dock_rect(tb, offset);
            apply_rect(hwnd, state, r);
        }
    }

    // Repaint (updates countdown text + any fresh poll data).
    // SAFETY: hwnd valid.
    unsafe {
        let _ = InvalidateRect(hwnd, None, false);
    }
}

/// Drag finished: persist the new offset, then force a re-dock next tick.
fn on_drag_end(hwnd: HWND) {
    // Current window position (screen coords).
    let mut r = RECT::default();
    // SAFETY: hwnd valid; r out-param.
    unsafe {
        let _ = GetWindowRect(hwnd, &mut r);
    }
    let Some((tb, edge)) = taskbar_rect() else {
        return;
    };
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
            with_state(hwnd, |state| tick(hwnd, state));
            LRESULT(0)
        }
        WM_NCHITTEST => {
            // Whole body acts as a drag handle.
            LRESULT(HTCAPTION as isize)
        }
        WM_NCRBUTTONUP => {
            // Forward to the tray window's existing right-click handler (shows menu).
            with_state(hwnd, |state| unsafe {
                let _ = PostMessageW(
                    state.tray_hwnd.0,
                    crate::tray::window::WM_APP_TRAYICON,
                    WPARAM(0),
                    LPARAM(WM_RBUTTONUP as isize),
                );
            });
            LRESULT(0)
        }
        WM_NCLBUTTONDBLCLK => {
            // Forward to the tray window's left-click handler (opens dashboard).
            with_state(hwnd, |state| unsafe {
                let _ = PostMessageW(
                    state.tray_hwnd.0,
                    crate::tray::window::WM_APP_TRAYICON,
                    WPARAM(0),
                    LPARAM(WM_LBUTTONUP as isize),
                );
            });
            LRESULT(0)
        }
        WM_EXITSIZEMOVE => {
            // Drag finished: persist the new offset, then snap back into the band.
            on_drag_end(hwnd);
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

/// Paint the widget: off-screen GDI+ bitmap, then one blit to the window DC.
fn paint(hwnd: HWND, state: &WidgetState) {
    // Window client size.
    let mut rc = RECT::default();
    // SAFETY: hwnd valid; rc out-param.
    unsafe {
        let _ = GetClientRect(hwnd, &mut rc);
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
    let s =
        unsafe { GdipCreateBitmapFromScan0(w, h, 0, PIXEL_FORMAT_32BPP_ARGB, None, &mut bitmap) };
    if s != Status(0) {
        anyhow::bail!("GdipCreateBitmapFromScan0 failed: {s:?}");
    }
    let mut g: *mut GpGraphics = std::ptr::null_mut();
    let s = unsafe { GdipGetImageGraphicsContext(bitmap as *mut GpImage, &mut g) };
    if s != Status(0) {
        unsafe { GdipDisposeImage(bitmap as *mut GpImage) };
        anyhow::bail!("GdipGetImageGraphicsContext failed: {s:?}");
    }

    // 2) Panel background (#26282b opaque).
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
        ("5h", row_state(&status, five.as_ref(), now)),
        ("7d", row_state(&status, seven.as_ref(), now)),
    ];

    // Reusable white text brush + font family.
    let font_name: Vec<u16> = "Segoe UI"
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
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
        draw_text(
            g,
            label,
            gray as *mut GpBrush,
            font,
            pad,
            row_y,
            label_w,
            row_h,
            false,
        );

        // Track (dark) — always drawn.
        let mut track: *mut GpSolidFill = std::ptr::null_mut();
        unsafe { GdipCreateSolidFill(0xFF34_363Bu32, &mut track) };
        unsafe { GdipFillRectangleI(g, track as *mut GpBrush, bar_x, bar_y, bar_w, bar_h) };
        unsafe { GdipDeleteBrush(track as *mut GpBrush) };

        match rs {
            RowState::Data {
                util,
                pct,
                countdown,
            } => {
                let (r, gg, b) = crate::tray::icon::anchored_gradient(*util);
                let argb =
                    0xFF00_0000u32 | (u32::from(r) << 16) | (u32::from(gg) << 8) | u32::from(b);
                let fill_w = bar_fill_width(bar_w, *util);
                let mut fill: *mut GpSolidFill = std::ptr::null_mut();
                unsafe { GdipCreateSolidFill(argb, &mut fill) };
                unsafe { GdipFillRectangleI(g, fill as *mut GpBrush, bar_x, bar_y, fill_w, bar_h) };
                unsafe { GdipDeleteBrush(fill as *mut GpBrush) };
                draw_text(
                    g,
                    &format!("{pct}%"),
                    white as *mut GpBrush,
                    font,
                    bar_x + bar_w,
                    row_y,
                    pct_w,
                    row_h,
                    false,
                );
                if let Some(cd) = countdown {
                    draw_text(
                        g,
                        cd,
                        gray as *mut GpBrush,
                        font,
                        bar_x + bar_w + pct_w,
                        row_y,
                        time_w,
                        row_h,
                        false,
                    );
                }
            }
            RowState::Bang { countdown } => {
                let mut fill: *mut GpSolidFill = std::ptr::null_mut();
                unsafe { GdipCreateSolidFill(0xFFCC_2929u32, &mut fill) };
                unsafe { GdipFillRectangleI(g, fill as *mut GpBrush, bar_x, bar_y, bar_w, bar_h) };
                unsafe { GdipDeleteBrush(fill as *mut GpBrush) };
                draw_text(
                    g,
                    "!",
                    white as *mut GpBrush,
                    font,
                    bar_x + bar_w,
                    row_y,
                    pct_w,
                    row_h,
                    false,
                );
                if let Some(cd) = countdown {
                    draw_text(
                        g,
                        cd,
                        gray as *mut GpBrush,
                        font,
                        bar_x + bar_w + pct_w,
                        row_y,
                        time_w,
                        row_h,
                        false,
                    );
                }
            }
            RowState::Question => {
                draw_text(
                    g,
                    "?",
                    gray as *mut GpBrush,
                    font,
                    bar_x + bar_w,
                    row_y,
                    pct_w,
                    row_h,
                    false,
                );
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
    let align = if center {
        StringAlignmentCenter
    } else {
        StringAlignmentNear
    };
    unsafe { GdipSetStringFormatAlign(fmt, align) };
    unsafe { GdipSetStringFormatLineAlign(fmt, StringAlignmentCenter) };
    let layout = RectF {
        X: x as f32,
        Y: y as f32,
        Width: w as f32,
        Height: h as f32,
    };
    unsafe { GdipDrawString(g, PCWSTR(utf16.as_ptr()), len, font, &layout, fmt, brush) };
    unsafe { GdipDeleteStringFormat(fmt) };
}

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
        RECT {
            left: l,
            top: t,
            right: r,
            bottom: b,
        }
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
        assert!(matches!(
            row_state(&LastStatus::Ok, Some(&b), now),
            RowState::Bang { .. }
        ));
    }

    #[test]
    fn row_state_rate_limited_is_question_even_with_bucket() {
        let now = Utc::now();
        let b = bucket(0.5, Some(60), now);
        assert_eq!(
            row_state(&LastStatus::RateLimited, Some(&b), now),
            RowState::Question
        );
    }

    #[test]
    fn row_state_missing_bucket_is_question() {
        let now = Utc::now();
        assert_eq!(row_state(&LastStatus::Ok, None, now), RowState::Question);
    }
}
