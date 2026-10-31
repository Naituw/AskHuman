//! Interjections, new tasks and the "take over questions" button (plan §4.7).
//!
//! Slow host work (listing workspaces, starting a terminal) runs outside the actor and continues
//! through [`Cmd::Run`], so questions and answers keep flowing meanwhile.

use std::collections::HashMap;
use std::future::Future;

use serde_json::json;
use sha2::{Digest, Sha256};

use super::hostapi::{InterjectResult, StagedFile, TaskLaunch};
use super::{Actor, Cmd};
use crate::agents::model_catalog::{valid_effort, valid_model_id};
use crate::ios::store::JournalLookup;
use crate::ios::wire::{self, t};

/// The workspaces of the last `task.options` stay valid this long.
const WORKSPACES_TTL_MS: i64 = 30 * 60 * 1000;
/// A started task has this long to show up as an Agent session.
const TASK_MATCH_TIMEOUT_MS: i64 = 60_000;
const MAX_TASK_CHARS: usize = 3000;
const MAX_INTERJECT_CHARS: usize = 20_000;

/// Workspace ids a phone may name in `task.create`.
pub(crate) struct WorkspaceMap {
    pub at: i64,
    pub paths: HashMap<String, String>,
}

/// A task that was started and is waiting for its Agent session.
pub(crate) struct TaskWait {
    pub phone_id: String,
    pub client_msg_id: String,
    pub until: i64,
}

pub(super) fn body_hash(value: &serde_json::Value) -> String {
    let digest = Sha256::digest(serde_json::to_vec(value).unwrap_or_default());
    digest[..16].iter().map(|b| format!("{b:02x}")).collect()
}

/// What makes two `task.create` messages the same task: the journal replays the answer to a
/// repeat and calls anything else under the same id a conflict. A model and an effort count only
/// when one was picked, so a task that names neither keeps the identity it always had (a phone
/// that retries across a Mac update must not be told it conflicts with itself).
fn task_identity(create: &wire::TaskCreate) -> serde_json::Value {
    let mut identity = json!({
        "workspaceId": create.workspace_id,
        "agentKind": create.agent_kind,
        "permission": create.permission,
        "todoId": create.todo_id,
        "text": create.text,
    });
    if let Some(model) = &create.model {
        identity["model"] = json!(model);
    }
    if let Some(effort) = &create.effort {
        identity["effort"] = json!(effort);
    }
    identity
}

fn interject_identity(send: &wire::InterjectSend) -> serde_json::Value {
    let mut identity = json!({"agentKey": send.agent_key, "text": send.text, "files": send.files});
    if let Some(expected) = &send.expected_result_id {
        identity["expectedResultId"] = json!(expected);
    }
    identity
}

impl Actor {
    /// Runs `work` in the background and hands its result back to the actor.
    pub(super) fn spawn_then<T, Fut>(
        &self,
        work: Fut,
        then: impl FnOnce(&mut Actor, T) + Send + 'static,
    ) where
        T: Send + 'static,
        Fut: Future<Output = T> + Send + 'static,
    {
        let tx = self.cmd_tx.clone();
        tokio::spawn(async move {
            let out = work.await;
            let _ = tx.send(Cmd::Run(Box::new(move |actor| then(actor, out))));
        });
    }

    // ------------------------------------------------------------ interjections

    fn interject_ack(
        &mut self,
        phone_id: &str,
        send: &wire::InterjectSend,
        result: InterjectResult,
    ) -> wire::InterjectAck {
        let ack = wire::InterjectAck {
            notification_registered: None,
            client_msg_id: send.client_msg_id.clone(),
            agent_key: send.agent_key.clone(),
            status: result.status.to_string(),
            reason: result.reason,
            pending: result.pending,
        };
        self.send(phone_id, &ack);
        ack
    }

    pub(super) fn on_interject_send(&mut self, phone_id: &str, send: wire::InterjectSend) {
        let hash = body_hash(&interject_identity(&send));
        if let Some(lookup) =
            self.replay_or_start(phone_id, &send.client_msg_id, t::INTERJECT_SEND, &hash)
        {
            match lookup {
                JournalLookup::Replay(Some(result)) => {
                    self.send_value(phone_id, &result.msg_type, result.body);
                }
                JournalLookup::Conflict => {
                    self.interject_ack(phone_id, &send, InterjectResult::rejected("conflict"));
                }
                _ if self.live.interject_inflight.contains(&send.client_msg_id) => {
                    // The original delivery will acknowledge this message. Do not submit again.
                }
                _ => {
                    self.interject_ack(phone_id, &send, InterjectResult::rejected("interrupted"));
                }
            }
            return;
        }
        let files = match self.prepare_interject(phone_id, &send) {
            Ok(files) => files,
            Err(result) => return self.finish_interject(phone_id, &send, result),
        };
        self.live
            .interject_inflight
            .insert(send.client_msg_id.clone());
        let host = self.host.clone();
        let phone_id = phone_id.to_string();
        let destination = phone_id.clone();
        let message = send.clone();
        self.spawn_then(
            async move {
                host.interject(
                    &destination,
                    &message.client_msg_id,
                    &message.agent_key,
                    message.text.trim(),
                    files,
                    message.expected_result_id.clone(),
                )
                .await
            },
            move |actor, result| {
                actor.live.interject_inflight.remove(&send.client_msg_id);
                actor.finish_interject(&phone_id, &send, result);
            },
        );
    }

