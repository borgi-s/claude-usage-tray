//! Sessions table tab: TableBuilder over per-session summaries, with a sort
//! selector and a degenerate-session filter. Pure aggregation lives in
//! `crate::data::sessions`.

use crate::dashboard::filters::short_project;
use crate::dashboard::kpi::format_si;
use crate::data::parser::Turn;
use crate::data::sessions::{hide_degenerate, session_summaries, sort_sessions, SortKey};
use egui::Ui;
use egui_extras::{Column, TableBuilder};

/// Persistent table-local controls (live on DashboardApp).
pub struct TableControls {
    pub sort: SortKey,
    pub min_turns: usize,
    pub min_duration_s: f64,
}

impl Default for TableControls {
    fn default() -> Self {
        Self {
            sort: SortKey::Chronological,
            min_turns: 5,
            min_duration_s: 60.0,
        }
    }
}

pub fn render(ui: &mut Ui, turns: &[Turn], controls: &mut TableControls, tz: chrono_tz::Tz, w: &crate::settings::CostWeights) {
    // Controls row.
    ui.horizontal(|ui| {
        ui.label("Sort:");
        egui::ComboBox::from_id_salt("sessions_sort")
            .selected_text(sort_label(controls.sort))
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut controls.sort, SortKey::Chronological, "Chronological");
                ui.selectable_value(&mut controls.sort, SortKey::PeakCtx, "Peak ctx%");
                ui.selectable_value(&mut controls.sort, SortKey::TotalCost, "Total cost");
            });
        ui.separator();
        ui.label("Min turns:");
        ui.add(egui::DragValue::new(&mut controls.min_turns).range(0..=1000));
        ui.label("Min duration (s):");
        ui.add(egui::DragValue::new(&mut controls.min_duration_s).range(0.0..=86_400.0));
    });

    let mut summaries = session_summaries(turns, w);
    sort_sessions(&mut summaries, controls.sort);
    let (summaries, hidden) =
        hide_degenerate(summaries, controls.min_turns, controls.min_duration_s);

    ui.label(
        egui::RichText::new(format!(
            "{} session(s) · {} degenerate hidden",
            summaries.len(),
            hidden
        ))
        .size(11.0)
        .color(egui::Color32::GRAY),
    );
    ui.add_space(4.0);

    TableBuilder::new(ui)
        .striped(true)
        .resizable(true)
        .column(Column::initial(140.0)) // start
        .column(Column::initial(120.0)) // project
        .column(Column::initial(150.0)) // model
        .column(Column::initial(70.0)) // main_turns
        .column(Column::initial(70.0)) // subagents
        .column(Column::initial(80.0)) // peak ctx%
        .column(Column::initial(90.0)) // peak prompt
        .column(Column::initial(70.0)) // main M
        .column(Column::initial(70.0)) // sub M
        .column(Column::remainder()) // total M
        .header(20.0, |mut header| {
            for title in [
                "start",
                "project",
                "model",
                "main",
                "subs",
                "ctx%",
                "peak prompt",
                "main M",
                "sub M",
                "total M",
            ] {
                header.col(|ui| {
                    ui.strong(title);
                });
            }
        })
        .body(|mut body| {
            for s in &summaries {
                body.row(18.0, |mut row| {
                    row.col(|ui| {
                        ui.label(
                            s.start
                                .with_timezone(&tz)
                                .format("%Y-%m-%d %H:%M")
                                .to_string(),
                        );
                    });
                    row.col(|ui| {
                        ui.label(short_project(&s.project_cwd));
                    });
                    row.col(|ui| {
                        ui.label(&s.model);
                    });
                    row.col(|ui| {
                        ui.label(s.main_turns.to_string());
                    });
                    row.col(|ui| {
                        ui.label(s.subagent_count.to_string());
                    });
                    row.col(|ui| {
                        ui.label(format!("{:.1}", s.peak_context_pct * 100.0));
                    });
                    row.col(|ui| {
                        ui.label(format_si(s.peak_prompt_tokens as f64));
                    });
                    row.col(|ui| {
                        ui.label(format!("{:.2}", s.main_cost_weighted / 1e6));
                    });
                    row.col(|ui| {
                        ui.label(format!("{:.2}", s.subagent_cost_weighted / 1e6));
                    });
                    row.col(|ui| {
                        ui.label(format!("{:.2}", s.total_cost_weighted / 1e6));
                    });
                });
            }
        });
}

fn sort_label(key: SortKey) -> &'static str {
    match key {
        SortKey::Chronological => "Chronological",
        SortKey::PeakCtx => "Peak ctx%",
        SortKey::TotalCost => "Total cost",
    }
}
