# herdr architecture — technical report for a native-GUI reimplementation

**Repo:** `/home/catitw/mypros/herdr` (v0.8.0, `Cargo.toml:2`, Apache-2.0)
**Date of analysis:** 2026-08-05
**Method:** read-only source inspection; every claim cites `path:line`. Nothing below was modified.

---

## 0. Reality check: what herdr actually is (read this first)

The task description ("TUI that launches each hermes tool per work directory, left pane = workdir + tool, right pane = tabs") does **not** match the codebase. herdr is a full **terminal multiplexer with a persistent background server** (~160k lines of Rust):

- **herdr never launches the agent tool.** Each pane spawns an interactive **shell** (default `$SHELL`; `/bin/sh` fallback; `powershell.exe` on Windows). The user types `claude`/`omp`/`hermes` inside that shell (`src/pane.rs:1463` `pane_shell_command_builder`; `src/pane.rs:1319-1325` `default_pane_shell` per platform). There is no "tool command" configuration anywhere.
- **The tool is detected, not configured.** herdr watches the pane and infers (a) *which* agent is running and (b) its state, via three independent channels (§3): terminal-screen pattern matching, OS process/foreground-group inspection, and agent-side hook scripts that report state over a socket.
- **Client/server split.** A headless server owns all PTYs (`src/server/headless.rs`), persists sessions, and re-renders the TUI; the terminal client (`src/client/mod.rs`) is a *thin* client that receives already-rendered frames (`src/protocol/wire.rs`) over a Unix socket / Windows named pipe. Detaching (`ctrl+b q`) kills the client, not the agents.
- The "left pane = work dir + tool name" idea maps to the **sidebar** (workspace list on top, agent status panel below); "right pane tabs" maps to a **tab bar + BSP-split pane surface** (§4).

For a GUI reimplementation the biggest architectural insight is §3: the per-pane status is produced by a *generic terminal-pattern engine plus process polling plus agent hooks*, entirely decoupled from how panes are spawned. A GUI could reuse the same detection pipeline against its own PTY/terminal layers.

---

## 1. Process model

### 1.1 PTY crate: `portable-pty` (patched, vendored)

`Cargo.toml:29` — `portable-pty = "=0.9.0"`, and `Cargo.toml:40-41` redirects it to a local patch:

```toml
[patch.crates-io]
portable-pty = { path = "vendor/portable-pty" }
```

herdr does **not** use `vte` or a pure-Rust terminal emulator. The escape-sequence engine is a **vendored C library: libghostty-vt** (`vendor/libghostty-vt/`, wrapped by FFI in `src/ghostty/mod.rs`, compiled by `build.rs`). `src/ghostty/mod.rs:24` re-exports the generated bindings (`pub use bindings as ffi`).

### 1.2 Spawn path (Unix)

Every pane is a PTY pair created with `native_pty_system().openpty(...)`, then the master fd is duplicated for an I/O actor thread:

- `src/pty/backend/unix.rs:12-35` — `spawn_with_portable_pty`:
  ```rust
  let pty_system = native_pty_system();
  let pair = pty_system.openpty(PtySize { rows, cols, pixel_width: 0, pixel_height: 0 })?;
  let master_fd = pair.master.as_raw_fd()...;
  let actor_fd = fd::duplicate_cloexec_fd(master_fd)?;   // OwnedFd for the actor thread
  let child = pair.slave.spawn_command(cmd)?;
  drop(pair);
  ```
  On Windows (`src/pty/backend.rs:10-40`) the `Box<dyn MasterPty>` is kept instead of an fd.
- `src/pane.rs:1945` `PaneRuntime::spawn_command_builder` is the common constructor for all pane kinds:
  - `PaneRuntime::spawn` (`src/pane.rs:1635`) — plain shell pane,
  - `spawn_shell_command` (`src/pane.rs:1712`) — shell running a command string,
  - `spawn_argv_command` (`src/pane.rs:1750`) — raw argv (used for agent resume, e.g. `pi --session …` via `src/agent_resume.rs:151-157`),
  - each builds a `portable_pty::CommandBuilder`, sets `TERM=xterm-256color`/`COLORTERM=truecolor` (`src/pane.rs:57-60` `apply_pane_terminal_env`), applies launch env (`apply_pane_launch_env`, `src/pane.rs:112`), then calls `spawn_with_portable_pty` at `src/pane.rs:1982`.
