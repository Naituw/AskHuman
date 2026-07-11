//! `AskHuman agents <monitor|show|install|uninstall|update|help>` —— Agent 实时状态 + 集成。
//! 解决与原 `agents status`（GUI 窗口）命名冲突：状态窗口改名 `monitor`（增文本 / `--json`），
//! 集成动词 install/uninstall/update/show 复用 `integrations::{agent_rules,cursor_hook,claude_hook,agent_lifecycle}`。

use super::cfgio;
use crate::agents::AgentKind;
use crate::i18n::{err_prefix, Lang};
use crate::integrations::agent_rules::AgentTarget;
use crate::integrations::{
    agent_lifecycle, agent_mode, agent_rules, claude_hook, cursor_hook, mcp_config,
};
use serde_json::Value;
use std::process::exit;

const AGENTS: [&str; 4] = ["cursor", "claude", "codex", "grok"];

pub fn dispatch(args: &[String], lang: Lang) {
    // 无子命令 → 打印 help（与 channel/config 一致；不再默认开状态窗口）。
    let sub = args.first().map(|s| s.as_str()).unwrap_or("help");
    let rest = &args[args.len().min(1)..];
    let r = match sub {
        "monitor" => monitor(rest, lang),
        "mode" => mode_cmd(rest, lang),
        "install" => integrate(rest, Action::Install, lang),
        "uninstall" => integrate(rest, Action::Uninstall, lang),
        "update" => integrate(rest, Action::Update, lang),
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

    #[cfg(unix)]
    {
        if !json && !text && gui_available() {
            // 彻底路由到统一 GUI 宿主（全局单窗，spec D3）：宿主在则聚焦/新建 Agent 窗口、不在则拉起。
            if crate::gui_host::host_open(crate::gui_host::WindowKind::Agents, false, None, None)
                .is_ok()
            {
                exit(0);
            }
            // 兜底（宿主起不来）：本进程直接建窗。run_agents 进入事件循环并不会返回（-> !）。
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
    #[cfg(not(unix))]
    {
        let _ = (json, text);
        Err(cfgio::t(
            lang,
            "agents monitor requires the daemon (unsupported on this platform)",
            "agents monitor 依赖 daemon（当前平台暂不支持）",
        ))
    }
}

#[cfg(target_os = "macos")]
fn gui_available() -> bool {
    true
}
#[cfg(all(unix, not(target_os = "macos")))]
fn gui_available() -> bool {
    std::env::var_os("DISPLAY").is_some() || std::env::var_os("WAYLAND_DISPLAY").is_some()
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
            &format!("unknown agent: {agent} (expected cursor|claude|codex|grok)"),
            &format!("未知 agent: {agent}（应为 cursor|claude|codex|grok）"),
        )
    })?;
    let kind = AgentKind::parse(agent).unwrap();

    match args.get(1) {
        // 仅查询：当前模式 + 是否需更新。
        None => {
            let upd = if agent_mode::needs_update(target) {
                cfgio::t(lang, " (needs update)", "（需更新）")
            } else {
                String::new()
            };
            print_line(&format!(
                "[{}] {}{}",
                kind.label(),
                mode_label(agent_mode::current(target), lang),
                upd
            ));
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
            agent_mode::set(target, mode).map_err(|e| e.to_string())?;
            print_line(&format!(
                "[{}] {} {}",
                kind.label(),
                cfgio::t(lang, "mode set to", "模式已设为"),
                mode_label(mode, lang)
            ));
            Ok(())
        }
    }
}

fn mode_label(m: agent_mode::Mode, lang: Lang) -> String {
    match m {
        agent_mode::Mode::None => cfgio::t(lang, "off", "未集成"),
        agent_mode::Mode::Cli => "CLI".to_string(),
        agent_mode::Mode::Mcp => "MCP".to_string(),
    }
}

// ——— 集成 install / uninstall / update ———

#[derive(Clone, Copy, PartialEq)]
enum Action {
    Install,
    Uninstall,
    Update,
}

