//! New Agent tasks from the iOS app (docs/plans/ios-channel.md §4.7): what the phone may choose
//! from, and starting the Agent in a new terminal window through the same launcher IM `/new` uses.

use std::collections::HashMap;
use std::path::Path;

use sha2::{Digest, Sha256};

use super::ios_agents::agent_key;
use super::task_launch::{
    resolve_task_input, start_task, task_gate_now, StartError, Started, TaskBlock, TaskSpec,
};
use super::*;
use crate::config::AgentTaskPermission;
use crate::integrations::agent_launch::{self, LaunchPermission, LaunchRecord};
use crate::ios::gateway::{TaskChoices, TaskLaunch, TodoChoices};
use crate::ios::wire::{AgentOption, TaskOptions, TodoOption, WorkspaceOption};

/// Longest todo text sent to the phone; the full text stays on the Mac and is resolved by id.
const TODO_PREVIEW_CHARS: usize = 600;
/// Longest reason text passed through to the phone.
const REASON_MAX_CHARS: usize = 160;

fn block_reason(block: TaskBlock) -> &'static str {
    match block {
        TaskBlock::Disabled => "disabled",
        TaskBlock::NotReady => "notReady",
        TaskBlock::NoTerminal => "noTerminal",
    }
}

fn permission_mode(config: &AppConfig) -> &'static str {
    match config.agent_tasks.permission_prompt {
        AgentTaskPermission::Ask => "ask",
        AgentTaskPermission::AgentDefault => "agentDefault",
        AgentTaskPermission::Yolo => "yolo",
    }
}

