//! Per-turn cumulative-share series for the dashboard charts.

use crate::config::FIVE_HOUR_WINDOW_HOURS;
use crate::data::parser::Turn;
use chrono::{DateTime, Duration, Utc};

#[derive(Debug, Clone, PartialEq)]
pub struct WindowedTurn {
    pub ts: DateTime<Utc>,
    pub cumulative_share: f64,
    pub window_idx: usize,
}

/// Per-turn cumulative share across gap-detected 5h windows. If `cap` is None,
/// returns raw cumulative output tokens (not normalized).
pub fn cumulative_share_series_5h(turns: &[Turn], cap: Option<f64>) -> Vec<WindowedTurn> {
    let gap = Duration::milliseconds((FIVE_HOUR_WINDOW_HOURS * 3_600_000.0) as i64);
    let mut out: Vec<WindowedTurn> = Vec::with_capacity(turns.len());
    let mut current_start: Option<DateTime<Utc>> = None;
    let mut last_ts: Option<DateTime<Utc>> = None;
    let mut window_idx: usize = 0;
    let mut burn_in_window: u64 = 0;

    for t in turns {
        match (current_start, last_ts) {
            (None, _) => {
                current_start = Some(t.ts);
            }
            (Some(start), Some(prev)) => {
                let since_last = t.ts - prev;
                let since_start = t.ts - start;
                if since_last >= gap || since_start >= gap {
                    current_start = Some(t.ts);
                    burn_in_window = 0;
                    window_idx += 1;
                }
            }
            (Some(_), None) => unreachable!("current_start implies last_ts"),
        }
        burn_in_window += t.output_tokens;
        let share = match cap {
            Some(c) if c > 0.0 => burn_in_window as f64 / c,
            _ => burn_in_window as f64,
        };
        out.push(WindowedTurn {
            ts: t.ts,
            cumulative_share: share,
            window_idx,
        });
        last_ts = Some(t.ts);
    }
    out
}
