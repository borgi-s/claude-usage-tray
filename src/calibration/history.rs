//! Calibration-history math: implied-cap series + per-hour statistics for the
//! dashboard's Calibration tab. Pure functions; UI lives in
//! `dashboard/calibration_tab.rs`.

use crate::calibration::anchors::{five_hour_burn_at, weekly_burn_at};
use crate::calibration::WindowKind;
use crate::config;
use crate::data::parser::Turn;
use crate::log::calibration::CalibrationSample;
use chrono::{DateTime, Timelike, Utc};
use chrono_tz::Tz;

/// (sample ts, implied cap in raw output tokens) for every sample that
/// qualifies as an anchor for `kind`: util present, within
/// `config::MIN_ANCHOR_UTIL..=MAX_ANCHOR_UTIL`, and window burn > 0.
fn qualifying_implied(
    log: &[CalibrationSample],
    turns: &[Turn],
    kind: WindowKind,
) -> Vec<(DateTime<Utc>, f64)> {
    let mut out = Vec::new();
    for s in log {
        let util_opt = match kind {
            WindowKind::FiveHour => s.five_hour_util,
            WindowKind::Weekly => s.seven_day_util,
        };
        let Some(util) = util_opt else { continue };
        if !(config::MIN_ANCHOR_UTIL..=config::MAX_ANCHOR_UTIL).contains(&util) {
            continue;
        }
        let burn = match kind {
            WindowKind::FiveHour => five_hour_burn_at(turns, s.ts),
            WindowKind::Weekly => weekly_burn_at(turns, s.ts),
        };
        if burn == 0 || util <= 0.0 {
            continue;
        }
        out.push((s.ts, burn as f64 / util));
    }
    out
}

/// One implied-cap observation derived from a single calibration sample.
#[derive(Debug, Clone)]
pub struct ImpliedPoint {
    pub ts: DateTime<Utc>,
    pub cap: f64,        // raw output tokens
    pub local_hour: u32, // 0..=23, local-TZ hour of `ts`
}

/// Implied cap per qualifying sample, sorted by ts.
pub fn implied_cap_series(
    log: &[CalibrationSample],
    turns: &[Turn],
    kind: WindowKind,
) -> Vec<ImpliedPoint> {
    let tz: Tz = config::LOCAL_TZ
        .parse()
        .expect("LOCAL_TZ must be a valid IANA name");
    let mut out: Vec<ImpliedPoint> = qualifying_implied(log, turns, kind)
        .into_iter()
        .map(|(ts, cap)| ImpliedPoint {
            ts,
            cap,
            local_hour: ts.with_timezone(&tz).hour(),
        })
        .collect();
    out.sort_by_key(|p| p.ts);
    out
}

/// Per-local-hour summary of implied caps across qualifying anchors.
#[derive(Debug, Clone, Default)]
pub struct HourStat {
    pub median: Option<f64>,
    pub p25: Option<f64>,
    pub p75: Option<f64>,
    pub n: usize,
}

/// Median / p25 / p75 / count of implied caps per local hour-of-day bin.
pub fn per_hour_stats(
    log: &[CalibrationSample],
    turns: &[Turn],
    kind: WindowKind,
) -> [HourStat; 24] {
    let tz: Tz = config::LOCAL_TZ
        .parse()
        .expect("LOCAL_TZ must be a valid IANA name");
    let mut buckets: [Vec<f64>; 24] = Default::default();
    for (ts, cap) in qualifying_implied(log, turns, kind) {
        let h = ts.with_timezone(&tz).hour() as usize;
        buckets[h].push(cap);
    }
    let mut out: [HourStat; 24] = Default::default();
    for (h, vals) in buckets.iter_mut().enumerate() {
        out[h] = HourStat {
            median: median(vals),
            p25: percentile(vals, 0.25),
            p75: percentile(vals, 0.75),
            n: vals.len(),
        };
    }
    out
}

/// Median of a slice (sorts in place). `None` if empty.
pub fn median(values: &mut [f64]) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let n = values.len();
    Some(if n % 2 == 1 {
        values[n / 2]
    } else {
        (values[n / 2 - 1] + values[n / 2]) / 2.0
    })
}