    fn finish_interject(
        &mut self,
        phone_id: &str,
        send: &wire::InterjectSend,
        result: InterjectResult,
    ) {
        let delivered = result.status != "rejected";
        let notification_registered = result.started_control.as_ref().map(|control| {
            self.activate_control_follow(&send.agent_key, control);
            self.store.state.phone(phone_id).is_some_and(|phone| {
                phone.follow_turns.get(&send.agent_key) == Some(&control.turn_id)
            })
        });
        let ack = wire::InterjectAck {
            client_msg_id: send.client_msg_id.clone(),
            agent_key: send.agent_key.clone(),
            status: result.status.to_string(),
            reason: result.reason,
            pending: result.pending,
            notification_registered,
        };
        self.finish_journal(&send.client_msg_id, t::INTERJECT_ACK, &ack);
        self.send(phone_id, &ack);
        if delivered {
            // The pending marker of the Agent changed: show it without waiting for the next poll.
            self.live.agents_at = 0;
        }
    }

    fn prepare_interject(
        &mut self,
        phone_id: &str,
        send: &wire::InterjectSend,
    ) -> Result<Vec<StagedFile>, InterjectResult> {
        let text = send.text.trim();
        if text.chars().count() > MAX_INTERJECT_CHARS {
            return Err(InterjectResult::rejected("tooLong"));
        }
        if text.is_empty() && send.files.is_empty() {
            return Err(InterjectResult::rejected("empty"));
        }
        // The state decides whether the text can still reach the Agent: never go by the cache.
        self.refresh_agents(self.now());
        let Some(agent) = self
            .live
            .agents
            .iter()
            .find(|a| a.key == send.agent_key)
            .cloned()
        else {
            return Err(InterjectResult::rejected("notFound"));
        };
        if !agent.can_interject.ok {
            let reason = agent
                .can_interject
                .reason
                .as_deref()
                .unwrap_or("notWorking");
            return Err(InterjectResult::rejected(reason));
        }
        let files = match self.take_agent_uploads(phone_id, &send.agent_key, &send.files) {
            Ok(files) => files,
            Err(_) => return Err(InterjectResult::rejected("attachment")),
        };
        let staged: Vec<StagedFile> = files
            .into_iter()
            .map(|file| StagedFile {
                size: std::fs::metadata(&file.path).map(|m| m.len()).unwrap_or(0),
                is_image: file.mime.starts_with("image/"),
                name: file.name,
                mime: file.mime,
                path: file.path,
            })
            .collect();
        Ok(staged)
    }

    pub(super) fn on_interject_clear(&mut self, phone_id: &str, agent_key: &str) {
        self.host.interject_clear(agent_key);
        let (pending, _) = self.host.interject_state(agent_key);
        self.live.agents_at = 0;
        self.send(
            phone_id,
            &wire::InterjectState {
                agent_key: agent_key.to_string(),
                pending,
                text: None,
            },
        );
    }

    pub(super) fn on_interject_query(&mut self, phone_id: &str, agent_key: &str) {
        let (pending, text) = self.host.interject_state(agent_key);
        self.send(
            phone_id,
            &wire::InterjectState {
                agent_key: agent_key.to_string(),
                pending,
                text: Some(text),
            },
        );
    }

    pub(super) fn on_interject_consumed(&mut self, phone_id: &str, agent_key: &str) {
        self.live.agents_at = 0;
        if self.is_live(phone_id) {
            self.send(
                phone_id,
                &wire::InterjectConsumed {
                    agent_key: agent_key.to_string(),
                },
            );
        }
    }

    // ------------------------------------------------------------ new tasks

    pub(super) fn on_task_options_get(&mut self, phone_id: &str) {
        let host = self.host.clone();
        let phone = phone_id.to_string();
        self.spawn_then(
            async move { host.task_choices().await },
            move |actor, choices| {
                let now = actor.now();
                if let Some(rt) = actor.rt.get_mut(&phone) {
                    rt.workspaces = Some(WorkspaceMap {
                        at: now,
                        paths: choices.workspaces,
                    });
                }
                actor.send(&phone, &choices.options);
            },
        );
    }

