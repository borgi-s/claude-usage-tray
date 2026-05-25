//! Cross-thread snapshot of the app's state.

use crate::api::usage::UsageSnapshot;
use crate::calibration::anchors::DerivedCaps;
use crate::calibration::live::LiveUtil;
use crate::data::parser::Turn;
use crate::log::calibration::CalibrationSample;
use crate::render::LastStatus;
use chrono::{DateTime, Utc};
use std::sync::Arc;

/// What the polling thread writes; what the dashboard reads.
#[derive(Debug, Clone, Default)]
pub struct AppSnapshot {
    pub turns: Arc<Vec<Turn>>,
    pub log: Arc<Vec<CalibrationSample>>,
    pub caps: DerivedCaps,
    pub hourly_5h: [f64; 24],
    pub hourly_week: [f64; 24],
    pub live_util: LiveUtil,
    pub last_sample: Option<(UsageSnapshot, DateTime<Utc>)>,
    pub last_status: LastStatus,
    pub kpis: DashboardKpis,
}

/// Pre-computed KPIs so the dashboard doesn't recompute them every frame.
#[derive(Debug, Clone, Default)]
pub struct DashboardKpis {
    pub peak_5h_share: f64,
    pub peak_week_share: f64,
    pub total_cost_weighted: f64,
    pub daily_avg_cost_weighted: f64,
}

use crate::config;

/// Heuristic cost-weighted token count for a single turn. Used by the
/// dashboard's "total burn" KPI + daily bar chart. NOT used for cap math.
pub fn cost_weighted(turn: &Turn) -> f64 {
    turn.input_tokens as f64 * config::COST_WEIGHT_INPUT
        + turn.cache_creation_input_tokens as f64 * config::COST_WEIGHT_CACHE_CREATION
        + turn.cache_read_input_tokens as f64 * config::COST_WEIGHT_CACHE_READ
        + turn.output_tokens as f64 * config::COST_WEIGHT_OUTPUT
}

use crate::calibration::anchors::{peak_five_hour_burn, peak_weekly_burn};

/// Compute all four KPIs from the turns + caps. Called once per poll.
pub fn compute_kpis(turns: &[Turn], caps: &DerivedCaps) -> DashboardKpis {
    let total_cw: f64 = turns.iter().map(cost_weighted).sum();
    let daily_avg = if turns.len() < 2 {
        total_cw // sub-day data: report total as daily avg
    } else {
        let first = turns.first().unwrap().ts;
        let last = turns.last().unwrap().ts;
        let span_days = ((last - first).num_seconds() as f64 / 86_400.0).max(1.0);
        total_cw / span_days
    };
    DashboardKpis {
        peak_5h_share: peak_5h_share(turns, caps),
        peak_week_share: peak_week_share(turns, caps),
        total_cost_weighted: total_cw,
        daily_avg_cost_weighted: daily_avg,
    }
}

/// Max cumulative-share across any 5h window, or 0.0 if cap_5h is None.
fn peak_5h_share(turns: &[Turn], caps: &DerivedCaps) -> f64 {
    let Some(cap) = caps.cap_5h else { return 0.0 };
    if cap <= 0.0 {
        return 0.0;
    }
    peak_five_hour_burn(turns) as f64 / cap
}

