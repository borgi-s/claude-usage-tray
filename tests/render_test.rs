use chrono::{TimeZone, Utc};
use claude_usage_tray::api::credentials::Credentials;
use claude_usage_tray::api::usage::{UsageBucket, UsageSnapshot};
use claude_usage_tray::render::{draw_frame, LastStatus};

fn fake_creds() -> Credentials {
    Credentials {
        access_token: "x".to_string(),
        subscription_type: "pro".to_string(),
        rate_limit_tier: "default_claude_ai".to_string(),
    }
}

fn fake_snapshot() -> UsageSnapshot {
    UsageSnapshot {
        five_hour: Some(UsageBucket {
            utilization: 0.57,
            // 2 hours, 12 minutes from `now` below
            resets_at: Some(Utc.with_ymd_and_hms(2026, 5, 22, 16, 36, 1).unwrap()),
        }),
        seven_day: Some(UsageBucket {
            utilization: 0.57,
            // 1 day, 21 hours from `now` below
            resets_at: Some(Utc.with_ymd_and_hms(2026, 5, 24, 11, 24, 1).unwrap()),
        }),
    }
}

#[test]
fn ok_frame_includes_percent_and_reset_countdown_and_status_tag() {
    let creds = fake_creds();
    let snap = fake_snapshot();
    let now = Utc.with_ymd_and_hms(2026, 5, 22, 14, 24, 1).unwrap();
    let f = draw_frame(Some(&(snap, now)), &creds, 120, &LastStatus::Ok, now);

    assert!(f.body.contains("5h: 57%"), "body was:\n{}", f.body);
    assert!(f.body.contains("2h 12m"), "body was:\n{}", f.body);
    assert!(f.body.contains("7d: 57%"), "body was:\n{}", f.body);
    assert!(f.body.contains("1d 21h"), "body was:\n{}", f.body);
    assert!(f.body.contains("sub: pro / tier: default_claude_ai"));
    assert!(f.body.contains("[Ok]"));
    assert!(f.line_count >= 5);
}

#[test]
fn initial_frame_shows_fetching_placeholder() {
    let creds = fake_creds();
    let now = Utc.with_ymd_and_hms(2026, 5, 22, 14, 24, 1).unwrap();
    let f = draw_frame(None, &creds, 120, &LastStatus::Initial, now);

    assert!(f.body.contains("fetching"));
    // Even when no sample is available, header + footer + sub line should print.
    assert!(f.line_count >= 3);
}

#[test]
fn rate_limited_status_shows_stale_footer_with_last_good_sample() {
    let creds = fake_creds();
    let snap = fake_snapshot();
    let sample_taken_at = Utc.with_ymd_and_hms(2026, 5, 22, 14, 22, 1).unwrap();
    let now = Utc.with_ymd_and_hms(2026, 5, 22, 14, 24, 1).unwrap(); // 2 min later
    let f = draw_frame(
        Some(&(snap, sample_taken_at)),
        &creds,
        120,
        &LastStatus::RateLimited,
        now,
    );

    assert!(
        f.body.contains("5h: 57%"),
        "should still show the cached sample"
    );
    assert!(f.body.contains("stale"), "footer should indicate staleness");
    assert!(f.body.contains("rate-limited"), "footer should explain why");
}
