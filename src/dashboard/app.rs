//! DashboardApp is the eframe::App implementation. The first frame discovers
//! its own HWND via find_hwnd_by_title and writes it into the shared slot
//! so the tray UI thread can raise the window to front on subsequent clicks.

use crate::dashboard::{find_hwnd_by_title, SendHwnd, DASHBOARD_WINDOW_TITLE};
use crate::shared::SharedSnapshot;
use std::sync::{Arc, Mutex};

pub struct DashboardApp {
    shared: SharedSnapshot,
    hwnd_slot: Arc<Mutex<Option<SendHwnd>>>,
    hwnd_found: bool,
}

impl DashboardApp {
    pub fn new(shared: SharedSnapshot, hwnd_slot: Arc<Mutex<Option<SendHwnd>>>) -> Self {
        Self { shared, hwnd_slot, hwnd_found: false }
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
            ui.heading("Claude usage tracker");
            ui.label(format!("Snapshot turns: {}", self.shared.read().unwrap().turns.len()));
            ui.label("(dashboard content coming in later tasks)");
        });

        // Request a repaint at ~30fps so the snapshot view stays fresh.
        ctx.request_repaint_after(std::time::Duration::from_millis(33));
    }
}
