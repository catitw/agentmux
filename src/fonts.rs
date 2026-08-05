//! System-font discovery and egui font-fallback registration.
//!
//! egui's embedded fonts (Ubuntu-Light / Hack / emoji) lack CJK, Nerd Font
//! icon, and braille glyphs, which rendered as tofu (□). At startup we
//! discover installed fonts by family preference and append them as
//! fallbacks:
//!
//! - UI families (`Proportional` / `Monospace`) get the found icon, CJK and
//!   emoji faces appended — fixes the tab "●" marker, sidebar, toasts, and
//!   any CJK in tooltips.
//! - A custom `agentmux-terminal` family starts as a copy of egui's
//!   `Monospace` chain (Hack first, so terminal cell metrics stay driven by
//!   the primary monospace font) and appends the same fallbacks, so Nerd
//!   icons, braille spinners and CJK render inside the embedded terminal.
//!   The terminal view is pointed at it via `TerminalView::set_font`
//!   (egui_term's `TerminalFont` is just an egui `FontId`; egui does
//!   per-glyph fallback within the family, view.rs:337).
//!
//! No fonts are bundled: on systems without Nerd/CJK fonts the fallback
//! lists stay empty and the app behaves exactly as before (see
//! docs/fonts.md).

use egui::{FontData, FontDefinitions, FontFamily, FontId};

/// Custom font family used by the embedded terminals.
pub const TERMINAL_FAMILY: &str = "agentmux-terminal";

/// Bundled Nerd Font SymbolsOnly (Mono variant), nerd-fonts release v3.5.0,
/// sha256 2dc316f2505a0cbfbcf6060a1b4ba85b0a2974189e30c0037cdedc436a25a4ff.
/// Guarantees Nerd Font icon coverage on every machine (system fonts are
/// preferred; this is the floor). License: SIL OFL 1.1 (individual icon
/// sets retain their own licenses) — see docs/fonts.md.
const BUNDLED_SYMBOLS: &[u8] = include_bytes!("../assets/fonts/SymbolsNerdFontMono-Regular.ttf");

/// Result of the startup font setup.
pub struct FontSetup {
    /// Terminal font (primary = preferred terminal font or egui's monospace
    /// font) for `TerminalView::set_font`.
    pub terminal_font: egui_term::TerminalFont,
    /// Name of the font driving the terminal (preference match or
    /// "Hack (egui default)"), for the startup log.
    pub terminal_font_name: String,
    /// Human-readable list of registered fallback faces, for the startup
    /// log ("none found" when empty).
    pub registered: Vec<String>,
}

/// Discover system fonts by preference and register them into egui.
pub fn setup_fonts(ctx: &egui::Context) -> FontSetup {
    let mut db = fontdb::Database::new();
    db.load_system_fonts();

    let mut definitions = FontDefinitions::default();

    // Terminal family: starts with the preferred terminal font (if found —
    // it drives cell metrics via egui_term's font_measure), then egui's
    // Monospace chain (Hack), then the fallbacks below.
    let mut terminal_chain = definitions
        .families
        .get(&FontFamily::Monospace)
        .cloned()
        .unwrap_or_default();
    let mut ui_fallbacks = Vec::new();
    let mut registered = Vec::new();
    let mut terminal_font_name = "Hack (egui default)".to_owned();

    // Preferred terminal font: AGENTMUX_TERMINAL_FONT env override first
    // (exact family name), then the preference list (the user's ghostty
    // uses Maple Mono NF CN), else keep Hack. Registered into the terminal
    // family ONLY — UI fonts stay unchanged.
    if let Some((id, name)) = find_terminal_font(&db) {
        register_face(
            &db,
            id,
            &mut definitions,
            &mut terminal_chain,
            None,
            &mut registered,
            /* insert_first */ true,
        );
        terminal_font_name = name;
    }

    // Preference order: icons (system Nerd Font, then the bundled symbols
    // floor), then CJK, then emoji. Each face is registered once and added
    // to all three chains.
    if let Some(id) = find_icon_face(&db) {
        register_face(
            &db, id, &mut definitions, &mut terminal_chain, Some(&mut ui_fallbacks), &mut registered, false,
        );
    }
    // Bundled symbols: only a fallback for glyphs the system fonts lack,
    // so it never shadows the system Nerd Font.
    register_bytes(
        &mut definitions,
        &mut terminal_chain,
        Some(&mut ui_fallbacks),
        &mut registered,
        "bundled SymbolsNerdFontMono (v3.5.0)",
        BUNDLED_SYMBOLS.into(),
        0,
        false,
    );
    if let Some(id) = query_face(
        &db,
        &[
            "Noto Sans CJK SC",
            "Noto Sans CJK TC",
            "Noto Sans CJK JP",
            "Noto Sans CJK",
            "WenQuanYi Micro Hei",
            "WenQuanYi Zen Hei",
            "Microsoft YaHei",
            "PingFang SC",
            "Source Han Sans SC",
        ],
    ) {
        register_face(
            &db, id, &mut definitions, &mut terminal_chain, Some(&mut ui_fallbacks), &mut registered, false,
        );
    }
    if let Some(id) = query_face(&db, &["Noto Color Emoji"]) {
        register_face(
            &db, id, &mut definitions, &mut terminal_chain, Some(&mut ui_fallbacks), &mut registered, false,
        );
    }

    // Wire the fallbacks in: custom terminal family + UI family appends.
    definitions
        .families
        .insert(FontFamily::Name(TERMINAL_FAMILY.into()), terminal_chain);
    for face in &ui_fallbacks {
        definitions
            .families
            .entry(FontFamily::Proportional)
            .or_default()
            .push(face.clone());
        definitions
            .families
            .entry(FontFamily::Monospace)
            .or_default()
            .push(face.clone());
    }

    ctx.set_fonts(definitions);

    FontSetup {
        terminal_font: egui_term::TerminalFont::new(egui_term::FontSettings {
            font_type: FontId::new(14.0, FontFamily::Name(TERMINAL_FAMILY.into())),
        }),
        terminal_font_name,
        registered,
    }
}

