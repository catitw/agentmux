# Agent detection feasibility on egui_term — phase-2 design basis

**Date:** 2026-08-05 · **Method:** read-only source inspection (no project files modified) · **Scope:** what is provably observable from egui_term's public API for automatic agent identification + status in each tab.

## Sources analyzed (exact revisions)

| Crate | Path | Rev |
|---|---|---|
| egui_term | `/home/catitw/.cargo/git/checkouts/egui_term-f3a0317759f11520/31bbc7a/` | `31bbc7a` (2026-07-30, "build(deps): update egui and alacritty_terminal (#65)") — this is the code pinned by `agentmux/Cargo.toml:15` (`egui_term = { git = …, branch = "main" }`, **unpinned**, rev can move) |
| alacritty_terminal 0.26.0 | `/home/catitw/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/alacritty_terminal-0.26.0/` | registry snapshot |
| vte 0.15.0 (alacritty_terminal's parser; re-exported as `alacritty_terminal::vte`, lib.rs:40) | `/home/catitw/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/vte-0.15.0/` | registry snapshot |

Notation: `egui/src/backend/mod.rs:276` = egui_term, `ala/…` = alacritty_terminal, `vte/…` = vte. All line numbers verified by direct read.

---

## 1. Capability table

| # | Capability | Available? | Exact API path |
|---|---|---|---|
| 1 | Visible screen text (char + color/flags) | **YES** | `TerminalBackend::sync() -> &RenderableContent` (egui backend/mod.rs:276-288) clones the active `Grid<Cell>` into `RenderableContent.grid` (pub, backend/mod.rs:566-573); text via `grid.display_iter()` → `Indexed<&Cell> { point, cell }` (ala grid/mod.rs:422, 554, 593) → `cell.c` (ala term/cell.rs:135). Colors: `cell.fg/bg: Color`; styling: `cell.flags` (`Flags` bitflags, ala term/cell.rs:12-26: BOLD/ITALIC/INVERSE/UNDERLINE/DIM/HIDDEN/…). |
| 2 | Full grid / scrollback (not just viewport) | **YES** | The same `.grid` clone contains the whole `Storage` (scrollback + viewport). Bounds: `Dimensions` trait — `total_lines()`, `screen_lines()`, `topmost_line() = Line(-history_size)`, `bottommost_line()`, `history_size()` (ala grid/mod.rs:488-518, impl for `Grid` at :520-535). Iteration: `grid.iter_from(Point)` (grid/mod.rs:412) or `grid[Line] -> Row<Cell>` (grid/mod.rs:447-449). |
| 3 | Bottom N non-empty lines (herdr `bottom_non_empty_lines(14)`) | **YES** | `grid.bottommost_line()` (ala grid/mod.rs:510) walked upward via `BidirectionalIterator::prev()` (grid/mod.rs:632-637); skip rows with `Row::is_clear()` (ala grid/row.rs:155-161). |
| 4 | OSC 0/2 title | **YES** | vte `osc_dispatch` routes `b"0"|b"2"` → `handler.set_title` (vte ansi.rs:1354-1359) → `Term::set_title` (ala term/mod.rs:2221-2239) → `Event::Title(String)` / `Event::ResetTitle` (ala event.rs:19, 22) → app's channel via `EventProxy` (egui backend/mod.rs:595-601, forwarding at :150, :192-207). |
| 5 | OSC 9 / OSC 99 (progress notifications) | **NO** | vte `osc_dispatch` handles only `0|2, 4, 8, 10|11|12, 22, 50, 52, 104, 110, 111, 112`; everything else hits `_ => unhandled(params)` (vte ansi.rs:1523-1524) which only `debug!`-logs and drops. No event, no callback, no hook anywhere. |
| 6 | OSC 133 (FinalTerm shell-integration markers) | **NO** | Same fallback arm (vte ansi.rs:1523). No tap point exists: raw OSC params live inside vte's private `Performer` (vte ansi.rs:425-437); the `Handler` trait has no raw-OSC method (vte ansi.rs:495-497); alacritty's parser is a private `State.parser: ansi::Processor` (ala event_loop.rs:405) driven by `processor.advance(handler = Term)` (ala event_loop.rs:154). `grep osc` over alacritty_terminal src: only `Osc52` enum (ala term/mod.rs:372) — no override. **Kills the OSC-133 detection idea.** |
| 7 | PTY master fd / `tcgetpgrp` | **NO** | The `Pty` (which does expose `child()`/`file()` — ala tty/unix.rs:110-115 — and `tty::new` returns it, unix.rs:195) is moved into `EventLoop::new(…, pty, …)` (egui backend/mod.rs:184-185), the thread is spawned and its `JoinHandle` discarded (:189). `TerminalBackend`'s full public API (backend/mod.rs:147-303: `new`/`process_command`/`selection_point`/`selectable_content`/`sync`/`last_content`/`id`/`pty_id`) exposes no fd, no `Child`, no `try_wait`. **herdr Channel B's `tcgetpgrp` probe (herdr `src/platform/linux.rs:325`) is BLOCKED at this layer — definitive.** |
| 8 | Child PID | **PARTIAL — shell PID only** | `pty_id() -> u32` (egui backend/mod.rs:301-303), captured at spawn from `pty.child().id()` (:162). This is the shell's PID, not the agent's. Usable as the root of a `/proc` descendant scan (see §3). |
| 9 | Child exit status | **YES (shell exit only)** | `Event::ChildExit(ExitStatus)` (ala event.rs:58; emitted from `ChildEvent::Exited` at event_loop.rs:255-266, enum ala tty/mod.rs:82-85) followed by `Event::Exit` (`Term::exit()`, ala term/mod.rs:806-809). **Semantics: this fires when the PTY child — the shell — exits, i.e. the tab's session dies. The agent is the shell's foreground child and its exit is NOT an event** (only visible via screen text and/or `/proc` disappearance). |
| 10 | Write arbitrary bytes to PTY | **YES** | `BackendCommand::Write(Vec<u8>)` (egui backend/mod.rs:34) → `process_command` (:219-224) → `self.write` (:506-507) → `Notifier::notify` → `Msg::Input` (ala event_loop.rs:31-35, Notify impl :335-346) → `pty_write` → `self.pty.writer().write(…)` (ala event_loop.rs:174-203). Any bytes — including OSC/CSI queries — can be injected; responses return as screen text / Title / Wakeup. |

**Event surface (Q1, complete):** `PtyEvent` is a pure alias — `pub type PtyEvent = Event;` (egui backend/mod.rs:29) — so the app receives every `alacritty_terminal::event::Event` variant (ala event.rs:14-59): `MouseCursorDirty` (:16), `Title(String)` (:19), `ResetTitle` (:22), `ClipboardStore(ClipboardType, String)` (:25), `ClipboardLoad(ClipboardType, Arc<dyn Fn(&str)->String>)` (:31), `ColorRequest(usize, Arc<dyn Fn(Rgb)->String>)` (:37), `PtyWrite(String)` (:40, terminal-generated output, re-echoed to the PTY by egui_term itself, backend/mod.rs:202-203), `TextAreaSizeRequest(Arc<dyn Fn(WindowSize)->String>)` (:43), `CursorBlinkingChange` (:46), `Wakeup` (:49, fired after every parsed output batch — event_loop.rs:271 — i.e. a per-content-change trigger), `Bell` (:52), `Exit` (:55), `ChildExit(ExitStatus)` (:58). **No** OSC-9/progress event, **no** OSC-133 event, **no** mouse/selection event (selection is app-side via `BackendCommand::Select*`).

**Screen access detail (Q2):** `sync()` re-clones `term.grid()` per call — `Grid<Cell>` is `Clone` (ala grid/mod.rs:109) and holds scrollback + viewport; `last_content()` (egui backend/mod.rs:293-295) returns the snapshot without re-cloning. `Term::grid()` (ala term/mod.rs:645-648) returns the **active** buffer — `swap_alt` `mem::swap`s the buffers (ala term/mod.rs:731), so TUI agents running on the alternate screen (Claude Code, vim-style TUIs) ARE captured; alt screen just has empty scrollback. `selectable_content()` (egui backend/mod.rs:263-273) returns **only the current selection's text** (empty `String` when nothing selected) — not whole content. Note alacritty's own borrowed, viewport-only `RenderableContent` (ala term/mod.rs:2393-2417) is a different type egui_term does not use.

---

## 2. Verdict — usable detection channels

Given egui_term's encapsulation, these channels are provably usable:

1. **Screen text** (full grid incl. scrollback + alt screen, per-cell char/color/flags) — herdr Channel A, fully replicated.
2. **OSC 0/2 title** events (herdr's `osc_title` manifest region) — fully replicated.
3. **`Wakeup` event cadence** — every content change is already delivered to the app; detection can be event-driven.
4. **Shell PID** via `pty_id()` → `/proc` descendant + argv scan for **agent identification** (which binary is running) — a degraded version of herdr Channel B: gives ID, not exact foreground-vs-background.
5. **`ChildExit`/`Exit`** — tab-session death only (same semantics as herdr's `child.wait() → PaneDied`).

Blocked, definitively:

- **OSC 9/99/133** — parsed and silently dropped inside vte (vte ansi.rs:1523-1524); no hook exists at egui_term *or* alacritty_terminal level. Shell-integration (FinalTerm) detection and OSC-progress status are unavailable without forking the stack.
- **Master fd / `tcgetpgrp` / exact foreground process group** — `Pty` is owned by the unreachable event-loop thread. Process detection is limited to `/proc` heuristics rooted at the shell PID. **Correction to the task premise:** the child PID *is* exposed (`pty_id()`), so the process channel is *partially* open — enough to identify the agent binary, not enough to resolve foreground vs background jobs with certainty.

---

## 3. Recommended phase-2 detection design

All signals below are public API today; nothing requires forking egui_term.

### 3.1 Agent identification (which tool)

Two independent signals, combined:

- **Process scan (primary).** Poll `/proc` every ~500 ms while unidentified: walk descendants of `pty_id()` (shell PID) via `/proc/*/stat` ppid field; read `/proc/<pid>/cmdline` for each live non-zombie descendant; match against a known-agent table (`claude`, `omp`, `codex`, `cursor`, `opencode`, `gemini`, `kilo`, `pi`, `hermes`, `qodercli`, …) including wrapper-peeling heuristics (`node …/claude-code`, `python -m …`, etc. — same argv normalization herdr does in `identify_agent_in_job`). Confidence is high when exactly one agent-shaped process is alive; with multiple candidates (background jobs), defer to the screen signal (below).
- **Screen/title (secondary).** `osc_title` region (Title events) and bottom-region matches for agent UI banners (`Claude Code`, `✦ omp`, `codex`, …). Low priority — used to break ties and as a cross-check.

### 3.2 Status (working / idle / done / error)

**Screen-regex engine** over the regions herdr defines, fed by our grid access:

- Regions: `bottom_non_empty_lines(14)` (rows walked upward from `grid.bottommost_line()` skipping `is_clear()` rows — §1 row 3), `osc_title` (§1 row 4), `after_last_prompt_marker` (shell-prompt regex — **not** FinalTerm, since OSC 133 is unavailable).
- Per-agent manifest rules with `state` / `priority` / `region` / `contains | regex | line_regex` gates; evaluate all rules, take highest priority; known agent with no match → `Idle` fallback (mirrors herdr's manifest engine, `src/detect/manifest.rs:414-557`).
- Rule sketches per state:
  - **Working:** spinner/progress frames in bottom region; `⏳`/`…`/`Thinking`/`[tool call]` markers; title prefix conventions (e.g. `⚠`/`⏳`); `Wakeup` events arriving continuously.
  - **Blocked:** question prompts (`needs your input`, `Select`, `y/n`, `↑/↓ to choose`), `⚠`-prefixed titles, edit-mode UIs.
  - **Done / error:** final banner (`Done`, `Exited with code N`, error traces) **conjuncted with** the agent process disappearing from the `/proc` scan and the shell prompt region reappearing — the prompt-reappearance + process-gone conjunction is the strongest available "done" signal (agent exit is not an event).
  - **Idle:** agent process alive but no `Wakeup` for T seconds and bottom region = plain prompt.
- Cadence: re-scan on `Wakeup`/`Title` events plus a 300–500 ms timer while an agent is identified (herdr uses 500/300 ms — herdr-architecture.md §3.2). `sync()` clones the full grid per call, so keep the clone path on the event/timer cadence, not every UI frame.
- **Tab death:** `ChildExit`/`Exit` → close/clear status (shell exited = session over, not agent done).

### 3.3 Honesty box

- **OSC 133 is not usable** → no FinalTerm prompt semantics; prompt-region detection is regex-based.
- **OSC 9/99 is not usable** → no title-progress channel; status is screen-only (plus process-existence).
- **No exact foreground-pgrp** → a backgrounded agent job (e.g. `claude &`) can be misreported as active; mitigation: require screen corroboration (agent UI visible) before reporting non-idle states, and report `Idle` when the bottom region looks like a plain prompt even if agent processes linger.
- Optional (safe, since write-to-PTY exists): none needed for phase 2 — avoid keystroke injection; it would interfere with the user's shell.
- `Wakeup`-driven scanning means detection reacts to output *as it happens*, so a 300–500 ms timer is only a backstop.

---

## 4. Next step — can phase 2 ship on egui_term as-is?

**Yes.** Screen-text + title + shell-PID + shell-exit + write-to-PTY are all public API; the phase-2 design above needs zero egui_term changes. Two recommendations:

1. **Pin the dependency.** `Cargo.toml:15` tracks `branch = "main"` (a moving target; current rev `31bbc7a`). Pin to `rev = "31bbc7a"` (or a fork) so detection behavior doesn't shift under the implementation.
2. **Fork only if accuracy demands it.** The only capabilities a fork would unlock: (a) raw OSC tap (9/99/133) — requires replacing alacritty's event loop or vte's `Performer` (vte is re-exported, so a forked egui_term/alacritty event loop could drive `vte::Parser` with its own `Perform` to see raw OSC params — moderate surgery, no vte fork needed); (b) master fd exposure for exact `tcgetpgrp` (one-line-ish change in egui_term's backend: retain `Pty::file()` clone before moving the `Pty` into the event loop — the fd accessor already exists at ala tty/unix.rs:113-115).
3. **Full-control fallback** (only if the screen engine proves insufficient): drop egui_term for a hand-rolled `portable-pty` + `vte`/`alacritty_terminal` stack with own event loop (the Horizon/herdr model) — grants master fd, raw OSC, and agent-exit observation via the child handle, at the cost of rewriting the terminal plumbing and rendering glue.

**Recommended path: implement phase 2 on egui_term as-is with a pinned rev; re-evaluate the fork only after measuring screen-regex accuracy against real Claude Code / omp / codex sessions.**
