use crate::api::usage::UsageSnapshot;
use crate::calibration::anchors::DerivedCaps;
use crate::calibration::live::LiveUtil;
use crate::render::{format_duration, LastStatus};
use crate::tray::icon::{self, IconRenderer};
use crate::tray::poller::{PollEvent, WM_APP_POLL};
use crate::updater::{self, ReleaseInfo, UpdateEvent};
use anyhow::{anyhow, Result};
use chrono::{DateTime, Duration as ChronoDuration, Local, Utc};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::Arc;
use std::time::Instant;
use windows::core::PCWSTR;
use windows::Win32::Foundation::{HMODULE, HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu, DestroyWindow,
    DispatchMessageW, GetCursorPos, GetMessageW, GetWindowLongPtrW, PostMessageW, PostQuitMessage,
    RegisterClassExW, SetForegroundWindow, SetWindowLongPtrW, TrackPopupMenu, TranslateMessage,
    CREATESTRUCTW, CW_USEDEFAULT, GWLP_USERDATA, HICON, HMENU, HWND_MESSAGE, MF_CHECKED,
    MF_SEPARATOR, MF_STRING, MF_UNCHECKED, MSG, SW_SHOWNORMAL, TPM_LEFTBUTTON, TPM_RIGHTBUTTON,
    WINDOW_EX_STYLE, WINDOW_STYLE,
    WM_APP, WM_COMMAND, WM_DESTROY, WM_LBUTTONUP, WM_NCCREATE, WM_NCDESTROY, WM_RBUTTONUP,
    WNDCLASSEXW,
};

/// Custom message: shell sends this when the user interacts with the tray icon.
pub const WM_APP_TRAYICON: u32 = WM_APP + 2;

/// Custom message: an update-checking thread posted a new UpdateEvent.
pub const WM_APP_UPDATE: u32 = WM_APP + 3;

/// Quit menu item ID.
pub const IDM_QUIT: usize = 1;
/// "Update available" menu item ID.
pub const IDM_UPDATE: usize = 2;
/// "Check for updates now" menu item ID.
pub const IDM_CHECK_UPDATES: usize = 3;
/// Tray menu command id: toggle "start at login".
pub const IDM_AUTOSTART: usize = 4;

/// Window class name (UTF-16, null-terminated).
const CLASS_NAME: &[u16] = &[
    'c' as u16, 'l' as u16, 'a' as u16, 'u' as u16, 'd' as u16, 'e' as u16, '-' as u16, 'u' as u16,
    's' as u16, 'a' as u16, 'g' as u16, 'e' as u16, '-' as u16, 't' as u16, 'r' as u16, 'a' as u16,
    'y' as u16, '.' as u16, 't' as u16, 'r' as u16, 'a' as u16, 'y' as u16, 0,
];

/// State carried inside the window via GWLP_USERDATA.
pub struct TrayState {
    pub last_sample: Option<(UsageSnapshot, DateTime<Utc>)>,
    pub last_status: LastStatus,
    pub renderer: IconRenderer,
    pub current_hicon: Option<HICON>,
    pub rx: Receiver<PollEvent>,
    pub shutdown: Arc<AtomicBool>,
    pub last_caps: Option<DerivedCaps>,
    pub last_local_util: Option<LiveUtil>,
    pub last_hourly_5h: Option<[f64; 24]>,
    pub last_hourly_week: Option<[f64; 24]>,
    pub shared: crate::shared::SharedSnapshot,
    pub settings: crate::shared::SharedSettings,
    pub dashboard: std::sync::Arc<std::sync::Mutex<Option<crate::dashboard::DashboardHandle>>>,
    pub update_rx: Receiver<UpdateEvent>,
    pub update_tx: Sender<UpdateEvent>,
    pub available_update: Option<ReleaseInfo>,
    pub manual_check_history: Vec<Instant>,
}

impl Drop for TrayState {
    fn drop(&mut self) {
        if let Some(h) = self.current_hicon.take() {
            // SAFETY: we own this handle (set by drain_and_redraw / the initial render).
            unsafe {
                let _ = windows::Win32::UI::WindowsAndMessaging::DestroyIcon(h);
            }
        }
    }
}

