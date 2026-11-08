//! The terminal version of the setup wizard (`AskHuman setup --text`; spec D20, §3.3).
//!
//! The same three steps as the GUI wizard: connect Agents (with the collaboration style), set up
//! remote answering (optional), choose background behaviour. Every side effect goes through
//! [`TextDeps`] and every line of terminal I/O through [`TextIo`], so tests script the whole flow
//! with fakes; [`run_real`] wires the real ones.
//!
//! The flow never blocks on the optional verifications (those exist only in the GUI) and an abort
//! (end of input, Ctrl-C) keeps whatever was already applied.

use crate::agents::AgentKind;
use crate::cli::{cfgio, channel_cmd};
use crate::config::{AppConfig, CollaborationStyle, DaemonLifecycleMode, MenuBarIconMode};
use crate::i18n::Lang;
use crate::integrations::agent_mode::Mode;
use crate::integrations::{agent_launch, login_item};
use crate::onboarding::apply::{self, ApplyResult, Selection};
use crate::onboarding::channels::{self, ChannelStatus};
use crate::onboarding::detect::{self, AgentScanRow};
use crate::onboarding::env::{self, EnvReport};
use crate::onboarding::state::{self, DOMAIN_AGENTS, DOMAIN_REMOTE};
use std::io::Write;
use std::path::Path;

/// The terminal: normal output, hints, and one line of input.
pub trait TextIo {
    /// A line of the flow itself (stdout).
    fn say(&mut self, line: &str);
    /// A hint or warning (stderr).
    fn note(&mut self, line: &str);
    /// Show `prompt` and read one line. End of input is an error: a flow that cannot be answered
    /// must stop, never loop.
    fn ask(&mut self, prompt: &str) -> Result<String, String>;
}

/// Everything the flow does to the machine.
pub trait TextDeps {
    fn scan(&mut self) -> Vec<AgentScanRow>;
    fn env_report(&mut self) -> EnvReport;
    /// A Dev instance never changes the everyday Agent integrations.
    fn agents_read_only(&mut self) -> bool;
    fn load_config(&mut self) -> AppConfig;
    fn save_config(&mut self, config: &AppConfig) -> Result<(), String>;
    /// Re-render the rules / skill of every integrated Agent after a style change.
    fn apply_style_to_integrations(&mut self) -> Result<(), String>;
    fn apply_agents(&mut self, selections: &[Selection]) -> Vec<ApplyResult>;
    fn channel_rows(&mut self) -> Vec<ChannelStatus>;
    /// The existing interactive `channel` wizard, with "enable" as the default answer.
    fn configure_channel(&mut self, id: &str) -> Result<(), String>;
    fn user_id_missing(&mut self, id: &str) -> bool;
    fn detect_user_id(&mut self, id: &str) -> Result<(), String>;
    fn test_channel(&mut self, id: &str) -> Result<(), String>;
    fn tray_supported(&mut self) -> bool;
    fn login_item(&mut self, install: bool) -> Result<(), String>;
    /// Whether this platform can start Agent tasks from the phone or IM (a supported terminal).
    fn agent_tasks_supported(&mut self) -> bool;
    /// Install the daemon's own login item. Settings does this before it saves the Agent-task
    /// switch, and a failure there leaves the feature off.
    fn install_daemon_login_item(&mut self) -> Result<(), String>;
    /// Make sure the daemon runs with the saved settings (Settings does this after saving).
    fn ensure_daemon(&mut self) -> Result<(), String>;
    fn mark_skipped(&mut self, domain: &str);
    fn mark_background_decided(&mut self);
    fn agents_decided(&mut self) -> bool;
}

/// Run the flow. Returns the process exit code: `0` when it ran to the end, `1` when it stopped
/// early (end of input or an unreadable answer).
pub fn run(lang: Lang, io: &mut dyn TextIo, deps: &mut dyn TextDeps) -> i32 {
    let mut flow = Flow {
        lang,
        io,
        deps,
        env: EnvReport::empty(),
    };
    match flow.run_all() {
        Ok(()) => 0,
        Err(error) => {
            let line = flow.t(
                &format!("Setup stopped: {error}. Whatever was already applied is kept; run `AskHuman setup` again to continue."),
                &format!("设置已中断：{error}。已应用的改动会保留；重新运行 `AskHuman setup` 即可继续。"),
            );
            flow.io.note(&line);
            1
        }
    }
}

/// The real terminal and the real machine.
pub fn run_real(lang: Lang) -> i32 {
    run(lang, &mut StdIo, &mut RealDeps { lang })
}

// ——— the flow ———

struct Flow<'a> {
    lang: Lang,
    io: &'a mut dyn TextIo,
    deps: &'a mut dyn TextDeps,
    env: EnvReport,
}

/// One Agent row of step 1 together with what the user wants done to it.
struct Entry {
    row: AgentScanRow,
    /// The mode this Agent should end up in; `None` leaves it as it is.
    target: Option<Mode>,
}

impl Entry {
    fn current(&self) -> Mode {
        Mode::parse(&self.row.mode).unwrap_or(Mode::None)
    }

    fn integrated(&self) -> bool {
        self.current() != Mode::None
    }

    fn readable(&self) -> bool {
        self.row.status_error.is_none()
    }

    fn recommended(&self) -> Mode {
        Mode::parse(&self.row.recommended).unwrap_or(Mode::Cli)
    }

    /// The mode applying would write, or `None` when nothing needs writing.
    fn pending(&self) -> Option<Mode> {
        let target = self.target?;
        if !self.readable() {
            return None;
        }
        (target != self.current() || self.row.needs_update).then_some(target)
    }

    /// The mode this Agent will be in once the flow is done.
    fn final_mode(&self) -> Mode {
        self.target.unwrap_or_else(|| self.current())
    }

