use chrono::{TimeZone, Utc};
use claude_usage_tray::dashboard::bands::{calendar_bands, BandKind};

fn tz() -> chrono_tz::Tz {
    claude_usage_tray::settings::CalParams::default().tz
}

#[test]
fn weekend_band_starts_saturday_0000_local_ends_monday_0000() {
    // Sat 2026-05-23 00:00 CEST (UTC+2 in May) = Fri 2026-05-22 22:00 UTC.
    // Mon 2026-05-25 00:00 CEST = Sun 2026-05-24 22:00 UTC.
    let range_start = Utc.with_ymd_and_hms(2026, 5, 20, 0, 0, 0).unwrap();
    let range_end = Utc.with_ymd_and_hms(2026, 5, 27, 0, 0, 0).unwrap();
    let bands = calendar_bands(range_start, range_end, tz());
    let weekends: Vec<_> = bands
        .iter()
        .filter(|(_, _, k)| *k == BandKind::Weekend)
        .collect();
    assert_eq!(weekends.len(), 1);
    let (s, e, _) = weekends[0];
    assert_eq!(s, &Utc.with_ymd_and_hms(2026, 5, 22, 22, 0, 0).unwrap());
    assert_eq!(e, &Utc.with_ymd_and_hms(2026, 5, 24, 22, 0, 0).unwrap());
}

#[test]
fn night_bands_one_per_calendar_day_in_range() {
    // Range covers exactly 3 calendar days (Mon 5/18, Tue 5/19, Wed 5/20).
    // Should produce 3 night bands: Mon→Tue, Tue→Wed, Wed→Thu.
    let range_start = Utc.with_ymd_and_hms(2026, 5, 18, 0, 0, 0).unwrap();
    let range_end = Utc.with_ymd_and_hms(2026, 5, 21, 0, 0, 0).unwrap();
    let bands = calendar_bands(range_start, range_end, tz());
    let nights: Vec<_> = bands
        .iter()
        .filter(|(_, _, k)| *k == BandKind::Night)
        .collect();
    assert_eq!(nights.len(), 3);
}

#[test]
fn empty_range_returns_empty_vec() {
    let t = Utc.with_ymd_and_hms(2026, 5, 24, 12, 0, 0).unwrap();
    let bands = calendar_bands(t, t, tz());
    assert!(bands.is_empty());
}
