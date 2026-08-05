//! Screen/title rule engine (herdr-manifest style, simplified) plus the
//! per-session [`Detector`].
//!
//! Rules are plain data, evaluated highest-priority-wins over two regions:
//! the bottom non-empty screen lines and the OSC 0/2 terminal title. A known
//! agent with no rule match falls back to [`AgentState::Idle`].
//!
//! Calibration: patterns marked `real` were observed in live PTY captures of
//! the actual agents on this machine (see docs/phase2-detection.md);
//! patterns based on herdr's bundled manifests or agent docs are marked
//! `inferred`.

use crate::detect::process::Candidate;
use crate::detect::{AgentKind, AgentState, Detection, AGENTS};
use regex::Regex;
use std::collections::BTreeMap;
use std::sync::LazyLock;
use std::time::Instant;

/// Region a rule reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Region {
    /// Joined bottom non-empty lines.
    Bottom,
    /// OSC 0/2 terminal title.
    Title,
}

/// One detection rule.
///
/// Text needles are stored lowercased and matched case-insensitively (real
/// agent UIs mix case, e.g. claude's "Enter to confirm   Esc to cancel").
/// Regexes are matched as written (add `(?i)` explicitly where needed).
pub struct Rule {
    pub state: AgentState,
    pub priority: u32,
    pub region: Region,
    /// Every one of these must be present in the region text.
    pub contains: Vec<String>,
    /// At least one group must be fully present.
    pub any_contains: Vec<Vec<String>>,
    /// None of these may be present.
    pub not_contains: Vec<String>,
    /// Regex matched against the whole region text.
    pub regex: Option<Regex>,
    /// Regex matched against any single bottom line.
    pub line_regex: Option<Regex>,
}

fn lc(s: &'static str) -> String {
    s.to_lowercase()
}

/// Compact rule constructor. Panics on an invalid regex — the table is
/// static and exercised by tests.
#[allow(clippy::too_many_arguments)] // compact static-table builder
fn rule(
    state: AgentState,
    priority: u32,
    region: Region,
    contains: &'static [&'static str],
    any_contains: &'static [&'static [&'static str]],
    not_contains: &'static [&'static str],
    regex: Option<&'static str>,
    line_regex: Option<&'static str>,
) -> Rule {
    Rule {
        state,
        priority,
        region,
        contains: contains.iter().map(|s| lc(s)).collect(),
        any_contains: any_contains
            .iter()
            .map(|group| group.iter().map(|s| lc(s)).collect())
            .collect(),
        not_contains: not_contains.iter().map(|s| lc(s)).collect(),
        regex: regex.map(|pat| Regex::new(pat).expect("invalid detection rule regex")),
        line_regex: line_regex.map(|pat| Regex::new(pat).expect("invalid detection rule line regex")),
    }
}

/// Rules shared by every agent (calibrated against real captures of
/// claude/omp/kimi, which all emit braille-spinner titles and OSC 4 progress
/// strings while working).
fn common_rules() -> Vec<Rule> {
    vec![
        // REAL: claude "⠂ Claude Code", omp "π ⠋ agentmux-cap", kimi " ⠼ working..."
        // all set braille-spinner titles while working (herdr: priority 1100).
        rule(
            AgentState::Working,
            1100,
            Region::Title,
            &[],
            &[],
            &[],
            Some(r"[\u{2800}-\u{28FF}]"),
            None,
        ),
        // REAL: all three emit "4;3" (progress) while working, "4;0" when done.
        rule(AgentState::Working, 1050, Region::Title, &[], &[], &[], Some(r"^4;3"), None),
        rule(AgentState::Idle, 250, Region::Title, &[], &[], &[], Some(r"^4;0"), None),
        // REAL (claude trust prompt): "Enter to confirm   Esc to cancel".
        // herdr-claude: priority 980, "esc to cancel" + enter confirm/select.
        rule(
            AgentState::Blocked,
            980,
            Region::Bottom,
            &["esc to cancel"],
            &[&["enter to confirm"], &["enter to select"]],
            &[],
            None,
            None,
        ),
        // herdr-claude generic permission prompt.
        rule(
            AgentState::Blocked,
            920,
            Region::Bottom,
            &["do you want to proceed?"],
            &[],
            &[],
            None,
            None,
        ),
        rule(AgentState::Blocked, 800, Region::Bottom, &["waiting for permission"], &[], &[], None, None),
    ]
}

