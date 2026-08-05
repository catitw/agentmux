# Phase 4 — session persistence & new-session dialog

Two coupled features: a new-session dialog (both "+" buttons no longer spawn
blindly) and metadata persistence so the session list survives restarts.

## New-session dialog (`src/new_session.rs`)

Both "+" buttons (sidebar header and tab bar) now emit `Action::NewSession`,
which opens a plain `egui::Window` instead of spawning:

- **Work directory** — text input, default `$HOME`.
- **Command** — text input, default = `default_shell_command()` (the
  `$SHELL`-based shell). Split into program + args on whitespace
  (`split_command`); the first token is the program alacritty spawns.
- **Label** — optional; empty derives from the command basename
  (`derive_label`: shell programs → "Shell" as today; anything else keeps
  its basename, e.g. "omp" → "omp").
- **Validation** (`validate`, pure): work dir must exist and be a directory
  (inline red error text; dialog stays open), command must be non-empty.
  Enter = Create, Esc = Cancel.
- On Create: spawn the session with the draft values, persist, close.

Deliberately minimal: no dir-picker crate, no clap, three text fields.

## Persistence (`src/persist.rs`)

- File: `config_dir()/agentmux/sessions.json` — same `dirs::config_dir()`
  base as the hook port file (honors `XDG_CONFIG_HOME` on Linux /
  `%APPDATA%` on Windows).
- Schema (`version: 1`):

  ```json
  { "version": 1, "sessions": [ { "work_dir": "...", "command": "...", "label": "..." } ] }
  ```

  Array order = sidebar order. Only metadata persists — no PIDs, no status.
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
