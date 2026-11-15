//! Where a new Antigravity task runs: `agy` in a terminal, or the desktop App through a sidecar.
//!
//! Same shape as the Codex and Cursor choices (`codex_desktop::launch`, `cursor_desktop::launch`):
//! a preference that only says which route to try first, a cheap static check of what is
//! available, and one resolved target per task that is never changed afterwards. What cannot be
//! known without acting (whether the sidecar of a directory can be created and comes online) is
//! found out when the task is created and refuses it there; it never turns into a silent switch to
//! the other route (docs/specs/antigravity-deep-integration.md §12.1).

use super::{runtime, Config, LaunchTarget};
use crate::agents::AgentKind;
use crate::config::AgentTaskPermission;
use crate::i18n::Lang;
use crate::integrations::agent_mode::Mode;

/// Whether `agy` in a terminal can start a task. The launch adapter for it is not there yet, so
/// the route is reported unavailable rather than offered and refused later.
const TERMINAL_LAUNCH: bool = false;

/// Why a route cannot be used right now.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum UnavailableReason {
    /// The App route only exists on macOS.
    UnsupportedPlatform,
    /// A Dev Instance never works on the user's Antigravity.
    DevInstance,
    AppNotInstalled,
    /// AskHuman's plugin is not installed in Antigravity (the integration mode is none).
    IntegrationDisabled,
    /// The App's sandbox blocks the command-line mode, so the sidecar's tasks could not ask.
    McpModeRequired,
    /// The user switched the plugin off inside Antigravity.
    PluginDisabled,
    /// The user has not allowed AskHuman to manage Antigravity sidecars.
    SidecarsNotAllowed,
    /// New tasks are fixed to YOLO, which neither route can start yet.
    YoloUnsupported,
    CliNotDetected,
    CliIntegrationNotConfigured,
    CliTrackingDisabled,
    CliTrackingNeedsUpdate,
    CliTrackingNotConfigured,
    TerminalUnavailable,
    /// Starting `agy` in a terminal is not offered yet.
    CliLaunchNotAvailable,
}

impl UnavailableReason {
    pub fn text(self, lang: Lang) -> &'static str {
        match (self, lang) {
            (Self::UnsupportedPlatform, Lang::Zh) => "当前平台不支持此运行方式",
            (Self::UnsupportedPlatform, Lang::En) => "This runtime is unsupported on this platform",
            (Self::DevInstance, Lang::Zh) => "Dev Instance 不使用 Antigravity App",
            (Self::DevInstance, Lang::En) => "A Dev Instance does not use the Antigravity App",
            (Self::AppNotInstalled, Lang::Zh) => "未检测到 Antigravity App",
            (Self::AppNotInstalled, Lang::En) => "The Antigravity App was not detected",
            (Self::IntegrationDisabled, Lang::Zh) => "Antigravity 的 AskHuman 集成未启用",
            (Self::IntegrationDisabled, Lang::En) => {
                "Antigravity integration with AskHuman is disabled"
            }
            (Self::McpModeRequired, Lang::Zh) => {
                "App 路径需要 MCP 模式（App 的沙箱不允许命令行模式）"
            }
            (Self::McpModeRequired, Lang::En) => {
                "The App route needs MCP mode (the App's sandbox does not allow CLI mode)"
            }
            (Self::PluginDisabled, Lang::Zh) => "AskHuman 插件已在 Antigravity 中停用",
            (Self::PluginDisabled, Lang::En) => "The AskHuman plugin is turned off in Antigravity",
            (Self::SidecarsNotAllowed, Lang::Zh) => {
                "尚未允许 AskHuman 管理 Antigravity sidecar（请在设置中开启）"
            }
            (Self::SidecarsNotAllowed, Lang::En) => {
                "AskHuman is not allowed to manage Antigravity sidecars yet (turn it on in Settings)"
            }
            (Self::YoloUnsupported, Lang::Zh) => {
                "Antigravity 暂不支持 YOLO 启动：请把「权限选择」改成「每次询问」或「总是 Agent 默认」"
            }
            (Self::YoloUnsupported, Lang::En) => {
                "Antigravity cannot start tasks in YOLO mode yet: set the permission choice to Ask or Agent default"
            }
            (Self::CliNotDetected, Lang::Zh) => "未检测到 agy 命令行",
            (Self::CliNotDetected, Lang::En) => "The agy command was not detected",
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
            (Self::CliLaunchNotAvailable, Lang::Zh) => "暂未开放用终端（agy）启动任务",
            (Self::CliLaunchNotAvailable, Lang::En) => {
                "Starting tasks with agy in a terminal is not available yet"
            }
        }
    }
}

