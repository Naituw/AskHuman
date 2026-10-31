//! A worktree-owned development workbench; ordinary settings have no development controls.

use super::{build, routing, session, state, verification, workspace};
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
    pub system_binary: PathBuf,
    pub worktrees: Vec<WorktreeInfo>,
    pub agents: Vec<session::Support>,
    pub instance_target: routing::Target,
}

fn current_target() -> Result<routing::Target, String> {
    let target = routing::inherited()?.ok_or("open this worktree's Dev workbench")?;
    if target.kind != routing::TargetKind::Dev || std::env::var_os(WINDOW_ENV).is_none() {
        return Err("development commands are available only inside the Dev workbench".into());
    }
    workspace::read(
        target
            .worktree
            .as_deref()
            .ok_or("Dev worktree identity is missing")?,
    )?;
    Ok(target)
}

fn require_root(root: &Path) -> Result<PathBuf, String> {
    let target = current_target()?;
    let expected = target.worktree.ok_or("Dev worktree identity is missing")?;
    if !state::same_path(root, &expected) {
        return Err("this workbench cannot operate another worktree".into());
    }
    Ok(expected)
}

pub fn status() -> Result<Status, String> {
    let target = current_target()?;
    let root = target
        .worktree
        .as_ref()
        .ok_or("Dev worktree identity is missing")?
        .clone();
    let workspace = workspace::read(&root)?;
    let (build, error) = match build::verify(&root) {
        Ok(build) => (Some(build), None),
        Err(error) => (None, Some(error)),
    };
    let worktree = WorktreeInfo {
        name: root
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string(),
        binary: target.binary.clone(),
        home: target.home.clone(),
        enabled: true,
        preferences: verification::preferences(&root)?,
        root,
        build,
        error,
    };
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
        system_binary: workspace.system_binary,
        worktrees: vec![worktree],
        agents,
        instance_target: target,
    })
}

pub fn open_window(root: &Path, kind: &str) -> Result<(), String> {
    if !matches!(
        kind,
        "workbench" | "settings" | "history" | "todos" | "agents"
    ) {
        return Err("unknown development window".into());
    }
    if kind == "workbench" && !cfg!(feature = "development-ui") {
        return Err("reinstall this Dev with the development UI feature".into());
    }
    build::verify(root)?;
    let target = routing::dev_target(root)?;
    let mut command = std::process::Command::new(&target.binary);
    routing::configure(&mut command, &target)?;
    crate::development::clear_client_context(&mut command);
    command
        .env(WINDOW_ENV, kind)
        .arg("__dev-window")
        .arg(kind)
        .current_dir(root)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    crate::daemon::spawn::configure_background(&mut command);
    crate::daemon::spawn::spawn_and_reap(&mut command).map_err(|error| error.to_string())?;
    Ok(())
}

pub fn run_window(kind: &str) -> Result<(), String> {
    let target = current_target()?;
    let config = crate::config::AppConfig::load_without_secrets();
    match kind {
        "workbench" | "settings" => crate::app::run_settings(config),
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
        .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn development_set_delivery(worktree: String, delivery: String) -> Result<(), String> {
    let root = require_root(Path::new(&worktree))?;
    let delivery = verification::Delivery::parse(&delivery)?;
    tokio::task::spawn_blocking(move || {
        verification::save_preferences(&root, &verification::Preferences { delivery })
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn development_prepare_profile(
    worktree: String,
    agent: String,
) -> Result<session::Profile, String> {
    let root = require_root(Path::new(&worktree))?;
    let kind = crate::agents::AgentKind::parse(&agent).ok_or("unknown test Agent")?;
    tokio::task::spawn_blocking(move || session::prepare(&root, kind))
        .await
        .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn development_start_session(
    worktree: String,
    agent: String,
    task: String,
) -> Result<serde_json::Value, String> {
    let root = require_root(Path::new(&worktree))?;
    let kind = crate::agents::AgentKind::parse(&agent).ok_or("unknown test Agent")?;
    tokio::task::spawn_blocking(move || session::launch(&root, kind, &task))
        .await
        .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn development_open_window(worktree: String, kind: String) -> Result<(), String> {
    let root = require_root(Path::new(&worktree))?;
    tokio::task::spawn_blocking(move || open_window(&root, &kind))
        .await
        .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn development_open_system_settings() -> Result<(), String> {
    current_target()?;
    tokio::task::spawn_blocking(|| {
        let target = super::relay::system_target()?;
        let mut command = std::process::Command::new(&target.binary);
        routing::configure(&mut command, &target)?;
        crate::development::clear_client_context(&mut command);
        command
            .arg("--settings")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        crate::daemon::spawn::configure_background(&mut command);
        crate::daemon::spawn::spawn_and_reap(&mut command).map_err(|error| error.to_string())?;
        Ok(())
    })
    .await
    .map_err(|error| error.to_string())?
}
