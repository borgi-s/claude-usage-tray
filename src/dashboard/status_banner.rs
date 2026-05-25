//! Live API status banner: a persistent read-only strip at the top of the
//! dashboard window. Shows the poll status badge, last-poll age, next-poll ETA,
//! and live 5h/7d utilization. Mirrors the `--watch` CLI footer in egui form.

use crate::api::usage::{UsageBucket, UsageSnapshot};
use crate::render::LastStatus;
use chrono::{DateTime, Duration, Utc};

/// Format a poll's age at seconds resolution: `12s ago`, `1m 5s ago`,
/// `1h 1m ago`. Negative spans clamp to `0s ago`.
fn format_age(d: Duration) -> String {
    let secs = d.num_seconds().max(0);
    if secs < 60 {
        format!("{}s ago", secs)
    } else if secs < 3600 {
        format!("{}m {}s ago", secs / 60, secs % 60)
    } else {
        format!("{}h {}m ago", secs / 3600, (secs % 3600) / 60)
    }
}

/// Format the time until the next poll at seconds resolution: `48s`, `2m 5s`.
/// Always less than the poll interval; negative spans clamp to `0s`.
fn format_eta(d: Duration) -> String {
    let secs = d.num_seconds().max(0);
    if secs < 60 {
        format!("{}s", secs)
    } else {
        format!("{}m {}s", secs / 60, secs % 60)
    }
}

/// Build the `5h … · 7d …` utilization string. `None` (no poll yet) shows
/// both windows as em-dashes.
fn util_line(sample: Option<&UsageSnapshot>, now: DateTime<Utc>) -> String {
    match sample {
        None => "5h \u{2014} \u{00B7} 7d \u{2014}".to_string(),
        Some(snap) => format!(
            "5h {} \u{00B7} 7d {}",
            bucket_str(snap.five_hour.as_ref(), now),
            bucket_str(snap.seven_day.as_ref(), now),
        ),
    }
}

/// One bucket: `43%`, or `43% (resets 2h 10m)` when a reset time is known,
/// or `—` when the bucket is absent.
fn bucket_str(b: Option<&UsageBucket>, now: DateTime<Utc>) -> String {
    match b {
        None => "\u{2014}".to_string(),
        Some(bucket) => {
            let pct = (bucket.utilization * 100.0).round() as i64;
            match bucket.resets_at {
                Some(when) => {
                    format!("{}% (resets {})", pct, crate::render::format_duration(when - now))
                }
                None => format!("{}%", pct),
            }
        }
    }
}

/// Visual severity of the current poll status — drives the strip background.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Severity {
    /// Initial or Ok: neutral strip (color would be visual noise when nothing's wrong).
    Neutral,
    /// Rate-limited: amber strip.
    Warn,
    /// Error: red strip.
    Error,
}

fn severity(status: &LastStatus) -> Severity {
    match status {
        LastStatus::Initial | LastStatus::Ok => Severity::Neutral,
        LastStatus::RateLimited => Severity::Warn,
        LastStatus::Error(_) => Severity::Error,
    }
}

/// Text shown next to the status dot. Empty for `Ok` — the green dot is enough.
fn badge_label(status: &LastStatus) -> String {
    match status {
        LastStatus::Initial => "fetching\u{2026}".to_string(),
        LastStatus::Ok => String::new(),
        LastStatus::RateLimited => "rate-limited".to_string(),
        LastStatus::Error(msg) => format!("error: {}", msg),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn now_fixed() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 5, 25, 12, 0, 0).unwrap()
    }

    #[test]
    fn format_age_buckets() {
        assert_eq!(format_age(Duration::seconds(5)), "5s ago");
        assert_eq!(format_age(Duration::seconds(0)), "0s ago");
        assert_eq!(format_age(Duration::seconds(65)), "1m 5s ago");
        assert_eq!(format_age(Duration::seconds(3700)), "1h 1m ago");
        // Negative (clock skew / future timestamp) clamps to zero.
        assert_eq!(format_age(Duration::seconds(-10)), "0s ago");
    }

    #[test]
    fn format_eta_buckets() {
        assert_eq!(format_eta(Duration::seconds(48)), "48s");
        assert_eq!(format_eta(Duration::seconds(125)), "2m 5s");
        // Next-poll time already past → clamp to zero.
        assert_eq!(format_eta(Duration::seconds(-3)), "0s");
    }

    #[test]
    fn util_line_no_sample_shows_dashes() {
        assert_eq!(util_line(None, now_fixed()), "5h \u{2014} \u{00B7} 7d \u{2014}");
    }

    #[test]
    fn util_line_both_buckets_with_and_without_reset() {
        let snap = UsageSnapshot {
            // 43%, resets in 2h 10m.
            five_hour: Some(UsageBucket {
                utilization: 0.43,
                resets_at: Some(now_fixed() + Duration::minutes(130)),
            }),
            // 71%, no reset time.
            seven_day: Some(UsageBucket {
                utilization: 0.71,
                resets_at: None,
            }),
        };
        assert_eq!(
            util_line(Some(&snap), now_fixed()),
            "5h 43% (resets 2h 10m) \u{00B7} 7d 71%"
        );
    }

    #[test]
    fn util_line_missing_bucket_shows_dash() {
        let snap = UsageSnapshot {
            five_hour: Some(UsageBucket {
                utilization: 0.50,
                resets_at: None,
            }),
            seven_day: None,
        };
        assert_eq!(
            util_line(Some(&snap), now_fixed()),
            "5h 50% \u{00B7} 7d \u{2014}"
        );
    }

    #[test]
    fn severity_maps_each_status() {
        assert_eq!(severity(&LastStatus::Initial), Severity::Neutral);
        assert_eq!(severity(&LastStatus::Ok), Severity::Neutral);
        assert_eq!(severity(&LastStatus::RateLimited), Severity::Warn);
        assert_eq!(severity(&LastStatus::Error("x".into())), Severity::Error);
    }

    #[test]
    fn badge_label_per_status() {
        assert_eq!(badge_label(&LastStatus::Initial), "fetching\u{2026}");
        assert_eq!(badge_label(&LastStatus::Ok), "");
        assert_eq!(badge_label(&LastStatus::RateLimited), "rate-limited");
        assert_eq!(badge_label(&LastStatus::Error("boom".into())), "error: boom");
    }
}
