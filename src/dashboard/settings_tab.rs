//! The Settings tab: edits a working-copy `Settings` (`draft`) and, on Save,
//! writes the shared store + persists to disk. Account-wide; ignores the global
//! filter bar (like the Calibration tab).

use crate::settings::{self, Settings, POLL_INTERVAL_CHOICES};
use crate::shared::SharedSettings;
use chrono::Weekday;
use egui::{ComboBox, DragValue, RichText, Ui};

const WEEKDAYS: [Weekday; 7] = [
    Weekday::Mon,
    Weekday::Tue,
    Weekday::Wed,
    Weekday::Thu,
    Weekday::Fri,
    Weekday::Sat,
    Weekday::Sun,
];

fn weekday_label(w: Weekday) -> &'static str {
    match w {
        Weekday::Mon => "Monday",
        Weekday::Tue => "Tuesday",
        Weekday::Wed => "Wednesday",
        Weekday::Thu => "Thursday",
        Weekday::Fri => "Friday",
        Weekday::Sat => "Saturday",
        Weekday::Sun => "Sunday",
    }
}

/// Render the Settings tab. `draft` is the editable working copy; `shared` is
/// the live store written on Save; `save_msg` shows the last save result.
pub fn render(
    ui: &mut Ui,
    draft: &mut Settings,
    shared: &SharedSettings,
    save_msg: &mut Option<Result<(), String>>,
    autostart_msg: &mut Option<Result<(), String>>,
) {
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        ui.heading("Settings");
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.button("Reset to defaults").clicked() {
                *draft = Settings::default();
            }
        });
    });
    ui.separator();
    ui.add_space(8.0);

    egui::Grid::new("settings_grid")
        .num_columns(2)
        .spacing([24.0, 12.0])
        .show(ui, |ui| {
            // Timezone
            ui.label("Timezone");
            ComboBox::from_id_salt("tz_combo")
                .selected_text(draft.local_tz.clone())
                .show_ui(ui, |ui| {
                    for tz in chrono_tz::TZ_VARIANTS {
                        let name = tz.name();
                        ui.selectable_value(&mut draft.local_tz, name.to_string(), name);
                    }
                });
            ui.end_row();

            // Weekly reset
            ui.label("Weekly reset");
            ui.horizontal(|ui| {
                ComboBox::from_id_salt("weekday_combo")
                    .selected_text(weekday_label(draft.weekly_reset_weekday))
                    .show_ui(ui, |ui| {
                        for w in WEEKDAYS {
                            ui.selectable_value(
                                &mut draft.weekly_reset_weekday,
                                w,
                                weekday_label(w),
                            );
                        }
                    });
                ui.label("at");
                ui.add(DragValue::new(&mut draft.weekly_reset_hour).range(0..=23));
                ui.label(":00 local");
            });
            ui.end_row();

            // Poll interval
            ui.label("Poll interval");
            ui.horizontal(|ui| {
                for secs in POLL_INTERVAL_CHOICES {
                    ui.selectable_value(&mut draft.poll_interval_secs, secs, format!("{secs}s"));
                }
            });
            ui.end_row();

            // Cost weights
            ui.label("Cost weights");
            ui.horizontal(|ui| {
                weight_field(ui, "input", &mut draft.cost_weights.input);
                weight_field(ui, "cache-write", &mut draft.cost_weights.cache_creation);
                weight_field(ui, "cache-read", &mut draft.cost_weights.cache_read);
                weight_field(ui, "output", &mut draft.cost_weights.output);
            });
            ui.end_row();
        });

    ui.add_space(8.0);
    ui.horizontal(|ui| {
        // Reads the live registry value each frame; applies immediately on toggle.
        let mut enabled = crate::autostart::is_enabled();
        if ui.checkbox(&mut enabled, "Start on login").changed() {
            let res = if enabled {
                crate::autostart::enable()
            } else {
                crate::autostart::disable()
            };
            *autostart_msg = Some(res.map_err(|e| e.to_string()));
        }
        if let Some(Err(e)) = autostart_msg.as_ref() {
            ui.label(RichText::new(format!("✗ {e}")).color(egui::Color32::from_rgb(220, 120, 120)));
        }
    });

    ui.add_space(16.0);
    ui.separator();
    ui.add_space(8.0);

    // Dirty = draft differs from the live store.
    let current = shared.read().map(|g| g.clone()).unwrap_or_default();
    let dirty = *draft != current;
    let valid = settings::validate(draft);

    ui.horizontal(|ui| {
        let can_save = dirty && valid.is_ok();
        if ui
            .add_enabled(can_save, egui::Button::new("Save"))
            .clicked()
        {
            if let Ok(mut g) = shared.write() {
                *g = draft.clone();
            }
            *save_msg = Some(settings::save(draft).map_err(|e| e.to_string()));
        }

        match (&valid, dirty, save_msg.as_ref()) {
            (Err(msg), _, _) => {
                ui.label(
                    RichText::new(format!("✗ {msg}")).color(egui::Color32::from_rgb(220, 120, 120)),
                );
            }
            (Ok(()), true, _) => {
                ui.label(
                    RichText::new("● unsaved changes")
                        .color(egui::Color32::from_rgb(220, 200, 120)),
                );
            }
            (Ok(()), false, Some(Ok(()))) => {
                ui.label(RichText::new("✓ Saved").color(egui::Color32::from_rgb(120, 200, 120)));
            }
            (Ok(()), false, Some(Err(e))) => {
                ui.label(
                    RichText::new(format!("✗ save failed: {e}"))
                        .color(egui::Color32::from_rgb(220, 120, 120)),
                );
            }
            (Ok(()), false, None) => {}
        }
    });
}

fn weight_field(ui: &mut Ui, label: &str, value: &mut f64) {
    ui.vertical(|ui| {
        ui.label(RichText::new(label).small());
        ui.add(DragValue::new(value).speed(0.05).range(0.0..=f64::MAX));
    });
}
