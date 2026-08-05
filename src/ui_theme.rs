//! UI chrome visuals derived from the terminal palette.
//!
//! egui's default dark visuals use neutral grays that clash with the
//! navy-tinted terminal palette. We derive an egui [`Visuals`] from the
//! SAME palette the terminal uses (see [`crate::theme`]) so the chrome and
//! the terminal read as one surface family, with ONE accent — the palette
//! blue — for selection/active/focused widgets. Status colors
//! (working/idle/blocked/done/error) stay semantic and are NOT accents.

use crate::theme::UiPalette;
use egui::style::WidgetVisuals;
use egui::{Color32, CornerRadius, Shadow, Stroke, Visuals};

/// Blend two colors: `t = 0.0` → a, `t = 1.0` → b.
pub fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let f = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgb(f(a.r(), b.r()), f(a.g(), b.g()), f(a.b(), b.b()))
}

/// Derive the app chrome visuals from a terminal palette.
pub fn build_visuals(p: &UiPalette) -> Visuals {
    let mut v = Visuals::dark();
    let bg = p.bg;
    let fg = p.fg;
    let accent = p.ansi[4];

    // Surfaces: one background family, raised by blending toward the text
    // color — no neutral grays, no pure black.
    v.panel_fill = bg;
    v.window_fill = bg;
    v.extreme_bg_color = mix(bg, fg, 0.10);
    v.faint_bg_color = mix(bg, fg, 0.04);
    v.code_bg_color = mix(bg, fg, 0.07);
    v.text_edit_bg_color = Some(mix(bg, fg, 0.08));

    // Containers: slightly rounder, low-contrast strokes, tinted shadows.
    v.window_corner_radius = CornerRadius::same(6);
    v.menu_corner_radius = CornerRadius::same(6);
    v.window_stroke = Stroke::new(1.0, mix(bg, fg, 0.14));
    v.window_shadow = Shadow {
        offset: [0, 6],
        blur: 20,
        spread: 0,
        color: Color32::from_rgba_unmultiplied(0x0a, 0x0a, 0x16, 120), // navy-tinted
    };
    v.popup_shadow = Shadow {
        offset: [0, 4],
        blur: 16,
        spread: 0,
        color: Color32::from_rgba_unmultiplied(0x0a, 0x0a, 0x16, 120),
    };

    // ONE accent: the palette blue, for selection and hyperlinks.
    v.selection.bg_fill = accent;
    v.selection.stroke = Stroke::new(1.0, accent);
    v.hyperlink_color = accent;

    // Semantic feedback colors from the palette (not accents).
    v.warn_fg_color = p.ansi[3];
    v.error_fg_color = p.ansi[1];

    // Widget states: neutral surfaces resting, accent-tinted on hover/press.
    let surface = |t: f32| mix(bg, fg, t);
    let accent_tint = |t: f32| mix(bg, accent, t);
    v.widgets.noninteractive = widget(surface(0.06), surface(0.05), surface(0.10), fg, 5);
    v.widgets.inactive = widget(surface(0.08), surface(0.07), surface(0.14), fg, 4);
    v.widgets.hovered = widget(accent_tint(0.16), accent_tint(0.10), surface(0.22), fg, 4);
    v.widgets.active = widget(accent_tint(0.26), accent_tint(0.18), surface(0.26), fg, 4);
    v.widgets.open = widget(accent_tint(0.12), surface(0.06), surface(0.16), fg, 5);

    v
}

fn widget(
    bg_fill: Color32,
    weak_bg_fill: Color32,
    bg_stroke_color: Color32,
    fg: Color32,
    radius: u8,
) -> WidgetVisuals {
    WidgetVisuals {
        bg_fill,
        weak_bg_fill,
        bg_stroke: Stroke::new(1.0, bg_stroke_color),
        corner_radius: CornerRadius::same(radius),
        fg_stroke: Stroke::new(1.0, fg),
        expansion: 0.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mix_blends_channels() {
        assert_eq!(mix(Color32::BLACK, Color32::WHITE, 0.0), Color32::BLACK);
        assert_eq!(mix(Color32::BLACK, Color32::WHITE, 1.0), Color32::WHITE);
        assert_eq!(
            mix(Color32::BLACK, Color32::WHITE, 0.5),
            Color32::from_rgb(128, 128, 128)
        );
        assert_eq!(
            mix(Color32::from_rgb(255, 0, 0), Color32::from_rgb(0, 0, 255), 0.5),
            Color32::from_rgb(128, 0, 128)
        );
    }

    #[test]
    fn visuals_derive_from_mocha() {
        let v = build_visuals(&crate::theme::MOCHA);
        // Surfaces come from the palette, not egui's neutral grays.
        assert_eq!(v.panel_fill, crate::theme::MOCHA.bg);
        assert_eq!(v.window_fill, crate::theme::MOCHA.bg);
        // One accent: selection == palette blue.
        assert_eq!(v.selection.bg_fill, crate::theme::MOCHA.ansi[4]);
        // No pure black surfaces.
        assert!(v.panel_fill != Color32::BLACK);
        assert!(v.extreme_bg_color != Color32::BLACK);
        // Error/warn colors are semantic, not the accent.
        assert_eq!(v.error_fg_color, crate::theme::MOCHA.ansi[1]);
        assert_ne!(v.error_fg_color, v.selection.bg_fill);
    }
}
