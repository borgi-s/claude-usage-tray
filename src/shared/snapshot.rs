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
