//! Median-of-anchors cap derivation.

use chrono::{DateTime, Utc};
use crate::data::parser::Turn;

/// Caps derived from the latest calibration log + cache.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DerivedCaps {
    pub cap_5h: Option<f64>,
    pub cap_week: Option<f64>,
    pub n_anchors_5h: usize,
    pub n_anchors_week: usize,
}

use crate::config;
use chrono::{Datelike, Duration, TimeZone, Weekday};
use chrono_tz::Tz;

/// Returns the most-recent weekly-reset moment (Sun 07:00 local) at or before
/// `anchor_ts`, expressed in UTC.
pub fn last_weekly_reset(anchor_ts: DateTime<Utc>) -> DateTime<Utc> {
    let tz: Tz = config::LOCAL_TZ.parse().expect("LOCAL_TZ must be a valid IANA name");
    let local = anchor_ts.with_timezone(&tz);

    // days_back: how many days from `local`'s weekday back to Sunday (0..=6).
    let target = config::WEEKLY_RESET_WEEKDAY;
    let days_back = ((local.weekday().num_days_from_monday() as i64)
        - (target.num_days_from_monday() as i64))
        .rem_euclid(7);

    // Sun of the same week at 07:00 local.
    let candidate_date = local.date_naive() - Duration::days(days_back);
    let candidate_naive = candidate_date
        .and_hms_opt(config::WEEKLY_RESET_HOUR_LOCAL, 0, 0)
        .expect("07:00 is always valid");
    let candidate_local = tz
        .from_local_datetime(&candidate_naive)
        .single()
        .or_else(|| tz.from_local_datetime(&candidate_naive).earliest())
        .expect("Sun 07:00 should resolve unambiguously");

    let candidate = if candidate_local > local {
        candidate_local - Duration::days(7)
    } else {
        candidate_local
    };

    candidate.with_timezone(&Utc)
}

/// Sum `output_tokens` for the gap-based 5h window containing `anchor_ts`.
///
/// `turns` is assumed sorted by `ts` ascending. The window resets to start at
/// the current turn whenever:
///   - the gap from the previous turn is `>= FIVE_HOUR_WINDOW_HOURS`, OR
///   - the window has been open for `>= FIVE_HOUR_WINDOW_HOURS`.
pub fn five_hour_burn_at(turns: &[Turn], anchor_ts: DateTime<Utc>) -> u64 {
    let gap = Duration::milliseconds((config::FIVE_HOUR_WINDOW_HOURS * 3_600_000.0) as i64);
    let mut current_start: Option<DateTime<Utc>> = None;
    let mut last_ts: Option<DateTime<Utc>> = None;
    let mut burn: u64 = 0;

    for t in turns.iter().filter(|t| t.ts <= anchor_ts) {
        match (current_start, last_ts) {
            (None, _) => {
                current_start = Some(t.ts);
            }
            (Some(start), Some(prev)) => {
                let since_last = t.ts - prev;
                let since_start = t.ts - start;
                if since_last >= gap || since_start >= gap {
                    current_start = Some(t.ts);
                    burn = 0;
                }
            }
            (Some(_), None) => unreachable!("current_start implies last_ts"),
        }
        burn += t.output_tokens;
        last_ts = Some(t.ts);
    }

    burn
}

/// Sum `output_tokens` since the most-recent Sun 07:00-local reset.
/// `turns` may be in any order; we filter, not iterate-in-order.
pub fn weekly_burn_at(turns: &[Turn], anchor_ts: DateTime<Utc>) -> u64 {
    let win_start = last_weekly_reset(anchor_ts);
    turns
        .iter()
        .filter(|t| t.ts >= win_start && t.ts <= anchor_ts)
        .map(|t| t.output_tokens)
        .sum()
}

use crate::calibration::WindowKind;
use crate::log::calibration::CalibrationSample;

