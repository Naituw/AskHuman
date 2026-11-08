//! Optional macOS Cursor Desktop background-completion control, owned by the daemon.

mod create;
mod launch;
#[cfg(target_os = "macos")]
mod runtime;

pub use create::{execute, plan, Operation, PlanError};
pub use launch::{launch_status, resolve_launch, LaunchStatus};

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// The same choice of route as Codex's, so IM and iOS carry one kind of target for every Agent.
pub use crate::codex_desktop::LaunchTarget;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Config {
    pub suppress_background_completion_follow_up: bool,
    /// Which route a new Cursor task tries first; the other one is the fallback, as for Codex.
    /// Unlike Codex's, the default is the terminal: the Agents-window route is experimental, so
    /// it goes first only once the user has chosen it.
    pub launch_preference: LaunchTarget,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            suppress_background_completion_follow_up: false,
            launch_preference: LaunchTarget::Terminal,
        }
    }
}

/// One recent skipped unload. Anonymous by construction: no chat id and no content.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct RecentSkip {
    pub at: u64,
    pub bubbles: u64,
    /// `all` when a running chat's whole loaded history was kept, otherwise `partial`.
    pub scope: String,
}

/// Running-chat protection of one window, reported apart from the completion guard's fields.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct RetentionStatus {
    /// `active`, `notApplicable` (IDE windows), `unavailable`, or `pending`.
    pub state: String,
    pub error: Option<String>,
    pub checks: u32,
    pub skipped: u64,
    pub skipped_bubbles: u64,
    pub passed: u64,
    pub last_skipped_at: u64,
    pub recent: Vec<RecentSkip>,
    /// Running chats whose last message of yours is already outside the loaded history; `None`
    /// when it could not be measured. Purely diagnostic: it never affects the guard.
    pub at_risk: Option<u32>,
    pub at_risk_at: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct StallStatus {
    pub state: String,
    pub error: Option<String>,
    pub timeout_ms: Option<i64>,
    pub retry_window_ms: Option<u64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct WindowStatus {
    pub pid: u32,
    pub id: u32,
    /// The phase, `verified` and `checks` below describe the completion guard only.
    pub phase: String,
    pub surface: Option<String>,
    pub suppressed: u64,
    pub verified: bool,
    pub checks: u32,
    pub error: Option<String>,
    pub retention: Option<RetentionStatus>,
    pub stall: Option<StallStatus>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GuardState {
    Active,
    NotApplicable,
    Failed,
    Pending,
}

fn completion_state(window: &WindowStatus) -> GuardState {
    if window.phase == "active" && window.verified && window.checks >= 6 {
        GuardState::Active
    } else if window.phase == "error" {
        GuardState::Failed
    } else {
        GuardState::Pending
    }
}

fn retention_state(window: &WindowStatus) -> GuardState {
    match window.retention.as_ref() {
        Some(retention) if retention.state == "active" && retention.checks >= 6 => {
            GuardState::Active
        }
        // Only an IDE window may be exempt; the surface is known once the completion guard saw it.
        Some(retention) if retention.state == "notApplicable" => {
            if matches!(window.surface.as_deref(), None | Some("ide")) {
                GuardState::NotApplicable
            } else {
                GuardState::Failed
            }
        }
        Some(retention) if retention.state == "unavailable" => GuardState::Failed,
        _ => GuardState::Pending,
    }
}

fn stall_state(window: &WindowStatus) -> GuardState {
    match window.stall.as_ref() {
        Some(status) if status.state == "active" => {
            if status.timeout_ms == Some(300_000) && status.retry_window_ms == Some(600_000) {
                GuardState::Active
            } else {
                GuardState::Failed
            }
        }
        Some(status) if status.state == "notApplicable" && status.timeout_ms.is_some() => {
            GuardState::NotApplicable
        }
        Some(status) if status.state == "unavailable" => GuardState::Failed,
        _ => GuardState::Pending,
    }
}

/// How completely one window is protected. The guards are judged independently, so one that
/// cannot attach never hides or disables the other.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Coverage {
    /// Every guard that applies to the window is verified.
    Full,
    /// A guard is verified and another applicable guard is unavailable.
    Partial,
    /// No applicable guard is working.
    Failed,
    /// Still attaching or verifying.
    Attaching,
}

pub(super) fn coverage(window: &WindowStatus) -> Coverage {
    use GuardState::*;
    let states = [
        completion_state(window),
        retention_state(window),
        stall_state(window),
    ];
    if states.contains(&Pending) {
        Coverage::Attaching
    } else if !states.contains(&Failed) {
        Coverage::Full
    } else if states.contains(&Active) {
        Coverage::Partial
    } else {
        Coverage::Failed
    }
}

/// The overall `(active, phase)` of one reconcile pass. `attached` means every Cursor process
/// answered and has at least one window; `error` is a process-level failure.
pub(super) fn overall(
    attached: bool,
    error: bool,
    windows: &[WindowStatus],
) -> (bool, &'static str) {
    let count = |wanted| windows.iter().filter(|w| coverage(w) == wanted).count();
    if error || count(Coverage::Failed) > 0 {
        (false, "error")
    } else if windows.is_empty() {
        (false, "waiting")
    } else if attached && count(Coverage::Full) == windows.len() {
        (true, "active")
    } else if !attached || count(Coverage::Attaching) > 0 {
        (false, "attaching")
    } else {
        (false, "partial")
    }
}

/// A saved `active` claim must be backed by its own windows.
fn windows_fully_covered(windows: &[WindowStatus]) -> bool {
    !windows.is_empty() && windows.iter().all(|w| coverage(w) == Coverage::Full)
}

/// A saved `partial` claim needs at least one partly protected window and none that is worse.
fn windows_partly_covered(windows: &[WindowStatus]) -> bool {
    windows.iter().any(|w| coverage(w) == Coverage::Partial)
        && windows
            .iter()
            .all(|w| matches!(coverage(w), Coverage::Full | Coverage::Partial))
}

/// One diagnostic line for the running-chat protection; `None` while no window reports it.
/// Counts only: no chat ids and no content.
pub fn retention_summary(windows: &[WindowStatus]) -> Option<String> {
    let reported: Vec<&RetentionStatus> = windows
        .iter()
        .filter_map(|w| w.retention.as_ref())
        .collect();
    if reported.is_empty() {
        return None;
    }
    let count = |state: &str| reported.iter().filter(|r| r.state == state).count();
    let active: Vec<_> = reported.iter().filter(|r| r.state == "active").collect();
    let at_risk = if active.is_empty() || active.iter().any(|r| r.at_risk.is_none()) {
        "unknown".to_string()
    } else {
        active
            .iter()
            .filter_map(|r| r.at_risk)
            .fold(0_u32, |sum, n| sum.saturating_add(n))
            .to_string()
    };
    Some(format!(
        "{} active, {} not applicable, {} unavailable, {} pending; unloads skipped {}; running chats at risk {}",
        count("active"),
        count("notApplicable"),
        count("unavailable"),
        reported.len() - count("active") - count("notApplicable") - count("unavailable"),
        reported.iter().map(|r| r.skipped).sum::<u64>(),
        at_risk,
    ))
}

pub fn stall_summary(windows: &[WindowStatus]) -> Option<String> {
    let reported: Vec<_> = windows.iter().filter_map(|w| w.stall.as_ref()).collect();
    if reported.is_empty() {
        return None;
    }
    let count = |state: GuardState| windows.iter().filter(|w| stall_state(w) == state).count();
    let mut native: Vec<_> = reported
        .iter()
        .filter(|status| status.state == "notApplicable")
        .filter_map(|status| status.timeout_ms)
        .map(|ms| ms.max(0))
        .collect();
    native.sort_unstable();
    native.dedup();
    let suffix = if native.is_empty() {
        String::new()
    } else {
        format!("; native timeout values in ms {native:?} (0 means disabled)")
    };
    Some(format!(
        "{} active (300s stall, 600s retry window), {} native, {} unavailable, {} pending{suffix}",
        count(GuardState::Active),
        count(GuardState::NotApplicable),
        count(GuardState::Failed),
        count(GuardState::Pending),
    ))
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Status {
    pub supported: bool,
    pub preference_enabled: bool,
    pub enabled: bool,
    pub active: bool,
    pub phase: String,
    pub processes: usize,
    pub cursor_versions: Vec<String>,
    pub adapter_version: String,
    pub adapter_revision: String,
    pub windows: Vec<WindowStatus>,
    pub error: Option<String>,
    pub updated_at: u64,
    pub daemon_pid: u32,
}

pub fn configured(config: &Config) -> bool {
    cfg!(target_os = "macos")
        && !crate::dev_instance::is_dev_instance_context()
        && config.suppress_background_completion_follow_up
}

pub fn active(config: &Config) -> bool {
    configured(config)
        && crate::integrations::agent_mode::current(
            crate::integrations::agent_rules::AgentTarget::Cursor,
        ) != crate::integrations::agent_mode::Mode::None
}

/// Native idle control is separate from the optional background completion preference.
fn messages_enabled() -> bool {
    cfg!(target_os = "macos")
        && !crate::dev_instance::is_dev_instance_context()
        && crate::integrations::agent_mode::current(
            crate::integrations::agent_rules::AgentTarget::Cursor,
        ) != crate::integrations::agent_mode::Mode::None
        && {
            let lifecycle =
                crate::integrations::agent_lifecycle::status(crate::agents::AgentKind::Cursor);
            lifecycle.enabled && lifecycle.installed
        }
}

pub fn messages_available(session_id: &str) -> bool {
    messages_enabled() && crate::agents::cursor_vscdb::session_metadata(session_id).is_some()
}

pub enum MessageResult {
    Started(String),
    Rejected(String),
    Unknown,
}

pub async fn send_message(session_id: &str, id: &str, text: &str) -> MessageResult {
    if !messages_available(session_id) {
        return MessageResult::Rejected("nativeUnavailable".into());
    }
    #[cfg(target_os = "macos")]
    return runtime::send_message(session_id, id, text).await;
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (session_id, id, text);
        MessageResult::Rejected("unsupported".into())
    }
}

fn messages_need_cleanup() -> bool {
    #[cfg(target_os = "macos")]
    return !messages_enabled() && runtime::has_message_bridges();
    #[cfg(not(target_os = "macos"))]
    false
}

pub fn needs_cleanup() -> bool {
    #[cfg(target_os = "macos")]
    return runtime::has_managed_bridges();
    #[cfg(not(target_os = "macos"))]
    false
}

fn status_path() -> PathBuf {
    crate::paths::state_dir().join("cursor-desktop/status.json")
}

fn now_ms() -> u64 {
    chrono::Utc::now().timestamp_millis().max(0) as u64
}

fn daemon_alive(pid: u32) -> bool {
    #[cfg(unix)]
    return pid > 0 && pid <= i32::MAX as u32 && unsafe { libc::kill(pid as i32, 0) } == 0;
    #[cfg(not(unix))]
    {
        let _ = pid;
        false
    }
}

/// Read the daemon's bounded heartbeat without attaching to Cursor from a GUI/CLI process.
pub fn status(config: &Config) -> Status {
    let supported = cfg!(target_os = "macos");
    let enabled = active(config);
    let initial = Status {
        supported,
        preference_enabled: config.suppress_background_completion_follow_up,
        adapter_version: env!("CARGO_PKG_VERSION").into(),
        adapter_revision: {
            #[cfg(target_os = "macos")]
            {
                runtime::adapter_revision()
            }
            #[cfg(not(target_os = "macos"))]
            {
                String::new()
            }
        },
        enabled,
        phase: if !supported {
            "unsupported"
        } else if !enabled && needs_cleanup() {
            "detaching"
        } else if !enabled {
            "disabled"
        } else {
            "attaching"
        }
        .into(),
        ..Status::default()
    };
    let saved = std::fs::metadata(status_path())
        .ok()
        .filter(|metadata| metadata.len() <= 128 * 1024)
        .and_then(|_| std::fs::read(status_path()).ok())
        .and_then(|bytes| serde_json::from_slice::<Status>(&bytes).ok());
    if (enabled || needs_cleanup())
        && saved
            .as_ref()
            .is_some_and(|saved| !daemon_alive(saved.daemon_pid))
    {
        return Status {
            phase: "offline".into(),
            ..initial
        };
    }
    match saved {
        Some(mut saved)
            if supported
                && saved.adapter_revision == initial.adapter_revision
                && now_ms().saturating_sub(saved.updated_at) < 10_000 =>
        {
            saved.preference_enabled = config.suppress_background_completion_follow_up;
            if enabled && saved.enabled {
                // A saved claim must be backed by the saved windows themselves.
                if (saved.active && !windows_fully_covered(&saved.windows))
                    || (saved.phase == "partial" && !windows_partly_covered(&saved.windows))
                {
                    saved.active = false;
                    saved.phase = "attaching".into();
                }
                saved
            } else if !enabled && saved.enabled {
                Status {
                    phase: "detaching".into(),
                    ..initial
                }
            } else if !enabled && saved.error.is_some() {
                saved
            } else {
                initial
            }
        }
        _ => initial,
    }
}

/// Mode changes must also wake a stopped daemon so it can restore an existing bridge for None.
pub fn integration_changed() -> anyhow::Result<()> {
    if configured(&crate::config::AppConfig::load_without_secrets().cursor_desktop)
        || needs_cleanup()
        || messages_need_cleanup()
    {
        // Mode mutation is synchronous and may be called from a Tokio worker. Use the client's
        // serialized startup protocol on a separate thread rather than nesting a runtime.
        std::thread::spawn(|| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()?
                .block_on(async {
                    crate::client::ensure_running().await?;
                    let config = crate::config::AppConfig::load_without_secrets();
                    if !active(&config.cursor_desktop) {
                        // An integration uninstall must finish restoration before callers remove
                        // the binary or stop the daemon. This does not wait on Cursor model turns.
                        for _ in 0..60 {
                            if !needs_cleanup() && !messages_need_cleanup() { return Ok(()); }
                            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                        }
                        return Err(std::io::Error::other("Cursor restoration is still pending; check doctor before removing AskHuman"));
                    }
                    Ok(())
                })
        })
        .join()
        .map_err(|_| anyhow::anyhow!("Cursor adapter startup worker failed"))??;
    }
    Ok(())
}