/// Claude Code rules. Calibrated against live captures of v2.1.220 plus
/// herdr's claude manifest.
fn claude_rules() -> Vec<Rule> {
    vec![
        // REAL: workspace trust prompt.
        rule(AgentState::Blocked, 970, Region::Bottom, &["Quick safety check"], &[], &[], None, None),
        // REAL: bottom input box `❯` / `❯ …`. Excluded when a prompt form is
        // visible (the trust prompt also renders a `❯`-marked option list).
        rule(
            AgentState::Idle,
            950,
            Region::Bottom,
            &[],
            &[],
            &["esc to cancel", "enter to select", "enter to confirm", "do you want to proceed?"],
            None,
            Some(r"^\s*❯"),
        ),
        // REAL: "Thought for 4s", "Nucleating…", "✻ Worked for 9s", "✢ Web Search".
        rule(
            AgentState::Working,
            900,
            Region::Bottom,
            &[],
            &[&["Thought for "], &["Nucleating"], &["Worked for"], &["✢"]],
            &[],
            None,
            None,
        ),
        // REAL: idle title "✳ Claude Code".
        rule(AgentState::Idle, 250, Region::Title, &[], &[], &[], Some(r"^✳"), None),
    ]
}

/// omp (pi-coding-agent) rules. Title patterns are real; screen patterns are
/// inferred from herdr's pi manifest and the omp capture.
fn omp_rules() -> Vec<Rule> {
    vec![
        // herdr pi.toml: "Working..." screen marker (inferred for omp).
        rule(AgentState::Working, 900, Region::Bottom, &["Working..."], &[], &[], None, None),
        // REAL: idle titles "π >" / "π > agentmux-cap" and "Oh My Pi: Complete".
        rule(AgentState::Idle, 900, Region::Title, &[], &[], &[], Some(r"^π\s*>"), None),
        rule(AgentState::Idle, 850, Region::Title, &["Oh My Pi: Complete"], &[], &[], None, None),
    ]
}

/// Kimi Code rules. All patterns from the live capture.
fn kimi_rules() -> Vec<Rule> {
    vec![
        // REAL: " ⠼ working..." spinner line.
        rule(AgentState::Working, 950, Region::Bottom, &["working..."], &[], &[], None, None),
        // REAL: idle prompt box "│ >".
        rule(AgentState::Idle, 950, Region::Bottom, &[], &[], &[], None, Some(r"^\s*│\s*>\s")),
        // REAL: idle title "Kimi Code".
        rule(AgentState::Idle, 800, Region::Title, &["Kimi Code"], &[], &[], None, None),
    ]
}

/// hermes rules, from herdr's hermes manifest (inferred — no hermes binary
/// on this machine).
fn hermes_rules() -> Vec<Rule> {
    vec![
        rule(AgentState::Blocked, 1100, Region::Title, &[], &[], &[], Some(r"^⚠"), None),
        rule(AgentState::Working, 1050, Region::Title, &[], &[], &[], Some(r"^⏳"), None),
        rule(
            AgentState::Blocked,
            900,
            Region::Bottom,
            &["dangerous"],
            &[&["approval"], &["allow once", "deny"]],
            &[],
            None,
            None,
        ),
        rule(AgentState::Blocked, 900, Region::Bottom, &["hermes needs your"], &[], &[], None, None),
        rule(AgentState::Blocked, 850, Region::Bottom, &[], &[], &[], None, Some(r"^\s*ask\s+\S")),
    ]
}

/// OpenCode / Kilo rules, from herdr's manifests (inferred — neither binary
/// on this machine).
fn opencode_rules() -> Vec<Rule> {
    vec![
        rule(AgentState::Blocked, 900, Region::Bottom, &["△ Permission required"], &[], &[], None, None),
        rule(
            AgentState::Blocked,
            900,
            Region::Bottom,
            &["esc dismiss"],
            &[&["enter confirm"], &["enter submit"], &["enter toggle"]],
            &[],
            None,
            None,
        ),
        rule(
            AgentState::Working,
            850,
            Region::Bottom,
            &[],
            &[&["esc to interrupt"], &["ctrl+c to interrupt"], &["press esc to interrupt"]],
            &[],
            None,
            None,
        ),
    ]
}

