//! Native egui dashboard window.

pub mod app;

use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use windows::Win32::Foundation::HWND;

pub const DASHBOARD_WINDOW_TITLE: &str = "Claude usage tracker";

/// Thread-safe wrapper for HWND. The pointer is opaque from Rust's perspective;
/// Win32 functions that take an HWND are themselves thread-safe.
#[derive(Clone, Copy)]
pub struct SendHwnd(pub HWND);

// SAFETY: see doc comment.
unsafe impl Send for SendHwnd {}
unsafe impl Sync for SendHwnd {}

pub struct DashboardHandle {
    pub hwnd: Arc<Mutex<Option<SendHwnd>>>,
    pub join: JoinHandle<()>,
}

use crate::dashboard::app::DashboardApp;
use crate::shared::SharedSnapshot;

use windows::Win32::Foundation::{BOOL, LPARAM};
use windows::Win32::UI::WindowsAndMessaging::{EnumWindows, GetWindowTextW, IsWindowVisible};

/// Search top-level windows for one whose title equals `target` (case-sensitive).
///
/// Returns the first match. Used to find the dashboard's HWND after eframe
/// creates it — we don't have a clean handle to it from the egui-side API.
///
/// Title is unique to this process: see DASHBOARD_WINDOW_TITLE.
pub fn find_hwnd_by_title(target: &str) -> Option<HWND> {
    let mut state = EnumState {
        target_utf16: target.encode_utf16().collect::<Vec<u16>>(),
        result: None,
    };

    // SAFETY: state lives for the duration of EnumWindows; the callback is
    // invoked synchronously from this thread; we cast a &mut to LPARAM and back.
    unsafe {
        let _ = EnumWindows(
            Some(enum_proc),
            LPARAM(&mut state as *mut _ as isize),
        );
    }
    state.result
}

struct EnumState {
    target_utf16: Vec<u16>,
    result: Option<HWND>,
}

extern "system" fn enum_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
    // SAFETY: lparam was constructed from a &mut EnumState in find_hwnd_by_title.
    let state = unsafe { &mut *(lparam.0 as *mut EnumState) };

    if unsafe { IsWindowVisible(hwnd) != BOOL(1) } {
        return BOOL(1); // continue
    }

    let mut buf = [0u16; 256];
    // SAFETY: buf has room for 256 UTF-16 units; GetWindowTextW handles its own length.
    let len = unsafe { GetWindowTextW(hwnd, &mut buf) } as usize;
    if len == 0 {
        return BOOL(1);
    }

    if buf[..len] == state.target_utf16[..] {
        state.result = Some(hwnd);
        return BOOL(0); // stop
    }
    BOOL(1) // continue
}

/// Spawn the dashboard window on a fresh thread. Returns immediately;
/// the HWND inside the returned handle is populated asynchronously by the
/// dashboard thread once eframe builds the window.
pub fn launch(shared: SharedSnapshot) -> DashboardHandle {
    let hwnd_slot: Arc<Mutex<Option<SendHwnd>>> = Arc::new(Mutex::new(None));
    let hwnd_slot_for_thread = hwnd_slot.clone();

    let join = std::thread::spawn(move || {
        let app = DashboardApp::new(shared, hwnd_slot_for_thread);
        let native_options = eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default()
                .with_inner_size([1100.0, 720.0])
                .with_min_inner_size([700.0, 480.0])
                .with_title(DASHBOARD_WINDOW_TITLE),
            // Opt into non-main-thread EventLoop creation. Without this,
            // winit panics on Windows when constructed off the main thread.
            event_loop_builder: Some(Box::new(|builder| {
                use winit::platform::windows::EventLoopBuilderExtWindows;
                builder.with_any_thread(true);
            })),
            ..Default::default()
        };

        if let Err(e) = eframe::run_native(
            "claude-usage-tray-dashboard",
            native_options,
            Box::new(|_cc| Ok(Box::new(app))),
        ) {
            tracing::warn!(error = ?e, "eframe::run_native failed");
        }
        // run_native returned → window closed → thread is about to exit.
    });

    DashboardHandle { hwnd: hwnd_slot, join }
}
