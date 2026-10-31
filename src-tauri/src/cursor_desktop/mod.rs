//! Optional macOS Cursor Desktop background-completion control, owned by the daemon.

#[cfg(target_os = "macos")]
mod runtime;

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Config {
    pub suppress_background_completion_follow_up: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct WindowStatus {
    pub pid: u32,
    pub id: u32,
    pub phase: String,
    pub surface: Option<String>,
    pub suppressed: u64,
    pub verified: bool,
    pub checks: u32,
    pub error: Option<String>,
}

pub(super) fn windows_verified(windows: &[WindowStatus]) -> bool {
    !windows.is_empty()
        && windows
            .iter()
            .all(|window| window.phase == "active" && window.verified && window.checks >= 6)
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
                if saved.active && !windows_verified(&saved.windows) {
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
                            if !needs_cleanup() { return Ok(()); }
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

    #[test]
    fn unverified_or_partial_windows_cannot_report_active() {
        let mut window: WindowStatus = serde_json::from_str(r#"{"phase":"active"}"#).unwrap();
        assert!(!windows_verified(std::slice::from_ref(&window)));
        window.verified = true;
        window.checks = 5;
        assert!(!windows_verified(std::slice::from_ref(&window)));
        window.checks = 6;
        assert!(windows_verified(std::slice::from_ref(&window)));
        assert!(!windows_verified(&[window, WindowStatus::default()]));
        assert!(!windows_verified(&[]));
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
