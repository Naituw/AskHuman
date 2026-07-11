//! `AskHuman doctor [--json]` —— 一屏体检：daemon / 渠道 / agent 集成 健康状态。
//! 复用 `client::request_status`、`channel_cmd` 的判定、`integrations::*` 的状态查询。

use super::cfgio;
use super::channel_cmd;
use crate::config::AppConfig;
use crate::i18n::Lang;
use crate::integrations::agent_rules::AgentTarget;
use crate::integrations::{
    agent_lifecycle, agent_mode, agent_permission, agent_rules, claude_hook, cursor_hook,
    mcp_config,
};

const AGENTS: [&str; 4] = ["cursor", "claude", "codex", "grok"];

pub fn dispatch(args: &[String], lang: Lang) {
    let json = args.iter().any(|a| a == "--json");
    if args
        .iter()
        .any(|a| a == "help" || a == "-h" || a == "--help")
    {
        super::print_line(&cfgio::t(
            lang,
            "AskHuman doctor [--json] — one-screen health check (daemon, channels, integrations)",
            "AskHuman doctor [--json] —— 一屏体检（daemon、渠道、集成）",
        ));
        return;
    }

    let cfg = AppConfig::load_without_secrets();
    let status = cfgio::daemon_status();

    if json {
        super::print_line(&render_json(&cfg, &status));
    } else {
        super::print_line(&render_text(&cfg, &status, lang));
    }
}

fn render_text(cfg: &AppConfig, status: &Option<crate::ipc::StatusInfo>, lang: Lang) -> String {
    let yes = cfgio::t(lang, "yes", "是");
    let no = cfgio::t(lang, "no", "否");
    let yn = |b: bool| if b { yes.clone() } else { no.clone() };
    let mut out = String::new();

    // Daemon
    out.push_str(&cfgio::t(lang, "Daemon\n", "Daemon\n"));
    match status {
        Some(s) => {
            out.push_str(&format!(
                "  {}: {}  v{}  {}: {}  {}: {}\n",
                cfgio::t(lang, "running", "运行中"),
                yes,
                s.version,
                cfgio::t(lang, "in-flight", "在途请求"),
                s.active_requests,
                cfgio::t(lang, "IM connections", "IM 连接"),
                if s.im_connections.is_empty() {
                    cfgio::t(lang, "none", "无")
                } else {
                    s.im_connections.join(", ")
                },
            ));
        }
        None => out.push_str(&format!(
            "  {}: {}\n",
            cfgio::t(lang, "running", "运行中"),
            no
        )),
    }

    // Channels
    let conns = status
        .as_ref()
        .map(|s| s.im_connections.clone())
        .unwrap_or_default();
    let daemon_up = status.is_some();
    out.push_str(&cfgio::t(lang, "\nChannels\n", "\n渠道\n"));
    for &name in &channel_cmd::CHANNELS {
        let connected = if daemon_up {
            yn(conns.contains(&channel_cmd::conn_name(name).to_string()))
        } else {
            cfgio::t(lang, "n/a", "—")
        };
        out.push_str(&format!(
            "  {:<10} {}={} {}={} {}={}\n",
            name,
            cfgio::t(lang, "enabled", "启用"),
            yn(channel_cmd::is_enabled(cfg, name)),
            cfgio::t(lang, "configured", "齐全"),
            yn(channel_cmd::is_configured(cfg, name)),
            cfgio::t(lang, "connected", "连接"),
            connected,
        ));
    }

    // Integrations
    out.push_str(&cfgio::t(lang, "\nIntegrations\n", "\n集成\n"));
    for &name in &AGENTS {
        let target = AgentTarget::parse(name).unwrap();
        let kind = crate::agents::AgentKind::parse(name).unwrap();
        let rules = state_label(
            agent_rules::is_installed(target),
            agent_rules::needs_update(target),
            lang,
        );
        let hook = match target {
            AgentTarget::Cursor => state_label(
                cursor_hook::is_installed(),
                cursor_hook::needs_update(),
                lang,
            ),
            AgentTarget::ClaudeCode => state_label(
                claude_hook::is_installed(),
                claude_hook::needs_update(),
                lang,
            ),
            AgentTarget::Codex | AgentTarget::Grok => cfgio::t(lang, "n/a", "—"),
        };
        let mcp = state_label(
            mcp_config::is_installed(target),
            mcp_config::needs_update(target),
            lang,
        );
        let lc = agent_lifecycle::status(kind);
        let permission = agent_permission::status(target);
        let permission_label = if !permission.supported {
            cfgio::t(lang, "unsupported", "不支持")
        } else {
            let mut label = format!(
                "{}:{}",
                if permission.enabled { "on" } else { "off" },
                state_label(permission.installed, permission.outdated, lang)
            );
            if let Some(reason) = &permission.known_blocked_reason {
                label.push_str(&format!(" blocked={reason}"));
            }
            if permission.other_handlers_detected {
                label.push_str(" coexist=detected");
            }
            label
        };
        let lifecycle = if !lc.supported {
            cfgio::t(lang, "n/a", "—")
        } else {
            state_label(lc.installed, lc.outdated, lang)
        };
        out.push_str(&format!(
            "  {:<8} {}={} {}={} {}={} {}={} {}={} {}={}\n",
            name,
            cfgio::t(lang, "mode", "模式"),
            mode_label(agent_mode::current(target), lang),
            cfgio::t(lang, "rules", "规则"),
            rules,
            cfgio::t(lang, "hook", "hook"),
            hook,
            cfgio::t(lang, "mcp", "mcp"),
            mcp,
            cfgio::t(lang, "lifecycle", "生命周期"),
            lifecycle,
            cfgio::t(lang, "permission", "权限审批"),
            permission_label,
        ));
    }

    out.trim_end().to_string()
}

