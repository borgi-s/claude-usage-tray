//! Live util for the tooltip.

use crate::calibration::anchors::{five_hour_burn_at, weekly_burn_at, DerivedCaps};
use crate::data::parser::Turn;
use crate::settings::CalParams;
use chrono::{DateTime, Utc};

/// Current local utilization, in [0.0, ∞). `None` means "uncalibrated" — i.e.
/// the corresponding cap in `DerivedCaps` is `None`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LiveUtil {
    pub util_5h: Option<f64>,
    pub util_week: Option<f64>,
}

/// Compute the current util against the supplied caps. `now` is passed in for
/// testability — production callers should use `live_util_now`.
pub fn live_util_at(
    turns: &[Turn],
    caps: &DerivedCaps,
    now: DateTime<Utc>,
    cp: CalParams,
) -> LiveUtil {
    LiveUtil {
        util_5h: caps
            .cap_5h
            .map(|c| five_hour_burn_at(turns, now) as f64 / c),
        util_week: caps
            .cap_week
            .map(|c| weekly_burn_at(turns, now, cp) as f64 / c),
    }
}

/// Convenience wrapper using `Utc::now()`.
pub fn live_util_now(turns: &[Turn], caps: &DerivedCaps, cp: CalParams) -> LiveUtil {
    live_util_at(turns, caps, Utc::now(), cp)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::parser::Turn;
    use crate::settings::CalParams;
    use chrono::{TimeZone, Utc};
    use std::path::PathBuf;

    fn turn(ts: chrono::DateTime<Utc>, output: u64) -> Turn {
        Turn {
            ts,
            session_id: String::new(),
            subagent_id: None,
            is_subagent: false,
            project_cwd: String::new(),
            model: String::new(),
            version: String::new(),
            input_tokens: 0,
            output_tokens: output,
            cache_creation_input_tokens: 0,
            cache_read_input_tokens: 0,
            source_file: PathBuf::new(),
            is_rate_limit_error: false,
        }
    }

    #[test]
    fn live_util_at_no_caps_returns_no_util() {
        let caps = DerivedCaps::default();
        let now = Utc.with_ymd_and_hms(2026, 5, 24, 12, 0, 0).unwrap();
        let live = live_util_at(&[], &caps, now, CalParams::default());
        assert_eq!(live.util_5h, None);
        assert_eq!(live.util_week, None);
    }

    #[test]
    fn live_util_at_with_caps_returns_burn_over_cap() {
        let caps = DerivedCaps {
            cap_5h: Some(1000.0),
            cap_week: Some(10_000.0),
            n_anchors_5h: 1,
            n_anchors_week: 1,
        };
        let now = Utc.with_ymd_and_hms(2026, 5, 24, 12, 0, 0).unwrap();
        let turns = vec![turn(
            Utc.with_ymd_and_hms(2026, 5, 24, 11, 0, 0).unwrap(),
            250,
        )];
        let live = live_util_at(&turns, &caps, now, CalParams::default());
        assert_eq!(live.util_5h, Some(0.25));
        // Weekly window includes the same turn.
        assert_eq!(live.util_week, Some(250.0 / 10_000.0));
    }
}
