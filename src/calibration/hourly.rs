//! 24-bin hour-of-day cap series. Built ahead of Stage 6; not displayed in v0.5.0.

use crate::calibration::anchors::{five_hour_burn_at, weekly_burn_at};
use crate::calibration::WindowKind;
use crate::config;
use crate::data::parser::Turn;
use crate::log::calibration::CalibrationSample;
use chrono::Timelike;
use chrono_tz::Tz;

/// One implied cap per local hour-of-day, computed as median across anchors
/// whose timestamp falls in that bin. Bins with no anchors are `None`.
pub fn per_hour_medians(
    log: &[CalibrationSample],
    turns: &[Turn],
    kind: WindowKind,
) -> [Option<f64>; 24] {
    let tz: Tz = config::LOCAL_TZ
        .parse()
        .expect("LOCAL_TZ must be a valid IANA name");
    let mut buckets: [Vec<f64>; 24] = Default::default();

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
        let implied = burn as f64 / util;
        let local_hour = s.ts.with_timezone(&tz).hour() as usize;
        buckets[local_hour].push(implied);
    }

    let mut out: [Option<f64>; 24] = [None; 24];
    for (h, samples) in buckets.iter_mut().enumerate() {
        if samples.is_empty() {
            continue;
        }
        samples.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let n = samples.len();
        out[h] = Some(if n % 2 == 1 {
            samples[n / 2]
        } else {
            (samples[n / 2 - 1] + samples[n / 2]) / 2.0
        });
    }
    out
}

/// Circular rolling median over a 24-bin array. Window size 3 means each bin
/// gets the median of itself and its two neighbors (with wrap). None values
/// are skipped.
pub fn smooth_rolling_circular(raw: &[Option<f64>; 24], window: usize) -> [Option<f64>; 24] {
    let half = (window / 2) as isize;
    let n = 24isize;
    let mut out: [Option<f64>; 24] = [None; 24];
    for i in 0..24isize {
        let mut neighbors: Vec<f64> = Vec::new();
        for offset in -half..=half {
            let j = ((i + offset) % n + n) % n;
            if let Some(v) = raw[j as usize] {
                neighbors.push(v);
            }
        }
        if neighbors.is_empty() {
            continue;
        }
        neighbors.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let m = neighbors.len();
        out[i as usize] = Some(if m % 2 == 1 {
            neighbors[m / 2]
        } else {
            (neighbors[m / 2 - 1] + neighbors[m / 2]) / 2.0
        });
    }
    out
}

/// Linear-interpolate `None` bins across the array, with circular wrap.
/// If all bins are `None`, returns `[0.0; 24]`.
pub fn interpolate_empty_circular(smoothed: &[Option<f64>; 24]) -> [f64; 24] {
    let n: isize = 24;
    let any = smoothed.iter().any(|v| v.is_some());
    if !any {
        return [0.0; 24];
    }
    let mut out = [0.0f64; 24];
    for h in 0..24usize {
        if let Some(v) = smoothed[h] {
            out[h] = v;
            continue;
        }
        // Search backward for nearest non-None.
        let mut prev: Option<(usize, isize)> = None;
        for off in 1..=n {
            let j = ((h as isize - off) % n + n) % n;
            if smoothed[j as usize].is_some() {
                prev = Some((j as usize, off));
                break;
            }
        }
        // Search forward for nearest non-None.
        let mut next: Option<(usize, isize)> = None;
        for off in 1..=n {
            let j = ((h as isize + off) % n + n) % n;
            if smoothed[j as usize].is_some() {
                next = Some((j as usize, off));
                break;
            }
        }
        out[h] = match (prev, next) {
            (Some((pi, pd)), Some((ni, nd))) => {
                let pv = smoothed[pi].unwrap();
                let nv = smoothed[ni].unwrap();
                let total = (pd + nd) as f64;
                pv * (nd as f64 / total) + nv * (pd as f64 / total)
            }
            (Some((pi, _)), None) => smoothed[pi].unwrap(),
            (None, Some((ni, _))) => smoothed[ni].unwrap(),
            (None, None) => 0.0,
        };
    }
    out
}