fn state_label(installed: bool, needs_update: bool, lang: Lang) -> String {
    if !installed {
        cfgio::t(lang, "off", "未装")
    } else if needs_update {
        cfgio::t(lang, "stale", "需更新")
    } else {
        cfgio::t(lang, "ok", "正常")
    }
}

fn mode_label(m: agent_mode::Mode, lang: Lang) -> String {
    match m {
        agent_mode::Mode::None => cfgio::t(lang, "off", "未集成"),
        agent_mode::Mode::Cli => "cli".to_string(),
        agent_mode::Mode::Mcp => "mcp".to_string(),
    }
}

fn render_json(cfg: &AppConfig, status: &Option<crate::ipc::StatusInfo>) -> String {
    let conns = status
        .as_ref()
        .map(|s| s.im_connections.clone())
        .unwrap_or_default();
    let daemon_up = status.is_some();
    let channels: Vec<serde_json::Value> = channel_cmd::CHANNELS
        .iter()
        .map(|&name| {
            let connected = if daemon_up {
                serde_json::Value::Bool(conns.contains(&channel_cmd::conn_name(name).to_string()))
            } else {
                serde_json::Value::Null
            };
            serde_json::json!({
                "name": name,
                "enabled": channel_cmd::is_enabled(cfg, name),
                "configured": channel_cmd::is_configured(cfg, name),
                "connected": connected,
            })
        })
        .collect();
    let integrations: Vec<serde_json::Value> = AGENTS
        .iter()
        .map(|&name| {
            let target = AgentTarget::parse(name).unwrap();
            let kind = crate::agents::AgentKind::parse(name).unwrap();
            let hook = match target {
                AgentTarget::Cursor => Some((cursor_hook::is_installed(), cursor_hook::needs_update())),
                AgentTarget::ClaudeCode => Some((claude_hook::is_installed(), claude_hook::needs_update())),
                AgentTarget::Codex | AgentTarget::Grok => None,
            };
            let lc = agent_lifecycle::status(kind);
            let permission = agent_permission::status(target);
            serde_json::json!({
                "name": name,
                "mode": agent_mode::current(target).as_str(),
                "rules": { "installed": agent_rules::is_installed(target), "needsUpdate": agent_rules::needs_update(target) },
                "hook": hook.map(|(i, u)| serde_json::json!({ "installed": i, "needsUpdate": u })),
                "mcp": { "installed": mcp_config::is_installed(target), "needsUpdate": mcp_config::needs_update(target) },
                "permission": permission,
                "lifecycle": { "installed": lc.installed, "needsUpdate": lc.outdated, "supported": lc.supported },
            })
        })
        .collect();
    let daemon = match status {
        Some(s) => serde_json::json!({
            "running": true,
            "version": s.version,
            "activeRequests": s.active_requests,
            "imConnections": s.im_connections,
        }),
        None => serde_json::json!({ "running": false }),
    };
    serde_json::to_string_pretty(&serde_json::json!({
        "daemon": daemon,
        "channels": channels,
        "integrations": integrations,
    }))
    .unwrap_or_default()
}
