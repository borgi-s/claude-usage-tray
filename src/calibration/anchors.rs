//! Median-of-anchors cap derivation.

use chrono::{DateTime, Utc};

/// Caps derived from the latest calibration log + cache.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DerivedCaps {
    pub cap_5h: Option<f64>,
    pub cap_week: Option<f64>,
    pub n_anchors_5h: usize,
    pub n_anchors_week: usize,
}

// Subsequent tasks add `last_weekly_reset`, `five_hour_burn_at`,
// `weekly_burn_at`, `global_cap_from_anchors`.
#[allow(dead_code)]
fn _placeholder(_t: DateTime<Utc>) {}
