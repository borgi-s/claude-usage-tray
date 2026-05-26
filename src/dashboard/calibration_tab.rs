//! Calibration tab: how the app's caps are derived from the calibration log.
//! Four plots — implied 5h/weekly cap over time (hour-banded scatter) and
//! hour-of-day cap bins (median line + IQR band + fitted curve) for 5h/weekly.
//! Always account-wide: this tab ignores the global filter bar.

use crate::calibration::history::{median, HourStat, ImpliedPoint};
use crate::shared::snapshot::AppSnapshot;
use egui::{Color32, RichText, Stroke, Ui};
use egui_plot::{HLine, Legend, Line, LineStyle, Plot, PlotPoints, PlotUi, Points, Polygon};
use std::sync::Arc;

/// Derived chart inputs, memoized by the dashboard. Vecs sit behind `Arc` so
/// the per-frame clone of the memo is cheap (matches the `AppSnapshot` pattern).
#[derive(Clone)]
pub struct CalibData {
    pub implied_5h: Arc<Vec<ImpliedPoint>>,
    pub implied_week: Arc<Vec<ImpliedPoint>>,
    pub stats_5h: [HourStat; 24],
    pub stats_week: [HourStat; 24],
}

// Hour-band scatter colors.
const C_NIGHT: Color32 = Color32::from_rgb(120, 110, 220); // 0–6  indigo
const C_MORNING: Color32 = Color32::from_rgb(60, 190, 180); // 6–12 teal
const C_AFTERNOON: Color32 = Color32::from_rgb(240, 180, 70); // 12–18 amber
const C_EVENING: Color32 = Color32::from_rgb(220, 90, 180); // 18–24 magenta

// Hour-of-day chart colors.
const C_MEDIAN: Color32 = Color32::from_rgb(79, 140, 255); // blue (matches chart_5h)
const C_FITTED: Color32 = Color32::from_rgb(255, 165, 79); // orange
const C_MEDIAN_HLINE: Color32 = Color32::from_rgb(120, 120, 120);
// Soft blue IQR fill at low opacity (premultiplied: rgb already * alpha/255).
const C_IQR: Color32 = Color32::from_rgba_premultiplied(20, 35, 64, 90);

const M: f64 = 1_000_000.0; // tokens → millions

const UNCALIBRATED: &str = "(uncalibrated — no ≥95% anchors observed yet)";

pub fn render(ui: &mut Ui, snap: &AppSnapshot, calib: &CalibData) {
    egui::ScrollArea::vertical().show(ui, |ui| {
        ui.add_space(8.0);
        ui.label(RichText::new("Implied 5h cap over time").strong());
        scatter_over_time(ui, "calib_implied_5h", &calib.implied_5h);
        ui.add_space(16.0);
        ui.separator();

        ui.add_space(8.0);
        ui.label(RichText::new("Implied weekly cap over time").strong());
        scatter_over_time(ui, "calib_implied_week", &calib.implied_week);
        ui.add_space(16.0);
        ui.separator();

        ui.add_space(8.0);
        ui.label(RichText::new("Hour-of-day cap — 5h").strong());
        hour_of_day(ui, "calib_hod_5h", &calib.stats_5h, &snap.hourly_5h);
        ui.add_space(16.0);
        ui.separator();

        ui.add_space(8.0);
        ui.label(RichText::new("Hour-of-day cap — weekly").strong());
        hour_of_day(ui, "calib_hod_week", &calib.stats_week, &snap.hourly_week);
        ui.add_space(8.0);
    });
}

