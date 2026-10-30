//! Developer settings operate on explicit worktrees without changing the GUI host's home.

use super::{build, routing, session, state, verification};
use serde::Serialize;
use std::path::{Path, PathBuf};

pub const WINDOW_ENV: &str = "ASKHUMAN_DEV_TEST_WINDOW";

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorktreeInfo {
    pub root: PathBuf,
    pub name: String,
    pub binary: PathBuf,
    pub home: PathBuf,
    pub enabled: bool,
    pub build: Option<build::BuildInfo>,
    pub error: Option<String>,
    pub preferences: verification::Preferences,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub mode: state::RoutingMode,
    pub system_binary: Option<PathBuf>,
    pub worktrees: Vec<WorktreeInfo>,
    pub agents: Vec<session::Support>,
    pub instance_target: Option<routing::Target>,
}

pub fn status() -> Result<Status, String> {
    let state = state::read()?;
    let mut roots = state.worktrees.clone();
    if let Some(root) = std::env::current_dir()
        .ok()
        .and_then(|cwd| crate::dev_instance::find_dev_root(&cwd))
    {
        if !roots
            .iter()
            .any(|existing| state::same_path(existing, &root))
        {
            roots.push(root);
        }
    }
    let worktrees = roots
        .into_iter()
        .map(|root| {
            let enabled = root
                .join(crate::dev_instance::DEV_DIR)
                .join(crate::dev_instance::ENABLED_MARKER)
                .is_file();
            let result = build::verify(&root);
            let (build, error) = match result {
                Ok(build) => (Some(build), None),
                Err(error) => (None, Some(error)),
            };
            WorktreeInfo {
                name: root
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string(),
                binary: crate::dev_instance::instance_bin(&root),
                home: crate::dev_instance::instance_home(&root),
                enabled,
                preferences: verification::preferences(&root).unwrap_or_default(),
                root,
                build,
                error,
            }
        })
        .collect();
    let agents = [
        crate::agents::AgentKind::Codex,
        crate::agents::AgentKind::Claude,
        crate::agents::AgentKind::Grok,
        crate::agents::AgentKind::Pi,
        crate::agents::AgentKind::Cursor,
        crate::agents::AgentKind::Antigravity,
    ]
    .into_iter()
    .map(session::support)
    .collect();
    Ok(Status {
        mode: state.mode,
        system_binary: state.system_binary,
        worktrees,
        agents,
        instance_target: routing::inherited()?,
    })
}

pub fn register(root: &Path) -> Result<(), String> {
    let root = root.canonicalize().map_err(|e| e.to_string())?;
    if !root
        .join(crate::dev_instance::DEV_DIR)
        .join(crate::dev_instance::ENABLED_MARKER)
        .is_file()
    {
        return Err("enable this worktree's Dev Instance before adding it".into());
    }
    state::register_worktree(&root)
}

pub fn open_window(root: &Path, kind: &str) -> Result<(), String> {
    if !matches!(kind, "settings" | "history" | "todos" | "agents") {
        return Err("unknown development window".into());
    }
    build::verify(root)?;
    let target = routing::dev_target(root)?;
    let mut command = std::process::Command::new(&target.binary);
    routing::configure(&mut command, &target)?;
    crate::development::clear_client_context(&mut command);
    command
        .env(WINDOW_ENV, "1")
        .arg("__dev-window")
        .arg(kind)
        .current_dir(root)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    crate::daemon::spawn::configure_background(&mut command);
    crate::daemon::spawn::spawn_and_reap(&mut command).map_err(|e| e.to_string())?;
    Ok(())
}

pub fn run_window(kind: &str) -> Result<(), String> {
    let target = routing::inherited()?.ok_or("development window requires a resolved target")?;
    if target.kind != routing::TargetKind::Dev {
        return Err("development window cannot use system data".into());
    }
    let config = crate::config::AppConfig::load_without_secrets();
    match kind {
        "settings" => crate::app::run_settings(config),
        "history" => crate::app::run_history(
            target.worktree.unwrap().to_string_lossy().to_string(),
            false,
            config,
        ),
        "todos" => crate::app::run_todos(
            target.worktree.unwrap().to_string_lossy().to_string(),
            config,
        ),
        "agents" => crate::app::run_agents(config),
        _ => Err("unknown development window".into()),
    }
}

#[tauri::command]
pub async fn development_status() -> Result<Status, String> {
    tokio::task::spawn_blocking(status)
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn development_register(worktree: String) -> Result<(), String> {
    tokio::task::spawn_blocking(move || register(Path::new(&worktree)))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn development_set_delivery(worktree: String, delivery: String) -> Result<(), String> {
    let delivery = verification::Delivery::parse(&delivery)?;
    tokio::task::spawn_blocking(move || {
        verification::save_preferences(
            Path::new(&worktree),
            &verification::Preferences { delivery },
        )
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn development_prepare_profile(
    worktree: String,
    agent: String,
) -> Result<session::Profile, String> {
    let kind = crate::agents::AgentKind::parse(&agent).ok_or("unknown test Agent")?;
    tokio::task::spawn_blocking(move || session::prepare(Path::new(&worktree), kind))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn development_start_session(
    worktree: String,
    agent: String,
    task: String,
) -> Result<serde_json::Value, String> {
    let kind = crate::agents::AgentKind::parse(&agent).ok_or("unknown test Agent")?;
    tokio::task::spawn_blocking(move || session::launch(Path::new(&worktree), kind, &task))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn development_open_window(worktree: String, kind: String) -> Result<(), String> {
    tokio::task::spawn_blocking(move || open_window(Path::new(&worktree), &kind))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn development_open_system_settings() -> Result<(), String> {
    tokio::task::spawn_blocking(|| {
        let target = super::relay::system_target()?;
        let mut command = std::process::Command::new(&target.binary);
        routing::configure(&mut command, &target)?;
        crate::development::clear_client_context(&mut command);
        command
            .args(["--settings", "development"])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        crate::daemon::spawn::configure_background(&mut command);
        crate::daemon::spawn::spawn_and_reap(&mut command).map_err(|e| e.to_string())?;
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn development_migration(
    action: String,
    id: Option<String>,
) -> Result<Option<super::migration::Journal>, String> {
    tokio::task::spawn_blocking(move || match action.as_str() {
        "status" => super::migration::latest(),
        "plan" => super::migration::plan().map(Some),
        "apply" => super::migration::apply(id.as_deref().ok_or("missing migration ID")?).map(Some),
        "rollback" => {
            super::migration::rollback(id.as_deref().ok_or("missing migration ID")?).map(Some)
        }
        _ => Err("unknown migration action".into()),
    })
    .await
    .map_err(|e| e.to_string())?
}
