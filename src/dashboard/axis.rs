//! Shared axis formatting + timezone helpers for the dashboard charts.

use chrono::offset::LocalResult;
use chrono::{DateTime, Duration, NaiveDateTime, TimeZone, Utc};
use chrono_tz::Tz;

/// Format an x-axis tick (Unix seconds as f64) as a local-time date label.
/// Shows "MMM DD" (e.g. "May 13"); date-granularity is fine for our 1D–All ranges.
pub fn format_x_tick(secs: f64, tz: Tz) -> String {
    match DateTime::<Utc>::from_timestamp(secs as i64, 0) {
        Some(dt) => dt.with_timezone(&tz).format("%b %d").to_string(),
        None => String::new(),
    }
}

/// Resolve a local wall-clock time to a UTC instant **without panicking on DST
/// transitions**. `chrono`'s `from_local_datetime` returns three cases:
/// - `Single` — the normal, unambiguous case.
/// - `Ambiguous` — the wall time occurs twice (autumn fall-back hour); we pick
///   the earliest occurrence.
/// - `None` — the wall time does not exist (spring-forward gap); we nudge an
///   hour forward to land past the gap, with a UTC reinterpretation as a last
///   resort.
///
/// Used for chart band boundaries and daily-bar positions, where the exact
/// instant chosen during a 1-hour DST transition is cosmetically irrelevant —
/// the important property is that it never panics for an arbitrary user-chosen
/// timezone whose transition happens to land on a band-boundary hour.
pub fn local_to_utc(tz: Tz, naive: NaiveDateTime) -> DateTime<Utc> {
    match tz.from_local_datetime(&naive) {
        LocalResult::Single(dt) => dt.with_timezone(&Utc),
        LocalResult::Ambiguous(earliest, _latest) => earliest.with_timezone(&Utc),
        LocalResult::None => match tz.from_local_datetime(&(naive + Duration::hours(1))) {
            LocalResult::Single(dt) => dt.with_timezone(&Utc),
            LocalResult::Ambiguous(earliest, _) => earliest.with_timezone(&Utc),
            LocalResult::None => Utc.from_utc_datetime(&naive),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    fn naive(y: i32, m: u32, d: u32, h: u32, mi: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(y, m, d)
            .unwrap()
            .and_hms_opt(h, mi, 0)
            .unwrap()
    }

    #[test]
    fn local_to_utc_normal_time() {
        // 2026-06-15 12:00 Copenhagen (CEST, UTC+2) → 10:00 UTC.
        let got = local_to_utc(chrono_tz::Europe::Copenhagen, naive(2026, 6, 15, 12, 0));
        assert_eq!(got, Utc.with_ymd_and_hms(2026, 6, 15, 10, 0, 0).unwrap());
    }

    #[test]
    fn local_to_utc_ambiguous_falls_back_to_earliest() {
        // 2026-10-25: clocks go back 03:00 CEST → 02:00 CET, so 02:30 local
        // occurs twice. Earliest occurrence is CEST (UTC+2) → 00:30 UTC.
        let got = local_to_utc(chrono_tz::Europe::Copenhagen, naive(2026, 10, 25, 2, 30));
        assert_eq!(got, Utc.with_ymd_and_hms(2026, 10, 25, 0, 30, 0).unwrap());
    }

    #[test]
    fn local_to_utc_nonexistent_nudges_past_gap() {
        // 2026-03-29: clocks jump 02:00 CET → 03:00 CEST, so 02:30 local does
        // not exist. Nudging +1h lands at 03:30 CEST (UTC+2) → 01:30 UTC.
        let got = local_to_utc(chrono_tz::Europe::Copenhagen, naive(2026, 3, 29, 2, 30));
        assert_eq!(got, Utc.with_ymd_and_hms(2026, 3, 29, 1, 30, 0).unwrap());
    }
}
