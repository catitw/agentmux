//! Application state and the `eframe::App` implementation.

use crate::detect::engine::Detector;
use crate::detect::{AgentState, Detection};
use crate::hooks::{HookAuthority, HookState, ReportServer};
use crate::new_session::{self, NewSessionDraft};
use crate::notify::ToastQueue;
use crate::persist::{self, SessionMeta};
use crate::project::{ProjectClassifier, ProjectInfo};
use crate::session::{Session, SessionStatus};
use crate::status::status_from_pty_event;
use crate::{detect, fonts, hooks, project, sidebar, terminal_pane, theme, ui_theme};
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
    /// Current working directory of the session (live from /proc on Linux,
    /// else the spawn work_dir). Drives the project grouping.
    pub cwd: PathBuf,
    /// Last project/branch classification of `cwd`.
    pub project: ProjectInfo,
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
    /// Live hook authority (herdr Channel C); while present and its agent
    /// process is alive, its state overrides the screen engine.
    pub hook: Option<HookAuthority>,
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
    /// Loopback hook report server (lives for the whole app lifetime).
    hook_server: ReportServer,
    /// Optional transition log (`AGENTMUX_DEBUG_LOG`), appended on every
    /// detection transition: `session N: agent=X state=Y source=hook|screen`.
    debug_log: Option<PathBuf>,
    /// Terminal font family (preferred terminal font or egui monospace +
    /// system fallbacks), built once at startup.
    terminal_font: egui_term::TerminalFont,
    /// Terminal color theme (ghostty palette or egui_term default), built
    /// once at startup.
    terminal_theme: egui_term::TerminalTheme,
    /// Project/branch classifier with its HEAD-read cache.
    classifier: ProjectClassifier,
    /// Sidebar projects collapsed by the user (project root paths).
    collapsed_projects: std::collections::HashSet<PathBuf>,
    /// Open new-session dialog draft (None = dialog closed).
    new_session: Option<NewSessionDraft>,
}

