# Fonts

egui's embedded fonts (Ubuntu-Light, Hack, its emoji font) cover Latin and a
small symbol set only. CJK text, Nerd Font icons and braille spinners
rendered as tofu (□). agentmux discovers installed system fonts at startup
and appends them as egui fallbacks — no fonts are bundled.

## Fallback chain (src/fonts.rs)

Discovery runs once in `AgentMuxApp::new` (`fonts::setup_fonts`), using
`fontdb` (system font database, TTC-aware: the correct face index of a
collection is loaded into egui's `FontData.index`).

Preference order, first match wins:

1. **Icons (Nerd Font)** — exact `Symbols Nerd Font Mono` / `Symbols Nerd
   Font`, else the first installed family whose name contains
   `Nerd Font Mono`, else any containing `Nerd Font`. (On this machine:
   `FantasqueSansM Nerd Font Mono`.) After any system match, the **bundled
   Symbols Nerd Font Mono** (`assets/fonts/SymbolsNerdFontMono-Regular.ttf`,
   nerd-fonts release **v3.5.0**, sha256
   `2dc316f2505a0cbfbcf6060a1b4ba85b0a2974189e30c0037cdedc436a25a4ff`) is
   registered as the guaranteed icon floor: system fonts are preferred, but
   icon coverage no longer depends on what is installed.
2. **CJK** — `Noto Sans CJK SC`, `Noto Sans CJK TC`, `Noto Sans CJK JP`,
   `Noto Sans CJK`, `WenQuanYi Micro Hei`, `WenQuanYi Zen Hei`,
   `Microsoft YaHei`, `PingFang SC`, `Source Han Sans SC`. (This machine:
   `Noto Sans CJK SC`.) The CJK face also supplies braille (U+2800–28FF)
   and common symbols like `●`.
3. **Emoji** — `Noto Color Emoji` (optional; egui only renders its outline
   coverage, if any).

Registration:

- **UI families**: the found faces are appended to egui's `Proportional`
  and `Monospace` families — fixes the tab state marker `●`, sidebar dots,
  toasts and CJK in tooltips.
- **Terminal family**: a custom family `agentmux-terminal` is built as a
  copy of egui's `Monospace` chain (Hack first, so terminal cell metrics
  stay driven by the primary monospace font) with the same fallbacks
  appended. egui_term's `TerminalFont` is just an egui `FontId`
  (`font.rs:11-44`), and the view paints each glyph with
  `Shape::text(…, self.font.font_type(), …)` (`view.rs:337`), so egui's
  per-glyph family fallback does the rest. The terminal is pointed at the
  family via `TerminalView::set_font` (font size 14.0, unchanged).

Startup logging: `agentmux fonts: registered fallbacks: <face names…>` or
`…no system fallback fonts found…` when nothing matched.

## How to override

The preference lists are hardcoded in `fonts.rs::setup_fonts`. To force a
specific font: install it and adjust the lists (or reorder them); no config
file is read. `AGENTMUX_SEED_COMMAND` (see below) does not affect fonts.

## Bundled font & attribution

`assets/fonts/SymbolsNerdFontMono-Regular.ttf` is **Nerd Fonts
SymbolsOnly (Mono) v3.5.0**, by the nerd-fonts project
(https://github.com/ryanoasis/nerd-fonts, SIL OFL 1.1). Individual icon
sets inside the font retain their own licenses (CC BY 4.0, MIT, Apache
2.0, … — see the `README.md` table shipped with the release; the release
packaging itself is MIT). The font is embedded via `include_bytes!`
(~2.4 MB) and registered as the final icon fallback; it is never the
primary terminal font, so cell metrics stay driven by the monospace font.

## Limitation

CJK is system-only — bundling a CJK font is deliberately avoided (too
large). A minimal system without any CJK font still renders CJK as tofu
(and logs that only the bundled symbols were found). Nerd Font icons are
now guaranteed by the bundle.

## Verification hook

`AGENTMUX_SEED_COMMAND=<shell command line>` replaces the seeded session's
command (run via `sh -c` / `cmd.exe /C`; workdir stays `$HOME`). Useful for
headless glyph and behavior checks, e.g.:

```
AGENTMUX_SEED_COMMAND="echo '中文测试 ❯ ● ⚡ ⠋'; exec bash" agentmux
AGENTMUX_SEED_COMMAND=omp agentmux
```