    fn supports(&self, mode: Mode) -> bool {
        match mode {
            Mode::Cli => self.row.supports_cli,
            Mode::Mcp => self.row.supports_mcp,
            Mode::None => false,
        }
    }
}

/// D19: integrated → keep its mode; detected but not integrated → the recommended mode; neither
/// → untouched.
fn initial_target(row: &AgentScanRow) -> Option<Mode> {
    if row.status_error.is_some() {
        return None;
    }
    let current = Mode::parse(&row.mode).unwrap_or(Mode::None);
    if current != Mode::None {
        Some(current)
    } else if row.detected {
        Some(Mode::parse(&row.recommended).unwrap_or(Mode::Cli))
    } else {
        None
    }
}

fn mode_label(mode: Mode) -> &'static str {
    match mode {
        Mode::Cli => "CLI",
        Mode::Mcp => "MCP",
        Mode::None => "-",
    }
}

/// What the user typed at the confirmation prompt.
#[derive(Debug, PartialEq, Eq)]
enum Choice {
    Confirm,
    Each,
    One(usize),
    Skip,
    Unknown,
}

fn parse_choice(answer: &str, count: usize, offer_each: bool) -> Choice {
    let answer = answer.trim().to_lowercase();
    match answer.as_str() {
        "" | "y" | "yes" => Choice::Confirm,
        "n" | "no" if offer_each => Choice::Each,
        "s" | "skip" => Choice::Skip,
        other => match other.parse::<usize>() {
            Ok(number) if (1..=count).contains(&number) => Choice::One(number - 1),
            _ => Choice::Unknown,
        },
    }
}

/// How many terminal cells `text` takes: East Asian wide characters (the Chinese channel names)
/// take two, everything else one.
fn cells(text: &str) -> usize {
    text.chars()
        .map(|c| {
            let wide = matches!(c as u32,
                0x1100..=0x115F
                | 0x2E80..=0xA4CF
                | 0xAC00..=0xD7A3
                | 0xF900..=0xFAFF
                | 0xFE30..=0xFE6F
                | 0xFF00..=0xFF60
                | 0xFFE0..=0xFFE6);
            if wide {
                2
            } else {
                1
            }
        })
        .sum()
}

/// `text` followed by spaces up to `width` terminal cells, so a column lines up in any language.
fn pad_cells(text: &str, width: usize) -> String {
    format!("{text}{}", " ".repeat(width.saturating_sub(cells(text))))
}

/// `~`-abbreviate a path under the home directory.
fn tilde(path: &Path, home: &Path) -> String {
    match path.strip_prefix(home) {
        Ok(rest) if rest.as_os_str().is_empty() => "~".to_string(),
        Ok(rest) => format!("~/{}", rest.display()),
        Err(_) => path.display().to_string(),
    }
}

