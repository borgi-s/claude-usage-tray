//! DashboardApp — the eframe::App. A single instance lives for the process.
//! Close requests hide the window instead of destroying it; the tray re-shows
//! it via the shared signals.

use crate::calibration::history;
use crate::calibration::WindowKind;
use crate::dashboard::calibration_tab::CalibData;
use crate::dashboard::filters::FilterState;
use crate::dashboard::range::Range;
use crate::dashboard::sessions_table::TableControls;
use crate::dashboard::DashboardSignals;
use crate::shared::snapshot::{compute_kpis, AppSnapshot};
use crate::shared::SharedSnapshot;
use chrono::{DateTime, Utc};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

/// Off-screen parking spot. The window is moved here instead of hidden, because
/// hiding the root viewport (`Visible(false)`) parks eframe's event loop — it
/// then ignores repaint timers and cross-thread wakes, so we could never bring
/// it back. An off-screen-but-visible window keeps the loop ticking.
const OFFSCREEN: egui::Pos2 = egui::pos2(-32000.0, -32000.0);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tab {
    Charts,
    Sessions,
    Calibration,
}

/// Cheap signature for the filtered-view memo. Equal signature ⇒ reuse cache.
#[derive(Debug, Clone, PartialEq)]
struct ViewSig {
    filter: FilterState,
    n_turns: usize,
    last_ts: Option<DateTime<Utc>>,
}

/// Cheap signature for the calib-data memo. Equal signature ⇒ reuse cache.
#[derive(Debug, Clone, PartialEq)]
struct CalibSig {
    n_log: usize,
    n_turns: usize,
}

pub struct DashboardApp {
    shared: SharedSnapshot,
    signals: Arc<DashboardSignals>,
    visible: bool,
    ctx_published: bool,
    saved_pos: Option<egui::Pos2>,
    range_5h: Range,
    range_week: Range,
    range_daily: Range,
    tab: Tab,
    filters: FilterState,
    table_controls: TableControls,
    cached_view: Option<(ViewSig, AppSnapshot)>,
    cached_calib: Option<(CalibSig, CalibData)>,
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
            tab: Tab::Charts,
            filters: FilterState::default(),
            table_controls: TableControls::default(),
            cached_view: None,
            cached_calib: None,
        }
    }

    /// Build (or reuse) the filtered AppSnapshot: turns filtered + KPIs
    /// recomputed; caps/hourly/live copied through unchanged. Memoized on the
    /// filter state and the turn vector's length+last-timestamp.
    fn filtered_view(&mut self, snap: &AppSnapshot) -> AppSnapshot {
        let sig = ViewSig {
            filter: self.filters.clone(),
            n_turns: snap.turns.len(),
            last_ts: snap.turns.last().map(|t| t.ts),
        };
        if let Some((cached_sig, view)) = &self.cached_view {
            if *cached_sig == sig {
                return view.clone();
            }
        }
        let filtered = self.filters.apply(&snap.turns, crate::settings::CalParams::default().tz);
        let kpis = compute_kpis(&filtered, &snap.caps, &crate::settings::CostWeights::default(), crate::settings::CalParams::default());
        let mut view = snap.clone();
        view.turns = Arc::new(filtered);
        view.kpis = kpis;
        self.cached_view = Some((sig, view.clone()));
        view
    }

    /// Build (or reuse) the Calibration tab's derived series. Always uses the
    /// UNFILTERED snapshot — calibration is account-wide. Memoized on the log +
    /// turn lengths (both append-only, so a length change ⇒ new data).
    fn calib_data(&mut self, snap: &AppSnapshot) -> CalibData {
        let sig = CalibSig {
            n_log: snap.log.len(),
            n_turns: snap.turns.len(),
        };
        if let Some((cached_sig, data)) = &self.cached_calib {
            if *cached_sig == sig {
                return data.clone();
            }
        }
        let data = CalibData {
            implied_5h: Arc::new(history::implied_cap_series(
                &snap.log,
                &snap.turns,
                WindowKind::FiveHour,
                crate::settings::CalParams::default(),
            )),
            implied_week: Arc::new(history::implied_cap_series(
                &snap.log,
                &snap.turns,
                WindowKind::Weekly,
                crate::settings::CalParams::default(),
            )),
            stats_5h: history::per_hour_stats(&snap.log, &snap.turns, WindowKind::FiveHour, crate::settings::CalParams::default()),
            stats_week: history::per_hour_stats(&snap.log, &snap.turns, WindowKind::Weekly, crate::settings::CalParams::default()),
        };
        self.cached_calib = Some((sig, data.clone()));
        data
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

        // 5. Visible: filter bar + tab strip + tab content.
        let snap = self.shared.read().unwrap().clone();
        let all_turns = snap.turns.clone();
        let view = self.filtered_view(&snap);

        egui::TopBottomPanel::top("status_banner_panel").show(ctx, |ui| {
            crate::dashboard::status_banner::render(
                ui,
                snap.last_sample.as_ref(),
                &snap.last_status,
                snap.interval_secs,
                Utc::now(),
            );
        });

        egui::TopBottomPanel::top("filter_bar_panel").show(ctx, |ui| {
            ui.add_space(4.0);
            crate::dashboard::filter_bar::render(
                ui,
                &all_turns,
                &mut self.filters,
                view.turns.len(),
                all_turns.len(),
            );
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.selectable_value(&mut self.tab, Tab::Charts, "Charts");
                ui.selectable_value(&mut self.tab, Tab::Sessions, "Sessions");
                ui.selectable_value(&mut self.tab, Tab::Calibration, "Calibration");
            });
        });

        egui::CentralPanel::default().show(ctx, |ui| match self.tab {
            Tab::Charts => {
                let caps_available = view.caps.cap_5h.is_some() || view.caps.cap_week.is_some();
                egui::ScrollArea::vertical().show(ui, |ui| {
                    ui.add_space(8.0);
                    crate::dashboard::kpi::render(ui, &view.kpis, caps_available);
                    ui.add_space(16.0);
                    if view.caps.cap_5h.is_none() && view.caps.cap_week.is_none() {
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
                    crate::dashboard::chart_5h::render(ui, &view, &mut self.range_5h, crate::settings::CalParams::default().tz);
                    ui.add_space(16.0);
                    ui.separator();
                    ui.add_space(8.0);
                    crate::dashboard::chart_weekly::render(ui, &view, &mut self.range_week, crate::settings::CalParams::default());
                    ui.add_space(16.0);
                    ui.separator();
                    ui.add_space(8.0);
                    crate::dashboard::chart_daily::render(ui, &view, &mut self.range_daily, &crate::settings::CostWeights::default(), crate::settings::CalParams::default().tz);
                    ui.add_space(8.0);
                });
            }
            Tab::Sessions => {
                crate::dashboard::sessions_table::render(ui, &view.turns, &mut self.table_controls, crate::settings::CalParams::default().tz, &crate::settings::CostWeights::default());
            }
            Tab::Calibration => {
                let calib = self.calib_data(&snap);
                crate::dashboard::calibration_tab::render(ui, &snap, &calib);
            }
        });

        ctx.request_repaint_after(Duration::from_millis(33));
    }
}
