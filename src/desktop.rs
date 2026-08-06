//! freedesktop desktop launcher entry (Linux/BSD): `--install-desktop-entry`
//! / `--uninstall-desktop-entry` so agentmux appears in the application menu
//! like a normal GUI app. On other platforms (macOS/Windows) the commands
//! print "not applicable" and exit 0 — start-menu/bundle integration there
//! is future work.

use std::path::{Path, PathBuf};

const ICON_ASSET: &str = include_str!("../assets/icon.svg");
const ICON_NAME: &str = "agentmux";

/// The two files the installer owns (desktop entry + themed icon).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DesktopPaths {
    pub desktop_file: PathBuf,
    pub icon_file: PathBuf,
}

/// Compute the install paths under a data dir (pure, for tests).
pub fn paths_for(data_dir: &Path) -> DesktopPaths {
    DesktopPaths {
        desktop_file: data_dir.join("applications").join("agentmux.desktop"),
        icon_file: data_dir
            .join("icons")
            .join("hicolor")
            .join("scalable")
            .join("apps")
            .join("agentmux.svg"),
    }
}

/// The applications dir: `$XDG_DATA_HOME/applications`, falling back to
/// `~/.local/share/applications` (dirs::data_dir does exactly this).
fn applications_dir() -> Option<PathBuf> {
    dirs::data_dir().map(|dir| dir.join("applications"))
}

/// The themed icon dir: `data_dir/icons/hicolor/scalable/apps`.
fn icons_dir() -> Option<PathBuf> {
    dirs::data_dir().map(|dir| dir.join("icons").join("hicolor"))
}

/// The desktop-entry content with the real binary path injected. Pure for
/// tests.
pub fn desktop_entry_content(exec_path: &str) -> String {
    format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Name=agentmux\n\
         Comment=Native GUI for running multiple hermes coding agents in tabs\n\
         Exec={exec_path}\n\
         Icon={ICON_NAME}\n\
         Terminal=false\n\
         Categories=Development;TerminalEmulator;Utility;\n\
         Keywords=terminal;agent;claude;omp;\n\
         StartupNotify=true\n"
    )
}

/// Install the desktop entry + icon. Runs on Linux/BSD only.
#[cfg(any(
    target_os = "linux",
    target_os = "freebsd",
    target_os = "netbsd",
    target_os = "openbsd",
    target_os = "dragonfly"
))]
pub fn install_desktop_entry() -> std::io::Result<()> {
    // The running binary's real path: resolves symlinks, so a launcher
    // entry made from a cargo-installed binary points at ~/.cargo/bin.
    let exec = std::env::current_exe()?;
    let exec = exec.canonicalize().unwrap_or(exec);
    let exec_str = exec.display().to_string();

    let desktop_dir = applications_dir()
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "no data directory"))?;
    std::fs::create_dir_all(&desktop_dir)?;
    let desktop_file = desktop_dir.join("agentmux.desktop");
    std::fs::write(&desktop_file, desktop_entry_content(&exec_str))?;
    println!("installed desktop entry -> {}", desktop_file.display());

    let icon_dir = icons_dir()
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "no data directory"))?;
    std::fs::create_dir_all(icon_dir.join("scalable").join("apps"))?;
    let icon_file = icon_dir.join("scalable").join("apps").join("agentmux.svg");
    std::fs::write(&icon_file, ICON_ASSET)?;
    println!("installed icon -> {}", icon_file.display());

    // Refresh caches when the tools exist; failure is fine (ignored).
    let _ = std::process::Command::new("update-desktop-database")
        .arg(&desktop_dir)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
    let _ = std::process::Command::new("gtk-update-icon-cache")
        .args(["-f", "-t"])
        .arg(&icon_dir)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();

    Ok(())
}

/// Remove exactly the two files the installer wrote.
#[cfg(any(
    target_os = "linux",
    target_os = "freebsd",
    target_os = "netbsd",
    target_os = "openbsd",
    target_os = "dragonfly"
))]
pub fn uninstall_desktop_entry() -> std::io::Result<()> {
    let Some(data_dir) = dirs::data_dir() else {
        println!("no data directory found — nothing to uninstall");
        return Ok(());
    };
    let paths = paths_for(&data_dir);
    for path in [paths.desktop_file, paths.icon_file] {
        if path.exists() {
            std::fs::remove_file(&path)?;
            println!("removed -> {}", path.display());
        } else {
            println!("not installed (skipped) -> {}", path.display());
        }
    }
    Ok(())
}

#[cfg(not(any(
    target_os = "linux",
    target_os = "freebsd",
    target_os = "netbsd",
    target_os = "openbsd",
    target_os = "dragonfly"
)))]
pub fn install_desktop_entry() -> std::io::Result<()> {
    println!("not applicable on this platform (freedesktop desktop entries are Linux/BSD only)");
    Ok(())
}

#[cfg(not(any(
    target_os = "linux",
    target_os = "freebsd",
    target_os = "netbsd",
    target_os = "openbsd",
    target_os = "dragonfly"
)))]
pub fn uninstall_desktop_entry() -> std::io::Result<()> {
    println!("not applicable on this platform (freedesktop desktop entries are Linux/BSD only)");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn desktop_entry_content_is_complete() {
        let content = desktop_entry_content("/home/user/.cargo/bin/agentmux");
        assert!(content.starts_with("[Desktop Entry]\n"));
        assert!(content.contains("Type=Application\n"));
        assert!(content.contains("Name=agentmux\n"));
        assert!(content.contains("Comment=Native GUI for running multiple hermes coding agents in tabs\n"));
        assert!(content.contains("Exec=/home/user/.cargo/bin/agentmux\n"));
        assert!(content.contains("Icon=agentmux\n"));
        assert!(content.contains("Terminal=false\n"));
        assert!(content.contains("Categories=Development;TerminalEmulator;Utility;\n"));
        assert!(content.contains("StartupNotify=true\n"));
    }

    #[test]
    fn paths_are_computed_under_data_dir() {
        let paths = paths_for(Path::new("/tmp/amx-data"));
        assert_eq!(
            paths.desktop_file,
            PathBuf::from("/tmp/amx-data/applications/agentmux.desktop")
        );
        assert_eq!(
            paths.icon_file,
            PathBuf::from("/tmp/amx-data/icons/hicolor/scalable/apps/agentmux.svg")
        );
    }

    #[test]
    fn icon_asset_is_valid_svg() {
        assert!(ICON_ASSET.starts_with("<svg"));
        assert!(ICON_ASSET.contains("#1e1e2e"), "palette background");
        assert!(ICON_ASSET.contains("#89b4fa"), "accent prompt motif");
    }
}