pub fn start() {
    #[cfg(target_os = "macos")]
    tokio::spawn(runtime::run());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn retention(state: &str, checks: u32) -> Option<RetentionStatus> {
        Some(RetentionStatus {
            state: state.into(),
            checks,
            ..RetentionStatus::default()
        })
    }

    fn native_stall() -> Option<StallStatus> {
        Some(StallStatus {
            state: "notApplicable".into(),
            timeout_ms: Some(0),
            ..StallStatus::default()
        })
    }

    /// `completion`: `active` (verified), `error`, or `attaching`; `guard`: the retention state
    /// the window reports, or `None` when it reports nothing.
    fn window(completion: &str, surface: &str, guard: Option<(&str, u32)>) -> WindowStatus {
        WindowStatus {
            id: 1,
            phase: completion.into(),
            surface: Some(surface.into()),
            verified: completion == "active",
            checks: if completion == "active" { 6 } else { 0 },
            retention: guard.and_then(|(state, checks)| retention(state, checks)),
            stall: native_stall(),
            ..WindowStatus::default()
        }
    }

    #[test]
    fn unverified_or_partial_windows_cannot_report_active() {
        let mut window: WindowStatus = serde_json::from_str(r#"{"phase":"active"}"#).unwrap();
        window.retention = retention("active", 10);
        window.stall = native_stall();
        assert_ne!(
            coverage(&window),
            Coverage::Full,
            "completion is not verified"
        );
        window.verified = true;
        window.checks = 5;
        assert_ne!(
            coverage(&window),
            Coverage::Full,
            "too few completion checks"
        );
        window.checks = 6;
        assert_eq!(coverage(&window), Coverage::Full);
        window.retention = retention("active", 5);
        assert_ne!(
            coverage(&window),
            Coverage::Full,
            "too few retention checks"
        );
        window.retention = None;
        assert_ne!(
            coverage(&window),
            Coverage::Full,
            "retention was never reported"
        );
        window.retention = retention("active", 10);
        assert!(windows_fully_covered(std::slice::from_ref(&window)));
        assert!(!windows_fully_covered(&[window, WindowStatus::default()]));
        assert!(!windows_fully_covered(&[]));
    }

    #[test]
    fn each_guard_is_judged_on_its_own() {
        use Coverage::*;
        let table = [
            // (completion, retention state, expected)
            ("active", Some(("active", 10)), Full),
            ("active", Some(("notApplicable", 0)), Full),
            ("active", Some(("unavailable", 0)), Partial),
            ("active", Some(("pending", 0)), Attaching),
            ("active", None, Attaching),
            ("error", Some(("active", 10)), Partial),
            ("error", Some(("notApplicable", 0)), Failed),
            ("error", Some(("unavailable", 0)), Failed),
            ("error", Some(("pending", 0)), Attaching),
            ("error", None, Attaching),
            ("attaching", Some(("active", 10)), Attaching),
            ("attaching", Some(("unavailable", 0)), Attaching),
            ("attaching", Some(("notApplicable", 0)), Attaching),
            ("attaching", None, Attaching),
        ];
        for (completion, guard, expected) in table {
            assert_eq!(
                coverage(&window(completion, "agents", guard)),
                // An Agents window is never exempt, so `notApplicable` there means a broken guard.
                match (guard, expected) {
                    (Some(("notApplicable", _)), Full) => Partial,
                    (Some(("notApplicable", _)), Failed) => Failed,
                    (_, expected) => expected,
                },
                "agents window: completion {completion}, retention {guard:?}"
            );
            if matches!(guard, Some(("notApplicable", _))) {
                assert_eq!(
                    coverage(&window(completion, "ide", guard)),
                    expected,
                    "ide window: completion {completion}"
                );
            }
        }
    }

    #[test]
    fn stall_guard_requires_both_verified_timers() {
        let mut window = window("active", "ide", Some(("notApplicable", 0)));
        window.stall = None;
        assert_eq!(coverage(&window), Coverage::Attaching);
        window.stall = Some(StallStatus {
            state: "active".into(),
            timeout_ms: Some(300_000),
            retry_window_ms: Some(180_000),
            error: None,
        });
        assert_eq!(coverage(&window), Coverage::Partial);
        window.stall.as_mut().unwrap().retry_window_ms = Some(600_000);
        assert_eq!(coverage(&window), Coverage::Full);
    }

    #[test]
    fn only_an_ide_window_is_exempt_from_the_running_chat_protection() {
        let agents = window("active", "agents", Some(("notApplicable", 0)));
        assert_eq!(coverage(&agents), Coverage::Partial);
        let ide = window("active", "ide", Some(("notApplicable", 0)));
        assert_eq!(coverage(&ide), Coverage::Full);
        // Before the completion guard has seen the surface, the retention guard's own call stands.
        let unknown = WindowStatus {
            surface: None,
            ..window("error", "ide", Some(("notApplicable", 0)))
        };
        assert_eq!(coverage(&unknown), Coverage::Failed);
    }

    #[test]
    fn overall_phase_follows_the_windows_and_never_overclaims() {
        let full = window("active", "agents", Some(("active", 10)));
        let partial = window("active", "agents", Some(("unavailable", 0)));
        let attaching = window("attaching", "agents", None);
        let failed = window("error", "ide", Some(("notApplicable", 0)));
        let owned = |windows: &[&WindowStatus]| -> Vec<WindowStatus> {
            windows.iter().map(|w| (*w).clone()).collect()
        };

        assert_eq!(overall(false, false, &[]), (false, "waiting"));
        assert_eq!(overall(true, false, &[]), (false, "waiting"));
        assert_eq!(overall(false, true, &[]), (false, "error"));
        assert_eq!(overall(true, false, &owned(&[&full])), (true, "active"));
        assert_eq!(
            overall(true, false, &owned(&[&full, &full])),
            (true, "active")
        );
        assert_eq!(
            overall(true, false, &owned(&[&full, &partial])),
            (false, "partial")
        );
        assert_eq!(
            overall(true, false, &owned(&[&partial])),
            (false, "partial")
        );
        assert_eq!(
            overall(true, false, &owned(&[&partial, &attaching])),
            (false, "attaching")
        );
        assert_eq!(
            overall(true, false, &owned(&[&full, &attaching])),
            (false, "attaching")
        );
        assert_eq!(
            overall(true, false, &owned(&[&full, &failed])),
            (false, "error")
        );
        assert_eq!(
            overall(true, false, &owned(&[&partial, &failed])),
            (false, "error")
        );
        // A Cursor process that has not produced its windows yet keeps the claim at attaching.
        assert_eq!(
            overall(false, false, &owned(&[&full])),
            (false, "attaching")
        );
        assert_eq!(
            overall(false, false, &owned(&[&partial])),
            (false, "attaching")
        );
        // A process-level failure always wins, even when the windows look healthy.
        assert_eq!(overall(true, true, &owned(&[&full])), (false, "error"));
    }

    #[test]
    fn saved_claims_need_matching_windows() {
        let full = window("active", "agents", Some(("active", 10)));
        let partial = window("active", "agents", Some(("unavailable", 0)));
        assert!(windows_fully_covered(std::slice::from_ref(&full)));
        assert!(!windows_fully_covered(&[full.clone(), partial.clone()]));
        assert!(windows_partly_covered(&[full.clone(), partial.clone()]));
        assert!(!windows_partly_covered(std::slice::from_ref(&full)));
        assert!(!windows_partly_covered(&[]));
        let failed = window("error", "ide", Some(("notApplicable", 0)));
        assert!(!windows_partly_covered(&[partial, failed]));
    }

    #[test]
    fn a_status_saved_before_the_protection_existed_still_loads() {
        let old = r#"{"supported":true,"enabled":true,"active":true,"phase":"active",
            "windows":[{"pid":1,"id":2,"phase":"active","surface":"agents","suppressed":3,
            "verified":true,"checks":6}]}"#;
        let status: Status = serde_json::from_str(old).unwrap();
        assert!(status.windows[0].retention.is_none());
        // Without a retention report the window cannot be fully covered.
        assert!(!windows_fully_covered(&status.windows));
    }

    #[test]
    fn retention_status_uses_camel_case_and_tolerates_missing_fields() {
        let reported: WindowStatus = serde_json::from_str(
            r#"{"id":1,"phase":"active","verified":true,"checks":6,"retention":{
                "state":"active","checks":10,"skipped":2,"skippedBubbles":12,"passed":4,
                "lastSkippedAt":1790000000000,"atRisk":null,"atRiskAt":0,
                "recent":[{"at":1790000000000,"bubbles":6,"scope":"all"}]}}"#,
        )
        .unwrap();
        let retention = reported.retention.unwrap();
        assert_eq!(retention.skipped_bubbles, 12);
        assert_eq!(retention.last_skipped_at, 1_790_000_000_000);
        assert_eq!(retention.at_risk, None);
        assert_eq!(retention.recent[0].scope, "all");
        let minimal: WindowStatus =
            serde_json::from_str(r#"{"retention":{"state":"pending"}}"#).unwrap();
        assert_eq!(minimal.retention.unwrap().recent.len(), 0);
        let value = serde_json::to_value(RetentionStatus::default()).unwrap();
        for key in [
            "skippedBubbles",
            "lastSkippedAt",
            "atRisk",
            "atRiskAt",
            "recent",
        ] {
            assert!(value.get(key).is_some(), "{key} must be serialized");
        }
    }

    #[test]
    fn retention_summary_counts_only_and_marks_unknown_risk() {
        assert_eq!(retention_summary(&[]), None);
        assert_eq!(retention_summary(&[WindowStatus::default()]), None);
        let mut one = window("active", "agents", Some(("active", 10)));
        one.retention.as_mut().unwrap().skipped = 3;
        one.retention.as_mut().unwrap().at_risk = Some(2);
        let mut two = window("active", "agents", Some(("active", 10)));
        two.retention.as_mut().unwrap().skipped = 4;
        two.retention.as_mut().unwrap().at_risk = Some(1);
        let ide = window("active", "ide", Some(("notApplicable", 0)));
        let broken = window("active", "agents", Some(("unavailable", 0)));
        assert_eq!(
            retention_summary(&[one.clone(), two.clone(), ide.clone(), broken]).unwrap(),
            "2 active, 1 not applicable, 1 unavailable, 0 pending; unloads skipped 7; running chats at risk 3"
        );
        two.retention.as_mut().unwrap().at_risk = None;
        assert!(retention_summary(&[one, two, ide])
            .unwrap()
            .ends_with("running chats at risk unknown"));
    }

    #[test]
    fn old_config_is_opt_out_and_serializes_one_preference() {
        let config: crate::config::AppConfig = serde_json::from_str("{}").unwrap();
        assert!(
            !config
                .cursor_desktop
                .suppress_background_completion_follow_up
        );
        let value = serde_json::to_value(config).unwrap();
        assert_eq!(
            value["cursorDesktop"]["suppressBackgroundCompletionFollowUp"],
            false
        );
    }

    #[test]
    fn disabled_status_never_claims_a_saved_attachment() {
        let status = status(&Config::default());
        assert!(!status.enabled);
        assert!(!status.active);
        assert!(status.windows.is_empty());
    }
}
