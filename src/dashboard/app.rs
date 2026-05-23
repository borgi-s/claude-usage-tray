//! DashboardApp — the eframe::App. A single instance lives for the process.
//! Close requests hide the window instead of destroying it; the tray re-shows
//! it via the shared signals.

use crate::dashboard::range::Range;
use crate::dashboard::DashboardSignals;
use crate::shared::SharedSnapshot;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

/// Off-screen parking spot. The window is moved here instead of hidden, because
/// hiding the root viewport (`Visible(false)`) parks eframe's event loop — it
/// then ignores repaint timers and cross-thread wakes, so we could never bring
/// it back. An off-screen-but-visible window keeps the loop ticking.
const OFFSCREEN: egui::Pos2 = egui::pos2(-32000.0, -32000.0);

pub struct DashboardApp {
    shared: SharedSnapshot,
    signals: Arc<DashboardSignals>,
    visible: bool,
    ctx_published: bool,
    saved_pos: Option<egui::Pos2>,
    range_5h: Range,
    range_week: Range,
    range_daily: Range,
}

impl DashboardApp {
    pub fn new(shared: SharedSnapshot, signals: Arc<DashboardSignals>) -> Self {
        Self {
            shared,
            signals,
            visible: true,
            ctx_published: false,
            saved_pos: None,
            range_5h: Range::D5,
            range_week: Range::D14,
            range_daily: Range::D14,
        }
    }
}

impl eframe::App for DashboardApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // 0. Publish our Context once so the tray thread can request_repaint()
        //    to wake us even while hidden.
        if !self.ctx_published {
            *self.signals.ctx.lock().unwrap() = Some(ctx.clone());
            self.ctx_published = true;
            tracing::debug!("dashboard: published egui context");
        }

        // 1. App-quit: close the viewport so run_native returns + thread exits.
        if self.signals.quit_requested.load(Ordering::Relaxed) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }

        // 2. Tray asked us to show: move back on-screen + focus.
        if self.signals.show_requested.swap(false, Ordering::Relaxed) {
            tracing::debug!("dashboard: show_requested seen, restoring");
            self.visible = true;
            let pos = self.saved_pos.unwrap_or(egui::pos2(200.0, 100.0));
            ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(pos));
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        }

        // 3. User clicked the X: cancel the real close, park the window
        //    off-screen instead. Only act when currently visible — a stale
        //    close_requested on a freshly-restored window must not re-park it.
        if self.visible && ctx.input(|i| i.viewport().close_requested()) {
            tracing::debug!("dashboard: close intercepted, parking off-screen");
            self.saved_pos = ctx.input(|i| i.viewport().outer_rect.map(|r| r.min));
            self.visible = false;
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(OFFSCREEN));
        }

        // 4. When parked off-screen, skip the heavy chart render; just keep the
        //    loop ticking so we notice show/quit. The window is still "visible"
        //    to winit, so the timer keeps firing (unlike Visible(false), which
        //    parks the loop).
        if !self.visible {
            ctx.request_repaint_after(Duration::from_millis(150));
            return;
        }

        // 5. Visible: render the dashboard.
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
                                .color(egui::Color32::from_rgb(220, 200, 120)),
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

        ctx.request_repaint_after(Duration::from_millis(33));
    }
}