/// Gemini CLI rules, from herdr's gemini manifest (inferred — no gemini
/// binary on this machine).
fn gemini_rules() -> Vec<Rule> {
    vec![
        rule(
            AgentState::Blocked,
            900,
            Region::Bottom,
            &[],
            &[&["│ Apply this change"], &["│ Allow execution"], &["waiting for user confirmation"]],
            &[],
            None,
            None,
        ),
        rule(AgentState::Working, 100, Region::Bottom, &["esc to cancel"], &[], &[], None, None),
    ]
}

/// Codex rules (inferred — the local codex binary has no usable TUI without
/// configuration; its documented permission prompt is "do you want to
/// proceed?"-style, covered by the common rules).
fn codex_rules() -> Vec<Rule> {
    vec![rule(
        AgentState::Blocked,
        900,
        Region::Bottom,
        &[],
        &[&["should codex"], &["allow this command"]],
        &[],
        None,
        None,
    )]
}

/// Per-agent rule table, built once. Common rules are prepended to every
/// agent's list; agent-specific rules follow.
static RULES: LazyLock<BTreeMap<AgentKind, Vec<Rule>>> = LazyLock::new(|| {
    let mut table = BTreeMap::new();
    for def in AGENTS {
        let mut rules = common_rules();
        let specific: Vec<Rule> = match def.kind {
            AgentKind::ClaudeCode => claude_rules(),
            AgentKind::Omp => omp_rules(),
            AgentKind::Kimi => kimi_rules(),
            AgentKind::Hermes => hermes_rules(),
            AgentKind::OpenCode | AgentKind::Kilo => opencode_rules(),
            AgentKind::Gemini => gemini_rules(),
            AgentKind::Codex => codex_rules(),
            AgentKind::Pi => vec![rule(
                AgentState::Working,
                900,
                Region::Bottom,
                &["Working..."],
                &[],
                &[],
                None,
                None,
            )],
        };
        rules.extend(specific);
        table.insert(def.kind, rules);
    }
    table
});

/// Evaluate all rules for one agent over screen regions. Highest priority
/// wins; a known agent with no rule match is [`AgentState::Idle`].
pub fn evaluate(agent: AgentKind, bottom_lines: &[String], title: Option<&str>) -> AgentState {
    let bottom = bottom_lines.join("\n");
    let title = title.unwrap_or("");
    let mut best: Option<(u32, AgentState)> = None;
    for rule in RULES.get(&agent).into_iter().flatten() {
        if !matches_rule(rule, &bottom, bottom_lines, title) {
            continue;
        }
        if best.is_none_or(|(priority, _)| rule.priority > priority) {
            best = Some((rule.priority, rule.state));
        }
    }
    best.map_or(AgentState::Idle, |(_, state)| state)
}

fn matches_rule(rule: &Rule, bottom: &str, bottom_lines: &[String], title: &str) -> bool {
    let text = match rule.region {
        Region::Bottom => bottom,
        Region::Title => title,
    };
    let has_needles =
        !rule.contains.is_empty() || !rule.any_contains.is_empty() || !rule.not_contains.is_empty();
    let text_lower = has_needles.then(|| text.to_lowercase());
    let text_lower = text_lower.as_deref().unwrap_or(text);
    rule.contains.iter().all(|needle| text_lower.contains(needle))
        && (rule.any_contains.is_empty()
            || rule.any_contains.iter().any(|group| group.iter().all(|needle| text_lower.contains(needle))))
        && rule.not_contains.iter().all(|needle| !text_lower.contains(needle))
        && rule.regex.as_ref().is_none_or(|re| re.is_match(text))
        && rule
            .line_regex
            .as_ref()
            .is_none_or(|re| bottom_lines.iter().any(|line| re.is_match(line)))
}

/// Per-session detection state: sync throttle + last process candidates.
pub struct Detector {
    /// Last time the grid was cloned for this session (sync throttle).
    pub last_sync: Option<Instant>,
    /// Agent candidates from the last process scan.
    pub candidates: Vec<Candidate>,
}

impl Default for Detector {
    fn default() -> Self {
        Self::new()
    }
}

impl Detector {
    pub fn new() -> Self {
        Self {
            last_sync: None,
            candidates: Vec::new(),
        }
    }

    /// Combine the current process candidates + screen regions into a
    /// detection. Returns `None` when no agent process is alive.
    pub fn evaluate(&self, bottom_lines: &[String], title: Option<&str>) -> Option<Detection> {
        let agent = pick_agent(&self.candidates, bottom_lines, title)?;
        let state = evaluate(agent, bottom_lines, title);
        Some(Detection { agent, state })
    }
}

