//! Process-scan agent identification (sysinfo-based).
//!
//! Each session owns a shell PID (`TerminalBackend::pty_id()`). We walk that
//! shell's descendants via parent links in a shared sysinfo snapshot and
//! match each live non-zombie descendant's cmdline against the known-agent
//! table, peeling node/bun/python wrappers where needed.

use crate::detect::{AgentKind, AGENTS};
use std::collections::HashMap;
use std::ffi::OsString;
use std::path::Path;
use sysinfo::{Pid, ProcessStatus, System};

/// A matched agent process under a session's shell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Candidate {
    pub kind: AgentKind,
    pub pid: u32,
    /// Distance from the shell process (shell itself = 0).
    pub depth: u32,
}

/// Interpreters whose argv we peel to find wrapped agents.
const WRAPPER_EXES: &[&str] = &["node", "deno", "bun", "python", "python3"];

/// Match one process's exe + argv against the known-agent table.
///
/// Direct match: the exe (or argv[0]) basename equals a known exe name
/// (`claude`, `omp`, `codex`, …). Wrapper peel: if the exe basename is an
/// interpreter (node/deno/bun/python), each remaining argv entry is matched
/// by basename against the table's exe names (`python -m hermes`) and by
/// substring against the table's `wrapped_contains` patterns
/// (`node .../claude-code/cli.js`).
pub fn match_cmdline(exe: Option<&Path>, cmd: &[OsString]) -> Option<AgentKind> {
    let exe_base = exe.and_then(|p| p.file_name()).and_then(|s| s.to_str());
    let argv0_base = cmd
        .first()
        .and_then(|arg| Path::new(arg).file_name())
        .and_then(|s| s.to_str());
    let bases = [exe_base, argv0_base];

    // Direct exe-name match against either the exe or argv[0] basename.
    for base in bases.iter().flatten() {
        if let Some(def) = AGENTS.iter().find(|def| def.exe_names.contains(base)) {
            return Some(def.kind);
        }
    }

    // Wrapper peeling for interpreters.
    if bases.iter().flatten().any(|b| WRAPPER_EXES.contains(b)) {
        for arg in cmd.iter().skip(1) {
            let arg_str = arg.to_string_lossy();
            let arg_base = Path::new(arg_str.as_ref())
                .file_name()
                .map(|s| s.to_string_lossy());
            for def in AGENTS {
                if let Some(arg_base) = &arg_base
                    && def.exe_names.contains(&arg_base.as_ref())
                {
                    return Some(def.kind);
                }
                if def.wrapped_contains.iter().any(|pat| arg_str.contains(pat)) {
                    return Some(def.kind);
                }
            }
        }
    }

    None
}

