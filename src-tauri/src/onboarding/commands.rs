//! Tauri commands of the setup wizard window, its Settings entries and the popup strip
//! (`docs/plans/onboarding.md` §3.1).
//!
//! Anything that reads the disk, the login shell, the keychain or the daemon runs on a blocking
//! thread, so a window never waits on it while painting.

use crate::config::{AppConfig, MenuBarIconMode};
use crate::integrations::login_item;
use crate::onboarding::apply::{self, ApplyResult, Selection};
use crate::onboarding::channels::{self, ChannelStatus};
use crate::onboarding::detect::{self, AgentScanRow};
use crate::onboarding::env::{self, EnvReport};
use crate::onboarding::hint::{self, SetupHint};
use crate::onboarding::probe_run;
use crate::onboarding::state::{self, Decided};
use serde::Serialize;
use tauri::AppHandle;

async fn blocking<T, F>(f: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, String> + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|e| format!("setup task failed: {e}"))?
}

/// Which questions are settled and what this machine can offer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SetupStatus {
    pub agents_decided: bool,
    pub remote_decided: bool,
    pub background_decided: bool,
    pub login_item_installed: bool,
    /// The platform has a login item this instance may manage (a Dev Instance has none).
    pub login_item_available: bool,
    pub tray_available: bool,
    /// A Dev Instance must not change the everyday Agent integration.
    pub everyday_integrations_read_only: bool,
}

/// Pure assembly of [`SetupStatus`]; the inputs are what the command reads from the machine.
pub fn build_status(
    decided: Decided,
    login_item_installed: bool,
    tray_available: bool,
    dev_instance_context: bool,
    everyday_integrations_read_only: bool,
) -> SetupStatus {
    SetupStatus {
        agents_decided: decided.agents,
        remote_decided: decided.remote,
        background_decided: decided.background,
        login_item_installed,
        login_item_available: tray_available && !dev_instance_context,
        tray_available,
        everyday_integrations_read_only,
    }
}

#[tauri::command]
pub async fn setup_status() -> Result<SetupStatus, String> {
    blocking(|| {
        Ok(build_status(
            state::decided(),
            login_item::instance_has_login_item(),
            crate::app::gui_host::tray_supported(),
            crate::dev_instance::is_dev_instance_context(),
            // Same decision as `get_settings`: when it cannot be made, the write path refuses too.
            crate::integrations::mutation_lock::everyday_changes_blocked().unwrap_or(true),
        ))
    })
    .await
}

/// Every Agent available on this platform: what is installed, what is integrated, what applying
/// would change.
#[tauri::command]
pub async fn setup_agents_scan() -> Result<Vec<AgentScanRow>, String> {
    blocking(|| Ok(detect::scan())).await
}

/// The silent environment check (`AskHuman` on PATH, a temporary install location, several builds).
#[tauri::command]
pub async fn setup_env_check() -> Result<EnvReport, String> {
    blocking(|| {
        let lang = crate::i18n::Lang::resolve(&AppConfig::load_without_secrets().general.language);
        Ok(env::check(lang))
    })
    .await
}

/// Apply the rows the user confirmed, one Agent at a time; a failure on one never stops the rest.
#[tauri::command]
pub async fn setup_apply_agents(
    app: AppHandle,
    selections: Vec<Selection>,
) -> Result<Vec<ApplyResult>, String> {
    let results = blocking(move || Ok(apply::apply(&selections))).await?;
    // The tray's integration-update row and the wizard's pending flag follow what was written.
    let _ = blocking(move || {
        crate::app::gui_host::refresh_integration_updates(&app);
        Ok(())
    })
    .await;
    Ok(results)
}

/// State of the four remote-answering channels. `connected` is `null` while no daemon runs: the
/// query never starts one.
#[tauri::command]
pub async fn setup_channel_status() -> Result<Vec<ChannelStatus>, String> {
    let connections = crate::client::request_status()
        .await
        .map(|status| status.im_connections);
    blocking(move || {
        Ok(channels::status(
            &AppConfig::load_without_secrets(),
            connections.as_deref(),
        ))
    })
    .await
}

/// The user passed on a step on purpose (`agents` | `remote`).
#[tauri::command]
pub async fn setup_skip(app: AppHandle, domain: String) -> Result<(), String> {
    blocking(move || {
        state::mark_skipped(&domain);
        Ok(())
    })
    .await?;
    crate::app::gui_host::refresh_setup_state(&app);
    Ok(())
}