/// Stable id of a workspace on the wire: the phone never sees (or sends) a path.
fn workspace_id(path: &str) -> String {
    Sha256::digest(path.as_bytes())
        .iter()
        .take(8)
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn clip(text: &str, max: usize) -> String {
    let text = text.trim();
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut out: String = text.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

/// `~/code/app` for a workspace inside the home directory.
fn parent_label(path: &str) -> String {
    let parent = Path::new(path)
        .parent()
        .map(|parent| parent.to_string_lossy().to_string())
        .unwrap_or_default();
    let home = crate::paths::home().to_string_lossy().to_string();
    match parent.strip_prefix(&home) {
        Some(rest) => format!("~{rest}"),
        None => parent,
    }
}

/// What a phone may pick from. Mirrors the gates of IM `/new`, but reports them as codes.
pub(super) async fn choices(state: &Arc<ServerState>) -> TaskChoices {
    let config = state.config_snapshot();
    let mode = permission_mode(&config);
    if let Err(block) = task_gate_now(&config) {
        return TaskChoices::blocked(mode, block_reason(block));
    }
    // The workspace scan and the Agent probes (login-shell lookups that can take seconds on a slow
    // shell) are independent; run side by side so the phone's 15 s wait is spent once, not twice.
    let (workspaces, probes) = tokio::join!(
        tokio::task::spawn_blocking(crate::agents::workspaces::refresh),
        tokio::task::spawn_blocking(|| (
            agent_launch::all_readiness(),
            crate::codex_desktop::launch_status(false),
        )),
    );
    let workspaces = workspaces.unwrap_or_default();
    let (readiness, codex) =
        probes.unwrap_or_else(|_| (Vec::new(), crate::codex_desktop::launch_status(false)));
    if workspaces.is_empty() {
        return TaskChoices::blocked(mode, "noWorkspace");
    }
    let lang = Lang::current();
    let agents: Vec<AgentOption> = readiness
        .into_iter()
        .map(|item| {
            let is_codex = item.kind == AgentKind::Codex;
            let ready = if is_codex {
                codex.target.is_some()
            } else {
                item.ready
            };
            let subtitle = if is_codex {
                codex.description(lang)
            } else {
                item.integration_mode.clone()
            };
            let reason = (!ready).then(|| {
                let reason = if is_codex {
                    codex.description(lang)
                } else {
                    item.diagnostics.first().cloned().unwrap_or_default()
                };
                clip(&reason, REASON_MAX_CHARS)
            });
            AgentOption {
                kind: item.kind.as_str().to_string(),
                label: item.label,
                ready,
                subtitle,
                // Pi has no permission step; otherwise the Mac decides whether the user is asked.
                permission_selectable: item.kind != AgentKind::Pi && mode == "ask",
                reason: reason.filter(|reason| !reason.is_empty()),
            }
        })
        .collect();
    if !agents.iter().any(|agent| agent.ready) {
        return TaskChoices::blocked(mode, "noAgent");
    }
    let mut ids = HashMap::new();
    let workspaces = workspaces
        .into_iter()
        .map(|workspace| {
            let id = workspace_id(&workspace.path);
            ids.insert(id.clone(), workspace.path.clone());
            WorkspaceOption {
                id,
                subtitle: parent_label(&workspace.path),
                label: workspace.label,
                pinned: workspace.pinned,
                last_used_at: workspace.last_used_at as i64 * 1000,
            }
        })
        .collect();
    TaskChoices {
        options: TaskOptions {
            enabled: true,
            blocked: None,
            permission_mode: mode.to_string(),
            workspaces,
            agents,
        },
        workspaces: ids,
    }
}

/// The TODOs of the project a workspace belongs to, as IM `/new` offers them.
pub(super) fn todos(workspace: &str) -> TodoChoices {
    let project = crate::project::detect_from(Path::new(workspace));
    let mut todos = crate::todos::list(&project);
    let overflow = todos.len().saturating_sub(crate::todos::MAX_OPTION_TODOS) as u64;
    todos.truncate(crate::todos::MAX_OPTION_TODOS);
    TodoChoices {
        todos: todos
            .into_iter()
            .map(|todo| TodoOption {
                id: todo.id,
                text: clip(&todo.text, TODO_PREVIEW_CHARS),
                auto: todo.auto,
                attachments: todo.attachments.len() as u64,
            })
            .collect(),
        overflow,
    }
}

/// The permission a launch will use, or `None` when the phone asked for something the Mac's
/// setting no longer allows (the phone's options are stale).
fn effective_permission(
    mode: AgentTaskPermission,
    kind: AgentKind,
    requested: &str,
) -> Option<LaunchPermission> {
    if kind == AgentKind::Pi {
        return Some(LaunchPermission::AgentDefault);
    }
    let requested = match requested {
        "default" => LaunchPermission::AgentDefault,
        "yolo" => LaunchPermission::Yolo,
        _ => return None,
    };
    let fixed = match mode {
        AgentTaskPermission::Ask => return Some(requested),
        AgentTaskPermission::AgentDefault => LaunchPermission::AgentDefault,
        AgentTaskPermission::Yolo => LaunchPermission::Yolo,
    };
    (fixed == requested).then_some(fixed)
}

/// Remembers a terminal launch so the Agent's first turn can be matched back to the phone.
fn register_pending(
    state: &Arc<ServerState>,
    record: &LaunchRecord,
    phone_id: &str,
    client_msg_id: &str,
) {
    state
        .pending_launches
        .lock()
        .unwrap()
        .push(PendingLaunchWatch {
            id: record.id.clone(),
            channel: None,
            ios: Some((phone_id.to_string(), client_msg_id.to_string())),
            kind: record.kind,
            cwd: record.cwd.clone(),
            task_sha256: record.task_sha256.clone(),
            source_session_id: None,
            created_at: now_secs(),
        });
}

/// Starts a validated `task.create`. `Err` is a short reason for the phone.
pub(super) async fn create(state: &Arc<ServerState>, launch: TaskLaunch) -> Result<(), String> {
    let config = state.config_snapshot();
    task_gate_now(&config).map_err(|block| block_reason(block).to_string())?;
    let kind = AgentKind::parse(&launch.agent_kind).ok_or("invalid")?;
    if !Path::new(&launch.workspace).is_dir() {
        return Err("workspaceGone".into());
    }
    let ready = tokio::task::spawn_blocking(move || {
        if kind == AgentKind::Codex {
            crate::codex_desktop::launch_status(false).target.is_some()
        } else {
            agent_launch::readiness(kind).ready
        }
    })
    .await
    .unwrap_or(false);
    if !ready {
        return Err("agentNotReady".into());
    }
    let permission = effective_permission(
        config.agent_tasks.permission_prompt,
        kind,
        &launch.permission,
    )
    .ok_or("permissionChanged")?;
    let project = crate::project::detect_from(Path::new(&launch.workspace));
    let todo = match launch.todo_id.as_deref() {
        Some(id) => Some(
            crate::todos::list(&project)
                .into_iter()
                .find(|todo| todo.id == id)
                .ok_or("todoGone")?,
        ),
        None => None,
    };
    let task = resolve_task_input(todo.as_ref(), Some(&launch.text)).map_err(|_| "invalid")?;

    // Asking for work from the phone makes it the place to answer what the Agent asks next, the
    // same as `/new` does for the IM it was typed in.
    let _ = set_active_channel(state, "ios").await;

    let phone_id = launch.phone_id.clone();
    let client_msg_id = launch.client_msg_id.clone();
    let spec = TaskSpec {
        channel: "ios".to_string(),
        source_target: launch.phone_id,
        workspace: launch.workspace,
        kind,
        permission,
        target: None,
        task,
        todo: todo.map(|todo| (project, todo)),
    };
    let result = start_task(state, spec, |record| {
        register_pending(state, record, &phone_id, &client_msg_id)
    })
    .await;
    state.watch.notify.notify_one();
    match result {
        Ok(Started::Terminal) => Ok(()),
        Ok(Started::Desktop(answer)) => {
            // The Codex App names the session right away: there is nothing to wait for.
            if let (Some(sid), Some(gateway)) = (answer["sessionId"].as_str(), ios_gateway(state)) {
                gateway.task_matched(&phone_id, &client_msg_id, &agent_key(AgentKind::Codex, sid));
            }
            Ok(())
        }
        Err(StartError::Message(message)) => Err(clip(&message, REASON_MAX_CHARS)),
        Err(StartError::Terminal(error)) => {
            log(&format!("ios: task launch failed: {error:#}"));
            Err("terminalFailed".into())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_ids_are_stable_and_distinct() {
        assert_eq!(workspace_id("/a/b"), workspace_id("/a/b"));
        assert_ne!(workspace_id("/a/b"), workspace_id("/a/c"));
        assert_eq!(workspace_id("/a/b").len(), 16);
    }

    #[test]
    fn the_mac_setting_decides_what_the_phone_may_ask_for() {
        use AgentTaskPermission::*;
        let allowed = |mode, kind, requested: &str| effective_permission(mode, kind, requested);
        // Asking: the phone chooses.
        assert_eq!(
            allowed(Ask, AgentKind::Claude, "yolo"),
            Some(LaunchPermission::Yolo)
        );
        assert_eq!(
            allowed(Ask, AgentKind::Claude, "default"),
            Some(LaunchPermission::AgentDefault)
        );
        // A fixed setting is not overridden, and a stale request is refused instead of changed.
        assert_eq!(
            allowed(AgentDefault, AgentKind::Claude, "default"),
            Some(LaunchPermission::AgentDefault)
        );
        assert_eq!(allowed(AgentDefault, AgentKind::Claude, "yolo"), None);
        assert_eq!(
            allowed(Yolo, AgentKind::Claude, "yolo"),
            Some(LaunchPermission::Yolo)
        );
        assert_eq!(allowed(Yolo, AgentKind::Claude, "default"), None);
        // Pi has no permission step at all.
        assert_eq!(
            allowed(Yolo, AgentKind::Pi, "yolo"),
            Some(LaunchPermission::AgentDefault)
        );
        assert_eq!(allowed(Ask, AgentKind::Claude, "root"), None);
    }

    #[test]
    fn blocks_have_stable_codes() {
        assert_eq!(block_reason(TaskBlock::Disabled), "disabled");
        assert_eq!(block_reason(TaskBlock::NotReady), "notReady");
        assert_eq!(block_reason(TaskBlock::NoTerminal), "noTerminal");
    }

    #[test]
    fn long_text_is_clipped_with_an_ellipsis() {
        assert_eq!(clip("  short  ", 10), "short");
        let clipped = clip(&"x".repeat(50), 10);
        assert_eq!(clipped.chars().count(), 10);
        assert!(clipped.ends_with('…'));
    }
}
