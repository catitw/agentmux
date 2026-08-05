# Terminal theme (palette + font)

agentmux's embedded terminals adopt the look of the user's ghostty:
the same ANSI 16-color palette / background / foreground, and the same
terminal font family. This is a *look* match — layout and window width are
independent (fastfetch wraps differently at different widths; that is not a
bug).

## Palette

Resolution order (evaluated once at startup, cached for the app lifetime —
never shelled out per frame):

1. **`ghostty +show-config` output** — the *resolved* palette, theme
   included. This is the source of truth even when the user's config file
   is empty (the default theme's palette only appears here). Requires the
   `ghostty` binary on PATH.
2. **Explicit lines in `~/.config/ghostty/config`**
   (`$GHOSTTY_CONFIG_DIR` overrides): `palette = N=#rrggbb` (0–15),
   `background = #rrggbb`, `foreground = #rrggbb`.
3. **egui_term's default palette** (fallback).

`AGENTMUX_TERMINAL_PALETTE=ghostty|default` forces the source (default =
egui_term palette; unknown values warn and use default).

Parsing is dependency-free: malformed lines are skipped, never fatal; a
palette is only adopted when background + foreground + all 16 ANSI colors
are present (ghostty's resolved output always is). The 16–231 cube stays
egui_term's fixed xterm table (ghostty's per-index overrides above 15 are
ignored — egui_term's `ColorPalette` only models the 16 named colors, fg,
bg; the cube is hardcoded). Dim colors are derived as the base colors
blended 50% toward the background (alacritty's default dim behavior; the
ghostty themes used here define no dim colors).

Applied via egui_term's public theme API on every `TerminalView`:
`ColorPalette` (hex-string fields, theme.rs:9-36) →
`TerminalTheme::new(Box<ColorPalette>)` (theme.rs:83-88) →
`TerminalView::set_theme(theme)` (view.rs:91).

## Font

Terminal font preference (first match wins):

1. `AGENTMUX_TERMINAL_FONT` env override (exact family name, e.g.
   `AGENTMUX_TERMINAL_FONT="Maple Mono NF CN"`).
2. `Maple Mono NF CN` (the user's ghostty font), `Maple Mono NF`.
3. Common Nerd Font terminal faces (`JetBrainsMono Nerd Font Mono`,
   `CaskaydiaCove Nerd Font Mono`, `FiraCode Nerd Font Mono`,
   `FantasqueSansMono Nerd Font Mono`, `MesloLGS Nerd Font Mono`,
   `Hack Nerd Font Mono`).
4. egui's embedded Hack (previous behavior).

The matched font is registered and placed FIRST in the `agentmux-terminal`
family, so it drives cell metrics (egui_term's `font_measure` reads the
FontId's primary font); all fallbacks (Nerd icons, CJK, emoji, bundled
symbols) still apply after it. UI fonts (sidebar, tabs, toasts) are
unchanged. Font size stays egui_term's default 14 pt (ghostty's 12 pt is a
size preference, deliberately not copied — the family match is the visible
difference).

## Startup log

```
agentmux theme: font 'Maple Mono NF CN', palette ghostty(+show-config)
```

The ghostty dependency is optional: without the binary or config, the
palette falls back to egui_term's default and the log says so; fonts fall
back to Hack.

## Verification

Fastfetch seed (`AGENTMUX_SEED_COMMAND="fastfetch; exec bash"`), screenshot
`/tmp/theme_agentmux.png`: all 16 fastfetch color blocks sampled
pixel-perfect against ghostty's resolved noctalia palette
(0=#45475A … 7=#A6ADC8, 8=#585B70 … 15=#BAC2DE) and the terminal background
samples as #1E1E2E — the same values `ghostty +show-config` reports. A live
ghostty side-by-side capture is not possible on this machine (ghostty
selects its own GTK backend, ignoring GDK_BACKEND; its windows are
Wayland-native and not capturable by X tools), so the numeric palette
comparison stands in for it.