impl Flow<'_> {
    fn t(&self, en: &str, zh: &str) -> String {
        cfgio::t(self.lang, en, zh)
    }

    fn say(&mut self, en: &str, zh: &str) {
        let line = self.t(en, zh);
        self.io.say(&line);
    }

    fn note(&mut self, en: &str, zh: &str) {
        let line = self.t(en, zh);
        self.io.note(&line);
    }

    fn blank(&mut self) {
        self.io.say("");
    }

    fn ask(&mut self, en: &str, zh: &str) -> Result<String, String> {
        let prompt = self.t(en, zh);
        Ok(self.io.ask(&prompt)?.trim().to_string())
    }

    /// A yes/no question; Enter takes the default and anything unclear asks again.
    fn yes_no(&mut self, en: &str, zh: &str, default_yes: bool) -> Result<bool, String> {
        let hint = if default_yes { "[Y/n]" } else { "[y/N]" };
        loop {
            let answer = self.ask(&format!("{en} {hint}: "), &format!("{zh} {hint}: "))?;
            match answer.to_lowercase().as_str() {
                "" => return Ok(default_yes),
                "y" | "yes" | "1" | "true" | "是" => return Ok(true),
                "n" | "no" | "0" | "false" | "否" => return Ok(false),
                _ => self.say("Please answer y or n.", "请回答 y 或 n。"),
            }
        }
    }

    fn run_all(&mut self) -> Result<(), String> {
        self.say(
            "AskHuman setup — 3 steps, about 2 minutes. Press Ctrl-C at any time; what is already applied stays.",
            "AskHuman 设置向导 —— 共 3 步，约 2 分钟。随时可按 Ctrl-C 退出，已应用的改动会保留。",
        );
        self.env = self.deps.env_report();
        self.environment_cards();
        self.step_agents()?;
        self.step_remote()?;
        self.step_background()?;
        self.summary();
        Ok(())
    }

    // ——— step 0: environment ———

    /// The cards that apply right away. Silent when everything is fine. The PATH card waits for
    /// the Agent choice (see `path_card`).
    fn environment_cards(&mut self) {
        if let Some(transient) = self.env.transient.clone() {
            let (en_place, zh_place) = match transient.kind.as_str() {
                "downloads" => ("your Downloads folder", "「下载」文件夹"),
                "npx" => ("npm's npx cache", "npm 的 npx 缓存"),
                _ => ("a temporary directory", "临时目录"),
            };
            self.blank();
            self.say(
                &format!(
                    "! This AskHuman runs from {en_place} ({}) and may disappear. Move it somewhere permanent first (see the README, \"Install\"), then run setup again.",
                    transient.path
                ),
                &format!(
                    "! 当前运行的 AskHuman 位于{zh_place}（{}），可能随时被清理。建议先把它放到固定位置（见 README「安装」），再重新运行本向导。",
                    transient.path
                ),
            );
        }
        if let Some(installs) = self.env.installs.clone() {
            self.blank();
            let line = format!("! {}", installs.message);
            self.io.say(&line);
        }
    }

    /// Print the PATH fix when `AskHuman` cannot be found from a shell and some Agent will use its
    /// CLI mode.
    fn path_card(&mut self, entries: &[Entry]) {
        if self.env.askhuman_on_path != "missing" {
            return;
        }
        let Some(fix) = self.env.path_fix.clone() else {
            return;
        };
        if !entries.iter().any(|entry| entry.final_mode() == Mode::Cli) {
            return;
        }
        self.blank();
        self.say(
            "! A terminal cannot find the `AskHuman` command, which Agents in CLI mode need.",
            "! 终端里找不到 `AskHuman` 命令，而 CLI 模式的 Agent 需要它。",
        );
        if cfg!(windows) {
            self.say(
                "  Run this in PowerShell, then open a new terminal:",
                "  在 PowerShell 里运行下面这条命令，然后新开一个终端：",
            );
        } else {
            self.say(
                "  Run this command to fix it:",
                "  运行下面这条命令即可修复：",
            );
        }
        let command = format!("    {}", fix.command);
        self.io.say(&command);
        if fix.needs_path_edit {
            self.say(
                "  Also add ~/.local/bin to PATH in your shell profile: export PATH=\"$HOME/.local/bin:$PATH\"",
                "  还需要在 shell 配置里把 ~/.local/bin 加入 PATH：export PATH=\"$HOME/.local/bin:$PATH\"",
            );
        }
    }

    // ——— step 1: Agents ———

    fn load_entries(&mut self) -> Vec<Entry> {
        let mut entries: Vec<Entry> = self
            .deps
            .scan()
            .into_iter()
            .map(|row| Entry {
                target: initial_target(&row),
                row,
            })
            .collect();
        // Detected or integrated first; the table order is kept within each group.
        entries.sort_by_key(|entry| !(entry.row.detected || entry.integrated()));
        entries
    }

    fn describe(&self, entry: &Entry) -> String {
        if let Some(error) = &entry.row.status_error {
            return self.t(
                &format!("cannot read its state: {error}"),
                &format!("无法读取状态：{error}"),
            );
        }
        let current = mode_label(entry.current());
        let mut text = match (entry.target, entry.integrated()) {
            (Some(target), true) if target == entry.current() => {
                if entry.row.needs_update {
                    self.t(
                        &format!("integrated ({current}), update available → will update"),
                        &format!("已接入（{current}），有更新 → 将更新"),
                    )
                } else {
                    self.t(
                        &format!("integrated ({current})"),
                        &format!("已接入（{current}）"),
                    )
                }
            }
            (Some(target), true) => self.t(
                &format!(
                    "integrated ({current}) → will switch to {}",
                    mode_label(target)
                ),
                &format!("已接入（{current}）→ 将切换为 {}", mode_label(target)),
            ),
            (Some(target), false) => {
                let recommended = target == entry.recommended();
                self.t(
                    &format!(
                        "→ will integrate in {} mode{}",
                        mode_label(target),
                        if recommended { " (recommended)" } else { "" }
                    ),
                    &format!(
                        "→ 将接入 {} 模式{}",
                        mode_label(target),
                        if recommended { "（推荐）" } else { "" }
                    ),
                )
            }
            (None, true) => self.t(
                &format!("integrated ({current}), left as is"),
                &format!("已接入（{current}），本次不改动"),
            ),
            (None, false) if entry.row.detected => {
                self.t("detected, not integrated", "已检测到，未接入")
            }
            (None, false) => self.t("not detected", "未检测到"),
        };
        if entry.integrated() && entry.row.delivery.as_deref() == Some("untrusted") {
            text.push_str(&self.t(" [needs trust]", " [待信任]"));
        }
        text
    }

    fn show_entries(&mut self, entries: &[Entry]) {
        self.blank();
        self.say(
            "Agents on this machine (detected ones first):",
            "本机上的 Agent（已检测到的在前）：",
        );
        for (index, entry) in entries.iter().enumerate() {
            let line = format!(
                "  {}) {} {}",
                index + 1,
                pad_cells(&entry.row.title, 12),
                self.describe(entry)
            );
            self.io.say(&line);
        }
        if entries
            .iter()
            .any(|entry| entry.integrated() && entry.row.delivery.as_deref() == Some("untrusted"))
        {
            self.say(
                "  Codex: trust the AskHuman hook when Codex asks the first time it starts.",
                "  Codex：首次启动时按提示信任 AskHuman 的 Hook。",
            );
        }
        if entries
            .iter()
            .any(|entry| entry.row.id == "cursor" && entry.target.is_some())
        {
            self.say(
                "  Cursor: the global rule only applies to projects under your home directory.",
                "  Cursor：全局规则只对位于用户主目录下的项目生效。",
            );
        }
    }

    /// "Files that will change", with created / modified marked.
    fn show_planned_files(&mut self, entries: &[Entry]) {
        let home = crate::paths::home();
        let mut lines = Vec::new();
        for entry in entries {
            let Some(mode) = entry.pending() else {
                continue;
            };
            let files = match mode {
                Mode::Cli => &entry.row.planned.cli,
                Mode::Mcp => &entry.row.planned.mcp,
                Mode::None => continue,
            };
            lines.push(format!("  {} ({})", entry.row.title, mode_label(mode)));
            for file in files {
                let mark = if file.exists {
                    self.t("modify", "修改")
                } else {
                    self.t("new", "新建")
                };
                lines.push(format!("    {mark:<6} {}", tilde(&file.path, &home)));
            }
        }
        if lines.is_empty() {
            return;
        }
        self.blank();
        self.say("Files that will change:", "将改动的文件：");
        for line in lines {
            self.io.say(&line);
        }
    }

    fn step_agents(&mut self) -> Result<(), String> {
        self.blank();
        self.say("[1/3] Connect your Agents", "[1/3] 接入 Agent");

        if self.deps.agents_read_only() {
            self.say(
                "This is a Dev instance: it never changes your everyday Agent integrations, so this step is read-only and skipped.",
                "这是 Dev 实例：它不会改动日常的 Agent 集成，此步只读，已跳过。",
            );
            return Ok(());
        }

        let mut entries = self.load_entries();
        loop {
            self.show_entries(&entries);
            let pending = entries
                .iter()
                .filter(|entry| entry.pending().is_some())
                .count();
            let answer = if pending > 0 {
                self.show_planned_files(&entries);
                self.blank();
                self.ask(
                    "Apply these changes? [Y apply / n adjust each / number adjust one / s skip this step]: ",
                    "应用以上设置？[Y 应用 / n 逐家调整 / 编号 只调整一家 / s 跳过此步]: ",
                )?
            } else {
                self.blank();
                if entries
                    .iter()
                    .any(|entry| entry.integrated() || entry.target.is_some())
                {
                    self.say(
                        "Your Agents are up to date; nothing to change.",
                        "已接入的 Agent 都是最新的，无需改动。",
                    );
                } else {
                    self.say(
                        "No Agent was detected on this machine. Type a number to integrate one by hand, or skip.",
                        "没有检测到已安装的 Agent。可输入编号手动接入某一家，或跳过此步。",
                    );
                }
                self.ask(
                    "[Enter continue / number adjust one / s skip this step]: ",
                    "[回车 继续 / 编号 只调整一家 / s 跳过此步]: ",
                )?
            };
            match parse_choice(&answer, entries.len(), pending > 0) {
                Choice::Confirm => break,
                Choice::Each => {
                    for entry in entries.iter_mut() {
                        self.adjust(entry)?;
                    }
                }
                Choice::One(index) => self.adjust(&mut entries[index])?,
                Choice::Skip => {
                    if !self.deps.agents_decided() {
                        self.deps.mark_skipped(DOMAIN_AGENTS);
                    }
                    return Ok(());
                }
                Choice::Unknown => self.say(
                    "That is not one of the choices; try again.",
                    "没有这个选项，请重新输入。",
                ),
            }
        }

        self.path_card(&entries);
        self.ask_style()?;
        self.apply_entries(&entries);
        Ok(())
    }

    /// Ask which mode one Agent should be in.
    fn adjust(&mut self, entry: &mut Entry) -> Result<(), String> {
        if !entry.readable() {
            let title = entry.row.title.clone();
            self.note(
                &format!("{title}: its state cannot be read, so it is left alone."),
                &format!("{title}：无法读取状态，保持不动。"),
            );
            return Ok(());
        }
        let mut options = Vec::new();
        if entry.supports(Mode::Cli) {
            options.push("c=CLI");
        }
        if entry.supports(Mode::Mcp) {
            options.push("m=MCP");
        }
        options.push("x");
        let default = match entry.target {
            Some(Mode::Cli) => 'c',
            Some(Mode::Mcp) => 'm',
            _ => 'x',
        };
        let hint = options.join(" / ");
        let title = entry.row.title.clone();
        loop {
            let answer = self.ask(
                &format!("{title}: mode [{hint}; x = leave as is] (Enter = {default}): "),
                &format!("{title}：接入模式 [{hint}；x = 不改动]（回车 = {default}）: "),
            )?;
            let letter = match answer.to_lowercase().as_str() {
                "" => default,
                "c" | "cli" => 'c',
                "m" | "mcp" => 'm',
                "x" => 'x',
                _ => '?',
            };
            match letter {
                'c' if entry.supports(Mode::Cli) => entry.target = Some(Mode::Cli),
                'm' if entry.supports(Mode::Mcp) => entry.target = Some(Mode::Mcp),
                'x' => entry.target = None,
                _ => {
                    self.say(
                        "That mode is not available here; try again.",
                        "没有这个模式，请重新输入。",
                    );
                    continue;
                }
            }
            return Ok(());
        }
    }

    /// The collaboration style, asked after the choice is final and before anything is written.
    /// A change is saved first and pushed to the integrations that already exist; the Agents
    /// applied afterwards are rendered with the new style.
    fn ask_style(&mut self) -> Result<(), String> {
        let current = self.deps.load_config().general.collaboration_style;
        self.blank();
        self.say(
            "Collaboration style (how often your Agents check in with you):",
            "协作风格（Agent 向你确认的频繁程度）：",
        );
        let mark = self.t(" (current)", "（当前）");
        let aligned_mark = if current == CollaborationStyle::Aligned {
            mark.clone()
        } else {
            String::new()
        };
        let autonomous_mark = if current == CollaborationStyle::Autonomous {
            mark.clone()
        } else {
            String::new()
        };
        self.say(
            &format!("  1) Aligned{aligned_mark} — confirm requirements and plans, then change things at scale"),
            &format!("  1) 对齐{aligned_mark} —— 多确认需求与方案，理解一致后再大改"),
        );
        self.say(
            &format!("  2) Autonomous{autonomous_mark} — fewer interruptions; ask only when blocked or at high risk"),
            &format!("  2) 自主{autonomous_mark} —— 少打断，只在阻塞或高风险时询问"),
        );
        if current == CollaborationStyle::Custom {
            self.say(
                &format!("  3) Custom{mark} — edit it in Settings"),
                &format!("  3) 自定义{mark} —— 在设置页编辑"),
            );
        }
        let default = match current {
            CollaborationStyle::Aligned => 1,
            CollaborationStyle::Autonomous => 2,
            CollaborationStyle::Custom => 3,
        };
        let chosen = loop {
            let answer = self.ask(
                &format!("Choose [{default}]: "),
                &format!("请选择 [{default}]: "),
            )?;
            let number = if answer.is_empty() {
                Some(default)
            } else {
                answer.parse::<u8>().ok()
            };
            match number {
                Some(1) => break CollaborationStyle::Aligned,
                Some(2) => break CollaborationStyle::Autonomous,
                Some(3) if current == CollaborationStyle::Custom => break current,
                _ => self.say(
                    "That is not one of the choices; try again.",
                    "没有这个选项，请重新输入。",
                ),
            }
        };
        if chosen == current {
            return Ok(());
        }
        let mut config = self.deps.load_config();
        config.general.collaboration_style = chosen;
        if let Err(error) = self.deps.save_config(&config) {
            self.say(
                &format!("  ✗ Could not save the style: {error}"),
                &format!("  ✗ 无法保存协作风格：{error}"),
            );
            return Ok(());
        }
        if let Err(error) = self.deps.apply_style_to_integrations() {
            self.say(
                &format!("  ✗ Saved, but updating the existing integrations failed: {error}"),
                &format!("  ✗ 已保存，但更新已有集成失败：{error}"),
            );
        }
        Ok(())
    }

    fn apply_entries(&mut self, entries: &[Entry]) {
        let selections: Vec<Selection> = entries
            .iter()
            .filter_map(|entry| {
                entry.pending().map(|mode| Selection {
                    agent: entry.row.id.clone(),
                    mode: mode.as_str().to_string(),
                })
            })
            .collect();
        if selections.is_empty() {
            return;
        }
        let results = self.deps.apply_agents(&selections);
        self.blank();
        let mut any_ok = false;
        let mut codex_untrusted = false;
        let mut cursor_ok = false;
        for result in &results {
            let title = entries
                .iter()
                .find(|entry| entry.row.id == result.agent)
                .map_or(result.agent.clone(), |entry| entry.row.title.clone());
            let mode = Mode::parse(&result.requested).map_or("-", mode_label);
            if result.ok {
                any_ok = true;
                if result.agent == "codex" && result.delivery.as_deref() == Some("untrusted") {
                    codex_untrusted = true;
                }
                if result.agent == "cursor" {
                    cursor_ok = true;
                }
                self.say(
                    &format!("  ✓ {title} ({mode}) is set up"),
                    &format!("  ✓ {title}（{mode}）已就绪"),
                );
            } else {
                let error = result.error.clone().unwrap_or_default();
                self.say(
                    &format!("  ✗ {title}: {error}"),
                    &format!("  ✗ {title}：{error}"),
                );
            }
        }
        if any_ok {
            self.say(
                "Note: Agent sessions that are already open must be restarted to load the new setup.",
                "提醒：已打开的 Agent 会话需要重启才会加载新配置。",
            );
        }
        if codex_untrusted {
            self.say(
                "Codex: trust the AskHuman hook when Codex asks the first time it starts.",
                "Codex：首次启动时按提示信任 AskHuman 的 Hook。",
            );
        }
        if cursor_ok {
            self.say(
                "Cursor: the global rule only applies to projects under your home directory.",
                "Cursor：全局规则只对位于用户主目录下的项目生效。",
            );
        }
    }

    // ——— step 2: remote answering ———

    fn channel_state(&self, row: &ChannelStatus) -> String {
        match (row.enabled, row.configured) {
            (true, true) => self.t("enabled", "已启用"),
            (true, false) => self.t("enabled, incomplete", "已启用，配置不完整"),
            (false, true) => self.t("configured, not enabled", "已配置，未启用"),
            (false, false) => self.t("not configured", "未配置"),
        }
    }

    fn step_remote(&mut self) -> Result<(), String> {
        self.blank();
        self.say(
            "[2/3] Remote answering (optional)",
            "[2/3] 远程回答（可选，可跳过）",
        );
        self.say(
            "Answer your Agents' questions from your phone or IM when you are away from the computer.",
            "离开电脑时，也能在手机 / IM 里回答 Agent 的提问。",
        );
        loop {
            let rows = self.deps.channel_rows();
            for (index, id) in channels::ORDER.iter().enumerate() {
                let state = rows
                    .iter()
                    .find(|row| row.id == *id)
                    .map(|row| self.channel_state(row))
                    .unwrap_or_default();
                let line = format!(
                    "  {}) {} {state}",
                    index + 1,
                    pad_cells(channels::title(id, self.lang), 9)
                );
                self.io.say(&line);
            }
            self.say("  s) skip", "  s) 跳过");
            let answer = self.ask("Choose [s]: ", "请选择 [s]: ")?;
            let lowered = answer.to_lowercase();
            if lowered.is_empty() || lowered == "s" || lowered == "skip" {
                break;
            }
            match lowered.parse::<usize>() {
                Ok(number) if (1..=channels::ORDER.len()).contains(&number) => {
                    self.configure_channel(channels::ORDER[number - 1])?;
                }
                _ => self.say(
                    "That is not one of the choices; try again.",
                    "没有这个选项，请重新输入。",
                ),
            }
        }
        if !self.deps.channel_rows().iter().any(|row| row.enabled) {
            self.deps.mark_skipped(DOMAIN_REMOTE);
        }
        Ok(())
    }

    /// Run the existing channel wizard, then offer to detect the user id and to test.
    fn configure_channel(&mut self, id: &str) -> Result<(), String> {
        if let Err(error) = self.deps.configure_channel(id) {
            self.say(&format!("  ✗ {error}"), &format!("  ✗ {error}"));
            return Ok(());
        }
        if channels::can_detect_user_id(id)
            && self.deps.user_id_missing(id)
            && self.yes_no(
                "Detect your user id automatically?",
                "自动识别你的用户 ID？",
                true,
            )?
        {
            if let Err(error) = self.deps.detect_user_id(id) {
                self.say(&format!("  ✗ {error}"), &format!("  ✗ {error}"));
            }
        }
        let configured = self
            .deps
            .channel_rows()
            .iter()
            .any(|row| row.id == id && row.configured);
        if !configured {
            self.say(
                &format!("  Not fully configured yet; finish it with `AskHuman channel set {id}`."),
                &format!("  还没有配置完整；可用 `AskHuman channel set {id}` 补全。"),
            );
            return Ok(());
        }
        if self.yes_no("Test the connection?", "测试连接？", false)? {
            match self.deps.test_channel(id) {
                Ok(()) => self.say("  ✓ The test went through.", "  ✓ 测试通过。"),
                Err(error) => self.say(
                    &format!("  ✗ The test failed: {error}"),
                    &format!("  ✗ 测试失败：{error}"),
                ),
            }
        }
        Ok(())
    }

    // ——— step 3: background and startup ———

    /// The two lines under the Agent-task question: the remote channels and the Agents it covers,
    /// those already set up first and the rest marked as usable once they are. Nothing is checked
    /// here: allowing is a permission, and whatever is not set up yet works as soon as it is.
    fn tasks_overview(&mut self, config: &AppConfig) {
        // The phone app first (it is the one with a "New task" button), then the IM channels.
        let mut ready_channels = Vec::new();
        let mut later_channels = Vec::new();
        let ios = self.t("iOS app", "iOS App");
        if config.channels.ios.enabled {
            ready_channels.push(ios);
        } else {
            later_channels.push(ios);
        }
        let rows = self.deps.channel_rows();
        for id in channels::ORDER {
            let title = channels::title(id, self.lang).to_string();
            if rows.iter().any(|row| row.id == id && row.enabled) {
                ready_channels.push(title);
            } else {
                later_channels.push(title);
            }
        }
        let mut ready_agents = Vec::new();
        let mut later_agents = Vec::new();
        for row in self.deps.scan() {
            // An Agent with no way to start a task (Antigravity) is not part of the picture.
            if !AgentKind::parse(&row.id).is_some_and(agent_launch::launch_supported) {
                continue;
            }
            if Mode::parse(&row.mode).is_some_and(|mode| mode != Mode::None) {
                ready_agents.push(row.title);
            } else {
                later_agents.push(row.title);
            }
        }
        let channels_line = self.overview_side(
            &ready_channels,
            &later_channels,
            "usable once set up",
            "配置后可用",
        );
        let agents_line = self.overview_side(
            &ready_agents,
            &later_agents,
            "usable once connected",
            "接入后可用",
        );
        let channels_label = self.t("Channels:", "渠道：");
        let agents_label = self.t("Agents:", "Agent：");
        let width = cells(&channels_label).max(cells(&agents_label)) + 1;
        self.io.say(&format!(
            "  {}{channels_line}",
            pad_cells(&channels_label, width)
        ));
        self.io.say(&format!(
            "  {}{agents_line}",
            pad_cells(&agents_label, width)
        ));
    }

    /// One side of the overview: "Feishu, Slack; usable once set up: Telegram". The ones in place
    /// come first; with none, it says so.
    fn overview_side(
        &self,
        ready: &[String],
        later: &[String],
        later_en: &str,
        later_zh: &str,
    ) -> String {
        let (comma, semicolon, colon, nothing, later_label) = match self.lang {
            Lang::En => (", ", "; ", ": ", "none yet", later_en),
            Lang::Zh => ("、", "；", "：", "暂无", later_zh),
        };
        let ready_text = if ready.is_empty() {
            nothing.to_string()
        } else {
            ready.join(comma)
        };
        if later.is_empty() {
            ready_text
        } else {
            format!(
                "{ready_text}{semicolon}{later_label}{colon}{}",
                later.join(comma)
            )
        }
    }

    fn step_background(&mut self) -> Result<(), String> {
        self.blank();
        self.say("[3/3] Background and startup", "[3/3] 后台与启动");
        let mut config = self.deps.load_config();
        // A remote channel of any kind: an IM channel, or the iOS app (set up in Settings, not in
        // this flow, but it reaches the Mac all the same).
        let any_remote =
            self.deps.channel_rows().iter().any(|row| row.enabled) || config.channels.ios.enabled;
        let mut changed = false;

        if self.deps.tray_supported() {
            let current = config.general.menu_bar_icon;
            let keep_icon = self.yes_no(
                "Keep the menu-bar icon and start AskHuman at login?",
                "菜单栏常驻并开机自启？",
                current == MenuBarIconMode::Always,
            )?;
            if keep_icon {
                if current != MenuBarIconMode::Always {
                    config.general.menu_bar_icon = MenuBarIconMode::Always;
                    changed = true;
                }
                if let Err(error) = self.deps.login_item(true) {
                    self.say(
                        &format!("  ✗ Could not set the login item: {error}"),
                        &format!("  ✗ 无法设置开机自启：{error}"),
                    );
                }
            } else if current == MenuBarIconMode::Always {
                config.general.menu_bar_icon = MenuBarIconMode::Active;
                changed = true;
                if let Err(error) = self.deps.login_item(false) {
                    self.say(
                        &format!("  ✗ Could not remove the login item: {error}"),
                        &format!("  ✗ 无法取消开机自启：{error}"),
                    );
                }
            }
        }

        // Allowing Agent tasks to be created from the phone or IM comes before "keep running":
        // allowing it pins the daemon resident, so there is nothing left to ask about that.
        let mut tasks_started = false;
        if self.deps.agent_tasks_supported() {
            let current = config.agent_tasks.enabled;
            // A permission, not a check: the question says what it covers and asks. A remote
            // channel is not a condition either: it is set up on its own and may come later, and
            // this is the one moment the person is asked.
            self.blank();
            self.say(
                "Allow Agent tasks to be created from your phone or IM?",
                "要允许从手机 / IM 远程创建 Agent 任务吗？",
            );
            self.say(
                "If you allow it, the channels you chose can create tasks for these Agents remotely.",
                "允许后，你选的这些渠道就能远程创建这些 Agent 的任务。",
            );
            self.tasks_overview(&config);
            let wanted = self.yes_no("Allow it?", "允许？", current)?;
            if wanted && !current {
                // Settings installs the daemon's login item before it saves the switch; so does
                // this flow, and a failure leaves the feature off.
                match self.deps.install_daemon_login_item() {
                    Ok(()) => {
                        config.agent_tasks.enabled = true;
                        changed = true;
                        tasks_started = true;
                        if !any_remote {
                            self.say(
                                "  No remote channel is set up yet: add an IM channel or the iOS app in Settings, and it works from there.",
                                "  还没有配置远程渠道：之后到「设置」里添加 IM 渠道或 iOS App，就能用了。",
                            );
                        }
                    }
                    Err(error) => self.say(
                        &format!("  ✗ Could not set up the background login item: {error}. Agent tasks stay off."),
                        &format!("  ✗ 无法设置后台登录项：{error}。Agent 任务保持关闭。"),
                    ),
                }
            } else if !wanted && current {
                config.agent_tasks.enabled = false;
                changed = true;
            }
        }

        // The Agent-task switch needs a resident daemon whether or not a channel exists yet (the
        // channel may be added later), so it pins that before any question about keeping it running.
        if config.agent_tasks.enabled || any_remote {
            if config.agent_tasks.enabled {
                // The feature depends on a resident daemon (Settings pins it the same way).
                if config.general.daemon_lifecycle != DaemonLifecycleMode::KeepAlive {
                    config.general.daemon_lifecycle = DaemonLifecycleMode::KeepAlive;
                    changed = true;
                }
                self.say(
                    "  Keeping AskHuman running in the background: on (Agent tasks need it).",
                    "  保持后台常驻：已开启（Agent 任务需要它）。",
                );
            } else {
                let current = config.general.daemon_lifecycle == DaemonLifecycleMode::KeepAlive;
                let keep_alive = self.yes_no(
                    "Keep AskHuman running in the background (so your phone or IM can reach this computer and start commands any time; uses a little memory)?",
                    "保持后台常驻（让手机 App / IM 随时能连上这台电脑并发起命令，会常驻少量资源）？",
                    current,
                )?;
                if keep_alive != current {
                    config.general.daemon_lifecycle = if keep_alive {
                        DaemonLifecycleMode::KeepAlive
                    } else {
                        DaemonLifecycleMode::Activity
                    };
                    changed = true;
                }
            }
        }

        if changed {
            if let Err(error) = self.deps.save_config(&config) {
                self.say(
                    &format!("  ✗ Could not save the settings: {error}"),
                    &format!("  ✗ 无法保存设置：{error}"),
                );
                tasks_started = false;
            }
        }
        if tasks_started {
            // Settings makes sure the daemon is up with the new settings once it has saved them.
            if let Err(error) = self.deps.ensure_daemon() {
                self.say(
                    &format!("  ✗ Could not start AskHuman in the background: {error}"),
                    &format!("  ✗ 无法在后台启动 AskHuman：{error}"),
                );
            }
        }
        self.deps.mark_background_decided();
        // Getting here means the user has been through the wizard: an Agent step that was neither
        // applied nor skipped counts as skipped.
        if !self.deps.agents_decided() {
            self.deps.mark_skipped(DOMAIN_AGENTS);
        }
        Ok(())
    }

    // ——— summary ———

    fn summary(&mut self) {
        let config = self.deps.load_config();
        let integrated: Vec<String> = self
            .deps
            .scan()
            .iter()
            .filter_map(|row| {
                let mode = Mode::parse(&row.mode).filter(|mode| *mode != Mode::None)?;
                Some(format!("{} ({})", row.title, mode_label(mode)))
            })
            .collect();
        let mut enabled: Vec<String> = self
            .deps
            .channel_rows()
            .iter()
            .filter(|row| row.enabled)
            .filter_map(|row| channels::ORDER.iter().find(|id| **id == row.id))
            .map(|id| channels::title(id, self.lang).to_string())
            .collect();
        if config.channels.ios.enabled {
            enabled.push(self.t("iOS app", "iOS App"));
        }
        let style = match config.general.collaboration_style {
            CollaborationStyle::Aligned => self.t("aligned", "对齐"),
            CollaborationStyle::Autonomous => self.t("autonomous", "自主"),
            CollaborationStyle::Custom => self.t("custom", "自定义"),
        };
        let mut background = Vec::new();
        match config.general.menu_bar_icon {
            MenuBarIconMode::Always => {
                background.push(self.t("menu-bar icon, starts at login", "菜单栏常驻并开机自启"))
            }
            MenuBarIconMode::Active => {
                background.push(self.t("menu-bar icon while active", "活动时显示菜单栏图标"))
            }
            MenuBarIconMode::Off => background.push(self.t("no menu-bar icon", "不显示菜单栏图标")),
        }
        if config.general.daemon_lifecycle == DaemonLifecycleMode::KeepAlive {
            background.push(self.t("kept running in the background", "后台常驻"));
        }
        if config.agent_tasks.enabled {
            background.push(self.t(
                "Agent tasks allowed from phone or IM",
                "已允许从手机 / IM 创建 Agent 任务",
            ));
        }
        let none = self.t("none", "无");
        self.blank();
        self.say("Done.", "完成。");
        let agents_line = if integrated.is_empty() {
            none.clone()
        } else {
            integrated.join(", ")
        };
        self.say(
            &format!("  Agents:           {agents_line}"),
            &format!("  Agent：           {agents_line}"),
        );
        self.say(
            &format!("  Collaboration:    {style}"),
            &format!("  协作风格：        {style}"),
        );
        let channels_line = if enabled.is_empty() {
            none
        } else {
            enabled.join(", ")
        };
        self.say(
            &format!("  Remote answering: {channels_line}"),
            &format!("  远程回答：        {channels_line}"),
        );
        let background_line = background.join("; ");
        self.say(
            &format!("  Background:       {background_line}"),
            &format!("  后台：            {background_line}"),
        );
        self.blank();
        self.say(
            "Next time, just use your Agent as usual. Run `AskHuman setup` any time to change this.",
            "下次直接在 Agent 里使用即可；随时运行 `AskHuman setup` 重新调整。",
        );
        // Left off: say where to allow it later. The switch sits on each remote channel's card, so it
        // is found where a channel is added.
        if self.deps.agent_tasks_supported() && !config.agent_tasks.enabled {
            self.say(
                "To allow creating Agent tasks from your phone or IM later, turn on \"Allow creating Agent tasks from phone or IM\" in Settings, on a remote channel's card or under Advanced.",
                "想以后允许从手机 / IM 创建 Agent 任务：到「设置」里打开「允许从手机 / IM 创建 Agent 任务」，在任一远程渠道的卡片里，或「高级」里都有。",
            );
        }
    }
}

