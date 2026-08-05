//! Lightweight toast notifications (top-right overlay, auto-dismissing).

use std::time::{Duration, Instant};

/// Maximum toasts kept in the queue at once (oldest dropped).
const MAX_TOASTS: usize = 5;

pub struct Toast {
    pub text: String,
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

    /// Enqueue a toast (dropping the oldest when over capacity).
    pub fn push(&mut self, text: impl Into<String>) {
        self.toasts.push(Toast {
            text: text.into(),
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

    /// Render the queue as an egui overlay pinned to the top-right corner.
    pub fn show(&mut self, ctx: &egui::Context) {
        self.update(Instant::now());
        if self.toasts.is_empty() {
            return;
        }
        egui::Area::new(egui::Id::new("agentmux_toasts"))
            .anchor(egui::Align2::RIGHT_TOP, [-12.0, 12.0])
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    for toast in &self.toasts {
                        let age = Instant::now().duration_since(toast.created);
                        // Fade out over the last 25% of the lifetime.
                        let remaining = self.dismiss_after.saturating_sub(age);
                        let alpha = (remaining.as_secs_f32() / (self.dismiss_after.as_secs_f32() * 0.25))
                            .clamp(0.0, 1.0);
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(&toast.text)
                                    .color(ui.visuals().text_color().gamma_multiply(alpha)),
                            )
                            .wrap(),
                        );
                        ui.add_space(4.0);
                    }
                });
            });
    }
}
