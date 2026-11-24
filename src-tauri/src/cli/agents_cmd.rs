//! `AskHuman agents <monitor|show|install|uninstall|update|help>` —— Agent 实时状态 + 集成。
//! 解决与原 `agents status`（GUI 窗口）命名冲突：状态窗口改名 `monitor`（增文本 / `--json`），
//! 集成动词 install/uninstall/update/show 复用 `integrations::{agent_rules,cursor_hook,claude_hook,agent_lifecycle}`。

use super::cfgio;
use crate::agents::AgentKind;
use crate::i18n::{err_prefix, Lang};
use crate::integrations::agent_rules::AgentTarget;
use crate::integrations::codex_protocol::{self, Delivery};
use crate::integrations::{
    agent_delegated, agent_lifecycle, agent_mode, agent_permission, agent_profiles, agent_rules,
    agent_stop, antigravity_plugin, claude_hook, cursor_hook, mcp_config,
};
use serde_json::Value;
use std::process::exit;

const AGENTS: [&str; 6] = ["cursor", "claude", "codex", "grok", "pi", "antigravity"];

// Dedicated queries remain available and return an explicit unsupported-platform error.
pub(super) fn aggregate_agents() -> Vec<&'static str> {
    AGENTS
        .iter()
        .copied()
        .filter(|name| *name != "antigravity" || cfg!(unix))
        .collect()
}

pub fn dispatch(args: &[String], lang: Lang) {
    // 无子命令 → 打印 help（与 channel/config 一致；不再默认开状态窗口）。
    let sub = args.first().map(|s| s.as_str()).unwrap_or("help");
    let rest = &args[args.len().min(1)..];
    let r = match sub {
        "monitor" => monitor(rest, lang),
        "mode" => mode_cmd(rest, lang),
        "update" => update_cmd(rest, lang),
        "permission" => permission_cmd(rest, lang),
        "stop" => stop_cmd(rest, lang),
        "delegated" => delegated_cmd(rest, lang),
        "lifecycle" => lifecycle_cmd(rest, lang),
        "profile" | "profiles" => profile_cmd(rest, lang),
        "cleanup" => cleanup_cmd(rest, lang),
        "sidecars" => super::antigravity_sidecars::run(rest, lang),
        "install" | "uninstall" => Err(legacy_write_error(sub, lang)),
        "show" => show(rest, lang),
        "help" | "-h" | "--help" => {
            print_line(&help(lang));
            Ok(())
        }
        other => Err(cfgio::t(
            lang,
            &format!("unknown subcommand: {other}\n\n{}", help(lang)),
            &format!("未知子命令: {other}\n\n{}", help(lang)),
        )),
    };
    if let Err(e) = r {
        eprintln!("{}{}", err_prefix(lang), e);
        exit(1);
    }
}

// ——— monitor（状态）———

fn monitor(args: &[String], lang: Lang) -> Result<(), String> {
    let json = args.iter().any(|a| a == "--json");
    let text = args.iter().any(|a| a == "--text");

    if !json && !text && gui_available() {
        if crate::gui_host::host_open(crate::gui_host::WindowKind::Agents, false, None, None, None)
            .is_ok()
        {
            exit(0);
        }
        crate::app::run_agents(crate::config::AppConfig::load_without_secrets());
    }
    match cfgio::block_on(crate::client::request_agents_snapshot()) {
        Some(v) if json => {
            print_line(&serde_json::to_string_pretty(&v).unwrap_or_default());
            Ok(())
        }
        Some(v) => {
            print_line(&render_text(&v, lang));
            Ok(())
        }
        None => Err(cfgio::t(lang, "daemon not running", "daemon 未运行")),
    }
}

/// Whether a graphical session can show a window (also asked by `setup`).
#[cfg(any(target_os = "macos", target_os = "windows"))]
pub(crate) fn gui_available() -> bool {
    true
}
#[cfg(all(unix, not(target_os = "macos")))]
pub(crate) fn gui_available() -> bool {
    std::env::var_os("DISPLAY").is_some() || std::env::var_os("WAYLAND_DISPLAY").is_some()
}
#[cfg(not(any(unix, windows)))]
pub(crate) fn gui_available() -> bool {
    false
}

/// 把快照（AgentRecord 数组）渲染为分组文本：工作中 / 空闲 / 已结束。
fn render_text(snapshot: &Value, lang: Lang) -> String {
    let empty = vec![];
    let list = snapshot.as_array().unwrap_or(&empty);
    if list.is_empty() {
        return cfgio::t(lang, "No agents tracked.", "暂无被追踪的 agent。");
    }
    let now = now_secs();
    let mut out = String::new();
    for (state, title) in [
        ("working", cfgio::t(lang, "Working", "工作中")),
        ("idle", cfgio::t(lang, "Idle", "空闲")),
        ("ended", cfgio::t(lang, "Ended", "已结束")),
    ] {
        let group: Vec<&Value> = list
            .iter()
            .filter(|r| r.get("state").and_then(|s| s.as_str()) == Some(state))
            .collect();
        if group.is_empty() {
            continue;
        }
        out.push_str(&format!("{title} ({})\n", group.len()));
        for r in group {
            out.push_str(&format!("  {}\n", render_record(r, now, lang)));
        }
    }
    out.trim_end().to_string()
}

