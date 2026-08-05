//! Hook-channel integration (herdr's Channel C): agents report lifecycle
//! states to an agentmux-owned loopback HTTP server; the reports carry a PID
//! which agentmux resolves to a session via the sysinfo ancestor walk.
//!
//! Protocol: `POST /report` with
//! `{"pid": u32, "agent": "<exe name>", "state": "working"|"idle"|"blocked"|"clear",
//!   "message": optional string}`.
//!
//! PID semantics: claude hooks are spawned per event and report `$PPID` (the
//! claude process — possibly behind an intermediate `sh -c`); omp/pi
//! extensions run inside the agent and report their own pid. Either way the
//! session is found by walking ancestors to a known session `shell_pid`.

pub mod install;
pub mod server;

pub use server::ReportServer;

use crate::detect::process::match_cmdline;
use crate::detect::{AgentKind, AgentState, AGENTS};
use std::time::Instant;
use sysinfo::{Pid, System};

/// States the report protocol accepts (hook assets map their events onto
/// these). `Clear` releases hook authority (SessionEnd / session_shutdown).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookState {
    Working,
    Idle,
    Blocked,
    Clear,
}

impl HookState {
    /// The agent state this hook state implies; `Clear` has none.
    pub fn as_agent_state(self) -> Option<AgentState> {
        match self {
            HookState::Working => Some(AgentState::Working),
            HookState::Idle => Some(AgentState::Idle),
            HookState::Blocked => Some(AgentState::Blocked),
            HookState::Clear => None,
        }
    }
}

/// One decoded report from the loopback server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HookReport {
    pub pid: u32,
    pub agent: AgentKind,
    pub state: HookState,
    pub message: Option<String>,
}

/// Live hook authority for a session: while it is held, the hook's state
/// overrides the screen engine.
#[derive(Debug, Clone)]
pub struct HookAuthority {
    /// Agent the report identified (the hook knows its agent).
    pub agent: AgentKind,
    pub state: AgentState,
    pub reported_at: Instant,
    pub message: Option<String>,
    /// Persistent agent process the report referenced (resolved past any
    /// wrapper shells); authority stays live only while this remains a
    /// descendant of the session's shell.
    pub agent_pid: u32,
}

/// Parse a report JSON body. `Err` carries a human-readable reason (the
/// server maps it to 400).
pub fn parse_report(body: &str) -> Result<HookReport, String> {
    let value: serde_json::Value =
        serde_json::from_str(body).map_err(|err| format!("invalid JSON: {err}"))?;
    let pid = value
        .get("pid")
        .and_then(|v| v.as_u64())
        .filter(|pid| *pid > 0 && *pid <= u64::from(u32::MAX))
        .map(|pid| pid as u32)
        .ok_or("missing or invalid \"pid\"")?;
    let agent_name = value
        .get("agent")
        .and_then(|v| v.as_str())
        .ok_or("missing \"agent\"")?;
    let agent = parse_agent(agent_name)?;
    let state_name = value
        .get("state")
        .and_then(|v| v.as_str())
        .ok_or("missing \"state\"")?;
    let state = parse_state(state_name)?;
    let message = value.get("message").and_then(|v| v.as_str()).map(str::to_owned);
    Ok(HookReport { pid, agent, state, message })
}

/// Parse the agent field: any known exe name (case-insensitive) or display
/// name, e.g. "claude", "omp", "Claude Code".
fn parse_agent(name: &str) -> Result<AgentKind, String> {
    AGENTS
        .iter()
        .find(|def| {
            def.exe_names
                .iter()
                .any(|exe| exe.eq_ignore_ascii_case(name))
                || def.display_name.eq_ignore_ascii_case(name)
        })
        .map(|def| def.kind)
        .ok_or_else(|| format!("unknown agent \"{name}\""))
}

fn parse_state(name: &str) -> Result<HookState, String> {
    match name {
        "working" => Ok(HookState::Working),
        "idle" => Ok(HookState::Idle),
        "blocked" => Ok(HookState::Blocked),
        "clear" => Ok(HookState::Clear),
        _ => Err(format!("unknown state \"{name}\"")),
    }
}

/// Find the session whose shell pid is an ancestor of `pid` in the given
/// process snapshot. `shells` maps shell pids to session ids.
///
/// Walks up parent links until a known shell is hit, the chain ends, or a
/// loop is detected. A reported pid outside any session (e.g. an agent
/// launched outside agentmux) yields `None` and is ignored.
pub fn find_session_for_pid(system: &System, pid: u32, shells: &[(u32, u64)]) -> Option<u64> {
    let mut current = pid;
    for _ in 0..1024 {
        if let Some((_, session_id)) = shells.iter().find(|(shell, _)| *shell == current) {
            return Some(*session_id);
        }
        let parent = system.process(Pid::from_u32(current))?.parent()?.as_u32();
        if parent == current {
            return None;
        }
        current = parent;
    }
    None
}

