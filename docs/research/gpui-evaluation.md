# GPUI Evaluation for agentmux

**Date:** 2026-08-05 · **Author:** research agent · **Scope:** read-only evaluation of zed-industries GPUI as a replacement for the agentmux eframe/egui stack

**Bottom line up front:** GPUI (the `gpui` crate inside zed-industries/zed) answers the three questions as follows:
(a) **cross-platform: yes** — macOS (native), Linux X11+Wayland (wgpu/Vulkan), Windows (stable since 2025-10-15, DirectX 11); (b) **Nerd Font / glyph fallback: yes, real per-glyph automatic system fallback** on all three platforms, plus a configurable user fallback chain — this directly solves agentmux's tofu problem; (c) **terminal escape sequences: not GPUI's concern** — GPUI is a UI framework with no terminal emulation; escape support comes from the terminal crate you pair with it (alacritty_terminal, which agentmux already uses). The catch is ecosystem maturity: crates.io is ~9.5 months stale behind a breaking-API main, upstream paused community-facing GPUI work in Feb 2026, Zed's terminal/UI crates are GPL (not reusable in a closed app), and a terminal renderer for GPUI must be written or vendored. **Verdict: not recommended as a near-term migration; the font pain point is cheaper to fix on the egui stack (already in progress). Revisit GPUI only if Windows shipping or GPU-text performance become hard requirements.** Details and evidence below.

---

## 1. Cross-platform status

### 1.1 Platforms supported TODAY (2026-08-05)