/// Register the class (idempotent — second call returns ALREADY_EXISTS, fine).
fn register_class(hinst: HMODULE) -> Result<()> {
    let wc = WNDCLASSEXW {
        cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
        lpfnWndProc: Some(wndproc),
        hInstance: hinst.into(),
        lpszClassName: PCWSTR(CLASS_NAME.as_ptr()),
        ..Default::default()
    };
    // SAFETY: wc is on the stack and lives for the duration of the call.
    let atom = unsafe { RegisterClassExW(&wc) };
    if atom == 0 {
        // 1410 = ERROR_CLASS_ALREADY_EXISTS — fine in repeat invocations of the same binary.
        let err = unsafe { windows::Win32::Foundation::GetLastError() };
        if err.0 == 1410 {
            return Ok(());
        }
        return Err(anyhow!("RegisterClassExW failed: {err:?}"));
    }
    Ok(())
}

/// Create the hidden message-only window. The window's GWLP_USERDATA slot is
/// populated with `Box::into_raw(state)` so WndProc can read it.
pub fn create(hinst: HMODULE, state: Box<TrayState>) -> Result<HWND> {
    register_class(hinst)?;

    // Leak the Box so we can recover it via Box::from_raw on WM_NCDESTROY.
    let state_ptr = Box::into_raw(state);

    // SAFETY: classname is null-terminated; all other args are valid for HWND_MESSAGE.
    // In windows-0.58+, CreateWindowExW returns Result<HWND>.
    let hwnd = unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE(0),
            PCWSTR(CLASS_NAME.as_ptr()),
            PCWSTR(CLASS_NAME.as_ptr()), // window name; unused for message-only
            WINDOW_STYLE(0),
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            HWND_MESSAGE,
            HMENU::default(),
            hinst,
            Some(state_ptr.cast()),
        )
    };

    match hwnd {
        Ok(h) => Ok(h),
        Err(e) => {
            // Reclaim and drop the leaked Box so we don't leak state on the error path.
            unsafe { drop(Box::from_raw(state_ptr)) };
            Err(anyhow!("CreateWindowExW failed: {e}"))
        }
    }
}

