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
pub fn clamp_x_range(now: DateTime<Utc>, range: Range) -> (DateTime<Utc>, DateTime<Utc>) {
    let end = now;
    let start = match range.duration() {
        Some(d) => end - d,
        None => end,
    };
    (start, end)
}
