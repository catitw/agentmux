//! Left panel: the session overview, grouped as a project → branch →
//! session tree (see crate::project).

use crate::app::{Action, SessionEntry};
use egui::{Align, Align2, Color32};
use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;

const ROW_HEIGHT: f32 = 28.0;

/// Render the sidebar. Returns the action the user requested, if any.
/// `collapsed` holds project roots the user collapsed (not persisted).
pub fn show(
    ui: &mut egui::Ui,
    sessions: &BTreeMap<u64, SessionEntry>,
    selected: Option<u64>,
    collapsed: &mut HashSet<PathBuf>,
    renaming: &mut Option<(u64, String)>,
) -> Option<Action> {
    let mut action = None;

    ui.add_space(8.0);
    ui.horizontal(|ui| {
        // Typography hierarchy: smaller than the old heading() — the list
        // is the content, the header should not compete with it.
        ui.label(egui::RichText::new("Sessions").size(14.0).strong());
        ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
            if ui.button("+").on_hover_text("New session").clicked() {
                action = Some(Action::NewSession);
            }
        });
    });
    ui.add_space(4.0);
    ui.separator();

    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let grouping = crate::project::group_sessions(
                &sessions
                    .iter()
                    .map(|(id, entry)| (*id, entry.project.clone()))
                    .collect::<Vec<_>>(),
            );
            for group in grouping {
                project_header(ui, &group, collapsed);
                if collapsed.contains(&group.path) {
                    continue;
                }
                ui.indent(egui::Id::new(("project", &group.path)), |ui| {
                    if group.branches.is_empty() {
                        // Non-git project: sessions directly under it.
                        for id in &group.sessions {
                            session_row_with_select(
                                ui, sessions, *id, selected, &mut action, renaming,
                            );
                        }
                    } else {
                        for branch in &group.branches {
                            // "⎇" (U+2387) exists in no available font; the
                            // diamond U+25C6 is covered by the CJK fallback.
                            ui.label(
                                egui::RichText::new(format!("◆ {}", branch.branch)).weak(),
                            );
                            ui.indent(
                                egui::Id::new(("branch", &group.path, &branch.branch)),
                                |ui| {
                                    for id in &branch.sessions {
                                        session_row_with_select(
                                            ui, sessions, *id, selected, &mut action, renaming,
                                        );
                                    }
                                },
                            );
                        }
                    }
                });
            }
        });

    action
}

/// One project header row: collapse chevron + name; tooltip shows the full
/// path. Click toggles collapse.
fn project_header(
    ui: &mut egui::Ui,
    group: &crate::project::ProjectGroup,
    collapsed: &mut HashSet<PathBuf>,
) {
    let is_collapsed = collapsed.contains(&group.path);
    // U+25B6/U+25BC: covered by egui's defaults and the CJK fallback
    // (the small U+25B8/U+25BE variants are not).
    let chevron = if is_collapsed { "▶" } else { "▼" };
    let header = ui
        .add(
            egui::Label::new(
                egui::RichText::new(format!("{chevron} {}", group.name)).strong(),
            )
            .sense(egui::Sense::click()),
        )
        .on_hover_text(group.path.display().to_string());
    if header.clicked() {
        if is_collapsed {
            collapsed.remove(&group.path);
        } else {
            collapsed.insert(group.path.clone());
        }
    }
}

/// Session row + click-to-select, with a small top spacing for readability
/// inside the tree.
fn session_row_with_select(
    ui: &mut egui::Ui,
    sessions: &BTreeMap<u64, SessionEntry>,
    id: u64,
    selected: Option<u64>,
    action: &mut Option<Action>,
    renaming: &mut Option<(u64, String)>,
) {
    let Some(entry) = sessions.get(&id) else {
        return;
    };
    let is_renaming = renaming.as_ref().is_some_and(|(rid, _)| *rid == id);
    let row = session_row(ui, entry, selected == Some(id), is_renaming);

    // Right-click context menu: rename entry point (session rows only).
    row.context_menu(|ui| {
        if ui.button("Rename session").clicked() {
            *action = Some(Action::StartRename(id));
            ui.close();
        }
    });

    if is_renaming
        && let Some((_, text)) = renaming.as_mut()
    {
        // Inline rename edit overlaying the painted label.
        let edit_id = ui.id().with(("rename", id));
        let edit_rect = egui::Rect::from_min_size(
            egui::pos2(row.rect.left() + 22.0, row.rect.top() + 3.0),
            egui::vec2(row.rect.width() - 30.0, ROW_HEIGHT - 6.0),
        );
        let response = ui.put(
            edit_rect,
            egui::TextEdit::singleline(text)
                .id(edit_id)
                .desired_width(edit_rect.width() - 8.0),
        );
        if !ui.memory(|m| m.has_focus(edit_id)) {
            ui.memory_mut(|m| m.request_focus(edit_id));
        }
        if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            *action = Some(Action::CommitRename(id, Some(text.clone())));
        } else if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            *action = Some(Action::CancelRename);
        }
        let _ = response;
        return; // the painted row body is skipped while renaming
    }

    if row.clicked() {
        *action = Some(Action::Select(id));
    }
}

