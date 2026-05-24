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

/// Per-token-type weights for the "cost-weighted" KPI/chart aggregate. These
/// mirror the Python project's config.COST_WEIGHTS — they're heuristic, not
/// authoritative Anthropic pricing, and only used for the dashboard's spend
/// view (NOT for cap calibration, which uses output_tokens only — Stage 5).
pub const COST_WEIGHT_INPUT: f64 = 1.0;
pub const COST_WEIGHT_CACHE_CREATION: f64 = 1.25;
pub const COST_WEIGHT_CACHE_READ: f64 = 0.1;
pub const COST_WEIGHT_OUTPUT: f64 = 5.0;

/// Per-model context window in tokens. Prefix-matched against the model
/// string; the FIRST matching entry wins, so more specific prefixes must come
/// before shorter ones (e.g. `claude-sonnet-4-6` before `claude-sonnet-4`).
/// Mirrors the Python project's `config.MODEL_CONTEXT_WINDOWS`.
pub const MODEL_CONTEXT_WINDOWS: &[(&str, u64)] = &[
    ("claude-opus-4-7", 1_000_000),
    ("claude-opus-4-6", 1_000_000),
    ("claude-sonnet-4-6", 1_000_000),
    ("claude-sonnet-4-5", 200_000),
    ("claude-sonnet-4", 200_000),
    ("claude-haiku-4-5", 200_000),
    ("claude-3-7-sonnet", 200_000),
    ("claude-3-5-sonnet", 200_000),
    ("claude-3-5-haiku", 200_000),
    ("claude-3-opus", 200_000),
];

/// Fallback context window for empty or unrecognized model strings.
pub const DEFAULT_CONTEXT_WINDOW: u64 = 200_000;

/// Returns the context window for `model` via prefix match, or the default.
pub fn context_window_for(model: &str) -> u64 {
    if model.is_empty() {
        return DEFAULT_CONTEXT_WINDOW;
    }
    for (prefix, window) in MODEL_CONTEXT_WINDOWS {
        if model.starts_with(prefix) {
            return *window;
        }
    }
    DEFAULT_CONTEXT_WINDOW
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn context_window_prefix_match_prefers_longer_prefix() {
        // sonnet-4-6 is 1M and must win over the sonnet-4 (200k) entry.
        assert_eq!(context_window_for("claude-sonnet-4-6-20260101"), 1_000_000);
        assert_eq!(context_window_for("claude-sonnet-4-5-20251101"), 200_000);
    }

    #[test]
    fn context_window_opus_is_one_million() {
        assert_eq!(context_window_for("claude-opus-4-7"), 1_000_000);
        assert_eq!(context_window_for("claude-opus-4-6"), 1_000_000);
    }

    #[test]
    fn context_window_unknown_and_empty_fall_back_to_default() {
        assert_eq!(context_window_for("gpt-9"), DEFAULT_CONTEXT_WINDOW);
        assert_eq!(context_window_for(""), DEFAULT_CONTEXT_WINDOW);
    }
}