/// Scatter of implied cap (M tokens) vs time, points split into 4 hour bands
/// with a legend, plus a dashed line at the median implied cap.
fn scatter_over_time(ui: &mut Ui, id: &str, points: &[ImpliedPoint]) {
    if points.is_empty() {
        ui.label(RichText::new(UNCALIBRATED).color(Color32::from_rgb(220, 200, 120)));
        return;
    }

    // (color, legend name, points) for each of the four bands.
    let mut bands: [(Color32, &str, Vec<[f64; 2]>); 4] = [
        (C_NIGHT, "night 0–6", Vec::new()),
        (C_MORNING, "morning 6–12", Vec::new()),
        (C_AFTERNOON, "afternoon 12–18", Vec::new()),
        (C_EVENING, "evening 18–24", Vec::new()),
    ];
    for p in points {
        let idx = match p.local_hour {
            0..=5 => 0,
            6..=11 => 1,
            12..=17 => 2,
            _ => 3,
        };
        bands[idx].2.push([p.ts.timestamp() as f64, p.cap / M]);
    }

    let mut caps_m: Vec<f64> = points.iter().map(|p| p.cap / M).collect();
    let median_m = median(&mut caps_m);

    Plot::new(id)
        .height(240.0)
        .show_x(true)
        .show_y(true)
        .y_axis_label("M tokens")
        .legend(Legend::default())
        .x_axis_formatter({
            let tz = crate::settings::CalParams::default().tz;
            move |mark: egui_plot::GridMark, _range: &std::ops::RangeInclusive<f64>| {
                crate::dashboard::axis::format_x_tick(mark.value, tz)
            }
        })
        .show(ui, |plot_ui| {
            for (color, name, pts) in &bands {
                if pts.is_empty() {
                    continue;
                }
                plot_ui.points(
                    Points::new(PlotPoints::from(pts.clone()))
                        .color(*color)
                        .radius(3.0)
                        .name(*name),
                );
            }
            if let Some(m) = median_m {
                plot_ui.hline(
                    HLine::new(m)
                        .color(C_MEDIAN_HLINE)
                        .style(LineStyle::dashed_dense())
                        .name("median cap"),
                );
            }
        });
}

/// Hour-of-day chart: IQR band (p25–p75) + median line with count-scaled
/// markers + the fitted (smoothed/interpolated) curve.
fn hour_of_day(ui: &mut Ui, id: &str, stats: &[HourStat; 24], fitted: &[f64; 24]) {
    if !stats.iter().any(|s| s.median.is_some()) {
        ui.label(RichText::new(UNCALIBRATED).color(Color32::from_rgb(220, 200, 120)));
        return;
    }

    Plot::new(id)
        .height(240.0)
        .show_x(true)
        .show_y(true)
        .x_axis_label("hour of day (local)")
        .y_axis_label("M tokens")
        .legend(Legend::default())
        .show(ui, |plot_ui| {
            // IQR band: one filled polygon per contiguous run of populated hours.
            let mut run: Vec<(f64, f64, f64)> = Vec::new(); // (hour, p25_M, p75_M)
            for (h, s) in stats.iter().enumerate() {
                match (s.p25, s.p75) {
                    (Some(lo), Some(hi)) => run.push((h as f64, lo / M, hi / M)),
                    _ => {
                        draw_iqr_run(plot_ui, &run);
                        run.clear();
                    }
                }
            }
            draw_iqr_run(plot_ui, &run);

            // Median line through populated hours.
            let med_line: Vec<[f64; 2]> = stats
                .iter()
                .enumerate()
                .filter_map(|(h, s)| s.median.map(|m| [h as f64, m / M]))
                .collect();
            if med_line.len() >= 2 {
                plot_ui.line(
                    Line::new(PlotPoints::from(med_line))
                        .color(C_MEDIAN)
                        .name("median"),
                );
            }

            // Count-scaled markers on the median.
            for (h, s) in stats.iter().enumerate() {
                if let Some(m) = s.median {
                    let radius = (2.0 + s.n as f64).min(12.0) as f32;
                    plot_ui.points(
                        Points::new(PlotPoints::from(vec![[h as f64, m / M]]))
                            .color(C_MEDIAN)
                            .radius(radius),
                    );
                }
            }

            // Fitted curve (dense, 24 hours) — only if it carries signal.
            if fitted.iter().any(|&v| v > 0.0) {
                let curve: Vec<[f64; 2]> = (0..24).map(|h| [h as f64, fitted[h] / M]).collect();
                plot_ui.line(
                    Line::new(PlotPoints::from(curve))
                        .color(C_FITTED)
                        .style(LineStyle::dotted_dense())
                        .name("fitted"),
                );
            }
        });
}

/// Draw one IQR polygon: p25 left→right, then p75 right→left, closing the band.
fn draw_iqr_run(plot_ui: &mut PlotUi, run: &[(f64, f64, f64)]) {
    if run.len() < 2 {
        return;
    }
    let mut poly: Vec<[f64; 2]> = Vec::with_capacity(run.len() * 2);
    for &(h, lo, _hi) in run.iter() {
        poly.push([h, lo]);
    }
    for &(h, _lo, hi) in run.iter().rev() {
        poly.push([h, hi]);
    }
    plot_ui.polygon(
        Polygon::new(PlotPoints::from(poly))
            .fill_color(C_IQR)
            .stroke(Stroke::NONE),
    );
}
