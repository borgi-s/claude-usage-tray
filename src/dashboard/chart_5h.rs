//! 5h cumulative-share chart: stepped line + cap line + hour-of-day overlay +
//! calendar bands + range selector.

use crate::dashboard::bands::calendar_bands;
use crate::dashboard::range::{clamp_x_range, Range};
use crate::dashboard::series::cumulative_share_series_5h;
use crate::shared::snapshot::AppSnapshot;
use chrono::{DateTime, Utc};
use egui::{Color32, Stroke, Ui};
use egui_plot::{HLine, Line, LineStyle, Plot, PlotPoints, Polygon};

const COLOR_LINE: Color32 = Color32::from_rgb(79, 140, 255);
// Soft blue-grey band at ~16% opacity.
// from_rgba_premultiplied requires RGB already multiplied by alpha/255:
//   120 * 40/255 ≈ 19, 120 * 40/255 ≈ 19, 140 * 40/255 ≈ 22
const COLOR_BAND: Color32 = Color32::from_rgba_premultiplied(19, 19, 22, 40);
const COLOR_CAP: Color32 = Color32::from_rgb(120, 120, 120);
const COLOR_HOURLY: Color32 = Color32::from_rgba_premultiplied(180, 180, 180, 80);

pub fn render(ui: &mut Ui, snap: &AppSnapshot, range: &mut Range) {
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("5h cumulative share").strong());
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

    let cap_5h = snap.caps.cap_5h;
    let series = cumulative_share_series_5h(&snap.turns, cap_5h);

    let x = |t: DateTime<Utc>| t.timestamp() as f64;

    let visible: Vec<_> = series
        .iter()
        .filter(|w| w.ts >= x_start && w.ts <= x_end)
        .collect();

    // Group by window_idx to draw separate stepped segments.
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

    let y_label = if cap_5h.is_some() { "% of cap" } else { "output tokens" };

    Plot::new("chart_5h")
        .height(280.0)
        .show_x(true)
        .show_y(true)
        .y_axis_label(y_label)
        .x_axis_formatter(|mark: egui_plot::GridMark, _range: &std::ops::RangeInclusive<f64>| {
            crate::dashboard::axis::format_x_tick(mark.value)
        })
        .show(ui, |plot_ui| {
            // Calendar bands.
            for (s, e, _kind) in calendar_bands(x_start, x_end) {
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

            // Hour-of-day overlay (if cap_5h available).
            if let Some(cap) = cap_5h {
                let overlay = hourly_overlay_points(x_start, x_end, snap.hourly_5h, cap);
                plot_ui.line(
                    Line::new(PlotPoints::from(overlay))
                        .color(COLOR_HOURLY)
                        .style(LineStyle::dashed_loose())
                        .name("hourly cap"),
                );
            }

            // Cap line at 100% (if cap exists).
            if cap_5h.is_some() {
                plot_ui.hline(
                    HLine::new(100.0)
                        .color(COLOR_CAP)
                        .style(LineStyle::dashed_dense()),
                );
            }

            // Cumulative share segments.
            for (i, seg) in segments.iter().enumerate() {
                if seg.is_empty() {
                    continue;
                }
                plot_ui.line(
                    Line::new(PlotPoints::from(seg.clone()))
                        .color(COLOR_LINE)
                        .name(if i == 0 { "5h share" } else { "" }),
                );
            }
        });
}

/// Sample the hour-of-day cap curve at each hour boundary in [x_start, x_end],
/// converting to (timestamp_seconds, percent-of-cap).
fn hourly_overlay_points(
    x_start: DateTime<Utc>,
    x_end: DateTime<Utc>,
    hourly: [f64; 24],
    cap: f64,
) -> Vec<[f64; 2]> {
    use chrono::Timelike;
    use chrono_tz::Tz;
    let tz: Tz = crate::config::LOCAL_TZ.parse().expect("LOCAL_TZ");
    let mut out = Vec::new();
    let mut cur = x_start;
    while cur < x_end {
        let local = cur.with_timezone(&tz);
        let h = local.hour() as usize;
        let pct = (hourly[h] / cap) * 100.0;
        out.push([cur.timestamp() as f64, pct]);
        cur += chrono::Duration::hours(1);
    }
    out
}
