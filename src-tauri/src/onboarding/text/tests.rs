//! Scripted runs of the terminal flow: a fake terminal feeds the answers, a fake machine records
//! what the flow did to it.

use super::*;
use crate::onboarding::agents;
use crate::onboarding::detect::Signals;
use crate::onboarding::env::{Installs, PathFix, Transient};
use crate::onboarding::plan::{PlannedByMode, PlannedFile};
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;

// ——— fakes ———

struct FakeIo {
    answers: VecDeque<String>,
    said: Vec<String>,
    noted: Vec<String>,
    prompts: Vec<String>,
}

impl FakeIo {
    fn new(answers: &[&str]) -> Self {
        Self {
            answers: answers.iter().map(|answer| answer.to_string()).collect(),
            said: Vec::new(),
            noted: Vec::new(),
            prompts: Vec::new(),
        }
    }

    fn said(&self) -> String {
        self.said.join("\n")
    }

    fn prompted(&self, needle: &str) -> bool {
        self.prompts.iter().any(|prompt| prompt.contains(needle))
    }
}

impl TextIo for FakeIo {
    fn say(&mut self, line: &str) {
        self.said.push(line.to_string());
    }

    fn note(&mut self, line: &str) {
        self.noted.push(line.to_string());
    }

    fn ask(&mut self, prompt: &str) -> Result<String, String> {
        self.prompts.push(prompt.to_string());
        self.answers
            .pop_front()
            .ok_or_else(|| "input closed".to_string())
    }
}

struct FakeDeps {
    rows: Vec<AgentScanRow>,
    env: EnvReport,
    read_only: bool,
    config: AppConfig,
    channels: Vec<ChannelStatus>,
    missing_user_id: HashSet<&'static str>,
    tray: bool,
    agents_decided: bool,
    apply_errors: HashMap<&'static str, &'static str>,
    codex_delivery: Option<&'static str>,
    configure_fails: bool,
    test_fails: bool,
    /// Every side effect, in order.
    calls: Vec<String>,
    /// `(agent, mode)` of every selection handed to `apply_agents`, in order.
    applied: Vec<(String, String)>,
}

fn home_file(relative: &str) -> PathBuf {
    crate::paths::home().join(relative)
}

fn row(id: &str, detected: bool, mode: &str) -> AgentScanRow {
    let def = agents::find(id).expect("known agent");
    let planned = |supported: bool, tag: &str| {
        if supported {
            vec![PlannedFile {
                path: home_file(&format!(".{id}/{tag}.md")),
                exists: false,
            }]
        } else {
            Vec::new()
        }
    };
    AgentScanRow {
        id: id.to_string(),
        title: def.title.to_string(),
        signals: Signals {
            cli: detected,
            ..Signals::default()
        },
        detected,
        mode: mode.to_string(),
        needs_update: false,
        status_error: None,
        supports_cli: def.cli,
        supports_mcp: def.mcp,
        recommended: def.recommended.as_str().to_string(),
        delivery: None,
        planned: PlannedByMode {
            cli: planned(def.cli, "cli"),
            mcp: planned(def.mcp, "mcp"),
        },
    }
}

/// Every Agent the platform offers; `detected` lists the installed ones, `integrated` the ones
/// already set up as `(id, mode)`.
fn rows(detected: &[&str], integrated: &[(&str, &str)]) -> Vec<AgentScanRow> {
    agents::available()
        .into_iter()
        .map(|def| {
            let mode = integrated
                .iter()
                .find(|(id, _)| *id == def.id)
                .map_or("none", |(_, mode)| *mode);
            row(def.id, detected.contains(&def.id) || mode != "none", mode)
        })
        .collect()
}

fn channel_rows() -> Vec<ChannelStatus> {
    channels::ORDER
        .iter()
        .map(|id| ChannelStatus {
            id: id.to_string(),
            enabled: false,
            configured: false,
            connected: None,
        })
        .collect()
}

impl FakeDeps {
    /// A new user with Cursor and Claude Code installed on a healthy machine.
    fn fresh() -> Self {
        Self {
            rows: rows(&["cursor", "claude"], &[]),
            env: EnvReport::empty(),
            read_only: false,
            config: AppConfig::default(),
            channels: channel_rows(),
            missing_user_id: HashSet::new(),
            tray: true,
            agents_decided: false,
            apply_errors: HashMap::new(),
            codex_delivery: None,
            configure_fails: false,
            test_fails: false,
            calls: Vec::new(),
            applied: Vec::new(),
        }
    }

    fn with_rows(mut self, rows: Vec<AgentScanRow>) -> Self {
        self.rows = rows;
        self
    }

    fn channel_mut(&mut self, id: &str) -> &mut ChannelStatus {
        self.channels
            .iter_mut()
            .find(|row| row.id == id)
            .expect("known channel")
    }

    fn called(&self, call: &str) -> bool {
        self.calls.iter().any(|candidate| candidate == call)
    }

    fn position(&self, prefix: &str) -> usize {
        self.calls
            .iter()
            .position(|call| call.starts_with(prefix))
            .unwrap_or_else(|| panic!("no call starting with {prefix:?} in {:?}", self.calls))
    }
}

impl TextDeps for FakeDeps {
    fn scan(&mut self) -> Vec<AgentScanRow> {
        self.rows.clone()
    }

