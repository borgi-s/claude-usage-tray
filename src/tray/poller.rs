use crate::api::credentials::Credentials;
use crate::api::usage::{FetchError, UsageSnapshot};
use crate::poll::poll_once;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::PostMessageW;

/// Custom message the polling thread posts to the UI thread to indicate a new
/// event has been queued in the mpsc channel.
pub const WM_APP_POLL: u32 = windows::Win32::UI::WindowsAndMessaging::WM_APP + 1;

/// One outcome of a single poll attempt. Sent from the polling thread to the
/// UI thread via mpsc.
#[derive(Debug)]
pub enum PollEvent {
    Ok(UsageSnapshot),
    RateLimited,
    Error(String),
}

/// HWND wrapper that's safe to send across threads.
///
/// `windows::Win32::Foundation::HWND` is a `*mut c_void` newtype which Rust
/// won't auto-impl `Send` for. We never dereference the pointer from the
/// polling thread; we only pass it back through Win32 (`PostMessageW`), which
/// is itself thread-safe.
#[derive(Clone, Copy)]
pub struct SendHwnd(pub HWND);

// SAFETY: see the doc comment on SendHwnd above.
unsafe impl Send for SendHwnd {}

/// Spawn the polling thread. The thread runs until `shutdown` becomes true.
///
/// `creds` is moved into the thread. `interval_secs` is the cadence between
/// successive fetches; the cadence anchors to the START of each fetch (so a
/// slow fetch shortens the next sleep instead of stretching the schedule).
pub fn spawn(
    creds: Credentials,
    interval_secs: u64,
    shutdown: Arc<AtomicBool>,
    hwnd: SendHwnd,
    tx: Sender<PollEvent>,
) -> JoinHandle<()> {
    let interval = Duration::from_secs(interval_secs);
    thread::spawn(move || polling_loop(creds, interval, shutdown, hwnd, tx))
}

fn polling_loop(
    creds: Credentials,
    interval: Duration,
    shutdown: Arc<AtomicBool>,
    hwnd: SendHwnd,
    tx: Sender<PollEvent>,
) {
    tracing::info!(interval_secs = interval.as_secs(), "polling thread starting");

    while !shutdown.load(Ordering::Relaxed) {
        let fetch_at = Instant::now();

        let event = match poll_once(&creds) {
            Ok(snap) => PollEvent::Ok(snap),
            Err(FetchError::RateLimited) => PollEvent::RateLimited,
            Err(other) => PollEvent::Error(other.to_string()),
        };

        // If the UI thread has already dropped the receiver, send fails — we
        // simply exit the loop on the next shutdown check.
        let _ = tx.send(event);

        // Wake the UI thread to drain the channel.
        // SAFETY: PostMessageW is thread-safe; the HWND is valid until shutdown.
        unsafe {
            let _ = PostMessageW(hwnd.0, WM_APP_POLL, WPARAM(0), LPARAM(0));
        }

        sleep_interruptible(&shutdown, fetch_at, interval);
    }

    tracing::info!("polling thread exiting");
}

fn sleep_interruptible(shutdown: &Arc<AtomicBool>, fetch_at: Instant, interval: Duration) {
    let target = fetch_at + interval;
    while Instant::now() < target {
        if shutdown.load(Ordering::Relaxed) {
            return;
        }
        let remaining = target.saturating_duration_since(Instant::now());
        thread::sleep(remaining.min(Duration::from_millis(500)));
    }
}
