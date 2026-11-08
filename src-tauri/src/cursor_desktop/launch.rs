//! Where a new Cursor task runs: the CLI in a terminal, or Cursor Desktop's Agents window.
//!
//! Same shape as the Codex choice (`codex_desktop::launch`): a preference that only says which
//! route to try first, a cheap static check of what is available, and one resolved target per task
//! that is never changed afterwards. Whether an Agents window is open and knows the project can
//! only be found out by attaching to Cursor, so those conditions are checked when the task is
//! created and refuse it there; they never turn into a silent switch to the other route
//! (docs/specs/cursor-desktop-task-creation.md §3).

use super::LaunchTarget;
use crate::agents::AgentKind;
use crate::i18n::Lang;

/// Why a route cannot be used right now.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum UnavailableReason {
    /// The Desktop route only exists on macOS.
    UnsupportedPlatform,
    /// A Dev Instance never attaches to the user's Cursor.
    DevInstance,
    /// Cursor's integration with AskHuman is off.
    IntegrationDisabled,
    /// The hooks that tell AskHuman about the chat are not installed or are off.
    TrackingNotConfigured,
    /// No Cursor Desktop process is running.
    AppNotRunning,
    /// More than one Cursor Desktop process: there is no unambiguous one to attach to.
    MultipleApps,
    CliNotDetected,
    CliIntegrationNotConfigured,
    CliTrackingDisabled,
    CliTrackingNeedsUpdate,
    CliTrackingNotConfigured,
    TerminalUnavailable,
}

impl UnavailableReason {
    pub fn text(self, lang: Lang) -> &'static str {
        match (self, lang) {
            (Self::UnsupportedPlatform, Lang::Zh) => "当前平台不支持此运行方式",
            (Self::UnsupportedPlatform, Lang::En) => "This runtime is unsupported on this platform",
            (Self::DevInstance, Lang::Zh) => "Dev Instance 不使用 Cursor Desktop",
            (Self::DevInstance, Lang::En) => "A Dev Instance does not use Cursor Desktop",
            (Self::IntegrationDisabled, Lang::Zh) => "Cursor 集成未启用",
            (Self::IntegrationDisabled, Lang::En) => "Cursor integration is disabled",
            (Self::TrackingNotConfigured, Lang::Zh) => "Cursor 的生命周期追踪未配置",
            (Self::TrackingNotConfigured, Lang::En) => {
                "Cursor lifecycle tracking is not configured"
            }
            (Self::AppNotRunning, Lang::Zh) => "未检测到运行中的 Cursor",
            (Self::AppNotRunning, Lang::En) => "Cursor is not running",
            (Self::MultipleApps, Lang::Zh) => "检测到多个 Cursor 进程",
            (Self::MultipleApps, Lang::En) => "More than one Cursor process is running",
            (Self::CliNotDetected, Lang::Zh) => "未检测到 Cursor CLI",
            (Self::CliNotDetected, Lang::En) => "Cursor CLI was not detected",
            (Self::CliIntegrationNotConfigured, Lang::Zh) => "CLI 的 AskHuman 集成未配置",
            (Self::CliIntegrationNotConfigured, Lang::En) => {
                "CLI integration with AskHuman is not configured"
            }
            (Self::CliTrackingDisabled, Lang::Zh) => "CLI 的生命周期追踪未启用",
            (Self::CliTrackingDisabled, Lang::En) => "CLI lifecycle tracking is disabled",
            (Self::CliTrackingNeedsUpdate, Lang::Zh) => "CLI 的生命周期追踪需要更新",
            (Self::CliTrackingNeedsUpdate, Lang::En) => "CLI lifecycle tracking needs updating",
            (Self::CliTrackingNotConfigured, Lang::Zh) => "CLI 的生命周期追踪未配置",
            (Self::CliTrackingNotConfigured, Lang::En) => {
                "CLI lifecycle tracking is not configured"
            }
            (Self::TerminalUnavailable, Lang::Zh) => "系统终端不可用",
            (Self::TerminalUnavailable, Lang::En) => "The system terminal is unavailable",
        }
    }
}

