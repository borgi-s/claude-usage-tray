//! Daily cost-weighted bar chart.

use crate::dashboard::range::{clamp_x_range, Range};
use crate::dashboard::series::daily_aggregates;
use crate::shared::snapshot::AppSnapshot;
use chrono::{NaiveTime, TimeZone, Utc};
use chrono_tz::Tz;
use egui::{Color32, Ui};
use egui_plot::{Bar, BarChart, Plot};

const COLOR_BAR: Color32 = Color32::from_rgb(79, 140, 255);

pub fn render(ui: &mut Ui, snap: &AppSnapshot, range: &mut Range) {
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
    if *range == Range::All {
        if let Some(first) = snap.turns.first() {
            x_start = first.ts;
        }
    }

    let tz: Tz = crate::config::LOCAL_TZ.parse().expect("LOCAL_TZ");
    let aggregates = daily_aggregates(&snap.turns);

    let bars: Vec<Bar> = aggregates
        .iter()
        .filter_map(|(date, val)| {
            let date_naive = date.and_time(NaiveTime::from_hms_opt(12, 0, 0).unwrap());
            let local_dt = tz.from_local_datetime(&date_naive).single()?;
            let utc_dt = local_dt.with_timezone(&Utc);
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
            |mark: egui_plot::GridMark, _range: &std::ops::RangeInclusive<f64>| {
                crate::dashboard::axis::format_x_tick(mark.value)
            },
        )
        .show(ui, |plot_ui| {
            plot_ui.bar_chart(BarChart::new(bars).color(COLOR_BAR));
        });
}
