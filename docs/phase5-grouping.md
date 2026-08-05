# Phase 5 — Project → Branch → Session grouping

The sidebar groups sessions into a three-level tree: project → branch →
sessions. The classification is **derived** from each session's working
directory — no schema change to `sessions.json` (`work_dir` was already
persisted) and nothing is stored back.

## Classification rules (`src/project.rs`)

- **Live cwd**: on Linux, `/proc/<shell_pid>/cwd` is read via one `readlink`
  on the detection tick (500 ms), so `cd` in a terminal re-classifies the
  session immediately. Other platforms have no cheap equivalent: the spawn
  work_dir is used and the session's project stays fixed for its lifetime
  (documented caveat). `/proc` read failures fall back to the spawn
  work_dir.
- **Project**: walk up from the cwd looking for `.git` — a directory OR a
  file (worktrees have a `.git` FILE). Found → the repo root is the project
  (display name = basename, full path in the header tooltip). Not found →
  the cwd itself is the project (standalone directory project, sessions
  directly under it, no branch line).
- **Branch**: `HEAD` is parsed without spawning git:
  - normal repo: `<root>/.git/HEAD` → `ref: refs/heads/<branch>`;
  - linked worktree: `<root>/.git` is a file, `gitdir: <path>` (relative to
    `root`), then `<path>/HEAD`;
  - detached HEAD: the 40-hex sha, shortened to 7 characters;
  - unparseable/missing → no branch level (sessions under a `<no branch>`
    group when the same project has parseable branches, else directly under
    the project).
- **Refresh cadence**: cwd changes re-classify immediately (next tick);
  HEAD is re-read per project root at most every 2 s (catches `git checkout`
  without a cwd change). Reads are cached by path with timestamps; all pure
  file access, no git processes, no new dependencies.

## Sidebar UI

```
Sessions                              [+]
▼ agentmux                            (project; tooltip = full path; click toggles collapse)
  ◆ main                              (branch sub-header)
    ● Shell              agentmux     (existing session rows, indented)
```

- Groups are derived from live sessions only (no empty groups). Sorting is
  deterministic: projects by name, branches by name, sessions by id.
- Project headers are clickable to collapse (chevron ▼/▶); collapse state
  lives in the app only and is not persisted.
- Session rows keep all existing behavior (select / close / status dot /
  agent detection / tooltip).
- Glyph note: the small triangles (U+25BE/U+25B8) and `⎇` (U+2387) exist in
  no available font and rendered as tofu; the headers use `▼`/`▶` (U+25BC/
  U+25B6, covered by egui defaults + CJK) and `◆` (U+25C6, CJK).

## Verification

`cargo test` — 52 tests (43 + 9 new): git-root discovery against real
`git init` + `commit` repos (found from a nested subdir; non-git dirs →
standalone), HEAD parsing via handcrafted `.git` dirs (regular branch,
detached sha, worktree `gitdir:` file, garbage → None), grouping purity
(mixed repos/standalone, branches sorted, sessions by id, orphan without
branch), classifier HEAD-cache cadence (change invisible within 2 s, picked
up after).

E2E: `XDG_CONFIG_HOME=/tmp/amx-p5 AGENTMUX_SEED_DIR=/home/catitw/mypros/agentmux
AGENTMUX_SEED_COMMAND=bash` → screenshot `/tmp/grouping_e2e2.png`: the
sidebar shows `Sessions` / `▼ agentmux` / `◆ main` / `● Shell` (selected,
blue dot, right label `agentmux`) with indent guides; no tofu. The live
cwd path was confirmed independently: `/proc/<shell-pid>/cwd` →
`/home/catitw/mypros/agentmux`. (Multiple sessions under one project/branch
are covered by the grouping unit tests; adding sessions interactively is
not possible on this machine.)
