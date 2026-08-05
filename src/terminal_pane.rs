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
            // Tab label: agent marker (state-colored dot + agent name) when
            // an agent is detected, else the terminal title / tool name.
            // A ⚡ marks hook-authoritative state.
            let label: egui::WidgetText = match &entry.detection {
                Some(detection) => {
                    tab_label_with_marker(detection, entry.hook.is_some())
                }
                None => entry
                    .terminal_title
                    .as_deref()
                    .unwrap_or(&entry.session.tool_name)
                    .into(),
            };
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

/// Tab label for a detected agent: a state-colored "●" dot followed by the
/// agent's display name (e.g. "● Claude Code"), with a "⚡" marker when the
/// state is hook-authoritative.
fn tab_label_with_marker(detection: &crate::detect::Detection, hook_authoritative: bool) -> egui::WidgetText {
    let font = egui::FontId::proportional(14.0);
    let mut job = egui::text::LayoutJob::default();
    job.append(
        "● ",
        0.0,
        egui::TextFormat {
            font_id: font.clone(),
            color: detection.state.color(),
            ..Default::default()
        },
    );
    job.append(
        detection.agent.display_name(),
        0.0,
        egui::TextFormat {
            font_id: font.clone(),
            color: egui::Color32::PLACEHOLDER, // inherits widget color
            ..Default::default()
        },
    );
    if hook_authoritative {
        job.append(
            " ⚡",
            0.0,
            egui::TextFormat {
                font_id: font,
                color: egui::Color32::PLACEHOLDER,
                ..Default::default()
            },
        );
    }
    job.into()
}

/// Render the selected session's embedded terminal, or a failure placeholder
/// if the terminal could not be spawned.
pub fn terminal_view(
    ui: &mut egui::Ui,
    entry: &mut SessionEntry,
    terminal_font: &egui_term::TerminalFont,
) {
    match &mut entry.backend {
        Some(backend) => {
            let view = TerminalView::new(ui, backend)
                .set_font(terminal_font.clone())
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
