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
                            session_row_with_select(ui, sessions, *id, selected, &mut action);
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
                                            ui, sessions, *id, selected, &mut action,
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
) {
    let Some(entry) = sessions.get(&id) else {
        return;
    };
    let row = session_row(ui, entry, selected == Some(id));
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

    // Agent layer takes visual precedence when present; a ⚡ marks
    // hook-authoritative state (herdr Channel C).
    let (dot_color, primary_label) = match &entry.detection {
        Some(detection) if entry.hook.is_some() => (
            detection.state.color(),
            format!("{} ⚡", detection.agent.display_name()),
        ),
        Some(detection) => (detection.state.color(), detection.agent.display_name().to_owned()),
        None => (session.status.color(), session.tool_name.clone()),
    };

    let dot_center = egui::pos2(rect.left() + 12.0, rect.center().y);
    ui.painter().circle_filled(dot_center, 4.5, dot_color);

    let text_color = if is_selected {
        visuals.selection.stroke.color
    } else {
        visuals.text_color()
    };
    ui.painter().text(
        egui::pos2(rect.left() + 24.0, rect.center().y),
        Align2::LEFT_CENTER,
        primary_label,
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