/// Max cumulative-share across any weekly window, or 0.0 if cap_week is None.
fn peak_week_share(turns: &[Turn], caps: &DerivedCaps) -> f64 {
    let Some(cap) = caps.cap_week else { return 0.0 };
    if cap <= 0.0 {
        return 0.0;
    }
    peak_weekly_burn(turns) as f64 / cap
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn turn(input: u64, cc: u64, cr: u64, output: u64) -> Turn {
        Turn {
            ts: chrono::Utc::now(),
            session_id: String::new(),
            subagent_id: None,
            is_subagent: false,
            project_cwd: String::new(),
            model: String::new(),
            version: String::new(),
            input_tokens: input,
            output_tokens: output,
            cache_creation_input_tokens: cc,
            cache_read_input_tokens: cr,
            source_file: PathBuf::new(),
            is_rate_limit_error: false,
        }
    }

    #[test]
    fn cost_weighted_applies_each_coefficient() {
        // Input=100, cache_create=200, cache_read=300, output=400.
        // Expected: 100*1 + 200*1.25 + 300*0.1 + 400*5 = 100 + 250 + 30 + 2000 = 2380.
        let t = turn(100, 200, 300, 400);
        assert!((cost_weighted(&t) - 2380.0).abs() < 0.001);
    }

    use chrono::TimeZone;
    fn utc(y: i32, m: u32, d: u32, h: u32, mi: u32) -> chrono::DateTime<chrono::Utc> {
        chrono::Utc.with_ymd_and_hms(y, m, d, h, mi, 0).unwrap()
    }
    fn turn_at(ts: chrono::DateTime<chrono::Utc>, output: u64) -> Turn {
        let mut t = turn(0, 0, 0, output);
        t.ts = ts;
        t
    }

    #[test]
    fn compute_kpis_peak_5h_share_max_across_windows() {
        // Two 5h windows. Window 1 (10:00-12:00): 100+200+300 = 600 output.
        // Window 2 (18:00): 100 output. 6h gap so they're separate.
        // cap_5h = 1000. Peak share = 600/1000 = 0.6.
        let turns = vec![
            turn_at(utc(2026, 5, 24, 10, 0), 100),
            turn_at(utc(2026, 5, 24, 11, 0), 200),
            turn_at(utc(2026, 5, 24, 12, 0), 300),
            turn_at(utc(2026, 5, 24, 18, 0), 100), // 6h gap → new window
        ];
        let caps = DerivedCaps {
            cap_5h: Some(1000.0),
            cap_week: None,
            n_anchors_5h: 1,
            n_anchors_week: 0,
        };
        let k = compute_kpis(&turns, &caps);
        assert!((k.peak_5h_share - 0.6).abs() < 0.001);
    }

    #[test]
    fn compute_kpis_peak_share_zero_when_cap_none() {
        let turns = vec![turn_at(utc(2026, 5, 24, 10, 0), 100)];
        let caps = DerivedCaps::default(); // both caps None
        let k = compute_kpis(&turns, &caps);
        assert_eq!(k.peak_5h_share, 0.0);
        assert_eq!(k.peak_week_share, 0.0);
    }

    #[test]
    fn compute_kpis_total_and_daily_avg_cost_weighted() {
        // 3 turns on 2026-05-24, 1 turn on 2026-05-25 → span 1 day.
        // Each turn: 1 input, 1 cache_create, 1 cache_read, 1 output.
        // cost_weighted per turn = 1*1 + 1*1.25 + 1*0.1 + 1*5 = 7.35.
        let turns_raw = vec![
            turn_at(utc(2026, 5, 24, 10, 0), 1),
            turn_at(utc(2026, 5, 24, 11, 0), 1),
            turn_at(utc(2026, 5, 24, 12, 0), 1),
            turn_at(utc(2026, 5, 25, 10, 0), 1),
        ];
        // Patch input/cache_create/cache_read = 1 for each.
        let turns: Vec<Turn> = turns_raw
            .into_iter()
            .map(|mut t| {
                t.input_tokens = 1;
                t.cache_creation_input_tokens = 1;
                t.cache_read_input_tokens = 1;
                t
            })
            .collect();
        let caps = DerivedCaps::default();
        let k = compute_kpis(&turns, &caps);
        // total = 4 * 7.35 = 29.4
        assert!((k.total_cost_weighted - 29.4).abs() < 0.01);
        // first=24 10:00, last=25 10:00 → span = exactly 1 day.
        // daily_avg = total / span_days = 29.4 / 1.0 = 29.4
        assert!((k.daily_avg_cost_weighted - 29.4).abs() < 0.01);
    }

    #[test]
    fn compute_kpis_empty_turns_returns_zeros() {
        let k = compute_kpis(&[], &DerivedCaps::default());
        assert_eq!(k.peak_5h_share, 0.0);
        assert_eq!(k.total_cost_weighted, 0.0);
        assert_eq!(k.daily_avg_cost_weighted, 0.0);
    }
}
