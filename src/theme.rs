//! Terminal theme: adopt ghostty's resolved palette so agentmux's embedded
//! terminals look like the user's ghostty (same background, foreground and
//! 16 named ANSI colors; the 16-231 cube stays egui_term's fixed xterm
//! table). Font preference lives in fonts.rs.
//!
//! Palette resolution order (cached once at startup, never per frame):
//! 1. `ghostty +show-config` output — the RESOLVED palette (theme included),
//!    when the ghostty binary exists;
//! 2. explicit `palette = N=#RRGGBB` / `background` / `foreground` lines in
//!    `~/.config/ghostty/config` (`$GHOSTTY_CONFIG_DIR` overrides);
//! 3. egui_term's default palette.
//!
//! `AGENTMUX_TERMINAL_PALETTE=ghostty|default` forces the source.
//!
//! All parsing is dependency-free; malformed lines are skipped and never
//! fatal.

use std::path::PathBuf;

/// Where the active palette came from (startup log + tests).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaletteSource {
    GhosttyShowConfig,
    GhosttyConfigFile,
    Default,
}

impl PaletteSource {
    pub fn label(self) -> &'static str {
        match self {
            PaletteSource::GhosttyShowConfig => "ghostty(+show-config)",
            PaletteSource::GhosttyConfigFile => "ghostty(config)",
            PaletteSource::Default => "egui_term default",
        }
    }
}

impl std::fmt::Display for PaletteSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.label())
    }
}

/// A complete parsed palette: foreground, background, 16 ANSI colors.
/// Colors are stored as `#rrggbb` strings (egui_term's `ColorPalette` field
/// format).
struct ParsedPalette {
    fg: String,
    bg: String,
    colors: [String; 16],
}

/// Load the terminal theme once at startup. Returns the theme plus where the
/// palette came from (for the startup log).
pub fn load_terminal_theme() -> (egui_term::TerminalTheme, PaletteSource) {
    let force = std::env::var("AGENTMUX_TERMINAL_PALETTE").ok();
    let (parsed, source) = match force.as_deref() {
        Some("default") => (None, PaletteSource::Default),
        Some("ghostty") => resolve_ghostty_palette(),
        Some(other) => {
            eprintln!(
                "agentmux theme: unknown AGENTMUX_TERMINAL_PALETTE '{other}' \
                 (expected ghostty|default), using egui_term default"
            );
            (None, PaletteSource::Default)
        }
        None => resolve_ghostty_palette(),
    };
    let theme = match parsed {
        Some(parsed) => build_theme(&parsed),
        None => egui_term::TerminalTheme::default(),
    };
    (theme, source)
}

/// Try the two ghostty sources in priority order.
fn resolve_ghostty_palette() -> (Option<ParsedPalette>, PaletteSource) {
    // (a) Resolved config via the CLI (theme included). This is the source
    // of truth even when the user's config file is empty.
    if let Ok(output) = std::process::Command::new("ghostty").arg("+show-config").output()
        && output.status.success()
        && let Some(parsed) = parse_palette_lines(&String::from_utf8_lossy(&output.stdout))
    {
        return (Some(parsed), PaletteSource::GhosttyShowConfig);
    }

    // (b) Explicit palette/background/foreground lines in the config file.
    let config_dir = std::env::var_os("GHOSTTY_CONFIG_DIR")
        .map(PathBuf::from)
        .or_else(|| dirs::config_dir().map(|dir| dir.join("ghostty")));
    if let Some(config_dir) = config_dir
        && let Ok(content) = std::fs::read_to_string(config_dir.join("config"))
        && let Some(parsed) = parse_palette_lines(&content)
    {
        return (Some(parsed), PaletteSource::GhosttyConfigFile);
    }

    (None, PaletteSource::Default)
}

