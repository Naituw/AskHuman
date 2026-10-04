use super::{runtime, Config, LaunchTarget};
use crate::i18n::Lang;

pub fn integration_enabled() -> bool {
    cfg!(target_os = "macos")
        && crate::integrations::agent_mode::current(
            crate::integrations::agent_rules::AgentTarget::Codex,
        ) != crate::integrations::agent_mode::Mode::None
}

pub fn available(config: &Config) -> bool {
    integration_enabled() && runtime::detect(config).is_ok()
}

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LaunchStatus {
    pub preference: LaunchTarget,
    pub target: Option<LaunchTarget>,
    pub desktop_available: bool,
    pub terminal_available: bool,
    pub integrated: bool,
    pub desktop_reason: Option<UnavailableReason>,
    pub terminal_reason: Option<UnavailableReason>,
}

pub fn select_target(
    preference: LaunchTarget,
    desktop: bool,
    terminal: bool,
) -> Option<LaunchTarget> {
    match (preference, desktop, terminal) {
        (LaunchTarget::Desktop, true, _) => Some(LaunchTarget::Desktop),
        (LaunchTarget::Terminal, _, true) => Some(LaunchTarget::Terminal),
        (_, true, _) => Some(LaunchTarget::Desktop),
        (_, _, true) => Some(LaunchTarget::Terminal),
        _ => None,
    }
}

pub fn launch_status(force: bool) -> LaunchStatus {
    use crate::agents::AgentKind;
    use crate::integrations::{agent_launch, agent_lifecycle};
    let config = crate::config::AppConfig::load_without_secrets().codex_desktop;
    let cli = if force {
        agent_launch::readiness_fresh(AgentKind::Codex)
    } else {
        agent_launch::readiness(AgentKind::Codex)
    };
    let integrated = cli.integration_mode != "none";
    let desktop_reason = if !integrated {
        Some(UnavailableReason::IntegrationDisabled)
    } else {
        runtime::inspect(&config).err()
    };
    let lifecycle = agent_lifecycle::status(AgentKind::Codex);
    let terminal_reason = cli_unavailable_reason(
        cli.binary_ready,
        cli.integration_ready,
        &lifecycle,
        agent_launch::terminal_available(),
    );
    let desktop_available = desktop_reason.is_none();
    let terminal_available = terminal_reason.is_none();
    LaunchStatus {
        preference: config.launch_preference,
        target: select_target(
            config.launch_preference,
            desktop_available,
            terminal_available,
        ),
        desktop_available,
        terminal_available,
        desktop_reason,
        terminal_reason,
        integrated,
    }
}

fn cli_unavailable_reason(
    binary: bool,
    integrated: bool,
    tracking: &crate::integrations::agent_lifecycle::LifecycleStatus,
    terminal: bool,
) -> Option<UnavailableReason> {
    use UnavailableReason::*;
    if !binary {
        Some(CliNotDetected)
    } else if !integrated {
        Some(CliIntegrationNotConfigured)
    } else if !tracking.supported {
        Some(UnsupportedPlatform)
    } else if !tracking.enabled {
        Some(CliTrackingDisabled)
    } else if tracking.outdated {
        Some(CliTrackingNeedsUpdate)
    } else if !tracking.installed {
        Some(CliTrackingNotConfigured)
    } else if !terminal {
        Some(TerminalUnavailable)
    } else {
        None
    }
}