// ——— the real terminal and machine ———

/// stdin / stdout / stderr.
pub struct StdIo;

impl TextIo for StdIo {
    fn say(&mut self, line: &str) {
        let mut out = std::io::stdout();
        let _ = writeln!(out, "{line}");
        let _ = out.flush();
    }

    fn note(&mut self, line: &str) {
        let mut err = std::io::stderr();
        let _ = writeln!(err, "{line}");
        let _ = err.flush();
    }

    fn ask(&mut self, prompt: &str) -> Result<String, String> {
        let mut err = std::io::stderr();
        let _ = write!(err, "{prompt}");
        let _ = err.flush();
        let mut line = String::new();
        match std::io::stdin().read_line(&mut line) {
            Ok(0) => Err("input closed".to_string()),
            Ok(_) => Ok(line.trim_end_matches(['\n', '\r']).to_string()),
            Err(error) => Err(error.to_string()),
        }
    }
}

/// The machine: Agent files, `config.json`, the keychain-backed channel wizard, login items and
/// `ui-state.json`.
pub struct RealDeps {
    pub lang: Lang,
}

impl TextDeps for RealDeps {
    fn scan(&mut self) -> Vec<AgentScanRow> {
        detect::scan()
    }

    fn env_report(&mut self) -> EnvReport {
        env::check(self.lang)
    }

