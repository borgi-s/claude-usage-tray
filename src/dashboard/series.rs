//! Per-turn cumulative-share series for the dashboard charts.

use crate::calibration::anchors::last_weekly_reset;
use crate::config::FIVE_HOUR_WINDOW_HOURS;
use crate::data::parser::Turn;
use crate::settings::CalParams;
use chrono::{DateTime, Duration, NaiveDate, Utc};
use chrono_tz::Tz;
use std::collections::BTreeMap;

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

/// Per-turn cumulative share within each fixed Sunday-07:00-local week.
pub fn cumulative_share_series_weekly(turns: &[Turn], cap: Option<f64>, cp: CalParams) -> Vec<WindowedTurn> {
    let mut out: Vec<WindowedTurn> = Vec::with_capacity(turns.len());
    let mut current_reset: Option<DateTime<Utc>> = None;
    let mut window_idx: usize = 0;
    let mut burn_in_window: u64 = 0;

    for t in turns {
        let this_reset = last_weekly_reset(t.ts, cp);
        match current_reset {
            None => {
                current_reset = Some(this_reset);
            }
            Some(prev) if prev != this_reset => {
                current_reset = Some(this_reset);
                burn_in_window = 0;
                window_idx += 1;
            }
            _ => {}
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
    }
    out
}

/// Sum cost-weighted tokens per local-date, returned in ascending date order.
pub fn daily_aggregates(turns: &[Turn], w: &crate::settings::CostWeights) -> Vec<(NaiveDate, f64)> {
    use crate::shared::snapshot::cost_weighted;

    let tz: Tz = crate::config::LOCAL_TZ
        .parse()
        .expect("LOCAL_TZ must be valid IANA name");
    let mut map: BTreeMap<NaiveDate, f64> = BTreeMap::new();
    for t in turns {
        let local_date = t.ts.with_timezone(&tz).date_naive();
        *map.entry(local_date).or_default() += cost_weighted(t, w);
    }
    map.into_iter().collect()
}
