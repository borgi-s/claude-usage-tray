use crate::api::credentials::Credentials;
use crate::api::usage::{UsageBucket, UsageSnapshot};
use chrono::{DateTime, Duration, Utc};
use std::fmt::Write;

/// Result of running one render pass. `body` is the printable text;
/// `line_count` is the number of lines `body` occupies (= number of `\n` chars).
/// The watch loop uses `line_count` to compute the ANSI cursor-up escape for
/// the next frame.
#[derive(Debug, Clone)]
pub struct Frame {
    pub body: String,
    pub line_count: u16,
}

/// Status badge shown in the footer of each frame.
#[derive(Debug, Clone)]
pub enum LastStatus {
    /// Before the first poll completes.
    Initial,
    /// Most recent poll succeeded.
    Ok,
    /// Most recent poll was HTTP 429.
    RateLimited,
    /// Most recent poll failed with some other error.
    Error(String),
}

/// Pure: build the on-screen frame for one tick.
/// - `last_success`: the most recent successful sample + when it was received, or None if no poll has succeeded yet.
/// - `interval_secs`: polling cadence, shown in the header.
/// - `status`: badge for the footer.
/// - `now`: current time (passed in for testability — production passes `Utc::now()`).
pub fn draw_frame(
    last_success: Option<&(UsageSnapshot, DateTime<Utc>)>,
    creds: &Credentials,
    interval_secs: u64,
    status: &LastStatus,
    now: DateTime<Utc>,
) -> Frame {
    let mut body = String::new();
    let mut lines: u16 = 0;

    // Header.
    writeln!(
        body,
        "claude-usage-tray  watching ({}s)  press Ctrl-C to quit",
        interval_secs
    )
    .unwrap();
    lines += 1;
    writeln!(body).unwrap();
    lines += 1;

    // Body.
    match last_success {
        Some((snap, _)) => {
            writeln!(body, "  5h: {}", format_bucket_opt(snap.five_hour.as_ref(), now)).unwrap();
            writeln!(body, "  7d: {}", format_bucket_opt(snap.seven_day.as_ref(), now)).unwrap();
            lines += 2;
        }
        None => {
            writeln!(body, "  5h: (fetching\u{2026})").unwrap();
            writeln!(body, "  7d: (fetching\u{2026})").unwrap();
            lines += 2;
        }
    }
    writeln!(
        body,
        "  sub: {} / tier: {}",
        creds.subscription_type, creds.rate_limit_tier
    )
    .unwrap();
    lines += 1;

    // Footer.
    let footer = format_footer(last_success.map(|(_, t)| *t), interval_secs, status, now);
    writeln!(body, "  {}", footer).unwrap();
    lines += 1;

    Frame { body, line_count: lines }
}

fn format_bucket_opt(b: Option<&UsageBucket>, now: DateTime<Utc>) -> String {
    match b {
        Some(bucket) => format_bucket(bucket, now),
        None => "(no data)".to_string(),
    }
}

fn format_bucket(b: &UsageBucket, now: DateTime<Utc>) -> String {
    let pct = (b.utilization * 100.0).round() as i64;
    match b.resets_at {
        Some(when) => format!("{}% resets in {}", pct, format_duration(when - now)),
        None => format!("{}% (no reset time)", pct),
    }
}

pub fn format_duration(d: Duration) -> String {
    let secs = d.num_seconds().max(0);
    let days = secs / 86_400;
    let hours = (secs % 86_400) / 3600;
    let mins = (secs % 3600) / 60;
    if days > 0 {
        format!("{}d {}h", days, hours)
    } else if hours > 0 {
        format!("{}h {}m", hours, mins)
    } else {
        format!("{}m", mins)
    }
}

fn format_footer(
    last_sample_at: Option<DateTime<Utc>>,
    interval_secs: u64,
    status: &LastStatus,
    now: DateTime<Utc>,
) -> String {
    let last_str = match last_sample_at {
        Some(t) => format!("last poll: {}", t.format("%H:%M:%S")),
        None => "last poll: \u{2014}".to_string(),
    };
    let next_str = match last_sample_at {
        Some(t) => {
            let next = t + Duration::seconds(interval_secs as i64);
            format!("next: {}", next.format("%H:%M:%S"))
        }
        None => "next: \u{2014}".to_string(),
    };

    let badge = match status {
        LastStatus::Initial => "[fetching\u{2026}]".to_string(),
        LastStatus::Ok => "[Ok]".to_string(),
        LastStatus::RateLimited => {
            let age = last_sample_at
                .map(|t| format_duration(now - t))
                .unwrap_or_else(|| "?".to_string());
            format!("[stale {} \u{00B7} rate-limited]", age)
        }
        LastStatus::Error(msg) => {
            let age = last_sample_at
                .map(|t| format_duration(now - t))
                .unwrap_or_else(|| "?".to_string());
            format!("[stale {} \u{00B7} error: {}]", age, msg)
        }
    };

    format!("{}  {}  {}", last_str, next_str, badge)
}
