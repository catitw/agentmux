//! Right pane: the tab strip plus the selected session's terminal view.

use crate::app::{Action, SessionEntry};
use egui_term::TerminalView;
use std::collections::BTreeMap;

/// Horizontal tab strip: one selectable tab per session with a close button,
/// and a "+" button to create a new session.
pub fn tab_bar(
    ui: &mut egui::Ui,
    sessions: &BTreeMap<u64, SessionEntry>,
    selected: Option<u64>,
) -> Option<Action> {
    let mut action = None;
    ui.horizontal(|ui| {
        for (id, entry) in sessions {
            let is_selected = selected == Some(*id);
            let label = entry
                .terminal_title
                .as_deref()
                .unwrap_or(&entry.session.tool_name);
            if ui.selectable_label(is_selected, label).clicked() {
                action = Some(Action::Select(*id));
            }
            if ui
                .small_button("×")
                .on_hover_text("Close tab")
                .clicked()
            {
                action = Some(Action::Close(*id));
            }
        }
        ui.separator();
        if ui.button("+").on_hover_text("New session").clicked() {
            action = Some(Action::NewSession);
        }
    });
    action
}

/// Render the selected session's embedded terminal, or a failure placeholder
/// if the terminal could not be spawned.
pub fn terminal_view(ui: &mut egui::Ui, entry: &mut SessionEntry) {
    match &mut entry.backend {
        Some(backend) => {
            let view = TerminalView::new(ui, backend)
                .set_focus(true)
                .set_size(ui.available_size());
            ui.add(view);
        }
        None => {
            ui.centered_and_justified(|ui| {
                let msg = entry.spawn_error.as_deref().unwrap_or("unknown error");
                ui.label(format!(
                    "Failed to start {}: {}",
                    entry.session.tool_name, msg
                ));
            });
        }
    }
}

/// Placeholder shown when there are no sessions yet.
pub fn empty_placeholder(ui: &mut egui::Ui) {
    ui.centered_and_justified(|ui| {
        ui.label("No session — click + to create one");
    });
}
