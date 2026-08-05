//! Hook installer / uninstaller (`agentmux --install-hooks` /
//! `--uninstall-hooks`). These CLI paths run without starting the GUI.
//!
//! - claude: drops the hook script into `~/.claude/hooks` and merges hook
//!   entries into `~/.claude/settings.json` (non-destructive: existing
//!   entries preserved, timestamped backup before any modification,
//!   idempotent). `CLAUDE_CONFIG_DIR` overrides the directory.
//! - omp: drops the extension into omp's extensions dir, which omp
//!   auto-loads (`~/.omp/agent/extensions`, overridable via
//!   `PI_CODING_AGENT_DIR` / `PI_CONFIG_DIR`).

use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Map, Value};

const CLAUDE_HOOK_NAME: &str = "agentmux-claude-hook.sh";
const CLAUDE_HOOK_ASSET: &str = include_str!("../../assets/hooks/agentmux-claude-hook.sh");
const OMP_EXTENSION_NAME: &str = "agentmux-omp-extension.ts";
const OMP_EXTENSION_ASSET: &str = include_str!("../../assets/hooks/agentmux-omp-extension.ts");

/// Claude events mapped to hook actions (matcher "*").
const CLAUDE_EVENTS: &[(&str, &str)] = &[
    ("UserPromptSubmit", "working"),
    ("PreToolUse", "working"),
    ("PostToolUse", "working"),
    ("Notification", "blocked"),
    ("Stop", "idle"),
    ("SessionEnd", "release"),
];

pub fn install() -> io::Result<()> {
    install_claude()?;
    install_omp()?;
    Ok(())
}

pub fn uninstall() -> io::Result<()> {
    uninstall_claude()?;
    uninstall_omp()?;
    Ok(())
}

fn home_dir() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

fn claude_dir() -> PathBuf {
    std::env::var_os("CLAUDE_CONFIG_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| home_dir().join(".claude"))
}

/// omp's extension dir: `$PI_CODING_AGENT_DIR/extensions`, else
/// `$HOME/$PI_CONFIG_DIR/agent/extensions` (default `~/.omp/agent/extensions`).
fn omp_extension_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("PI_CODING_AGENT_DIR").filter(|v| !v.is_empty()) {
        return PathBuf::from(dir).join("extensions");
    }
    let base = std::env::var_os("PI_CONFIG_DIR")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(".omp"));
    home_dir().join(base).join("agent").join("extensions")
}

fn install_claude() -> io::Result<()> {
    let dir = claude_dir();
    let hooks_dir = dir.join("hooks");
    std::fs::create_dir_all(&hooks_dir)?;
    let hook_path = hooks_dir.join(CLAUDE_HOOK_NAME);
    std::fs::write(&hook_path, CLAUDE_HOOK_ASSET)?;
    make_executable(&hook_path)?;
    println!("installed claude hook -> {}", hook_path.display());

    let settings_path = dir.join("settings.json");
    let existing = match std::fs::read_to_string(&settings_path) {
        Ok(content) => content,
        Err(err) if err.kind() == io::ErrorKind::NotFound => String::new(),
        Err(err) => return Err(err),
    };
    let result = merge_settings_content(&existing, &hook_path, MergeAction::Install)?;
    if result.malformed {
        println!(
            "warning: {} was not valid JSON; the original is preserved in the backup",
            settings_path.display()
        );
    }
    if result.changed {
        if !existing.is_empty() {
            let backup = backup_path(&settings_path);
            std::fs::write(&backup, &existing)?;
            println!("backup written -> {}", backup.display());
        }
        std::fs::write(&settings_path, result.content)?;
        println!("updated {}", settings_path.display());
    } else {
        println!("{} already up to date (idempotent no-op)", settings_path.display());
    }
    Ok(())
}

fn uninstall_claude() -> io::Result<()> {
    let dir = claude_dir();
    let hook_path = dir.join("hooks").join(CLAUDE_HOOK_NAME);
    let settings_path = dir.join("settings.json");
    match std::fs::read_to_string(&settings_path) {
        Ok(existing) => {
            let result = merge_settings_content(&existing, &hook_path, MergeAction::Uninstall)?;
            if result.changed {
                std::fs::write(&settings_path, result.content)?;
                println!("updated {}", settings_path.display());
            } else {
                println!("{} had no agentmux entries to remove", settings_path.display());
            }
        }
        Err(err) if err.kind() == io::ErrorKind::NotFound => {
            println!("{} does not exist — nothing to do", settings_path.display());
        }
        Err(err) => return Err(err),
    }

    if hook_path.exists() {
        std::fs::remove_file(&hook_path)?;
        println!("removed claude hook -> {}", hook_path.display());
    }
    Ok(())
}

fn install_omp() -> io::Result<()> {
    let dir = omp_extension_dir();
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(OMP_EXTENSION_NAME);
    std::fs::write(&path, OMP_EXTENSION_ASSET)?;
    println!("installed omp extension -> {}", path.display());
    Ok(())
}

