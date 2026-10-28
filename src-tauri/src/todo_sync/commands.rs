//! Tauri commands of the iCloud todo sync: the settings card and the todos window's sidebar footer.
//!
//! Every command that touches the helper or the sync state runs on a blocking thread — the helper is
//! another process and a tick can take seconds. The window never waits for the network; it asks for
//! the status, shows what it has, and asks again after an action.

use super::service::{self, ProjectDetails, Status, SyncProject};
use serde::Serialize;

async fn blocking<T, F>(f: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, String> + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|e| format!("todo sync task failed: {e}"))?
}

/// The status the UI draws from; one call for the card and the sidebar footer alike.
#[tauri::command]
pub async fn todo_sync_status() -> Result<Status, String> {
    blocking(|| Ok(service::status())).await
}

/// How many finished todos of `project` the cloud holds (the number the "clear history" confirmation
/// quotes when sync is on); `None` when the project does not sync.
#[tauri::command]
pub async fn todo_sync_history_count(project: String) -> Result<Option<usize>, String> {
    blocking(move || Ok(service::cloud_history_count(&project))).await
}

/// The synced projects with their directories on this Mac and the Macs that work on them: what the
/// sidebar needs to merge directories of one repository and to list projects this Mac has no
/// directory for. Empty while sync is off.
#[tauri::command]
pub async fn todo_sync_projects() -> Result<Vec<SyncProject>, String> {
    blocking(|| Ok(service::projects())).await
}

/// Where the todos of `project` came from (and finished todos were finished), and which attachments
/// live on another Mac. Everything arrives after first paint; nothing here is needed to render.
#[tauri::command]
pub async fn todo_sync_project_details(project: String) -> Result<ProjectDetails, String> {
    blocking(move || Ok(service::project_details(&project))).await
}

/// "Remove from iCloud": delete every todo and finished todo of the project on every Mac. Returns
/// how many were removed.
#[tauri::command]
pub async fn todo_sync_remove_project(project: String) -> Result<usize, String> {
    blocking(move || service::remove_project(&project).map_err(|e| e.to_string())).await
}

/// Turn sync on for this Mac (the first sync runs in the background).
#[tauri::command]
pub async fn todo_sync_enable() -> Result<Status, String> {
    blocking(|| {
        service::enable().map_err(|e| e.to_string())?;
        Ok(service::status())
    })
    .await
}

/// "Stop sync, keep this Mac's copy".
#[tauri::command]
pub async fn todo_sync_disable() -> Result<Status, String> {
    blocking(|| {
        service::disable().map_err(|e| e.to_string())?;
        Ok(service::status())
    })
    .await
}

/// "Sync now": one complete tick, however long it takes. A soft outcome (offline, no helper,
/// paused) is reported through the status, not as an error.
#[tauri::command]
pub async fn todo_sync_now() -> Result<Status, String> {
    blocking(|| {
        service::run_tick("manual").map_err(|e| e.to_string())?;
        Ok(service::status())
    })
    .await
}

/// The window opened or got focus: pull if the last pull is stale. Returns at once.
#[tauri::command]
pub fn todo_sync_touch() {
    if service::is_enabled() {
        std::thread::spawn(service::refresh_if_stale);
    }
}

/// "Clear iCloud data": delete the cloud zone. Every Mac pauses until its user decides.
#[tauri::command]
pub async fn todo_sync_clear_cloud() -> Result<Status, String> {
    blocking(|| {
        service::clear_cloud().map_err(|e| e.to_string())?;
        Ok(service::status())
    })
    .await
}

/// After a pause: continue with this Mac's copy (it is uploaded again).
#[tauri::command]
pub async fn todo_sync_restart_from_local() -> Result<Status, String> {
    blocking(|| {
        service::restart_from_local().map_err(|e| e.to_string())?;
        let _ = service::run_tick("restart");
        Ok(service::status())
    })
    .await
}

/// After a quota pause: try again.
#[tauri::command]
pub async fn todo_sync_resume() -> Result<Status, String> {
    blocking(|| {
        service::resume().map_err(|e| e.to_string())?;
        let _ = service::run_tick("resume");
        Ok(service::status())
    })
    .await
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RepairOutcome {
    /// The helper is registered and was started.
    pub ok: bool,
    /// macOS wants the user to allow it under System Settings → Login Items.
    pub needs_approval: bool,
    pub message: String,
}

/// "Repair": register the helper app as a login item and start it.
#[tauri::command]
pub async fn todo_sync_repair() -> Result<RepairOutcome, String> {
    blocking(|| {
        let app = service::helper_app_path().ok_or_else(|| "helperNotInstalled".to_string())?;
        let exe = app.join("Contents/MacOS/AskHumanTodoSync");
        let out = std::process::Command::new(&exe)
            .arg("--register")
            .output()
            .map_err(|e| format!("cannot run the helper: {e}"))?;
        let code = out.status.code().unwrap_or(1);
        Ok(RepairOutcome {
            ok: code == 0,
            // The helper exits 3 when the registration waits for the user's approval.
            needs_approval: code == 3,
            message: String::from_utf8_lossy(&out.stdout).trim().to_string(),
        })
    })
    .await
}

/// Open System Settings at Login Items (where the user approves the helper).
#[tauri::command]
pub fn todo_sync_open_login_items() {
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("open")
            .arg("x-apple.systempreferences:com.apple.LoginItems-Settings.extension")
            .spawn();
    }
}
