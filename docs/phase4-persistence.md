# Phase 4 — session persistence & session management

Session metadata persistence so the session list survives restarts, plus
the new-session and rename flows that feed it.

## New session (direct spawn)

All "+" entry points (sidebar header, tab bar, empty-state button) spawn a
shell session immediately — there is no dialog:

- **Work directory**: the currently SELECTED session's live cwd
  (`project::live_cwd(shell_pid)`, fallback = that session's spawn
  work_dir), so a new tab opens where you are working; with no selection,
  `default_work_dir()` (`$HOME`).
- **Command**: `default_shell_command()` (`$SHELL` → `/etc/passwd` login
  shell → `bash`).
- **Label**: `session::derive_label(command)` — shells map to "Shell",
  anything else keeps its basename (e.g. "omp").
- Persisted like any spawned session.

## Rename

Right-click a session row → **Rename session** → inline single-line edit:
Enter commits, Esc cancels, empty/whitespace commit CLEARS the custom name.
`Session.custom_name` takes display precedence everywhere (sidebar row, tab
label): custom name > detected agent > terminal title / tool name.

## Persistence (`src/persist.rs`)

- File: `config_dir()/agentmux/sessions.json` — same `dirs::config_dir()`
  base as the hook port file (honors `XDG_CONFIG_HOME` on Linux /
  `%APPDATA%` on Windows).
- Schema (`version: 1` — `custom_name` is optional, so files written before
  the rename feature load unchanged):

  ```json
  { "version": 1, "sessions": [ { "work_dir": "...", "command": "...", "label": "...", "custom_name": "..." } ] }
  ```

  Array order = sidebar order. Only metadata persists — no PIDs, no status.
  Restore applies `custom_name` when present.
- **Save**: after every spawn and every close (both are rare; no debounce).
  Atomic: write `sessions.json.tmp` then rename. Includes all live sessions
  (even ones whose agent is running) EXCEPT transient ones: sessions spawned
  via `AGENTMUX_SEED_COMMAND` are verification scaffolding and are never
  persisted. When every live session is transient (a pure seed run), the
  file is not written at all — no artifact is created and an existing file
  with real sessions stays untouched.
- **Restore**: at startup, if the file exists and parses, spawn one session
  per entry; entries whose `work_dir` no longer exists are skipped with a
  per-entry warning. Missing file → "no sessions file, seeded default";
  malformed file (unparsable JSON or unsupported schema version) →
  "sessions file malformed (<err>), seeded default". Malformed never
  crashes.
- **Precedence**: `AGENTMUX_SEED_COMMAND` (the verification hook) > restore
  > default seed. With the seed env set, exactly that one session is
  spawned and restore is skipped.
- Startup log lines (verified in e2e):
  - `agentmux: restored N session(s) from <path>`
  - `agentmux: no sessions file, seeded default`
  - `agentmux: sessions file malformed (<err>), seeded default`
  - `agentmux: skipping session, work dir missing: <dir>`

## Honest limitation

Processes do **not** survive app exit — there is no daemon. Restore
respawns fresh shells in the saved directories; any agents running at exit
are gone. Persistence is about the session *list* (dirs + commands), not
live state.

## Verification

- `cargo test` — 38 tests (28 existing + 10 new): persist save/load
  roundtrip with order preserved, missing file → NotFound, malformed JSON →
  Malformed, unsupported version → Malformed, atomic save (no leftover tmp,
  parent dirs created); dialog validation (existing dir + command ok /
  bad dir / file-as-dir / empty command), label derivation (shells → Shell,
  agents → basename), command splitting.
- Scripted e2e with `XDG_CONFIG_HOME=/tmp/amx-p4`:
  1. `AGENTMUX_SEED_COMMAND=omp` launch → `sessions.json` is NOT written
     (seed sessions are transient; only the hook port file appears). A
     pre-existing `sessions.json` is left untouched.
  2. Plain launch (no seed) → a default session is seeded and persisted;
     the next plain launch restores it: `agentmux: restored 1 session(s)
     from /tmp/amx-p4/agentmux/sessions.json`.
  3. Missing-dir entry → `skipping session, work dir missing` + `restored 0
     session(s)`.
  4. Garbage file → `sessions file malformed (…), seeded default`.
- `cargo build` + `cargo clippy --all-targets` clean; `timeout 8s cargo run`
  with default config → exit 124, no panic.