/// Parse ghostty-style lines:
///   palette = 0=#45475a
///   background = #1e1e2e
///   foreground = #cdd6f4
///
/// Returns `Some` only when foreground, background and all 16 ANSI colors
/// are present (ghostty's resolved output always is). Malformed lines are
/// skipped.
fn parse_palette_lines(text: &str) -> Option<ParsedPalette> {
    let mut colors: [Option<String>; 16] = Default::default();
    let mut fg: Option<String> = None;
    let mut bg: Option<String> = None;

    for line in text.lines() {
        let line = line.trim();
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let (key, value) = (key.trim(), value.trim());
        match key {
            "palette" => {
                let Some((index_str, hex)) = value.split_once('=') else {
                    continue;
                };
                let Ok(index) = index_str.trim().parse::<usize>() else {
                    continue;
                };
                if index < 16
                    && let Some(hex) = normalize_hex(hex.trim())
                {
                    colors[index] = Some(hex);
                }
            }
            "background" => {
                if let Some(hex) = normalize_hex(value) {
                    bg = Some(hex);
                }
            }
            "foreground" => {
                if let Some(hex) = normalize_hex(value) {
                    fg = Some(hex);
                }
            }
            _ => {}
        }
    }

    let colors: Option<[String; 16]> = {
        let mut out: [String; 16] = std::array::from_fn(|_| String::new());
        for (slot, value) in colors.iter().enumerate() {
            out[slot] = value.clone()?;
        }
        Some(out)
    };
    Some(ParsedPalette {
        fg: fg?,
        bg: bg?,
        colors: colors?,
    })
}

/// Accept `#rrggbb` (or bare `rrggbb`); anything else is skipped.
fn normalize_hex(value: &str) -> Option<String> {
    let value = value.strip_prefix('#').unwrap_or(value);
    if value.len() != 6 || !value.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    Some(format!("#{value}"))
}

fn parse_rgb(hex: &str) -> (u8, u8, u8) {
    let hex = hex.trim_start_matches('#');
    (
        u8::from_str_radix(&hex[0..2], 16).unwrap_or(0),
        u8::from_str_radix(&hex[2..4], 16).unwrap_or(0),
        u8::from_str_radix(&hex[4..6], 16).unwrap_or(0),
    )
}

/// Blend two hex colors: `t=0.0` → a, `t=1.0` → b. Used for the dim_*
/// palette entries (alacritty-style dim = mix toward the background).
fn blend(a: &str, b: &str, t: f32) -> String {
    let (ar, ag, ab) = parse_rgb(a);
    let (br, bg, bb) = parse_rgb(b);
    let mix = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    format!("#{:02x}{:02x}{:02x}", mix(ar, br), mix(ag, bg), mix(ab, bb))
}

