//! Left panel: the session overview list (status dot + tool + work dir).

use crate::app::{Action, SessionEntry};
use egui::{Align, Align2, Color32};
use std::collections::BTreeMap;

const ROW_HEIGHT: f32 = 28.0;

/// Render the sidebar. Returns the action the user requested, if any.
pub fn show(
    ui: &mut egui::Ui,
    sessions: &BTreeMap<u64, SessionEntry>,
    selected: Option<u64>,
) -> Option<Action> {
    let mut action = None;

    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.heading("Sessions");
        ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
            if ui.button("+").on_hover_text("New session").clicked() {
                action = Some(Action::NewSession);
            }
        });
    });
    ui.separator();

    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for (id, entry) in sessions {
                let row = session_row(ui, entry, selected == Some(*id));
                if row.clicked() {
                    action = Some(Action::Select(*id));
                }
            }
        });

    action
}

/// One sidebar row: status dot, tool name, right-aligned work-dir basename.
fn session_row(ui: &mut egui::Ui, entry: &SessionEntry, is_selected: bool) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), ROW_HEIGHT),
        egui::Sense::click(),
    );

    let visuals = ui.visuals();
    let bg = if is_selected {
        visuals.selection.bg_fill
    } else if response.hovered() {
        visuals.widgets.hovered.weak_bg_fill
    } else {
        Color32::TRANSPARENT
    };
    ui.painter().rect_filled(rect, 4.0, bg);

    let session = &entry.session;

    let dot_center = egui::pos2(rect.left() + 12.0, rect.center().y);
    ui.painter().circle_filled(dot_center, 4.5, session.status.color());

    let text_color = if is_selected {
        visuals.selection.stroke.color
    } else {
        visuals.text_color()
    };
    ui.painter().text(
        egui::pos2(rect.left() + 24.0, rect.center().y),
        Align2::LEFT_CENTER,
        &session.tool_name,
        egui::FontId::proportional(14.0),
        text_color,
    );

    let basename = session
        .work_dir
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| session.work_dir.display().to_string());
    ui.painter().text(
        egui::pos2(rect.right() - 8.0, rect.center().y),
        Align2::RIGHT_CENTER,
        basename,
        egui::FontId::proportional(12.0),
        visuals.weak_text_color(),
    );

    response.on_hover_text(format!(
        "session {}: {} — {}\n{}\nstatus: {}",
        session.id,
        session.tool_name,
        session.command,
        session.work_dir.display(),
        session.status.label()
    ))
}