    fn agents_read_only(&mut self) -> bool {
        crate::integrations::mutation_lock::everyday_changes_blocked().unwrap_or(true)
    }

    fn load_config(&mut self) -> AppConfig {
        AppConfig::load_without_secrets()
    }

    fn save_config(&mut self, config: &AppConfig) -> Result<(), String> {
        config.save().map_err(|error| error.to_string())
    }

    fn apply_style_to_integrations(&mut self) -> Result<(), String> {
        crate::commands::collaboration_style_apply_integrations()
    }

    fn apply_agents(&mut self, selections: &[Selection]) -> Vec<ApplyResult> {
        apply::apply(selections)
    }

    fn channel_rows(&mut self) -> Vec<ChannelStatus> {
        channels::status(&AppConfig::load_without_secrets(), None)
    }

    fn configure_channel(&mut self, id: &str) -> Result<(), String> {
        channel_cmd::wizard_for_setup(id, self.lang)
    }

    fn user_id_missing(&mut self, id: &str) -> bool {
        channels::user_id_missing(&AppConfig::load_without_secrets(), id)
    }

    fn detect_user_id(&mut self, id: &str) -> Result<(), String> {
        channel_cmd::detect(&[id.to_string()], self.lang)
    }

    fn test_channel(&mut self, id: &str) -> Result<(), String> {
        channel_cmd::test(&[id.to_string()], self.lang)
    }