fn integrate(args: &[String], action: Action, lang: Lang) -> Result<(), String> {
    let agent = args.first().ok_or_else(|| {
        cfgio::t(
            lang,
            "usage: agents <install|uninstall|update> <agent> [--rules] [--hook] [--mcp] [--lifecycle]",
            "用法: agents <install|uninstall|update> <agent> [--rules] [--hook] [--mcp] [--lifecycle]",
        )
    })?;
    let target = AgentTarget::parse(agent).ok_or_else(|| {
        cfgio::t(
            lang,
            &format!("unknown agent: {agent} (expected cursor|claude|codex|grok)"),
            &format!("未知 agent: {agent}（应为 cursor|claude|codex|grok）"),
        )
    })?;
    let kind = AgentKind::parse(agent).unwrap();

    let want_rules = args.iter().any(|a| a == "--rules");
    let want_hook = args.iter().any(|a| a == "--hook");
    let want_mcp = args.iter().any(|a| a == "--mcp");
    let want_lifecycle = args.iter().any(|a| a == "--lifecycle");
    if !want_rules && !want_hook && !want_mcp && !want_lifecycle {
        return Err(cfgio::t(
            lang,
            "specify at least one of --rules / --hook / --mcp / --lifecycle (no default bundle)",
            "至少指定 --rules / --hook / --mcp / --lifecycle 之一（无默认捆绑）",
        ));
    }

    if want_rules {
        let r = match action {
            Action::Install => agent_rules::install(target),
            Action::Update => agent_rules::update(target),
            Action::Uninstall => agent_rules::uninstall(target),
        };
        report("rules", r, lang);
    }
    if want_hook {
        match hook_action(target, action) {
            Some(r) => report("hook", r, lang),
            None => print_line(&cfgio::t(
                lang,
                &format!("hook: skipped ({agent} has no hooks)"),
                &format!("hook: 跳过（{agent} 无 hook）"),
            )),
        }
    }
    if want_mcp {
        let r = match action {
            Action::Install => mcp_config::install(target),
            Action::Update => mcp_config::update(target),
            Action::Uninstall => mcp_config::uninstall(target),
        };
        report("mcp", r, lang);
    }
    if want_lifecycle {
        let r = match action {
            // 生命周期无独立 update：重装即刷新（幂等 upsert）。
            Action::Install | Action::Update => agent_lifecycle::install(kind),
            Action::Uninstall => agent_lifecycle::uninstall(kind),
        };
        report("lifecycle", r, lang);
    }
    Ok(())
}

/// Hook 包操作：超时 hook（cursor/claude）+ permission hook（claude/codex）。
/// 返回 None 当该 agent 无任何 hook 支持时。
fn hook_action(target: AgentTarget, action: Action) -> Option<anyhow::Result<String>> {
    use crate::integrations::permission_hook;

    let has_timeout = agent_mode::timeout_hook_supported(target);
    let has_perm = permission_hook::supported(target);
    if !has_timeout && !has_perm {
        return None;
    }

    let mut results: Vec<String> = Vec::new();

    if has_timeout {
        let r = match target {
            AgentTarget::Cursor => match action {
                Action::Install => cursor_hook::install(),
                Action::Update => cursor_hook::update(),
                Action::Uninstall => cursor_hook::uninstall(),
            },
            AgentTarget::ClaudeCode => match action {
                Action::Install => claude_hook::install(),
                Action::Update => claude_hook::update(),
                Action::Uninstall => claude_hook::uninstall(),
            },
            _ => Ok("skipped".to_string()),
        };
        match r {
            Ok(msg) => results.push(format!("timeout: {msg}")),
            Err(e) => return Some(Err(e)),
        }
    }

    if has_perm {
        let r = match action {
            Action::Install | Action::Update => permission_hook::install(target),
            Action::Uninstall => permission_hook::uninstall(target),
        };
        match r {
            Ok(()) => results.push("permission: ok".to_string()),
            Err(e) => return Some(Err(e)),
        }
    }

    Some(Ok(results.join("; ")))
}