fn render_record(r: &Value, now: u64, lang: Lang) -> String {
    let kind = r.get("kind").and_then(|k| k.as_str()).unwrap_or("?");
    let kind_label = AgentKind::parse(kind).map(|k| k.label()).unwrap_or(kind);
    let title = r
        .get("title")
        .and_then(|t| t.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .unwrap_or_else(|| cfgio::t(lang, "(untitled)", "（无标题）"));
    let sid = r.get("sessionId").and_then(|s| s.as_str()).unwrap_or("");
    let short = sid.chars().take(8).collect::<String>();
    let last = r.get("lastActivity").and_then(|v| v.as_u64()).unwrap_or(0);
    format!(
        "{kind_label} — {title}  ({}{}, {})",
        cfgio::t(lang, "session ", "会话 "),
        short,
        rel_time(now, last, lang)
    )
}

fn rel_time(now: u64, ts: u64, lang: Lang) -> String {
    if ts == 0 {
        return cfgio::t(lang, "unknown", "未知");
    }
    let d = now.saturating_sub(ts);
    if d < 60 {
        cfgio::t(lang, &format!("{d}s ago"), &format!("{d} 秒前"))
    } else if d < 3600 {
        let m = d / 60;
        cfgio::t(lang, &format!("{m}m ago"), &format!("{m} 分钟前"))
    } else if d < 86400 {
        let h = d / 3600;
        cfgio::t(lang, &format!("{h}h ago"), &format!("{h} 小时前"))
    } else {
        let days = d / 86400;
        cfgio::t(lang, &format!("{days}d ago"), &format!("{days} 天前"))
    }
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

// ——— mode（三态编排：none | cli | mcp，与设置页同源）———

fn mode_cmd(args: &[String], lang: Lang) -> Result<(), String> {
    let agent = args.first().ok_or_else(|| {
        cfgio::t(
            lang,
            "usage: agents mode <agent> [none|cli|mcp]  (omit mode to query)",
            "用法: agents mode <agent> [none|cli|mcp]（省略模式则查询）",
        )
    })?;
    let target = AgentTarget::parse(agent).ok_or_else(|| {
        cfgio::t(
            lang,
            &format!("unknown agent: {agent} (expected cursor|claude|codex|grok|pi|antigravity)"),
            &format!("未知 agent: {agent}（应为 cursor|claude|codex|grok|pi|antigravity）"),
        )
    })?;
    let kind = AgentKind::parse(agent).unwrap();

    match args.get(1) {
        // 仅查询：当前模式 + 是否需更新。
        None => {
            let (mode, needs_update) = if target == AgentTarget::Antigravity {
                let (mode, updates, _) =
                    agent_mode::antigravity_status().map_err(|e| e.to_string())?;
                (mode, updates.rule || updates.hook || updates.mcp)
            } else {
                (
                    agent_mode::current(target),
                    agent_mode::needs_update(target),
                )
            };
            let upd = if needs_update {
                cfgio::t(lang, " (needs update)", "（需更新）")
            } else {
                String::new()
            };
            print_line(&format!(
                "[{}] {}{}",
                kind.label(),
                mode_label(mode, lang),
                upd
            ));
            print_antigravity_notes(target, mode, lang);
            Ok(())
        }
        // 切换：先卸非目标产物，再装目标产物（底层幂等）。
        Some(want) => {
            let mode = agent_mode::Mode::parse(want).ok_or_else(|| {
                cfgio::t(
                    lang,
                    &format!("unknown mode: {want} (expected none|cli|mcp)"),
                    &format!("未知模式: {want}（应为 none|cli|mcp）"),
                )
            })?;
            let env = parse_mode_env(target, mode, &args[2..])?;
            agent_mode::set_with_env(target, mode, &env).map_err(|e| e.to_string())?;
            print_line(&format!(
                "[{}] {} {}",
                kind.label(),
                cfgio::t(lang, "mode set to", "模式已设为"),
                mode_label(mode, lang)
            ));
            print_antigravity_notes(target, mode, lang);
            Ok(())
        }
    }
}

/// What Antigravity's own settings say about the plugin, for the lines a person reads after a
/// mode query or change. Silent for every other agent.
fn print_antigravity_notes(target: AgentTarget, mode: agent_mode::Mode, lang: Lang) {
    if target != AgentTarget::Antigravity || mode == agent_mode::Mode::None {
        return;
    }
    let host = antigravity_plugin::host_view();
    if host.plugin_disabled {
        print_line(&cfgio::t(
            lang,
            "  ! The plugin is switched off in Antigravity, so its hooks and MCP server do not run. Switch it on there.",
            "  ! 插件在 Antigravity 中已停用，它的 hooks 与 MCP server 不会运行。请在 Antigravity 里重新启用。",
        ));
    } else if mode == agent_mode::Mode::Mcp && !host.mcp_allowed {
        print_line(&cfgio::t(
            lang,
            "  Note: the first call in the Antigravity app asks once for permission. Choose \"Always allow\" (or add mcp(askhuman_mcp/*) in its permission settings).",
            "  提示：Antigravity App 里第一次调用会询问一次权限，请选「始终允许」（或在其权限设置里添加 mcp(askhuman_mcp/*)）。",
        ));
    }
}

fn parse_mode_env(
    target: AgentTarget,
    mode: agent_mode::Mode,
    args: &[String],
) -> Result<std::collections::BTreeMap<String, String>, String> {
    let mut env = std::collections::BTreeMap::new();
    if !args.is_empty() && (target != AgentTarget::Codex || mode != agent_mode::Mode::Mcp) {
        return Err("--env is supported only by agents mode codex mcp".into());
    }
    let mut args = args.iter();
    while let Some(flag) = args.next() {
        if flag != "--env" {
            return Err(format!("unknown option: {flag}"));
        }
        let pair = args.next().ok_or("--env requires NAME=VALUE")?;
        let (name, value) = pair.split_once('=').ok_or("--env requires NAME=VALUE")?;
        if !crate::integrations::mcp_config::valid_env_name(name) || value.contains('\0') {
            return Err("invalid environment name or NUL value".into());
        }
        env.insert(name.to_string(), value.to_string());
    }
    Ok(env)
}

fn mode_label(m: agent_mode::Mode, lang: Lang) -> String {
    match m {
        agent_mode::Mode::None => cfgio::t(lang, "off", "未集成"),
        agent_mode::Mode::Cli => "CLI".to_string(),
        agent_mode::Mode::Mcp => "MCP".to_string(),
    }
}

// ——— 整包更新与独立 capability ———

fn parse_target(agent: &str, lang: Lang) -> Result<AgentTarget, String> {
    AgentTarget::parse(agent).ok_or_else(|| {
        cfgio::t(
            lang,
            &format!("unknown agent: {agent} (expected cursor|claude|codex|grok|pi|antigravity)"),
            &format!("未知 agent: {agent}（应为 cursor|claude|codex|grok|pi|antigravity）"),
        )
    })
}

#[derive(Debug, PartialEq, Eq)]
struct AggregateUpdatePlan {
    targets: Vec<(String, AgentTarget)>,
    failures: Vec<(String, String)>,
}

fn plan_aggregate_updates(
    names: impl IntoIterator<Item = &'static str>,
    mut included: impl FnMut(AgentTarget) -> Result<bool, String>,
) -> AggregateUpdatePlan {
    let mut plan = AggregateUpdatePlan {
        targets: Vec::new(),
        failures: Vec::new(),
    };
    for name in names {
        let target = AgentTarget::parse(name).unwrap();
        match included(target) {
            Ok(true) => plan.targets.push((name.to_string(), target)),
            Ok(false) => {}
            Err(error) => plan.failures.push((name.to_string(), error)),
        }
    }
    plan
}

fn update_cmd(args: &[String], lang: Lang) -> Result<(), String> {
    if args.len() > 1 || args.first().is_some_and(|arg| arg.starts_with('-')) {
        return Err(cfgio::t(
            lang,
            "usage: agents update [<agent>] (artifact flags are no longer supported)",
            "用法: agents update [<agent>]（不再支持产物 flags）",
        ));
    }
    let mut failures = Vec::new();
    let targets = if let Some(agent) = args.first() {
        vec![(agent.clone(), parse_target(agent, lang)?)]
    } else {
        let plan = plan_aggregate_updates(aggregate_agents(), |target| {
            let (mode, any_primary_update) = if target == AgentTarget::Antigravity {
                let (mode, updates, _) =
                    agent_mode::antigravity_status().map_err(|error| error.to_string())?;
                (mode, updates.rule || updates.hook || updates.mcp)
            } else {
                (
                    agent_mode::current(target),
                    agent_mode::needs_update(target),
                )
            };
            let any_profile_update = agent_profiles::statuses_for_agent(target, mode)
                .into_iter()
                .any(|p| p.enabled && p.needs_update);
            Ok(mode != agent_mode::Mode::None || any_primary_update || any_profile_update)
        });
        for (name, error) in plan.failures {
            print_line(&format!(
                "[{name}] {}{error}",
                cfgio::t(lang, "error: ", "错误: ")
            ));
            failures.push(name);
        }
        plan.targets
    };
    for (name, target) in targets {
        match agent_mode::update(target) {
            Ok(()) => print_line(&format!("[{name}] {}", cfgio::t(lang, "updated", "已更新"))),
            Err(error) => {
                print_line(&format!(
                    "[{name}] {}{error}",
                    cfgio::t(lang, "error: ", "错误: ")
                ));
                failures.push(name);
            }
        }
    }
    if failures.is_empty() {
        Ok(())
    } else {
        Err(cfgio::t(
            lang,
            &format!("failed to update: {}", failures.join(", ")),
            &format!("更新失败: {}", failures.join("、")),
        ))
    }
}

fn permission_cmd(args: &[String], lang: Lang) -> Result<(), String> {
    let agent = args.first().ok_or_else(|| {
        cfgio::t(
            lang,
            "usage: agents permission <claude|codex> [on|off]",
            "用法: agents permission <claude|codex> [on|off]",
        )
    })?;
    if args.len() > 2 || !matches!(agent.as_str(), "claude" | "codex") {
        return Err(cfgio::t(
            lang,
            "permission approval is supported only for claude and codex",
            "权限审批仅支持 claude 与 codex",
        ));
    }
    let target = parse_target(agent, lang)?;
    if let Some(value) = args.get(1) {
        let enabled = match value.as_str() {
            "on" => true,
            "off" => false,
            _ => return Err(cfgio::t(lang, "expected on or off", "应为 on 或 off")),
        };
        agent_permission::set_enabled(target, enabled).map_err(|e| e.to_string())?;
        print_line(&cfgio::t(
            lang,
            "This changes future permission prompts only; approvals already delivered by AskHuman remain valid.",
            "此设置只影响后续权限请求；AskHuman 已投递的在途审批仍然有效。",
        ));
    }
    let status = agent_permission::status(target);
    print_line(&format!(
        "[{agent}] {} — {}",
        if status.enabled { "on" } else { "off" },
        if status.installed {
            cfgio::t(lang, "configured", "已配置")
        } else {
            cfgio::t(lang, "not configured", "未配置")
        }
    ));
    Ok(())
}

fn stop_cmd(args: &[String], lang: Lang) -> Result<(), String> {
    let agent = args.first().ok_or_else(|| {
        cfgio::t(
            lang,
            "usage: agents stop <claude|codex|cursor|grok|pi|antigravity> [on|off]",
            "用法: agents stop <claude|codex|cursor|grok|pi|antigravity> [on|off]",
        )
    })?;
    if args.len() > 2 {
        return Err(cfgio::t(lang, "too many arguments", "参数过多"));
    }
    let kind =
        AgentKind::parse(agent).ok_or_else(|| cfgio::t(lang, "unknown agent", "未知 agent"))?;
    if !agent_stop::supported(kind) {
        return Err(cfgio::t(
            lang,
            "Completion/error confirmation is supported for claude, codex, cursor, grok, pi, and antigravity",
            "结束／出错确认支持 claude、codex、cursor、grok、pi 与 antigravity",
        ));
    }
    if let Some(value) = args.get(1) {
        let enabled = match value.as_str() {
            "on" => true,
            "off" => false,
            _ => return Err(cfgio::t(lang, "expected on or off", "应为 on 或 off")),
        };
        agent_stop::set_enabled(kind, enabled).map_err(|error| error.to_string())?;
    }
    let state = agent_stop::status(kind);
    print_line(&format!(
        "[{agent}] {}{}",
        if state.enabled { "on" } else { "off" },
        if state.outdated {
            cfgio::t(lang, " (needs update)", "（需更新）")
        } else {
            String::new()
        }
    ));
    Ok(())
}

/// `agents delegated codex [on|off]`: whether delegated (non-interactive) runs may use AskHuman.
/// `on` = allow; the default (`off`) keeps them silent, as if AskHuman were not integrated.
fn delegated_cmd(args: &[String], lang: Lang) -> Result<(), String> {
    let agent = args.first().ok_or_else(|| {
        cfgio::t(
            lang,
            "usage: agents delegated codex [on|off]",
            "用法: agents delegated codex [on|off]",
        )
    })?;
    if args.len() > 2 {
        return Err(cfgio::t(lang, "too many arguments", "参数过多"));
    }
    let kind =
        AgentKind::parse(agent).ok_or_else(|| cfgio::t(lang, "unknown agent", "未知 agent"))?;
    if !agent_delegated::supported(kind) {
        return Err(cfgio::t(
            lang,
            "delegated-run handling is supported only for codex",
            "委派运行开关仅支持 codex",
        ));
    }
    if let Some(value) = args.get(1) {
        let allowed = match value.as_str() {
            "on" => true,
            "off" => false,
            _ => return Err(cfgio::t(lang, "expected on or off", "应为 on 或 off")),
        };
        agent_delegated::set_allowed(kind, allowed).map_err(|error| error.to_string())?;
        print_line(&cfgio::t(
            lang,
            "This affects runs that start afterwards only; runs already in progress keep their behavior.",
            "此设置只影响之后新启动的运行；已在进行的运行保持原有行为。",
        ));
    }
    let status = agent_delegated::status(kind);
    print_line(&format!(
        "[{agent}] {} — {}",
        if status.enabled { "on" } else { "off" },
        delegated_label(status.enabled, lang)
    ));
    Ok(())
}

/// Plain-language state of the delegated-run preference (shared with `agents show` / `doctor`).
pub(super) fn delegated_label(allowed: bool, lang: Lang) -> String {
    if allowed {
        cfgio::t(
            lang,
            "delegated runs may use AskHuman",
            "委派运行可使用 AskHuman",
        )
    } else {
        cfgio::t(
            lang,
            "delegated runs stay silent, as if AskHuman were not integrated (default)",
            "委派运行保持静默，与未集成 AskHuman 一致（默认）",
        )
    }
}

/// How the Codex protocol is delivered, in words (shared with `doctor`'s warning lines).
pub(super) fn protocol_delivery_label(delivery: Delivery, lang: Lang) -> String {
    match delivery {
        Delivery::NotInstalled => cfgio::t(lang, "not installed", "未安装"),
        Delivery::Trusted => cfgio::t(lang, "trusted", "已信任"),
        Delivery::Untrusted => cfgio::t(
            lang,
            "not trusted (Codex skips the hook, so sessions get no protocol)",
            "未信任（Codex 会跳过该 hook，会话收不到协议）",
        ),
        Delivery::NeedsUpdate => cfgio::t(lang, "needs update", "需更新"),
        Delivery::UnsupportedVersion => cfgio::t(
            lang,
            &format!(
                "unsupported Codex version (protocol injection needs >= {})",
                codex_protocol::MINIMUM_VERSION
            ),
            &format!(
                "该版本不支持协议注入（需要 Codex {} 或更高）",
                codex_protocol::MINIMUM_VERSION
            ),
        ),
    }
}

fn lifecycle_cmd(args: &[String], lang: Lang) -> Result<(), String> {
    let agent = args.first().ok_or_else(|| {
        cfgio::t(
            lang,
            "usage: agents lifecycle <agent> [on|off]",
            "用法: agents lifecycle <agent> [on|off]",
        )
    })?;
    if args.len() > 2 {
        return Err(cfgio::t(lang, "too many arguments", "参数过多"));
    }
    let kind =
        AgentKind::parse(agent).ok_or_else(|| cfgio::t(lang, "unknown agent", "未知 agent"))?;
    if let Some(value) = args.get(1) {
        match value.as_str() {
            "on" => agent_lifecycle::install(kind).map_err(|e| e.to_string())?,
            "off" => agent_lifecycle::uninstall(kind).map_err(|e| e.to_string())?,
            _ => return Err(cfgio::t(lang, "expected on or off", "应为 on 或 off")),
        };
    }
    let status = agent_lifecycle::status(kind);
    print_line(&format!(
        "[{agent}] {}; {}{}",
        if status.enabled { "on" } else { "off" },
        if status.installed {
            cfgio::t(lang, "configured", "已配置")
        } else {
            cfgio::t(lang, "not configured", "未配置")
        },
        if status.needs_update {
            cfgio::t(lang, " (needs update)", "（需更新）")
        } else {
            String::new()
        }
    ));
    Ok(())
}

fn cleanup_targets() -> Vec<(AgentTarget, AgentKind)> {
    aggregate_agents()
        .into_iter()
        .map(|name| {
            (
                AgentTarget::parse(name).unwrap(),
                AgentKind::parse(name).unwrap(),
            )
        })
        .collect()
}

fn cleanup_cmd(args: &[String], lang: Lang) -> Result<(), String> {
    if !args.is_empty() {
        return Err(cfgio::t(
            lang,
            "usage: agents cleanup",
            "用法: agents cleanup",
        ));
    }

    let mut errors = Vec::new();
    for (target, kind) in cleanup_targets() {
        if let Err(error) = agent_mode::set(target, agent_mode::Mode::None) {
            errors.push(format!("{} integration: {error}", kind.label()));
        }
    }
    if errors.is_empty() {
        print_line(&cfgio::t(
            lang,
            "Removed all AskHuman-managed Agent integrations.",
            "已移除全部由 AskHuman 托管的 Agent 集成。",
        ));
        Ok(())
    } else {
        Err(errors.join("\n"))
    }
}

fn legacy_write_error(command: &str, lang: Lang) -> String {
    cfgio::t(
        lang,
        &format!("agents {command} was removed; use `agents mode <agent> <cli|mcp|none>`, `agents update [agent]`, or an integration capability command"),
        &format!("agents {command} 已移除；请改用 `agents mode <agent> <cli|mcp|none>`、`agents update [agent]` 或集成内 capability 命令"),
    )
}

// ——— show（手动集成 + 状态）———

fn show(args: &[String], lang: Lang) -> Result<(), String> {
    let targets: Vec<&str> = match args.first().map(|s| s.as_str()) {
        Some(a) if !a.starts_with('-') => {
            if AgentTarget::parse(a).is_none() {
                return Err(cfgio::t(
                    lang,
                    &format!("unknown agent: {a}"),
                    &format!("未知 agent: {a}"),
                ));
            }
            vec![a]
        }
        _ => aggregate_agents(),
    };

    // Show the target-specific body for a single agent. The all-agent view keeps the generic
    // reference because one shared block cannot represent Codex and non-Codex scope rules.
    if targets == ["grok"] {
        print_line(&crate::prompts::grok_skill_body());
    } else if targets.len() == 1 {
        let kind = AgentKind::parse(targets[0]).unwrap();
        print_line(&crate::prompts::cli_reference_for(kind));
    } else {
        print_line(&crate::prompts::cli_reference());
    }
    print_line("");
    let yes = cfgio::t(lang, "installed", "已安装");
    let no = cfgio::t(lang, "not installed", "未安装");
    let upd = cfgio::t(lang, " (needs update)", "（需更新）");
    let na = cfgio::t(lang, "n/a", "不适用");

    for name in targets {
        let target = AgentTarget::parse(name).unwrap();
        let (mode, rule_installed, rule_outdated) = if target == AgentTarget::Antigravity {
            let (mode, updates, installed) =
                agent_mode::antigravity_status().map_err(|e| e.to_string())?;
            (mode, installed, updates.rule)
        } else {
            (
                agent_mode::current(target),
                agent_rules::is_installed(target),
                agent_rules::needs_update(target),
            )
        };
        let kind = AgentKind::parse(name).unwrap();
        print_line(&format!("[{}]", kind.label()));

        // 当前模式（三态聚合）
        print_line(&format!(
            "  {}: {}",
            cfgio::t(lang, "mode", "模式"),
            mode_label(mode, lang)
        ));

        // Rules. Codex's protocol travels through its SessionStart hook, so report how delivery
        // stands (trusted / not trusted / needs update / unsupported version) rather than a block.
        let rules = if target == AgentTarget::Codex {
            protocol_delivery_label(codex_protocol::delivery(), lang)
        } else if rule_installed {
            format!(
                "{yes}{}",
                if rule_outdated {
                    upd.clone()
                } else {
                    String::new()
                }
            )
        } else {
            no.clone()
        };
        // Grok 的指令载体是 skill（非 rules 文件），标签相应改为 skill。
        let rules_label = match target {
            AgentTarget::Grok => cfgio::t(lang, "skill", "skill"),
            AgentTarget::Codex => cfgio::t(
                lang,
                "protocol (SessionStart hook)",
                "协议（SessionStart Hook）",
            ),
            _ => cfgio::t(lang, "rules", "规则"),
        };
        print_line(&format!(
            "  {}: {} — {}",
            rules_label,
            rules,
            agent_rules::display_path(target)
        ));
        if target == AgentTarget::Codex {
            // The automatic integration needs no paste; a manual one still pastes into AGENTS.md.
            print_line(&format!(
                "  {}: {}",
                cfgio::t(lang, "manual paste target", "手动粘贴位置"),
                agent_rules::manual_paste_path(target)
            ));
            print_line(&format!(
                "  {}: {}",
                cfgio::t(lang, "delegated runs", "委派运行"),
                delegated_label(agent_delegated::status(kind).enabled, lang)
            ));
        }

        // Hook
        let hook = match target {
            AgentTarget::Cursor => hook_state(
                cursor_hook::is_installed(),
                cursor_hook::needs_update(),
                &yes,
                &no,
                &upd,
            ),
            AgentTarget::ClaudeCode => hook_state(
                claude_hook::is_installed(),
                claude_hook::needs_update(),
                &yes,
                &no,
                &upd,
            ),
            AgentTarget::Codex | AgentTarget::Grok | AgentTarget::Antigravity => na.clone(),
            AgentTarget::Pi => hook_state(
                agent_mode::timeout_hook_is_installed(target),
                agent_mode::timeout_hook_needs_update(target),
                &yes,
                &no,
                &upd,
            ),
        };
        let hook_label = if target == AgentTarget::Pi {
            cfgio::t(lang, "runtime extension", "运行时 Extension")
        } else {
            cfgio::t(lang, "timeout hook", "超时 hook")
        };
        print_line(&format!("  {}: {}", hook_label, hook));

        let permission = agent_permission::status(target);
        let permission_text = if !permission.supported {
            na.clone()
        } else {
            format!(
                "{}; {}{}",
                if permission.enabled { "on" } else { "off" },
                if permission.installed { &yes } else { &no },
                if permission.outdated { &upd } else { "" }
            )
        };
        print_line(&format!(
            "  {}: {}",
            cfgio::t(lang, "permission approval", "权限审批"),
            permission_text
        ));

        let stop = agent_stop::status(kind);
        let stop_text = if !stop.supported {
            na.clone()
        } else {
            format!(
                "{}; {}{}",
                if stop.enabled { "on" } else { "off" },
                if stop.installed { &yes } else { &no },
                if stop.outdated { &upd } else { "" }
            )
        };
        print_line(&format!(
            "  {}: {}",
            cfgio::t(lang, "Completion/error confirmation", "结束／出错确认"),
            stop_text
        ));

        // MCP 配置（用户级全局）
        let mcp = if !mcp_config::supported(target) {
            na.clone()
        } else if mcp_config::is_installed(target) {
            format!(
                "{yes}{}",
                if mcp_config::needs_update(target) {
                    upd.clone()
                } else {
                    String::new()
                }
            )
        } else {
            no.clone()
        };
        if mcp_config::supported(target) {
            print_line(&format!(
                "  {}: {} — {}",
                cfgio::t(lang, "mcp config", "MCP 配置"),
                mcp,
                mcp_config::display_path(target)
            ));
        } else {
            print_line(&format!(
                "  {}: {}",
                cfgio::t(lang, "mcp config", "MCP 配置"),
                mcp
            ));
        }

        // Lifecycle capability owned by the active automatic integration.
        let st = agent_lifecycle::status(kind);
        let lc = if !st.supported {
            na.clone()
        } else {
            format!(
                "{}; {}{}",
                if st.enabled { "on" } else { "off" },
                if st.installed { &yes } else { &no },
                if st.needs_update {
                    upd.clone()
                } else {
                    String::new()
                }
            )
        };
        print_line(&format!(
            "  {}: {}",
            cfgio::t(lang, "lifecycle tracking", "生命周期追踪"),
            lc
        ));
        print_antigravity_notes(target, mode, lang);
        print_profiles_summary(target, kind, mode, lang);
        print_line("");
    }
    Ok(())
}

fn print_profiles_summary(
    target: AgentTarget,
    kind: AgentKind,
    mode: agent_mode::Mode,
    lang: Lang,
) {
    let profiles = agent_profiles::statuses_for_agent(target, mode);
    let discovered = agent_profiles::discover_for_agent(kind);
    if !profiles.is_empty() {
        print_line(&format!(
            "  {}:",
            cfgio::t(lang, "extra config directories", "额外配置目录")
        ));
        for p in &profiles {
            let state = profile_state_label(&p.state, lang);
            print_line(&format!(
                "    - [{}] {} ({}) — {}",
                p.id, p.label, state, p.root
            ));
            if let Some(error) = &p.error {
                print_line(&format!("      ! {error}"));
            }
            if p.host_plugin_disabled {
                print_line(&cfgio::t(
                    lang,
                    "      ! Plugin is switched off in this Antigravity profile.",
                    "      ! 此 Antigravity 目录内插件已被停用。",
                ));
            }
        }
    }
    if !discovered.is_empty() {
        print_line(&format!(
            "  {}:",
            cfgio::t(
                lang,
                "discovered T3 directories (not yet enabled)",
                "发现的 T3 目录（尚未启用）"
            )
        ));
        for d in &discovered {
            print_line(&format!("    - {} — {}", d.label, d.root));
        }
        print_line(&format!(
            "    {}",
            cfgio::t(
                lang,
                "Enable with: AskHuman agents profile enable antigravity <path>",
                "启用方式: AskHuman agents profile enable antigravity <路径>"
            )
        ));
    }
}

pub(super) fn profile_state_label(state: &str, lang: Lang) -> String {
    match state {
        "synced" => cfgio::t(lang, "synced", "已同步"),
        "outdated" => cfgio::t(lang, "needs update", "需更新"),
        "notInstalled" => cfgio::t(lang, "not installed", "未安装"),
        "conflict" => cfgio::t(lang, "conflict", "冲突"),
        "rootMissing" => cfgio::t(lang, "directory missing", "目录不存在"),
        "disabled" => cfgio::t(lang, "disabled", "已停用"),
        other => other.to_string(),
    }
}

// ——— profile（多目录 / Extra Profiles）———

fn profile_cmd(args: &[String], lang: Lang) -> Result<(), String> {
    let sub = args.first().map(String::as_str).unwrap_or("list");
    let rest = &args[args.len().min(1)..];
    match sub {
        "list" => profile_list_cmd(rest, lang),
        "add" => profile_add_cmd(rest, lang),
        "enable" => profile_toggle_cmd(rest, true, lang),
        "disable" => profile_toggle_cmd(rest, false, lang),
        "remove" | "rm" => profile_remove_cmd(rest, lang),
        "sync" => profile_sync_cmd(rest, lang),
        "dismiss" => profile_dismiss_cmd(rest, lang),
        "help" | "-h" | "--help" => {
            print_line(&profile_help(lang));
            Ok(())
        }
        other => Err(cfgio::t(
            lang,
            &format!(
                "unknown profile subcommand: {other}\n\n{}",
                profile_help(lang)
            ),
            &format!("未知 profile 子命令: {other}\n\n{}", profile_help(lang)),
        )),
    }
}

fn profile_help(lang: Lang) -> String {
    cfgio::t(
        lang,
        "AskHuman agents profile — manage extra agent config directories\n\
\n\
  agents profile list [<agent>] [--json]              List configured and discovered directories\n\
  agents profile add <agent> <path> [--label <name>] [--disabled]  Enroll a custom config directory\n\
  agents profile enable <agent> <id|path>             Enable and sync a saved or discovered directory\n\
  agents profile disable <agent> <id|path> [--detach] Uninstall managed files and disable\n\
  agents profile remove <agent> <id|path> [--detach]  Uninstall (or detach) and remove entry\n\
  agents profile sync <agent> [<id|path>]             Reconcile one or all enabled extra directories\n\
  agents profile dismiss <agent> <path>               Ignore a discovered T3 directory",
        "AskHuman agents profile —— 管理额外 Agent 配置目录\n\
\n\
  agents profile list [<agent>] [--json]              列出已配置与自动发现的目录\n\
  agents profile add <agent> <路径> [--label <名称>] [--disabled]  添加自定义配置目录\n\
  agents profile enable <agent> <id|路径>             启用并同步已保存或自动发现的目录\n\
  agents profile disable <agent> <id|路径> [--detach] 卸载托管产物并停用\n\
  agents profile remove <agent> <id|路径> [--detach]  卸载（或仅取消托管）并删除条目\n\
  agents profile sync <agent> [<id|路径>]             同步单个或全部已启用额外目录\n\
  agents profile dismiss <agent> <路径>               忽略已发现的 T3 目录",
    )
}

fn profile_list_cmd(args: &[String], lang: Lang) -> Result<(), String> {
    let mut json = false;
    let mut agent_arg: Option<&str> = None;
    for arg in args {
        if arg == "--json" {
            json = true;
        } else if arg.starts_with('-') {
            return Err(format!("unknown option: {arg}"));
        } else if agent_arg.is_none() {
            agent_arg = Some(arg.as_str());
        } else {
            return Err(cfgio::t(lang, "too many arguments", "参数过多"));
        }
    }
    let names: Vec<&str> = if let Some(agent) = agent_arg {
        let _ = parse_target(agent, lang)?;
        vec![agent]
    } else {
        aggregate_agents()
    };

    let mut rows = Vec::new();
    for name in names {
        let target = AgentTarget::parse(name).unwrap();
        let kind = AgentKind::parse(name).unwrap();
        let mode = if target == AgentTarget::Antigravity {
            agent_mode::antigravity_status()
                .map(|(m, _, _)| m)
                .unwrap_or(agent_mode::Mode::None)
        } else {
            agent_mode::current(target)
        };
        let profiles = agent_profiles::statuses_for_agent(target, mode);
        let discovered = agent_profiles::discover_for_agent(kind);
        rows.push((name, kind, mode, profiles, discovered));
    }

    if json {
        let payload: Vec<Value> = rows
            .into_iter()
            .map(|(name, _, mode, profiles, discovered)| {
                serde_json::json!({
                    "agent": name,
                    "mode": mode.as_str(),
                    "profiles": profiles,
                    "discovered": discovered,
                })
            })
            .collect();
        print_line(&serde_json::to_string_pretty(&payload).unwrap_or_default());
        return Ok(());
    }

    let mut any = false;
    for (_, kind, mode, profiles, discovered) in rows {
        if profiles.is_empty() && discovered.is_empty() {
            continue;
        }
        any = true;
        print_line(&format!("[{}] ({})", kind.label(), mode_label(mode, lang)));
        for p in &profiles {
            print_line(&format!(
                "  {}  [{}] {} — {}",
                p.id,
                profile_state_label(&p.state, lang),
                p.label,
                if p.display_root.is_empty() {
                    &p.root
                } else {
                    &p.display_root
                }
            ));
            if let Some(error) = &p.error {
                print_line(&format!("    ! {error}"));
            }
        }
        for d in &discovered {
            print_line(&format!(
                "  ({}) {} — {}",
                cfgio::t(lang, "discovered", "新发现"),
                d.label,
                if d.display_root.is_empty() {
                    &d.root
                } else {
                    &d.display_root
                }
            ));
        }
    }
    if !any {
        print_line(&cfgio::t(
            lang,
            "No extra agent config directories configured or discovered.",
            "暂无已配置或自动发现的额外 Agent 配置目录。",
        ));
    }
    Ok(())
}

fn profile_add_cmd(args: &[String], lang: Lang) -> Result<(), String> {
    if args.len() < 2 {
        return Err(cfgio::t(
            lang,
            "usage: agents profile add <agent> <path> [--label <name>] [--disabled]",
            "用法: agents profile add <agent> <路径> [--label <名称>] [--disabled]",
        ));
    }
    let target = parse_target(&args[0], lang)?;
    let path = std::path::PathBuf::from(&args[1]);
    let mut label: Option<String> = None;
    let mut enable = true;
    let mut iter = args[2..].iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--label" => {
                let value = iter
                    .next()
                    .ok_or_else(|| cfgio::t(lang, "--label requires a value", "--label 需要值"))?;
                label = Some(value.clone());
            }
            "--disabled" => enable = false,
            other => return Err(format!("unknown option: {other}")),
        }
    }
    let status = agent_profiles::add(
        target,
        &path,
        label.as_deref(),
        agent_profiles::ProfileSource::Manual,
        enable,
    )
    .map_err(|e| format!("{e:#}"))?;
    print_line(&format!(
        "[{}] {} ({}) — {}",
        status.id,
        status.label,
        profile_state_label(&status.state, lang),
        status.root
    ));
    Ok(())
}