/// The wizard's last page: record the background choice, install the login item when the
/// menu-bar icon stays on `always`, and settle an Agent step that is still open. It does not
/// touch `config.json`; the page saves its choices first.
#[tauri::command]
pub async fn setup_finish(app: AppHandle) -> Result<(), String> {
    blocking(|| {
        state::mark_background_decided();
        if AppConfig::load_without_secrets().general.menu_bar_icon == MenuBarIconMode::Always {
            // A refusal (no login item on this platform / instance) must not fail the wizard.
            let _ = login_item::ensure_installed();
        }
        if !state::agents_decided() {
            state::mark_skipped(state::DOMAIN_AGENTS);
        }
        Ok(())
    })
    .await?;
    crate::app::gui_host::refresh_setup_state(&app);
    Ok(())
}

/// Settings' "launch at login is not enabled yet" button.
#[tauri::command]
pub async fn setup_login_item_enable(app: AppHandle) -> Result<(), String> {
    blocking(|| {
        login_item::ensure_installed().map_err(|e| e.to_string())?;
        state::mark_background_decided();
        Ok(())
    })
    .await?;
    crate::app::gui_host::refresh_setup_state(&app);
    Ok(())
}

/// Start one of the optional checks: `popup`, `channel` (with the channel id) or `agent`. The
/// returned id comes back in the `setup-probe-result` event when the check ends.
#[tauri::command]
pub async fn setup_probe_start(
    app: AppHandle,
    kind: String,
    channel: Option<String>,
) -> Result<String, String> {
    blocking(move || probe_run::start(app, &kind, channel.as_deref())).await
}

/// Stop a running check (the person cancelled, or the wizard window closed).
#[tauri::command]
pub async fn setup_probe_cancel(probe_id: String) -> Result<(), String> {
    probe_run::cancel(&probe_id);
    Ok(())
}

/// Open (or focus) the wizard, optionally on a page (`agents` | `remote` | `done`). Popups call
/// it too, so it goes through the unified GUI Host like every other window.
#[tauri::command]
pub fn open_setup(app: AppHandle, page: Option<String>) -> Result<(), String> {
    crate::commands::route_open_window(
        app,
        crate::gui_host::WindowKind::Setup,
        false,
        page,
        None,
        None,
    );
    Ok(())
}

/// The popup footer's pointer to an unfinished wizard page, or `null`.
#[tauri::command]
pub async fn popup_setup_hint() -> Result<Option<SetupHint>, String> {
    blocking(|| Ok(hint::next_hint())).await
}

/// "Don't remind me" / following the pointer: either way the question counts as answered.
#[tauri::command]
pub async fn popup_setup_hint_dismiss(app: AppHandle, id: String) -> Result<(), String> {
    blocking(move || {
        // Only ids the table knows can record a decision.
        if let Some(domain) = hint::domain_of(&id) {
            state::mark_skipped(domain);
        }
        Ok(())
    })
    .await?;
    // A popup is another process without a Host state: this only reaches hosted windows when the
    // call comes from one of them (the others refresh on focus).
    crate::app::gui_host::refresh_setup_state(&app);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decided(agents: bool, remote: bool, background: bool) -> Decided {
        Decided {
            agents,
            remote,
            background,
        }
    }

    #[test]
    fn the_status_carries_the_three_decisions_and_the_machine_facts() {
        let status = build_status(decided(true, false, true), true, true, false, false);
        assert_eq!(
            status,
            SetupStatus {
                agents_decided: true,
                remote_decided: false,
                background_decided: true,
                login_item_installed: true,
                login_item_available: true,
                tray_available: true,
                everyday_integrations_read_only: false,
            }
        );
    }

    #[test]
    fn a_dev_instance_has_no_login_item_to_offer() {
        let status = build_status(decided(false, false, false), false, true, true, true);
        assert!(status.tray_available);
        assert!(!status.login_item_available);
        assert!(status.everyday_integrations_read_only);
    }

    #[test]
    fn no_tray_means_no_login_item_either() {
        let status = build_status(decided(false, false, false), false, false, false, false);
        assert!(!status.tray_available);
        assert!(!status.login_item_available);
    }

    #[test]
    fn the_status_serializes_in_camel_case() {
        let value = serde_json::to_value(build_status(
            decided(false, false, false),
            false,
            true,
            false,
            false,
        ))
        .unwrap();
        for key in [
            "agentsDecided",
            "remoteDecided",
            "backgroundDecided",
            "loginItemInstalled",
            "loginItemAvailable",
            "trayAvailable",
            "everydayIntegrationsReadOnly",
        ] {
            assert!(value.get(key).is_some(), "missing {key}");
        }
    }
}