/// Resolve once before dispatch. An existing desktop receipt pins retries to their original route.
pub fn resolve_launch(
    kind: crate::agents::AgentKind,
    id: Option<&str>,
    explicit: Option<LaunchTarget>,
) -> Result<LaunchTarget, String> {
    if kind != crate::agents::AgentKind::Codex {
        return Ok(LaunchTarget::Terminal);
    }
    if let Some(id) = id {
        uuid::Uuid::parse_str(id).map_err(|_| "Invalid operation ID")?;
        if crate::paths::state_dir()
            .join("codex-desktop-actions")
            .join(format!("{id}.json"))
            .exists()
        {
            return Ok(LaunchTarget::Desktop);
        }
    }
    if let Some(target) = explicit {
        return Ok(target);
    }
    let status = launch_status(true);
    status.target.ok_or_else(|| status.description(Lang::En))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum UnavailableReason {
    AppNotDetected,
    AppRuntimeMissing,
    AppDataUnavailable,
    CliNotDetected,
    CliTrackingDisabled,
    CliTrackingNeedsUpdate,
    CliTrackingNotConfigured,
    CliIntegrationNotConfigured,
    TerminalUnavailable,
    UnsupportedPlatform,
    IntegrationDisabled,
}
impl UnavailableReason {
    pub fn text(self, lang: Lang) -> &'static str {
        match (self, lang) {
            (Self::AppNotDetected, Lang::Zh) => "未检测到 Desktop App",
            (Self::AppNotDetected, Lang::En) => "Desktop App was not detected",
            (Self::AppRuntimeMissing, Lang::Zh) => "Desktop App 的内置运行程序不可用",
            (Self::AppRuntimeMissing, Lang::En) => "Desktop App’s bundled runtime is unavailable",
            (Self::AppDataUnavailable, Lang::Zh) => "Desktop App 的本地数据不可用",
            (Self::AppDataUnavailable, Lang::En) => "Desktop App’s local data is unavailable",
            (Self::CliNotDetected, Lang::Zh) => "未检测到 Codex CLI",
            (Self::CliNotDetected, Lang::En) => "Codex CLI was not detected",
            (Self::CliTrackingDisabled, Lang::Zh) => "CLI 的生命周期追踪未启用",
            (Self::CliTrackingDisabled, Lang::En) => "CLI lifecycle tracking is disabled",
            (Self::CliTrackingNeedsUpdate, Lang::Zh) => "CLI 的生命周期追踪需要更新",
            (Self::CliTrackingNeedsUpdate, Lang::En) => "CLI lifecycle tracking needs updating",
            (Self::CliTrackingNotConfigured, Lang::Zh) => "CLI 的生命周期追踪未配置",
            (Self::CliTrackingNotConfigured, Lang::En) => {
                "CLI lifecycle tracking is not configured"
            }
            (Self::CliIntegrationNotConfigured, Lang::Zh) => "CLI 的 AskHuman 集成未配置",
            (Self::CliIntegrationNotConfigured, Lang::En) => {
                "CLI integration with AskHuman is not configured"
            }
            (Self::TerminalUnavailable, Lang::Zh) => "系统终端不可用",
            (Self::TerminalUnavailable, Lang::En) => "The system terminal is unavailable",
            (Self::UnsupportedPlatform, Lang::Zh) => "当前平台不支持此运行方式",
            (Self::UnsupportedPlatform, Lang::En) => "This runtime is unsupported on this platform",
            (Self::IntegrationDisabled, Lang::Zh) => "Codex 集成未启用",
            (Self::IntegrationDisabled, Lang::En) => "Codex integration is disabled",
        }
    }
}
impl LaunchStatus {
    pub fn description(&self, lang: Lang) -> String {
        if let Some(target) = self.target {
            let result = match (target, lang) {
                (LaunchTarget::Desktop, Lang::Zh) => "新任务将使用 Desktop App",
                (LaunchTarget::Terminal, Lang::Zh) => "新任务将使用 CLI",
                (LaunchTarget::Desktop, Lang::En) => "New tasks will use Desktop App",
                (LaunchTarget::Terminal, Lang::En) => "New tasks will use CLI",
            };
            if target != self.preference {
                let reason = match self.preference {
                    LaunchTarget::Desktop => self.desktop_reason,
                    LaunchTarget::Terminal => self.terminal_reason,
                };
                if let Some(reason) = reason {
                    return match lang {
                        Lang::Zh => format!("{result}（{}）", reason.text(lang)),
                        Lang::En => format!("{result} ({})", reason.text(lang)),
                    };
                }
            }
            return result.into();
        }
        if self.desktop_reason == Some(UnavailableReason::AppNotDetected)
            && self.terminal_reason == Some(UnavailableReason::CliNotDetected)
        {
            return match lang {
                Lang::Zh => "暂时无法启动：未检测到 Desktop App 和 Codex CLI",
                Lang::En => "Unable to start: neither Desktop App nor Codex CLI was detected",
            }
            .into();
        }
        let mut reasons = Vec::new();
        for reason in [self.desktop_reason, self.terminal_reason]
            .into_iter()
            .flatten()
        {
            if !reasons.contains(&reason.text(lang)) {
                reasons.push(reason.text(lang));
            }
        }
        match lang {
            Lang::Zh => format!("暂时无法启动：{}", reasons.join("；")),
            Lang::En => format!("Unable to start: {}", reasons.join("; ")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::integrations::agent_lifecycle::LifecycleStatus;

    fn tracking() -> LifecycleStatus {
        LifecycleStatus {
            enabled: true,
            preference_configured: true,
            installed: true,
            outdated: false,
            supported: true,
            needs_update: false,
            cleanup_required: false,
        }
    }
    #[test]
    fn cli_reasons_distinguish_installation_tracking_and_terminal() {
        let mut state = tracking();
        assert_eq!(cli_unavailable_reason(true, true, &state, true), None);
        assert_eq!(
            cli_unavailable_reason(false, true, &state, true),
            Some(UnavailableReason::CliNotDetected)
        );
        assert_eq!(
            cli_unavailable_reason(true, false, &state, true),
            Some(UnavailableReason::CliIntegrationNotConfigured)
        );
        state.enabled = false;
        assert_eq!(
            cli_unavailable_reason(true, true, &state, true),
            Some(UnavailableReason::CliTrackingDisabled)
        );
        state.enabled = true;
        state.installed = false;
        assert_eq!(
            cli_unavailable_reason(true, true, &state, true),
            Some(UnavailableReason::CliTrackingNotConfigured)
        );
        state.installed = true;
        state.outdated = true;
        assert_eq!(
            cli_unavailable_reason(true, true, &state, true),
            Some(UnavailableReason::CliTrackingNeedsUpdate)
        );
        state.outdated = false;
        assert_eq!(
            cli_unavailable_reason(true, true, &state, false),
            Some(UnavailableReason::TerminalUnavailable)
        );
    }
    #[test]
    fn fallback_description_names_the_actual_missing_requirement() {
        let mut status = LaunchStatus {
            preference: LaunchTarget::Desktop,
            target: Some(LaunchTarget::Terminal),
            desktop_available: false,
            terminal_available: true,
            integrated: true,
            desktop_reason: Some(UnavailableReason::AppNotDetected),
            terminal_reason: None,
        };
        assert_eq!(
            status.description(Lang::Zh),
            "新任务将使用 CLI（未检测到 Desktop App）"
        );
        status.target = None;
        status.terminal_reason = Some(UnavailableReason::CliNotDetected);
        assert_eq!(
            status.description(Lang::Zh),
            "暂时无法启动：未检测到 Desktop App 和 Codex CLI"
        );
        status.target = Some(LaunchTarget::Desktop);
        status.desktop_reason = None;
        assert_eq!(status.description(Lang::Zh), "新任务将使用 Desktop App");
    }
}
