//! KPI strip layout. Four equal-width columns above the charts.

use crate::shared::snapshot::DashboardKpis;
use egui::{Color32, Ui};

pub fn render(ui: &mut Ui, kpis: &DashboardKpis, caps_available: bool) {
    ui.columns(4, |cols| {
        kpi_share(&mut cols[0], "Peak 5h share", kpis.peak_5h_share, caps_available);
        kpi_share(&mut cols[1], "Peak weekly share", kpis.peak_week_share, caps_available);
        kpi_total(&mut cols[2], "Total burn", kpis.total_cost_weighted, "cost-weighted");
        kpi_rate(&mut cols[3], "Daily avg", kpis.daily_avg_cost_weighted, "/ day");
    });
}

fn kpi_share(ui: &mut Ui, label: &str, share: f64, caps_available: bool) {
    ui.label(egui::RichText::new(label).size(11.0).color(Color32::GRAY));
    if caps_available {
        ui.label(egui::RichText::new(format!("{}%", (share * 100.0).round() as i64)).size(22.0));
        let pct = share.clamp(0.0, 1.0);
        let (rect, _) = ui.allocate_exact_size(
            egui::vec2(ui.available_width(), 4.0),
            egui::Sense::hover(),
        );
        let painter = ui.painter();
        painter.rect_filled(rect, 1.0, Color32::from_gray(60));
        let mut fill_rect = rect;
        fill_rect.max.x = rect.min.x + rect.width() * pct as f32;
        painter.rect_filled(fill_rect, 1.0, Color32::from_rgb(79, 140, 255));
    } else {
        ui.label(egui::RichText::new("—").size(22.0).color(Color32::GRAY));
        let (rect, _) = ui.allocate_exact_size(
            egui::vec2(ui.available_width(), 4.0),
            egui::Sense::hover(),
        );
        ui.painter().rect_filled(rect, 1.0, Color32::from_gray(40));
    }
}

fn kpi_total(ui: &mut Ui, label: &str, value: f64, suffix: &str) {
    ui.label(egui::RichText::new(label).size(11.0).color(Color32::GRAY));
    ui.label(egui::RichText::new(format_si(value)).size(22.0));
    ui.label(egui::RichText::new(suffix).size(11.0).color(Color32::GRAY));
}

fn kpi_rate(ui: &mut Ui, label: &str, value: f64, suffix: &str) {
    ui.label(egui::RichText::new(label).size(11.0).color(Color32::GRAY));
    ui.label(egui::RichText::new(format_si(value)).size(22.0));
    ui.label(egui::RichText::new(suffix).size(11.0).color(Color32::GRAY));
}

/// Format a number with SI suffixes (e.g., 42_500_000 → "42.5M").
pub fn format_si(v: f64) -> String {
    let abs = v.abs();
    if abs >= 1e9 {
        format!("{:.1}B", v / 1e9)
    } else if abs >= 1e6 {
        format!("{:.1}M", v / 1e6)
    } else if abs >= 1e3 {
        format!("{:.1}K", v / 1e3)
    } else {
        format!("{:.0}", v)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn format_si_picks_suffix() {
        assert_eq!(format_si(42_500_000.0), "42.5M");
        assert_eq!(format_si(8_100.0), "8.1K");
        assert_eq!(format_si(950.0), "950");
        assert_eq!(format_si(0.0), "0");
        assert_eq!(format_si(1_200_000_000.0), "1.2B");
    }
}