fn uninstall_omp() -> io::Result<()> {
    let path = omp_extension_dir().join(OMP_EXTENSION_NAME);
    if path.exists() {
        std::fs::remove_file(&path)?;
        println!("removed omp extension -> {}", path.display());
    } else {
        println!("omp extension not installed ({})", path.display());
    }
    Ok(())
}

/// What a merge pass did, for the CLI output and tests.
pub struct MergeOutcome {
    pub content: String,
    pub changed: bool,
    pub malformed: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum MergeAction {
    Install,
    Uninstall,
}

/// Merge agentmux's hook entries into an existing claude settings.json
/// string. Pure (no file I/O) so it is directly unit-testable.
///
/// Install: appends our `{matcher: "*", hooks: [{type: "command", command:
/// "<hook> <action>", timeout: 10}]}` entry to each event's array, keeping
/// every existing entry. Idempotent: an entry whose command matches ours for
/// that action is not added twice. Malformed input falls back to a minimal
/// object (the caller preserves the original in the backup).
///
/// Uninstall: removes only entries whose command starts with our hook path,
/// drops empty arrays, and drops the `hooks` object when empty.
pub fn merge_settings_content(
    existing: &str,
    hook_path: &Path,
    action: MergeAction,
) -> io::Result<MergeOutcome> {
    let trimmed = existing.trim();
    let (mut root, malformed) = if trimmed.is_empty() {
        (Value::Object(Map::new()), false)
    } else {
        match serde_json::from_str(trimmed) {
            Ok(value) => (value, false),
            Err(_) => (Value::Object(Map::new()), true),
        }
    };

    let mut changed = false;
    match action {
        MergeAction::Install => {
            for &(event, action) in CLAUDE_EVENTS {
                changed |= ensure_event_entry(&mut root, event, hook_path, action)?;
            }
        }
        MergeAction::Uninstall => {
            if let Some(hooks) = root.get_mut("hooks").and_then(Value::as_object_mut) {
                for &(event, action) in CLAUDE_EVENTS {
                    changed |= remove_event_entry(hooks, event, hook_path, action);
                }
                if hooks.is_empty() {
                    root.as_object_mut().expect("checked").remove("hooks");
                }
            }
        }
    }

    let content = if changed || malformed {
        let mut serialized = serde_json::to_string_pretty(&root)
            .map_err(|err| io::Error::other(format!("serialize settings: {err}")))?;
        serialized.push('\n');
        serialized
    } else {
        existing.to_owned()
    };

    Ok(MergeOutcome {
        content,
        changed: changed || malformed,
        malformed,
    })
}

/// The canonical command string for one event/action entry.
fn hook_command(hook_path: &Path, action: &str) -> String {
    format!("{} {}", hook_path.display(), action)
}

/// Append our entry for `(event, action)` if it is not already present.
fn ensure_event_entry(
    root: &mut Value,
    event: &str,
    hook_path: &Path,
    action: &str,
) -> io::Result<bool> {
    let hooks = root
        .as_object_mut()
        .ok_or_else(|| io::Error::other("settings root must be an object"))?
        .entry("hooks")
        .or_insert_with(|| Value::Object(Map::new()));
    let hooks = hooks
        .as_object_mut()
        .ok_or_else(|| io::Error::other("\"hooks\" must be an object"))?;
    let entries = hooks
        .entry(event)
        .or_insert_with(|| Value::Array(Vec::new()));
    let entries = entries
        .as_array_mut()
        .ok_or_else(|| io::Error::other(format!("hook entries for {event} must be an array")))?;

    let command = hook_command(hook_path, action);
    let already = entries.iter().any(|entry| {
        entry
            .get("hooks")
            .and_then(Value::as_array)
            .is_some_and(|hooks| {
                hooks.iter().any(|hook| {
                    hook.get("type").and_then(Value::as_str) == Some("command")
                        && hook.get("command").and_then(Value::as_str) == Some(command.as_str())
                })
            })
    });
    if already {
        return Ok(false);
    }

    entries.push(Value::Object(Map::from_iter([
        ("matcher".to_owned(), Value::String("*".to_owned())),
        (
            "hooks".to_owned(),
            Value::Array(vec![Value::Object(Map::from_iter([
                ("type".to_owned(), Value::String("command".to_owned())),
                ("command".to_owned(), Value::String(command)),
                ("timeout".to_owned(), Value::Number(10.into())),
            ]))]),
        ),
    ])));
    Ok(true)
}

/// Remove our entry for `(event, action)`; drop the event when empty.
fn remove_event_entry(
    hooks: &mut Map<String, Value>,
    event: &str,
    hook_path: &Path,
    action: &str,
) -> bool {
    let Some(entries) = hooks.get_mut(event).and_then(Value::as_array_mut) else {
        return false;
    };
    let command = hook_command(hook_path, action);
    let before = entries.len();
    entries.retain(|entry| {
        !entry
            .get("hooks")
            .and_then(Value::as_array)
            .is_some_and(|hooks| {
                hooks.iter().any(|hook| {
                    hook.get("type").and_then(Value::as_str) == Some("command")
                        && hook.get("command").and_then(Value::as_str) == Some(command.as_str())
                })
            })
    });
    let removed = entries.len() != before;
    if entries.is_empty() {
        hooks.remove(event);
    }
    removed
}

fn backup_path(settings_path: &Path) -> PathBuf {
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let name = settings_path.file_name().map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "settings.json".to_owned());
    settings_path.with_file_name(format!("{name}.agentmux-bak-{ts}"))
}

