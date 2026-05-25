//! Calibration-history math: implied-cap series + per-hour statistics for the
//! dashboard's Calibration tab. Pure functions; UI lives in
//! `dashboard/calibration_tab.rs`.

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
}