| Platform | Status | Evidence |
|---|---|---|
| macOS | First-class, native (Cocoa + CoreText + Metal) | `crates/gpui_macos` (src includes `text_system.rs`, `metal_renderer.rs`); zed is a macOS-origin product |
| Linux | **Shipped**, X11 + Wayland, Vulkan via wgpu | `crates/gpui_linux/Cargo.toml`: `default = ["wayland", "x11"]`; X11 via `x11rb` + `xim`, Wayland via `wayland-client` + `text-input`/`xdg` protocols; docs: "we use [Vulkan](https://www.vulkan.org/) to communicate with your GPU" (https://zed.dev/docs/linux) |
| Windows | **Stable since 2025-10-15** (not experimental) | Blog "Windows When? Windows Now" by Max Brunsfeld, Oct 15 2025 (https://zed.dev/blog/zed-for-windows-is-here): "The Windows build uses DirectX 11 for rendering, and DirectWrite for text rendering"; official install docs + winget (https://zed.dev/docs/windows); release downloads page ships macOS/Windows/Linux builds (https://zed.dev/releases/stable) |
| Web | Experimental only (GPUI has a wasm target; Zed does not ship web) | Zed README: "Other platforms are not yet available: Web ([tracking discussion #26195](https://github.com/zed-industries/zed/discussions/26195))" |

Notes and caveats:

- **Linux renderer history**: Linux launched on the Blade (Vulkan) renderer; it was replaced with a **wgpu**-based backend (PR [#46758](https://github.com/zed-industries/zed/pull/46758): "The blade graphics library is a mess and causes several issues for both Zed users as well as other 3rd party apps using GPUI" — per community doc [buiy prior-art](https://github.com/intendednull/buiy/blob/main/docs/prior-art/gpui/history.md)). Current `crates/gpui_wgpu` re-exports `pub use wgpu;` and contains the wgpu renderer + atlas (crates/gpui_wgpu/src/gpui_wgpu.rs:7). Note the Windows backend is **DirectX 11, not wgpu** — GPUI uses a native backend per platform (DX11/DirectWrite on Windows, Metal/CoreText on macOS, wgpu/cosmic-text on Linux).
- **Wayland still lags X11 in some corners**: native popup windows landed 2026-07-08 "with wayland xdg_popup implementation only so far" (PR #60232). IME exists on both: XIM on X11 (#11657 closed), `text_input_v3` + xkb compose on Wayland (#11712 closed).
- **Zed release cadence**: Zed is at 1.13.2 (2026-08-02, https://zed.dev/releases/stable); Zed 1.0 shipped October 2025 [community-sourced: buiy history.md cites linuxiac coverage; not directly verified on zed.dev].

### 1.2 The standalone `gpui` crate on crates.io is STALE vs upstream

- Latest crates.io version: **`gpui` 0.2.2, published 2025-10-22** (crates.io API, fetched 2026-08-05). That's ~9.5 months behind today. Publication history: 0.2.0 (2025-10-09) → 0.2.1 (2025-10-14) → 0.2.2 (2025-10-22). The crates.io publish set is self-consistent (the zed workspace crates were renamed for publishing — `gpui_collections`, `gpui_refineable`, `gpui_sum_tree`, `gpui_util_macros`, `gpui_http_client`, `gpui_media`, `gpui_semantic_version`, all `^0.2.2`, plus `zed-font-kit 0.14.1-zed`, `zed-scap`, `zed-xim`), so `gpui = "0.2.2"` **does resolve and build standalone** (docs.rs built it).
- But upstream main has drifted with breaking changes since that snapshot:
  - **`Render` API unification**: PR [#58087](https://github.com/zed-industries/zed/pull/58087) "Unify Render and RenderOnce into View", merged **2026-07-08**. Published 0.2.2 (docs.rs): `fn render(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement`; main (gpui.rs homepage, https://gpui.rs): `fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement` — every app written against 0.2.2's API needs changes.
  - taffy pinned `=0.9.0` in 0.2.2 vs `=0.12.2` on main (crates/gpui/Cargo.toml on main).
  - Linux backend: blade in 0.2.2 → wgpu on main.
  - New platform APIs on main since 0.2.2: native popup windows (#60232), system notifications (#61189).
- **The version string is not a version**: main's `crates/gpui/Cargo.toml` still says `version = "0.2.2"` (fetched 2026-08-05) — the repo doesn't bump the crate version per release; the crates.io publish is a point-in-time snapshot. "0.2.2 on crates.io" ≠ "0.2.2 on main".
- **Depending on the git repo is the norm** (no one in the wild uses crates.io):
  - Official scaffold `zed-industries/create-gpui-app` template: `gpui = { git = "https://github.com/zed-industries/zed" }` (templates/default/_Cargo.toml).
  - `longbridge/gpui-component` workspace deps on `gpui = { git = ... }` (main/Cargo.toml).
  - Independent apps: `rust-kotlin/ashell` (`gpui = { git = ... }`), `l0ng-ai/tty7` (pins rev `1d217ee3…`), `vicanso/zedis` (workspace git deps).
- **MSRV/toolchain**: the zed repo pins `rust-toolchain.toml` → `channel = "1.95.0"`, edition 2024 (fetched 2026-08-05). crates.io 0.2.2 declares no `rust-version` (crates.io API). agentmux currently declares `rust-version = "1.92"` (agentmux Cargo.toml) — building main-branch GPUI will very likely require a toolchain bump [INFERENCE: the repo pin is authoritative for Zed CI; no statement of a formal MSRV exists].
- Linux system requirements (from https://zed.dev/docs/linux): Vulkan-capable GPU, glibc ≥ 2.31 (x86_64) / ≥ 2.35 (aarch64); Windows requires a DirectX 11-capable GPU (https://zed.dev/docs/windows).

## 2. License

- Zed repo root carries **two license files**: `LICENSE-GPL` (GPL-3.0, standard text) and `LICENSE-APACHE` (Apache-2.0, "Copyright 2022 - 2025 Zed Industries, Inc."). README: "Zed source code is licensed primarily under GPL-3.0-or-later, with Apache-2.0 components where marked."
- The split is per-crate, set in each crate's Cargo.toml:
  - **Apache-2.0**: `gpui` (crates/gpui/Cargo.toml: `license = "Apache-2.0"`), `gpui_linux`, `gpui_tokio` (both Apache-2.0), and by pattern the other `gpui_*` platform crates.
  - **GPL-3.0-or-later**: `terminal` (crates/terminal/Cargo.toml), `terminal_view`, and the entire Zed UI widget layer — `ui`, `component`, `icons` (each `license = "GPL-3.0-or-later"`).
  - The alacritty fork agentmux would touch is Apache-2.0 (github.com/zed-industries/alacritty, `license: Apache-2.0`).
- **Consequences**:
  - A **closed-source** app may depend on `gpui` itself (Apache-2.0) — this is why Longbridge's commercial trading client runs on it.
  - The **same closed app may NOT vendor Zed's `terminal`/`terminal_view`/`ui` crates** (GPL-3.0-or-later) without open-sourcing the app. This is the single most consequential licensing fact for agentmux.
  - An open-source agentmux could use everything.
- History: Zed was open-sourced January 2024 under this deliberate split — "editor is GPL to keep forks copyleft; GPUI is Apache so others can use it" [community-sourced: buiy history.md Phase 4; the license files themselves are primary evidence of the current state].

## 3. Font handling / Nerd Font / CJK

### 3.1 Text stacks per platform (all do automatic glyph fallback)

- **Linux**: `crates/gpui_wgpu/src/cosmic_text_system.rs` — shaping via **cosmic-text 0.14** over a **fontdb** database loaded with system fonts (`FontSystem::new()` + `db()`; system fonts are enumerated, cosmic_text_system.rs:65-74, 107). When cosmic-text picks a fallback font for characters missing from the requested font, GPUI lazily loads it: "This is used when cosmic_text has chosen a fallback font instead of using the requested font, typically to handle some unicode characters" (doc comment on `font_id_for_cosmic_id`, cosmic_text_system.rs:~437-443).
- **macOS**: CoreText via the `zed-font-kit` fork (gpui Cargo.toml: `font-kit = { git = "https://github.com/zed-industries/font-kit", package = "zed-font-kit", version = "0.14.1-zed" }`, macOS-target only) — CoreText's cascade-list fallback [INFERENCE: font-kit's coretext backend performs cascade fallback; not verified line-by-line in the fork].
- **Windows**: DirectWrite, including its **native `IDWriteFontFallback`** (`crates/gpui_windows/src/direct_write.rs:34` `fallbacks: Option<IDWriteFontFallback>`), with the user fallback list compiled into a `CreateFontFallbackBuilder` chain (direct_write.rs:388-401).

### 3.2 Is fallback automatic, or must the app configure it?

**Both, layered.** There is no tofu-first design like egui's default fonts:

1. **Automatic platform fallback** (glyph-level) is the baseline: cosmic-text/fontdb on Linux, CoreText on macOS, DirectWrite on Windows (see 3.1).
2. **User-configurable chain on top**: `buffer_font_fallbacks` / `ui_font_fallbacks` settings — "this will be merged with the platform's default fallbacks" (assets/settings/default.json:32-34 and :60-62). The terminal inherits the same machinery (`terminal_settings.rs:26-27`: `font_family`, `font_fallbacks`).
3. **Per-run span algorithm**: for each text run, GPUI computes per-character spans choosing the first font in the chain that covers the char — `compute_run_spans(text, offs, run.len, run.font_id, &fallback_chain, &covers)` + `pick_covering_slot` + "falls through chain in order" tests (cosmic_text_system.rs:585-621, 836-932, tests at 1248-1414). A static family-resolution stack exists too (`.ZedMono`, `.ZedSans`, Helvetica, Segoe UI, Ubuntu, Noto Sans, DejaVu Sans, Arial — text_system.rs:71-83), used only when a named family can't load.

So: an app on GPUI gets system-wide CJK/emoji/braille coverage out of the box (same mechanism Zed's editor relies on), and `font_fallbacks` (or `terminal.font_fallbacks`) is the escape hatch for ordering/Nerd-Font preference — the equivalent of the fontdb fallback chain agentmux is hand-building for egui today (agentmux src/fonts.rs).

### 3.3 Nerd Font PUA glyphs — works, with known Linux bugs

- Nerd Font private-use-area icons **do render** if a Nerd Font is installed and reachable by the fallback machinery (this is the standard user recipe: set `terminal.font_family` to a Nerd Font, or add it to `font_fallbacks`).
- Historical confirmation of both the mechanism and its warts: issue [#18064](https://github.com/zed-industries/zed/issues/18064) "Nerd symbols not rendered correctly when using 'Symbols Nerd Font' as fallback" (macOS, Sep 2024, closed) — fallback found the glyphs but metrics were wrong; [#22437](https://github.com/zed-industries/zed/issues/22437) (Dec 2024, closed) escape-sequence issues with JetBrains Mono Nerd Font.
- **Current open Linux bug directly on this path** — [#61660](https://github.com/zed-industries/zed/issues/61660) (opened 2026-07-26, open): "Linux: fonts without a Latin 'm' glyph are removed from the font database, making emoji and symbol fonts unusable (tofu)". Root cause visible in source: `load_family` **evicts faces where `charmap().map('m') == 0`** (the "HACK: … We should actually do better font fallback" block, cosmic_text_system.rs:~300-310). A standalone "Symbols Nerd Font"-style face (no Latin 'm') can therefore be **dropped from the fallback pool → tofu**. Related open bugs: [#60155](https://github.com/zed-industries/zed/issues/60155) (2026-06-30) fallback settings fail when `font_weight != 400` on Linux; [#56527](https://github.com/zed-industries/zed/issues/56527) italic variants not applied to fallbacks; [#15925](https://github.com/zed-industries/zed/issues/15925) bitmap emoji fonts don't render.
- **Braille spinners (U+2800 block)** are covered by system fonts (DejaVu Sans / Noto Sans Symbols on Linux) — subject to the same #61660 eviction risk for symbol-only faces [INFERENCE: braille lives in symbol fonts; standard text fonts like DejaVu Sans include it].
- **Zed's own UI does not use Nerd Fonts at all**: Zed ships its own SVG icon set (assets/icons, 296 entries) and bundles only IBM Plex Sans + Lilex (assets/fonts); its UI icon font is not a Nerd Font. Nerd Font relevance is purely for terminal *content* (i.e., agent TUIs) — exactly agentmux's use case.

## 4. Terminal

### 4.1 What Zed ships

- **`crates/terminal`** — the terminal *model*: `terminal.rs` (~197 KB ≈ 5,500 lines) + `alacritty.rs` + `pty_info.rs` + `mappings/` (mouse/keys/colors). It wraps **`alacritty_terminal` from a Zed-maintained fork** (workspace Cargo.toml:523: `alacritty_terminal = { git = "https://github.com/zed-industries/alacritty", rev = "4c129667…" }`), which stays synced with upstream (fork last merged upstream master 2026-06-16; it's Apache-2.0).
- **`crates/terminal_view`** — the GPUI *renderer and UI*: `terminal_element.rs` (~114 KB), `terminal_view.rs` (~115 KB), `terminal_panel.rs` (~96 KB), `persistence.rs`, `terminal_scrollbar.rs`.
- **Not published and not separable cheaply**: workspace `publish = false` (Cargo.toml `[workspace.package]`), so there is no crates.io package; `crates/terminal` would be vendored as source. Its dependency surface (crates/terminal/Cargo.toml): gpui, settings, theme, theme_settings, task, util, collections, release_channel, sysinfo, vte, url, urlencoding, alacritty_terminal(fork), plus schemars/serde. `terminal.rs` itself imports only those (no editor/workspace imports — terminal.rs:8-77). The renderer is a different story: `terminal_element.rs` imports `editor`, `language`, `workspace`, `ui`, `theme`, `theme_settings`, `settings`, `util` (terminal_element.rs:1-25) — **deeply coupled to Zed's editor/workspace stack; not reusable without major surgery**.
- **Escape-sequence support**: GPUI itself implements **zero terminal emulation** — it's a UI framework. All escape handling lives in the emulator crate you bring: Zed's `crates/terminal` uses `vte` + `alacritty_terminal`'s parser (terminal.rs:54-55). agentmux **already depends on `alacritty_terminal 0.26` from crates.io** (agentmux Cargo.toml) — the same parser family. Switching to GPUI changes nothing about escape-sequence coverage; it only changes who draws the grid.

### 4.2 The realistic reuse options for agentmux

1. **Vendor `crates/terminal` (model only, GPL!) + write your own GPUI element renderer** (~1–2k lines: grid cells, cursor, selection, IME, scrollback). GPL-3.0-or-later → closed-source agentmux can't.
2. **Hand-roll the renderer over `alacritty_terminal` (which agentmux already has) + GPUI's text/atlas APIs** — license-clean (Apache-2.0 both sides), and the proven community path (see 4.3).
3. Take an existing independent implementation and adapt (see below).

### 4.3 Independent (non-Zed) projects embedding a terminal in GPUI — yes, several

| Project | Stars* | Terminal approach |
|---|---|---|
| [l0ng-ai/tty7](https://github.com/l0ng-ai/tty7) | 588 | **Terminal workbench for coding agents** (shells, SSH, persistent sessions, agents) — closest analogue to agentmux. Own GPUI view over **its own fork of Zed's alacritty fork** (Cargo.toml: "Our fork of Zed's alacritty_terminal fork: the VT parser + grid…"), Apache-2.0, ships macOS/Windows/Linux builds |
| [rust-kotlin/ashell](https://github.com/rust-kotlin/ashell) | 229 | GPUI-component SSH/local terminal client; uses **Zed's alacritty fork** directly (`alacritty_terminal = { git = zed-industries/alacritty }`) + gpui git + gpui-component git |
| [chi11321/CrabPort](https://github.com/chi11321/CrabPort) | 135 | SSH/SFTP client with integrated terminal on GPUI |
| [vicanso/zedis](https://github.com/vicanso/zedis) | 1,987 | Redis GUI on gpui + gpui-component (no terminal, but the largest non-Zed GPUI app) |

\* stars as of 2026-08-05 (GitHub API).

**Friction evidence from tty7** (the best analogue): it maintains **three forks** — `l0ng-ai/zed` (branch `tty7`: Windows font-fallback rasterization fix, resvg/usvg bump, IME handling), `l0ng-ai/gpui-component` (branch `tty7`), and `l0ng-ai/alacritty` (VT fixes "still missing from alacritty master as of 852e971") — pinned via `[patch]` (tty7 Cargo.toml:338-370). That is the honest cost of living on GPUI today: expect to carry patches.

## 5. Migration cost for agentmux

### 5.1 Inventory (agentmux, ~3.5k LOC total)

| Area | Current | Port cost |
|---|---|---|
| Sidebar / tab bar / status (`src/app.rs` 533, `sidebar.rs` 141, `status.rs` 43) | egui | Rewrite in GPUI's element model: `div().flex().flex_col()` trees; gpui-component has sidebar/tab/table components (its examples: `sidebar`, `table_in_scrollable`, `dialog_overlay`) — but note gpui-component is itself git-pinned and forked by tty7 |
| Terminal widget (`terminal_pane.rs` 120 + egui_term git dep) | egui_term (wraps alacritty_terminal 0.26) | **The big ticket**: egui_term's renderer has no GPUI equivalent that is license-clean. Options: write a GPUI element renderer over the existing `alacritty_terminal 0.26` (pattern: tty7/ashell), or vendor Zed's GPL `terminal`+renderer (blocked for closed-source) |
| Detection engine (`detect/*` ~1.1k) | Pure Rust (regex, std, sysinfo) — **no egui dependency** except one `egui::Color32` import (`detect/mod.rs:19`) | Ports as-is; swap one color type |
| Hook server (`hooks/*` ~1.05k) | tiny_http + `std::sync::mpsc` (`hooks/server.rs:14,53`) | Port the channel bridge (below) |
| Fonts (`fonts.rs` 185) | fontdb fallback chain hand-built for egui | **Delete**: GPUI's built-in fallback replaces it; optionally bundle a Nerd Font via `add_fonts` for deterministic PUA coverage (mitigates Linux #61660) |
| Toasts (`notify.rs` 74) | egui | gpui-component Toast, or hand-rolled; Zed's notifications crate is GPL — avoid |

### 5.2 API mappings

- **Panels/layout**: plain elements + flex/grid; `uniform_list` for virtualized lists (example in repo: crates/gpui/examples/uniform_list.rs); gpui-component for prebuilt widgets.
- **Custom paint**: implement the `Element` trait (`paint`); GPU text via `TextSystem`; glyph atlas handled by the platform (wgpu atlas on Linux, metal atlas on macOS, DirectWrite/DX11 on Windows).
- **Timers / frames**: `BackgroundExecutor::timer(Duration) -> Task<()>` (crates/gpui/src/executor.rs:162), `Window::request_animation_frame()` (window.rs:2357), `Context::spawn` (app.rs:1884).
- **Async model — the real friction**: GPUI runs **its own executor** (foreground on the main thread, a background thread pool via `AppContext::background_executor()`, app.rs:303/1869, executor.rs:89). Everything is `Entity<T>` + `Context`/`AsyncApp` (crates/gpui/docs/contexts.md). agentmux's `std::sync::mpsc` bridges (`app.rs:13,102` pty bridge; `hooks/server.rs:14,53` hook events) must become **async channels (async-channel/postage) awaited in `cx.spawn` tasks**, or a dedicated std thread + channel with a spawn-awaiting task. A blocking `mpsc::recv` inside a background task would stall one of GPUI's worker threads — avoid. tiny_http's blocking `recv` loop belongs on its own std thread feeding an async channel. There is a `gpui_tokio` bridge crate (Apache-2.0) if the app ever wants a tokio runtime inside.
- **Windows note**: GPUI on Windows uses **DirectX 11** (not wgpu/DX12 like eframe); GPUI's Windows team explicitly targeted VMs ("To run on almost all Windows versions, including VMs, we created a new rendering backend based on DirectX 11" — zed.dev/windows). Behavior will differ subtly from eframe's backend on the same machine.
- **Effort estimate** [INFERENCE]: with a ~3.5k-LOC app and three working precedent projects, a full port is plausibly 2–4 weeks of focused work; the terminal renderer is the uncertain half. This is an order-of-magnitude guess, not a commitment.

## 6. Verdict

**Not recommended today.** The user's three questions come back: cross-platform **yes** (all three desktop OSes, Windows stable since Oct 2025), Nerd Font/CJK **yes** (genuine per-glyph automatic fallback on every platform — the tofu problem would genuinely be solved), escape sequences **unchanged** (alacritty_terminal either way). But the surrounding ecosystem facts are disqualifying for a near-term migration of a small, working app:

### Top 3 risks

1. **API churn + deprioritized upstream.** crates.io is 9.5 months stale behind main, which merged breaking API changes in Jul 2026 (Render unification, #58087). Upstream explicitly paused community-facing GPUI work in Feb 2026 ("We gotta focus on some business relevant work in 2026" — HN [thread 47003569](https://news.ycombinator.com/item?id=47003569), 2026-02-13; community fork [gpui-ce](https://github.com/gpui-ce/gpui-ce), 850★, ~2-3k commits behind mainline [INFERENCE on commit distance]). Git-pinned revs rot; expect to carry patches (tty7 maintains 3 forks).
2. **Terminal widget rebuild + GPL wall.** egui_term does not exist for GPUI; the only ready-made GPUI terminal renderer is Zed's `terminal_view`, which is GPL-3.0-or-later AND welded to the editor/workspace crates. A closed-source agentmux must write its own GPUI terminal element (~the size of the current terminal_pane+egui_term work, plus scrollback/selection/IME polish) — or go GPL.
3. **Platform rough edges that recreate tofu.** Linux glyph fallback still has live bugs directly on the Nerd Font path: #61660 (symbol-only faces evicted from fontdb — "unusable (tofu)", open 2026-07-26), #60155 (fallback + weight), #56527 (italic fallbacks). The Windows (DX11) and Linux (wgpu) renderers are younger than the macOS one; Wayland still catches up in corners (popups only just landed).

### Top 3 benefits

1. **The stated pain point is genuinely solved by architecture**: per-glyph automatic system fallback (CoreText / DirectWrite / cosmic-text+fontdb) with a configurable chain — CJK, braille, and Nerd Font PUA icons render through the same pipeline Zed's editor ships; agentmux's hand-built fontdb hack (fonts.rs) becomes config, not code.
2. **Proven high-performance GPU text**: the same pipeline runs Zed 1.x in production on three OSes, with subpixel rendering, color emoji, and a 120 FPS design goal (https://zed.dev/blog/videogame).
3. **Real cross-platform story + a small but real ecosystem**: Windows stable with a dedicated port team, macOS first-class, Linux on wgpu; `gpui-component` (12.4k★) is in production at Longbridge; tty7 proves an agent-oriented GPUI terminal workbench is buildable and shippable (Apache-2.0 — worth studying as the reference architecture if this path is ever taken).

### Suggested conditions to revisit

Reconsider GPUI when any of: (a) a stable standalone GPUI repo / resumed crates.io publishes with semver discipline (gpui-ce or a zed-industries extraction — iamnbttler: "Zed Industries would generally benefit if gpui did get pulled out of Zed" [same HN thread]); (b) Windows shipping becomes a hard requirement for agentmux (GPUI is the strongest current route in Rust); or (c) the Linux font bugs above get fixed and the Render API stabilizes for a full release cycle. Meanwhile the egui + fontdb-fallback-chain fix in flight is the right call: it solves the same tofu problem at a fraction of the risk.

---

## Sources (all fetched 2026-08-05 unless noted)

- zed-industries/zed repo: README; Cargo.toml (workspace, publish=false, alacritty_terminal rev at :523); rust-toolchain.toml (1.95.0); LICENSE-GPL; LICENSE-APACHE; crates/gpui/Cargo.toml (Apache-2.0, taffy =0.12.2, font-kit fork); crates/gpui/src/{gpui.rs, app.rs, window.rs, executor.rs, text_system.rs:40-240}; crates/gpui/docs/contexts.md; crates/gpui_wgpu/src/{gpui_wgpu.rs, cosmic_text_system.rs}; crates/gpui_windows/src/direct_write.rs; crates/gpui_linux/Cargo.toml; crates/terminal/Cargo.toml + src/terminal.rs; crates/terminal_view/src/terminal_element.rs; crates/ui, crates/component, crates/icons, crates/gpui_tokio Cargo.toml; assets/settings/default.json (:26-85); assets/fonts; assets/icons; PRs #58087, #60232, #61189; issues #18064, #22437, #61660, #60155, #56527, #15925, #11657, #11712, #26195
- crates.io API: gpui (0.2.2, 2025-10-22), gpui/0.2.2/dependencies, gpui_macros, gpui_util, zed-font-kit, gpui-component (0.5.1, 2026-02-05)
- docs.rs/gpui/0.2.2/trait.Render.html
- zed.dev: /docs/linux, /docs/windows, /windows, /blog/zed-for-windows-is-here (2025-10-15), /releases/stable, https://gpui.rs
- GitHub API: zed-industries/{alacritty, create-gpui-app (incl. templates/default/_Cargo.toml)}, longbridge/gpui-component, gpui-ce, l0ng-ai/tty7, rust-kotlin/ashell, vicanso/zedis; repo search `gpui in:description language:rust` (stars 2026-08-05)
- HN thread 47003569 (2026-02-13): GPUI community-work pause, gpui-ce, license split comment
- [community-sourced, secondary] intendednull/buiy docs/prior-art/gpui/{history.md, distribution-and-governance.md} (dated 2026-05-22) — used only for Zed 1.0 (Oct 2025), Blade→wgpu (PR #46758), and 2024 license-split history; all load-bearing claims were verified against primary sources above