fn profile_toggle_cmd(args: &[String], enable: bool, lang: Lang) -> Result<(), String> {
    if args.len() < 2 {
        return Err(cfgio::t(
            lang,
            if enable {
                "usage: agents profile enable <agent> <id|path>"
            } else {
                "usage: agents profile disable <agent> <id|path> [--detach]"
            },
            if enable {
                "用法: agents profile enable <agent> <id|路径>"
            } else {
                "用法: agents profile disable <agent> <id|路径> [--detach]"
            },
        ));
    }
    let target = parse_target(&args[0], lang)?;
    let id_or_path = &args[1];
    let mut detach = false;
    for arg in &args[2..] {
        if !enable && arg == "--detach" {
            detach = true;
        } else {
            return Err(format!("unknown option: {arg}"));
        }
    }
    let status = agent_profiles::set_enabled(target, id_or_path, enable, detach)
        .map_err(|e| format!("{e:#}"))?;
    print_line(&format!(
        "[{}] {} ({}) — {}",
        status.id,
        status.label,
        profile_state_label(&status.state, lang),
        status.root
    ));
    Ok(())
}

fn profile_remove_cmd(args: &[String], lang: Lang) -> Result<(), String> {
    if args.len() < 2 {
        return Err(cfgio::t(
            lang,
            "usage: agents profile remove <agent> <id|path> [--detach]",
            "用法: agents profile remove <agent> <id|路径> [--detach]",
        ));
    }
    let target = parse_target(&args[0], lang)?;
    let id_or_path = &args[1];
    let mut detach = false;
    for arg in &args[2..] {
        if arg == "--detach" {
            detach = true;
        } else {
            return Err(format!("unknown option: {arg}"));
        }
    }
    agent_profiles::remove(target, id_or_path, detach).map_err(|e| format!("{e:#}"))?;
    print_line(&cfgio::t(
        lang,
        "Extra profile removed.",
        "已移除该额外配置目录。",
    ));
    Ok(())
}