/// Resolve the persistent agent process for a report: the reported pid
/// itself, or the first ancestor whose cmdline matches the reported agent
/// kind. Claude may run hooks via an intermediate `sh -c`, so the reported
/// `$PPID` can be a short-lived shell; the matching ancestor (the claude
/// process) is the one whose disappearance releases hook authority.
pub fn resolve_agent_pid(system: &System, report_pid: u32, agent: AgentKind) -> u32 {
    let mut current = report_pid;
    for _ in 0..64 {
        if let Some(proc) = system.process(Pid::from_u32(current))
            && match_cmdline(proc.exe(), proc.cmd()) == Some(agent)
        {
            return current;
        }
        let Some(parent) = system.process(Pid::from_u32(current)).and_then(|p| p.parent()) else {
            break;
        };
        let parent = parent.as_u32();
        if parent == current {
            break;
        }
        current = parent;
    }
    report_pid
}

/// Hook authority liveness: the resolved agent process must still be a
/// descendant of the session's shell in the current snapshot.
pub fn hook_is_live(system: &System, agent_pid: u32, shell_pid: u32) -> bool {
    let mut current = agent_pid;
    for _ in 0..1024 {
        if current == shell_pid {
            return true;
        }
        let Some(parent) = system.process(Pid::from_u32(current)).and_then(|p| p.parent()) else {
            return false;
        };
        let parent = parent.as_u32();
        if parent == current {
            return false;
        }
        current = parent;
    }
    false
}