/// p-th percentile (`p` in 0.0..=1.0) via linear interpolation between order
/// statistics. Sorts in place. `None` if empty.
pub fn percentile(values: &mut [f64], p: f64) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let n = values.len();
    if n == 1 {
        return Some(values[0]);
    }
    let rank = p.clamp(0.0, 1.0) * (n - 1) as f64;
    let lo = rank.floor() as usize;
    let hi = rank.ceil() as usize;
    let frac = rank - lo as f64;
    Some(values[lo] + (values[hi] - values[lo]) * frac)
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::calibration::WindowKind;
    use crate::data::parser::Turn;
    use crate::log::calibration::CalibrationSample;
    use chrono::{DateTime, TimeZone, Utc};
    use std::path::PathBuf;

    fn utc(y: i32, m: u32, d: u32, h: u32, mi: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(y, m, d, h, mi, 0).unwrap()
    }

    fn turn(ts: DateTime<Utc>, output: u64) -> Turn {
        Turn {
            ts,
            session_id: String::new(),
            subagent_id: None,
            is_subagent: false,
            project_cwd: String::new(),
            model: String::new(),
            version: String::new(),
            input_tokens: 0,
            output_tokens: output,
            cache_creation_input_tokens: 0,
            cache_read_input_tokens: 0,
            source_file: PathBuf::new(),
            is_rate_limit_error: false,
        }
    }

    fn sample(ts: DateTime<Utc>, util: f64) -> CalibrationSample {
        CalibrationSample {
            schema_version: 1,
            ts,
            five_hour_util: Some(util),
            five_hour_resets_at: None,
            seven_day_util: Some(util),
            seven_day_resets_at: None,
            subscription_type: "pro".into(),
            rate_limit_tier: "default".into(),
        }
    }

    #[test]
    fn implied_filters_util_range_and_computes_cap() {
        // Anchor 2026-05-24 14:00 UTC, util 1.0, one prior turn of 100 output
        // tokens in the same 5h window => implied cap = 100 / 1.0 = 100.
        let turns = vec![turn(utc(2026, 5, 24, 13, 0), 100)];
        let log = vec![
            sample(utc(2026, 5, 24, 14, 0), 1.0),  // qualifies
            sample(utc(2026, 5, 24, 15, 0), 0.5),  // util too low => excluded
            sample(utc(2026, 5, 24, 16, 0), 1.2),  // util too high => excluded
        ];
        let pts = implied_cap_series(&log, &turns, WindowKind::FiveHour);
        assert_eq!(pts.len(), 1);
        assert!((pts[0].cap - 100.0).abs() < 1e-9);
    }

    #[test]
    fn implied_drops_zero_burn_windows() {
        // Util qualifies but there are no turns => burn 0 => dropped.
        let log = vec![sample(utc(2026, 5, 24, 14, 0), 1.0)];
        let pts = implied_cap_series(&log, &[], WindowKind::FiveHour);
        assert!(pts.is_empty());
    }

    #[test]
    fn implied_local_hour_is_local_not_utc() {
        // 14:00 UTC = 16:00 local (Europe/Copenhagen, CEST).
        let turns = vec![turn(utc(2026, 5, 24, 13, 0), 100)];
        let log = vec![sample(utc(2026, 5, 24, 14, 0), 1.0)];
        let pts = implied_cap_series(&log, &turns, WindowKind::FiveHour);
        assert_eq!(pts[0].local_hour, 16);
    }

    #[test]
    fn implied_empty_log_is_empty() {
        assert!(implied_cap_series(&[], &[], WindowKind::FiveHour).is_empty());
    }

    #[test]
    fn median_handles_odd_even_empty() {
        assert_eq!(median(&mut []), None);
        assert_eq!(median(&mut [5.0]), Some(5.0));
        assert_eq!(median(&mut [3.0, 1.0, 2.0]), Some(2.0));
        assert_eq!(median(&mut [4.0, 1.0, 3.0, 2.0]), Some(2.5));
    }

    #[test]
    fn percentile_interpolates_between_order_stats() {
        assert_eq!(percentile(&mut [], 0.5), None);
        assert_eq!(percentile(&mut [10.0], 0.25), Some(10.0));
        assert_eq!(percentile(&mut [0.0, 1.0, 2.0, 3.0, 4.0], 0.5), Some(2.0));
        assert_eq!(percentile(&mut [0.0, 1.0, 2.0, 3.0, 4.0], 0.25), Some(1.0));
        assert_eq!(percentile(&mut [0.0, 1.0, 2.0, 3.0, 4.0], 0.75), Some(3.0));
        // Two values, p25 interpolates: 0 + (10-0)*0.25 = 2.5
        assert_eq!(percentile(&mut [0.0, 10.0], 0.25), Some(2.5));
    }

    #[test]
    fn per_hour_stats_percentiles_across_samples() {
        // Three separate days, each one turn before a 14:00-UTC (16:00 local)
        // anchor at util 1.0. Day gaps exceed the 4.5h window, so each window's
        // burn is that day's single turn => implied caps 100/200/300 in bin 16.
        let turns = vec![
            turn(utc(2026, 5, 18, 13, 0), 100),
            turn(utc(2026, 5, 19, 13, 0), 200),
            turn(utc(2026, 5, 20, 13, 0), 300),
        ];
        let log = vec![
            sample(utc(2026, 5, 18, 14, 0), 1.0),
            sample(utc(2026, 5, 19, 14, 0), 1.0),
            sample(utc(2026, 5, 20, 14, 0), 1.0),
        ];
        let stats = per_hour_stats(&log, &turns, WindowKind::FiveHour);
        let s = &stats[16];
        assert_eq!(s.n, 3);
        assert_eq!(s.median, Some(200.0));
        assert_eq!(s.p25, Some(150.0));
        assert_eq!(s.p75, Some(250.0));
    }

    #[test]
    fn per_hour_stats_empty_bins_are_default() {
        let stats = per_hour_stats(&[], &[], WindowKind::FiveHour);
        for s in &stats {
            assert!(s.median.is_none());
            assert!(s.p25.is_none());
            assert!(s.p75.is_none());
            assert_eq!(s.n, 0);
        }
    }

    #[test]
    fn per_hour_stats_median_agrees_with_hourly_per_hour_medians() {
        let turns = vec![
            turn(utc(2026, 5, 18, 13, 0), 100),
            turn(utc(2026, 5, 19, 13, 0), 300),
        ];
        let log = vec![
            sample(utc(2026, 5, 18, 14, 0), 1.0),
            sample(utc(2026, 5, 19, 14, 0), 1.0),
        ];
        let stats = per_hour_stats(&log, &turns, WindowKind::FiveHour);
        let raw = crate::calibration::hourly::per_hour_medians(&log, &turns, WindowKind::FiveHour);
        assert_eq!(stats[16].median, raw[16]);
    }
}
