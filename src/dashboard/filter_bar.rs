//! Global filter bar: date range pickers + project/model multiselect menus.
//! Mutates the shared `FilterState`. Distinct option lists come from the FULL
//! (unfiltered) turn set so de-selected options remain re-selectable.

use crate::dashboard::filters::{distinct_models, distinct_projects, short_project, FilterState};
use crate::data::parser::Turn;
use egui::Ui;
use egui_extras::DatePickerButton;

pub fn render(
    ui: &mut Ui,
    all_turns: &[Turn],
    filter: &mut FilterState,
    shown: usize,
    total: usize,
) {
    ui.horizontal_wrapped(|ui| {
        // Date from.
        ui.checkbox(&mut filter.use_date_from, "From");
        ui.add_enabled(
            filter.use_date_from,
            DatePickerButton::new(&mut filter.date_from).id_salt("filter_from"),
        );
        ui.separator();
        // Date to.
        ui.checkbox(&mut filter.use_date_to, "To");
        ui.add_enabled(
            filter.use_date_to,
            DatePickerButton::new(&mut filter.date_to).id_salt("filter_to"),
        );
        ui.separator();

        // Project multiselect.
        let projects = distinct_projects(all_turns);
        ui.menu_button(project_button_label(filter), |ui| {
            for p in &projects {
                let mut checked = filter.projects.contains(p);
                if ui.checkbox(&mut checked, short_project(p)).changed() {
                    if checked {
                        filter.projects.insert(p.clone());
                    } else {
                        filter.projects.remove(p);
                    }
                }
            }
            if ui.button("Clear").clicked() {
                filter.projects.clear();
            }
        });

        // Model multiselect.
        let models = distinct_models(all_turns);
        ui.menu_button(model_button_label(filter), |ui| {
            for m in &models {
                let mut checked = filter.models.contains(m);
                if ui.checkbox(&mut checked, m).changed() {
                    if checked {
                        filter.models.insert(m.clone());
                    } else {
                        filter.models.remove(m);
                    }
                }
            }
            if ui.button("Clear").clicked() {
                filter.models.clear();
            }
        });

        ui.separator();
        ui.label(
            egui::RichText::new(format!("Showing {shown} of {total} turns"))
                .size(11.0)
                .color(egui::Color32::GRAY),
        );
    });
}

fn project_button_label(filter: &FilterState) -> String {
    if filter.projects.is_empty() {
        "Projects: all".to_string()
    } else {
        format!("Projects: {}", filter.projects.len())
    }
}

fn model_button_label(filter: &FilterState) -> String {
    if filter.models.is_empty() {
        "Models: all".to_string()
    } else {
        format!("Models: {}", filter.models.len())
    }
}
