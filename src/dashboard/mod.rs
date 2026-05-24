//! Native egui dashboard window. A single persistent thread owns the one
//! EventLoop the process is allowed (winit forbids recreating it). Closing the
//! window hides it; the tray re-shows it via the `show_requested` flag; app
//! quit sets `quit_requested` to let the EventLoop exit.

pub mod app;
pub mod axis;
pub mod bands;
pub mod chart_5h;
pub mod chart_daily;
pub mod chart_weekly;
pub mod filters;
pub mod kpi;
pub mod range;
pub mod series;
pub mod sessions_table;

use crate::dashboard::app::DashboardApp;
use crate::shared::SharedSnapshot;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

pub const DASHBOARD_WINDOW_TITLE: &str = "Claude usage tracker";

/// Tray → dashboard signals. The dashboard thread polls these every frame.
#[derive(Default)]
pub struct DashboardSignals {
    /// Tray sets true on left-click to un-hide + focus the window.
    pub show_requested: AtomicBool,
    /// Tray sets true on Quit so the dashboard closes its viewport and the
    /// thread exits, allowing a clean join.
    pub quit_requested: AtomicBool,
    /// The dashboard publishes a clone of its egui Context here on its first
    /// frame. The tray uses it to `request_repaint()` cross-thread, which wakes
    /// the event loop even when the window is hidden/occluded (and would
    /// otherwise not be ticking to notice `show_requested`).
    pub ctx: Mutex<Option<egui::Context>>,
}

impl DashboardSignals {
    /// Tray-side helper: request show + wake the (possibly idle) event loop.
    pub fn request_show(&self) {
        self.show_requested.store(true, Ordering::Relaxed);
        if let Some(ctx) = self.ctx.lock().unwrap().as_ref() {
            ctx.request_repaint();
        }
    }

    /// Tray-side helper: request quit + wake the loop so it sees the flag.
    pub fn request_quit(&self) {
        self.quit_requested.store(true, Ordering::Relaxed);
        if let Some(ctx) = self.ctx.lock().unwrap().as_ref() {
            ctx.request_repaint();
        }
    }
}

pub struct DashboardHandle {
    pub signals: Arc<DashboardSignals>,
    pub join: JoinHandle<()>,
}

/// Spawn the single dashboard thread. Called at most once per process run
/// (the tray only spawns when no live handle exists). Returns immediately.
pub fn launch(shared: SharedSnapshot) -> DashboardHandle {
    let signals = Arc::new(DashboardSignals::default());
    let signals_for_thread = signals.clone();

    let join = std::thread::spawn(move || {
        let app = DashboardApp::new(shared, signals_for_thread);
        let native_options = eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default()
                .with_inner_size([1100.0, 720.0])
                .with_min_inner_size([700.0, 480.0])
                .with_title(DASHBOARD_WINDOW_TITLE),
            // winit on Windows requires opting in to non-main-thread EventLoop.
            event_loop_builder: Some(Box::new(|builder| {
                use winit::platform::windows::EventLoopBuilderExtWindows;
                builder.with_any_thread(true);
            })),
            ..Default::default()
        };

        if let Err(e) = eframe::run_native(
            "claude-usage-tray-dashboard",
            native_options,
            Box::new(|_cc| Ok(Box::new(app))),
        ) {
            tracing::warn!(error = ?e, "eframe::run_native failed");
        }
    });

    DashboardHandle { signals, join }
}