fn profile_sync_cmd(args: &[String], lang: Lang) -> Result<(), String> {
    let agent = args.first().ok_or_else(|| {
        cfgio::t(
            lang,
            "usage: agents profile sync <agent> [<id|path>]",
            "用法: agents profile sync <agent> [<id|路径>]",
        )
    })?;
    let target = parse_target(agent, lang)?;
    if let Some(id_or_path) = args.get(1) {
        if args.len() > 2 {
            return Err(cfgio::t(lang, "too many arguments", "参数过多"));
        }
        let status = agent_profiles::sync_one(target, id_or_path).map_err(|e| format!("{e:#}"))?;
        print_line(&format!(
            "[{}] {} ({}) — {}",
            status.id,
            status.label,
            profile_state_label(&status.state, lang),
            status.root
        ));
    } else {
        let statuses = agent_profiles::sync_all(target).map_err(|e| format!("{e:#}"))?;
        print_line(&cfgio::t(
            lang,
            &format!(
                "Synced {} extra profile(s).",
                statuses.iter().filter(|s| s.enabled).count()
            ),
            &format!(
                "已同步 {} 个额外配置目录。",
                statuses.iter().filter(|s| s.enabled).count()
            ),
        ));
    }
    Ok(())
}

fn profile_dismiss_cmd(args: &[String], lang: Lang) -> Result<(), String> {
    if args.len() != 2 {
        return Err(cfgio::t(
            lang,
            "usage: agents profile dismiss <agent> <path>",
            "用法: agents profile dismiss <agent> <路径>",
        ));
    }
    let target = parse_target(&args[0], lang)?;
    agent_profiles::dismiss_discovered(target, std::path::Path::new(&args[1]))
        .map_err(|e| format!("{e:#}"))?;
    print_line(&cfgio::t(
        lang,
        "Discovered directory ignored.",
        "已忽略该发现目录。",
    ));
    Ok(())
}

