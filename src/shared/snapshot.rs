//! Cross-thread snapshot of the app's state.

use crate::api::usage::UsageSnapshot;
use crate::calibration::anchors::DerivedCaps;
use crate::calibration::live::LiveUtil;
use crate::data::parser::Turn;
use crate::render::LastStatus;
use chrono::{DateTime, Utc};
use std::sync::Arc;

/// What the polling thread writes; what the dashboard reads.
#[derive(Debug, Clone, Default)]
pub struct AppSnapshot {
    pub turns: Arc<Vec<Turn>>,
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
}
