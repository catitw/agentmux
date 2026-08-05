# Phase 2 — agent detection engine

Auto-detects which hermes coding-agent runs in each session and what it is
doing, shown in the sidebar / tab bar with toasts on key transitions.

## Channels used

Everything rides on egui_term's public API (pinned at rev `31bbc7a`), per
`docs/research/detection-feasibility.md`:

| Channel | Source | Used for |
|---|---|---|
| Process scan | `TerminalBackend::pty_id()` (shell PID) + a shared `sysinfo` snapshot (`refresh_processes_specifics(All, true, everything())` — plain `refresh_processes` omits cmdlines) | **Identification**: walk shell descendants via parent links, match exe/argv against the agent table |
| Screen text | `backend.sync().grid` → `bottom_non_empty_lines(14)` (walk up from `bottommost_line()`, skip `is_clear()` rows) | **Status**: rule engine regions |
| OSC title | `PtyEvent::Title`/`ResetTitle` (already tracked as `terminal_title`) | **Status**: rule engine title region; agents emit braille-spinner / ✳ / π / 4;N titles |
| `PtyEvent::Wakeup` | fires after every parsed output batch | event-driven rescan dirty flags |
| `PtyEvent::ChildExit`/`Exit` | session death only (shell exited) | process layer (`SessionStatus`), unchanged |

Cadence: process scan every 500 ms (shared across sessions), per-session
screen clone throttled to ≥ 250 ms, `ctx.request_repaint_after(300ms)`
backstop in `ui()`.

## Rule table

`src/detect/engine.rs` — herdr-manifest style: `{ state, priority, region,
contains | any_contains | regex | line_regex | not_contains }`, highest
priority wins, known agent + no match → `Idle`. Text needles are matched
case-insensitively (real UIs mix case, e.g. claude's "Esc to cancel").

**Common (all agents)** — calibrated against live captures:

- title contains braille `[\u2800-\u28FF]` → **Working** (1100) — real:
  claude `⠂ Claude Code`, omp `π ⠋ agentmux-cap`, kimi ` ⠼ working...`
- title `^4;3` (OSC-4 progress, leaks through as Title events) → **Working** (1050) — real on all three
- title `^4;0` → **Idle** (250) — real on all three
- bottom `esc to cancel` + (`enter to confirm` | `enter to select`) → **Blocked** (980) — real (claude trust prompt); herdr-claude 980
- bottom `do you want to proceed?` → **Blocked** (920) — herdr-claude, inferred for others
- bottom `waiting for permission` → **Blocked** (800) — herdr

**Per agent**:

- **Claude Code** (real: v2.1.220): `Quick safety check` → Blocked 970
  (trust prompt); line `^\s*❯` input box → Idle 950 (excluded when a prompt
  form is visible — the trust prompt renders a `❯`-marked option list);
  `Thought for ` / `Nucleating` / `Worked for` / `✢` → Working 900;
  title `^✳` → Idle 250.
- **omp** (real: pi-coding-agent): title `^π\s*>` → Idle 900; title
  `Oh My Pi: Complete` → Idle 850; screen `Working...` → Working 900
  (herdr pi manifest, inferred for omp).
- **Kimi Code** (real): `working...` → Working 950; prompt box line
  `^\s*│\s*>\s` → Idle 950; title `Kimi Code` → Idle 800.
- **hermes / OpenCode / Kilo / Gemini / Codex / pi**: rules transcribed from
  herdr's bundled manifests (⚠/⏳ titles, `△ Permission required`,
  `esc dismiss`, `esc to interrupt`, `│ Apply this change`, `should codex`…)
  — **inferred**, no binary / no TUI on this machine (codex exists but has
  no usable TUI without configuration).

## Agent table

`src/detect/mod.rs` — `AGENTS: &[AgentDef]` with `exe_names` (exact basename:
`claude`, `omp`, `codex`, …) and `wrapped_contains` (substrings for
node/bun/python wrapper argv: `claude-code`, `pi-coding-agent`, `kimi-code`,
…). `src/detect/process.rs::match_cmdline` does direct + wrapper-peeled
matching (verified against real argv of this machine's claude/omp: claude is
a Bun-compiled ELF, omp is `bun .../.bun/bin/omp`). omp's worker daemons
(`bun cli.js __omp_worker_*`) are explicitly non-matching.

Multiple candidates → prefer the one corroborated by screen/title text,
else the deepest descendant. No candidate → `detection = None` (plain shell
shows no agent).

## UI

- Sidebar row: agent display name + state dot (Working #569CFF, Idle
  #8B949E, Blocked #E5A50A) when detected; process dot otherwise. Tooltip
  shows both layers.
- Tab label: `● <agent>` (state-colored dot) when detected, else the
  terminal title / tool name.
- Toasts (top-right, 4 s auto-dismiss): "X detected" (None→Some),
  "X needs attention" (→Blocked), "X finished" (Working→Idle). No toasts on
  agent-exit or shell-exit.

## Known limitations

- **No OSC 9/99/133** — vte drops them inside egui_term/alacritty; no
  FinalTerm prompt semantics, no progress notifications. (OSC-4 progress
  strings DO arrive as Title events and are used.)
- **No exact foreground process group** (`tcgetpgrp` blocked — PTY fd is
  owned by egui_term's event-loop thread): a *backgrounded* agent job
  (`claude &`) is still reported as the session's agent; mitigation is the
  screen-corroboration tie-break only.
- **Agent exit has no event** — detected purely by process disappearance
  (≤ 500 ms) plus the shell prompt reappearing; status is not derived from
  exit codes.
- Spawn-failure sessions (`shell_pid = 0`) are skipped by detection.
- Since the color work, every session is spawned with
  `TERM=xterm-256color` + `COLORTERM=truecolor` (agents emit full color;
  the screen engine reads cell *characters*, so color depth does not affect
  detection).
- Rule calibration is biased to the agents installed on this machine
  (claude 2.1.220, omp, kimi captured live; herdr manifests transcribed for
  the rest) — new agent versions may need pattern updates.

## Tests

`cargo test` — 15 tests: cmdline matching + wrapper peeling (real argv
shapes incl. omp worker non-matches), no-false-positives for plain shells,
live process-scan checks (plain bash → no candidates; a fake
`node …/claude-code/cli.js` descendant → found), rule engine over real
captured fixtures (claude trust-prompt blocked / braille-title working /
❯-box idle, omp π + braille titles, kimi working + idle, hermes title
states, common blocked rule for every agent, Idle fallback, detector
combining process + screen).