    fn tray_supported(&mut self) -> bool {
        crate::app::gui_host::tray_supported()
    }

    fn login_item(&mut self, install: bool) -> Result<(), String> {
        let result = if install {
            login_item::ensure_installed()
        } else {
            login_item::uninstall()
        };
        result.map_err(|error| error.to_string())
    }

    fn agent_tasks_supported(&mut self) -> bool {
        // The same platforms as the settings page (a terminal the daemon can drive).
        cfg!(any(target_os = "macos", target_os = "windows"))
    }

    fn install_daemon_login_item(&mut self) -> Result<(), String> {
        // A Dev instance has no daemon login item of its own (Settings skips it too).
        if crate::dev_instance::is_dev_instance_context() {
            return Ok(());
        }
        login_item::sync_daemon(true).map_err(|error| error.to_string())
    }

    fn ensure_daemon(&mut self) -> Result<(), String> {
        cfgio::block_on(crate::client::ensure_running()).map_err(|error| error.to_string())
    }

    fn mark_skipped(&mut self, domain: &str) {
        state::mark_skipped(domain);
    }

    fn mark_background_decided(&mut self) {
        state::mark_background_decided();
    }

    fn agents_decided(&mut self) -> bool {
        state::agents_decided()
    }
}

#[cfg(test)]
mod tests;