fn report(part: &str, r: anyhow::Result<String>, lang: Lang) {
    match r {
        Ok(msg) => print_line(&format!("{part}: {msg}")),
        Err(e) => print_line(&format!(
            "{part}: {}{}",
            cfgio::t(lang, "error: ", "错误: "),
            e
        )),
    }
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
        _ => AGENTS.to_vec(),
    };

    // 参考提示词：单独查看 Grok 时给「skill 正文」（其指令载体是 skill、非 shell 版规则），
    // 其余（含默认列出全部）沿用共享的 CLI 参考正文。
    if targets == ["grok"] {
        print_line(&crate::prompts::grok_skill_body());
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
        let kind = AgentKind::parse(name).unwrap();
        print_line(&format!("[{}]", kind.label()));

        // 当前模式（三态聚合）
        print_line(&format!(
            "  {}: {}",
            cfgio::t(lang, "mode", "模式"),
            mode_label(agent_mode::current(target), lang)
        ));

        // Rules
        let rules = if agent_rules::is_installed(target) {
            format!(
                "{yes}{}",
                if agent_rules::needs_update(target) {
                    upd.clone()
                } else {
                    String::new()
                }
            )
        } else {
            no.clone()
        };
        // Grok 的指令载体是 skill（非 rules 文件），标签相应改为 skill。
        let rules_label = if matches!(target, AgentTarget::Grok) {
            cfgio::t(lang, "skill", "skill")
        } else {
            cfgio::t(lang, "rules", "规则")
        };
        print_line(&format!(
            "  {}: {} — {}",
            rules_label,
            rules,
            agent_rules::display_path(target)
        ));

        // Hooks
        {
            use crate::integrations::permission_hook;
            let timeout = match target {
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
                AgentTarget::Codex | AgentTarget::Grok => na.clone(),
            };
            print_line(&format!(
                "  {}: {}",
                cfgio::t(lang, "timeout hook", "超时 hook"),
                timeout
            ));

            let perm = if permission_hook::supported(target) {
                hook_state(
                    permission_hook::is_installed(target),
                    permission_hook::needs_update(target),
                    &yes,
                    &no,
                    &upd,
                )
            } else {
                na.clone()
            };
            print_line(&format!(
                "  {}: {}",
                cfgio::t(lang, "permission hook", "权限 hook"),
                perm
            ));
        }

        // MCP 配置（用户级全局）
        let mcp = if mcp_config::is_installed(target) {
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
        print_line(&format!(
            "  {}: {} — {}",
            cfgio::t(lang, "mcp config", "MCP 配置"),
            mcp,
            mcp_config::display_path(target)
        ));

        // Lifecycle（实验性）
        let st = agent_lifecycle::status(kind);
        let lc = if !st.supported {
            na.clone()
        } else if st.installed {
            format!(
                "{yes}{}",
                if st.outdated {
                    upd.clone()
                } else {
                    String::new()
                }
            )
        } else {
            no.clone()
        };
        print_line(&format!(
            "  {}: {}",
            cfgio::t(
                lang,
                "lifecycle hook (experimental)",
                "生命周期 hook（实验性）"
            ),
            lc
        ));
        print_line("");
    }
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
        "AskHuman agents — agent status + integrations (cursor | claude | codex | grok)\n\
\n\
  agents monitor [--json|--text]     Live agent status (opens a window when a GUI is available)\n\
  agents mode <agent> [none|cli|mcp] Switch the integration mode (omit to query); auto-swaps products\n\
  agents show [<agent>]              Manual-integration prompt + paste paths + install status\n\
  agents install <agent>   --rules --hook --mcp --lifecycle    Auto-integrate (pick at least one)\n\
  agents uninstall <agent> [flags]   Remove the selected integrations\n\
  agents update <agent> [flags]      Refresh managed products to the latest\n\
\n\
  Modes: cli = rules + hooks;  mcp = rules/skill + MCP server config + permission hook;  none = remove.\n\
  Grok only supports none | mcp (skill + MCP config); it has no CLI mode and no hooks.\n\
\n\
  --rules      global prompt rules (cursor/claude/codex); for grok this installs the AskHuman skill\n\
  --hook       hook package (timeout: cursor/claude; permission: claude/codex)\n\
  --mcp        MCP server config (user-level global; all agents)\n\
  --lifecycle  lifecycle hook (experimental; all agents)",
        "AskHuman agents —— agent 状态 + 集成（cursor | claude | codex | grok）\n\
\n\
  agents monitor [--json|--text]     实时 agent 状态（有 GUI 时开窗）\n\
  agents mode <agent> [none|cli|mcp] 切换集成模式（省略则查询）；自动切换底层产物\n\
  agents show [<agent>]              手动集成提示词 + 粘贴位置 + 安装状态\n\
  agents install <agent>   --rules --hook --mcp --lifecycle    自动集成（至少选一项）\n\
  agents uninstall <agent> [选项]    移除所选集成\n\
  agents update <agent> [选项]       把托管的产物刷新到最新\n\
\n\
  模式: cli = 规则 + 超时 hook；mcp = 规则/skill + MCP server 配置；none = 移除。\n\
  Grok 仅支持 none | mcp（skill + MCP 配置）；无 CLI 模式、无超时 hook。\n\
\n\
  --rules      全局提示词规则（cursor/claude/codex）；对 grok 则安装 AskHuman skill\n\
  --hook       超时 hook（仅 cursor 与 claude；codex/grok 跳过）\n\
  --mcp        MCP server 配置（用户级全局；全部支持）\n\
  --lifecycle  生命周期 hook（实验性；全部支持）",
    )
}

fn print_line(s: &str) {
    super::print_line(s);
}
