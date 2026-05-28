//! Range selector buttons (1D / 5D / 14D / 1M / All) above each chart.

use chrono::{DateTime, Duration, Utc};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Range {
    D1,
    D5,
    D14,
    M1,
    All,
}

impl Range {
    pub fn label(&self) -> &'static str {
        match self {
            Range::D1 => "1D",
            Range::D5 => "5D",
            Range::D14 => "14D",
            Range::M1 => "1M",
            Range::All => "All",
        }
    }

    pub fn duration(&self) -> Option<Duration> {
        match self {
            Range::D1 => Some(Duration::days(1)),
            Range::D5 => Some(Duration::days(5)),
            Range::D14 => Some(Duration::days(14)),
            Range::M1 => Some(Duration::days(30)),
            Range::All => None,
        }
    }

    pub const VARIANTS: &'static [Range] =
        &[Range::D1, Range::D5, Range::D14, Range::M1, Range::All];
}

/// Returns (start, end). For `Range::All`, returns (now, now); caller substitutes
/// turns.first().ts for the actual data-start time.
///
/// Callers should additionally clamp `end` to the last data point via
/// [`clamp_end_to_data`] so calendar bands (drawn against this explicit range)
/// don't extend past the plotted data into blank chart area when the dataset is
/// stale.
pub fn clamp_x_range(now: DateTime<Utc>, range: Range) -> (DateTime<Utc>, DateTime<Utc>) {
    let end = now;
    let start = match range.duration() {
        Some(d) => end - d,
        None => end,
    };
    (start, end)
}

/// Clamp the band/window end to the last turn's timestamp, if any. Keeps the
/// manually-drawn calendar bands aligned with egui_plot's data-driven autoscale
/// (which fits to the actual data extent, not to `now`).
pub fn clamp_end_to_data(x_end: DateTime<Utc>, last_ts: Option<DateTime<Utc>>) -> DateTime<Utc> {
    match last_ts {
        Some(ts) => x_end.min(ts),
        None => x_end,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clamp_end_to_data_caps_at_last_point_when_stale() {
        let now = "2026-05-28T12:00:00Z".parse::<DateTime<Utc>>().unwrap();
        let last = "2026-05-20T08:00:00Z".parse::<DateTime<Utc>>().unwrap();
        // Stale data: end is pulled back to the last turn, not left at `now`.
        assert_eq!(clamp_end_to_data(now, Some(last)), last);
    }

    #[test]
    fn clamp_end_to_data_keeps_now_when_data_is_fresh() {
        let now = "2026-05-28T12:00:00Z".parse::<DateTime<Utc>>().unwrap();
        let last = "2026-05-28T11:59:00Z".parse::<DateTime<Utc>>().unwrap();
        assert_eq!(clamp_end_to_data(now, Some(last)), now.min(last));
        // No data → unchanged.
        assert_eq!(clamp_end_to_data(now, None), now);
    }
}
