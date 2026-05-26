use crate::api::credentials::Credentials;
use crate::api::usage::{FetchError, UsageSnapshot};
use crate::calibration::anchors::DerivedCaps;
use crate::calibration::live::LiveUtil;
use crate::data::parser::Turn;
use crate::poll::poll_once;
use crate::render::LastStatus;
use crate::shared::snapshot::{compute_kpis, AppSnapshot};
use crate::shared::SharedSnapshot;
use crate::updater::{self, UpdateEvent};
use chrono::Duration as ChronoDuration;
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
    update_tx: Sender<UpdateEvent>,
    shared: SharedSnapshot,
) -> JoinHandle<()> {
    let interval = Duration::from_secs(interval_secs);
    thread::spawn(move || polling_loop(creds, interval, shutdown, hwnd, tx, update_tx, shared))
}

fn polling_loop(
    creds: Credentials,
    interval: Duration,
    shutdown: Arc<AtomicBool>,
    hwnd: SendHwnd,
    tx: Sender<PollEvent>,
    update_tx: Sender<UpdateEvent>,
    shared: SharedSnapshot,
) {
    tracing::info!(
        interval_secs = interval.as_secs(),
        "polling thread starting"
    );

    // Stage 6.5: this thread is the SOLE writer of state.json. Load once, keep in memory.
    let mut app_state = crate::state::load();

    // Track the last successful sample so the dashboard's shared snapshot can
    // surface it between polls even when the most recent poll was 429/error.
    let mut last_sample: Option<(
        crate::api::usage::UsageSnapshot,
        chrono::DateTime<chrono::Utc>,
    )> = None;
    // Starts as Initial; written to the shared snapshot immediately so the
    // dashboard can show "fetching…" before the first poll completes.
    let mut last_status = LastStatus::Initial;
    // Publish the Initial status to the shared snapshot right away so the
    // dashboard doesn't show stale/default data while the first poll is running.
    if let Ok(mut g) = shared.write() {
        g.last_status = last_status.clone();
        g.interval_secs = interval.as_secs();
    }

    // Stage 7: best-effort Supabase sync. `None` when unconfigured (no .env) —
    // the agent then behaves exactly as before.
    let syncer = match crate::sync::Syncer::from_env() {
        Ok(s) => {
            if s.is_some() {
                tracing::info!("supabase sync enabled");
            } else {
                tracing::info!("supabase sync disabled (no .env config)");
            }
            s
        }
        Err(e) => {
            tracing::warn!(error = %e, "supabase sync config invalid; disabled");
            None
        }
    };

    while !shutdown.load(Ordering::Relaxed) {
        let fetch_at = Instant::now();

        // Stage 5: refresh local cache + derive caps + live util. NOW also
        // returns the turns Arc so we can put it on the shared snapshot.
        let (calib, turns_arc, log_arc) = compute_calibration_with_turns();

        // API fetch. Update persistent last_sample / last_status so the shared
        // snapshot always carries the freshest available status even on 429/error.
        let event = match poll_once(&creds) {
            Ok(snap) => {
                last_sample = Some((snap.clone(), chrono::Utc::now()));
                last_status = LastStatus::Ok;
                PollEvent::Ok {
                    snap,
                    calib: Box::new(calib.clone()),
                }
            }
            Err(FetchError::RateLimited) => {
                last_status = LastStatus::RateLimited;
                PollEvent::RateLimited
            }
            Err(other) => {
                let msg = other.to_string();
                last_status = LastStatus::Error(msg.clone());
                PollEvent::Error(msg)
            }
        };

        // Write the shared snapshot for the dashboard. Build BEFORE sending
        // the mpsc event so an immediate UI-thread reaction can see fresh data.
        let kpis = compute_kpis(&turns_arc, &calib.caps, &crate::settings::CostWeights::default(), crate::settings::CalParams::default());
        let snapshot = AppSnapshot {
            turns: turns_arc,
            log: log_arc,
            caps: calib.caps,
            hourly_5h: calib.hourly_5h,
            hourly_week: calib.hourly_week,
            live_util: calib.live,
            last_sample: last_sample.clone(),
            last_status: last_status.clone(),
            kpis,
            interval_secs: interval.as_secs(),
        };
        // Stage 7: best-effort upload of the snapshot we just built. Re-read the
        // calibration log so the parquet matches this tick.
        if let Some(syncer) = &syncer {
            let samples = match crate::log::calibration::read_all_default() {
                Ok(s) => s,
                Err(e) => {
                    tracing::warn!(error = %e, "calibration log read failed; uploading empty samples this tick");
                    Vec::new()
                }
            };
            syncer.run_once(&snapshot, &creds, &samples);
        }
        match shared.write() {
            Ok(mut g) => *g = snapshot,
            Err(e) => {
                tracing::warn!(error = ?e, "SharedSnapshot lock poisoned, dashboard data stale")
            }
        }

        let _ = tx.send(event);

        // SAFETY: PostMessageW is thread-safe; the HWND is valid until shutdown.
        unsafe {
            let _ = PostMessageW(hwnd.0, WM_APP_POLL, WPARAM(0), LPARAM(0));
        }

        // Skip the (blocking, ~10s) update check if we're already shutting down,
        // so Quit isn't delayed by an in-flight GitHub fetch. The check is gated
        // to once/24h internally; this only avoids starting it during shutdown.
        if !shutdown.load(Ordering::Relaxed) {
            maybe_check_for_update(&mut app_state, hwnd, &update_tx);
        }

        sleep_interruptible(&shutdown, fetch_at, interval);
    }

    tracing::info!("polling thread exiting");
}

