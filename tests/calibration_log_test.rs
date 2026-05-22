use chrono::{TimeZone, Utc};
use claude_usage_tray::api::credentials::Credentials;
use claude_usage_tray::api::usage::{UsageBucket, UsageSnapshot};
use claude_usage_tray::log::calibration::{append, sample_from, CalibrationSample};
use tempfile::TempDir;

fn fake_creds() -> Credentials {
    Credentials {
        access_token: "irrelevant".to_string(),
        subscription_type: "pro".to_string(),
        rate_limit_tier: "default_claude_ai".to_string(),
    }
}

fn fake_snapshot() -> UsageSnapshot {
    UsageSnapshot {
        five_hour: Some(UsageBucket {
            utilization: 0.56,
            resets_at: Some(Utc.with_ymd_and_hms(2026, 5, 22, 17, 0, 0).unwrap()),
        }),
        seven_day: Some(UsageBucket {
            utilization: 0.42,
            resets_at: Some(Utc.with_ymd_and_hms(2026, 5, 24, 5, 0, 0).unwrap()),
        }),
    }
}

#[test]
fn sample_from_maps_all_fields() {
    let snap = fake_snapshot();
    let creds = fake_creds();
    let s = sample_from(&snap, &creds);

    assert_eq!(s.schema_version, 1);
    assert!((s.five_hour_util.unwrap() - 0.56).abs() < 1e-9);
    assert!((s.seven_day_util.unwrap() - 0.42).abs() < 1e-9);
    assert!(s.five_hour_resets_at.is_some());
    assert!(s.seven_day_resets_at.is_some());
    assert_eq!(s.subscription_type, "pro");
    assert_eq!(s.rate_limit_tier, "default_claude_ai");
}

#[test]
fn sample_from_handles_missing_buckets() {
    let snap = UsageSnapshot {
        five_hour: None,
        seven_day: None,
    };
    let s = sample_from(&snap, &fake_creds());

    assert!(s.five_hour_util.is_none());
    assert!(s.five_hour_resets_at.is_none());
    assert!(s.seven_day_util.is_none());
    assert!(s.seven_day_resets_at.is_none());
}

#[test]
fn append_writes_one_line_per_call_and_round_trips() {
    let dir = TempDir::new().expect("tempdir");
    let path = dir.path().join("log.jsonl");

    let s1 = sample_from(&fake_snapshot(), &fake_creds());
    let s2 = sample_from(&fake_snapshot(), &fake_creds());

    append(&path, &s1).expect("first append");
    append(&path, &s2).expect("second append");

    let raw = std::fs::read_to_string(&path).expect("read back");
    let lines: Vec<&str> = raw.lines().collect();
    assert_eq!(lines.len(), 2, "expected exactly 2 lines, got: {raw}");

    let parsed: CalibrationSample = serde_json::from_str(lines[0]).expect("first line parses");
    assert_eq!(parsed.schema_version, 1);
    assert_eq!(parsed.subscription_type, "pro");
    assert!((parsed.five_hour_util.unwrap() - 0.56).abs() < 1e-9);
}

#[test]
fn append_creates_parent_directory_lazily() {
    let dir = TempDir::new().expect("tempdir");
    let nested = dir.path().join("does/not/exist/yet/log.jsonl");

    let s = sample_from(&fake_snapshot(), &fake_creds());
    append(&nested, &s).expect("should create dirs and write");

    assert!(
        nested.exists(),
        "expected file to exist at {}",
        nested.display()
    );
}
