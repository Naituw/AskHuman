//! Starting an Agent task on this Mac, shared by every surface that can ask for one: IM `/new`
//! and the iOS app. A surface collects the choices, words the outcome in its own way and, for the
//! terminal path, says what to remember about the launch before the window opens.

use super::*;
use crate::agents::model_catalog::ModelChoice;
use crate::codex_desktop::LaunchTarget;
use crate::integrations::agent_launch::{self, LaunchPermission, LaunchRecord, LaunchSource};
use crate::todos::TodoEntry;

/// Longest task text, in characters (the limit of the IM input card).
pub(super) const MAX_TASK_CHARS: usize = 3000;

/// Why creating a task is not possible right now. Each surface words it itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum TaskBlock {
    /// "Create Agent tasks from IM" is off in Settings.
    Disabled,
    /// The daemon is not kept alive by its login item, so a new terminal could not be driven.
    NotReady,
    /// Neither a supported terminal nor the Codex App is available.
    NoTerminal,
}

/// The gate in front of every task flow, in the order the settings page explains them.
pub(super) fn task_gate(
    enabled: bool,
    keepalive_ready: bool,
    terminal: bool,
    codex_desktop: bool,
) -> Result<(), TaskBlock> {
    if !enabled {
        return Err(TaskBlock::Disabled);
    }
    if !keepalive_ready {
        return Err(TaskBlock::NotReady);
    }
    if !terminal && !codex_desktop {
        return Err(TaskBlock::NoTerminal);
    }
    Ok(())
}

/// [`task_gate`] with the facts read from this machine.
pub(super) fn task_gate_now(config: &AppConfig) -> Result<(), TaskBlock> {
    let keepalive_ready = config.general.daemon_lifecycle
        == crate::config::DaemonLifecycleMode::KeepAlive
        && crate::integrations::login_item::daemon_is_installed()
        && !crate::integrations::login_item::daemon_needs_update();
    task_gate(
        config.agent_tasks.enabled,
        keepalive_ready,
        agent_launch::terminal_available(),
        crate::codex_desktop::available(&config.codex_desktop),
    )
}

/// The text of a task: the TODO's own text followed by what the user added, or just what the user
/// typed. Rejects empty, NUL-carrying and over-long text.
pub(super) fn resolve_task_input(
    todo: Option<&TodoEntry>,
    input: Option<&str>,
) -> Result<String, String> {
    let input = input.unwrap_or_default().trim();
    let task = match todo {
        Some(todo) if input.is_empty() => todo.text.trim().to_string(),
        Some(todo) => format!("{}\n\n{input}", todo.text.trim()),
        None => input.to_string(),
    };
    if task.is_empty() {
        return Err("Task must not be empty".into());
    }
    if task.contains('\0') {
        return Err("Task must not contain NUL characters".into());
    }
    if task.chars().count() > MAX_TASK_CHARS {
        return Err("Task exceeds 3000 characters".into());
    }
    Ok(task)
}

/// Everything a launch needs once the user decided.
pub(super) struct TaskSpec {
    /// Additional trusted local files, such as task-scoped iOS uploads.
    pub files: Vec<String>,
    /// Who asked (`LaunchSource::channel`): an IM channel id or `ios`.
    pub channel: String,
    /// Where the asker lives (`LaunchSource::target`): the IM user or the phone id.
    pub source_target: String,
    pub workspace: String,
    pub kind: AgentKind,
    pub permission: LaunchPermission,
    pub target: Option<LaunchTarget>,
    pub task: String,
    /// `(project key, entry)` of the TODO the task comes from; taken off the list once started.
    pub todo: Option<(String, TodoEntry)>,
    /// The model and effort picked for the task; the default leaves both to the Agent. Callers
    /// pass values already matched against the Agent's catalog.
    pub choice: ModelChoice,
}

/// Where the task went.
pub(super) enum Started {
    /// A new terminal window was opened.
    Terminal,
    /// The Codex App accepted it; the value is the app's answer (carries `sessionId`).
    Desktop(serde_json::Value),
}

