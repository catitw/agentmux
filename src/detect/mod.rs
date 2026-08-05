//! Agent identification and status detection (phase 2).
//!
//! Two layers, kept separate:
//! - process layer: [`crate::session::SessionStatus`] (shell child alive /
//!   exited) — unchanged from phase 1;
//! - agent layer: which hermes tool is running in a session
//!   ([`AgentKind`]) and what it is doing ([`AgentState`]).
//!
//! Signals used: a sysinfo process scan rooted at the session's shell PID
//! (identification) plus the terminal screen / OSC title (status), exactly
//! the channels the phase-2 feasibility report proved available through
//! egui_term's public API. OSC 9/99/133 and `tcgetpgrp` are NOT available
//! (see docs/phase2-detection.md for the honesty box).

pub mod engine;
pub mod process;
pub mod screen;

use egui::Color32;

/// The hermes coding-agent tools agentmux can identify.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AgentKind {
    ClaudeCode,
    Omp,
    Pi,
    Codex,
    Gemini,
    OpenCode,
    Kilo,
    Kimi,
    Hermes,
}

/// What a detected agent is doing right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentState {
    /// Actively working (streaming, running tools, thinking).
    Working,
    /// Prompt visible, no activity.
    Idle,
    /// Waiting for user input to continue (permission/question UI).
    Blocked,
}

/// A detected agent plus its current state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Detection {
    pub agent: AgentKind,
    pub state: AgentState,
}

/// Static description of one known agent: how to match it on the command
/// line and how to display it.
pub struct AgentDef {
    pub kind: AgentKind,
    /// Human-readable name for the UI.
    pub display_name: &'static str,
    /// Exact exe / argv[0] basenames that identify this agent directly
    /// (e.g. `claude`, `omp`, `codex`).
    pub exe_names: &'static [&'static str],
    /// Substrings matched against wrapper argv entries (node/bun/python
    /// wrappers), e.g. "claude-code" for
    /// `node .../node_modules/@anthropic-ai/claude-code/cli.js`.
    pub wrapped_contains: &'static [&'static str],
}

/// The known-agent table. Extend here, not in scattered match arms.
pub static AGENTS: &[AgentDef] = &[
    AgentDef {
        kind: AgentKind::ClaudeCode,
        display_name: "Claude Code",
        exe_names: &["claude"],
        wrapped_contains: &["claude-code"],
    },
    AgentDef {
        kind: AgentKind::Omp,
        display_name: "omp",
        exe_names: &["omp"],
        // omp is the @oh-my-pi/pi-coding-agent CLI, run via its bun shim.
        wrapped_contains: &["pi-coding-agent", "/omp"],
    },
    AgentDef {
        kind: AgentKind::Pi,
        display_name: "pi",
        exe_names: &["pi"],
        wrapped_contains: &["/pi", "pi-agent"],
    },
    AgentDef {
        kind: AgentKind::Codex,
        display_name: "Codex",
        exe_names: &["codex"],
        wrapped_contains: &["codex"],
    },
    AgentDef {
        kind: AgentKind::Gemini,
        display_name: "Gemini CLI",
        exe_names: &["gemini"],
        wrapped_contains: &["gemini-cli", "/gemini"],
    },
    AgentDef {
        kind: AgentKind::OpenCode,
        display_name: "OpenCode",
        exe_names: &["opencode"],
        wrapped_contains: &["/opencode", "opencode-"],
    },
    AgentDef {
        kind: AgentKind::Kilo,
        display_name: "Kilo",
        exe_names: &["kilo"],
        wrapped_contains: &["/kilo"],
    },
    AgentDef {
        kind: AgentKind::Kimi,
        display_name: "Kimi Code",
        exe_names: &["kimi"],
        wrapped_contains: &["kimi-code"],
    },
    AgentDef {
        kind: AgentKind::Hermes,
        display_name: "hermes",
        exe_names: &["hermes", "hermes-agent"],
        wrapped_contains: &["/hermes"],
    },
];

impl AgentKind {
    /// Display label for the UI (sidebar rows, tabs, toasts).
    pub fn display_name(self) -> &'static str {
        AGENTS
            .iter()
            .find(|def| def.kind == self)
            .expect("every AgentKind has an entry in AGENTS")
            .display_name
    }
}

impl AgentState {
    /// Accent color for the state indicator (sidebar dot, tab marker).
    pub fn color(self) -> Color32 {
        match self {
            AgentState::Working => Color32::from_rgb(86, 156, 255),
            AgentState::Idle => Color32::from_rgb(139, 148, 158),
            AgentState::Blocked => Color32::from_rgb(229, 165, 10),
        }
    }

    /// Short human label, used in tooltips.
    pub fn label(self) -> &'static str {
        match self {
            AgentState::Working => "working",
            AgentState::Idle => "idle",
            AgentState::Blocked => "blocked",
        }
    }
}
