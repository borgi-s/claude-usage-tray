//! Daily cost-weighted bar chart.

use crate::dashboard::range::{clamp_end_to_data, clamp_x_range, Range};
use crate::dashboard::series::daily_aggregates;
use crate::shared::snapshot::AppSnapshot;
use chrono::{NaiveTime, Utc};
use egui::{Color32, Ui};
use egui_plot::{Bar, BarChart, Plot};

const COLOR_BAR: Color32 = Color32::from_rgb(79, 140, 255);

pub fn render(
    ui: &mut Ui,
    snap: &AppSnapshot,
    range: &mut Range,
    w: &crate::settings::CostWeights,
    tz: chrono_tz::Tz,
) {
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("Daily burn").strong());
        ui.separator();
        for &r in Range::VARIANTS {
            if ui.selectable_label(*range == r, r.label()).clicked() {
                *range = r;
            }
        }
    });

    let now = Utc::now();
    let (mut x_start, x_end) = clamp_x_range(now, *range);
    let x_end = clamp_end_to_data(x_end, snap.turns.last().map(|t| t.ts));
    if *range == Range::All {
        if let Some(first) = snap.turns.first() {
            x_start = first.ts;
        }
    }

    let aggregates = daily_aggregates(&snap.turns, w, tz);

    let bars: Vec<Bar> = aggregates
        .iter()
        .filter_map(|(date, val)| {
            let date_naive = date.and_time(NaiveTime::from_hms_opt(12, 0, 0).unwrap());
            let utc_dt = crate::dashboard::axis::local_to_utc(tz, date_naive);
            if utc_dt < x_start || utc_dt > x_end {
                return None;
            }
            // Bar width: 60_000 seconds ≈ ~17h, so adjacent days don't overlap.
            Some(Bar::new(utc_dt.timestamp() as f64, *val).width(60_000.0))
        })
        .collect();

    Plot::new("chart_daily")
        .height(220.0)
        .show_x(true)
        .show_y(true)
        .y_axis_label("cost-weighted")
        .x_axis_formatter(
            move |mark: egui_plot::GridMark, _range: &std::ops::RangeInclusive<f64>| {
                crate::dashboard::axis::format_x_tick(mark.value, tz)
            },
        )
        .show(ui, |plot_ui| {
            plot_ui.bar_chart(BarChart::new(bars).color(COLOR_BAR));
        });
}
