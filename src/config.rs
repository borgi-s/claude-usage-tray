//! Calibration + display constants. Centralized so future overrides
//! (e.g., `~/.claude-usage-tray/config.toml`) have a single source of truth.

use chrono::Weekday;

/// IANA timezone name for local-time displays and weekly-reset math.
pub const LOCAL_TZ: &str = "Europe/Copenhagen";

/// Weekday on which Anthropic's weekly window resets (verified empirically).
pub const WEEKLY_RESET_WEEKDAY: Weekday = Weekday::Sun;

/// Hour (in local time) at which the weekly window resets. 07:00 local.
pub const WEEKLY_RESET_HOUR_LOCAL: u32 = 7;

/// Effective 5h burn-window length. Anthropic publishes 5h but observation
/// suggests the cap behaves like a ~4.5h window.
pub const FIVE_HOUR_WINDOW_HOURS: f64 = 4.5;

/// Minimum API utilization for a sample to be considered an anchor.
pub const MIN_ANCHOR_UTIL: f64 = 0.95;

/// Maximum API utilization for a sample to be considered an anchor.
/// Allows a small overshoot above 1.0 since the API can briefly report >100%.
pub const MAX_ANCHOR_UTIL: f64 = 1.01;
