//! Application state and the `eframe::App` implementation.

use crate::session::{Session, SessionStatus};
use crate::status::status_from_pty_event;
use crate::{sidebar, terminal_pane};
use egui_term::{BackendSettings, PtyEvent, TerminalBackend};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};

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

        self.selected_id = Some(id);
        self.sessions.insert(
            id,
            SessionEntry {
                session,
                backend,
                terminal_title: None,
                spawn_error,
            },
        );
    }

    /// Drain the PTY event channel and update the owning sessions.
    fn drain_pty_events(&mut self) {
        while let Ok((id, event)) = self.pty_receiver.try_recv() {
            if let Some(status) = status_from_pty_event(&event) {
                if let Some(entry) = self.sessions.get_mut(&id) {
                    entry.session.status = status;
                }
            } else {
                match event {
                    PtyEvent::Title(title) => {
                        if let Some(entry) = self.sessions.get_mut(&id) {
                            entry.terminal_title = Some(title);
                        }
                    }
                    PtyEvent::ResetTitle => {
                        if let Some(entry) = self.sessions.get_mut(&id) {
                            entry.terminal_title = None;
                        }
                    }
                    _ => {}
                }
            }
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
