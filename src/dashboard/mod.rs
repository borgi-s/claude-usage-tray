//! Native egui dashboard window.

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