- Child exit is a blocking `child.wait()` task that emits `AppEvent::PaneDied` (`src/pane.rs:1985-2008`).

### 1.3 PTY I/O: a dedicated actor thread, not tokio

`src/pty/mod.rs` declares `actor`, `backend`, and (unix) `fd` submodules. The unix actor (`src/pty/actor/unix.rs`, 1463 lines) is a raw-fd poll loop: `PtyIoActorConfig` (`src/pty/actor/unix.rs:79-85`) carries `master_fd: OwnedFd`, an `on_read: ReadCallback`, and a wake fd (`src/pty/fd.rs`). Reads are pushed into the embedded terminal emulator via `terminal.process_pty_bytes(...)` (`src/pane.rs:2064-2075`, implementation `src/pane/terminal.rs:1108`); the callback also bumps a `detection_content_seq` counter when agent detection is enabled (`src/pane.rs:2069-2071`). User keystrokes go back through `PtyIoActorHandle::write_user_input` (`src/pty/actor/unix.rs:99-148`).

### 1.4 Terminal emulator

`src/ghostty/mod.rs` wraps libghostty-vt C FFI: `Terminal::new(cols, rows, scrollback_limit_bytes)` is created at `src/pane.rs:1971-1978`, wrapped by `GhosttyPaneTerminal` (`src/pane/terminal.rs`), then `Arc<PaneTerminal>` (`src/pane.rs:1980`). Rendering is ratatui-side: the emulator exposes a render state (row/cell iterators, `src/ghostty/mod.rs` `RenderState`/`RowIter`/`RowCells`), and `TerminalRuntime` (`src/terminal/runtime.rs:16-17`, a newtype over `PaneRuntime`) is what the UI actually renders. **A GUI rewrite must either port this C emulator or vendor a replacement (e.g. alacritty's `vte`+`alacritty_terminal`); the detection pipeline (§3) only needs the *text* of the visible screen, so it is emulator-independent.**

### 1.5 Persistence

Sessions are persisted so agents survive client detach: `src/persist/` (snapshot/restore, `persist/snapshot.rs`, `persist/restore.rs`), with pane history under `src/server/headless.rs` and handoff of PTY fds between processes (`src/pty/actor/unix.rs` `BeginHandoff`/`DuplicateForHandoff`, `src/server/handoff.rs`).

---

## 2. Configuration

### 2.1 File and format

TOML at `~/.config/herdr/config.toml` (or `$XDG_CONFIG_HOME/herdr/config.toml`):

- `src/config/io.rs:29-35` — `config_dir()` honors `XDG_CONFIG_HOME`, else `$HOME/.config/herdr` on Unix (`src/config/io.rs:61-67`), `%APPDATA%\herdr` on Windows (`src/config/io.rs:44-59`). Overridable via `HERDR_CONFIG_PATH` (`src/config.rs:52`).
- `src/config/io.rs:168-172` — `config_path() = config_dir().join("config.toml")`.
- Parsed with `toml` + `serde`, unknown keys diagnosed via `serde_ignored` (`src/config/io.rs` `deserialize_with_ignored`), sections load independently (`load_live_section`), invalid sections reported but do not abort.

### 2.2 The model

`src/config/model.rs:313-325`:

```rust
pub struct Config {
    pub onboarding: Option<bool>,
    pub theme: ThemeConfig,
    pub terminal: TerminalConfig,
    pub session: SessionConfig,
    pub update: UpdateConfig,
    pub keys: KeysConfig,
    pub ui: UiConfig,
    pub worktrees: WorktreesConfig,
    pub advanced: AdvancedConfig,
    pub experimental: ExperimentalConfig,
    pub remote: RemoteConfig,
}
```

Relevant sub-structs:

- `TerminalConfig` (`src/config/model.rs:258-266`): `default_shell: String` (empty = `$SHELL` then `/bin/sh`), `shell_mode: ShellModeConfig` (`auto`/`login`/`non_login`), `new_cwd: NewTerminalCwdConfig` (`follow`/`home`/`current`/fixed path) — **this is the closest thing to "which command runs in a pane": a shell, not an agent tool.**
- `SessionConfig` (`src/config/model.rs:268-274`): `resume_agents_on_restore`.
- `UiConfig` (`src/config/model.rs:809-867`): sidebar widths, tab bar position (`top`/`bottom`), status indicator style (`dots`/`symbols`), sound, toasts, accent color.
- `KeysConfig` (`src/config/model.rs:334-450`): prefix key + full action binding table.
- `WorktreesConfig` (`src/config/model.rs:792-797`): root directory for managed git worktrees.

A complete annotated sample ships as `DEFAULT_CONFIG` in `src/main.rs:109-263` (e.g. `[terminal] default_shell = ""`, `shell_mode = "auto"`, `new_cwd = "follow"`; `[ui] sidebar_width = 26` …). There is **no** `[agents]`/tool section — nothing in the config names a tool, a command line, or a workdir-to-tool mapping.

### 2.3 Work directories and grouping

- A **workspace** *is* a directory: `Workspace` has `identity_cwd: PathBuf` plus git-derived metadata (`src/workspace.rs:171-202`, struct at `:171`).
- Startup: the server seeds one workspace from its launch cwd via `seed_startup_workspace_if_empty` / `take_startup_cwd` (`src/server/headless.rs:4759-4793`, env `STARTUP_CWD_ENV_VAR` from `src/server/autodetect.rs`); interactive creation: `create_workspace_with_launch_env` (`src/app/creation.rs:240`), which builds `Workspace::new_with_extra_env(initial_cwd, …)` (`src/app/creation.rs:247`).
- Grouping: workspaces contain tabs; tabs contain panes; `worktree_space: Option<WorktreeSpaceMembership>` links a workspace to a git-worktree grouping (`src/workspace.rs:187`, struct `src/workspace.rs:35`); workspaces are ordered and numbered, with public ids like `w1`/`w1:p1`/`w1:t1` (`src/workspace.rs:63-76` `public_*_id_for_number`).
- Per-pane launch overrides exist only at the API/CLI level (e.g. `herdr terminal`, `herdr pane` commands; `src/cli/pane.rs`), not in the config file.

---

## 3. Status & notification (the critical part)

herdr detects per-pane state through **three independent channels**, arbitrated in `TerminalState`:

### 3.1 The status enum

`src/detect/mod.rs:11-19`:

```rust
pub enum AgentState {
    /// Agent finished, prompt visible, nothing happening.
    Idle,
    /// Agent is actively working/processing.
    Working,
    /// Agent needs human input and is blocked on a response.
    Blocked,
    /// Plain shell or unrecognized program.
    Unknown,
}
```

Carried with confidence metadata in `AgentDetection` (`src/detect/mod.rs:24-33`): `state`, `skip_state_update`, `visible_idle`, `visible_blocker`, `visible_working`.

### 3.2 Channel A — terminal-screen pattern matching (primary, 19 agents)

The engine is regex/contains matching over regions of the *live screen text* (not a protocol, not stdout parsing — it matches whatever the agent paints).

- Entry: `detect_agent_with_osc` (`src/detect/mod.rs:254-277`) → `manifest::detect_with_osc` (`src/detect/manifest.rs:334`).
- Per-agent **manifests** are bundled TOML files, `src/detect/manifests/*.toml` (19 files: amp, antigravity, claude, cline, codex, cursor, devin, droid, gemini, github-copilot, grok, hermes, kilo, kimi, kiro, maki, opencode, pi, qodercli), registered at `src/detect/manifest.rs:239-259` (`BUNDLED_MANIFESTS`, `include_str!`). Remote update support: `src/detect/manifest_update.rs`.
- Manifest schema (`src/detect/manifest.rs:140-203`): `AgentManifest { id, version, min_engine_version, aliases, rules }`; each `ManifestRule` has `id`, `state`, `priority`, `region`, `visible_idle/blocker/working`, `skip_state_update`, and gates (`all`/`any`/`not`, each with `contains`/`regex`/`line_regex`). Regions are named slices of the tail: `"bottom_non_empty_lines(14)"`, `"osc_title"`, `"after_last_prompt_marker"`, etc. (`src/detect/manifest.rs` region helpers ~`:874-1020`).
- Evaluation: `evaluate_loaded_manifest` (`src/detect/manifest.rs:414-495`) — evaluate every rule, keep the **highest-priority match**; if none matched and the agent is known, fall back to `Idle` (`fallback_explain`, `src/detect/manifest.rs:497-557`, reason constant `DEFAULT_KNOWN_AGENT_IDLE_FALLBACK` at `:14`).

Real example — `src/detect/manifests/hermes.toml` (verbatim, abridged):

```toml
id = "hermes"
version = "2026.07.24.1"
aliases = ["hermes-agent"]

[[rules]]
id = "osc_title_blocked"
state = "blocked"
priority = 1100
region = "osc_title"
visible_blocker = true
regex = ['^⚠[\u{fe0e}\u{fe0f}]?(?:\s|$)']

[[rules]]
id = "clarification_prompt"
state = "blocked"
priority = 900
region = "bottom_non_empty_lines(14)"
visible_blocker = true
any = [
  { contains = ["hermes needs your"] },
  { line_regex = ['^\s*ask\s+\S'] },
]
all = [
  { any = [{ contains = ["enter confirm"] }, { contains = ["↑/↓ to select"] }] },
]
```

I.e. "if the bottom 14 non-empty lines contain a clarification prompt and an enter-to-confirm hint → Blocked".

**How the screen text is sampled** (the per-pane detection task, `src/pane.rs:2121-2478`):

- Loop cadence: 500 ms while agent unidentified, 300 ms while identified, 50 ms during release (`src/pane.rs:2093-2096`).
- **Process probe**: reads the pane's foreground process group (`detect::foreground_process_group_id`, `src/platform/linux.rs:325`, `tcgetpgrp` on the PTY fd), scans the foreground job (`foreground_job`, `src/platform/linux.rs:130`, /proc group scan at `:142`), identifies the agent from process name/argv (`detect::identify_agent` `src/detect/mod.rs:237`; `identify_agent_in_job` `:242-272`; argv peeling for `node`/`python`/shell wrappers in `normalized_process_name` `:322-347`).
- **Content probe**: `terminal.detection_text()` (`src/pane/terminal.rs:404`, impl `:1824`, ghostty `detection_text`) returns the live bottom-of-buffer text; bytes bump `detection_content_seq` (`observe_detection_content_change`, `src/pane/agent_detection.rs:319`) so the loop re-scans only when output changed.
- **OSC channels**: `AgentOscStateTracker` (`src/pane/osc.rs:474-489`) captures OSC 0/2 → title, OSC 9 → progress (plus a debug channel on OSC 21337); fed from `process_pty_bytes` (`src/pane/terminal.rs:1151`), read by the loop as `terminal.agent_osc_title()` / `agent_osc_progress()` (`src/pane/terminal.rs:488-495`). OSC title/progress feed manifest regions `"osc_title"`/`"osc_progress"` and give hermes-style agents (`⚠`/`⏳`/`✓` prefixed titles) high-priority signal.
- Publish: `detection_update_for_publish_with_osc` (`src/pane/agent_detection.rs:298`) → `apply_agent_detection_publish_update` (`src/pane.rs:180-215`) → `AppEvent` → `TerminalState::set_detected_state_with_screen_signals_at` (`src/terminal/state.rs:277`).

### 3.3 Channel B — process exit / foreground-shell tracking

- `child.wait()` → `AppEvent::PaneDied` (`src/pane.rs:1985-2008`).
- Within the detection loop, `process_exited` is derived from foreground-job probes: `pending_foreground_shell_clear && agent.is_some() && !foreground_shell_exit_reported` (`src/pane.rs:2257-2259`); exit clears hook authority and releases the agent session (`src/terminal/state.rs:277-363`, `agent_released` logic at `:304-316`).

### 3.4 Channel C — agent-side hooks (full-lifecycle IPC)

For agents that support it, herdr **installs a hook script into the agent itself**, which reports state over herdr's socket — the most reliable channel (hook-authoritative while live):

- Installable integrations per agent: `src/integration/mod.rs` — e.g. omp extension `herdr-omp-agent-state.ts` (`:27`), pi `herdr-agent-state.ts` (`:24`), Claude/Codex/Kimi/Droid/Qodercli shell or PowerShell hooks (`:30-37`), hermes Python plugin (`:165-168`), opencode/kilo JS plugins (`:117-126`). Install logic per target: `src/integration/targets.rs` (`install_*` functions).
- The omp asset (`src/integration/assets/omp/herdr-agent-state.ts`) is a Node extension injected into omp via env (`HERDR_ENV=1`, `HERDR_SOCKET_PATH`, `HERDR_PANE_ID`; `:12-16`), reporting `sendState("working" | "blocked" | "idle")` (`:163`) over the socket with `source = "herdr:omp"` (`:16`). Claude uses its `Stop`/`PreToolUse`/`PostToolUse` hooks; Kimi installs hook events like `^AskUserQuestion$` (`src/integration/mod.rs:90-96`).
- Server side: hook reports land in `TerminalState.hook_authority: Option<HookAuthority>` (`src/terminal/state.rs:119-141`, struct `:18-24`: `source`, `agent_label`, `state`, `message`, `reported_at`, `session_ref`). Only "official" sources get full authority: `full_lifecycle_hook_authority` matches `("herdr:pi","pi") | ("herdr:omp","omp") | ("herdr:mastracode","mastracode") | ("herdr:opencode","opencode") | ("herdr:kilo","kilo") | ("herdr:kimi","kimi")` (`src/detect/mod.rs:283-294`); hermes/antigravity are *session-identity-only* (`session_identity_only_integration`, `src/detect/mod.rs:295-301`).
- While a live hook authority exists, screen detection is suppressed: `should_ignore_detected_state_under_full_lifecycle_hook` (`src/terminal/state.rs:737-745`).

### 3.5 Arbitration (the single source of truth)

`recompute_effective_state` (`src/terminal/state.rs:2006-2046`):

```rust
let state = if self.visible_blocker_overrides_hook() {
    AgentState::Blocked
} else {
    self.hook_authority
        .as_ref()
        .filter(|authority| self.hook_authority_is_effective(authority))
        .map(|authority| authority.state)
        .unwrap_or(self.fallback_state)
};
```

Order: **hook authority (agent self-report) > screen fallback**, with a visible-blocker override that can force `Blocked`. `fallback_state` is what Channel A published (`set_detected_state_with_screen_signals_at`, `src/terminal/state.rs:277-363`). Changes emit `EffectiveStateChange` (`src/terminal/state.rs:78-91`) → UI + notifications.

### 3.6 Notifications (what the UI does with state)

- Toast mapping: `Blocked → ToastKind::NeedsAttention`, `Idle` after a non-idle state (`is_completion_transition`) → `ToastKind::Finished` (`src/app/actions.rs:145-173`).
- `PendingAgentNotification { pane_id, workspace_id, agent_label, … }` queued in `AppState.pending_agent_notifications` (`src/app/state.rs:1506`; push site `src/app/actions.rs:3145-3150`), shown as toasts (title like `"pi needs attention"`, `src/app/actions.rs:5028-5032` test), optionally with sound (`src/sound.rs`, `Sound::Request`/`Sound::Done`, `src/app/actions.rs:209-213`) and terminal OSC 9/99 notifications (`src/terminal_notify.rs:8-14`, backends Ghostty/iTerm2/Kitty/WezTerm via `TERM_PROGRAM`).
- Sidebar "seen" flag: `PaneState.seen` (`src/pane/state.rs:6-11`) — false = "Done" (agent finished while user was elsewhere).

---

## 4. Layout model

### 4.1 Top-level geometry

`compute_view_internal` (`src/ui.rs:215-309`) splits the screen:

```rust
let [sidebar_area, main_area] =
    Layout::horizontal([Constraint::Length(sidebar_w), Constraint::Min(1)]).areas(area);  // ui.rs:237-238
```

- Sidebar width: clamped `app.sidebar_width` between `sidebar_min_width`/`sidebar_max_width` (`src/ui.rs:232-235`), collapsed to a 4-column rail (`COLLAPSED_WIDTH`, `src/ui.rs:103`).
- Main area: `[tab_bar (Length 1), terminal_area (Min 1)]` stacked vertically, tab bar position per config (`src/ui.rs:196-209`).
- Terminal area → tab surface: BSP-tiled panes via `compute_tab_surface` (`src/ui/tab_surface.rs:15-20`).

### 4.2 Sidebar (the "left pane")

Two sections (split at `sidebar_section_split: f32`, heights computed by `sidebar_section_heights`, `src/ui/sidebar.rs:42-61`):
- **Workspace list** — one row per workspace: number, git branch/ahead-behind label, collapsed groups (`workspace_list_entries` etc., `src/ui/sidebar.rs`).
- **Agent panel** — one row per agent-bearing pane: `AgentPanelEntry` (`src/ui/sidebar.rs:23-32`):

```rust
pub(crate) struct AgentPanelEntry {
    pub ws_idx: usize, pub tab_idx: usize, pub pane_id: crate::layout::PaneId,
    pub primary_label: String, pub primary_tab_label: Option<String>, pub pane_label: Option<String>,
    pub terminal_title: Option<String>, pub terminal_title_stripped: Option<String>,
    pub agent_label: Option<String>, pub agent_kind_label: Option<String>,
    pub agent: Option<crate::detect::Agent>,
    pub state: AgentState,
    pub seen: bool,
    pub last_agent_state_change_seq: Option<u64>,
    pub state_labels: std::collections::HashMap<String, String>,
    pub tokens: std::collections::HashMap<String, String>,
}
```

Collected from `app.workspaces` + `app.terminals` by `agent_panel_entries` (`src/ui/sidebar.rs:112`). Status visuals: `state_icon_symbol`/`state_label` (`src/ui/status.rs:196-237`) — dots `●` or symbols per `StatusIndicatorStyle`.

### 4.3 Tabs and panes (the "right pane")

- Tab bar: `TabBarView` with per-tab hit areas, scroll, new-tab button (`src/ui/tabs.rs:23-30`), widths from `tab_chrome_label` (`src/ui/tabs.rs:36-44`).
- Tab: `src/workspace/tab.rs:38-47`:

```rust
pub struct Tab {
    pub custom_name: Option<String>,
    pub number: usize,
    pub root_pane: PaneId,
    pub layout: TileLayout,
    pub panes: HashMap<PaneId, PaneState>,
    pub zoomed: bool,
    pub events: mpsc::Sender<AppEvent>,
    ...
}
```

- Pane tree: BSP layout, `TileLayout` with `Node::Split`/`Node::Leaf` (`src/layout.rs:73`, `src/layout.rs:84`; `src/layout.rs:11-13` for `PaneId(u32)`); `PaneState` is only viewport glue (`src/pane/state.rs:6-11`: `attached_terminal_id`, `seen`).
- Workspace: `src/workspace.rs:171-202` (`id`, `custom_name`, `identity_cwd`, `tabs: Vec<Tab>`, `active_tab: usize`, public pane/tab numbering).
- App state root: `AppState` (`src/app/state.rs:1414-1523`) — `terminals: HashMap<TerminalId, TerminalState>`, `workspaces: Vec<Workspace>`, `active: Option<usize>`, `mode: Mode`, sidebar/section fields, `pending_agent_notifications`, toasts, copy mode, navigator, etc. `Workspace` derefs to its active tab (`src/workspace.rs:204-212`).
- Per-terminal state: `TerminalState` (`src/terminal/state.rs:119-141`) holds `cwd`, `detected_agent`, `fallback_state`, `hook_authority`, `agent_metadata`, `terminal_title`, `manual_label`, `agent_name`, `state`, `launch_argv`, `respawn_shell_on_exit`, session resume plan. **This is the struct a GUI port needs to mirror; it is deliberately decoupled from pane/view state.**

### 4.4 Client/server rendering

The attached client never sees `AppState`: the server renders the ratatui frame and ships it over the wire as `FrameData`/`ServerMessage` (`src/protocol/wire.rs:506`, `:640`, bincode framing `write_message` at `:215-236`, `PROTOCOL_VERSION = 19` at `:16`). Client input is semantic (`ClientInputEvent`, `src/protocol/wire.rs:341`). A GUI rewrite has two options: (a) reimplement the whole server (AppState + TerminalState + detection) in the GUI process, or (b) keep herdr's server and write a GUI *client* against the socket protocol.

---

## 5. Tool registry

There is **no user-extensible command registry**; tools are a **hardcoded typed enum + bundled pattern manifests + installable hook integrations**:

- `Agent` enum, 21 variants (`Pi, Claude, Codex, Gemini, Cursor, Devin, Antigravity, Cline, Omp, Mastracode, OpenCode, GithubCopilot, Kimi, Kiro, Droid, Amp, Grok, Hermes, Kilo, Qodercli, Maki`), `src/detect/mod.rs:43-57`; canonical labels `agent_label` (`:109-121`), CLI executable names `interactive_agent_executable` (`:123-157`), alias→enum `lookup_agent` (`:206-234`, e.g. `"hermes" | "hermes-agent" => Agent::Hermes`, `"omp" => Agent::Omp`).
- Distinction between agents with **screen manifests** (19 agents, `SCREEN_MANIFEST_AGENTS`, `src/detect/mod.rs:60-82`) vs **hook-only** (omp, mastracode have *no* bundled screen manifest; their state comes solely from hooks — see `src/detect/manifests/` listing, no `omp.toml`/`mastracode.toml`).
- Integration layer (`src/integration/mod.rs`, `src/integration/targets.rs`, `src/integration/registry.rs`) implements per-tool install/uninstall/status/versioning (`IntegrationTarget` schema enum; 16 installable targets per `integration_specs`, `src/integration/registry.rs:201-209`).
- **Adding a new tool = new enum variant + bundled manifest TOML + optional hook asset** (plus `agent_resume.rs` resume-arg mapping for pi/omp-style session resume).

---

## 6. Cross-platform

What a rewrite must handle, by layer:

| Layer | Unix (linux/macos) | Windows |
|---|---|---|
| PTY | `portable-pty`; raw master fd handed to actor thread (`src/pty/backend/unix.rs:12-35`) | `Box<dyn MasterPty>` kept; separate windows actor (`src/pty/backend.rs:10-40`, `src/pty/actor.rs` windows mod) |
| Process/foreground-job scanning | Linux: `tcgetpgrp` + `/proc` group scan (`src/platform/linux.rs:130-163`, `:325-336`); macOS: `src/platform/macos.rs` | `wmi` + `windows-sys` (`Cargo.toml:45-66`; `src/platform/windows.rs`, 2918 lines); no foreground-group model — Windows uses different detection heuristics (`src/detect/mod.rs` `windows_cmd_arg_agent_name`/`powershell_arg_agent_name` argv peeling, `:365-420`) |
| Shells | `/bin/sh` default; login-shell resolution via `resolve_shell_for_login_mode` (`src/pane.rs:1319-1325`, `:1394`) | `powershell.exe` default (`src/pane.rs:1324-1325`); cwd reported via wrapped prompt emitting OSC 9;9 (`WINDOWS_POWERSHELL_SHELL_INTEGRATION_COMMAND`, `src/pane.rs:1439`); recent-dir fallback `src/pane/terminal/windows_recent_fallback.rs` |
| IPC | Unix domain sockets (`interprocess`, `src/ipc.rs:29-36`) | Windows named pipes (`src/ipc.rs:38-45`, `to_ns_name`) |
| Terminal I/O | crossterm + kitty keyboard protocol push/pop (`src/main.rs:26-42`, `src/terminal_modes.rs:9-33`) | crossterm win32 input mode, VTI (`src/client/input/windows_vti.rs`, `src/client/mod.rs` windows fns) |
| Misc | SIGWINCH handling (`src/platform/mod.rs:49-90`), daemon detach via setsid (`:72-82`) | size polling (`take_terminal_resize_signal` no-op, `src/platform/mod.rs:85-90`) |

The platform layer is deliberately centralized behind `src/platform/mod.rs` (types `ForegroundProcess`/`ForegroundJob` at `:12-20`, per-OS submodules at `:117-131`) so a rewrite only needs to reimplement that boundary.

---

## 7. Dependencies (`Cargo.toml`)

| Crate | Purpose |
|---|---|
| `portable-pty =0.9.0` (patched→`vendor/portable-pty`) | PTY creation/spawn, cross-platform (`Cargo.toml:29,40-41`) |
| `ratatui 0.30` | TUI rendering (`Cargo.toml:33`) |
| `crossterm 0.29` | host terminal raw mode, events, mouse, keyboard protocols |
| `tokio 1` (rt-multi-thread, macros, sync, time) | async app/server event loops |
| `bincode 2` + `serde` + `serde_json` | wire protocol serialization + config + API |
| `serde_ignored` | unknown-key config diagnostics (`src/config/io.rs`) |
| `interprocess 2.4.2` | Unix sockets / Windows named pipes (`src/ipc.rs:10-11`) |
| `regex` | manifest pattern engine (`src/detect/manifest.rs`) |
| `toml 0.8` | config parsing |
| `jsonc-parser` | editing agent JSON configs (Claude settings, `src/integration/claude_settings.rs`) |
| `base64` / `bytes` / `png` | wire payloads, kitty-graphics PNG decode (`src/ghostty/mod.rs` decode trampoline) |
| `sha2` | checksums (`src/checksum.rs`) |
| `libc` / `ctrlc` | Unix signals, SIGINT handling |
| `clap` + `clap_complete` | CLI + shell completions |
| `unicode-width` | text metrics for ratatui layout |
| `schemars` | JSON-schema generation for the socket API (`docs/next/api/herdr-api.schema.json`) |
| `tracing` + `tracing-subscriber` | structured logging |
| Windows-only: `wmi`, `windows-sys` | process enumeration, job objects, console, IME (`Cargo.toml:45-66`) |
| Vendored C: `libghostty-vt` (via `build.rs`) | the terminal emulator itself (`vendor/libghostty-vt/`) |

Notably **absent**: `vte`, any electron/gui framework, any `notify` crate (config reload is manual), any DB (persistence is file/socket-based JSON+bincode).

---

## 8. Reimplementation checklist (what "core behavior" actually requires)

1. **PTY per pane** with a real terminal emulator (port libghostty-vt or substitute), shell-launched like herdr (`TERM=xterm-256color`).
2. **The three-channel state machine**: (A) periodic screen-tail regex engine over per-agent manifest TOMLs (regions `bottom_non_empty_lines(n)`/`osc_title`/…, priority-ordered rules, Idle fallback); (B) OS foreground-process polling (tcgetpgrp+/proc or equivalent per-OS) with argv-based agent identification; (C) agent hook scripts reporting state over IPC, treated as authority while live. Arbitration exactly as `recompute_effective_state` (`src/terminal/state.rs:2006-2046`).
3. **`AgentState` 4-state enum + `seen` flag** for the "done/blocked" UX, toasts for `Blocked→needs attention` / `Idle→finished` (`src/app/actions.rs:145-213`).
4. **Workspace = cwd** with tabs (`Vec<Tab>`, active tab index) and BSP-splittable panes; sidebar + agent panel as a *view* over `workspaces + terminals` (`src/app/state.rs:1414`, `src/ui/sidebar.rs:23-32`).
5. **Persistent server + thin client** if multi-terminal attach/detach is in scope; otherwise the whole AppState/TerminalState stack can run in-process (the structs are deliberately UI-agnostic).
6. **Cross-platform boundary**: replicate `src/platform/mod.rs` (process/fg-job/cwd per OS) and the socket transport (`src/ipc.rs`).