/// Median implied cap across all valid anchors. Returns (None, 0) if no anchors.
///
/// An anchor is a `CalibrationSample` where the relevant util (5h or weekly)
/// falls in `[MIN_ANCHOR_UTIL, MAX_ANCHOR_UTIL]`. For each anchor we compute
/// `burn_in_window(anchor.ts) / util` summing `output_tokens`, then take the
/// median across anchors.
pub fn global_cap_from_anchors(
    log: &[CalibrationSample],
    turns: &[Turn],
    kind: WindowKind,
) -> (Option<f64>, usize) {
    let mut implied: Vec<f64> = Vec::new();
    for s in log {
        let util_opt = match kind {
            WindowKind::FiveHour => s.five_hour_util,
            WindowKind::Weekly => s.seven_day_util,
        };
        let Some(util) = util_opt else { continue };
        if util < config::MIN_ANCHOR_UTIL || util > config::MAX_ANCHOR_UTIL {
            continue;
        }
        let burn = match kind {
            WindowKind::FiveHour => five_hour_burn_at(turns, s.ts),
            WindowKind::Weekly => weekly_burn_at(turns, s.ts),
        };
        if burn == 0 || util <= 0.0 {
            continue;
        }
        implied.push(burn as f64 / util);
    }
    if implied.is_empty() {
        return (None, 0);
    }
    let n = implied.len();
    implied.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let median = if n % 2 == 1 {
        implied[n / 2]
    } else {
        (implied[n / 2 - 1] + implied[n / 2]) / 2.0
    };
    (Some(median), n)
}

/// Compute both 5h and weekly caps in one call.
pub fn derive_caps(log: &[CalibrationSample], turns: &[Turn]) -> DerivedCaps {
    let (cap_5h, n5) = global_cap_from_anchors(log, turns, WindowKind::FiveHour);
    let (cap_week, n7) = global_cap_from_anchors(log, turns, WindowKind::Weekly);
    DerivedCaps {
        cap_5h,
        cap_week,
        n_anchors_5h: n5,
        n_anchors_week: n7,
    }
}

