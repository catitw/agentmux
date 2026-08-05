//! Application state and the `eframe::App` implementation.

use crate::detect::engine::Detector;
use crate::detect::{AgentState, Detection};
use crate::notify::ToastQueue;
use crate::session::{Session, SessionStatus};
use crate::status::status_from_pty_event;
use crate::{detect, sidebar, terminal_pane};
use egui_term::{BackendSettings, PtyEvent, TerminalBackend};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};
use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System};

/// How often the shared sysinfo process snapshot is refreshed.
const PROCESS_SCAN_INTERVAL: Duration = Duration::from_millis(500);
/// Minimum interval between per-session grid clones (screen evaluations).
const SCREEN_SYNC_INTERVAL: Duration = Duration::from_millis(250);
/// Backstop repaint cadence so detection keeps running between events.
const REPAINT_BACKSTOP: Duration = Duration::from_millis(300);

/// A live session: its metadata plus the embedded terminal running it.
///
/// `backend` is `None` only when spawning the terminal failed; the session
/// stays visible (marked `Error`) instead of crashing the app.
pub struct SessionEntry {
    pub session: Session,
    pub backend: Option<TerminalBackend>,
    /// Title reported by the terminal (OSC 0/2), used as the tab label.
    pub terminal_title: Option<String>,
    /// Why the terminal failed to spawn, when `backend` is `None`.
    pub spawn_error: Option<String>,
    /// Shell PID from the backend's PTY — root of the agent process scan.
    pub shell_pid: u32,
    /// Current agent-layer detection (`None` = no agent running).
    pub detection: Option<Detection>,
    /// When the current agent was first detected (toast timing).
    pub agent_detected_at: Option<Instant>,
    /// When the current agent state began.
    pub state_since: Option<Instant>,
    /// Per-session detection machinery (sync throttle, process candidates).
    pub detector: Detector,
    /// Set when PTY events arrived since the last detection pass.
    pub needs_rescan: bool,
}

/// UI actions produced by the sidebar / tab bar, applied by the app after
/// the panel closures have released their borrows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// Select a session's tab.
    Select(u64),
    /// Close a session's tab.
    Close(u64),
    /// Spawn a new default shell session.
    NewSession,
}

pub struct AgentMuxApp {
    sessions: BTreeMap<u64, SessionEntry>,
    selected_id: Option<u64>,
    next_id: u64,
    /// Shared channel: egui_term backends push `(session id, event)` here.
    pty_sender: Sender<(u64, PtyEvent)>,
    /// MUST outlive every backend: egui_term's subscription thread panics if
    /// the receiving end is dropped while a terminal is alive.
    pty_receiver: Receiver<(u64, PtyEvent)>,
    /// Shared process snapshot for agent identification, refreshed once per
    /// detection tick and walked per session.
    system: System,
    last_process_scan: Instant,
    toasts: ToastQueue,
}