    fn env_report(&mut self) -> EnvReport {
        self.env.clone()
    }

    fn agents_read_only(&mut self) -> bool {
        self.read_only
    }

    fn load_config(&mut self) -> AppConfig {
        self.config.clone()
    }

    fn save_config(&mut self, config: &AppConfig) -> Result<(), String> {
        self.calls.push("save_config".to_string());
        self.config = config.clone();
        Ok(())
    }

    fn apply_style_to_integrations(&mut self) -> Result<(), String> {
        self.calls.push("apply_style".to_string());
        Ok(())
    }

    fn apply_agents(&mut self, selections: &[Selection]) -> Vec<ApplyResult> {
        self.calls.push(format!(
            "apply {}",
            selections
                .iter()
                .map(|selection| format!("{}:{}", selection.agent, selection.mode))
                .collect::<Vec<_>>()
                .join(" ")
        ));
        let mut results = Vec::new();
        for selection in selections {
            self.applied
                .push((selection.agent.clone(), selection.mode.clone()));
            let error = self
                .apply_errors
                .get(selection.agent.as_str())
                .map(|error| error.to_string());
            let ok = error.is_none();
            if ok {
                self.agents_decided = true;
                if let Some(row) = self.rows.iter_mut().find(|row| row.id == selection.agent) {
                    row.mode = selection.mode.clone();
                    row.needs_update = false;
                }
            }
            results.push(ApplyResult {
                agent: selection.agent.clone(),
                requested: selection.mode.clone(),
                ok,
                error,
                mode_after: selection.mode.clone(),
                delivery: (selection.agent == "codex")
                    .then(|| self.codex_delivery.map(str::to_string))
                    .flatten(),
            });
        }
        results
    }

    fn channel_rows(&mut self) -> Vec<ChannelStatus> {
        self.channels.clone()
    }

    fn configure_channel(&mut self, id: &str) -> Result<(), String> {
        self.calls.push(format!("configure {id}"));
        if self.configure_fails {
            return Err("the wizard could not save".to_string());
        }
        let can_detect = channels::can_detect_user_id(id);
        let needs_id = self.missing_user_id.contains(id);
        let row = self.channel_mut(id);
        row.enabled = true;
        // A channel that still lacks its user id is not fully configured until detection fills it.
        row.configured = !(can_detect && needs_id);
        Ok(())
    }

    fn user_id_missing(&mut self, id: &str) -> bool {
        self.missing_user_id.contains(id)
    }

    fn detect_user_id(&mut self, id: &str) -> Result<(), String> {
        self.calls.push(format!("detect {id}"));
        self.missing_user_id.remove(id);
        self.channel_mut(id).configured = true;
        Ok(())
    }

    fn test_channel(&mut self, id: &str) -> Result<(), String> {
        self.calls.push(format!("test {id}"));
        if self.test_fails {
            Err("no answer from the platform".to_string())
        } else {
            Ok(())
        }
    }

    fn tray_supported(&mut self) -> bool {
        self.tray
    }

    fn login_item(&mut self, install: bool) -> Result<(), String> {
        self.calls
            .push(format!("login_item {}", if install { "on" } else { "off" }));
        Ok(())
    }

    fn mark_skipped(&mut self, domain: &str) {
        self.calls.push(format!("skipped {domain}"));
        if domain == DOMAIN_AGENTS {
            self.agents_decided = true;
        }
    }

    fn mark_background_decided(&mut self) {
        self.calls.push("background_decided".to_string());
    }

    fn agents_decided(&mut self) -> bool {
        self.agents_decided
    }
}

fn run_en(deps: &mut FakeDeps, answers: &[&str]) -> (i32, FakeIo) {
    let mut io = FakeIo::new(answers);
    let code = run(Lang::En, &mut io, deps);
    (code, io)
}

fn row_mut<'a>(rows: &'a mut [AgentScanRow], id: &str) -> &'a mut AgentScanRow {
    rows.iter_mut()
        .find(|row| row.id == id)
        .expect("known agent row")
}