#[cfg(unix)]
fn make_executable(path: &Path) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mut permissions = std::fs::metadata(path)?.permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(path, permissions)
}

#[cfg(not(unix))]
fn make_executable(_path: &Path) -> io::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOOK: &str = "/home/user/.claude/hooks/agentmux-claude-hook.sh";
    const FOREIGN_ENTRY: &str = r#"{"matcher":"*","hooks":[{"type":"command","command":"echo keep","timeout":5}]}"#;

    fn merge_install(existing: &str) -> MergeOutcome {
        merge_settings_content(existing, Path::new(HOOK), MergeAction::Install).unwrap()
    }

    #[test]
    fn install_creates_minimal_settings() {
        let outcome = merge_install("");
        assert!(outcome.changed);
        assert!(!outcome.malformed);
        let root: Value = serde_json::from_str(&outcome.content).unwrap();
        let hooks = root["hooks"].as_object().unwrap();
        for &(event, action) in CLAUDE_EVENTS {
            let entries = hooks[event].as_array().expect("event array");
            assert_eq!(entries.len(), 1, "{event} should have exactly our entry");
            let command = entries[0]["hooks"][0]["command"].as_str().unwrap();
            assert_eq!(command, format!("{HOOK} {action}"));
            assert_eq!(entries[0]["matcher"], "*");
        }
    }

    #[test]
    fn install_preserves_existing_hooks() {
        let existing = format!(
            r#"{{"hooks":{{"UserPromptSubmit":[{FOREIGN_ENTRY}],"Stop":[{FOREIGN_ENTRY}]}},"alpha":1}}"#
        );
        let outcome = merge_install(&existing);
        let root: Value = serde_json::from_str(&outcome.content).unwrap();
        // Foreign entries still there, ours appended.
        assert_eq!(root["hooks"]["UserPromptSubmit"].as_array().unwrap().len(), 2);
        assert_eq!(root["hooks"]["Stop"].as_array().unwrap().len(), 2);
        assert_eq!(root["alpha"], 1);
        let ours = &root["hooks"]["Stop"].as_array().unwrap()[1];
        assert!(ours["hooks"][0]["command"].as_str().unwrap().starts_with(HOOK));
    }

    #[test]
    fn install_is_idempotent() {
        let once = merge_install("").content;
        let twice = merge_install(&once);
        assert!(!twice.changed, "second install must be a no-op");
        assert_eq!(once, twice.content);
    }

    #[test]
    fn malformed_input_falls_back_to_minimal() {
        let outcome = merge_install("{ this is not json !");
        assert!(outcome.malformed);
        assert!(outcome.changed);
        let root: Value = serde_json::from_str(&outcome.content).unwrap();
        assert!(root["hooks"].is_object());
    }

    #[test]
    fn uninstall_removes_only_our_entries() {
        let installed = merge_install("").content;
        // Add a foreign entry to one event, then uninstall.
        let with_foreign = {
            let mut root: Value = serde_json::from_str(&installed).unwrap();
            root["hooks"]["Stop"].as_array_mut().unwrap().push(
                serde_json::from_str(FOREIGN_ENTRY).unwrap(),
            );
            serde_json::to_string_pretty(&root).unwrap()
        };
        let outcome =
            merge_settings_content(&with_foreign, Path::new(HOOK), MergeAction::Uninstall).unwrap();
        assert!(outcome.changed);
        let root: Value = serde_json::from_str(&outcome.content).unwrap();
        // Foreign entry preserved.
        assert_eq!(root["hooks"]["Stop"].as_array().unwrap().len(), 1);
        assert_eq!(
            root["hooks"]["Stop"][0]["hooks"][0]["command"].as_str(),
            Some("echo keep")
        );
        // Ours removed everywhere; events we emptied are gone.
        assert!(root["hooks"].get("UserPromptSubmit").is_none());
        // Second uninstall is a no-op.
        let again =
            merge_settings_content(&outcome.content, Path::new(HOOK), MergeAction::Uninstall).unwrap();
        assert!(!again.changed);
    }
}
