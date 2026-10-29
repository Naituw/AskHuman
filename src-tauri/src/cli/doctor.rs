//! `AskHuman doctor [--json]` —— 一屏体检：daemon / 渠道 / agent 集成 健康状态。
//! Reuses the error-preserving client status query, channel checks and integration status APIs.

use super::cfgio;
use super::channel_cmd;
use super::install_check;
use crate::client::diagnostics;
use crate::config::AppConfig;
use crate::i18n::Lang;
use crate::integrations::agent_rules::AgentTarget;
use crate::integrations::codex_protocol::{self, Delivery};
use crate::integrations::{
    agent_delegated, agent_lifecycle, agent_mode, agent_permission, agent_rules,
    agent_subagent_guard, claude_hook, cursor_hook, mcp_config,
};

fn daemon_login_ready() -> bool {
    crate::integrations::login_item::daemon_is_installed()
        && !crate::integrations::login_item::daemon_needs_update()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct IntegrationReady {
    mode: agent_mode::Mode,
    rule_installed: bool,
    rule_outdated: bool,
}

#[derive(Debug, PartialEq, Eq)]
struct IntegrationResult {
    name: &'static str,
    status: Result<IntegrationReady, String>,
}

fn collect_integration_status(target: AgentTarget) -> Result<IntegrationReady, String> {
    if target == AgentTarget::Antigravity {
        let (mode, updates, rule_installed) =
            agent_mode::antigravity_status().map_err(|error| error.to_string())?;
        Ok(IntegrationReady {
            mode,
            rule_installed,
            rule_outdated: updates.rule,
        })
    } else {
        Ok(IntegrationReady {
            mode: agent_mode::current(target),
            rule_installed: agent_rules::is_installed(target),
            rule_outdated: agent_rules::needs_update(target),
        })
    }
}

fn collect_integration_results(
    names: impl IntoIterator<Item = &'static str>,
    mut collect: impl FnMut(AgentTarget) -> Result<IntegrationReady, String>,
) -> Vec<IntegrationResult> {
    names
        .into_iter()
        .map(|name| IntegrationResult {
            name,
            status: collect(AgentTarget::parse(name).unwrap()),
        })
        .collect()
}

pub fn dispatch(args: &[String], lang: Lang) {
    // `doctor install` is the installed-binaries check on its own (`install.sh` calls it).
    if args.first().map(String::as_str) == Some("install") {
        install_check::dispatch(&args[1..], lang);
        return;
    }
    let json = args.iter().any(|a| a == "--json");
    if args
        .iter()
        .any(|a| a == "help" || a == "-h" || a == "--help")
    {
        super::print_line(&cfgio::t(
            lang,
            "AskHuman doctor [--json] — one-screen health check (daemon, installed binaries, channels, integrations)\nAskHuman doctor install [--warn-only] [--json] — only the installed-binaries check",
            "AskHuman doctor [--json] —— 一屏体检（daemon、已装二进制、渠道、集成）\nAskHuman doctor install [--warn-only] [--json] —— 只做已装二进制检查",
        ));
        return;
    }

    let cfg = AppConfig::load_without_secrets();
    let status = cfgio::daemon_status_result();
    let install = install_check::collect(status.as_ref().ok().and_then(Option::as_ref));

    let output = if json {
        render_json(&cfg, &status, &install, lang)
    } else {
        render_text(&cfg, &status, &install, lang)
    };
    match output {
        Ok((output, had_errors)) => {
            super::print_line(&output);
            if had_errors {
                std::process::exit(1);
            }
        }
        Err(error) => {
            eprintln!("Cannot render doctor report: {error}");
            std::process::exit(1);
        }
    }
}

fn render_text(
    cfg: &AppConfig,
    status: &std::io::Result<Option<crate::ipc::StatusInfo>>,
    install: &install_check::Report,
    lang: Lang,
) -> Result<(String, bool), String> {
    let yes = cfgio::t(lang, "yes", "是");
    let no = cfgio::t(lang, "no", "否");
    let yn = |b: bool| if b { yes.clone() } else { no.clone() };
    let mut out = String::new();

    // Daemon
    out.push_str(&render_daemon_text(status, lang));
    let status = status.as_ref().ok().and_then(Option::as_ref);

    if let Some(s) = status.as_ref().filter(|s| !s.exe_path.is_empty()) {
        out.push_str(&format!(
            "  {}: {}{}\n",
            cfgio::t(lang, "binary", "二进制"),
            s.exe_path,
            s.fingerprint
                .map(|fp| format!("  fp {}", fp.short()))
                .unwrap_or_default(),
        ));
    }

    // Install
    out.push('\n');
    out.push_str(&install_check::render_text(install, lang, false));
    out.push('\n');

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
    let mut had_errors = false;
    for IntegrationResult { name, status } in collect_integration_results(
        super::agents_cmd::aggregate_agents(),
        collect_integration_status,
    ) {
        let target = AgentTarget::parse(name).unwrap();
        let kind = crate::agents::AgentKind::parse(name).unwrap();
        let IntegrationReady {
            mode,
            rule_installed,
            rule_outdated,
        } = match status {
            Ok(status) => status,
            Err(error) => {
                had_errors = true;
                out.push_str(&format!(
                    "  {:<8} {}={}\n",
                    name,
                    cfgio::t(lang, "error", "错误"),
                    error
                ));
                continue;
            }
        };
        // Codex's protocol travels through its SessionStart hook, so report whether it actually
        // reaches sessions (a hook Codex does not trust is skipped silently).
        let delivery = (target == AgentTarget::Codex).then(codex_protocol::delivery);
        let (rules_key, rules) = match delivery {
            Some(delivery) => (
                cfgio::t(lang, "protocol", "协议"),
                delivery_state_label(delivery, lang),
            ),
            None => (
                cfgio::t(lang, "rules", "规则"),
                state_label(rule_installed, rule_outdated, lang),
            ),
        };
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
            AgentTarget::Codex | AgentTarget::Grok | AgentTarget::Antigravity => {
                cfgio::t(lang, "n/a", "—")
            }
            AgentTarget::Pi => state_label(
                agent_mode::timeout_hook_is_installed(target),
                agent_mode::timeout_hook_needs_update(target),
                lang,
            ),
        };
        let mcp = if mcp_config::supported(target) {
            state_label(
                mcp_config::is_installed(target),
                mcp_config::needs_update(target),
                lang,
            )
        } else {
            cfgio::t(lang, "n/a", "—")
        };
        let guard = if agent_subagent_guard::supported(target) {
            let status = agent_subagent_guard::status(target);
            state_label(
                status.installed,
                agent_subagent_guard::needs_update(target, agent_mode::current(target)),
                lang,
            )
        } else {
            cfgio::t(lang, "n/a", "—")
        };
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
            format!(
                "{}:{}",
                if lc.enabled { "on" } else { "off" },
                state_label(lc.installed, lc.needs_update, lang)
            )
        };
        let mut line = format!(
            "  {:<8} {}={} {}={} {}={} {}={} {}={} {}={} {}={}",
            name,
            cfgio::t(lang, "mode", "模式"),
            mode_label(mode, lang),
            rules_key,
            rules,
            cfgio::t(lang, "hook", "hook"),
            hook,
            cfgio::t(lang, "mcp", "mcp"),
            mcp,
            cfgio::t(lang, "subagent guard", "子 Agent 防护"),
            guard,
            cfgio::t(lang, "lifecycle", "生命周期"),
            lifecycle,
            cfgio::t(lang, "permission", "权限审批"),
            permission_label,
        );
        if let Some(delivery) = delivery {
            line.push_str(&format!(
                " {}={}",
                cfgio::t(lang, "delegated", "委派运行"),
                if agent_delegated::allowed(kind) {
                    cfgio::t(lang, "allowed", "允许")
                } else {
                    cfgio::t(lang, "silent", "静默")
                }
            ));
            out.push_str(&line);
            out.push('\n');
            // A protocol that does not arrive is a silent failure, so say it out loud.
            if agent_mode::current(target) != agent_mode::Mode::None
                && matches!(delivery, Delivery::Untrusted | Delivery::UnsupportedVersion)
            {
                out.push_str(&format!(
                    "  ! codex {}: {}{}\n",
                    cfgio::t(lang, "protocol", "协议"),
                    super::agents_cmd::protocol_delivery_label(delivery, lang),
                    if delivery == Delivery::Untrusted {
                        cfgio::t(
                            lang,
                            " — fix: AskHuman agents update codex",
                            " —— 修复：AskHuman agents update codex",
                        )
                    } else {
                        String::new()
                    }
                ));
            }
        } else {
            out.push_str(&line);
            out.push('\n');
        }
    }

    out.push_str(&cfgio::t(lang, "\nIM Agent tasks\n", "\nIM Agent 任务\n"));
    let workspaces = crate::agents::workspaces::list()
        .into_iter()
        .filter(|workspace| !workspace.hidden && std::path::Path::new(&workspace.path).is_dir())
        .count();
    out.push_str(&format!(
        "  {}={} keepalive={} login-item={} terminal={} workspaces={}\n",
        cfgio::t(lang, "enabled", "启用"),
        yn(cfg.agent_tasks.enabled),
        yn(cfg.general.daemon_lifecycle == crate::config::DaemonLifecycleMode::KeepAlive),
        yn(daemon_login_ready()),
        yn(crate::integrations::agent_launch::terminal_available()),
        workspaces,
    ));
    for item in crate::integrations::agent_launch::all_readiness() {
        out.push_str(&format!(
            "  {:<8} ready={} cli={} lifecycle={} integration={}\n",
            item.kind.as_str(),
            yn(item.ready),
            yn(item.binary_ready),
            yn(item.lifecycle_ready),
            yn(item.integration_ready)
        ));
    }

    let cursor = crate::cursor_desktop::status(&cfg.cursor_desktop);
    out.push_str(&format!(
        "\nCursor Desktop\n  background-followup-control: {} ({} windows)\n",
        cursor.phase,
        cursor.windows.len(),
    ));
    if let Some(error) = cursor.error {
        out.push_str(&format!("  {error}\n"));
    }
    Ok((out.trim_end().to_string(), had_errors))
}

