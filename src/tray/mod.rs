//! Win32 tray icon mode. Entry point: [`run`].
//!
//! Threading: caller (main thread) runs the Win32 message loop. A polling
//! thread spawned by `run` sends `PollEvent`s via mpsc and wakes the UI thread
//! with `PostMessageW(hwnd, WM_APP+1, ...)`.

pub mod icon;
pub mod poller;
pub mod widget;
pub mod window;

use crate::api::credentials::load_from_default_path;
use crate::render::LastStatus;
use anyhow::Result;
use std::sync::atomic::AtomicBool;
use std::sync::mpsc;
use std::sync::Arc;

/// Run the tray app. Blocks until the user clicks Quit or the process is
/// otherwise terminated. Returns `Ok(())` on clean shutdown.
pub fn run() -> Result<()> {
    let creds = load_from_default_path()?;
    use crate::shared::{new_shared_settings, new_shared_snapshot};
    let shared = new_shared_snapshot();
    let settings = new_shared_settings();
    let dashboard: std::sync::Arc<std::sync::Mutex<Option<crate::dashboard::DashboardHandle>>> =
        std::sync::Arc::new(std::sync::Mutex::new(None));
    let hinst = window::current_hinstance()?;
    let renderer = icon::IconRenderer::new();

    let shutdown = Arc::new(AtomicBool::new(false));
    let (tx, rx) = mpsc::channel();
    let (update_tx, update_rx) = mpsc::channel();

    let state = Box::new(window::TrayState {
        last_sample: None,
        last_status: LastStatus::Initial,
        renderer,
        current_hicon: None,
        rx,
        shutdown: shutdown.clone(),
        last_caps: None,
        last_local_util: None,
        last_hourly_5h: None,
        last_hourly_week: None,
        shared: shared.clone(),
        settings: settings.clone(),
        dashboard: dashboard.clone(),
        update_rx,
        update_tx: update_tx.clone(),
        available_update: None,
        manual_check_history: Vec::new(),
    });

    let hwnd = window::create(hinst, state)?;

    // Build initial tooltip and register the tray icon with a freshly-rendered HICON.
    let initial_tooltip =
        window::format_tooltip(&LastStatus::Initial, None, None, chrono::Utc::now());
    render_and_store_initial_icon(hwnd, &initial_tooltip)?;

    let send_hwnd = poller::SendHwnd(hwnd);
    let poll_handle = poller::spawn(
        creds,
        shutdown.clone(),
        send_hwnd,
        tx,
        update_tx,
        shared.clone(),
        settings.clone(),
    );

    // Run the message loop until WM_QUIT.
    window::message_loop();

    // Polling thread should be exiting; join cleanly. Errors here are non-fatal.
    if let Err(e) = poll_handle.join() {
        tracing::warn!(error = ?e, "polling thread panicked");
    }

    // Take + join the dashboard handle if one was ever created.
    let dash = dashboard.lock().unwrap().take();
    if let Some(handle) = dash {
        if let Err(e) = handle.join.join() {
            tracing::warn!(error = ?e, "dashboard thread panicked");
        }
    }

    Ok(())
}

/// Render the initial gray-question icon and register it with the shell.
/// Stores the HICON in `state.current_hicon` so the next render can destroy it.
fn render_and_store_initial_icon(
    hwnd: windows::Win32::Foundation::HWND,
    tooltip: &[u16],
) -> anyhow::Result<()> {
    use windows::Win32::UI::WindowsAndMessaging::{GetWindowLongPtrW, GWLP_USERDATA};
    let state_ptr = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *mut window::TrayState;
    // SAFETY: pointer set by `create`; window is on this thread; sole owner.
    let state = unsafe { &mut *state_ptr };

    let hicon = state.renderer.render(&state.last_status, None)?;
    state.current_hicon = Some(hicon);
    icon::add(hwnd, window::WM_APP_TRAYICON, hicon, tooltip)?;
    Ok(())
}