/// What the cheap static check of the App route looks at.
#[derive(Clone, Copy, Debug)]
pub(crate) struct DesktopFacts {
    pub supported: bool,
    pub dev_instance: bool,
    pub app_installed: bool,
    /// The integration mode of AskHuman's plugin.
    pub mode: Mode,
    /// The user turned the plugin off inside Antigravity.
    pub plugin_disabled: bool,
    pub manage_sidecars: bool,
    /// New tasks are fixed to YOLO.
    pub yolo: bool,
}

pub(crate) fn desktop_reason(facts: DesktopFacts) -> Option<UnavailableReason> {
    use UnavailableReason::*;
    if !facts.supported {
        Some(UnsupportedPlatform)
    } else if facts.dev_instance {
        Some(DevInstance)
    } else if !facts.app_installed {
        Some(AppNotInstalled)
    } else if facts.mode == Mode::None {
        Some(IntegrationDisabled)
    } else if facts.mode != Mode::Mcp {
        Some(McpModeRequired)
    } else if facts.plugin_disabled {
        Some(PluginDisabled)
    } else if !facts.manage_sidecars {
        Some(SidecarsNotAllowed)
    } else if facts.yolo {
        Some(YoloUnsupported)
    } else {
        None
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

/// Why the terminal route cannot be used: its own preconditions, then the rule that is still
/// missing for both routes. The route is closed as a whole until `agy` launches are implemented,
/// whatever the machine looks like.
fn terminal_reason(
    launchable: bool,
    yolo: bool,
    own: Option<UnavailableReason>,
) -> Option<UnavailableReason> {
    if !launchable {
        Some(UnavailableReason::CliLaunchNotAvailable)
    } else if own.is_some() {
        own
    } else if yolo {
        Some(UnavailableReason::YoloUnsupported)
    } else {
        None
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
        target: crate::codex_desktop::select_target(
            preference,
            desktop_available,
            terminal_available,
        ),
        desktop_available,
        terminal_available,
        integrated,
        desktop_reason,
        terminal_reason,
    }
}

/// What can be read without attaching to anything: the machine, the plugin, the settings.
pub(crate) fn static_facts(config: &Config, yolo: bool) -> DesktopFacts {
    use crate::integrations::{agent_mode, agent_rules, antigravity_plugin};
    DesktopFacts {
        supported: cfg!(target_os = "macos"),
        dev_instance: crate::dev_instance::is_dev_instance_context(),
        app_installed: runtime::app_installed(),
        mode: agent_mode::current(agent_rules::AgentTarget::Antigravity),
        plugin_disabled: antigravity_plugin::host_view().plugin_disabled,
        manage_sidecars: config.manage_sidecars,
        yolo,
    }
}

/// Reads the facts from this machine. Never starts the App and never touches a sidecar.
pub fn launch_status(force: bool) -> LaunchStatus {
    use crate::integrations::{agent_launch, agent_lifecycle};
    let app_config = crate::config::AppConfig::load_without_secrets();
    let yolo = app_config.agent_tasks.permission_prompt == AgentTaskPermission::Yolo;
    let config = app_config.antigravity_desktop;
    let cli = if force {
        agent_launch::readiness_fresh(AgentKind::Antigravity)
    } else {
        agent_launch::readiness(AgentKind::Antigravity)
    };
    let tracking = agent_lifecycle::status(AgentKind::Antigravity);
    let desktop = desktop_reason(static_facts(&config, yolo));
    let terminal = terminal_reason(
        TERMINAL_LAUNCH,
        yolo,
        cli_reason(
            cli.binary_ready,
            cli.integration_ready,
            &tracking,
            agent_launch::terminal_available(),
        ),
    );
    status_from(
        config.launch_preference,
        desktop,
        terminal,
        cli.integration_mode != "none",
    )
}

/// Resolve once before dispatch. An existing receipt pins a retry to the route it started on.
pub fn resolve_launch(
    id: Option<&str>,
    explicit: Option<LaunchTarget>,
) -> Result<LaunchTarget, String> {
    resolve_with(&super::receipt::dir(), id, explicit, || launch_status(true))
}

fn resolve_with(
    receipts: &std::path::Path,
    id: Option<&str>,
    explicit: Option<LaunchTarget>,
    status: impl FnOnce() -> LaunchStatus,
) -> Result<LaunchTarget, String> {
    if let Some(id) = id {
        uuid::Uuid::parse_str(id).map_err(|_| "Invalid operation ID")?;
        if receipts.join(format!("{id}.json")).exists() {
            return Ok(LaunchTarget::Desktop);
        }
    }
    if let Some(target) = explicit {
        return Ok(target);
    }
    let status = status();
    status.target.ok_or_else(|| status.description(Lang::En))
}

impl LaunchStatus {
    pub fn description(&self, lang: Lang) -> String {
        if let Some(target) = self.target {
            let result = match (target, lang) {
                (LaunchTarget::Desktop, Lang::Zh) => "新任务将使用 Antigravity App",
                (LaunchTarget::Terminal, Lang::Zh) => "新任务将使用 agy（终端）",
                (LaunchTarget::Desktop, Lang::En) => "New tasks will use the Antigravity App",
                (LaunchTarget::Terminal, Lang::En) => "New tasks will use agy in a terminal",
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
            app_installed: true,
            mode: Mode::Mcp,
            plugin_disabled: false,
            manage_sidecars: true,
            yolo: false,
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
    fn the_app_route_needs_the_app_the_mcp_plugin_and_the_users_permission() {
        use UnavailableReason::*;
        assert_eq!(desktop_reason(facts()), None);
        let with = |change: fn(&mut DesktopFacts)| {
            let mut value = facts();
            change(&mut value);
            desktop_reason(value)
        };
        assert_eq!(with(|f| f.supported = false), Some(UnsupportedPlatform));
        assert_eq!(with(|f| f.dev_instance = true), Some(DevInstance));
        assert_eq!(with(|f| f.app_installed = false), Some(AppNotInstalled));
        assert_eq!(with(|f| f.mode = Mode::None), Some(IntegrationDisabled));
        // The App's sandbox blocks the command-line mode, so only MCP mode can ask.
        assert_eq!(with(|f| f.mode = Mode::Cli), Some(McpModeRequired));
        assert_eq!(with(|f| f.plugin_disabled = true), Some(PluginDisabled));
        assert_eq!(
            with(|f| f.manage_sidecars = false),
            Some(SidecarsNotAllowed)
        );
        assert_eq!(with(|f| f.yolo = true), Some(YoloUnsupported));
        // The first problem is the one reported, in the order of the settings page.
        assert_eq!(
            with(|f| {
                f.app_installed = false;
                f.manage_sidecars = false;
                f.yolo = true;
            }),
            Some(AppNotInstalled)
        );
        assert_eq!(
            with(|f| {
                f.manage_sidecars = false;
                f.yolo = true;
            }),
            Some(SidecarsNotAllowed)
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
    fn the_terminal_route_stays_closed_until_agy_launches_exist() {
        use UnavailableReason::*;
        // Whatever the machine looks like, a route that cannot launch is not offered.
        assert_eq!(
            terminal_reason(false, false, None),
            Some(CliLaunchNotAvailable)
        );
        assert_eq!(
            terminal_reason(false, true, Some(CliNotDetected)),
            Some(CliLaunchNotAvailable)
        );
        // Once it can launch, its own problems come first and the YOLO rule after them.
        assert_eq!(
            terminal_reason(true, true, Some(CliNotDetected)),
            Some(CliNotDetected)
        );
        assert_eq!(terminal_reason(true, true, None), Some(YoloUnsupported));
        assert_eq!(terminal_reason(true, false, None), None);
    }

    #[test]
    fn the_preference_only_decides_which_route_goes_first() {
        use LaunchTarget::*;
        let status = |preference, desktop: bool, terminal: bool| {
            status_from(
                preference,
                (!desktop).then_some(UnavailableReason::AppNotInstalled),
                (!terminal).then_some(UnavailableReason::CliNotDetected),
                true,
            )
            .target
        };
        assert_eq!(status(Terminal, true, true), Some(Terminal));
        assert_eq!(status(Desktop, true, true), Some(Desktop));
        assert_eq!(status(Desktop, true, false), Some(Desktop));
        assert_eq!(status(Desktop, false, true), Some(Terminal));
        // The fallback goes both ways: a preferred route that cannot be used gives way to the other.
        assert_eq!(status(Terminal, true, false), Some(Desktop));
        assert_eq!(status(Terminal, false, true), Some(Terminal));
        // Only when neither route can be used is there nowhere to run.
        assert_eq!(status(Terminal, false, false), None);
        assert_eq!(status(Desktop, false, false), None);
    }

    #[test]
    fn the_status_line_explains_a_fallback_and_a_dead_end() {
        use UnavailableReason::*;
        let fallback = status_from(LaunchTarget::Desktop, Some(AppNotInstalled), None, true);
        assert_eq!(fallback.target, Some(LaunchTarget::Terminal));
        assert_eq!(
            fallback.description(Lang::Zh),
            "新任务将使用 agy（终端）（未检测到 Antigravity App）"
        );
        assert_eq!(
            fallback.description(Lang::En),
            "New tasks will use agy in a terminal (The Antigravity App was not detected)"
        );
        let plain = status_from(LaunchTarget::Terminal, Some(AppNotInstalled), None, true);
        assert_eq!(
            plain.description(Lang::En),
            "New tasks will use agy in a terminal"
        );
        // A CLI that cannot be used gives way to the App, and the line says why.
        let to_app = status_from(LaunchTarget::Terminal, None, Some(CliNotDetected), true);
        assert_eq!(to_app.target, Some(LaunchTarget::Desktop));
        assert_eq!(
            to_app.description(Lang::Zh),
            "新任务将使用 Antigravity App（未检测到 agy 命令行）"
        );
        assert_eq!(
            to_app.description(Lang::En),
            "New tasks will use the Antigravity App (The agy command was not detected)"
        );
        let direct = status_from(
            LaunchTarget::Desktop,
            None,
            Some(CliLaunchNotAvailable),
            true,
        );
        assert_eq!(
            direct.description(Lang::En),
            "New tasks will use the Antigravity App"
        );
        let stuck = status_from(
            LaunchTarget::Desktop,
            Some(SidecarsNotAllowed),
            Some(CliLaunchNotAvailable),
            true,
        );
        assert_eq!(stuck.target, None);
        assert_eq!(
            stuck.description(Lang::En),
            "Unable to start: AskHuman is not allowed to manage Antigravity sidecars yet (turn it on in Settings); Starting tasks with agy in a terminal is not available yet"
        );
        // Both routes are named whichever was preferred, the preferred one first; a reason that
        // both share is said once.
        let both = status_from(
            LaunchTarget::Terminal,
            Some(YoloUnsupported),
            Some(YoloUnsupported),
            true,
        );
        assert_eq!(both.target, None);
        assert_eq!(
            both.description(Lang::Zh),
            "暂时无法启动：Antigravity 暂不支持 YOLO 启动：请把「权限选择」改成「每次询问」或「总是 Agent 默认」"
        );
    }

    #[test]
    fn the_status_serializes_the_way_the_form_reads_it() {
        let status = status_from(
            LaunchTarget::Desktop,
            Some(UnavailableReason::SidecarsNotAllowed),
            None,
            true,
        );
        let value = serde_json::to_value(&status).unwrap();
        assert_eq!(value["preference"], "desktop");
        assert_eq!(value["target"], "terminal");
        assert_eq!(value["desktopReason"], "sidecarsNotAllowed");
        assert_eq!(value["terminalReason"], serde_json::Value::Null);
        assert_eq!(value["desktopAvailable"], false);
        assert_eq!(value["terminalAvailable"], true);
        assert_eq!(value["integrated"], true);
    }

    #[test]
    fn an_existing_receipt_pins_a_retry_to_the_app_and_a_bad_id_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let id = "0b9a4c2e-8f60-4d5e-9d57-3b1f6f0f7a11";
        let terminal = || {
            status_from(
                LaunchTarget::Terminal,
                Some(UnavailableReason::AppNotInstalled),
                None,
                true,
            )
        };
        let never = || -> LaunchStatus { panic!("the status must not be read") };
        assert!(resolve_with(dir.path(), Some("not-a-uuid"), None, never).is_err());
        // Without a receipt the machine decides.
        assert_eq!(
            resolve_with(dir.path(), Some(id), None, terminal),
            Ok(LaunchTarget::Terminal)
        );
        // With the receipt of a first attempt the retry stays on the App, even if the machine now
        // says Terminal and the caller asked for it.
        std::fs::write(dir.path().join(format!("{id}.json")), "{}").unwrap();
        assert_eq!(
            resolve_with(dir.path(), Some(id), Some(LaunchTarget::Terminal), never),
            Ok(LaunchTarget::Desktop)
        );
        assert_eq!(
            resolve_with(dir.path(), Some(id), None, never),
            Ok(LaunchTarget::Desktop)
        );
        // An explicit target needs no machine reads.
        assert_eq!(
            resolve_with(dir.path(), None, Some(LaunchTarget::Terminal), never),
            Ok(LaunchTarget::Terminal)
        );
    }

    #[test]
    fn nowhere_to_run_is_an_error_that_names_the_obstacles() {
        let dir = tempfile::tempdir().unwrap();
        let stuck = || {
            status_from(
                LaunchTarget::Desktop,
                Some(UnavailableReason::AppNotInstalled),
                Some(UnavailableReason::CliLaunchNotAvailable),
                true,
            )
        };
        assert_eq!(
            resolve_with(dir.path(), None, None, stuck),
            Err("Unable to start: The Antigravity App was not detected; Starting tasks with agy in a terminal is not available yet".into())
        );
    }
}