/// One sidebar row: status dot, primary label, right-aligned work-dir
/// basename.
///
/// With an agent detected the row shows the agent's display name and a
/// state-colored dot (orange = blocked, blue = working, gray = idle);
/// otherwise the process-status dot and tool name as before.
fn session_row(
    ui: &mut egui::Ui,
    entry: &SessionEntry,
    is_selected: bool,
    is_renaming: bool,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), ROW_HEIGHT),
        egui::Sense::click(),
    );

    let visuals = ui.visuals();
    let accent = visuals.selection.bg_fill;
    // Selected: a soft accent-tinted fill (the saturated accent is reserved
    // for the edge bar) + a 2px accent bar at the left edge. Pressed is a
    // slightly darker accent tint; hover stays subtle.
    let bg = if is_selected {
        crate::ui_theme::mix(visuals.panel_fill, accent, 0.16)
    } else if response.is_pointer_button_down_on() {
        crate::ui_theme::mix(visuals.panel_fill, accent, 0.24)
    } else if response.hovered() {
        visuals.widgets.hovered.weak_bg_fill
    } else {
        Color32::TRANSPARENT
    };
    ui.painter().rect_filled(rect, 4.0, bg);
    if is_selected {
        ui.painter().rect_filled(
            egui::Rect::from_min_size(
                egui::pos2(rect.left() + 2.0, rect.top() + 4.0),
                egui::vec2(2.5, rect.height() - 8.0),
            ),
            egui::CornerRadius::ZERO,
            accent,
        );
    }

    let session = &entry.session;

    // Status dot stays semantic; the label follows the precedence
    // custom name > detected agent > tool name.
    let dot_color = match &entry.detection {
        Some(detection) => detection.state.color(),
        None => session.status.color(),
    };

    let dot_center = egui::pos2(rect.left() + 12.0, rect.center().y);
    ui.painter().circle_filled(dot_center, 4.5, dot_color);

    if !is_renaming {
        let text_color = visuals.text_color();
        ui.painter().text(
            egui::pos2(rect.left() + 24.0, rect.center().y),
            Align2::LEFT_CENTER,
            entry.sidebar_label(),
            egui::FontId::proportional(14.0),
            text_color,
        );
    }

    let basename = session
        .work_dir
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| session.work_dir.display().to_string());
    // On the selected tint the weak color can drop too low; keep it readable.
    let basename_color = if is_selected {
        visuals.text_color().gamma_multiply(0.8)
    } else {
        visuals.weak_text_color()
    };
    ui.painter().text(
        egui::pos2(rect.right() - 8.0, rect.center().y),
        Align2::RIGHT_CENTER,
        basename,
        egui::FontId::proportional(12.0),
        basename_color,
    );

    let agent_line = match &entry.detection {
        Some(detection) => {
            let source = match &entry.hook {
                Some(hook) => {
                    let age = std::time::Instant::now()
                        .duration_since(hook.reported_at)
                        .as_secs();
                    format!(
                        " (hook{} · {}s ago), source: hook",
                        hook.message
                            .as_deref()
                            .map(|m| format!(": {m}"))
                            .unwrap_or_default(),
                        age
                    )
                }
                None => ", source: screen".to_owned(),
            };
            format!(
                "agent: {} ({}){source}\n",
                detection.agent.display_name(),
                detection.state.label()
            )
        }
        None => String::new(),
    };
    response.on_hover_text(format!(
        "session {}: {} — {}\n{}{}\nstatus: {}",
        session.id,
        session.tool_name,
        session.command,
        agent_line,
        session.work_dir.display(),
        session.status.label()
    ))
}
