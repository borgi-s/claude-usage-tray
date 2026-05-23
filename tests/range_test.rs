use chrono::{TimeZone, Utc};
use claude_usage_tray::dashboard::range::{clamp_x_range, Range};

#[test]
fn d1_clamps_to_24h_back() {
    let now = Utc.with_ymd_and_hms(2026, 5, 24, 12, 0, 0).unwrap();
    let (start, end) = clamp_x_range(now, Range::D1);
    assert_eq!(end, now);
    assert_eq!(start, Utc.with_ymd_and_hms(2026, 5, 23, 12, 0, 0).unwrap());
}

#[test]
fn d5_clamps_to_5_days_back() {
    let now = Utc.with_ymd_and_hms(2026, 5, 24, 12, 0, 0).unwrap();
    let (start, _) = clamp_x_range(now, Range::D5);
    assert_eq!(start, Utc.with_ymd_and_hms(2026, 5, 19, 12, 0, 0).unwrap());
}

#[test]
fn d14_clamps_to_14_days_back() {
    let now = Utc.with_ymd_and_hms(2026, 5, 24, 12, 0, 0).unwrap();
    let (start, _) = clamp_x_range(now, Range::D14);
    assert_eq!(start, Utc.with_ymd_and_hms(2026, 5, 10, 12, 0, 0).unwrap());
}

#[test]
fn m1_clamps_to_30_days_back() {
    let now = Utc.with_ymd_and_hms(2026, 5, 24, 12, 0, 0).unwrap();
    let (start, _) = clamp_x_range(now, Range::M1);
    assert_eq!(start, Utc.with_ymd_and_hms(2026, 4, 24, 12, 0, 0).unwrap());
}

#[test]
fn all_returns_now_for_start() {
    let now = Utc.with_ymd_and_hms(2026, 5, 24, 12, 0, 0).unwrap();
    let (start, end) = clamp_x_range(now, Range::All);
    assert_eq!(start, now);
    assert_eq!(end, now);
}

#[test]
fn range_label_round_trip() {
    assert_eq!(Range::D1.label(), "1D");
    assert_eq!(Range::D5.label(), "5D");
    assert_eq!(Range::D14.label(), "14D");
    assert_eq!(Range::M1.label(), "1M");
    assert_eq!(Range::All.label(), "All");
}
