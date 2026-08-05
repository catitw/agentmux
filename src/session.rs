//! Session model: one unit of work = a work directory + a hermes tool name +
//! the process-layer status of the terminal running it.

use std::path::{Path, PathBuf};

/// A session is one unit of work: a work directory, a hermes tool, and the
/// embedded terminal running it.
pub struct Session {
    pub id: u64,
    pub work_dir: PathBuf,
    /// Display label, e.g. "Claude Code", "omp", "Shell".
    pub tool_name: String,
    /// Program to spawn in the terminal, e.g. "bash", "claude", "omp".
    pub command: String,
    /// User-assigned name (right-click → Rename). Takes display precedence
    /// over the tool/agent labels everywhere.
    pub custom_name: Option<String>,
    pub status: SessionStatus,
}

/// Process-layer status of a session's terminal child.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionStatus {
    /// Terminal child is alive (default right after spawn).
    Running,
    /// Child exited with status 0.
    Done,
    /// Child exited non-zero, or the terminal failed to spawn.
    Error,
}

/// Derive a display label from the command: shell commands map to "Shell"
/// (matching the default session's label); anything else uses the program
/// basename (e.g. "omp", "claude").
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
