//! Lightweight typed toasts (top-right overlay, auto-dismissing).
//!
//! Each toast carries a kind that drives a left accent bar — the semantic
//! status family, not the app accent: Info = neutral, Attention = orange,
//! Finished = green.

use egui::{Align2, Color32, Margin, Order, Rect, RichText, Vec2};
use std::time::{Duration, Instant};

/// Maximum toasts kept in the queue at once (oldest dropped).
const MAX_TOASTS: usize = 5;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToastKind {
    /// Agent detected / informational.
    Info,
    /// Agent needs attention (blocked).
    Attention,
    /// Agent finished (working → idle).
    Finished,
}

impl ToastKind {
    /// Left accent bar color (semantic status family, not the app accent).
    fn accent(self) -> Color32 {
        match self {
            ToastKind::Info => Color32::from_rgb(138, 148, 166), // neutral gray-blue
            ToastKind::Attention => Color32::from_rgb(229, 165, 10), // blocked orange
            ToastKind::Finished => Color32::from_rgb(87, 187, 138), // done green
        }
    }
}

pub struct Toast {
    pub text: String,
    pub kind: ToastKind,
    pub created: Instant,
}

/// Small queue of auto-dismissing toasts.
#[derive(Default)]
pub struct ToastQueue {
    pub toasts: Vec<Toast>,
    pub dismiss_after: Duration,
}

impl ToastQueue {
    pub fn new() -> Self {
        Self {
            toasts: Vec::new(),
            dismiss_after: Duration::from_secs(4),
        }
    }

    /// Enqueue a typed toast (dropping the oldest when over capacity).
    pub fn push(&mut self, text: impl Into<String>, kind: ToastKind) {
        self.toasts.push(Toast {
            text: text.into(),
            kind,
            created: Instant::now(),
        });
        if self.toasts.len() > MAX_TOASTS {
            self.toasts.remove(0);
        }
    }

    /// Drop toasts past their dismissal time.
    pub fn update(&mut self, now: Instant) {
        self.toasts
            .retain(|toast| now.duration_since(toast.created) < self.dismiss_after);
    }

    /// Render the queue as an egui overlay pinned to the top-right corner:
    /// a soft raised frame per toast with a kind-colored left accent bar.
    pub fn show(&mut self, ctx: &egui::Context) {
        self.update(Instant::now());
        if self.toasts.is_empty() {
            return;
        }
        egui::Area::new(egui::Id::new("agentmux_toasts"))
            .anchor(Align2::RIGHT_TOP, [-12.0, 12.0])
            .order(Order::Foreground)
            .show(ctx, |ui| {
                for toast in &self.toasts {
                    let age = Instant::now().duration_since(toast.created);
                    // Fade out over the last 25% of the lifetime.
                    let remaining = self.dismiss_after.saturating_sub(age);
                    let alpha = (remaining.as_secs_f32()
                        / (self.dismiss_after.as_secs_f32() * 0.25))
                    .clamp(0.0, 1.0);

                    egui::Frame::new()
                        .fill(ui.visuals().extreme_bg_color)
                        .stroke(ui.visuals().widgets.noninteractive.bg_stroke)
                        .corner_radius(6)
                        .inner_margin(Margin::symmetric(12, 8))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                // Kind-colored accent bar on the left edge.
                                let bar = Rect::from_min_size(
                                    ui.cursor().min,
                                    Vec2::new(3.0, 22.0),
                                );
                                ui.painter().rect_filled(
                                    bar,
                                    egui::CornerRadius::ZERO,
                                    toast.kind.accent(),
                                );
                                ui.add_space(8.0);
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(&toast.text)
                                            .color(ui.visuals().text_color().gamma_multiply(alpha)),
                                    )
                                    .wrap(),
                                );
                            });
                        });
                    ui.add_space(4.0);
                }
            });
    }
}