/// Pure arbitration between hook authority and the screen engine.
///
/// Returns `(effective detection, source, hook_released)`. A live hook wins
/// over the screen engine; a dead hook is released and the screen result is
/// used.
pub fn arbitrate(
    hook: Option<&HookAuthority>,
    hook_live: bool,
    screen: Option<crate::detect::Detection>,
) -> (Option<crate::detect::Detection>, &'static str, bool) {
    match hook {
        Some(hook) if hook_live => (
            Some(crate::detect::Detection {
                agent: hook.agent,
                state: hook.state,
            }),
            "hook",
            false,
        ),
        Some(_) => (screen, "screen", true),
        None => (screen, "screen", false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::detect::Detection;
    use std::process::Command;
    use sysinfo::ProcessRefreshKind;

    fn snapshot() -> System {
        let mut system = System::new();
        system.refresh_processes_specifics(
            sysinfo::ProcessesToUpdate::All,
            true,
            ProcessRefreshKind::everything(),
        );
        system
    }

    #[test]
    fn parse_report_valid() {
        let report =
            parse_report(r#"{"pid": 1234, "agent": "claude", "state": "working", "message": "hi"}"#)
                .unwrap();
        assert_eq!(report.pid, 1234);
        assert_eq!(report.agent, AgentKind::ClaudeCode);
        assert_eq!(report.state, HookState::Working);
        assert_eq!(report.message.as_deref(), Some("hi"));

        // Display names and case-insensitive exe names also work.
        let report = parse_report(r#"{"pid": 7, "agent": "Claude Code", "state": "blocked"}"#).unwrap();
        assert_eq!(report.agent, AgentKind::ClaudeCode);
        assert_eq!(report.state, HookState::Blocked);
        assert_eq!(report.message, None);

        let report = parse_report(r#"{"pid": 7, "agent": "omp", "state": "clear"}"#).unwrap();
        assert_eq!(report.agent, AgentKind::Omp);
        assert_eq!(report.state, HookState::Clear);
    }

    #[test]
    fn parse_report_invalid() {
        for (body, what) in [
            ("not json", "syntax"),
            (r#"{"agent": "claude", "state": "working"}"#, "missing pid"),
            (r#"{"pid": 0, "agent": "claude", "state": "working"}"#, "zero pid"),
            (r#"{"pid": 1, "state": "working"}"#, "missing agent"),
            (r#"{"pid": 1, "agent": "claude"}"#, "missing state"),
            (r#"{"pid": 1, "agent": "cortana", "state": "working"}"#, "unknown agent"),
            (r#"{"pid": 1, "agent": "claude", "state": "confused"}"#, "unknown state"),
            (r#"{"pid": 99999999999, "agent": "claude", "state": "working"}"#, "pid overflow"),
        ] {
            assert!(parse_report(body).is_err(), "{what} must be rejected: {body}");
        }
    }

    #[test]
    fn find_session_walks_real_process_tree() {
        // bash ("session shell") -> sleep (reported process).
        let mut shell = Command::new("bash").arg("-c").arg("sleep 30 & wait").spawn().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(300));
        let sleep_pid = {
            let system = snapshot();
            system
                .processes()
                .iter()
                .find(|(_, p)| p.parent().is_some_and(|pp| pp.as_u32() == shell.id()))
                .map(|(pid, _)| pid.as_u32())
                .expect("sleep child should exist")
        };

        let system = snapshot();
        // From the sleep (the "reported pid") the walk finds the shell.
        assert_eq!(
            find_session_for_pid(&system, sleep_pid, &[(shell.id(), 42)]),
            Some(42)
        );
        // The shell itself resolves to its own session.
        assert_eq!(
            find_session_for_pid(&system, shell.id(), &[(shell.id(), 42)]),
            Some(42)
        );
        // An unrelated process (another sleep) is not under this shell.
        let mut other = Command::new("sleep").arg("30").spawn().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(200));
        let system = snapshot();
        assert_eq!(
            find_session_for_pid(&system, other.id(), &[(shell.id(), 42)]),
            None
        );
        // A nonexistent pid is None.
        assert_eq!(find_session_for_pid(&system, u32::MAX - 1, &[(shell.id(), 42)]), None);

        let _ = other.kill();
        let _ = shell.kill();
        let _ = other.wait();
        let _ = shell.wait();
    }

    #[test]
    fn resolve_agent_pid_matches_kind_or_falls_back() {
        // The reported pid itself matches the agent kind -> returned as-is.
        let mut shell = Command::new("bash")
            .arg("-c")
            .arg("python3 -c 'import time; time.sleep(8)' /fake/node_modules/@anthropic-ai/claude-code/cli.js & wait")
            .spawn()
            .unwrap();
        std::thread::sleep(std::time::Duration::from_millis(300));
        let agent_pid = {
            let system = snapshot();
            system
                .processes()
                .iter()
                .find(|(_, p)| p.parent().is_some_and(|pp| pp.as_u32() == shell.id()))
                .map(|(pid, _)| pid.as_u32())
                .expect("python child should exist")
        };
        let system = snapshot();
        assert_eq!(
            resolve_agent_pid(&system, agent_pid, AgentKind::ClaudeCode),
            agent_pid
        );
        // A non-matching pid (the bash shell) falls back to the reported pid.
        assert_eq!(
            resolve_agent_pid(&system, shell.id(), AgentKind::ClaudeCode),
            shell.id()
        );
        let _ = shell.kill();
        let _ = shell.wait();
    }

    #[test]
    fn hook_liveness_tracks_process_gone() {
        let mut shell = Command::new("bash").arg("-c").arg("sleep 30 & wait").spawn().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(300));
        let sleep_pid = {
            let system = snapshot();
            system
                .processes()
                .iter()
                .find(|(_, p)| p.parent().is_some_and(|pp| pp.as_u32() == shell.id()))
                .map(|(pid, _)| pid.as_u32())
                .unwrap()
        };

        let system = snapshot();
        assert!(hook_is_live(&system, sleep_pid, shell.id()));

        // Kill the "agent": liveness must flip on a fresh snapshot.
        Command::new("kill").arg(sleep_pid.to_string()).status().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(300));
        let system = snapshot();
        assert!(!hook_is_live(&system, sleep_pid, shell.id()));

        let _ = shell.kill();
        let _ = shell.wait();
    }

    #[test]
    fn arbitrate_precedence() {
        fn authority(state: AgentState) -> HookAuthority {
            HookAuthority {
                agent: AgentKind::ClaudeCode,
                state,
                reported_at: Instant::now(),
                message: None,
                agent_pid: 1,
            }
        }
        let screen_working = Some(Detection { agent: AgentKind::ClaudeCode, state: AgentState::Working });

        // Live hook overrides the screen engine.
        let (det, source, released) = arbitrate(Some(&authority(AgentState::Blocked)), true, screen_working);
        assert_eq!(det.map(|d| d.state), Some(AgentState::Blocked));
        assert_eq!(source, "hook");
        assert!(!released);

        // Dead hook: released, screen result wins.
        let (det, source, released) = arbitrate(Some(&authority(AgentState::Blocked)), false, screen_working);
        assert_eq!(det, screen_working);
        assert_eq!(source, "screen");
        assert!(released);

        // No hook: screen as today.
        let (det, source, released) = arbitrate(None, false, screen_working);
        assert_eq!(det, screen_working);
        assert_eq!(source, "screen");
        assert!(!released);

        // Live hook with no screen result still yields the hook detection.
        let (det, source, _) = arbitrate(Some(&authority(AgentState::Idle)), true, None);
        assert_eq!(det.map(|d| d.agent), Some(AgentKind::ClaudeCode));
        assert_eq!(source, "hook");
    }
}