impl AgentMuxApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let (pty_sender, pty_receiver) = mpsc::channel();
        let mut app = Self {
            sessions: BTreeMap::new(),
            selected_id: None,
            next_id: 0,
            pty_sender,
            pty_receiver,
            system: System::new(),
            last_process_scan: Instant::now(),
            toasts: ToastQueue::new(),
        };
        // Seed one default session so the window is not empty on first launch.
        app.spawn_session(
            cc.egui_ctx.clone(),
            default_work_dir(),
            "Shell",
            &default_shell_command(),
        );
        app
    }

    /// Spawn a new session: a terminal running `command` in `work_dir`.
    fn spawn_session(
        &mut self,
        ctx: egui::Context,
        work_dir: PathBuf,
        tool_name: &str,
        command: &str,
    ) {
        let id = self.next_id;
        self.next_id += 1;

        let (backend, spawn_error) = match TerminalBackend::new(
            id,
            ctx,
            self.pty_sender.clone(),
            BackendSettings {
                shell: command.to_owned(),
                args: Vec::new(),
                working_directory: Some(work_dir.clone()),
            },
        ) {
            Ok(backend) => (Some(backend), None),
            Err(err) => (None, Some(err.to_string())),
        };

        let session = Session {
            id,
            work_dir,
            tool_name: tool_name.to_owned(),
            command: command.to_owned(),
            status: if spawn_error.is_some() {
                SessionStatus::Error
            } else {
                SessionStatus::Running
            },
        };

        let shell_pid = backend.as_ref().map(|b| b.pty_id()).unwrap_or(0);

        self.selected_id = Some(id);
        self.sessions.insert(
            id,
            SessionEntry {
                session,
                backend,
                terminal_title: None,
                spawn_error,
                shell_pid,
                detection: None,
                agent_detected_at: None,
                state_since: None,
                detector: Detector::new(),
                needs_rescan: false,
            },
        );
    }

    /// Drain the PTY event channel and update the owning sessions.
    ///
    /// Any event for a session marks it dirty so the next detection pass
    /// re-evaluates its screen (esp. `Wakeup`, which fires after every parsed
    /// output batch, and `Title`/`ResetTitle` which carry the OSC title).
    fn drain_pty_events(&mut self) {
        while let Ok((id, event)) = self.pty_receiver.try_recv() {
            if let Some(status) = status_from_pty_event(&event) {
                if let Some(entry) = self.sessions.get_mut(&id) {
                    entry.session.status = status;
                    entry.needs_rescan = true;
                }
            } else {
                match event {
                    PtyEvent::Title(title) => {
                        if let Some(entry) = self.sessions.get_mut(&id) {
                            entry.terminal_title = Some(title);
                            entry.needs_rescan = true;
                        }
                    }
                    PtyEvent::ResetTitle => {
                        if let Some(entry) = self.sessions.get_mut(&id) {
                            entry.terminal_title = None;
                            entry.needs_rescan = true;
                        }
                    }
                    PtyEvent::Wakeup => {
                        if let Some(entry) = self.sessions.get_mut(&id) {
                            entry.needs_rescan = true;
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    /// One detection pass over all sessions.
    ///
    /// The shared sysinfo snapshot is refreshed at most every
    /// [`PROCESS_SCAN_INTERVAL`]; per-session screen clones are throttled to
    /// [`SCREEN_SYNC_INTERVAL`]. A session is evaluated when it is dirty
    /// (PTY events arrived) or the process scan ticked.
    fn run_detection(&mut self, now: Instant) {
        let process_tick = now.duration_since(self.last_process_scan) >= PROCESS_SCAN_INTERVAL;
        if process_tick {
            // Everything: plain refresh_processes omits cmdlines, which we
            // need for agent matching.
            self.system.refresh_processes_specifics(
                ProcessesToUpdate::All,
                true,
                ProcessRefreshKind::everything(),
            );
            self.last_process_scan = now;
        }

        let mut new_toasts = Vec::new();
        for entry in self.sessions.values_mut() {
            if entry.backend.is_none() {
                continue;
            }
            if process_tick {
                entry.detector.candidates = detect::process::scan_agents(&self.system, entry.shell_pid);
            }
            if !entry.needs_rescan && !process_tick {
                continue;
            }
            let due = entry
                .detector
                .last_sync
                .is_none_or(|last| now.duration_since(last) >= SCREEN_SYNC_INTERVAL);
            if !due {
                continue;
            }
            entry.needs_rescan = false;
            entry.detector.last_sync = Some(now);

            let backend = entry.backend.as_mut().expect("checked above");
            let bottom = detect::screen::bottom_non_empty_lines(
                &backend.sync().grid,
                detect::screen::DEFAULT_BOTTOM_LINES,
            );
            let title = entry.terminal_title.as_deref();
            let new_detection = entry.detector.evaluate(&bottom, title);

            let old_detection = entry.detection;
            if new_detection != old_detection {
                match (old_detection, new_detection) {
                    (None, Some(detection)) => {
                        entry.agent_detected_at = Some(now);
                        entry.state_since = Some(now);
                        new_toasts.push(format!("{} detected", detection.agent.display_name()));
                    }
                    (Some(old), Some(new)) if old.agent == new.agent => {
                        if new.state == AgentState::Blocked && old.state != AgentState::Blocked {
                            new_toasts.push(format!("{} needs attention", new.agent.display_name()));
                        } else if old.state == AgentState::Working && new.state == AgentState::Idle {
                            new_toasts.push(format!("{} finished", new.agent.display_name()));
                        }
                        entry.state_since = Some(now);
                    }
                    // Agent change / agent exit: no toast (noise).
                    _ => {}
                }
                entry.detection = new_detection;
            }
        }

        for toast in new_toasts {
            self.toasts.push(toast);
        }
    }

    fn close_session(&mut self, id: u64) {
        let was_selected = self.selected_id == Some(id);
        self.sessions.remove(&id);
        if was_selected {
            // Prefer the next tab, fall back to the last remaining one.
            self.selected_id = self
                .sessions
                .range(id..)
                .next()
                .map(|(next_id, _)| *next_id)
                .or_else(|| self.sessions.keys().next_back().copied());
        }
    }

    fn apply_action(&mut self, ctx: egui::Context, action: Action) {
        match action {
            Action::Select(id) => self.selected_id = Some(id),
            Action::Close(id) => self.close_session(id),
            Action::NewSession => self.spawn_session(
                ctx,
                default_work_dir(),
                "Shell",
                &default_shell_command(),
            ),
        }
    }
}

impl eframe::App for AgentMuxApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        // Shut the terminal backends down cleanly when the window closes.
        if ui.ctx().input(|i| i.viewport().close_requested()) {
            self.sessions.clear();
            self.selected_id = None;
        }

        self.drain_pty_events();
        self.run_detection(Instant::now());

        // Backstop: keep the detection loop (and status UI) ticking even
        // when no PTY events arrive.
        ui.ctx().request_repaint_after(REPAINT_BACKSTOP);

        let mut action: Option<Action> = None;

        egui::Panel::left("agentmux_sidebar")
            .default_size(240.0)
            .show(ui, |ui| {
                action = sidebar::show(ui, &self.sessions, self.selected_id);
            });

        egui::Panel::top("agentmux_tab_bar").show(ui, |ui| {
            action = action.or_else(|| terminal_pane::tab_bar(ui, &self.sessions, self.selected_id));
        });

        egui::CentralPanel::default().show(ui, |ui| {
            let selected = self
                .selected_id
                .and_then(|id| self.sessions.get_mut(&id));
            match selected {
                Some(entry) => terminal_pane::terminal_view(ui, entry),
                None => terminal_pane::empty_placeholder(ui),
            }
        });

        self.toasts.show(ui.ctx());

        if let Some(action) = action {
            self.apply_action(ui.ctx().clone(), action);
        }
    }
}

/// Default work directory for new sessions: `$HOME` (or `$USERPROFILE` on
/// Windows), falling back to the current directory.
fn default_work_dir() -> PathBuf {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

/// Default shell for new sessions: `$SHELL` on Unix, `cmd.exe` on Windows.
fn default_shell_command() -> String {
    #[cfg(windows)]
    {
        "cmd.exe".to_owned()
    }
    #[cfg(not(windows))]
    {
        std::env::var("SHELL").unwrap_or_else(|_| "bash".to_owned())
    }
}