fn hook_state(installed: bool, needs_update: bool, yes: &str, no: &str, upd: &str) -> String {
    if installed {
        format!("{yes}{}", if needs_update { upd } else { "" })
    } else {
        no.to_string()
    }
}

fn help(lang: Lang) -> String {
    cfgio::t(
        lang,
        "AskHuman agents — agent status + integrations (cursor | claude | codex | grok | pi | antigravity)\n\
\n\
  agents monitor [--json|--text]     Live agent status (opens a window when a GUI is available)\n\
  agents mode <agent> [none|cli|mcp] Switch the integration mode (omit to query); auto-swaps products\n\
  agents mode codex mcp [--env NAME=VALUE] Repeatable; merge explicit environment overrides\n\
  agents update [<agent>]            Refresh each current mode's complete managed bundle\n\
  agents permission <claude|codex> [on|off]  Query or set permission approval\n\
  agents stop <claude|codex|cursor|grok|pi|antigravity> [on|off]  Query or set completion/error confirmation\n\
  agents delegated codex [on|off]    Query or set whether delegated Codex runs (codex exec, review, plugins) may use AskHuman (on = allow; default off = silent)\n\
  agents lifecycle <agent> [on|off]  Query or set lifecycle tracking inside an active integration\n\
  agents profile <list|add|enable|disable|remove|sync|dismiss> ...  Manage extra config directories (e.g. T3 Code Antigravity, custom CODEX_HOME)\n\
  agents cleanup                     Remove every AskHuman-managed Agent artifact\n\
  agents sidecars [list [--json]|remove <id>|remove --all]  The Antigravity App sidecars AskHuman keeps for starting tasks\n\
  agents show [<agent>]              Manual-integration prompt + paste paths + install status\n\
\n\
  Modes: cli = rules + runtime + lifecycle where supported; mcp = rules/skill + MCP + lifecycle where supported; none = remove.\n\
  Grok only supports none | mcp (skill + MCP config); it has no CLI mode and no timeout hook.\n\
  Pi only supports none | cli; its runtime artifact is a managed Extension and Stop defaults on.\n\
  Antigravity supports none | cli | mcp on Unix as one AskHuman plugin shared by the app and agy; MCP is recommended; lifecycle tracking uses the plugin's hooks.\n\
  Codex receives its protocol from a SessionStart hook (no AGENTS.md block); delegated runs such as `codex exec` stay silent unless `agents delegated codex on`.\n\
  Legacy install/uninstall and per-artifact write flags have been removed.",
        "AskHuman agents —— agent 状态 + 集成（cursor | claude | codex | grok | pi | antigravity）\n\
\n\
  agents monitor [--json|--text]     实时 agent 状态（有 GUI 时开窗）\n\
  agents mode <agent> [none|cli|mcp] 切换集成模式（省略则查询）；自动切换底层产物\n\
  agents mode codex mcp [--env NAME=VALUE] 可重复，合并显式环境变量\n\
  agents update [<agent>]            更新当前模式的完整托管产物包\n\
  agents permission <claude|codex> [on|off]  查询或设置权限审批\n\
  agents stop <claude|codex|cursor|grok|pi|antigravity> [on|off]  查询或设置结束／出错确认\n\
  agents delegated codex [on|off]    查询或设置被委派的 Codex 运行（codex exec、review、插件等）是否可使用 AskHuman（on = 允许；默认 off = 静默）\n\
  agents lifecycle <agent> [on|off]  查询或设置已启用集成内的生命周期追踪\n\
  agents profile <list|add|enable|disable|remove|sync|dismiss> ...  管理额外配置目录（如 T3 Code Antigravity、自定义 CODEX_HOME）\n\
  agents cleanup                     移除全部由 AskHuman 托管的 Agent 产物\n\
  agents sidecars [list [--json]|remove <id>|remove --all]  AskHuman 为启动任务在 Antigravity App 里保留的 sidecar\n\
  agents show [<agent>]              手动集成提示词 + 粘贴位置 + 安装状态\n\
\n\
  模式: cli = 规则 + runtime + 受支持的生命周期；mcp = 规则/skill + MCP + 受支持的生命周期；none = 移除。\n\
  Grok 仅支持 none | mcp（skill + MCP 配置）；无 CLI 模式、无超时 hook。\n\
  Pi 仅支持 none | cli；运行时产物为托管 Extension，Stop 默认开启。\n\
  Antigravity 在 Unix 支持 none | cli | mcp，以一个 AskHuman 插件的形式供 App 与 agy 共用；推荐 MCP；生命周期追踪走插件的 hooks。\n\
  Codex 通过 SessionStart hook 注入协议（不再写 AGENTS.md 区块）；委派运行（如 codex exec）默认静默，可用 `agents delegated codex on` 放行。\n\
  旧 install/uninstall 与逐产物写 flags 已移除。",
    )
}