/// Refresh cache, read log, derive caps, compute live util + hourly. Returns
/// (calibration, turns_arc) so the polling loop can put turns on the shared
/// snapshot. On any error, returns (default, Arc::new(Vec::new())) so the
/// poll itself still proceeds.
fn compute_calibration_with_turns() -> (
    PollCalibration,
    Arc<Vec<Turn>>,
    Arc<Vec<crate::log::calibration::CalibrationSample>>,
) {
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
            return (
                PollCalibration::default(),
                Arc::new(Vec::new()),
                Arc::new(Vec::new()),
            );
        }
    };
    let turns_arc = Arc::new(turns);
    let log = match log_calib::read_all_default() {
        Ok(l) => l,
        Err(e) => {
            tracing::warn!(error = %e, "calibration log read failed; skipping calibration this tick");
            return (PollCalibration::default(), turns_arc, Arc::new(Vec::new()));
        }
    };

    let caps = derive_caps(&log, &turns_arc, crate::settings::CalParams::default());
    let hourly_5h = hour_of_day_cap_series(&log, &turns_arc, WindowKind::FiveHour, crate::settings::CalParams::default());
    let hourly_week = hour_of_day_cap_series(&log, &turns_arc, WindowKind::Weekly, crate::settings::CalParams::default());
    let live = live_util_now(&turns_arc, &caps);

    tracing::debug!(
        n_anchors_5h = caps.n_anchors_5h,
        n_anchors_week = caps.n_anchors_week,
        cap_5h = ?caps.cap_5h,
        cap_week = ?caps.cap_week,
        n_turns = turns_arc.len(),
        "calibration computed"
    );

    (
        PollCalibration {
            caps,
            live,
            hourly_5h,
            hourly_week,
        },
        turns_arc,
        Arc::new(log),
    )
}

/// Gated daily GitHub update check. Sole writer of state.json. Persists
/// `last_check` (even on failure) so a failed check still throttles to daily;
/// computes the once-per-version `notify` flag and persists `last_notified_version`.
fn maybe_check_for_update(
    app_state: &mut crate::state::AppState,
    hwnd: SendHwnd,
    update_tx: &Sender<UpdateEvent>,
) {
    let due = match app_state.update.last_check {
        None => true,
        Some(t) => chrono::Utc::now() - t >= ChronoDuration::hours(24),
    };
    if !due {
        return;
    }

    app_state.update.last_check = Some(chrono::Utc::now());
    let current = updater::current_version();

    match updater::check_latest(&current) {
        Ok(check) => {
            let new_version = check.latest.version.to_string();
            let notify = check.is_newer
                && app_state.update.last_notified_version.as_deref() != Some(new_version.as_str());
            if notify {
                app_state.update.last_notified_version = Some(new_version);
            }
            if let Err(e) = crate::state::save(app_state) {
                tracing::warn!(error = %e, "failed to persist state.json");
            }
            let _ = update_tx.send(UpdateEvent::Result { check, notify });
            // SAFETY: PostMessageW is thread-safe; the HWND is valid until shutdown.
            unsafe {
                let _ = PostMessageW(
                    hwnd.0,
                    crate::tray::window::WM_APP_UPDATE,
                    WPARAM(0),
                    LPARAM(0),
                );
            }
        }
        Err(e) => {
            // Persist last_check anyway so we don't retry until tomorrow.
            if let Err(se) = crate::state::save(app_state) {
                tracing::warn!(error = %se, "failed to persist state.json");
            }
            tracing::warn!(error = %e, "auto update check failed");
        }
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
