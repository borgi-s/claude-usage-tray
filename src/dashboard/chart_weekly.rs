//! Weekly cumulative-share chart: per-week (Sun 07:00 local reset) stepped line.

use crate::dashboard::bands::calendar_bands;
use crate::dashboard::range::{clamp_x_range, Range};
use crate::dashboard::series::cumulative_share_series_weekly;
use crate::shared::snapshot::AppSnapshot;
use chrono::Utc;
use egui::{Color32, Stroke, Ui};
use egui_plot::{HLine, Line, LineStyle, Plot, PlotPoints, Polygon};

const COLOR_LINE: Color32 = Color32::from_rgb(79, 140, 255);
// Soft blue-grey band at ~16% opacity.
// from_rgba_premultiplied requires RGB already multiplied by alpha/255:
//   120 * 40/255 ≈ 19, 120 * 40/255 ≈ 19, 140 * 40/255 ≈ 22
const COLOR_BAND: Color32 = Color32::from_rgba_premultiplied(19, 19, 22, 40);
const COLOR_CAP: Color32 = Color32::from_rgb(120, 120, 120);

pub fn render(ui: &mut Ui, snap: &AppSnapshot, range: &mut Range) {
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("Weekly cumulative share").strong());
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

    let cap_week = snap.caps.cap_week;
    let series = cumulative_share_series_weekly(&snap.turns, cap_week);

    let x = |t: chrono::DateTime<Utc>| t.timestamp() as f64;

    let visible: Vec<_> = series
        .iter()
        .filter(|w| w.ts >= x_start && w.ts <= x_end)
        .collect();

    let mut segments: Vec<Vec<[f64; 2]>> = Vec::new();
    let mut current_win: isize = -1;
    for w in &visible {
        let pct = w.cumulative_share * 100.0;
        if w.window_idx as isize != current_win {
            segments.push(Vec::new());
            current_win = w.window_idx as isize;
        }
        segments.last_mut().unwrap().push([x(w.ts), pct]);
    }

    let y_label = if cap_week.is_some() {
        "% of cap"
    } else {
        "output tokens"
    };

    Plot::new("chart_weekly")
        .height(280.0)
        .show_x(true)
        .show_y(true)
        .y_axis_label(y_label)
        .x_axis_formatter(
            |mark: egui_plot::GridMark, _range: &std::ops::RangeInclusive<f64>| {
                crate::dashboard::axis::format_x_tick(mark.value)
            },
        )
        .show(ui, |plot_ui| {
            for (s, e, _) in calendar_bands(x_start, x_end) {
                plot_ui.polygon(
                    Polygon::new(PlotPoints::from(vec![
                        [x(s), 0.0],
                        [x(e), 0.0],
                        [x(e), 200.0],
                        [x(s), 200.0],
                    ]))
                    .fill_color(COLOR_BAND)
                    .stroke(Stroke::NONE),
                );
            }
            if cap_week.is_some() {
                plot_ui.hline(
                    HLine::new(100.0)
                        .color(COLOR_CAP)
                        .style(LineStyle::dashed_dense()),
                );
            }
            for (i, seg) in segments.iter().enumerate() {
                if seg.is_empty() {
                    continue;
                }
                plot_ui.line(
                    Line::new(PlotPoints::from(seg.clone()))
                        .color(COLOR_LINE)
                        .name(if i == 0 { "Weekly share" } else { "" }),
                );
            }
        });
}
