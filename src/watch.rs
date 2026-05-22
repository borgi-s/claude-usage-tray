use crate::api::credentials::{load_from_default_path, Credentials};
use crate::api::usage::{fetch_usage, FetchError, UsageSnapshot};
use crate::log::calibration::append_to_default_path;
use crate::render::{draw_frame, Frame, LastStatus};
use anyhow::Result;
use chrono::{DateTime, Utc};
use std::io::Write;
use std::thread;
use std::time::{Duration, Instant};

struct WatchState {
    last_success: Option<(UsageSnapshot, DateTime<Utc>)>,
    last_status: LastStatus,
    last_line_count: u16,
}

pub fn run(interval_secs: u64) -> Result<()> {
    let creds = load_from_default_path()?;
    let mut state = WatchState {
        last_success: None,
        last_status: LastStatus::Initial,
        last_line_count: 0,
    };

    tracing::info!(interval_secs, "watch loop starting");

    loop {
        let fetch_at = Instant::now();
        tick(&creds, &mut state);
        // `redraw` returns the new line_count so the next frame's cursor-up math is correct.
        // If the write fails (broken pipe, etc.), keep the prior count.
        state.last_line_count =
            redraw(&state, &creds, interval_secs).unwrap_or(state.last_line_count);
        sleep_until_next(fetch_at, interval_secs);
    }
}

fn tick(creds: &Credentials, state: &mut WatchState) {
    match fetch_usage(creds) {
        Ok(snap) => {
            // Append BEFORE updating in-memory state so a log failure doesn't
            // make us forget the fresh sample (we never propagate log errors).
            if let Err(e) = append_to_default_path(&snap, creds) {
                tracing::warn!(error = %e, "calibration log write failed");
            }
            state.last_success = Some((snap, Utc::now()));
            state.last_status = LastStatus::Ok;
        }
        Err(FetchError::RateLimited) => {
            tracing::warn!("rate limited; keeping last sample on screen");
            state.last_status = LastStatus::RateLimited;
        }
        Err(other) => {
            tracing::warn!(error = ?other, "poll failed");
            state.last_status = LastStatus::Error(other.to_string());
        }
    }
}

fn redraw(state: &WatchState, creds: &Credentials, interval_secs: u64) -> std::io::Result<u16> {
    let frame: Frame = draw_frame(
        state.last_success.as_ref(),
        creds,
        interval_secs,
        &state.last_status,
        Utc::now(),
    );

    let prefix = if state.last_line_count == 0 {
        String::new()
    } else {
        // Move cursor up N lines, then clear from cursor to end of screen.
        format!("\x1b[{}A\x1b[J", state.last_line_count)
    };

    let mut stdout = std::io::stdout().lock();
    write!(stdout, "{}{}", prefix, frame.body)?;
    stdout.flush()?;

    Ok(frame.line_count)
}

fn sleep_until_next(fetch_at: Instant, interval_secs: u64) {
    let target = fetch_at + Duration::from_secs(interval_secs);
    let now = Instant::now();
    if target > now {
        thread::sleep(target - now);
    }
    // If `target <= now` (fetch took longer than the interval), don't sleep — loop again immediately.
}