/// What the cheap static check of the Desktop route looks at.
#[derive(Clone, Copy, Debug)]
pub(crate) struct DesktopFacts {
    pub supported: bool,
    pub dev_instance: bool,
    pub integrated: bool,
    pub tracking_enabled: bool,
    pub tracking_installed: bool,
    /// Original Cursor Desktop processes of this user; `None` when they could not be listed.
    pub processes: Option<usize>,
}

pub(crate) fn desktop_reason(facts: DesktopFacts) -> Option<UnavailableReason> {
    use UnavailableReason::*;
    if !facts.supported {
        Some(UnsupportedPlatform)
    } else if facts.dev_instance {
        Some(DevInstance)
    } else if !facts.integrated {
        Some(IntegrationDisabled)
    } else if !facts.tracking_enabled || !facts.tracking_installed {
        Some(TrackingNotConfigured)
    } else {
        match facts.processes {
            Some(1) => None,
            Some(0) | None => Some(AppNotRunning),
            Some(_) => Some(MultipleApps),
        }
    }
}

pub(crate) fn cli_reason(
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

/// The route a preference resolves to, given what is available. The preference only decides which
/// route goes first: when it cannot be used the other one is taken, exactly as for Codex
/// (`codex_desktop::select_target`), so a Cursor that can run a task one way is never reported as
/// unable to run it at all (docs/specs/cursor-desktop-task-creation.md §3).
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

#[derive(Clone, Debug, serde::Serialize)]
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

pub(crate) fn status_from(
    preference: LaunchTarget,
    desktop_reason: Option<UnavailableReason>,
    terminal_reason: Option<UnavailableReason>,
    integrated: bool,
) -> LaunchStatus {
    let desktop_available = desktop_reason.is_none();
    let terminal_available = terminal_reason.is_none();
    LaunchStatus {
        preference,
        target: select_target(preference, desktop_available, terminal_available),
        desktop_available,
        terminal_available,
        integrated,
        desktop_reason,
        terminal_reason,
    }
}

#[cfg(target_os = "macos")]
fn running_processes() -> Option<usize> {
    super::runtime::process_count_blocking().ok()
}

#[cfg(not(target_os = "macos"))]
fn running_processes() -> Option<usize> {
    None
}

/// What can be read without looking at the running processes. `processes` is a placeholder.
pub(crate) fn static_facts() -> DesktopFacts {
    use crate::integrations::{agent_lifecycle, agent_mode, agent_rules};
    let tracking = agent_lifecycle::status(AgentKind::Cursor);
    DesktopFacts {
        supported: cfg!(target_os = "macos"),
        dev_instance: crate::dev_instance::is_dev_instance_context(),
        integrated: agent_mode::current(agent_rules::AgentTarget::Cursor) != agent_mode::Mode::None,
        tracking_enabled: tracking.enabled,
        tracking_installed: tracking.installed,
        processes: Some(1),
    }
}

/// Reads the facts from this machine. Never attaches to Cursor.
pub fn launch_status(force: bool) -> LaunchStatus {
    use crate::integrations::{agent_launch, agent_lifecycle};
    let preference = crate::config::AppConfig::load_without_secrets()
        .cursor_desktop
        .launch_preference;
    let cli = if force {
        agent_launch::readiness_fresh(AgentKind::Cursor)
    } else {
        agent_launch::readiness(AgentKind::Cursor)
    };
    let integrated = cli.integration_mode != "none";
    let tracking = agent_lifecycle::status(AgentKind::Cursor);
    let facts = static_facts();
    // The processes are listed only when everything else already allows the route.
    let desktop = desktop_reason(facts).or_else(|| {
        desktop_reason(DesktopFacts {
            processes: running_processes(),
            ..facts
        })
    });
    let terminal = cli_reason(
        cli.binary_ready,
        cli.integration_ready,
        &tracking,
        agent_launch::terminal_available(),
    );
    status_from(preference, desktop, terminal, integrated)
}

/// Resolve once before dispatch. An existing receipt pins a retry to the route it started on.
pub fn resolve_launch(
    id: Option<&str>,
    explicit: Option<LaunchTarget>,
) -> Result<LaunchTarget, String> {
    if let Some(id) = id {
        uuid::Uuid::parse_str(id).map_err(|_| "Invalid operation ID")?;
        if super::create::receipt_path(id).exists() {
            return Ok(LaunchTarget::Desktop);
        }
    }
    if let Some(target) = explicit {
        return Ok(target);
    }
    let status = launch_status(true);
    status.target.ok_or_else(|| status.description(Lang::En))
}

impl LaunchStatus {
    pub fn description(&self, lang: Lang) -> String {
        if let Some(target) = self.target {
            let result = match (target, lang) {
                (LaunchTarget::Desktop, Lang::Zh) => "新任务将使用 Cursor 的 Agents 窗口",
                (LaunchTarget::Terminal, Lang::Zh) => "新任务将使用 CLI",
                (LaunchTarget::Desktop, Lang::En) => "New tasks will use Cursor's Agents window",
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
        // Neither route works: say why for both, the preferred one first.
        let obstacles = match self.preference {
            LaunchTarget::Desktop => [self.desktop_reason, self.terminal_reason],
            LaunchTarget::Terminal => [self.terminal_reason, self.desktop_reason],
        };
        let mut reasons = Vec::new();
        for reason in obstacles.into_iter().flatten() {
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

    fn facts() -> DesktopFacts {
        DesktopFacts {
            supported: true,
            dev_instance: false,
            integrated: true,
            tracking_enabled: true,
            tracking_installed: true,
            processes: Some(1),
        }
    }

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
    fn the_desktop_route_needs_exactly_one_running_cursor_and_the_integration() {
        use UnavailableReason::*;
        assert_eq!(desktop_reason(facts()), None);
        let with = |change: fn(&mut DesktopFacts)| {
            let mut value = facts();
            change(&mut value);
            desktop_reason(value)
        };
        assert_eq!(with(|f| f.supported = false), Some(UnsupportedPlatform));
        assert_eq!(with(|f| f.dev_instance = true), Some(DevInstance));
        assert_eq!(with(|f| f.integrated = false), Some(IntegrationDisabled));
        assert_eq!(
            with(|f| f.tracking_enabled = false),
            Some(TrackingNotConfigured)
        );
        assert_eq!(
            with(|f| f.tracking_installed = false),
            Some(TrackingNotConfigured)
        );
        assert_eq!(with(|f| f.processes = Some(0)), Some(AppNotRunning));
        assert_eq!(with(|f| f.processes = None), Some(AppNotRunning));
        assert_eq!(with(|f| f.processes = Some(2)), Some(MultipleApps));
        // The first problem is the one reported, in the order of the settings page.
        assert_eq!(
            with(|f| {
                f.supported = false;
                f.processes = Some(0);
            }),
            Some(UnsupportedPlatform)
        );
    }

    #[test]
    fn the_terminal_route_reports_the_first_problem() {
        use UnavailableReason::*;
        let ok = tracking();
        assert_eq!(cli_reason(true, true, &ok, true), None);
        assert_eq!(cli_reason(false, true, &ok, true), Some(CliNotDetected));
        assert_eq!(
            cli_reason(true, false, &ok, true),
            Some(CliIntegrationNotConfigured)
        );
        let mut off = tracking();
        off.enabled = false;
        assert_eq!(
            cli_reason(true, true, &off, true),
            Some(CliTrackingDisabled)
        );
        let mut old = tracking();
        old.outdated = true;
        assert_eq!(
            cli_reason(true, true, &old, true),
            Some(CliTrackingNeedsUpdate)
        );
        let mut missing = tracking();
        missing.installed = false;
        assert_eq!(
            cli_reason(true, true, &missing, true),
            Some(CliTrackingNotConfigured)
        );
        assert_eq!(
            cli_reason(true, true, &ok, false),
            Some(TerminalUnavailable)
        );
    }

    #[test]
    fn the_preference_only_decides_which_route_goes_first() {
        use LaunchTarget::*;
        assert_eq!(select_target(Terminal, true, true), Some(Terminal));
        assert_eq!(select_target(Desktop, true, true), Some(Desktop));
        assert_eq!(select_target(Desktop, true, false), Some(Desktop));
        assert_eq!(select_target(Desktop, false, true), Some(Terminal));
        // The fallback goes both ways: a preferred route that cannot be used gives way to the other.
        assert_eq!(select_target(Terminal, true, false), Some(Desktop));
        assert_eq!(select_target(Terminal, false, true), Some(Terminal));
        // Only when neither route can be used is there nowhere to run.
        assert_eq!(select_target(Terminal, false, false), None);
        assert_eq!(select_target(Desktop, false, false), None);
    }

    #[test]
    fn the_status_line_explains_a_fallback_and_a_dead_end() {
        use UnavailableReason::*;
        let fallback = status_from(LaunchTarget::Desktop, Some(AppNotRunning), None, true);
        assert_eq!(fallback.target, Some(LaunchTarget::Terminal));
        assert_eq!(
            fallback.description(Lang::Zh),
            "新任务将使用 CLI（未检测到运行中的 Cursor）"
        );
        assert_eq!(
            fallback.description(Lang::En),
            "New tasks will use CLI (Cursor is not running)"
        );
        let plain = status_from(LaunchTarget::Terminal, Some(AppNotRunning), None, true);
        assert_eq!(plain.description(Lang::En), "New tasks will use CLI");
        // A CLI that cannot be used gives way to the Agents window, and the line says why.
        let to_window = status_from(LaunchTarget::Terminal, None, Some(CliNotDetected), true);
        assert_eq!(to_window.target, Some(LaunchTarget::Desktop));
        assert_eq!(
            to_window.description(Lang::Zh),
            "新任务将使用 Cursor 的 Agents 窗口（未检测到 Cursor CLI）"
        );
        assert_eq!(
            to_window.description(Lang::En),
            "New tasks will use Cursor's Agents window (Cursor CLI was not detected)"
        );
        let direct = status_from(LaunchTarget::Desktop, None, None, true);
        assert_eq!(
            direct.description(Lang::En),
            "New tasks will use Cursor's Agents window"
        );
        let stuck = status_from(
            LaunchTarget::Desktop,
            Some(AppNotRunning),
            Some(CliNotDetected),
            true,
        );
        assert_eq!(stuck.target, None);
        assert_eq!(
            stuck.description(Lang::En),
            "Unable to start: Cursor is not running; Cursor CLI was not detected"
        );
        // Both routes are named whichever was preferred, the preferred one first.
        let terminal_first = status_from(
            LaunchTarget::Terminal,
            Some(AppNotRunning),
            Some(CliNotDetected),
            true,
        );
        assert_eq!(terminal_first.target, None);
        assert_eq!(
            terminal_first.description(Lang::En),
            "Unable to start: Cursor CLI was not detected; Cursor is not running"
        );
        assert_eq!(
            terminal_first.description(Lang::Zh),
            "暂时无法启动：未检测到 Cursor CLI；未检测到运行中的 Cursor"
        );
    }

    #[test]
    fn the_status_serializes_the_way_the_form_reads_it() {
        let status = status_from(
            LaunchTarget::Desktop,
            Some(UnavailableReason::MultipleApps),
            None,
            true,
        );
        let value = serde_json::to_value(&status).unwrap();
        assert_eq!(value["preference"], "desktop");
        assert_eq!(value["target"], "terminal");
        assert_eq!(value["desktopReason"], "multipleApps");
        assert_eq!(value["terminalReason"], serde_json::Value::Null);
        assert_eq!(value["desktopAvailable"], false);
        assert_eq!(value["terminalAvailable"], true);
    }
}
