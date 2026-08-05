# Phase 3 — hook integration (herdr Channel C)

Agents that support hook/extension mechanisms report lifecycle states to an
agentmux-owned **HTTP loopback server**; this is the most authoritative
detection channel (the agent knows exactly what it is doing). The screen
engine (phase 2) remains the fallback when no hook is installed or an agent
has no hook support.

## Protocol

`POST http://127.0.0.1:<port>/report` with a JSON body:

```json
{ "pid": 1234, "agent": "claude", "state": "working", "message": "optional" }
```

- `pid` — the process the report refers to: claude hooks are spawned per
  event and report `$PPID` (the claude process); omp/pi extensions run inside
  the agent and report their own pid. agentmux resolves the session by
  walking ancestors to a known session `shell_pid` (a report from an agent
  launched *outside* agentmux finds no session and is dropped).
- `agent` — any known exe name or display name (case-insensitive): claude,
  omp, pi, codex, gemini, opencode, kilo, kimi, hermes.
- `state` — `working` | `idle` | `blocked` | `clear`. `clear` (SessionEnd /
  session_shutdown) releases hook authority.
- `message` — optional human text (e.g. the permission prompt), shown in the
  sidebar tooltip and appended to the "needs attention" toast.

Unknown agent/state/malformed JSON → 400; wrong method → 405; other paths →
404. Hook assets swallow every failure and never block the agent.

## State mapping

| claude event (settings.json matcher `*`) | action | mapped state |
|---|---|---|
| `UserPromptSubmit` / `PreToolUse` / `PostToolUse` | `working` | Working |
| `Notification` (permission prompts) | `blocked` | Blocked |
| `Stop` | `idle` | Idle |
| `SessionEnd` | `release` | Clear (authority released) |

The omp/pi extension maps the same way from its events
(`agent_start`/`tool_execution_start` → working; `tool_approval_requested` /
`ask` tool → blocked; `agent_end` → idle; `session_shutdown` → clear),
mirroring herdr's `herdr-agent-state.ts` minus herdr's env-var session
routing (we use PID correlation instead).

## PID semantics and arbitration

- Reports carry a PID whose ancestor chain includes the session shell. The
  session is found via `find_session_for_pid` (sysinfo parent walk, capped at
  1024 hops).
- `resolve_agent_pid` walks *up* from the reported pid to the first process
  whose cmdline matches the reported agent kind — claude may execute hooks
  via an intermediate `sh -c`, so `$PPID` can be a short-lived shell; the
  resolved claude process is what liveness tracks. Falls back to the reported
  pid.
- Authority is held while `hook_is_live`: the resolved agent pid is still a
  descendant of the session's shell. It is released when that process
  disappears from the scan or a `clear` report arrives.
- Arbitration (`hooks::arbitrate`): live hook → hook state wins over the
  screen engine; released/dead hook → screen engine as in phase 2. Hook- and
  screen-driven transitions fire the same toasts ("detected", "needs
  attention" + message, "finished"). The sidebar/tab show a `⚡` marker next
  to the state dot while a hook is authoritative, and the tooltip shows the
  hook message and report age.

## Server and port file

- `src/hooks/server.rs`: `tiny_http` bound to `127.0.0.1:0` (ephemeral), a
  background thread with a 100 ms accept poll, reports forwarded on an mpsc
  channel drained in `ui()` (same pattern as the PTY channel). `Drop` stops
  the thread and removes the port file (clean exit only; SIGKILL/SIGTERM can
  leave a stale file — harmless, the next start overwrites it and hooks just
  fail to connect).
- Port file: `~/.config/agentmux/agentmux.port` (Linux/macOS) /
  `%APPDATA%\agentmux\agentmux.port` (Windows, via `dirs::config_dir()`),
  mode 0600. Hooks read it to find the server; no port file → silent no-op.
  `AGENTMUX_PORT_FILE` overrides the path (used by the claude script).
- **Security note**: no auth token. Safety rests on loopback binding, the
  user-private port file, and same-user access — any process running as this
  user could forge reports (e.g. flip a session to blocked). Acceptable for a
  local dev tool; a token is the obvious hardening if this ever listens
  wider. The endpoint validates shapes but not authenticity.

## Installer (`agentmux --install-hooks` / `--uninstall-hooks`)

Runs without starting the GUI (`std::env::args` parse in `main.rs`, no
clap).

- **claude**: writes `~/.claude/hooks/agentmux-claude-hook.sh` (executable)
  and merges one `{matcher: "*", hooks: [{type: "command", command: "<hook>
  <action>", timeout: 10}]}` entry per event into `~/.claude/settings.json`
  (`CLAUDE_CONFIG_DIR` overrides). Non-destructive: existing hooks/entries
  and unrelated keys are preserved; a timestamped backup
  (`settings.json.agentmux-bak-<ts>`) is written before any modification;
  malformed input is replaced with a minimal file (original kept in the
  backup) with a printed warning; reinstall is an idempotent no-op.
  Uninstall removes only entries whose command starts with our hook path,
  drops emptied events, and preserves everything else.
- **omp**: copies `assets/hooks/agentmux-omp-extension.ts` into omp's
  extension dir (`$PI_CODING_AGENT_DIR/extensions`, else
  `~/.omp/agent/extensions`, honoring `PI_CONFIG_DIR`), which omp auto-loads
  — activation without any env-var session routing, exactly as herdr does
  (`install_omp` in herdr's `targets.rs`). Uninstall removes the file.
- Prints exactly what was changed (paths, backups, idempotent no-ops).

## Debug log

`AGENTMUX_DEBUG_LOG=<path>` appends one line per detection transition:
`session N: agent=X state=Y source=hook|screen` (source is `hook` while a
live hook is authoritative). This is the headless equivalent of watching the
UI and is what the scripted e2e asserts on.

## Limitations

- Claude hooks only fire on claude events: a long tool call with no
  intermediate events stays `working` (correct, just coarse); permission
  prompts surface as `Notification`.
- Hook configuration is global to claude (`~/.claude/settings.json`) — agents
  launched outside agentmux fire hooks too; their reports find no session and
  are dropped.
- The claude hook asset is POSIX sh + curl (Windows needs a future .ps1
  variant; the omp extension already handles win32 paths).
- `clear`/release does not scrub `detection` immediately — the screen engine
  re-derives it (usually `None` once the agent process is gone).
- A backgrounded agent whose hook reports keep arriving still holds
  authority (the report pid stays a shell descendant) — same caveat as the
  process scan.

## Verification

`cargo test` (28 tests): report JSON parse (valid + 8 invalid shapes),
server HTTP behavior (400/200, real socket round-trips), PID correlation
against real spawned process trees (session found from a grandchild; foreign
process not found; agent-kind resolution up the chain with fallback),
liveness flip on kill, arbitration precedence (live hook overrides screen;
dead hook releases), settings merge (foreign hooks preserved, idempotent
double-install, malformed fallback, surgical uninstall), port file roundtrip
with 0600 mode + cleanup.

Scripted e2e (run with a fixture `$SHELL` that backgrounds a `sleep` as the
"agent" then execs bash; `AGENTMUX_DEBUG_LOG` set; real curl POSTs):

```
session 0: agent=Claude Code state=blocked source=hook   # POST blocked
session 0: agent=Claude Code state=idle source=hook      # POST idle
session 0: agent=Claude Code state=working source=hook   # POST working
session 0: agent=none state=none source=screen           # kill sleep → release
```
