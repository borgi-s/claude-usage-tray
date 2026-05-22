use claude_usage_tray::api::usage::{parse_usage_response, UsageSnapshot};
use std::path::Path;

#[test]
fn parses_full_response() {
    let raw = std::fs::read_to_string(Path::new("tests/fixtures/usage_response_sample.json"))
        .expect("fixture should exist");
    let snap: UsageSnapshot = parse_usage_response(&raw).expect("should parse");

    let five = snap.five_hour.as_ref().expect("five_hour should be present");
    // utilization is normalized to 0.0-1.0 after parsing
    assert!((five.utilization - 0.56).abs() < 1e-9);
    assert!(five.resets_at.is_some());

    let week = snap.seven_day.as_ref().expect("seven_day should be present");
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
