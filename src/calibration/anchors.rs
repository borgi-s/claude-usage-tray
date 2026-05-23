//! Median-of-anchors cap derivation.

use chrono::{DateTime, Utc};

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

#[allow(dead_code)]
fn _silence_imports(_: Weekday) {}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn utc(y: i32, m: u32, d: u32, h: u32, mi: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(y, m, d, h, mi, 0).unwrap()
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
}