/// Public entry point: per-hour median → 3-bin circular smoothing → interpolation.
/// Returns `[0.0; 24]` if no valid anchors exist.
pub fn hour_of_day_cap_series(
    log: &[CalibrationSample],
    turns: &[Turn],
    kind: WindowKind,
) -> [f64; 24] {
    let raw = per_hour_medians(log, turns, kind);
    let smoothed = smooth_rolling_circular(&raw, 3);
    interpolate_empty_circular(&smoothed)
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

    fn sample(ts: DateTime<Utc>, util_5h: f64) -> CalibrationSample {
        CalibrationSample {
            schema_version: 1,
            ts,
            five_hour_util: Some(util_5h),
            five_hour_resets_at: None,
            seven_day_util: Some(0.0),
            seven_day_resets_at: None,
            subscription_type: "pro".to_string(),
            rate_limit_tier: "default_claude_ai".to_string(),
        }
    }

    #[test]
    fn per_hour_medians_empty_log_returns_all_none() {
        let raw = per_hour_medians(&[], &[], WindowKind::FiveHour);
        assert_eq!(raw.len(), 24);
        assert!(raw.iter().all(|v| v.is_none()));
    }

    #[test]
    fn per_hour_medians_bins_by_local_hour() {
        // Anchor at 2026-05-24 14:00 UTC = 16:00 local CEST.
        // Burn = 100, util = 1.0 → implied cap = 100.
        let log = vec![sample(utc(2026, 5, 24, 14, 0), 1.0)];
        let turns = vec![turn(utc(2026, 5, 24, 13, 0), 100)];
        let raw = per_hour_medians(&log, &turns, WindowKind::FiveHour);
        // Bin 16 (local) should be Some(100.0).
        assert_eq!(raw[16], Some(100.0));
        // All other bins should be None.
        for (h, v) in raw.iter().enumerate() {
            if h != 16 {
                assert_eq!(*v, None);
            }
        }
    }

    #[test]
    fn smooth_rolling_circular_passes_through_dense_data() {
        let mut raw = [Some(100.0); 24];
        raw[12] = Some(200.0);
        // 3-bin median at hour 12: median(100, 200, 100) = 100.
        // At hour 11: median(100, 100, 200) = 100. Identical.
        let out = smooth_rolling_circular(&raw, 3);
        assert_eq!(out[12], Some(100.0));
        assert_eq!(out[11], Some(100.0));
    }

    #[test]
    fn smooth_rolling_circular_handles_nones_by_skipping() {
        let mut raw: [Option<f64>; 24] = [None; 24];
        raw[10] = Some(50.0);
        raw[11] = Some(100.0);
        raw[12] = Some(150.0);
        let out = smooth_rolling_circular(&raw, 3);
        // At 11: median(50, 100, 150) = 100.
        assert_eq!(out[11], Some(100.0));
        // At 12: median(100, 150) = 125.
        assert_eq!(out[12], Some(125.0));
        // At 13: only 150 contributes from neighbor at 12. median(150) = 150.
        assert_eq!(out[13], Some(150.0));
    }

    #[test]
    fn interpolate_empty_circular_all_none_returns_zeros() {
        let raw: [Option<f64>; 24] = [None; 24];
        let out = interpolate_empty_circular(&raw);
        assert_eq!(out, [0.0; 24]);
    }

    #[test]
    fn interpolate_empty_circular_fills_gaps_linearly() {
        let mut raw: [Option<f64>; 24] = [None; 24];
        raw[0] = Some(100.0);
        raw[6] = Some(700.0);
        let out = interpolate_empty_circular(&raw);
        // Between bin 0 (100) and bin 6 (700), bin 3 should be exactly halfway = 400.
        assert_eq!(out[0], 100.0);
        assert_eq!(out[6], 700.0);
        assert!((out[3] - 400.0).abs() < 0.001);
    }

    #[test]
    fn hour_of_day_cap_series_empty_returns_zeros() {
        let out = hour_of_day_cap_series(&[], &[], WindowKind::FiveHour);
        assert_eq!(out, [0.0; 24]);
    }
}