/// Collapse runs of whitespace so assertions do not depend on column alignment.
fn squeezed(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

// ——— pure helpers ———

#[test]
fn the_confirmation_prompt_understands_its_answers() {
    for (answer, offer_each, expected) in [
        ("", true, Choice::Confirm),
        ("y", true, Choice::Confirm),
        (" YES ", true, Choice::Confirm),
        ("n", true, Choice::Each),
        ("n", false, Choice::Unknown),
        ("s", true, Choice::Skip),
        ("Skip", false, Choice::Skip),
        ("1", true, Choice::One(0)),
        ("3", true, Choice::One(2)),
        ("6", true, Choice::One(5)),
        ("7", true, Choice::Unknown),
        ("0", true, Choice::Unknown),
        ("maybe", true, Choice::Unknown),
    ] {
        assert_eq!(parse_choice(answer, 6, offer_each), expected, "{answer:?}");
    }
}

#[test]
fn starting_choices_follow_what_is_installed_and_integrated() {
    let target = |detected: bool, mode: &str, id: &str| initial_target(&row(id, detected, mode));
    // Installed but not integrated: the recommended mode.
    assert_eq!(target(true, "none", "cursor"), Some(Mode::Cli));
    assert_eq!(target(true, "none", "codex"), Some(Mode::Mcp));
    // Integrated: keep its mode, whatever the recommendation is.
    assert_eq!(target(true, "mcp", "cursor"), Some(Mode::Mcp));
    assert_eq!(target(false, "cli", "codex"), Some(Mode::Cli));
    // Not installed: left alone.
    assert_eq!(target(false, "none", "cursor"), None);
    // An unreadable state never gets a target.
    let mut broken = row("claude", true, "none");
    broken.status_error = Some("unreadable".to_string());
    assert_eq!(initial_target(&broken), None);
}

#[test]
fn home_paths_are_abbreviated() {
    let home = Path::new("/Users/me");
    assert_eq!(
        tilde(Path::new("/Users/me/.claude/CLAUDE.md"), home),
        "~/.claude/CLAUDE.md"
    );
    assert_eq!(tilde(Path::new("/Users/me"), home), "~");
    assert_eq!(tilde(Path::new("/etc/hosts"), home), "/etc/hosts");
}

#[test]
fn columns_line_up_by_terminal_cells_not_by_characters() {
    // Chinese characters take two cells, so a two-character name pads as a four-character one.
    assert_eq!(cells("Telegram"), 8);
    assert_eq!(cells("钉钉"), 4);
    assert_eq!(cells(""), 0);
    assert_eq!(pad_cells("Slack", 9), "Slack    ");
    assert_eq!(pad_cells("钉钉", 9), "钉钉     ");
    assert_eq!(
        cells(&pad_cells("飞书", 9)),
        cells(&pad_cells("Telegram", 9))
    );
    // A name wider than the column is left whole.
    assert_eq!(pad_cells("Antigravity", 9), "Antigravity");
}

#[test]
fn the_channel_list_lines_up_in_chinese() {
    let mut deps = FakeDeps::fresh();
    let mut io = FakeIo::new(&["", "", "", ""]);
    assert_eq!(run(Lang::Zh, &mut io, &mut deps), 0);
    let lines: Vec<&String> = io
        .said
        .iter()
        .filter(|line| {
            line.trim_start().starts_with(|c: char| c.is_ascii_digit()) && line.contains("未配置")
        })
        .collect();
    assert_eq!(lines.len(), channels::ORDER.len(), "{lines:?}");
    // Where the state column starts, in terminal cells.
    let state_column = |line: &str| cells(&line[..line.find("未配置").unwrap()]);
    let first = state_column(lines[0]);
    assert!(
        lines.iter().all(|line| state_column(line) == first),
        "{lines:?}"
    );
}

// ——— step 1: Agents ———

#[test]
fn a_new_user_pressing_enter_gets_the_recommended_setup() {
    let mut deps = FakeDeps::fresh();
    // apply, keep the style, skip remote answering, keep the menu-bar icon
    let (code, io) = run_en(&mut deps, &["", "", "", ""]);
    assert_eq!(code, 0);
    assert_eq!(
        deps.applied,
        [
            ("cursor".to_string(), "cli".to_string()),
            ("claude".to_string(), "cli".to_string())
        ]
    );
    assert!(deps.called("skipped remote"), "{:?}", deps.calls);
    assert!(
        !deps.called("skipped agents"),
        "applying decides the Agent domain"
    );
    assert!(deps.called("background_decided"));
    assert!(deps.called("login_item on"));
    // Nothing else was written: the style and the icon stayed as they were.
    assert!(!deps.called("save_config"), "{:?}", deps.calls);
    let said = io.said();
    assert!(said.contains("✓ Cursor (CLI) is set up"), "{said}");
    assert!(said.contains("✓ Claude Code (CLI) is set up"), "{said}");
    assert!(said.contains("must be restarted"), "{said}");
}

#[test]
fn detected_agents_are_listed_first_and_marked() {
    let mut deps = FakeDeps::fresh().with_rows(rows(&["claude"], &[("pi", "cli")]));
    let (_, io) = run_en(&mut deps, &["s", "s", ""]);
    let list: Vec<&String> = io
        .said
        .iter()
        .filter(|line| line.starts_with("  1)") || line.starts_with("  2)"))
        .collect();
    // Pi is integrated and Claude Code detected: both come before everything else.
    assert!(list[0].contains("Claude Code"), "{list:?}");
    assert!(list[0].contains("will integrate in CLI mode (recommended)"));
    assert!(list[1].contains("Pi"), "{list:?}");
    assert!(list[1].contains("integrated (CLI)"));
    let others = io.said().contains("not detected");
    assert!(others);
}

#[test]
fn the_preview_lists_what_will_change_and_marks_new_and_modified_files() {
    let mut deps = FakeDeps::fresh();
    row_mut(&mut deps.rows, "cursor").planned.cli = vec![
        PlannedFile {
            path: home_file(".cursor/rules/askhuman.mdc"),
            exists: false,
        },
        PlannedFile {
            path: home_file(".cursor/hooks.json"),
            exists: true,
        },
    ];
    let (_, io) = run_en(&mut deps, &["s", "s", ""]);
    let said = io.said();
    assert!(said.contains("Files that will change:"), "{said}");
    assert!(said.contains("Cursor (CLI)"), "{said}");
    assert!(
        said.contains("new    ~/.cursor/rules/askhuman.mdc"),
        "{said}"
    );
    assert!(said.contains("modify ~/.cursor/hooks.json"), "{said}");
}

#[test]
fn rerunning_with_nothing_to_apply_asks_no_confirmation() {
    let mut deps = FakeDeps::fresh().with_rows(rows(&[], &[("cursor", "cli"), ("claude", "mcp")]));
    deps.agents_decided = true;
    // continue, keep the style, skip remote answering, keep the icon
    let (code, io) = run_en(&mut deps, &["", "", "", ""]);
    assert_eq!(code, 0);
    assert!(deps.applied.is_empty());
    assert!(!deps.calls.iter().any(|call| call.starts_with("apply ")));
    assert!(!io.prompted("Apply these changes?"), "{:?}", io.prompts);
    assert!(io.prompted("[Enter continue"));
    assert!(io.said().contains("up to date"));
    // The existing integrations are listed as they are.
    assert!(io.said().contains("integrated (CLI)"));
    assert!(io.said().contains("integrated (MCP)"));
}

#[test]
fn an_outdated_integration_is_updated_in_place() {
    let mut scanned = rows(&[], &[("cursor", "cli")]);
    row_mut(&mut scanned, "cursor").needs_update = true;
    let mut deps = FakeDeps::fresh().with_rows(scanned);
    deps.agents_decided = true;
    let (_, io) = run_en(&mut deps, &["", "", "", ""]);
    assert!(io.said().contains("update available → will update"));
    assert_eq!(deps.applied, [("cursor".to_string(), "cli".to_string())]);
}

#[test]
fn n_walks_through_every_agent_and_applies_the_adjusted_choices() {
    let mut deps = FakeDeps::fresh();
    // `n`, then one question per Agent in list order: Cursor → MCP, the rest keep their default.
    let mut answers = vec!["n", "m"];
    answers.extend(std::iter::repeat_n("", agents::available().len() - 1));
    answers.extend(["", "", "", ""]); // confirm, style, remote, icon
    let (code, io) = run_en(&mut deps, &answers);
    assert_eq!(code, 0, "{:?}", io.noted);
    assert_eq!(
        deps.applied,
        [
            ("cursor".to_string(), "mcp".to_string()),
            ("claude".to_string(), "cli".to_string())
        ]
    );
}

#[test]
fn a_number_adjusts_only_that_agent() {
    let mut deps = FakeDeps::fresh();
    // Claude Code is the second row; give it MCP, then confirm.
    let (_, io) = run_en(&mut deps, &["2", "m", "", "", "", ""]);
    assert_eq!(
        deps.applied,
        [
            ("cursor".to_string(), "cli".to_string()),
            ("claude".to_string(), "mcp".to_string())
        ]
    );
    let mode_question = io
        .prompts
        .iter()
        .find(|prompt| prompt.starts_with("Claude Code: mode"))
        .expect("the mode question");
    assert!(mode_question.contains("c=CLI / m=MCP"), "{mode_question}");
}

#[test]
fn only_the_modes_an_agent_offers_can_be_chosen() {
    // Grok has no CLI mode: `c` is refused and asked again.
    let mut deps = FakeDeps::fresh().with_rows(rows(&["grok"], &[]));
    let (_, io) = run_en(&mut deps, &["1", "c", "", "", "", "", ""]);
    let question = io
        .prompts
        .iter()
        .find(|prompt| prompt.starts_with("Grok: mode"))
        .expect("the mode question");
    assert!(
        question.contains("m=MCP") && !question.contains("c=CLI"),
        "{question}"
    );
    assert!(io.said().contains("not available here"));
    assert_eq!(deps.applied, [("grok".to_string(), "mcp".to_string())]);
}

#[test]
fn x_leaves_an_agent_alone_even_when_it_is_detected() {
    let mut deps = FakeDeps::fresh();
    let (_, _) = run_en(&mut deps, &["1", "x", "", "", "", ""]);
    assert_eq!(deps.applied, [("claude".to_string(), "cli".to_string())]);
}

#[test]
fn skipping_the_agent_step_is_remembered_and_writes_nothing() {
    let mut deps = FakeDeps::fresh();
    // skip, skip remote answering, keep the icon
    let (code, io) = run_en(&mut deps, &["s", "", ""]);
    assert_eq!(code, 0);
    assert!(deps.applied.is_empty());
    assert!(deps.called("skipped agents"));
    assert!(!io.prompted("Choose [1]"), "no style question after a skip");
}

#[test]
fn skipping_does_not_overwrite_an_agent_domain_that_is_already_decided() {
    let mut deps = FakeDeps::fresh().with_rows(rows(&[], &[("cursor", "cli")]));
    deps.agents_decided = true;
    let (_, _) = run_en(&mut deps, &["s", "", ""]);
    assert!(!deps.called("skipped agents"), "{:?}", deps.calls);
}

#[test]
fn unclear_answers_are_asked_again() {
    let mut deps = FakeDeps::fresh();
    let (_, io) = run_en(&mut deps, &["what", "9", "", "", "", ""]);
    assert_eq!(io.said().matches("not one of the choices").count(), 2);
    assert_eq!(deps.applied.len(), 2);
}

#[test]
fn an_unreadable_agent_is_shown_but_never_touched() {
    let mut scanned = rows(&["cursor"], &[]);
    let claude = row_mut(&mut scanned, "claude");
    claude.status_error = Some("~/.claude is a link".to_string());
    claude.detected = true;
    let mut deps = FakeDeps::fresh().with_rows(scanned);
    // `n` asks about every Agent but the unreadable one; then confirm, style, remote, icon.
    let mut answers = vec!["n"];
    answers.extend(std::iter::repeat_n("", agents::available().len() - 1 + 4));
    let (_, io) = run_en(&mut deps, &answers);
    assert!(io
        .said()
        .contains("cannot read its state: ~/.claude is a link"));
    assert_eq!(deps.applied, [("cursor".to_string(), "cli".to_string())]);
    assert!(io.noted.iter().any(|note| note.contains("left alone")));
}

#[test]
fn a_failed_agent_is_reported_and_the_others_still_apply() {
    let mut deps = FakeDeps::fresh();
    deps.apply_errors.insert("claude", "disk full");
    let (code, io) = run_en(&mut deps, &["", "", "", ""]);
    assert_eq!(code, 0);
    let said = io.said();
    assert!(said.contains("✓ Cursor (CLI) is set up"), "{said}");
    assert!(said.contains("✗ Claude Code: disk full"), "{said}");
    assert!(
        said.contains("must be restarted"),
        "one success is enough for the reminder"
    );
}

#[test]
fn nothing_succeeding_prints_no_restart_reminder() {
    let mut deps = FakeDeps::fresh();
    deps.apply_errors.insert("cursor", "disk full");
    deps.apply_errors.insert("claude", "disk full");
    let (_, io) = run_en(&mut deps, &["", "", "", ""]);
    assert!(!io.said().contains("must be restarted"));
}

#[test]
fn codex_and_cursor_notes_follow_what_was_applied() {
    let mut deps = FakeDeps::fresh().with_rows(rows(&["cursor", "codex"], &[]));
    deps.codex_delivery = Some("untrusted");
    let (_, io) = run_en(&mut deps, &["", "", "", ""]);
    let said = io.said();
    assert!(said.contains("Codex: trust the AskHuman hook"), "{said}");
    assert!(
        said.contains("Cursor: the global rule only applies"),
        "{said}"
    );

    // A trusted Codex and no Cursor: neither note.
    let mut deps = FakeDeps::fresh().with_rows(rows(&["claude", "codex"], &[]));
    deps.codex_delivery = Some("trusted");
    let (_, io) = run_en(&mut deps, &["", "", "", ""]);
    let said = io.said();
    assert!(!said.contains("trust the AskHuman hook"), "{said}");
    assert!(!said.contains("Cursor:"), "{said}");
}

#[test]
fn an_integrated_codex_that_is_not_trusted_yet_is_tagged() {
    let mut scanned = rows(&[], &[("codex", "mcp")]);
    row_mut(&mut scanned, "codex").delivery = Some("untrusted".to_string());
    let mut deps = FakeDeps::fresh().with_rows(scanned);
    deps.agents_decided = true;
    let (_, io) = run_en(&mut deps, &["", "", "", ""]);
    assert!(io.said().contains("[needs trust]"));
}

#[test]
fn a_dev_instance_skips_the_agent_step_entirely() {
    let mut deps = FakeDeps::fresh();
    deps.read_only = true;
    // remote answering skipped, icon kept
    let (code, io) = run_en(&mut deps, &["", ""]);
    assert_eq!(code, 0);
    assert!(io.said().contains("Dev instance"));
    assert!(deps.applied.is_empty());
    assert!(!io.prompted("Choose [1]"), "no style question either");
    // The later steps still run.
    assert!(deps.called("background_decided"));
}

// ——— collaboration style ———

#[test]
fn a_style_change_is_saved_and_pushed_before_the_agents_are_applied() {
    let mut deps = FakeDeps::fresh();
    let (_, io) = run_en(&mut deps, &["", "2", "", ""]);
    assert_eq!(
        deps.config.general.collaboration_style,
        CollaborationStyle::Autonomous
    );
    let saved = deps.position("save_config");
    let pushed = deps.position("apply_style");
    let applied = deps.position("apply cursor");
    assert!(saved < pushed && pushed < applied, "{:?}", deps.calls);
    assert!(io.said().contains("1) Aligned (current)"), "{}", io.said());
    assert!(io.prompted("Choose [1]"));
}

#[test]
fn keeping_the_style_writes_nothing() {
    let mut deps = FakeDeps::fresh();
    deps.config.general.collaboration_style = CollaborationStyle::Autonomous;
    let (_, io) = run_en(&mut deps, &["", "", "", ""]);
    assert!(io.prompted("Choose [2]"), "Enter keeps the current style");
    assert!(!deps.called("save_config"));
    assert!(!deps.called("apply_style"));
    assert!(io.said().contains("2) Autonomous (current)"));
}

#[test]
fn a_custom_style_is_offered_as_current_and_can_be_kept_or_replaced() {
    let mut deps = FakeDeps::fresh();
    deps.config.general.collaboration_style = CollaborationStyle::Custom;
    let (_, io) = run_en(&mut deps, &["", "", "", ""]);
    assert!(io.said().contains("3) Custom (current)"));
    assert!(io.prompted("Choose [3]"));
    assert_eq!(
        deps.config.general.collaboration_style,
        CollaborationStyle::Custom
    );
    assert!(!deps.called("apply_style"));

    let mut deps = FakeDeps::fresh();
    deps.config.general.collaboration_style = CollaborationStyle::Custom;
    let (_, _) = run_en(&mut deps, &["", "1", "", ""]);
    assert_eq!(
        deps.config.general.collaboration_style,
        CollaborationStyle::Aligned
    );
    assert!(deps.called("apply_style"));
}

#[test]
fn option_three_is_refused_unless_the_style_is_custom() {
    let mut deps = FakeDeps::fresh();
    let (_, io) = run_en(&mut deps, &["", "3", "", "", ""]);
    assert!(io.said().contains("not one of the choices"));
}

// ——— environment ———

#[test]
fn a_healthy_machine_prints_no_warnings() {
    let mut deps = FakeDeps::fresh();
    let (_, io) = run_en(&mut deps, &["", "", "", ""]);
    assert!(
        !io.said.iter().any(|line| line.starts_with('!')),
        "{}",
        io.said()
    );
}

#[test]
fn a_binary_in_a_temporary_place_and_several_builds_are_reported_up_front() {
    let mut deps = FakeDeps::fresh();
    deps.env.transient = Some(Transient {
        path: "/tmp/AskHuman".to_string(),
        kind: "temp".to_string(),
    });
    deps.env.installs = Some(Installs {
        message: "Two builds of AskHuman are installed. Details: AskHuman doctor install"
            .to_string(),
    });
    let (_, io) = run_en(&mut deps, &["", "", "", ""]);
    let said = io.said();
    let transient = said
        .find("a temporary directory (/tmp/AskHuman)")
        .expect("transient card");
    let installs = said.find("! Two builds").expect("installs card");
    let first_step = said.find("[1/3]").unwrap();
    assert!(transient < first_step && installs < first_step, "{said}");
}

fn missing_on_path() -> EnvReport {
    EnvReport {
        askhuman_on_path: "missing".to_string(),
        path_fix: Some(PathFix {
            command: "ln -sf '/opt/AskHuman' '/usr/local/bin/AskHuman'".to_string(),
            dir: "/usr/local/bin".to_string(),
            needs_path_edit: false,
        }),
        transient: None,
        installs: None,
    }
}

#[test]
fn the_path_card_appears_after_the_choice_when_a_cli_agent_needs_it() {
    let mut deps = FakeDeps::fresh();
    deps.env = missing_on_path();
    let (_, io) = run_en(&mut deps, &["", "", "", ""]);
    let said = io.said();
    let card = said
        .find("cannot find the `AskHuman` command")
        .expect("PATH card");
    assert!(said.contains("ln -sf '/opt/AskHuman' '/usr/local/bin/AskHuman'"));
    // It comes after the list and before the results.
    assert!(said.find("Agents on this machine").unwrap() < card);
    assert!(card < said.find("is set up").unwrap());
}

#[test]
fn no_path_card_when_no_agent_uses_the_cli_mode() {
    // Codex is the only Agent and goes to MCP.
    let mut deps = FakeDeps::fresh().with_rows(rows(&["codex"], &[]));
    deps.env = missing_on_path();
    let (_, io) = run_en(&mut deps, &["", "", "", ""]);
    assert!(!io.said().contains("cannot find the `AskHuman` command"));

    // Skipping the step means nothing will use it either.
    let mut deps = FakeDeps::fresh();
    deps.env = missing_on_path();
    let (_, io) = run_en(&mut deps, &["s", "", ""]);
    assert!(!io.said().contains("cannot find the `AskHuman` command"));
}

#[test]
fn an_integrated_cli_agent_alone_is_enough_for_the_path_card() {
    let mut deps = FakeDeps::fresh().with_rows(rows(&[], &[("claude", "cli")]));
    deps.agents_decided = true;
    deps.env = missing_on_path();
    let (_, io) = run_en(&mut deps, &["", "", "", ""]);
    assert!(io.said().contains("cannot find the `AskHuman` command"));
}

#[test]
fn the_path_card_mentions_the_profile_when_the_directory_is_not_on_path() {
    let mut deps = FakeDeps::fresh();
    deps.env = missing_on_path();
    deps.env.path_fix = Some(PathFix {
        command: "mkdir -p ~/.local/bin && ln -sf '/opt/AskHuman' ~/.local/bin/AskHuman"
            .to_string(),
        dir: "~/.local/bin".to_string(),
        needs_path_edit: true,
    });
    let (_, io) = run_en(&mut deps, &["", "", "", ""]);
    assert!(io.said().contains("add ~/.local/bin to PATH"));
}

// ——— step 2: remote answering ———

#[test]
fn leaving_remote_answering_with_no_channel_enabled_counts_as_skipped() {
    let mut deps = FakeDeps::fresh();
    let (_, io) = run_en(&mut deps, &["", "", "s", ""]);
    assert!(deps.called("skipped remote"));
    let said = io.said();
    assert!(said.contains("1) Telegram"), "{said}");
    assert!(said.contains("2) Slack"), "{said}");
    assert!(said.contains("3) DingTalk"), "{said}");
    assert!(said.contains("4) Feishu"), "{said}");
    assert!(said.contains("not configured"));
}

#[test]
fn configuring_a_channel_keeps_the_remote_domain_open_for_the_popup_strip() {
    let mut deps = FakeDeps::fresh();
    // apply, style, Telegram, no test, leave | icon, keep-alive (default no)
    let (code, io) = run_en(&mut deps, &["", "", "1", "", "s", "", ""]);
    assert_eq!(code, 0, "{:?}", io.noted);
    assert!(deps.called("configure telegram"));
    assert!(
        !deps.called("skipped remote"),
        "an enabled channel decides the domain"
    );
    assert!(io.prompted("Test the connection?"));
    assert!(io.said().contains("1) Telegram  enabled"), "{}", io.said());
    assert!(io.prompted("Keep AskHuman running in the background"));
}

#[test]
fn a_channel_without_a_user_id_is_offered_detection_and_then_a_test() {
    let mut deps = FakeDeps::fresh();
    deps.missing_user_id.insert("dingding");
    // apply, style, DingTalk, detect (yes), test (yes), leave | icon, keep-alive
    let (_, io) = run_en(&mut deps, &["", "", "3", "", "y", "s", "", ""]);
    assert!(io.prompted("Detect your user id automatically?"));
    let configure = deps.position("configure dingding");
    let detect = deps.position("detect dingding");
    let test = deps.position("test dingding");
    assert!(configure < detect && detect < test, "{:?}", deps.calls);
    assert!(io.said().contains("✓ The test went through."));
}

#[test]
fn declining_detection_leaves_the_channel_incomplete_and_points_to_channel_set() {
    let mut deps = FakeDeps::fresh();
    deps.missing_user_id.insert("feishu");
    // apply, style, Feishu, detect (no), leave | icon, keep-alive
    let (_, io) = run_en(&mut deps, &["", "", "4", "n", "s", "", ""]);
    assert!(!deps.called("detect feishu"));
    assert!(!deps.called("test feishu"));
    assert!(
        !io.prompted("Test the connection?"),
        "an incomplete channel is not tested"
    );
    assert!(io.said().contains("AskHuman channel set feishu"));
}

#[test]
fn telegram_is_never_offered_user_id_detection() {
    let mut deps = FakeDeps::fresh();
    deps.missing_user_id.insert("telegram");
    let (_, io) = run_en(&mut deps, &["", "", "1", "", "s", "", ""]);
    assert!(!io.prompted("Detect your user id automatically?"));
    assert!(!deps.called("detect telegram"));
}

#[test]
fn a_failing_test_is_reported_and_the_flow_goes_on() {
    let mut deps = FakeDeps::fresh();
    deps.test_fails = true;
    let (code, io) = run_en(&mut deps, &["", "", "2", "y", "s", "", ""]);
    assert_eq!(code, 0);
    assert!(io
        .said()
        .contains("✗ The test failed: no answer from the platform"));
}

#[test]
fn a_failing_channel_wizard_returns_to_the_list() {
    let mut deps = FakeDeps::fresh();
    deps.configure_fails = true;
    let (code, io) = run_en(&mut deps, &["", "", "1", "s", ""]);
    assert_eq!(code, 0);
    assert!(io.said().contains("✗ the wizard could not save"));
    assert!(deps.called("skipped remote"), "nothing got enabled");
}

#[test]
fn an_unknown_channel_number_is_asked_again() {
    let mut deps = FakeDeps::fresh();
    let (_, io) = run_en(&mut deps, &["", "", "7", "s", ""]);
    assert!(io.said().contains("not one of the choices"));
}

// ——— step 3: background ———

#[test]
fn the_menu_bar_question_is_asked_only_where_a_tray_exists() {
    let mut deps = FakeDeps::fresh();
    deps.tray = false;
    // apply, style, skip remote — and no icon question
    let (code, io) = run_en(&mut deps, &["", "", ""]);
    assert_eq!(code, 0);
    assert!(!io.prompted("menu-bar icon"));
    assert!(!deps.called("login_item on"));
    assert!(deps.called("background_decided"));
}

#[test]
fn declining_the_menu_bar_icon_removes_the_login_item_from_an_always_setup() {
    let mut deps = FakeDeps::fresh();
    assert_eq!(deps.config.general.menu_bar_icon, MenuBarIconMode::Always);
    let (_, _) = run_en(&mut deps, &["", "", "", "n"]);
    assert_eq!(deps.config.general.menu_bar_icon, MenuBarIconMode::Active);
    assert!(deps.called("login_item off"));
    assert!(deps.called("save_config"));
}

#[test]
fn declining_the_menu_bar_icon_changes_nothing_for_active_or_off() {
    for current in [MenuBarIconMode::Active, MenuBarIconMode::Off] {
        let mut deps = FakeDeps::fresh();
        deps.config.general.menu_bar_icon = current;
        let (_, io) = run_en(&mut deps, &["", "", "", "n"]);
        assert_eq!(deps.config.general.menu_bar_icon, current);
        assert!(!deps.called("login_item off"), "{current:?}");
        assert!(!deps.called("save_config"), "{current:?}");
        assert!(
            io.prompted("[y/N]"),
            "the default follows the current mode ({current:?})"
        );
    }
}

#[test]
fn accepting_the_menu_bar_icon_switches_an_off_setup_to_always_with_a_login_item() {
    let mut deps = FakeDeps::fresh();
    deps.config.general.menu_bar_icon = MenuBarIconMode::Off;
    let (_, _) = run_en(&mut deps, &["", "", "", "y"]);
    assert_eq!(deps.config.general.menu_bar_icon, MenuBarIconMode::Always);
    assert!(deps.called("login_item on"));
    assert!(deps.called("save_config"));
}

#[test]
fn the_keep_alive_question_is_asked_only_when_a_channel_is_enabled() {
    let mut deps = FakeDeps::fresh();
    let (_, io) = run_en(&mut deps, &["", "", "", ""]);
    assert!(!io.prompted("Keep AskHuman running in the background"));

    let mut deps = FakeDeps::fresh();
    deps.channel_mut("slack").enabled = true;
    deps.channel_mut("slack").configured = true;
    // apply, style, leave remote untouched, icon, keep-alive → yes
    let (_, io) = run_en(&mut deps, &["", "", "s", "", "y"]);
    assert!(io.prompted("Keep AskHuman running in the background"));
    assert_eq!(
        deps.config.general.daemon_lifecycle,
        DaemonLifecycleMode::KeepAlive
    );
    assert!(deps.called("save_config"));
}

#[test]
fn keep_alive_can_be_turned_off_again() {
    let mut deps = FakeDeps::fresh();
    deps.config.general.daemon_lifecycle = DaemonLifecycleMode::KeepAlive;
    deps.channel_mut("telegram").enabled = true;
    let (_, io) = run_en(&mut deps, &["", "", "s", "", "n"]);
    let question = io
        .prompts
        .iter()
        .find(|prompt| prompt.starts_with("Keep AskHuman running in the background"))
        .expect("the keep-alive question");
    assert!(
        question.ends_with("[Y/n]: "),
        "the default follows the current setting: {question}"
    );
    assert_eq!(
        deps.config.general.daemon_lifecycle,
        DaemonLifecycleMode::Activity
    );
}

#[test]
fn an_agent_step_that_was_neither_applied_nor_skipped_is_closed_at_the_end() {
    // Nothing detected, Enter all the way: no Agent is integrated, so the domain is closed by the
    // end of the wizard (the user has seen the step).
    let mut deps = FakeDeps::fresh().with_rows(rows(&[], &[]));
    let (code, io) = run_en(&mut deps, &["", "", "", ""]);
    assert_eq!(code, 0);
    assert!(io.said().contains("No Agent was detected"));
    assert!(deps.called("skipped agents"));
    let end = deps.position("skipped agents");
    assert!(end > deps.position("skipped remote"), "{:?}", deps.calls);
}

// ——— whole-run behaviour ———

#[test]
fn end_of_input_stops_with_exit_one_and_keeps_what_was_applied() {
    let mut deps = FakeDeps::fresh();
    // apply and style, then the input ends at the remote-answering prompt
    let (code, io) = run_en(&mut deps, &["", ""]);
    assert_eq!(code, 1);
    assert_eq!(deps.applied.len(), 2, "the Agents stay applied");
    assert!(!deps.called("background_decided"));
    assert!(
        io.noted
            .iter()
            .any(|note| note.contains("Setup stopped: input closed")),
        "{:?}",
        io.noted
    );
}

#[test]
fn the_summary_lists_what_the_machine_ended_up_with() {
    let mut deps = FakeDeps::fresh();
    deps.channel_mut("slack").enabled = true;
    deps.channel_mut("slack").configured = true;
    let (_, io) = run_en(&mut deps, &["", "2", "s", "", "y"]);
    let said = squeezed(&io.said());
    assert!(said.contains("Done."), "{said}");
    assert!(
        said.contains("Agents: Cursor (CLI), Claude Code (CLI)"),
        "{said}"
    );
    assert!(said.contains("Collaboration: autonomous"), "{said}");
    assert!(said.contains("Remote answering: Slack"), "{said}");
    assert!(
        said.contains("Background: menu-bar icon, starts at login; kept running in the background"),
        "{said}"
    );
    assert!(said.contains("Run `AskHuman setup` any time"), "{said}");
}

#[test]
fn a_summary_without_agents_or_channels_says_none() {
    let mut deps = FakeDeps::fresh().with_rows(rows(&[], &[]));
    let (_, io) = run_en(&mut deps, &["s", "", ""]);
    let said = squeezed(&io.said());
    assert!(said.contains("Agents: none"), "{said}");
    assert!(said.contains("Remote answering: none"), "{said}");
}

#[test]
fn the_flow_speaks_chinese_when_asked_to() {
    let mut deps = FakeDeps::fresh();
    let mut io = FakeIo::new(&["", "", "", ""]);
    assert_eq!(run(Lang::Zh, &mut io, &mut deps), 0);
    let said = io.said();
    assert!(said.contains("[1/3] 接入 Agent"), "{said}");
    assert!(said.contains("[2/3] 远程回答"), "{said}");
    assert!(said.contains("[3/3] 后台与启动"), "{said}");
    assert!(said.contains("✓ Cursor（CLI）已就绪"), "{said}");
    assert!(said.contains("完成。"), "{said}");
    assert!(io.prompted("应用以上设置？"));
    assert!(io.prompted("菜单栏常驻并开机自启？"));
}