/// With multiple candidates, prefer the one corroborated by the screen or
/// title text (its display name or exe name visible), else the deepest
/// descendant (closest to the user's shell).
fn pick_agent(
    candidates: &[Candidate],
    bottom_lines: &[String],
    title: Option<&str>,
) -> Option<AgentKind> {
    if candidates.is_empty() {
        return None;
    }
    if candidates.len() == 1 {
        return Some(candidates[0].kind);
    }
    let haystack = format!("{}\n{}", bottom_lines.join("\n"), title.unwrap_or(""));
    for candidate in candidates {
        let def = AGENTS
            .iter()
            .find(|def| def.kind == candidate.kind)
            .expect("candidate kind is in AGENTS");
        if haystack.contains(def.display_name)
            || def.exe_names.iter().any(|name| haystack.contains(name))
        {
            return Some(candidate.kind);
        }
    }
    candidates.last().map(|c| c.kind)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(ls: &[&str]) -> Vec<String> {
        ls.iter().map(|s| s.to_string()).collect()
    }

    // --- Fixtures captured from REAL agent runs on this machine ---

    const CLAUDE_BLOCKED_TRUST: &[&str] = &[
        " Accessing workspace:",
        "",
        "/tmp/agentmux-cap",
        "",
        "Quick safety check: Is this a project you created or one you trust? (Like your",
        "own code, a well-known open source project, or work from your team). If not,",
        "take a moment to review what's in this folder first.",
        "",
        "Claude Code'll be able to read, edit, and execute files here.",
        "",
        "Security guide",
        "",
        "❯ 1. Yes, I trust this folder",
        "  2. No, exit",
        "",
        "Enter to confirm   Esc to cancel",
    ];

    const CLAUDE_WORKING: &[&str] = &[
        "● 2 + 2 = 4",
        "",
        "✻ Worked for 9s",
        "╰─",
        "❯ run ls",
        "│",
        "  Thought for 1s, listed 1 directory, ran 1 shell command",
    ];

    const CLAUDE_IDLE: &[&str] = &[
        "❯",
        "────────────────────────────────────────────────────────────────────────────────",
        "  cwd: /tmp/agentmux-cap   Model: Opus 5   Reset: 4hr 23m",
    ];

    const OMP_IDLE: &[&str] = &[
        "╭──  K3 · max  agentmux-cap  2.3%/1M (sub) ──╮",
        "╰─  ─╯",
        "2026-08-05 14:36:39  12K  37  12K  8.4s  4.1/s",
    ];

    const KIMI_IDLE: &[&str] = &[
        "╭───────────────────────────────────────────────────────────────────────────╮",
        "│ >                                                                         │",
        "╰───────────────────────────────────────────────────────────────────────────╯",
        "K3 thinking: high  /tmp/agentmux-cap   ! to run a shell command",
    ];

    const KIMI_WORKING: &[&str] = &[
        " ⠼ working...",
        "K3 thinking: high  /tmp/agentmux-cap",
    ];

    #[test]
    fn claude_blocked_trust_prompt() {
        let state = evaluate(AgentKind::ClaudeCode, &lines(CLAUDE_BLOCKED_TRUST), None);
        assert_eq!(state, AgentState::Blocked);
    }

    #[test]
    fn claude_working_screen_and_title() {
        // Braille-spinner title while working beats the ❯ prompt-box rule.
        assert_eq!(
            evaluate(AgentKind::ClaudeCode, &lines(CLAUDE_WORKING), Some("⠂ Simple math question")),
            AgentState::Working
        );
        // ✳ title + ❯ input box after a finished task → Idle (herdr model:
        // the prompt box is the authoritative idle signal; the "Thought for"
        // / "Worked for" summary lines are history, not activity).
        assert_eq!(
            evaluate(AgentKind::ClaudeCode, &lines(CLAUDE_WORKING), Some("✳ Claude Code")),
            AgentState::Idle
        );
    }

    #[test]
    fn claude_idle_input_box() {
        assert_eq!(
            evaluate(AgentKind::ClaudeCode, &lines(CLAUDE_IDLE), Some("✳ Claude Code")),
            AgentState::Idle
        );
        // The ❯-marked option list inside a blocked form must NOT read as idle.
        assert_eq!(
            evaluate(
                AgentKind::ClaudeCode,
                &lines(&["❯ 1. Yes, I trust this folder", "  2. No, exit", "", "Enter to confirm   Esc to cancel"]),
                None
            ),
            AgentState::Blocked
        );
    }

    #[test]
    fn omp_states() {
        // REAL: braille spinner title while working.
        assert_eq!(
            evaluate(AgentKind::Omp, &lines(OMP_IDLE), Some("π ⠋ agentmux-cap")),
            AgentState::Working
        );
        // REAL: "4;3" progress title.
        assert_eq!(
            evaluate(AgentKind::Omp, &lines(OMP_IDLE), Some("4;3")),
            AgentState::Working
        );
        // REAL: idle title.
        assert_eq!(
            evaluate(AgentKind::Omp, &lines(OMP_IDLE), Some("π > agentmux-cap")),
            AgentState::Idle
        );
        // REAL: completed title.
        assert_eq!(
            evaluate(AgentKind::Omp, &lines(OMP_IDLE), Some("Oh My Pi: Complete")),
            AgentState::Idle
        );
    }

    #[test]
    fn kimi_states() {
        assert_eq!(
            evaluate(AgentKind::Kimi, &lines(KIMI_WORKING), Some("4;3")),
            AgentState::Working
        );
        assert_eq!(
            evaluate(AgentKind::Kimi, &lines(KIMI_IDLE), Some("Kimi Code")),
            AgentState::Idle
        );
    }

    #[test]
    fn hermes_title_states() {
        assert_eq!(
            evaluate(AgentKind::Hermes, &lines(&["some prompt"]), Some("⚠ hermes needs your input")),
            AgentState::Blocked
        );
        assert_eq!(
            evaluate(AgentKind::Hermes, &lines(&["some prompt"]), Some("⏳ hermes working")),
            AgentState::Working
        );
    }

    #[test]
    fn common_blocked_rule_applies_to_all_agents() {
        for kind in AGENTS.iter().map(|def| def.kind) {
            let state = evaluate(
                kind,
                &lines(&["Do you want to proceed?", "❯ 1. Yes", "  2. No", "Enter to confirm   Esc to cancel"]),
                None,
            );
            assert_eq!(state, AgentState::Blocked, "common blocked rule failed for {kind:?}");
        }
    }

    #[test]
    fn unknown_screen_falls_back_to_idle() {
        // Known agent, no rule matches, no title signal → Idle fallback.
        assert_eq!(
            evaluate(AgentKind::ClaudeCode, &lines(&["hello world", "nothing special here"]), Some("weird title")),
            AgentState::Idle
        );
        // Plain-shell text must not be misread as working/blocked either.
        assert_eq!(
            evaluate(AgentKind::Codex, &lines(&["catitw@host ~/codex>"]), Some("~ - fish")),
            AgentState::Idle
        );
    }

    #[test]
    fn pick_agent_prefers_deepest_or_corroborated() {
        use crate::detect::process::Candidate;
        let candidates = vec![
            Candidate { kind: AgentKind::Omp, pid: 11, depth: 1 },
            Candidate { kind: AgentKind::ClaudeCode, pid: 12, depth: 2 },
        ];
        // No screen corroboration → deepest (claude).
        assert_eq!(pick_agent(&candidates, &lines(&["random"]), None), Some(AgentKind::ClaudeCode));
        // Screen shows omp's update banner ("Run: omp update") → omp wins.
        assert_eq!(
            pick_agent(&candidates, &lines(&["Update Available", "New version 17.2.9 is available. Run: omp update"]), None),
            Some(AgentKind::Omp)
        );
        // No candidates → None.
        assert_eq!(pick_agent(&[], &lines(&["x"]), None), None);
    }

    #[test]
    fn detector_combines_process_and_screen() {
        let mut detector = Detector::new();
        assert_eq!(detector.evaluate(&lines(&["x"]), None), None);
        detector.candidates.push(Candidate { kind: AgentKind::ClaudeCode, pid: 1, depth: 1 });
        let det = detector.evaluate(&lines(CLAUDE_IDLE), Some("✳ Claude Code")).unwrap();
        assert_eq!(det.agent, AgentKind::ClaudeCode);
        assert_eq!(det.state, AgentState::Idle);
        // Process gone → detection clears even if the screen lingers.
        detector.candidates.clear();
        assert_eq!(detector.evaluate(&lines(CLAUDE_IDLE), Some("✳ Claude Code")), None);
    }
}