/// Walk the descendants of `shell_pid` in the given process snapshot and
/// return every live (non-zombie) agent-shaped process, shallowest-first.
///
/// The snapshot must already be refreshed (once per detection tick, shared
/// across sessions — see `app.rs`).
pub fn scan_agents(system: &System, shell_pid: u32) -> Vec<Candidate> {
    // Build a parent -> children map from the snapshot.
    let mut children: HashMap<u32, Vec<u32>> = HashMap::new();
    for (pid, proc) in system.processes() {
        if let Some(parent) = proc.parent() {
            children.entry(parent.as_u32()).or_default().push(pid.as_u32());
        }
    }

    let mut out = Vec::new();
    let mut stack: Vec<(u32, u32)> = vec![(shell_pid, 0)];
    let mut visited = std::collections::HashSet::new();

    while let Some((pid, depth)) = stack.pop() {
        if !visited.insert(pid) {
            continue;
        }
        if pid != shell_pid
            && let Some(proc) = system.process(Pid::from_u32(pid))
            && proc.status() != ProcessStatus::Zombie
            && let Some(kind) = match_cmdline(proc.exe(), proc.cmd())
        {
            out.push(Candidate { kind, pid, depth });
        }
        if let Some(kids) = children.get(&pid) {
            for &kid in kids {
                stack.push((kid, depth + 1));
            }
        }
    }

    out.sort_by_key(|c| c.depth);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use sysinfo::ProcessRefreshKind;

    fn os(s: &str) -> OsString {
        OsString::from(s)
    }

    #[test]
    fn direct_exe_matches() {
        // Native binaries (claude is a Bun-compiled ELF on this machine).
        assert_eq!(
            match_cmdline(Some(Path::new("/opt/claude-code/bin/claude")), &[os("/opt/claude-code/bin/claude")]),
            Some(AgentKind::ClaudeCode)
        );
        assert_eq!(
            match_cmdline(Some(Path::new("/usr/bin/codex")), &[os("codex"), os("exec"), os("hi")]),
            Some(AgentKind::Codex)
        );
        assert_eq!(
            match_cmdline(Some(Path::new("/usr/local/bin/gemini")), &[os("gemini")]),
            Some(AgentKind::Gemini)
        );
        assert_eq!(
            match_cmdline(Some(Path::new("/usr/bin/kimi")), &[os("kimi")]),
            Some(AgentKind::Kimi)
        );
        // argv[0] fallback when exe is the interpreter running a script.
        assert_eq!(
            match_cmdline(Some(Path::new("/usr/bin/sh")), &[os("/usr/bin/claude")]),
            Some(AgentKind::ClaudeCode)
        );
    }

    #[test]
    fn wrapper_peeling() {
        // node wrapper with a claude-code path (npm global install).
        assert_eq!(
            match_cmdline(
                Some(Path::new("/usr/bin/node")),
                &[os("node"), os("/usr/lib/node_modules/@anthropic-ai/claude-code/cli.js"), os("-p"), os("hi")]
            ),
            Some(AgentKind::ClaudeCode)
        );
        // bun shim: `bun /home/.../.bun/bin/omp` (real argv on this machine).
        assert_eq!(
            match_cmdline(
                Some(Path::new("/usr/bin/bun")),
                &[os("bun"), os("/home/catitw/.cache/.bun/bin/omp")]
            ),
            Some(AgentKind::Omp)
        );
        // bun worker daemons spawned by omp must NOT match.
        assert_eq!(
            match_cmdline(
                Some(Path::new("/usr/bin/bun")),
                &[os("cli.js"), os("__omp_worker_daemon_broker")]
            ),
            None
        );
        assert_eq!(
            match_cmdline(
                Some(Path::new("/usr/bin/bun")),
                &[os("cli.js"), os("__omp_worker_lsp_mux")]
            ),
            None
        );
        // python module form.
        assert_eq!(
            match_cmdline(
                Some(Path::new("/usr/bin/python3")),
                &[os("python3"), os("-m"), os("hermes")]
            ),
            Some(AgentKind::Hermes)
        );
        // kimi-code bundled wrapper.
        assert_eq!(
            match_cmdline(
                Some(Path::new("/usr/bin/node")),
                &[os("node"), os("/opt/kimi-code/bin/kimi-code/index.js")]
            ),
            Some(AgentKind::Kimi)
        );
    }

    #[test]
    fn no_false_positives_plain_shells_and_tools() {
        // Plain shells and unrelated tools must never match.
        for cmd in [
            &[os("bash")][..],
            &[os("/usr/bin/fish")][..],
            &[os("zsh")][..],
            &[os("top")][..],
            &[os("vim"), os("README.md")][..],
            &[os("htop")][..],
            &[os("git"), os("status")][..],
            &[os("node"), os("/home/me/scripts/server.js")][..],
        ] {
            assert_eq!(match_cmdline(None, cmd), None, "unexpected match for {cmd:?}");
        }
    }

    #[test]
    fn scan_plain_bash_has_no_agents() {
        // Live end-to-end check: a plain bash session must yield no
        // candidates (the acceptance "no false positives" for the process
        // layer).
        let mut child = std::process::Command::new("bash")
            .arg("-c")
            .arg("sleep 30")
            .spawn()
            .expect("bash should be available");
        std::thread::sleep(std::time::Duration::from_millis(300));
        let mut system = System::new();
        system.refresh_processes_specifics(
            sysinfo::ProcessesToUpdate::All,
            true,
            ProcessRefreshKind::everything(),
        );
        let candidates = scan_agents(&system, child.id());
        assert!(
            candidates.is_empty(),
            "plain bash session matched agents: {candidates:?}"
        );
        let _ = child.kill();
        let _ = child.wait();
    }

    #[test]
    fn scan_finds_wrapped_agent_descendant() {
        // Live check with a fake wrapped agent: run an interpreter with a
        // claude-code-shaped argv entry and expect the descendant scan to
        // find it. Uses whichever interpreter exists on the machine.
        let interpreters: &[(&str, &[&str])] = &[
            ("node", &["-e", "setTimeout(()=>{}, 8000)"]),
            ("bun", &["-e", "setTimeout(()=>{}, 8000)"]),
            ("python3", &["-c", "import time; time.sleep(8)"]),
        ];
        let Some((interp, args)) = interpreters
            .iter()
            .find(|(interp, _)| which(interp))
        else {
            return; // no interpreter available; skip
        };
        // Shape: bash (the "session shell") -> interpreter with a
        // claude-code-shaped argv entry (the "agent"). Each arg is
        // single-quoted so spaces inside the interpreter args survive.
        let quoted: Vec<String> = std::iter::once(interp.to_string())
            .chain(args.iter().map(|a| format!("'{}'", a)))
            .chain(std::iter::once(
                "'/fake/node_modules/@anthropic-ai/claude-code/cli.js'".to_string(),
            ))
            .chain(std::iter::once("& wait".to_string()))
            .collect();
        let mut child = std::process::Command::new("bash")
            .arg("-c")
            .arg(quoted.join(" "))
            .spawn()
            .expect("bash should spawn");
        std::thread::sleep(std::time::Duration::from_millis(300));
        let mut system = System::new();
        system.refresh_processes_specifics(
            sysinfo::ProcessesToUpdate::All,
            true,
            ProcessRefreshKind::everything(),
        );
        let candidates = scan_agents(&system, child.id());
        assert!(
            candidates.iter().any(|c| c.kind == AgentKind::ClaudeCode),
            "wrapped claude-code descendant not found: {candidates:?}"
        );
        let _ = child.kill();
        let _ = child.wait();
    }

    fn which(bin: &str) -> bool {
        std::env::var_os("PATH")
            .map(|paths| {
                std::env::split_paths(&paths)
                    .any(|dir| dir.join(bin).is_file())
            })
            .unwrap_or(false)
    }
}