/// Build egui_term's theme from a parsed palette. dim_* entries are the
/// base colors blended 50% toward the background (ghostty/noctalia defines
/// no dim colors; this matches alacritty's default dim behavior).
fn build_theme(parsed: &ParsedPalette) -> egui_term::TerminalTheme {
    let c = |i: usize| parsed.colors[i].clone();
    let dim = |i: usize| blend(&parsed.colors[i], &parsed.bg, 0.5);
    let palette = egui_term::ColorPalette {
        foreground: parsed.fg.clone(),
        background: parsed.bg.clone(),
        black: c(0),
        red: c(1),
        green: c(2),
        yellow: c(3),
        blue: c(4),
        magenta: c(5),
        cyan: c(6),
        white: c(7),
        bright_black: c(8),
        bright_red: c(9),
        bright_green: c(10),
        bright_yellow: c(11),
        bright_blue: c(12),
        bright_magenta: c(13),
        bright_cyan: c(14),
        bright_white: c(15),
        bright_foreground: None,
        dim_foreground: blend(&parsed.fg, &parsed.bg, 0.5),
        dim_black: dim(0),
        dim_red: dim(1),
        dim_green: dim(2),
        dim_yellow: dim(3),
        dim_blue: dim(4),
        dim_magenta: dim(5),
        dim_cyan: dim(6),
        dim_white: dim(7),
    };
    egui_term::TerminalTheme::new(Box::new(palette))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ghostty_resolved_output() {
        let text = "\
font-family = Maple Mono NF CN
theme = noctalia
background = #1e1e2e
foreground = #cdd6f4
palette = 0=#45475a
palette = 1=#f38ba8
palette = 2=#a6e3a1
palette = 3=#f9e2af
palette = 4=#89b4fa
palette = 5=#f5c2e7
palette = 6=#94e2d5
palette = 7=#a6adc8
palette = 8=#585b70
palette = 9=#f37799
palette = 10=#89d88b
palette = 11=#ebd391
palette = 12=#74a8fc
palette = 13=#f2aede
palette = 14=#6bd7ca
palette = 15=#bac2de
palette = 16=#000000
palette = 255=#eeeeee
";
        let parsed = parse_palette_lines(text).expect("full palette parses");
        assert_eq!(parsed.fg, "#cdd6f4");
        assert_eq!(parsed.bg, "#1e1e2e");
        assert_eq!(parsed.colors[0], "#45475a");
        assert_eq!(parsed.colors[7], "#a6adc8");
        assert_eq!(parsed.colors[15], "#bac2de");
        // Entries above 15 are ignored (egui_term uses its fixed cube).
        assert_eq!(parsed.colors[5], "#f5c2e7");
    }

    #[test]
    fn malformed_lines_are_skipped() {
        // Bad hex, bad index, wrong format — all skipped without poisoning
        // the valid slots.
        let text = "\
background = #zzzzzz
background = #1e1e2e
foreground = #cdd6f4
palette = 0=#45475a
palette = 1=#f38ba8
palette = 2=#a6e3a1
palette = 3=#f9e2af
palette = 4=#89b4fa
palette = 5=#f5c2e7
palette = 6=#94e2d5
palette = 7=#a6adc8
palette = 8=#585b70
palette = 9=#f37799
palette = 10=#89d88b
palette = 11=#ebd391
palette = 12=#74a8fc
palette = 13=#f2aede
palette = 14=#6bd7ca
palette = 15=#bac2de
palette = x=#000000
palette = 99=#ffffff
palette = 255=#eeeeee
palette =
this is not a palette line
";
        let parsed = parse_palette_lines(text).expect("still complete");
        assert_eq!(parsed.colors[0], "#45475a");
        assert_eq!(parsed.colors[1], "#f38ba8");
        assert_eq!(parsed.colors[6], "#94e2d5");
        assert_eq!(parsed.colors[15], "#bac2de");
    }

    #[test]
    fn incomplete_palette_is_none() {
        let text = "\
background = #1e1e2e
foreground = #cdd6f4
palette = 0=#45475a
";
        assert!(parse_palette_lines(text).is_none());
        assert!(parse_palette_lines("").is_none());
        assert!(parse_palette_lines("background = #1e1e2e").is_none());
    }

    #[test]
    fn blend_mixes_toward_background() {
        assert_eq!(blend("#000000", "#ffffff", 0.5), "#808080");
        assert_eq!(blend("#000000", "#ffffff", 0.0), "#000000");
        assert_eq!(blend("#ff0000", "#ffffff", 1.0), "#ffffff");
    }

    #[test]
    fn theme_builds_from_parsed() {
        let text = "\
background = #1e1e2e
foreground = #cdd6f4
palette = 0=#45475a
palette = 1=#f38ba8
palette = 2=#a6e3a1
palette = 3=#f9e2af
palette = 4=#89b4fa
palette = 5=#f5c2e7
palette = 6=#94e2d5
palette = 7=#a6adc8
palette = 8=#585b70
palette = 9=#f37799
palette = 10=#89d88b
palette = 11=#ebd391
palette = 12=#74a8fc
palette = 13=#f2aede
palette = 14=#6bd7ca
palette = 15=#bac2de
";
        let parsed = parse_palette_lines(text).unwrap();
        let theme = build_theme(&parsed);
        // Not directly inspectable (private fields) — just ensure it builds
        // and the dim entries are valid hex via the public type's use later.
        let _ = theme;
    }
}