    fn workspace_path(&self, phone_id: &str, workspace_id: &str) -> Option<String> {
        let map = self.rt.get(phone_id)?.workspaces.as_ref()?;
        if self.now() - map.at > WORKSPACES_TTL_MS {
            return None;
        }
        map.paths.get(workspace_id).cloned()
    }

    pub(super) fn on_task_todos_get(&mut self, phone_id: &str, workspace_id: &str) {
        let workspace_id = workspace_id.to_string();
        let Some(path) = self.workspace_path(phone_id, &workspace_id) else {
            self.send(
                phone_id,
                &wire::TaskTodos {
                    workspace_id,
                    todos: Vec::new(),
                    overflow: 0,
                },
            );
            return;
        };
        let host = self.host.clone();
        let phone = phone_id.to_string();
        self.spawn_then(
            async move { host.task_todos(&path).await },
            move |actor, choices| {
                actor.send(
                    &phone,
                    &wire::TaskTodos {
                        workspace_id,
                        todos: choices.todos,
                        overflow: choices.overflow,
                    },
                );
            },
        );
    }

    fn task_status(
        &mut self,
        phone_id: &str,
        client_msg_id: &str,
        stage: &str,
        reason: Option<&str>,
    ) -> wire::TaskStatus {
        let status = wire::TaskStatus {
            client_msg_id: client_msg_id.to_string(),
            stage: stage.to_string(),
            reason: reason.map(str::to_string),
            agent_key: None,
        };
        self.send(phone_id, &status);
        status
    }

    /// The task did not start: tell the phone and make the journal replay that outcome.
    fn task_failed(&mut self, phone_id: &str, client_msg_id: &str, reason: &str) {
        let status = self.task_status(phone_id, client_msg_id, "failed", Some(reason));
        self.finish_journal(client_msg_id, t::TASK_STATUS, &status);
    }

    pub(super) fn on_task_create(&mut self, phone_id: &str, create: wire::TaskCreate) {
        let id = create.client_msg_id.clone();
        let hash = body_hash(&task_identity(&create));
        if let Some(lookup) = self.replay_or_start(phone_id, &id, t::TASK_CREATE, &hash) {
            match lookup {
                JournalLookup::Replay(Some(result)) => {
                    self.send_value(phone_id, &result.msg_type, result.body);
                }
                JournalLookup::Conflict => {
                    self.task_status(phone_id, &id, "failed", Some("conflict"));
                }
                _ if self.live.task_inflight.contains(&id) => {
                    // The first attempt is still starting the terminal: no second launch.
                    self.task_status(phone_id, &id, "accepted", None);
                }
                _ => {
                    self.task_status(phone_id, &id, "failed", Some("interrupted"));
                }
            }
            return;
        }
        let Some(workspace) = self.workspace_path(phone_id, &create.workspace_id) else {
            return self.task_failed(phone_id, &id, "workspaceExpired");
        };
        if create.agent_kind.trim().is_empty()
            || !matches!(create.permission.as_str(), "default" | "yolo")
            // Whether they name something the Agent offers is the host's call; a value that
            // could not be one is refused here, before anything else looks at it.
            || create.model.as_deref().is_some_and(|m| !valid_model_id(m))
            || create.effort.as_deref().is_some_and(|e| !valid_effort(e))
        {
            return self.task_failed(phone_id, &id, "invalid");
        }
        let text = create.text.trim().to_string();
        if text.contains('\0') || text.chars().count() > MAX_TASK_CHARS {
            return self.task_failed(phone_id, &id, "invalid");
        }
        if text.is_empty() && create.todo_id.is_none() {
            return self.task_failed(phone_id, &id, "empty");
        }
        self.task_status(phone_id, &id, "accepted", None);
        self.live.task_inflight.insert(id.clone());
        let launch = TaskLaunch {
            phone_id: phone_id.to_string(),
            client_msg_id: id.clone(),
            workspace,
            agent_kind: create.agent_kind,
            permission: create.permission,
            todo_id: create.todo_id,
            text,
            model: create.model,
            effort: create.effort,
        };
        let host = self.host.clone();
        let phone = phone_id.to_string();
        self.spawn_then(
            async move { host.task_create(launch).await },
            move |actor, result| {
                actor.live.task_inflight.remove(&id);
                let early = actor.live.task_early.remove(&id);
                match result {
                    Ok(()) => {
                        let status = actor.task_status(&phone, &id, "launched", None);
                        actor.finish_journal(&id, t::TASK_STATUS, &status);
                        let until = actor.now() + TASK_MATCH_TIMEOUT_MS;
                        actor.live.task_waits.push(TaskWait {
                            phone_id: phone.clone(),
                            client_msg_id: id.clone(),
                            until,
                        });
                        if let Some(agent_key) = early {
                            actor.on_task_matched(&phone, &id, &agent_key);
                        }
                    }
                    Err(reason) => actor.task_failed(&phone, &id, &reason),
                }
            },
        );
    }