pub(super) enum StartError {
    /// Nothing was started, or the Codex App refused; the text is meant for the user as it is.
    Message(String),
    /// The terminal could not be opened.
    Terminal(anyhow::Error),
}

/// Starts the task. `on_pending` runs for a terminal launch right before the window opens, with
/// the record the Agent's first session event will be matched against; when the window cannot be
/// opened the registration is dropped again.
pub(super) async fn start_task(
    state: &Arc<ServerState>,
    spec: TaskSpec,
    on_pending: impl FnOnce(&LaunchRecord),
) -> Result<Started, StartError> {
    let TaskSpec {
        files,
        channel,
        source_target,
        workspace,
        kind,
        permission,
        target,
        task,
        todo,
        choice,
    } = spec;
    let source = LaunchSource {
        channel,
        target: source_target,
    };
    let delivery_request_id = uuid::Uuid::new_v4().to_string();
    let mut delivery = if let Some((project, todo)) = todo.as_ref() {
        let expected: Vec<_> = todo.attachments.iter().map(|a| a.snapshot()).collect();
        crate::todos::prepare_delivery_consistent(
            project,
            &todo.id,
            &expected,
            &delivery_request_id,
        )
    } else {
        crate::todo_attachments::TodoDelivery::default()
    };
    delivery.files.extend(files);
    let take_todo = || {
        if let Some((project, todo)) = todo.as_ref() {
            let _ = crate::todos::take(project, std::slice::from_ref(&todo.id));
        }
    };
    let target = crate::codex_desktop::resolve_launch(kind, Some(&delivery_request_id), target)
        .map_err(StartError::Message)?;

    if target == LaunchTarget::Desktop {
        if !delivery.warnings.is_empty() {
            return Err(StartError::Message(delivery.warnings.join("\n")));
        }
        let answer = crate::codex_desktop::shared()
            .execute(crate::codex_desktop::Operation::Create {
                cwd: workspace,
                text: task,
                files: delivery.files,
                permission: match permission {
                    LaunchPermission::AgentDefault => "agent-default",
                    LaunchPermission::Yolo => "yolo",
                }
                .into(),
                id: delivery_request_id.clone(),
                model: choice.model,
                effort: choice.effort,
                speed: choice.speed,
            })
            .await
            .map_err(StartError::Message)?;
        if let Some(sid) = answer["sessionId"].as_str() {
            if let Some(session) = crate::codex_desktop::shared().session(sid) {
                if let Some(meta) = session.meta {
                    state
                        .agents
                        .update_desktop(sid, &meta.title, &meta.cwd, true);
                }
            }
        }
        take_todo();
        return Ok(Started::Desktop(answer));
    }

    let record = match agent_launch::create_record_with_choice(
        source,
        std::path::Path::new(&workspace),
        kind,
        permission,
        &task,
        &delivery.files,
        &delivery.warnings,
        &choice,
    ) {
        Ok(record) => record,
        Err(error) => {
            crate::todo_attachments::cleanup_delivery(&delivery_request_id);
            return Err(StartError::Terminal(error));
        }
    };
    on_pending(&record);
    match agent_launch::open_terminal(&record) {
        Ok(()) => {
            take_todo();
            Ok(Started::Terminal)
        }
        Err(error) => {
            crate::todo_attachments::cleanup_delivery(&delivery_request_id);
            state
                .pending_launches
                .lock()
                .unwrap()
                .retain(|item| item.id != record.id);
            Err(StartError::Terminal(error))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_gate_reports_the_first_problem_in_settings_order() {
        assert_eq!(
            task_gate(false, false, false, false),
            Err(TaskBlock::Disabled)
        );
        assert_eq!(
            task_gate(true, false, false, false),
            Err(TaskBlock::NotReady)
        );
        assert_eq!(
            task_gate(true, true, false, false),
            Err(TaskBlock::NoTerminal)
        );
        assert_eq!(task_gate(true, true, true, false), Ok(()));
        // The Codex App alone is enough: it needs no terminal.
        assert_eq!(task_gate(true, true, false, true), Ok(()));
    }
}