fn print_line(s: &str) {
    super::print_line(s);
}

#[cfg(test)]
mod mode_env_tests {
    use super::*;
    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|v| v.to_string()).collect()
    }
    #[test]
    fn parses_repeated_env_last_wins_and_empty_values() {
        let env = parse_mode_env(
            AgentTarget::Codex,
            agent_mode::Mode::Mcp,
            &args(&[
                "--env",
                "DISPLAY=:0",
                "--env",
                "DISPLAY=:1",
                "--env",
                "EMPTY=",
                "--env",
                "EQUAL=a=b",
                "--env",
                "1NAME=x",
                "--env",
                "WITH-DASH=y",
            ]),
        )
        .unwrap();
        assert_eq!(env["DISPLAY"], ":1");
        assert_eq!(env["EMPTY"], "");
        assert_eq!(env["EQUAL"], "a=b");
        assert_eq!(env["1NAME"], "x");
        assert_eq!(env["WITH-DASH"], "y");
    }
    #[test]
    fn rejects_invalid_and_unsupported_before_mutation() {
        for values in [
            vec!["--env"],
            vec!["--env", "NO_EQUALS"],
            vec!["--env", "=x"],
            vec!["--env", "NAME=a\0b"],
            vec!["--unknown"],
        ] {
            assert!(
                parse_mode_env(AgentTarget::Codex, agent_mode::Mode::Mcp, &args(&values)).is_err()
            );
        }
        for (target, mode) in [
            (AgentTarget::ClaudeCode, agent_mode::Mode::Mcp),
            (AgentTarget::Codex, agent_mode::Mode::Cli),
            (AgentTarget::Codex, agent_mode::Mode::None),
            (AgentTarget::Pi, agent_mode::Mode::Mcp),
            (AgentTarget::Antigravity, agent_mode::Mode::Cli),
            (AgentTarget::Antigravity, agent_mode::Mode::Mcp),
        ] {
            assert!(parse_mode_env(target, mode, &args(&["--env", "DISPLAY=:1"])).is_err());
        }
    }
}

