//! Shared axis formatting helpers for the dashboard charts.

use chrono::{DateTime, Utc};
use chrono_tz::Tz;

/// Format an x-axis tick (Unix seconds as f64) as a local-time date label.
/// Shows "MMM DD" (e.g. "May 13"); date-granularity is fine for our 1D–All ranges.
pub fn format_x_tick(secs: f64) -> String {
    let tz: Tz = crate::config::LOCAL_TZ.parse().unwrap_or(chrono_tz::UTC);
    match DateTime::<Utc>::from_timestamp(secs as i64, 0) {
        Some(dt) => dt.with_timezone(&tz).format("%b %d").to_string(),
        None => String::new(),
    }
}
