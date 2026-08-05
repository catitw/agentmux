//! Session persistence (metadata only — processes do not survive app exit;
//! there is no daemon, restore respawns fresh shells in the saved
//! directories). See docs/phase4-persistence.md.

use serde::{Deserialize, Serialize};
use std::io;
use std::path::{Path, PathBuf};

pub const SESSIONS_FILE_NAME: &str = "sessions.json";
pub const SCHEMA_VERSION: u32 = 1;

/// One persisted session (metadata only, no PID/status).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionMeta {
    pub work_dir: String,
    pub command: String,
    pub label: String,
    /// User-assigned name; absent in v1 files written before the rename
    /// feature (schema version stays 1 — the field is optional).
    #[serde(default)]
    pub custom_name: Option<String>,
}

/// On-disk file layout.
#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionsFile {
    pub version: u32,
    /// Array order = sidebar order.
    pub sessions: Vec<SessionMeta>,
}

/// The sessions file location: `config_dir()/agentmux/sessions.json`
/// (same `dirs::config_dir()` base as the hook port file; honors
/// `XDG_CONFIG_HOME` on Linux).
pub fn sessions_path() -> io::Result<PathBuf> {
    let base = dirs::config_dir()
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no config directory"))?;
    Ok(base.join("agentmux").join(SESSIONS_FILE_NAME))
}

/// Atomic save: write `<path>.tmp` then rename over the target. Creates the
/// parent directory if needed.
pub fn save(path: &Path, sessions: &[SessionMeta]) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let payload = SessionsFile {
        version: SCHEMA_VERSION,
        sessions: sessions.to_vec(),
    };
    let json = serde_json::to_string_pretty(&payload).map_err(io::Error::other)?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, json)?;
    std::fs::rename(&tmp, path)
}

/// Why loading failed: no file yet (fall back to seeding a default session)
/// vs. a file that exists but cannot be used (also fall back, but warn).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoadError {
    NotFound,
    Malformed(String),
}

/// Load and validate the sessions file. `Malformed` covers unparsable JSON
/// and unsupported schema versions — the caller must not crash on it.
pub fn load(path: &Path) -> Result<Vec<SessionMeta>, LoadError> {
    let content = match std::fs::read_to_string(path) {
        Ok(content) => content,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Err(LoadError::NotFound),
        Err(err) => return Err(LoadError::Malformed(err.to_string())),
    };
    let parsed: SessionsFile =
        serde_json::from_str(&content).map_err(|err| LoadError::Malformed(err.to_string()))?;
    if parsed.version != SCHEMA_VERSION {
        return Err(LoadError::Malformed(format!(
            "unsupported schema version {} (expected {SCHEMA_VERSION})",
            parsed.version
        )));
    }
    Ok(parsed.sessions)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static PATH_COUNTER: AtomicU64 = AtomicU64::new(0);

    fn unique_path() -> PathBuf {
        let n = PATH_COUNTER.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!("agentmux-sessions-test-{}-{n}.json", std::process::id()))
    }

    fn sample() -> Vec<SessionMeta> {
        vec![
            SessionMeta {
                work_dir: "/home/user/proj-a".into(),
                command: "omp".into(),
                label: "omp".into(),
                custom_name: Some("my agent".into()),
            },
            SessionMeta {
                work_dir: "/home/user".into(),
                command: "/bin/bash".into(),
                label: "Shell".into(),
                custom_name: None,
            },
        ]
    }

    #[test]
    fn save_load_roundtrip() {
        let path = unique_path();
        save(&path, &sample()).expect("save");
        let loaded = load(&path).expect("load");
        assert_eq!(loaded, sample());
        // Array order preserved (sidebar order).
        assert_eq!(loaded[0].label, "omp");
        assert_eq!(loaded[1].label, "Shell");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn missing_file_is_not_found() {
        assert_eq!(load(&unique_path()), Err(LoadError::NotFound));
    }

    #[test]
    fn malformed_json_is_an_error_not_a_crash() {
        let path = unique_path();
        std::fs::write(&path, "{ not json !!").unwrap();
        match load(&path) {
            Err(LoadError::Malformed(_)) => {}
            other => panic!("expected Malformed, got {other:?}"),
        }
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn unsupported_version_is_an_error() {
        let path = unique_path();
        std::fs::write(&path, r#"{"version": 99, "sessions": []}"#).unwrap();
        match load(&path) {
            Err(LoadError::Malformed(msg)) => assert!(msg.contains("version"), "{msg}"),
            other => panic!("expected Malformed, got {other:?}"),
        }
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn save_creates_parent_dirs_atomically() {
        let dir = std::env::temp_dir().join(format!("agentmux-persist-{}", std::process::id()));
        let path = dir.join("nested").join(SESSIONS_FILE_NAME);
        save(&path, &sample()).expect("save");
        assert!(path.is_file());
        // No leftover tmp file.
        assert!(!path.with_extension("json.tmp").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
mod rename_tests {
    use super::*;

    #[test]
    fn v1_file_without_custom_name_loads() {
        // Files written before the rename feature have no custom_name field;
        // schema version stays 1 and the field is optional.
        let path = std::env::temp_dir().join(format!(
            "agentmux-v1-{}.json",
            std::process::id()
        ));
        std::fs::write(
            &path,
            r#"{"version": 1, "sessions": [{"work_dir": "/home/user", "command": "bash", "label": "Shell"}]}"#,
        )
        .unwrap();
        let metas = load(&path).expect("v1 file loads");
        assert_eq!(metas.len(), 1);
        assert_eq!(metas[0].custom_name, None);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn custom_name_roundtrips() {
        let path = std::env::temp_dir().join(format!(
            "agentmux-name-{}.json",
            std::process::id()
        ));
        let metas = vec![SessionMeta {
            work_dir: "/home/user".into(),
            command: "bash".into(),
            label: "Shell".into(),
            custom_name: Some("agentmux dev".into()),
        }];
        save(&path, &metas).unwrap();
        assert_eq!(load(&path).unwrap(), metas);
        let _ = std::fs::remove_file(&path);
    }
}
