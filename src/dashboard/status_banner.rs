//! Live API status banner: a persistent read-only strip at the top of the
//! dashboard window. Shows the poll status badge, last-poll age, next-poll ETA,
//! and live 5h/7d utilization. Mirrors the `--watch` CLI footer in egui form.

use chrono::Duration;

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

#[cfg(test)]
mod tests {
    use super::*;

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
}