fn render_daemon_text(
    status: &std::io::Result<Option<crate::ipc::StatusInfo>>,
    lang: Lang,
) -> String {
    let mut out = String::from("Daemon\n");
    match status {
        Ok(Some(s)) => out.push_str(&format!(
            "  {}: {}  v{}  {}: {}  {}: {}\n",
            cfgio::t(lang, "running", "运行中"),
            cfgio::t(lang, "yes", "是"),
            s.version,
            cfgio::t(lang, "in-flight", "在途请求"),
            s.active_requests,
            cfgio::t(lang, "IM connections", "IM 连接"),
            if s.im_connections.is_empty() {
                cfgio::t(lang, "none", "无")
            } else {
                s.im_connections.join(", ")
            },
        )),
        Ok(None) => out.push_str(&format!(
            "  {}: {}\n",
            cfgio::t(lang, "running", "运行中"),
            cfgio::t(lang, "no", "否")
        )),
        Err(error) => out.push_str(&format!(
            "  {}: {}\n  {}\n",
            cfgio::t(lang, "running", "运行中"),
            cfgio::t(lang, "unknown", "未知"),
            diagnostics::status_unavailable(error, lang),
        )),
    }
    out
}

fn render_daemon_json(
    status: &std::io::Result<Option<crate::ipc::StatusInfo>>,
    lang: Lang,
) -> serde_json::Value {
    match status {
        Ok(Some(s)) => serde_json::json!({
            "running": true,
            "version": s.version,
            "activeRequests": s.active_requests,
            "imConnections": s.im_connections,
            "exePath": s.exe_path,
            "fingerprint": s.fingerprint.map(|fp| fp.short()),
            "binaryConflict": s.binary_conflict,
        }),
        Ok(None) => serde_json::json!({ "running": false }),
        Err(error) => serde_json::json!({
            "running": null,
            "error": diagnostics::status_unavailable(error, lang),
        }),
    }
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

/// Compact `doctor` wording for Codex protocol delivery (mirrors [`state_label`]'s vocabulary).
fn delivery_state_label(delivery: Delivery, lang: Lang) -> String {
    match delivery {
        Delivery::NotInstalled => cfgio::t(lang, "off", "未装"),
        Delivery::Trusted => cfgio::t(lang, "ok", "正常"),
        Delivery::Untrusted => cfgio::t(lang, "untrusted", "未信任"),
        Delivery::NeedsUpdate => cfgio::t(lang, "stale", "需更新"),
        Delivery::UnsupportedVersion => cfgio::t(lang, "unsupported", "版本不支持"),
    }
}

fn mode_label(m: agent_mode::Mode, lang: Lang) -> String {
    match m {
        agent_mode::Mode::None => cfgio::t(lang, "off", "未集成"),
        agent_mode::Mode::Cli => "cli".to_string(),
        agent_mode::Mode::Mcp => "mcp".to_string(),
    }
}

fn integration_error_row(name: &str, error: &str) -> serde_json::Value {
    serde_json::json!({ "name": name, "error": error })
}

fn render_json(
    cfg: &AppConfig,
    status: &std::io::Result<Option<crate::ipc::StatusInfo>>,
    install: &install_check::Report,
    lang: Lang,
) -> Result<(String, bool), String> {
    let observed = status.as_ref().ok().and_then(Option::as_ref);
    let conns = observed
        .map(|s| s.im_connections.clone())
        .unwrap_or_default();
    let daemon_up = observed.is_some();
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
    let mut had_errors = false;
    let integrations: Vec<serde_json::Value> = collect_integration_results(
        super::agents_cmd::aggregate_agents(),
        collect_integration_status,
    )
        .into_iter()
        .map(|IntegrationResult { name, status }| {
            let target = AgentTarget::parse(name).unwrap();
            let kind = crate::agents::AgentKind::parse(name).unwrap();
            let IntegrationReady {
                mode,
                rule_installed,
                rule_outdated,
            } = match status {
                Ok(status) => status,
                Err(error) => {
                    had_errors = true;
                    return integration_error_row(name, &error);
                }
            };
            let hook = match target {
                AgentTarget::Cursor => Some((cursor_hook::is_installed(), cursor_hook::needs_update())),
                AgentTarget::ClaudeCode => Some((claude_hook::is_installed(), claude_hook::needs_update())),
                AgentTarget::Codex | AgentTarget::Grok | AgentTarget::Antigravity => None,
                AgentTarget::Pi => Some((
                    agent_mode::timeout_hook_is_installed(target),
                    agent_mode::timeout_hook_needs_update(target),
                )),
            };
            let lc = agent_lifecycle::status(kind);
            let permission = agent_permission::status(target);
            let guard = agent_subagent_guard::status(target);
            serde_json::json!({
                "name": name,
                "mode": mode.as_str(),
                "rules": { "installed": rule_installed, "needsUpdate": rule_outdated },
                // Codex only: how the protocol reaches sessions (SessionStart hook) and whether
                // delegated runs may use AskHuman.
                "protocol": (target == AgentTarget::Codex).then(|| serde_json::json!({
                    "delivery": codex_protocol::delivery().as_str(),
                    "minimumVersion": codex_protocol::MINIMUM_VERSION,
                })),
                "delegated": agent_delegated::supported(kind).then(|| agent_delegated::status(kind)),
                "hook": hook.map(|(i, u)| serde_json::json!({ "installed": i, "needsUpdate": u })),
                "subagentGuard": if agent_subagent_guard::supported(target) {
                    Some(serde_json::json!({
                        "installed": guard.installed,
                        "needsUpdate": agent_subagent_guard::needs_update(target, agent_mode::current(target))
                    }))
                } else {
                    None
                },
                "mcp": { "installed": mcp_config::is_installed(target), "needsUpdate": mcp_config::needs_update(target) },
                "permission": permission,
                "lifecycle": {
                    "enabled": lc.enabled,
                    "preferenceConfigured": lc.preference_configured,
                    "installed": lc.installed,
                    "needsUpdate": lc.needs_update,
                    "cleanupRequired": lc.cleanup_required,
                    "supported": lc.supported
                },
            })
        })
        .collect();
    let daemon = render_daemon_json(status, lang);
    let task_readiness = crate::integrations::agent_launch::all_readiness();
    let task_workspaces = crate::agents::workspaces::list()
        .into_iter()
        .filter(|workspace| !workspace.hidden && std::path::Path::new(&workspace.path).is_dir())
        .count();
    let output = serde_json::to_string_pretty(&serde_json::json!({
        "daemon": daemon,
        "install": install_check::render_json_value(install, lang),
        "channels": channels,
        "integrations": integrations,
        "cursorDesktop": crate::cursor_desktop::status(&cfg.cursor_desktop),
        "agentTasks": {
            "enabled": cfg.agent_tasks.enabled,
            "keepalive": cfg.general.daemon_lifecycle == crate::config::DaemonLifecycleMode::KeepAlive,
            "loginItemReady": daemon_login_ready(),
            "terminalReady": crate::integrations::agent_launch::terminal_available(),
            "workspaceCount": task_workspaces,
            "agents": task_readiness,
        },
    }))
    .map_err(|e| e.to_string())?;
    Ok((output, had_errors))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn integration_collection_continues_after_one_status_error() {
        let results =
            collect_integration_results(
                ["cursor", "antigravity", "codex"],
                |target| match target {
                    AgentTarget::Antigravity => Err("Antigravity state unavailable".into()),
                    AgentTarget::Cursor => Ok(IntegrationReady {
                        mode: agent_mode::Mode::Cli,
                        rule_installed: true,
                        rule_outdated: false,
                    }),
                    AgentTarget::Codex => Ok(IntegrationReady {
                        mode: agent_mode::Mode::None,
                        rule_installed: false,
                        rule_outdated: false,
                    }),
                    _ => unreachable!(),
                },
            );
        assert_eq!(
            results,
            [
                IntegrationResult {
                    name: "cursor",
                    status: Ok(IntegrationReady {
                        mode: agent_mode::Mode::Cli,
                        rule_installed: true,
                        rule_outdated: false,
                    }),
                },
                IntegrationResult {
                    name: "antigravity",
                    status: Err("Antigravity state unavailable".into()),
                },
                IntegrationResult {
                    name: "codex",
                    status: Ok(IntegrationReady {
                        mode: agent_mode::Mode::None,
                        rule_installed: false,
                        rule_outdated: false,
                    }),
                },
            ]
        );
    }

    #[test]
    fn integration_error_is_machine_readable_without_fake_artifact_state() {
        let row = integration_error_row("antigravity", "Antigravity state unavailable");
        assert_eq!(row["name"], "antigravity");
        assert_eq!(row["error"], "Antigravity state unavailable");
        assert!(row.get("mode").is_none());
        assert!(row.get("rules").is_none());
        assert!(row.get("mcp").is_none());
    }
}

#[cfg(test)]
mod daemon_tests {
    use super::*;

    fn running_status() -> crate::ipc::StatusInfo {
        serde_json::from_value(serde_json::json!({
            "pid": 42,
            "version": "0.16.0",
            "protocolVersion": 4,
            "uptimeSecs": 5,
            "socket": "/tmp/askhuman-test.sock",
            "activeRequests": 2,
            "imConnections": ["telegram"],
            "exePath": "/test/AskHuman",
        }))
        .unwrap()
    }

    #[test]
    fn running_daemon_keeps_existing_text_and_json_fields() {
        let status = Ok(Some(running_status()));
        assert_eq!(
            render_daemon_text(&status, Lang::En),
            "Daemon\n  running: yes  v0.16.0  in-flight: 2  IM connections: telegram\n"
        );
        assert_eq!(
            render_daemon_json(&status, Lang::En),
            serde_json::json!({
                "running": true,
                "version": "0.16.0",
                "activeRequests": 2,
                "imConnections": ["telegram"],
                "exePath": "/test/AskHuman",
                "fingerprint": null,
                "binaryConflict": null,
            })
        );
    }

    #[test]
    fn absent_daemon_remains_not_running_without_error() {
        let status = Ok(None);
        assert_eq!(
            render_daemon_json(&status, Lang::En),
            serde_json::json!({"running": false})
        );
        assert_eq!(
            render_daemon_text(&status, Lang::En),
            "Daemon\n  running: no\n"
        );
        assert_eq!(
            render_daemon_text(&status, Lang::Zh),
            "Daemon\n  运行中: 否\n"
        );
    }

    #[cfg(unix)]
    #[test]
    fn denied_daemon_status_reports_unknown_with_original_error_in_both_formats() {
        for errno in [libc::EPERM, libc::EACCES] {
            let error = std::io::Error::from_raw_os_error(errno);
            let reason = error.to_string();
            let status = Err(error);
            for (lang, unknown, hint) in [
                (Lang::En, "running: unknown", "may be blocked by a sandbox"),
                (Lang::Zh, "运行中: 未知", "可能被沙箱"),
            ] {
                let daemon = render_daemon_json(&status, lang);
                assert!(daemon["running"].is_null());
                assert!(daemon["error"].as_str().unwrap().contains(&reason));
                assert!(daemon["error"].as_str().unwrap().contains(hint));
                let text = render_daemon_text(&status, lang);
                assert!(text.contains(unknown));
                assert!(text.contains(&reason));
                assert!(text.contains(hint));
                assert!(!text.contains("running: no\n"));
            }
        }
    }

    #[test]
    fn failed_status_exchange_does_not_claim_daemon_is_stopped() {
        let status = Err(std::io::Error::new(
            std::io::ErrorKind::UnexpectedEof,
            "daemon status response lost",
        ));
        let daemon = render_daemon_json(&status, Lang::En);
        assert!(daemon["running"].is_null());
        assert!(daemon["error"]
            .as_str()
            .unwrap()
            .contains("daemon status response lost"));
        assert!(!daemon["error"].as_str().unwrap().contains("sandbox"));
    }
}