#[cfg(test)]
mod antigravity_aggregate_tests {
    use super::*;

    #[test]
    fn aggregate_queries_preserve_existing_agents_and_dedicated_family_parse() {
        let targets = aggregate_agents();
        assert_eq!(&targets[..5], &["cursor", "claude", "codex", "grok", "pi"]);
        assert_eq!(targets.contains(&"antigravity"), cfg!(unix));
        assert_eq!(targets.len(), if cfg!(unix) { 6 } else { 5 });
        let cleanup = cleanup_targets();
        assert_eq!(cleanup.len(), targets.len());
        assert_eq!(
            cleanup.contains(&(AgentTarget::Antigravity, AgentKind::Antigravity)),
            cfg!(unix)
        );
        for (name, (target, kind)) in targets.iter().zip(cleanup) {
            assert_eq!(AgentTarget::parse(name), Some(target));
            assert_eq!(AgentKind::parse(name), Some(kind));
        }
        assert_eq!(
            AgentTarget::parse("antigravity"),
            Some(AgentTarget::Antigravity)
        );
    }

    #[test]
    fn aggregate_update_plan_keeps_healthy_targets_after_status_error() {
        let plan =
            plan_aggregate_updates(["cursor", "antigravity", "codex"], |target| match target {
                AgentTarget::Cursor => Ok(true),
                AgentTarget::Antigravity => Err("Antigravity state unavailable".into()),
                AgentTarget::Codex => Ok(false),
                _ => unreachable!(),
            });

        assert_eq!(
            plan.targets,
            vec![("cursor".to_string(), AgentTarget::Cursor)]
        );
        assert_eq!(
            plan.failures,
            vec![(
                "antigravity".to_string(),
                "Antigravity state unavailable".to_string()
            )]
        );
    }
}
