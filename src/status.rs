//! Mapping from PTY events to the process-layer session status, plus the
//! colors/labels the UI uses to render that status.

use crate::session::SessionStatus;
use egui::Color32;

/// Map a PTY event to a status transition, if the event carries one.
///
/// `ChildExit` carries the real exit status, which distinguishes a clean
/// `Done` from a non-zero `Error`. `Exit` is the PTY shutdown event that
/// follows the child's death; it carries no exit code, so it is kept as a
/// `Done` fallback (the `ChildExit` event normally arrives first anyway).
pub fn status_from_pty_event(event: &egui_term::PtyEvent) -> Option<SessionStatus> {
    match event {
        egui_term::PtyEvent::ChildExit(status) => Some(if status.success() {
            SessionStatus::Done
        } else {
            SessionStatus::Error
        }),
        egui_term::PtyEvent::Exit => Some(SessionStatus::Done),
        _ => None,
    }
}

impl SessionStatus {
    /// Accent color for the status indicator (sidebar dot).
    pub fn color(self) -> Color32 {
        match self {
            SessionStatus::Running => Color32::from_rgb(86, 156, 255),
            SessionStatus::Done => Color32::from_rgb(87, 187, 138),
            SessionStatus::Error => Color32::from_rgb(229, 72, 77),
        }
    }

    /// Short human label, used in tooltips.
    pub fn label(self) -> &'static str {
        match self {
            SessionStatus::Running => "running",
            SessionStatus::Done => "done",
            SessionStatus::Error => "error",
        }
    }
}
