use crate::api::credentials::Credentials;
use crate::api::usage::{FetchError, UsageSnapshot};
use crate::calibration::anchors::DerivedCaps;
use crate::calibration::live::LiveUtil;
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

/// Calibration outputs attached to a successful poll.
#[derive(Debug, Clone, Default)]
pub struct PollCalibration {
    pub caps: DerivedCaps,
    pub live: LiveUtil,
    pub hourly_5h: [f64; 24],
    pub hourly_week: [f64; 24],
}

/// One outcome of a single poll attempt. Sent from the polling thread to the
/// UI thread via mpsc.
///
/// `calib` is boxed because `PollCalibration` (two 24-element f64 arrays) is
/// ~520 bytes, much larger than the other variants. Boxing keeps `PollEvent`
/// channel sends to pointer-size for the common rate-limit / error cases.
#[derive(Debug)]
pub enum PollEvent {
    Ok {
        snap: UsageSnapshot,
        calib: Box<PollCalibration>,
    },
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
    tracing::info!(
        interval_secs = interval.as_secs(),
        "polling thread starting"
    );

    while !shutdown.load(Ordering::Relaxed) {
        let fetch_at = Instant::now();

        // Stage 5: refresh local cache + derive caps + live util.
        let calib = compute_calibration();

        // API fetch.
        let event = match poll_once(&creds) {
            Ok(snap) => PollEvent::Ok {
                snap,
                calib: Box::new(calib),
            },
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

/// Refresh cache, read calibration log, derive caps, compute live util + hourly.
/// On any error returns `PollCalibration::default()` so the poll itself still proceeds.
fn compute_calibration() -> PollCalibration {
    use crate::calibration::anchors::derive_caps;
    use crate::calibration::hourly::hour_of_day_cap_series;
    use crate::calibration::live::live_util_now;
    use crate::calibration::WindowKind;
    use crate::data::cache;
    use crate::log::calibration as log_calib;

    let turns = match cache::refresh() {
        Ok(t) => t,
        Err(e) => {
            tracing::warn!(error = %e, "cache::refresh failed; skipping calibration this tick");
            return PollCalibration::default();
        }
    };
    let log = match log_calib::read_all_default() {
        Ok(l) => l,
        Err(e) => {
            tracing::warn!(error = %e, "calibration log read failed; skipping calibration this tick");
            return PollCalibration::default();
        }
    };

    let caps = derive_caps(&log, &turns);
    let hourly_5h = hour_of_day_cap_series(&log, &turns, WindowKind::FiveHour);
    let hourly_week = hour_of_day_cap_series(&log, &turns, WindowKind::Weekly);
    let live = live_util_now(&turns, &caps);

    tracing::debug!(
        n_anchors_5h = caps.n_anchors_5h,
        n_anchors_week = caps.n_anchors_week,
        cap_5h = ?caps.cap_5h,
        cap_week = ?caps.cap_week,
        "calibration computed"
    );

    PollCalibration {
        caps,
        live,
        hourly_5h,
        hourly_week,
    }
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