/// Run the Win32 message loop. Returns when WM_QUIT is received.
pub fn message_loop() {
    let mut msg = MSG::default();
    // SAFETY: msg lives on the stack across the loop.
    while unsafe { GetMessageW(&mut msg, HWND::default(), 0, 0).as_bool() } {
        unsafe {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_NCCREATE => {
            // lparam carries the CREATESTRUCTW; stash lpCreateParams into GWLP_USERDATA.
            // SAFETY: WM_NCCREATE's lparam is always a *const CREATESTRUCTW.
            let cs = unsafe { &*(lparam.0 as *const CREATESTRUCTW) };
            let state_ptr = cs.lpCreateParams as isize;
            unsafe {
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, state_ptr);
            }
            // Continue default processing.
            unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
        }
        WM_APP_POLL => {
            with_state(hwnd, |state| {
                drain_and_redraw(hwnd, state);
            });
            LRESULT(0)
        }
        WM_APP_TRAYICON => {
            // lparam.0 carries the underlying mouse event id.
            if lparam.0 as u32 == WM_RBUTTONUP {
                show_context_menu(hwnd);
            } else if lparam.0 as u32 == WM_LBUTTONUP {
                with_state(hwnd, on_left_click);
            }
            LRESULT(0)
        }
        WM_APP_UPDATE => {
            with_state(hwnd, |state| drain_update_events(hwnd, state));
            LRESULT(0)
        }
        WM_COMMAND => {
            match wparam.0 & 0xFFFF {
                id if id == IDM_QUIT => {
                    with_state(hwnd, |state| {
                        state.shutdown.store(true, Ordering::Relaxed);

                        // Tell the dashboard thread (if any) to close its viewport
                        // so eframe::run_native returns and the thread can join.
                        // request_quit also wakes the (possibly hidden) event loop.
                        if let Some(handle) = state.dashboard.lock().unwrap().as_ref() {
                            handle.signals.request_quit();
                        }
                    });
                    icon::delete(hwnd);
                    unsafe {
                        let _ = DestroyWindow(hwnd);
                    }
                }
                id if id == IDM_UPDATE => {
                    with_state(hwnd, open_release_page);
                }
                id if id == IDM_CHECK_UPDATES => {
                    with_state(hwnd, |state| trigger_manual_check(hwnd, state));
                }
                id if id == IDM_AUTOSTART => {
                    let res = if crate::autostart::is_enabled() {
                        crate::autostart::disable()
                    } else {
                        crate::autostart::enable()
                    };
                    if let Err(e) = res {
                        tracing::warn!(error = %e, "failed to toggle auto-start");
                    }
                }
                _ => {}
            }
            LRESULT(0)
        }
        WM_DESTROY => {
            unsafe { PostQuitMessage(0) };
            LRESULT(0)
        }
        WM_NCDESTROY => {
            // Reclaim and drop the Box that GWLP_USERDATA points to.
            let state_ptr = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *mut TrayState;
            if !state_ptr.is_null() {
                // SAFETY: we set this pointer ourselves via Box::into_raw in `create`.
                unsafe { drop(Box::from_raw(state_ptr)) };
                unsafe {
                    SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                }
            }
            unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

fn with_state<F: FnOnce(&mut TrayState)>(hwnd: HWND, f: F) {
    let state_ptr = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *mut TrayState;
    if !state_ptr.is_null() {
        // SAFETY: pointer is set by `create` to a leaked Box<TrayState>; it lives
        // until WM_NCDESTROY. The window is single-threaded so no aliasing.
        let state = unsafe { &mut *state_ptr };
        f(state);
    }
}

fn drain_and_redraw(hwnd: HWND, state: &mut TrayState) {
    // Drain all queued events, keeping the most recent.
    while let Ok(event) = state.rx.try_recv() {
        match event {
            PollEvent::Ok { snap, calib } => {
                let calib = *calib;
                state.last_sample = Some((snap, Utc::now()));
                state.last_status = LastStatus::Ok;
                state.last_caps = Some(calib.caps);
                state.last_local_util = Some(calib.live);
                state.last_hourly_5h = Some(calib.hourly_5h);
                state.last_hourly_week = Some(calib.hourly_week);
            }
            PollEvent::RateLimited => {
                state.last_status = LastStatus::RateLimited;
            }
            PollEvent::Error(msg) => {
                state.last_status = LastStatus::Error(msg);
            }
        }
    }

    let sample = state.last_sample.as_ref().map(|(s, _)| s);

    // Render a fresh HICON for the current state.
    let next_hicon = match state.renderer.render(&state.last_status, sample) {
        Ok(h) => h,
        Err(e) => {
            tracing::warn!(error = %e, "IconRenderer::render failed, keeping previous icon");
            // Still refresh the tooltip — text-only update.
            let tooltip = format_tooltip(
                &state.last_status,
                state.last_sample.as_ref(),
                state.last_local_util.as_ref(),
                Utc::now(),
            );
            if let Some(current) = state.current_hicon {
                icon::modify(hwnd, WM_APP_TRAYICON, current, &tooltip);
            }
            return;
        }
    };

    let tooltip = format_tooltip(
        &state.last_status,
        state.last_sample.as_ref(),
        state.last_local_util.as_ref(),
        Utc::now(),
    );
    icon::modify(hwnd, WM_APP_TRAYICON, next_hicon, &tooltip);

    // Swap in the new HICON. Destroy the previous one (if any) only AFTER NIM_MODIFY
    // has been called with the new one — otherwise the shell might briefly point at a freed handle.
    if let Some(prev) = state.current_hicon.replace(next_hicon) {
        // SAFETY: previous handle we owned; no longer referenced by the shell after the
        // NIM_MODIFY call above.
        unsafe {
            let _ = windows::Win32::UI::WindowsAndMessaging::DestroyIcon(prev);
        }
    }
}

/// Drain UpdateEvents posted by a checking thread; update menu state + balloons.
/// The UI thread never touches state.json — `notify` was decided by the sender.
fn drain_update_events(hwnd: HWND, state: &mut TrayState) {
    while let Ok(ev) = state.update_rx.try_recv() {
        match ev {
            UpdateEvent::Result { check, notify } => {
                if check.is_newer {
                    state.available_update = Some(check.latest.clone());
                } else {
                    // No longer newer (e.g. upgraded in place, or release yanked) —
                    // drop the stale menu item.
                    state.available_update = None;
                }
                if notify {
                    if check.is_newer {
                        let body =
                            format!("v{} \u{00b7} click the tray to open", check.latest.version);
                        icon::show_balloon(hwnd, "Update available", &body);
                    } else {
                        let body = format!("You're up to date (v{})", env!("CARGO_PKG_VERSION"));
                        icon::show_balloon(hwnd, "Claude usage tray", &body);
                    }
                }
            }
            UpdateEvent::Failed { manual, msg } => {
                if manual {
                    icon::show_balloon(hwnd, "Claude usage tray", "Update check failed");
                }
                tracing::warn!(error = %msg, "update check failed");
            }
        }
    }
}

/// Open the stored release page in the default browser via ShellExecuteW.
fn open_release_page(state: &mut TrayState) {
    let Some(release) = state.available_update.as_ref() else {
        return;
    };
    let url: Vec<u16> = release
        .html_url
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    let verb: Vec<u16> = "open".encode_utf16().chain(std::iter::once(0)).collect();
    // SAFETY: both buffers are null-terminated and live for the call; null hwnd/params/dir are valid.
    let ret = unsafe {
        ShellExecuteW(
            None,
            PCWSTR(verb.as_ptr()),
            PCWSTR(url.as_ptr()),
            PCWSTR::null(),
            PCWSTR::null(),
            SW_SHOWNORMAL,
        )
    };
    // ShellExecuteW returns an HINSTANCE; a value <= 32 indicates failure.
    if ret.0 as usize <= 32 {
        tracing::warn!(url = %release.html_url, "ShellExecuteW failed to open release page");
    }
}

/// Enforce the manual-check rate limit, then spawn a one-shot checking thread.
fn trigger_manual_check(hwnd: HWND, state: &mut TrayState) {
    let now = Instant::now();
    if !updater::manual_check_allowed(&mut state.manual_check_history, now) {
        icon::show_balloon(
            hwnd,
            "Claude usage tray",
            "Update-check limit reached \u{2014} try again later",
        );
        return;
    }
    let tx = state.update_tx.clone();
    let send_hwnd = crate::tray::poller::SendHwnd(hwnd);
    // This detached thread may outlive TrayState if the user quits mid-check;
    // its tx.send / PostMessageW then fail harmlessly (Err/FALSE, both ignored).
    std::thread::spawn(move || {
        // Force the closure to capture the whole `SendHwnd` (which is `Send`)
        // rather than the inner non-Send `HWND` via disjoint capture.
        let send_hwnd = send_hwnd;
        let current = updater::current_version();
        let ev = match updater::check_latest(&current) {
            Ok(check) => UpdateEvent::Result {
                check,
                notify: true,
            },
            Err(e) => UpdateEvent::Failed {
                manual: true,
                msg: e.to_string(),
            },
        };
        let _ = tx.send(ev);
        // SAFETY: PostMessageW is thread-safe; the HWND is valid until shutdown.
        unsafe {
            let _ = PostMessageW(send_hwnd.0, WM_APP_UPDATE, WPARAM(0), LPARAM(0));
        }
    });
}

/// Format the tooltip text (UTF-16, null-terminated, <=127 chars per szTip cap).
pub(crate) fn format_tooltip(
    status: &LastStatus,
    last_sample: Option<&(UsageSnapshot, DateTime<Utc>)>,
    local: Option<&LiveUtil>,
    now: DateTime<Utc>,
) -> Vec<u16> {
    let text = match (last_sample, status) {
        (None, LastStatus::Initial) => "Claude usage tray\nfetching\u{2026}".to_string(),
        (None, LastStatus::RateLimited) => {
            "5h: --   7d: --\nno data yet (rate-limited)".to_string()
        }
        (None, LastStatus::Error(msg)) => {
            format!("5h: --   7d: --\nno data yet ({})", short(msg))
        }
        (None, LastStatus::Ok) => "Claude usage tray\nfetching\u{2026}".to_string(),
        (Some((snap, sample_at)), st) => {
            let h5 = snap
                .five_hour
                .as_ref()
                .map(|b| format!("{}%", (b.utilization * 100.0).round() as i64))
                .unwrap_or_else(|| "--".to_string());
            let d7 = snap
                .seven_day
                .as_ref()
                .map(|b| format!("{}%", (b.utilization * 100.0).round() as i64))
                .unwrap_or_else(|| "--".to_string());
            let updated = sample_at.with_timezone(&Local).format("%H:%M");
            let footer = match st {
                LastStatus::Ok => "(Ok)".to_string(),
                LastStatus::Initial => "(fetching)".to_string(),
                LastStatus::RateLimited => format!(
                    "(stale {})",
                    format_duration(ChronoDuration::seconds(
                        (now - *sample_at).num_seconds().max(0)
                    ))
                ),
                LastStatus::Error(msg) => format!("(error: {})", short(msg)),
            };
            let local_line = format_local_line(local);
            format!("5h: {h5}   7d: {d7}\n{local_line}\nupdated {updated} {footer}")
        }
    };
    encode_utf16(&text)
}

fn format_local_line(local: Option<&LiveUtil>) -> String {
    match local {
        None => "local: (uncalibrated)".to_string(),
        Some(l) => {
            let f = |u: Option<f64>| match u {
                Some(v) => format!("{}%", (v * 100.0).round() as i64),
                None => "(uncalibrated)".to_string(),
            };
            format!("local 5h: {}   local 7d: {}", f(l.util_5h), f(l.util_week))
        }
    }
}

fn short(msg: &str) -> String {
    if msg.len() > 32 {
        format!("{}\u{2026}", &msg[..32])
    } else {
        msg.to_string()
    }
}

fn encode_utf16(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn show_context_menu(hwnd: HWND) {
    // GetCursorPos
    let mut pt = POINT::default();
    unsafe {
        let _ = GetCursorPos(&mut pt);
    }

    // Required Win32 idiom so menu dismisses on click-away.
    unsafe {
        let _ = SetForegroundWindow(hwnd);
    }

    // Create + populate + show.
    let hmenu: HMENU = unsafe { CreatePopupMenu() }.unwrap_or_default();
    if hmenu.0.is_null() {
        tracing::warn!("CreatePopupMenu failed");
        return;
    }
    // Read what we need from state up front (the menu is built outside `with_state`).
    let mut update_label: Option<Vec<u16>> = None;
    with_state(hwnd, |state| {
        if let Some(rel) = state.available_update.as_ref() {
            update_label = Some(encode_utf16(&format!(
                "Update available \u{2014} v{}",
                rel.version
            )));
        }
    });

    let check_label = encode_utf16("Check for updates now");
    let autostart_label = encode_utf16("Start on login");
    let quit_label = encode_utf16("Quit");
    unsafe {
        if let Some(label) = update_label.as_ref() {
            let _ = AppendMenuW(hmenu, MF_STRING, IDM_UPDATE, PCWSTR(label.as_ptr()));
            let _ = AppendMenuW(hmenu, MF_SEPARATOR, 0, PCWSTR::null());
        }
        let _ = AppendMenuW(
            hmenu,
            MF_STRING,
            IDM_CHECK_UPDATES,
            PCWSTR(check_label.as_ptr()),
        );
        let autostart_flags = MF_STRING
            | if crate::autostart::is_enabled() {
                MF_CHECKED
            } else {
                MF_UNCHECKED
            };
        let _ = AppendMenuW(
            hmenu,
            autostart_flags,
            IDM_AUTOSTART,
            PCWSTR(autostart_label.as_ptr()),
        );
        let _ = AppendMenuW(hmenu, MF_STRING, IDM_QUIT, PCWSTR(quit_label.as_ptr()));
        let _ = TrackPopupMenu(
            hmenu,
            TPM_RIGHTBUTTON | TPM_LEFTBUTTON,
            pt.x,
            pt.y,
            0,
            hwnd,
            None,
        );
        let _ = DestroyMenu(hmenu);
    }
}

/// Handler for left-click on the tray icon.
fn on_left_click(state: &mut TrayState) {
    let mut guard = state.dashboard.lock().unwrap();
    match guard.as_ref() {
        Some(handle) if !handle.join.is_finished() => {
            // Dashboard thread alive — ask it to un-hide + focus (and wake the
            // event loop, which may be idle while the window is hidden).
            handle.signals.request_show();
        }
        _ => {
            // No dashboard yet (first click) — spawn the single persistent thread.
            tracing::info!("spawning dashboard window");
            *guard = Some(crate::dashboard::launch(
                state.shared.clone(),
                state.settings.clone(),
            ));
        }
    }
}

/// Resolve the process's HMODULE for use by RegisterClassExW / CreateWindowExW / CreateIcon.
pub fn current_hinstance() -> Result<HMODULE> {
    // SAFETY: passing None (null) returns the handle for the current .exe.
    let hmod = unsafe { GetModuleHandleW(PCWSTR::null()) }
        .map_err(|e| anyhow!("GetModuleHandleW failed: {e}"))?;
    Ok(hmod)
}

#[cfg(test)]
mod tooltip_tests {
    use super::*;

    #[test]
    fn format_local_line_none_says_uncalibrated() {
        assert_eq!(format_local_line(None), "local: (uncalibrated)");
    }

    #[test]
    fn format_local_line_both_caps_prints_both_pcts() {
        let live = LiveUtil {
            util_5h: Some(0.54),
            util_week: Some(0.40),
        };
        assert_eq!(
            format_local_line(Some(&live)),
            "local 5h: 54%   local 7d: 40%"
        );
    }

    #[test]
    fn format_local_line_partial_caps_prints_uncalibrated_per_window() {
        let live = LiveUtil {
            util_5h: Some(0.54),
            util_week: None,
        };
        assert_eq!(
            format_local_line(Some(&live)),
            "local 5h: 54%   local 7d: (uncalibrated)"
        );
    }
}
