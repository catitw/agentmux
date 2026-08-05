//! Right pane: the tab strip plus the selected session's terminal view.

use crate::app::{Action, SessionEntry};
use egui::{Align2, Color32, CornerRadius, FontId, Rect, RichText, Sense, Vec2};
use egui_term::TerminalView;
use std::collections::BTreeMap;

/// Horizontal tab strip: one tab per session with a close button, and a "+"
/// button to create a new session.
///
/// The active tab is visually connected to the terminal below it: it uses
/// the panel fill (which equals the terminal background) with a bottom
/// accent line; inactive tabs are muted and only gain a faint fill on hover.
pub fn tab_bar(
    ui: &mut egui::Ui,
    sessions: &BTreeMap<u64, SessionEntry>,
    selected: Option<u64>,
) -> Option<Action> {
    let mut action = None;
    ui.horizontal(|ui| {
        for (id, entry) in sessions {
            let is_selected = selected == Some(*id);
            // Tab label precedence: custom name > detected agent (with a
            // state-colored marker; ⚡ = hook-authoritative) > terminal
            // title > tool name.
            let label: egui::WidgetText = if let Some(name) = &entry.session.custom_name {
                name.clone().into()
            } else {
                match &entry.detection {
                    Some(detection) => {
                        tab_label_with_marker(detection, entry.hook.is_some())
                    }
                    None => entry
                        .terminal_title
                        .as_deref()
                        .unwrap_or(&entry.session.tool_name)
                        .into(),
                }
            };
            let (tab, close) = tab(ui, *id, &label, is_selected);
            if tab.clicked() {
                action = Some(Action::Select(*id));
            }
            if close.clicked() {
                action = Some(Action::Close(*id));
            }
        }
        ui.add_space(4.0);
        if ui.button("+").on_hover_text("New session").clicked() {
            action = Some(Action::NewSession);
        }
    });
    action
}

/// One custom-drawn tab. Returns (tab response, close-button response); the
/// close button is registered after the tab body so it wins the hit test.
fn tab(
    ui: &mut egui::Ui,
    id: u64,
    label: &egui::WidgetText,
    is_active: bool,
) -> (egui::Response, egui::Response) {
    const HEIGHT: f32 = 30.0;
    const PAD_X: f32 = 10.0;
    const CLOSE_W: f32 = 20.0;

    let galley = label.clone().into_galley(
        ui,
        Some(egui::TextWrapMode::Extend),
        f32::INFINITY,
        egui::FontSelection::Default,
    );
    let width = galley.size().x + PAD_X * 2.0 + CLOSE_W;
    let (rect, response) =
        ui.allocate_exact_size(Vec2::new(width, HEIGHT), Sense::click());

    let visuals = ui.visuals();
    let painter = ui.painter();
    if is_active {
        // Connected to the terminal: same fill as the central panel, with a
        // bottom accent line.
        painter.rect_filled(rect, CornerRadius::same(5), visuals.panel_fill);
        painter.rect_filled(
            Rect::from_min_size(
                egui::pos2(rect.left() + 6.0, rect.bottom() - 2.0),
                Vec2::new(rect.width() - 12.0, 2.0),
            ),
            CornerRadius::ZERO,
            visuals.selection.bg_fill,
        );
    } else if response.hovered() {
        painter.rect_filled(rect, CornerRadius::same(5), visuals.widgets.hovered.weak_bg_fill);
    }

    let text_color = if is_active {
        visuals.text_color()
    } else {
        visuals.weak_text_color()
    };
    // painter.galley anchors the galley's TOP-LEFT at the position, so the
    // y must back off by half the text height to sit on the rect's midline
    // (painter.text's Align2 doesn't apply to galley). The dot/⚡ prefix is
    // part of the same galley, so it shares the midline.
    painter.galley(
        egui::pos2(
            rect.left() + PAD_X,
            rect.center().y - galley.size().y / 2.0,
        ),
        galley,
        text_color,
    );

    // Close button: subtle until hovered.
    let close_rect = Rect::from_min_size(
        egui::pos2(rect.right() - CLOSE_W - 3.0, rect.top() + (HEIGHT - 18.0) / 2.0),
        Vec2::new(18.0, 18.0),
    );
    let close = ui.interact(close_rect, ui.id().with(("tab_close", id)), Sense::click());
    let close_color = if close.hovered() {
        visuals.text_color()
    } else {
        visuals.weak_text_color().gamma_multiply(0.7)
    };
    painter.text(
        close_rect.center(),
        Align2::CENTER_CENTER,
        "×",
        FontId::proportional(13.0),
        close_color,
    );
    if close.hovered() {
        painter.rect_filled(close_rect, CornerRadius::same(4), visuals.widgets.hovered.weak_bg_fill);
    }

    (response, close.on_hover_text("Close tab"))
}

/// Tab label for a detected agent: a state-colored "●" dot followed by the
/// agent's display name (e.g. "● Claude Code"), with a "⚡" marker when the
/// state is hook-authoritative.
fn tab_label_with_marker(
    detection: &crate::detect::Detection,
    hook_authoritative: bool,
) -> egui::WidgetText {
    let font = FontId::proportional(14.0);
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
            color: Color32::PLACEHOLDER, // inherits widget color
            ..Default::default()
        },
    );
    if hook_authoritative {
        job.append(
            " ⚡",
            0.0,
            egui::TextFormat {
                font_id: font,
                color: Color32::PLACEHOLDER,
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
    terminal_theme: &egui_term::TerminalTheme,
) {
    match &mut entry.backend {
        Some(backend) => {
            let view = TerminalView::new(ui, backend)
                .set_theme(terminal_theme.clone())
                .set_font(terminal_font.clone())
                .set_focus(true)
                .set_size(ui.available_size());
            ui.add(view);
        }
        None => spawn_failure_placeholder(ui, entry),
    }
}

/// Spawn-failure placeholder: an error-styled frame (error-colored title +
/// message), not a bare label.
fn spawn_failure_placeholder(ui: &mut egui::Ui, entry: &SessionEntry) {
    ui.centered_and_justified(|ui| {
        egui::Frame::new()
            .fill(ui.visuals().extreme_bg_color)
            .stroke(ui.visuals().widgets.noninteractive.bg_stroke)
            .corner_radius(6)
            .inner_margin(egui::Margin::same(18))
            .show(ui, |ui| {
                ui.set_max_width(420.0);
                ui.vertical_centered(|ui| {
                    ui.label(
                        RichText::new(format!("Failed to start {}", entry.session.tool_name))
                            .strong()
                            .color(ui.visuals().error_fg_color),
                    );
                    ui.add_space(6.0);
                    let msg = entry.spawn_error.as_deref().unwrap_or("unknown error");
                    ui.label(RichText::new(msg).weak());
                });
            });
    });
}

/// Composed empty state: title, one-line hint, and a "+" button that opens
/// the same new-session dialog as the toolbar buttons.
pub fn empty_state(ui: &mut egui::Ui) -> Option<Action> {
    let mut action = None;
    ui.centered_and_justified(|ui| {
        ui.vertical_centered(|ui| {
            ui.add_space(6.0);
            ui.label(RichText::new("No sessions yet").size(17.0).strong());
            ui.add_space(6.0);
            ui.label(RichText::new("Open a new session to start working").weak());
            ui.add_space(16.0);
            if ui.button("+ New session").clicked() {
                action = Some(Action::NewSession);
            }
        });
    });
    action
}
