//! Win32 tray icon mode. Entry point: [`run`].
//!
//! Threading: caller (main thread) runs the Win32 message loop. A polling
//! thread spawned by `run` sends `PollEvent`s via mpsc and wakes the UI thread
//! with `PostMessageW(hwnd, WM_APP+1, ...)`.

pub mod icon;
pub mod poller;
pub mod window;

use crate::api::credentials::load_from_default_path;
use crate::render::LastStatus;
use anyhow::Result;
use std::sync::atomic::AtomicBool;
use std::sync::mpsc;
use std::sync::Arc;

/// Run the tray app. Blocks until the user clicks Quit or the process is
/// otherwise terminated. Returns `Ok(())` on clean shutdown.
pub fn run(interval_secs: u64) -> Result<()> {
    let creds = load_from_default_path()?;
    let hinst = window::current_hinstance()?;
    let icons = icon::IconSet::new(hinst)?;

    let shutdown = Arc::new(AtomicBool::new(false));
    let (tx, rx) = mpsc::channel();

    let state = Box::new(window::TrayState {
        last_sample: None,
        last_status: LastStatus::Initial,
        icons,
        rx,
        shutdown: shutdown.clone(),
    });

    let hwnd = window::create(hinst, state)?;

    // Build initial tooltip and register the tray icon.
    let initial_tooltip =
        window::format_tooltip(&LastStatus::Initial, None, chrono::Utc::now());
    {
        // Borrow the icons through GWLP_USERDATA-owned state for the initial add.
        let initial_icon = peek_initial_icon(hwnd);
        icon::add(hwnd, window::WM_APP_TRAYICON, initial_icon, &initial_tooltip)?;
    }

    let send_hwnd = poller::SendHwnd(hwnd);
    let poll_handle = poller::spawn(creds, interval_secs, shutdown.clone(), send_hwnd, tx);

    // Run the message loop until WM_QUIT.
    window::message_loop();

    // Polling thread should be exiting; join cleanly. Errors here are non-fatal.
    if let Err(e) = poll_handle.join() {
        tracing::warn!(error = ?e, "polling thread panicked");
    }

    Ok(())
}

/// Peek at the window's TrayState long enough to retrieve its initial icon.
/// Used only at startup, immediately after `window::create`.
fn peek_initial_icon(hwnd: windows::Win32::Foundation::HWND) -> windows::Win32::UI::WindowsAndMessaging::HICON {
    use windows::Win32::UI::WindowsAndMessaging::{GetWindowLongPtrW, GWLP_USERDATA};
    let state_ptr = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *const window::TrayState;
    // SAFETY: pointer set by `create`; window is on this thread; we read only.
    let state = unsafe { &*state_ptr };
    state.icons.for_state(&state.last_status, None)
}
