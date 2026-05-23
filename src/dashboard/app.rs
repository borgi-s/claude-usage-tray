//! DashboardApp is the eframe::App implementation. The first frame discovers
//! its own HWND via find_hwnd_by_title and writes it into the shared slot
//! so the tray UI thread can raise the window to front on subsequent clicks.

use crate::dashboard::range::Range;
use crate::dashboard::{find_hwnd_by_title, SendHwnd, DASHBOARD_WINDOW_TITLE};
use crate::shared::SharedSnapshot;
use std::sync::{Arc, Mutex};

pub struct DashboardApp {
    shared: SharedSnapshot,
    hwnd_slot: Arc<Mutex<Option<SendHwnd>>>,
    hwnd_found: bool,
    range_5h: Range,
    range_week: Range,
    range_daily: Range,
}

impl DashboardApp {
    pub fn new(shared: SharedSnapshot, hwnd_slot: Arc<Mutex<Option<SendHwnd>>>) -> Self {
        Self {
            shared,
            hwnd_slot,
            hwnd_found: false,
            range_5h: Range::D5,
            range_week: Range::D14,
            range_daily: Range::D14,
        }
    }

    /// Try to find our own HWND. Called every frame until found.
    fn discover_hwnd_if_needed(&mut self) {
        if self.hwnd_found {
            return;
        }
        if let Some(hwnd) = find_hwnd_by_title(DASHBOARD_WINDOW_TITLE) {
            *self.hwnd_slot.lock().unwrap() = Some(SendHwnd(hwnd));
            self.hwnd_found = true;
            tracing::debug!("dashboard HWND discovered");
        }
    }
}

impl eframe::App for DashboardApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.discover_hwnd_if_needed();

        egui::CentralPanel::default().show(ctx, |ui| {
            let snap = self.shared.read().unwrap().clone();
            let caps_available = snap.caps.cap_5h.is_some() || snap.caps.cap_week.is_some();
            egui::ScrollArea::vertical().show(ui, |ui| {
                ui.add_space(8.0);
                crate::dashboard::kpi::render(ui, &snap.kpis, caps_available);
                ui.add_space(16.0);
                if snap.caps.cap_5h.is_none() && snap.caps.cap_week.is_none() {
                    egui::Frame::default()
                        .fill(egui::Color32::from_rgb(60, 50, 30))
                        .inner_margin(egui::Margin::same(8.0))
                        .show(ui, |ui| {
                            ui.label(
                                egui::RichText::new(
                                    "Uncalibrated — charts show raw output tokens until first ≥95% anchor is observed in the calibration log.",
                                )
                                .color(egui::Color32::from_rgb(220, 200, 120))
                            );
                        });
                    ui.add_space(8.0);
                }
                ui.separator();
                ui.add_space(8.0);
                crate::dashboard::chart_5h::render(ui, &snap, &mut self.range_5h);
                ui.add_space(16.0);
                ui.separator();
                ui.add_space(8.0);
                crate::dashboard::chart_weekly::render(ui, &snap, &mut self.range_week);
                ui.add_space(16.0);
                ui.separator();
                ui.add_space(8.0);
                crate::dashboard::chart_daily::render(ui, &snap, &mut self.range_daily);
                ui.add_space(8.0);
            });
        });

        // Request a repaint at ~30fps so the snapshot view stays fresh.
        ctx.request_repaint_after(std::time::Duration::from_millis(33));
    }
}