/// First face matching any of the exact family names.
fn query_face(db: &fontdb::Database, names: &[&str]) -> Option<fontdb::ID> {
    names.iter().find_map(|name| {
        db.query(&fontdb::Query {
            families: &[fontdb::Family::Name(name)],
            ..Default::default()
        })
    })
}

/// Nerd Font icon face: exact preference names first ("Symbols Nerd Font
/// Mono" and friends), then any installed family whose name contains
/// "Nerd Font Mono", then any containing "Nerd Font".
fn find_icon_face(db: &fontdb::Database) -> Option<fontdb::ID> {
    if let Some(id) = query_face(db, &["Symbols Nerd Font Mono", "Symbols Nerd Font"]) {
        return Some(id);
    }
    let mut families: Vec<&str> = db
        .faces()
        .filter_map(|face| face.families.first().map(|(name, _)| name.as_str()))
        .collect();
    families.sort_unstable();
    families.dedup();
    let by_contains = |needle: &str| {
        families.iter().find(|name| name.contains(needle)).and_then(|name| {
            query_face(db, &[name])
        })
    };
    by_contains("Nerd Font Mono").or_else(|| by_contains("Nerd Font"))
}

/// Load a face's bytes (with its TTC face index) and add it to the chains.
/// `ui_fallbacks` is `None` for fonts that belong in the terminal only.
/// `insert_first` puts the face at the FRONT of the terminal chain so it
/// drives cell metrics (used for the preferred terminal font).
#[allow(clippy::too_many_arguments)] // one registration point, six roles
fn register_face(
    db: &fontdb::Database,
    id: fontdb::ID,
    definitions: &mut FontDefinitions,
    terminal_chain: &mut Vec<String>,
    ui_fallbacks: Option<&mut Vec<String>>,
    registered: &mut Vec<String>,
    insert_first: bool,
) {
    let (source, face_index) = match db.face_source(id) {
        Some(found) => found,
        None => return,
    };
    let bytes: std::borrow::Cow<'static, [u8]> = match source {
        fontdb::Source::File(path) | fontdb::Source::SharedFile(path, _) => {
            match std::fs::read(path) {
                Ok(bytes) => bytes.into(),
                Err(_) => return,
            }
        }
        fontdb::Source::Binary(data) => data.as_ref().as_ref().to_vec().into(),
    };
    let label = db
        .face(id)
        .map(|face| face.post_script_name.clone())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| format!("agentmux-face-{}", registered.len()));

    register_bytes(definitions, terminal_chain, ui_fallbacks, registered, &label, bytes, face_index, insert_first);
}

/// Add raw font bytes to the chains (also used for the bundled symbols
/// font). `ui_fallbacks` is `None` for terminal-only fonts.
#[allow(clippy::too_many_arguments)] // one registration point, six roles
fn register_bytes(
    definitions: &mut FontDefinitions,
    terminal_chain: &mut Vec<String>,
    ui_fallbacks: Option<&mut Vec<String>>,
    registered: &mut Vec<String>,
    label: &str,
    bytes: std::borrow::Cow<'static, [u8]>,
    face_index: u32,
    insert_first: bool,
) {
    let mut key = label.to_owned();
    let mut suffix = 1;
    while definitions.font_data.contains_key(&key) {
        key = format!("{label}-{suffix}");
        suffix += 1;
    }
    definitions.font_data.insert(
        key.clone(),
        std::sync::Arc::new(FontData {
            font: bytes,
            index: face_index,
            tweak: Default::default(),
        }),
    );
    if insert_first {
        terminal_chain.insert(0, key.clone());
    } else {
        terminal_chain.push(key.clone());
    }
    if let Some(ui_fallbacks) = ui_fallbacks {
        ui_fallbacks.push(key);
    }
    registered.push(label.to_owned());
}

/// Preferred terminal font families, first match wins. The env override
/// `AGENTMUX_TERMINAL_FONT` (exact family name) comes before this list.
const TERMINAL_FONT_PREFERENCES: &[&str] = &[
    "Maple Mono NF CN",
    "Maple Mono NF",
    "JetBrainsMono Nerd Font Mono",
    "JetBrainsMono Nerd Font",
    "CaskaydiaCove Nerd Font Mono",
    "FiraCode Nerd Font Mono",
    "FantasqueSansMono Nerd Font Mono",
    "MesloLGS Nerd Font Mono",
    "Hack Nerd Font Mono",
];

/// Find the preferred terminal font face: `AGENTMUX_TERMINAL_FONT` env first
/// (exact family name), then the preference list. Returns the face id and
/// the matched family name.
fn find_terminal_font(db: &fontdb::Database) -> Option<(fontdb::ID, String)> {
    let mut preferences: Vec<String> = Vec::new();
    if let Some(override_name) = std::env::var_os("AGENTMUX_TERMINAL_FONT") {
        preferences.push(override_name.to_string_lossy().into_owned());
    }
    preferences.extend(TERMINAL_FONT_PREFERENCES.iter().map(|s| s.to_string()));

    for name in preferences {
        if let Some(id) = query_face(db, &[&name]) {
            return Some((id, name));
        }
    }
    None
}