#[allow(dead_code)]
fn _silence_imports(_: Weekday) {}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use crate::data::parser::Turn;
    use std::path::PathBuf;

    fn utc(y: i32, m: u32, d: u32, h: u32, mi: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(y, m, d, h, mi, 0).unwrap()
    }

    fn turn(ts: DateTime<Utc>, output: u64) -> Turn {
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
    fn five_hour_burn_at_single_window_sums_all() {
        let turns = vec![
            turn(utc(2026, 5, 24, 10, 0), 100),
            turn(utc(2026, 5, 24, 11, 0), 200),
            turn(utc(2026, 5, 24, 12, 0), 300),
        ];
        let anchor = utc(2026, 5, 24, 13, 0);
        assert_eq!(five_hour_burn_at(&turns, anchor), 600);
    }

    #[test]
    fn five_hour_burn_at_drops_pre_gap_turns() {
        // First turn at 04:00, big gap, then a session 10:00-12:00 totalling 500.
        // Anchor at 12:00 should include only the 10:00+ turns.
        let turns = vec![
            turn(utc(2026, 5, 24, 4, 0), 999),   // pre-gap — should be excluded
            turn(utc(2026, 5, 24, 10, 0), 100),
            turn(utc(2026, 5, 24, 11, 0), 200),
            turn(utc(2026, 5, 24, 12, 0), 200),
        ];
        let anchor = utc(2026, 5, 24, 12, 0);
        assert_eq!(five_hour_burn_at(&turns, anchor), 500);
    }

    #[test]
    fn five_hour_burn_at_window_rollover_by_duration() {
        // Continuous activity over >4.5 hours triggers rollover at the 4.5h mark.
        let turns = vec![
            turn(utc(2026, 5, 24, 8, 0), 100),
            turn(utc(2026, 5, 24, 10, 0), 200),
            turn(utc(2026, 5, 24, 12, 30), 300),  // 4.5h after 08:00 → new window starts here
            turn(utc(2026, 5, 24, 13, 0), 400),
        ];
        let anchor = utc(2026, 5, 24, 13, 0);
        // New window starts at 12:30. Sums 300 + 400 = 700.
        assert_eq!(five_hour_burn_at(&turns, anchor), 700);
    }

    #[test]
    fn last_weekly_reset_when_anchor_is_monday_picks_prior_sunday_0700() {
        // Mon 2026-05-25 14:30 UTC = Mon 16:30 local (CEST = UTC+2 in May).
        let anchor = utc(2026, 5, 25, 14, 30);
        let reset = last_weekly_reset(anchor);
        // Prior Sun 2026-05-24 07:00 local CEST = 2026-05-24 05:00 UTC.
        assert_eq!(reset, utc(2026, 5, 24, 5, 0));
    }

    #[test]
    fn last_weekly_reset_when_anchor_is_sunday_after_0700_picks_today() {
        let anchor = utc(2026, 5, 24, 8, 0);  // Sun 10:00 local CEST
        let reset = last_weekly_reset(anchor);
        assert_eq!(reset, utc(2026, 5, 24, 5, 0));
    }

    #[test]
    fn last_weekly_reset_when_anchor_is_sunday_before_0700_picks_prior_sunday() {
        let anchor = utc(2026, 5, 24, 4, 0);  // Sun 06:00 local CEST
        let reset = last_weekly_reset(anchor);
        // Prior Sun: 2026-05-17 05:00 UTC.
        assert_eq!(reset, utc(2026, 5, 17, 5, 0));
    }

    #[test]
    fn last_weekly_reset_handles_saturday() {
        let anchor = utc(2026, 5, 23, 10, 0);  // Sat 12:00 local
        let reset = last_weekly_reset(anchor);
        // Prior Sun = 2026-05-17 05:00 UTC.
        assert_eq!(reset, utc(2026, 5, 17, 5, 0));
    }

    #[test]
    fn weekly_burn_at_sums_since_last_reset() {
        let turns = vec![
            turn(utc(2026, 5, 17, 4, 0), 999),   // before Sun 05:00 UTC reset — excluded
            turn(utc(2026, 5, 17, 6, 0), 100),   // after reset
            turn(utc(2026, 5, 19, 12, 0), 200),
            turn(utc(2026, 5, 23, 8, 0), 300),
        ];
        let anchor = utc(2026, 5, 23, 12, 0);  // Sat — last reset was Sun 17 05:00 UTC
        assert_eq!(weekly_burn_at(&turns, anchor), 600);
    }

    #[test]
    fn weekly_burn_at_after_reset_excludes_prior_week() {
        let turns = vec![
            turn(utc(2026, 5, 23, 12, 0), 500),
            turn(utc(2026, 5, 24, 6, 0), 100),  // after Sun 05:00 UTC reset
        ];
        let anchor = utc(2026, 5, 24, 8, 0);
        // Only the 100 token row falls within the new week.
        assert_eq!(weekly_burn_at(&turns, anchor), 100);
    }

    use crate::log::calibration::CalibrationSample;
    use crate::calibration::WindowKind;

    fn sample(ts: DateTime<Utc>, util_5h: f64, util_7d: f64) -> CalibrationSample {
        CalibrationSample {
            schema_version: 1,
            ts,
            five_hour_util: Some(util_5h),
            five_hour_resets_at: None,
            seven_day_util: Some(util_7d),
            seven_day_resets_at: None,
            subscription_type: "pro".to_string(),
            rate_limit_tier: "default_claude_ai".to_string(),
        }
    }

    #[test]
    fn global_cap_zero_anchors_returns_none() {
        let log = vec![sample(utc(2026, 5, 24, 10, 0), 0.5, 0.4)];
        let turns = vec![turn(utc(2026, 5, 24, 9, 0), 100)];
        let (cap, n) = global_cap_from_anchors(&log, &turns, WindowKind::FiveHour);
        assert!(cap.is_none());
        assert_eq!(n, 0);
    }

    #[test]
    fn global_cap_single_anchor_returns_burn_over_util() {
        // burn at anchor = 1000; util = 1.00 → cap = 1000.
        let log = vec![sample(utc(2026, 5, 24, 10, 0), 1.00, 0.5)];
        let turns = vec![
            turn(utc(2026, 5, 24, 8, 0), 400),
            turn(utc(2026, 5, 24, 9, 0), 600),
        ];
        let (cap, n) = global_cap_from_anchors(&log, &turns, WindowKind::FiveHour);
        assert_eq!(cap, Some(1000.0));
        assert_eq!(n, 1);
    }

    #[test]
    fn global_cap_multi_anchor_returns_median() {
        // Three anchors, all util=1.00, but bigger windows for some.
        // Implied caps: 100, 200, 300 → median 200.
        let log = vec![
            sample(utc(2026, 5, 24, 10, 0), 1.00, 0.5),
            sample(utc(2026, 5, 24, 16, 0), 1.00, 0.5),
            sample(utc(2026, 5, 24, 22, 0), 1.00, 0.5),
        ];
        let turns = vec![
            turn(utc(2026, 5, 24, 9, 30), 100),    // anchor 1: burn 100, util 1 → cap 100
            // Anchor 1's window ends, anchor 2 starts a new window. 6h gap > 4.5h.
            turn(utc(2026, 5, 24, 15, 30), 200),   // anchor 2: burn 200, util 1 → cap 200
            // Anchor 2's window ends, anchor 3 starts a new window.
            turn(utc(2026, 5, 24, 21, 30), 300),   // anchor 3: burn 300, util 1 → cap 300
        ];
        let (cap, n) = global_cap_from_anchors(&log, &turns, WindowKind::FiveHour);
        assert_eq!(cap, Some(200.0));
        assert_eq!(n, 3);
    }
}
