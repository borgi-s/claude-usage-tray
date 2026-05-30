use claude_usage_tray::api::usage::{parse_usage_response, UsageSnapshot};
use std::path::Path;

#[test]
fn parses_full_response() {
    let raw = std::fs::read_to_string(Path::new("tests/fixtures/usage_response_sample.json"))
        .expect("fixture should exist");
    let snap: UsageSnapshot = parse_usage_response(&raw).expect("should parse");

    let five = snap
        .five_hour
        .as_ref()
        .expect("five_hour should be present");
    // utilization is normalized to 0.0-1.0 after parsing
    assert!((five.utilization - 0.56).abs() < 1e-9);
    assert!(five.resets_at.is_some());

    let week = snap
        .seven_day
        .as_ref()
        .expect("seven_day should be present");
    assert!((week.utilization - 0.56).abs() < 1e-9);
    assert!(week.resets_at.is_some());
}

#[test]
fn parses_response_with_missing_bucket() {
    let raw = r#"{"five_hour": {"utilization": 12, "resets_at": "2026-01-01T00:00:00Z"}}"#;
    let snap = parse_usage_response(raw).expect("should parse");
    assert!(snap.five_hour.is_some());
    assert!(snap.seven_day.is_none());
}

#[test]
fn parses_response_with_null_resets_at() {
    let raw = r#"{"five_hour": {"utilization": 99, "resets_at": null}}"#;
    let snap = parse_usage_response(raw).expect("should parse");
    let five = snap.five_hour.unwrap();
    assert!((five.utilization - 0.99).abs() < 1e-9);
    assert!(five.resets_at.is_none());
}

#[test]
fn parse_caps_snapshot_full_builds_both_buckets() {
    let body = br#"{
        "sample_util_5h": 0.42,
        "sample_util_7d": 0.1,
        "resets_5h_iso": "2026-05-23T12:00:00+00:00",
        "resets_7d_iso": "2026-05-25T07:00:00+00:00"
    }"#;
    let snap = claude_usage_tray::api::usage::parse_caps_snapshot(body).unwrap();
    let five = snap.five_hour.expect("five_hour present");
    assert!((five.utilization - 0.42).abs() < 1e-9);
    assert!(five.resets_at.is_some());
    let seven = snap.seven_day.expect("seven_day present");
    assert!((seven.utilization - 0.1).abs() < 1e-9);
}

#[test]
fn parse_caps_snapshot_null_resets_keeps_util_drops_reset() {
    let body = br#"{"sample_util_5h": 0.5, "sample_util_7d": null, "resets_5h_iso": null, "resets_7d_iso": null}"#;
    let snap = claude_usage_tray::api::usage::parse_caps_snapshot(body).unwrap();
    let five = snap.five_hour.expect("five_hour present");
    assert!((five.utilization - 0.5).abs() < 1e-9);
    assert!(five.resets_at.is_none());
    // null util => no bucket at all.
    assert!(snap.seven_day.is_none());
}

#[test]
fn parse_caps_snapshot_missing_fields_yields_empty_snapshot() {
    // caps.json with no sample (the "no data yet" case) => both buckets None.
    let body = br#"{"subscription_type": "pro", "rate_limit_tier": "default"}"#;
    let snap = claude_usage_tray::api::usage::parse_caps_snapshot(body).unwrap();
    assert!(snap.five_hour.is_none());
    assert!(snap.seven_day.is_none());
}
