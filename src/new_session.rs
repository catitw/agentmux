//! New-session dialog: a plain egui window with work-dir / command / label
//! fields. Validation lives in pure functions so it is unit-testable without
//! a UI.

use std::path::Path;

/// What the dialog wants the app to do this frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DraftAction {
    Submit,
    Cancel,
}

/// Mutable draft state, held by the app while the dialog is open.
#[derive(Debug, Clone)]
pub struct NewSessionDraft {
    pub work_dir: String,
    pub command: String,
    pub label: String,
    /// Inline validation error, shown in red under the fields.
    pub error: Option<String>,
}

impl NewSessionDraft {
    pub fn new(default_work_dir: String, default_command: String) -> Self {
        Self {
            work_dir: default_work_dir,
            command: default_command,
            label: String::new(),
            error: None,
        }
    }
}

/// Validate the draft: the work directory must exist and be a directory,
/// the command must be non-empty. Returns a human-readable error.
pub fn validate(work_dir: &str, command: &str) -> Result<(), String> {
    let dir = Path::new(work_dir.trim());
    if !dir.is_dir() {
        return Err(format!("not a directory: {}", work_dir.trim()));
    }
    if command.trim().is_empty() {
        return Err("command must not be empty".to_owned());
    }
    Ok(())
}

/// Split a command line into program + args (first whitespace token = the
/// program, the rest = its arguments).
pub fn split_command(command: &str) -> (String, Vec<String>) {
    let mut tokens = command.split_whitespace();
    let program = tokens.next().unwrap_or_default().to_owned();
    let args: Vec<String> = tokens.map(str::to_owned).collect();
    (program, args)
}

/// Derive a display label from the command: shell commands map to "Shell"
/// (matching the default session's label today); anything else uses the
/// program basename (e.g. "omp", "claude").
pub fn derive_label(command: &str) -> String {
    let first = command.split_whitespace().next().unwrap_or_default();
    let base = Path::new(first)
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| first.to_owned());
    match base.as_str() {
        "bash" | "sh" | "dash" | "zsh" | "fish" | "ksh" | "cmd" | "cmd.exe" | "powershell"
        | "pwsh" => "Shell".to_owned(),
        _ => base,
    }
}

/// Render the dialog window. Returns the action requested this frame, if
/// any. Enter = submit, Esc = cancel.
pub fn dialog(ctx: &egui::Context, draft: &mut NewSessionDraft) -> Option<DraftAction> {
    let mut action = None;
    egui::Window::new("New session")
        .collapsible(false)
        .resizable(false)
        .default_width(360.0)
        .show(ctx, |ui| {
            ui.spacing_mut().item_spacing.y = 8.0;
            ui.label("Work directory");
            ui.text_edit_singleline(&mut draft.work_dir);
            ui.add_space(6.0);
            ui.label("Command");
            ui.text_edit_singleline(&mut draft.command);
            ui.add_space(6.0);
            ui.label("Label (optional, empty = derive from command)");
            ui.text_edit_singleline(&mut draft.label);

            if let Some(error) = &draft.error {
                ui.add_space(2.0);
                ui.colored_label(ui.visuals().error_fg_color, error);
            }

            ui.add_space(14.0);
            ui.horizontal(|ui| {
                // Primary action wears the single accent; cancel stays quiet.
                let accent = ui.visuals().selection.bg_fill;
                let create = egui::Button::new(
                    egui::RichText::new("Create").color(ui.visuals().extreme_bg_color),
                )
                .fill(accent)
                .stroke(egui::Stroke::new(1.0, accent));
                if ui.add(create).clicked() {
                    action = Some(DraftAction::Submit);
                }
                if ui.button("Cancel").clicked() {
                    action = Some(DraftAction::Cancel);
                }
            });

            if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                action = Some(DraftAction::Submit);
            }
            if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                action = Some(DraftAction::Cancel);
            }
        });
    action
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_accepts_existing_dir_and_command() {
        // "/" always exists and is a directory.
        assert!(validate("/", "bash").is_ok());
        assert!(validate(" / ", " omp ").is_ok(), "trims inputs");
    }

    #[test]
    fn validate_rejects_bad_dir() {
        assert!(validate("/definitely/not/a/dir-xyz", "bash").is_err());
        // A regular file is not a directory.
        let file = std::env::temp_dir().join(format!("agentmux-val-{}", std::process::id()));
        std::fs::write(&file, "x").unwrap();
        assert!(validate(file.to_str().unwrap(), "bash").is_err());
        let _ = std::fs::remove_file(&file);
    }

    #[test]
    fn validate_rejects_empty_command() {
        assert!(validate("/", "").is_err());
        assert!(validate("/", "   ").is_err());
    }

    #[test]
    fn label_derivation() {
        assert_eq!(derive_label("bash"), "Shell");
        assert_eq!(derive_label("/usr/bin/fish"), "Shell");
        assert_eq!(derive_label("cmd.exe"), "Shell");
        assert_eq!(derive_label("omp"), "omp");
        assert_eq!(derive_label("/opt/claude-code/bin/claude"), "claude");
        assert_eq!(derive_label("bash -c 'echo hi'"), "Shell");
        assert_eq!(derive_label(""), "");
    }

    #[test]
    fn command_splitting() {
        assert_eq!(split_command("omp"), ("omp".to_owned(), vec![]));
        assert_eq!(
            split_command("bash -c 'echo hi'"),
            ("bash".to_owned(), vec!["-c".to_owned(), "'echo".to_owned(), "hi'".to_owned()])
        );
        assert_eq!(
            split_command("  claude  --resume foo "),
            ("claude".to_owned(), vec!["--resume".to_owned(), "foo".to_owned()])
        );
    }
}
