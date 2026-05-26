//! Calendar shading bands — weekends (Sat+Sun in local TZ) and nights (22:00-06:00 local).

use chrono::{DateTime, Datelike, Duration, TimeZone, Utc, Weekday};
use chrono_tz::Tz;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BandKind {
    Weekend,
    Night,
}

/// Yields (start_utc, end_utc, kind) for every weekend + night band that
/// intersects `[range_start, range_end]`.
pub fn calendar_bands(
    range_start: DateTime<Utc>,
    range_end: DateTime<Utc>,
    tz: Tz,
) -> Vec<(DateTime<Utc>, DateTime<Utc>, BandKind)> {
    if range_end <= range_start {
        return Vec::new();
    }
    let mut out = Vec::new();

    // Weekend: every Saturday 00:00 local → Monday 00:00 local.
    let cur = range_start.with_timezone(&tz);
    // Back up to most recent Saturday 00:00.
    let days_back =
        (cur.weekday().num_days_from_monday() + 7 - Weekday::Sat.num_days_from_monday()) % 7;
    let mut weekend_start_local = cur.date_naive() - Duration::days(days_back as i64);
    loop {
        let local_start_naive = weekend_start_local.and_hms_opt(0, 0, 0).unwrap();
        let local_end_naive = (weekend_start_local + Duration::days(2))
            .and_hms_opt(0, 0, 0)
            .unwrap();
        let local_start = tz.from_local_datetime(&local_start_naive).single().unwrap();
        let local_end = tz.from_local_datetime(&local_end_naive).single().unwrap();
        let utc_start = local_start.with_timezone(&Utc);
        let utc_end = local_end.with_timezone(&Utc);

        if utc_start >= range_end {
            break;
        }
        if utc_end > range_start {
            out.push((
                utc_start.max(range_start),
                utc_end.min(range_end),
                BandKind::Weekend,
            ));
        }
        weekend_start_local += Duration::days(7);
    }

    // Night: every day 22:00 local → next-day 06:00 local.
    let mut night_local_date = range_start.with_timezone(&tz).date_naive();
    loop {
        let local_start_naive = night_local_date.and_hms_opt(22, 0, 0).unwrap();
        let local_end_naive = (night_local_date + Duration::days(1))
            .and_hms_opt(6, 0, 0)
            .unwrap();
        let local_start = tz.from_local_datetime(&local_start_naive).single().unwrap();
        let local_end = tz.from_local_datetime(&local_end_naive).single().unwrap();
        let utc_start = local_start.with_timezone(&Utc);
        let utc_end = local_end.with_timezone(&Utc);

        if utc_start >= range_end {
            break;
        }
        if utc_end > range_start {
            out.push((
                utc_start.max(range_start),
                utc_end.min(range_end),
                BandKind::Night,
            ));
        }
        night_local_date += Duration::days(1);
    }

    out
}
