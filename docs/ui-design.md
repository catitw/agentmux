# UI design — audit & redesign

Applied the `redesign-existing-projects` skill to the egui app, per the
skill's sequence (scan → diagnose → fix), working with the existing stack
(egui immediate mode, no CSS, minimal animation).

## Audit (skill checklist → agentmux)

**Applied:**

| Skill item | agentmux finding |
|---|---|
| Palette cleanup, one accent, no clashing colors | Chrome was egui-default neutral gray (#191919) with a saturated VS-Code-blue selection fill — two unrelated "darks" next to the navy terminal. Fixed: chrome derived from the terminal palette, one accent (palette blue). |
| Hover/active/pressed states | Rows had hover only; no pressed or selection affordance beyond a full-width saturated fill. Fixed: soft tint + left accent bar + pressed tint; tabs got hover/subtle-close states. |
| Empty states | Bare `No session — click + to create one`. Fixed: composed centered state (title + hint + button opening the same dialog). |
| Error states | Spawn failure was a bare label. Fixed: error-colored title + message in a frame; inline dialog validation already existed. |
| Typography hierarchy | "Sessions" used `heading()` (large). Fixed: 14px strong label — the list is the content. |
| Spacing/rhythm | Default egui spacing. Fixed: 8px item spacing, comfortable button padding, dialog field spacing. |
| Generic components | Toasts were uniform popups; now typed with semantic accent bars. Dialog got a primary (accent) button. |
| One gray family / no pure black | All surfaces now come from the palette (no #000, no neutral grays). |
| Active-state indication (tabs) | Active tab was a floating saturated pill; now connected to the terminal (same fill) with a bottom accent line. |
| Consistent corner radii | Containers 6, controls 4-5 (egui per-widget-state radius; not per-widget granular). |

**Not applicable** (web/CSS-oriented): fonts (we intentionally keep egui's
default UI font; the terminal font is Maple Mono NF CN already), CSS
layout/grid/max-width/100vh items, box-shadows (egui shadows are tinted
navy now), gradients/noise/parallax (no images, immediate mode), SEO/meta/
legal/cookie items, favicon, icon sets (we use text glyphs), responsive
mobile items.

## Palette derivation (`src/ui_theme.rs`)

`theme::load_terminal_theme()` now returns the resolved `UiPalette`
(fg/bg/16 ANSI as `Color32`) alongside the terminal theme; when no ghostty
palette exists the built-in **Catppuccin Mocha** constants are used for BOTH
the terminal and the chrome (same values as ghostty's noctalia, so one color
family always).

`ui_theme::build_visuals(&UiPalette)` derives the egui `Visuals`:

- Surfaces: `panel_fill`/`window_fill` = palette bg (#1e1e2e); raised
  surfaces = bg blended toward the text color (10%, 8%, 7%…); text edits on
  `extreme_bg_color`; no pure black, one gray family (the palette's).
- ONE accent: `selection.bg_fill`/`stroke` + `hyperlink_color` = palette
  blue (ansi[4], #89b4fa). Hover/press tints blend the surface toward the
  accent (16%/26%) so interaction feedback stays in-family.
- Semantic colors stay separate: `warn_fg_color` = palette yellow,
  `error_fg_color` = palette red; status dots/toast bars keep their
  working/idle/blocked/done/error colors — those are NOT accents.
- Shadows tinted navy (from-rgba 0a0a16@120), strokes at 10-14% blends,
  window radius 6, widget radius 4-5.

Applied once at startup via `ctx.set_visuals` + `style_mut_of(Dark, …)`
(item spacing 8x5, button padding 10x4).

## Per-widget changes

- **sidebar.rs**: "Sessions" 14px strong; selected row = accent tint (16%)
  + 2.5px left accent bar + pressed tint (24%); basename stays readable on
  selection; tree glyphs (▼/◆/●) already covered by fonts.
- **terminal_pane.rs**: custom-drawn tabs — active = panel fill (connects to
  the terminal) with a bottom accent line; inactive muted, faint fill on
  hover; close × subtle (70% weak) until hovered; composed empty state;
  spawn-failure in an error frame (error-colored title + message).
- **notify.rs**: typed toasts (`Info`/`Attention`/`Finished`) with a
  kind-colored left accent bar (neutral / blocked-orange / done-green) in a
  raised frame; 4s dismiss + fade unchanged.
- **new_session.rs**: 8px field rhythm, error text via
  `visuals.error_fg_color`, primary Create button in the accent, Cancel
  quiet.

## Before / After

- `/tmp/ui_before.png`: neutral-gray chrome vs navy terminal, saturated
  VS-Code-blue selection pill, large heading, floating tab pills, bare
  placeholder, dim on-blue text.
- `/tmp/ui_after.png`: chrome == terminal (#1e1e2e sampled exactly), soft
  tinted selection + accent bar (#89b4fa sampled), understated header,
  connected active tab with accent underline, subtle close, one accent
  everywhere.
- `/tmp/ui_empty.png`: composed empty state (title/hint/+ button).
- `/tmp/ui_fail2.png`: error frame + red status dot.

## Notes / deviations

- Pixel-verified rather than eyeballed: sidebar bg (30,30,46) == terminal
  bg; accent bar and tab underline both exact #89B4FA; status dot #569CFF
  unchanged (semantic).
- The skill's "letterspacing/negative tracking" items are not available in
  egui; hierarchy is done with size/weight instead (per the director's
  note). No fonts were bundled; the UI keeps egui's proportional font.
- Toast/dialog interactions were not clickable in captures (no synthetic
  input on this machine); their code paths are small and reviewed, the
  visible states (empty, error) were captured.