impl AgentMuxApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        // Register system font fallbacks (CJK / Nerd icons / emoji) before
        // any text is laid out; see docs/fonts.md.
        let font_setup = fonts::setup_fonts(&cc.egui_ctx);
        if font_setup.registered.is_empty() {
            eprintln!("agentmux fonts: no system fallback fonts found (CJK/icon glyphs may render as tofu)");
        } else {
            eprintln!(
                "agentmux fonts: registered fallbacks: {}",
                font_setup.registered.join(", ")
            );
        }
        // Terminal theme (ghostty palette if obtainable), cached once, and
        // the chrome visuals derived from the same palette.
        let (terminal_theme, palette_source, ui_palette) = theme::load_terminal_theme();
        eprintln!(
            "agentmux theme: font '{}', palette {palette_source}",
            font_setup.terminal_font_name
        );
        cc.egui_ctx.set_visuals(ui_theme::build_visuals(&ui_palette));
        // Spacing rhythm: 8px-ish grid, comfortable button padding.
        cc.egui_ctx.style_mut_of(egui::Theme::Dark, |style| {
            style.spacing.item_spacing = egui::vec2(8.0, 5.0);
            style.spacing.button_padding = egui::vec2(10.0, 4.0);
        });

        let (pty_sender, pty_receiver) = mpsc::channel();
        let hook_server = ReportServer::start().expect("failed to start hook report server");
        eprintln!(
            "agentmux hook server listening on 127.0.0.1:{} (port file: {})",
            hook_server.port,
            ReportServer::port_file_path()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|_| "?".to_owned())
        );
        let debug_log = std::env::var_os("AGENTMUX_DEBUG_LOG").map(PathBuf::from);
        let mut app = Self {
            sessions: BTreeMap::new(),
            selected_id: None,
            next_id: 0,
            pty_sender,
            pty_receiver,
            system: System::new(),
            last_process_scan: Instant::now(),
            toasts: ToastQueue::new(),
            hook_server,
            debug_log,
            terminal_font: font_setup.terminal_font,
            terminal_theme,
            classifier: ProjectClassifier::new(),
            collapsed_projects: std::collections::HashSet::new(),
            new_session: None,
        };
        // Session startup: AGENTMUX_SEED_COMMAND (verification hook) takes
        // precedence; otherwise restore persisted sessions; otherwise seed
        // one default session so the window is never empty. The seed value
        // is a shell command line, run via `sh -c` (alacritty's tty layer
        // treats the shell field as a program path, not a command string).
        // AGENTMUX_SEED_DIR overrides the seeded session's workdir
        // (verification/testing hook; default unchanged).
        let seed_dir = std::env::var_os("AGENTMUX_SEED_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(default_work_dir);
        let seed = std::env::var_os("AGENTMUX_SEED_COMMAND")
            .map(|cmd| cmd.to_string_lossy().into_owned());
        match seed {
            Some(cmd) => {
                #[cfg(windows)]
                let (seed_command, seed_args) = ("cmd.exe".to_owned(), vec!["/C".to_owned(), cmd]);
                #[cfg(not(windows))]
                let (seed_command, seed_args) = ("/bin/sh".to_owned(), vec!["-c".to_owned(), cmd]);
                app.spawn_session(
                    cc.egui_ctx.clone(),
                    seed_dir,
                    "Shell",
                    &seed_command,
                    seed_args,
                );
            }
            None => {
                let sessions_path = match persist::sessions_path() {
                    Ok(path) => path,
                    Err(err) => {
                        eprintln!("agentmux: cannot locate sessions file ({err}), seeded default");
                        app.spawn_session(
                            cc.egui_ctx.clone(),
                            default_work_dir(),
                            "Shell",
                            &default_shell_command(),
                            Vec::new(),
                        );
                        return app;
                    }
                };
                match persist::load(&sessions_path) {
                Ok(metas) => {
                    let mut restored = 0usize;
                    for meta in metas {
                        let work_dir = PathBuf::from(&meta.work_dir);
                        if !work_dir.is_dir() {
                            eprintln!(
                                "agentmux: skipping session, work dir missing: {}",
                                meta.work_dir
                            );
                            continue;
                        }
                        app.spawn_session(
                            cc.egui_ctx.clone(),
                            work_dir,
                            &meta.label,
                            &meta.command,
                            Vec::new(),
                        );
                        restored += 1;
                    }
                    eprintln!(
                        "agentmux: restored {restored} session(s) from {}",
                        persist::sessions_path()
                            .map(|p| p.display().to_string())
                            .unwrap_or_else(|_| "?".to_owned())
                    );
                }
                Err(persist::LoadError::NotFound) => {
                    eprintln!("agentmux: no sessions file, seeded default");
                    app.spawn_session(
                        cc.egui_ctx.clone(),
                        default_work_dir(),
                        "Shell",
                        &default_shell_command(),
                        Vec::new(),
                    );
                }
                Err(persist::LoadError::Malformed(err)) => {
                    eprintln!("agentmux: sessions file malformed ({err}), seeded default");
                    app.spawn_session(
                        cc.egui_ctx.clone(),
                        default_work_dir(),
                        "Shell",
                        &default_shell_command(),
                        Vec::new(),
                    );
                }
                }
            }
        }
        app
    }

    /// Spawn a new session: a terminal running `command` (with `args`) in
    /// `work_dir`.
    fn spawn_session(
        &mut self,
        ctx: egui::Context,
        work_dir: PathBuf,
        tool_name: &str,
        command: &str,
        args: Vec<String>,
    ) {
        let id = self.next_id;
        self.next_id += 1;

        let (backend, spawn_error) = match TerminalBackend::new(
            id,
            ctx,
            self.pty_sender.clone(),
            BackendSettings {
                shell: command.to_owned(),
                args,
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

        // Initial classification: spawn work_dir (the live cwd will be
        // picked up on the next process tick).
        let cwd = session.work_dir.clone();
        let project = self.classifier.classify(&cwd, Instant::now());

        self.selected_id = Some(id);
        self.sessions.insert(
            id,
            SessionEntry {
                session,
                backend,
                terminal_title: None,
                spawn_error,
                shell_pid,
                cwd,
                project,
                detection: None,
                agent_detected_at: None,
                state_since: None,
                detector: Detector::new(),
                needs_rescan: false,
                hook: None,
            },
        );
        self.save_sessions();
    }

    /// Persist all live sessions' metadata (order = sidebar order). Failures
    /// are logged, never fatal.
    fn save_sessions(&self) {
        let metas: Vec<SessionMeta> = self
            .sessions
            .values()
            .map(|entry| SessionMeta {
                work_dir: entry.session.work_dir.display().to_string(),
                command: entry.session.command.clone(),
                label: entry.session.tool_name.clone(),
            })
            .collect();
        match persist::sessions_path() {
            Ok(path) => {
                if let Err(err) = persist::save(&path, &metas) {
                    eprintln!("agentmux: failed to save sessions: {err}");
                }
            }
            Err(err) => eprintln!("agentmux: failed to save sessions: {err}"),
        }
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

    /// Drain the hook report channel and apply reports to sessions.
    ///
    /// Each report is resolved against a fresh process snapshot (reports are
    /// rare, so the refresh cost is irrelevant): the session is found by
    /// walking ancestors from the reported pid; the persistent agent process
    /// is resolved by matching the reported agent kind up the same chain.
    /// `Clear` reports (SessionEnd / session_shutdown) release authority.
    /// Reports whose pid is not under any session shell are dropped
    /// (e.g. an agent launched outside agentmux).
    fn drain_hook_reports(&mut self) {
        let mut reports = Vec::new();
        while let Ok(report) = self.hook_server.receiver.try_recv() {
            reports.push(report);
        }
        if reports.is_empty() {
            return;
        }
        self.system.refresh_processes_specifics(
            ProcessesToUpdate::All,
            true,
            ProcessRefreshKind::everything(),
        );
        let now = Instant::now();
        let shells: Vec<(u32, u64)> = self
            .sessions
            .iter()
            .filter(|(_, entry)| entry.shell_pid != 0)
            .map(|(id, entry)| (entry.shell_pid, *id))
            .collect();

        for report in reports {
            let Some(session_id) = hooks::find_session_for_pid(&self.system, report.pid, &shells)
            else {
                continue;
            };
            let Some(entry) = self.sessions.get_mut(&session_id) else {
                continue;
            };
            match report.state {
                HookState::Clear => entry.hook = None,
                state => {
                    let Some(agent_state) = state.as_agent_state() else {
                        continue;
                    };
                    let agent_pid =
                        hooks::resolve_agent_pid(&self.system, report.pid, report.agent);
                    entry.hook = Some(HookAuthority {
                        agent: report.agent,
                        state: agent_state,
                        reported_at: now,
                        message: report.message,
                        agent_pid,
                    });
                }
            }
        }
    }

    /// Append one line to the debug transition log, if enabled.
    fn debug_log(&self, line: &str) {
        let Some(path) = &self.debug_log else { return };
        use std::io::Write;
        if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
            let _ = writeln!(file, "{line}");
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

        let mut new_toasts: Vec<(String, crate::notify::ToastKind)> = Vec::new();
        let mut log_lines = Vec::new();
        for entry in self.sessions.values_mut() {
            if entry.backend.is_none() {
                continue;
            }
            if process_tick {
                entry.detector.candidates = detect::process::scan_agents(&self.system, entry.shell_pid);
                // Live cwd (Linux /proc) + project/branch re-classification.
                // Branch re-reads are cadence-cached per project root.
                entry.cwd = project::live_cwd(entry.shell_pid)
                    .unwrap_or_else(|| entry.session.work_dir.clone());
                entry.project = self.classifier.classify(&entry.cwd, now);
            }

            // Hook authority liveness: released when the resolved agent
            // process disappears from the scan (or a clear report arrived in
            // drain_hook_reports).
            let hook_live = entry
                .hook
                .as_ref()
                .is_some_and(|hook| hooks::hook_is_live(&self.system, hook.agent_pid, entry.shell_pid));
            if entry.hook.is_some() && !hook_live {
                entry.hook = None;
            }

            // Screen engine result (throttled) when no hook authority is held.
            let screen = if entry.hook.is_some() {
                None
            } else {
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
                entry.detector.evaluate(&bottom, title)
            };

            // Arbitration: live hook state wins over the screen engine.
            let (new_detection, source, _hook_released) =
                hooks::arbitrate(entry.hook.as_ref(), entry.hook.is_some(), screen);

            let old_detection = entry.detection;
            if new_detection != old_detection {
                match (old_detection, new_detection) {
                    (None, Some(detection)) => {
                        entry.agent_detected_at = Some(now);
                        entry.state_since = Some(now);
                        new_toasts.push((
                            format!("{} detected", detection.agent.display_name()),
                            crate::notify::ToastKind::Info,
                        ));
                    }
                    (Some(old), Some(new)) if old.agent == new.agent => {
                        if new.state == AgentState::Blocked && old.state != AgentState::Blocked {
                            let message = entry
                                .hook
                                .as_ref()
                                .and_then(|hook| hook.message.clone())
                                .unwrap_or_default();
                            let suffix = if message.is_empty() {
                                String::new()
                            } else {
                                format!(": {message}")
                            };
                            new_toasts.push((
                                format!("{} needs attention{suffix}", new.agent.display_name()),
                                crate::notify::ToastKind::Attention,
                            ));
                        } else if old.state == AgentState::Working && new.state == AgentState::Idle {
                            new_toasts.push((
                                format!("{} finished", new.agent.display_name()),
                                crate::notify::ToastKind::Finished,
                            ));
                        }
                        entry.state_since = Some(now);
                    }
                    // Agent change / agent exit: no toast (noise).
                    _ => {}
                }
                entry.detection = new_detection;
                log_lines.push(format!(
                    "session {}: agent={} state={} source={source}",
                    entry.session.id,
                    new_detection
                        .map(|d| d.agent.display_name().to_owned())
                        .unwrap_or_else(|| "none".to_owned()),
                    new_detection
                        .map(|d| d.state.label().to_owned())
                        .unwrap_or_else(|| "none".to_owned()),
                ));
            }
        }

        for (text, kind) in new_toasts {
            self.toasts.push(text, kind);
        }
        for line in log_lines {
            self.debug_log(&line);
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
        self.save_sessions();
    }

    fn apply_action(&mut self, action: Action) {
        match action {
            Action::Select(id) => self.selected_id = Some(id),
            Action::Close(id) => self.close_session(id),
            // Both "+" buttons open the new-session dialog; the actual spawn
            // happens on dialog submit (with the draft values).
            Action::NewSession => {
                if self.new_session.is_none() {
                    self.new_session = Some(NewSessionDraft::new(
                        default_work_dir().display().to_string(),
                        default_shell_command(),
                    ));
                }
            }
        }
    }

    /// Render the new-session dialog (if open) and act on its outcome.
    fn update_new_session_dialog(&mut self, ctx: &egui::Context) {
        let Some(mut draft) = self.new_session.take() else {
            return;
        };
        match new_session::dialog(ctx, &mut draft) {
            Some(new_session::DraftAction::Submit) => {
                if let Err(error) = new_session::validate(&draft.work_dir, &draft.command) {
                    draft.error = Some(error);
                    self.new_session = Some(draft);
                    return;
                }
                let work_dir = PathBuf::from(draft.work_dir.trim());
                let (command, args) = new_session::split_command(&draft.command);
                let label = if draft.label.trim().is_empty() {
                    new_session::derive_label(&draft.command)
                } else {
                    draft.label.trim().to_owned()
                };
                self.spawn_session(ctx.clone(), work_dir, &label, &command, args);
            }
            Some(new_session::DraftAction::Cancel) => { /* drop draft */ }
            None => self.new_session = Some(draft),
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
        self.drain_hook_reports();
        self.run_detection(Instant::now());

        // Backstop: keep the detection loop (and status UI) ticking even
        // when no PTY events arrive.
        ui.ctx().request_repaint_after(REPAINT_BACKSTOP);

        let mut action: Option<Action> = None;

        egui::Panel::left("agentmux_sidebar")
            .default_size(240.0)
            .show(ui, |ui| {
                action = sidebar::show(
                    ui,
                    &self.sessions,
                    self.selected_id,
                    &mut self.collapsed_projects,
                );
            });

        egui::Panel::top("agentmux_tab_bar").show(ui, |ui| {
            action = action.or_else(|| terminal_pane::tab_bar(ui, &self.sessions, self.selected_id));
        });

        let mut central_action = None;
        egui::CentralPanel::default().show(ui, |ui| {
            let selected = self
                .selected_id
                .and_then(|id| self.sessions.get_mut(&id));
            match selected {
                Some(entry) => terminal_pane::terminal_view(
                    ui,
                    entry,
                    &self.terminal_font,
                    &self.terminal_theme,
                ),
                None => central_action = terminal_pane::empty_state(ui),
            }
        });

        self.toasts.show(ui.ctx());

        if action.is_none() {
            action = central_action;
        }
        if let Some(action) = action {
            self.apply_action(action);
        }
        self.update_new_session_dialog(ui.ctx());
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