    /// The Agent session of a task showed up: follow it for the phone when there is room.
    pub(super) fn on_task_matched(&mut self, phone_id: &str, client_msg_id: &str, agent_key: &str) {
        let before = self.live.task_waits.len();
        self.live
            .task_waits
            .retain(|w| !(w.phone_id == phone_id && w.client_msg_id == client_msg_id));
        if self.live.task_waits.len() == before {
            if self.live.task_inflight.contains(client_msg_id) {
                // The session beat the launcher's return: apply the match once it returns.
                self.live
                    .task_early
                    .insert(client_msg_id.to_string(), agent_key.to_string());
            }
            return;
        }
        let followed = self.follow_if_room(phone_id, agent_key);
        self.live.agents_at = 0;
        self.send(
            phone_id,
            &wire::TaskStatus {
                client_msg_id: client_msg_id.to_string(),
                stage: if followed { "watching" } else { "launched" }.to_string(),
                reason: (!followed).then(|| "followLimit".to_string()),
                agent_key: Some(agent_key.to_string()),
            },
        );
    }

    pub(super) fn tick_tasks(&mut self, now: i64) {
        if self.live.task_waits.is_empty() {
            return;
        }
        let (expired, waiting): (Vec<TaskWait>, Vec<TaskWait>) =
            std::mem::take(&mut self.live.task_waits)
                .into_iter()
                .partition(|w| now >= w.until);
        self.live.task_waits = waiting;
        for wait in expired {
            self.task_status(&wait.phone_id, &wait.client_msg_id, "unmatched", None);
        }
    }

    // ------------------------------------------------------------ slot

    pub(super) fn on_slot_take(&mut self, phone_id: &str, _client_msg_id: &str) {
        let host = self.host.clone();
        let phone = phone_id.to_string();
        self.spawn_then(async move { host.take_slot().await }, move |actor, ()| {
            actor.send_stream_snapshot(&phone, "slot");
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain() -> wire::TaskCreate {
        wire::TaskCreate {
            client_msg_id: "n1".into(),
            workspace_id: "w1".into(),
            agent_kind: "claude".into(),
            permission: "default".into(),
            todo_id: None,
            text: "build it".into(),
            model: None,
            effort: None,
        }
    }

    #[test]
    fn continuation_identity_includes_the_expected_result_without_changing_old_free_messages() {
        let mut send = wire::InterjectSend {
            client_msg_id: "one".into(),
            agent_key: "cursor:s".into(),
            text: "Continue".into(),
            files: vec![],
            expected_result_id: None,
        };
        let old = json!({"agentKey":"cursor:s", "text":"Continue", "files":[]});
        assert_eq!(interject_identity(&send), old);
        send.expected_result_id = Some("first".into());
        let first = interject_identity(&send);
        send.expected_result_id = Some("second".into());
        assert_ne!(body_hash(&first), body_hash(&interject_identity(&send)));
    }

    #[test]
    fn a_task_that_names_no_model_keeps_the_identity_it_had_before_models_existed() {
        // Computed from the old five-field identity, outside this code: a journal written by an
        // older Mac must still recognise the phone's retry.
        assert_eq!(
            body_hash(&task_identity(&plain())),
            "6f8f72b4ab3e6cabc3b99ad75b7a3b89"
        );
        let todo = wire::TaskCreate {
            todo_id: Some("t1".into()),
            text: String::new(),
            ..plain()
        };
        assert_eq!(
            body_hash(&task_identity(&todo)),
            "d3ff0bf473a4f6a884bc7aeda12c83ad"
        );
        // The message id is not part of what a task is.
        let again = wire::TaskCreate {
            client_msg_id: "n2".into(),
            ..plain()
        };
        assert_eq!(task_identity(&again), task_identity(&plain()));
    }

    #[test]
    fn a_picked_model_or_effort_makes_a_different_task() {
        let with = |model: Option<&str>, effort: Option<&str>| {
            body_hash(&task_identity(&wire::TaskCreate {
                model: model.map(str::to_string),
                effort: effort.map(str::to_string),
                ..plain()
            }))
        };
        let all = [
            with(None, None),
            with(Some("model-a"), None),
            with(Some("model-b"), None),
            with(None, Some("high")),
            with(Some("model-a"), Some("high")),
            with(Some("model-a"), Some("low")),
        ];
        let distinct: std::collections::HashSet<_> = all.iter().collect();
        assert_eq!(distinct.len(), all.len(), "{all:?}");
        assert_eq!(all[0], body_hash(&task_identity(&plain())));
    }
}
