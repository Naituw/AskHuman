//! One-shot phone messages and native proof that a new turn actually started.
use super::*;
use crate::agents::control::Operation;
use crate::agents::result::{Observation, TurnOutcome, TurnResult};
use crate::ios::gateway::{AgentEventInfo, InterjectResult};
use crate::models::FileAttachment;
#[cfg(test)]
use serde_json::json;
use serde_json::Value;

pub(super) const CONTINUE_PROMPT: &str = "Inspect why the previous attempt stopped and what has already been completed. Continue the current task from its existing progress, without repeating successful operations. If you cannot proceed, explain the blocker and ask for the specific help you need.";

pub(super) fn operation_id(phone: &str, message: &str) -> String {
    use sha2::{Digest, Sha256};
    let hash = Sha256::digest(serde_json::to_vec(&["ios-interject", phone, message]).unwrap());
    let mut bytes = [0; 16];
    bytes.copy_from_slice(&hash[..16]);
    uuid::Uuid::from_bytes(bytes).to_string()
}

pub(super) fn can_send(state: &Arc<ServerState>, rec: &Value) -> crate::ios::wire::CanInterject {
    let refuse = |reason: &str| crate::ios::wire::CanInterject {
        ok: false,
        reason: Some(reason.into()),
    };
    let (Some(kind), Some(sid)) = (
        rec["kind"].as_str().and_then(AgentKind::parse),
        rec["sessionId"].as_str(),
    ) else {
        return refuse("notFound");
    };
    if rec["state"] == "ended" {
        return refuse("ended");
    }
    if state.agent_controls.pending(kind, sid) {
        return refuse("controlStarting");
    }
    if crate::codex_desktop::is_desktop_session(rec) {
        return ios_agents::can_interject(rec);
    }
    if rec["state"] == "working" {
        return ios_agents::can_interject(rec);
    }
    let available = match kind {
        AgentKind::Cursor => crate::cursor_desktop::messages_available(sid),
        AgentKind::Pi => state.agent_controls.pi_generation(sid).is_some(),
        _ => false,
    };
    if available {
        crate::ios::wire::CanInterject {
            ok: true,
            reason: None,
        }
    } else {
        refuse("idleUnsupported")
    }
}

fn receipt(operation: &Operation) -> InterjectResult {
    if let Some(started_control) = operation.started_control() {
        InterjectResult {
            status: "delivered",
            reason: None,
            pending: 0,
            started_control: Some(started_control),
        }
    } else {
        InterjectResult::rejected(if operation.status == "rejected" {
            operation.reason.as_deref().unwrap_or("stateChanged")
        } else {
            "controlUnknown"
        })
    }
}

pub(super) struct Message<'a> {
    pub phone: &'a str,
    pub message_id: &'a str,
    pub kind: AgentKind,
    pub sid: &'a str,
    pub text: &'a str,
    pub files: Vec<FileAttachment>,
    pub expected_result_id: Option<String>,
}

pub(super) async fn send(state: &Arc<ServerState>, message: Message<'_>) -> InterjectResult {
    let _guard = state.agent_controls.send_gate.lock().await;
    let id = operation_id(message.phone, message.message_id);
    match state.agent_controls.existing(&id) {
        Ok(Some(operation)) => return receipt(&operation),
        Err(_) => return InterjectResult::rejected("controlUnknown"),
        _ => {}
    }
    let snapshot = state.agents.snapshot();
    let Some(rec) = snapshot.as_array().and_then(|records| {
        records
            .iter()
            .find(|rec| rec["kind"] == message.kind.as_str() && rec["sessionId"] == message.sid)
    }) else {
        return InterjectResult::rejected("notFound");
    };
    if rec["state"] != "idle"
        || rec["waitingRequestId"]
            .as_str()
            .is_some_and(|s| !s.is_empty())
    {
        return InterjectResult::rejected("stateChanged");
    }
    if let Some(expected) = message.expected_result_id.as_deref() {
        if !valid_result(rec, expected) {
            return InterjectResult::rejected("resultChanged");
        }
    }
    let can = can_send(state, rec);
    if !can.ok {
        return InterjectResult::rejected(can.reason.as_deref().unwrap_or("nativeUnavailable"));
    }
    let desktop = crate::codex_desktop::is_desktop_session(rec);
    let operation = Operation {
        id: id.clone(),
        kind: message.kind,
        session_id: message.sid.into(),
        phone_id: message.phone.into(),
        generation: (message.kind == AgentKind::Pi)
            .then(|| state.agent_controls.pi_generation(message.sid))
            .flatten(),
        text: if message.expected_result_id.is_some() {
            CONTINUE_PROMPT.into()
        } else {
            message.text.into()
        },
        files: message.files,
        expected_result_id: message.expected_result_id,
        status: "queued".into(),
        reason: None,
        turn_id: None,
    };
    let Ok(operation) = state.agent_controls.begin(operation) else {
        return InterjectResult::rejected("controlUnknown");
    };
    if operation.status != "queued" {
        return receipt(&operation);
    }
    if message.kind != AgentKind::Pi {
        if state
            .agent_controls
            .finish(&id, "dispatched", None)
            .is_err()
        {
            return InterjectResult::rejected("controlUnknown");
        }
        if desktop {
            let native = crate::codex_desktop::Operation::Send {
                id: id.clone(),
                session_id: message.sid.into(),
                text: operation.text.clone(),
                files: operation
                    .files
                    .iter()
                    .map(|file| file.path.clone())
                    .collect(),
            };
            let status = match crate::codex_desktop::shared().execute_idle(native).await {
                Ok(_) => "accepted",
                Err(_) if crate::codex_desktop::actions::outcome_unknown(&id) => "unknown",
                Err(error) => {
                    let reason = if error.contains("chat started another turn") {
                        "stateChanged"
                    } else {
                        "nativeUnavailable"
                    };
                    let _ = state.agent_controls.reject(&id, reason);
                    "rejected"
                }
            };
            if status != "rejected" {
                let _ = state.agent_controls.finish(&id, status, None);
            }
        } else if message.kind == AgentKind::Cursor {
            let mut text = operation.text.clone();
            for file in &operation.files {
                text.push_str(&format!("\n\nAttached file: {} ({})", file.name, file.path));
            }
            match crate::cursor_desktop::send_message(message.sid, &id, &text).await {
                crate::cursor_desktop::MessageResult::Started(turn_id) => {
                    let _ = state
                        .agent_controls
                        .finish(&id, "accepted", Some(turn_id.clone()));
                    apply(
                        state,
                        message.kind,
                        message.sid,
                        LifecycleEvent::TurnStart,
                        Observation {
                            turn_id: Some(turn_id),
                            control_operation_id: Some(id.clone()),
                            ..Observation::default()
                        },
                        None,
                    );
                }
                crate::cursor_desktop::MessageResult::Rejected(reason) => {
                    let _ = state.agent_controls.reject(&id, &reason);
                }
                crate::cursor_desktop::MessageResult::Unknown => {
                    let _ = state.agent_controls.finish(&id, "unknown", None);
                }
            }
        }
    }
    broadcast_agents_state(state);
    // Desktop state can prove a fast turn before the periodic registry refresh runs.
    if desktop {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(session) = crate::codex_desktop::shared().session(message.sid) {
                observe_desktop(state, message.sid, &session);
            }
            if state
                .agent_controls
                .existing(&id)
                .ok()
                .flatten()
                .is_none_or(|op| !matches!(op.status.as_str(), "accepted" | "dispatched"))
                || tokio::time::Instant::now() >= deadline
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }
    match state.agent_controls.wait_for_submission(&id).await {
        Ok(operation) => receipt(&operation),
        Err(_) => InterjectResult::rejected("controlUnknown"),
    }
}

pub(super) fn valid_result(rec: &Value, id: &str) -> bool {
    !id.is_empty()
        && rec["lastTurnResult"]["id"] == id
        && matches!(
            rec["lastTurnResult"]["outcome"].as_str(),
            Some("failed" | "incomplete")
        )
}

fn apply(
    state: &Arc<ServerState>,
    kind: AgentKind,
    sid: &str,
    event: LifecycleEvent,
    observation: Observation,
    cwd: Option<String>,
) {
    let started_control = state
        .agent_controls
        .started(
            kind,
            sid,
            observation.control_operation_id.as_deref(),
            observation.turn_id.as_deref(),
        )
        .and_then(|operation| operation.started_control());
    let applied = state
        .agents
        .apply_observed(kind, event, sid, None, cwd, 0, Some(&observation));
    if applied.changed {
        state.agents.persist();
    }
    if applied.accepted && event == LifecycleEvent::TurnStart {
        state.agent_controls.cancel_unsubmitted(kind, sid);
    }
    if applied.accepted || event == LifecycleEvent::TurnEnd && started_control.is_some() {
        if let Some(gateway) = ios_gateway(state) {
            gateway.agent_event(AgentEventInfo {
                agent_key: ios_agents::agent_key(kind, sid),
                event: ios_agents::event_kind(event),
                failed: observation
                    .result
                    .as_ref()
                    .is_some_and(|result| result.outcome == TurnOutcome::Failed),
                result: state.agents.last_turn_result(kind, sid),
                started_control,
            });
        }
    }
}

pub(super) fn observe_desktop(
    state: &Arc<ServerState>,
    sid: &str,
    session: &crate::codex_desktop::Session,
) {
    if !session.connected {
        return;
    }
    let Some(meta) = session.meta.as_ref() else {
        return;
    };
    let turns = crate::codex_desktop::protocol::turns(&session.state);
    if let Some(turn) = turns.last() {
        if let Some((event, mut observation)) = desktop_observation(turn) {
            let ids: Vec<&str> = crate::codex_desktop::protocol::entities(&turn["items"])
                .into_iter()
                .filter(|item| item["type"] == "userMessage")
                .flat_map(|item| [item["id"].as_str(), item["clientUserMessageId"].as_str()])
                .flatten()
                .collect();
            observation.control_operation_id =
                state
                    .agent_controls
                    .message_operation(AgentKind::Codex, sid, &ids);
            if event == LifecycleEvent::TurnEnd {
                // The canonical latest turn is also proof of a fast new round missed between polls.
                let start = Observation {
                    turn_id: observation.turn_id.clone(),
                    control_operation_id: observation.control_operation_id.clone(),
                    ..Observation::default()
                };
                apply(
                    state,
                    AgentKind::Codex,
                    sid,
                    LifecycleEvent::TurnStart,
                    start,
                    Some(meta.cwd.clone()),
                );
            }
            apply(
                state,
                AgentKind::Codex,
                sid,
                event,
                observation,
                Some(meta.cwd.clone()),
            );
        }
    } else {
        state
            .agents
            .update_desktop(sid, &meta.title, &meta.cwd, session.active());
    }
    state.agents.annotate_desktop(sid, &meta.title, &meta.cwd);
}

fn desktop_observation(turn: &Value) -> Option<(LifecycleEvent, Observation)> {
    let id = turn["turnId"].as_str()?.to_owned();
    let status = turn["status"].as_str()?;
    let event = if status == "inProgress" {
        LifecycleEvent::TurnStart
    } else {
        LifecycleEvent::TurnEnd
    };
    let outcome = match status {
        "inProgress" => None,
        "completed" => Some(TurnOutcome::Completed),
        "failed" => Some(TurnOutcome::Failed),
        "interrupted" => Some(TurnOutcome::Interrupted),
        _ => Some(TurnOutcome::Unknown),
    };
    let result = outcome.map(|outcome| TurnResult {
        source: Some("codexDesktop".into()),
        id: String::new(),
        outcome,
        turn_id: Some(id.clone()),
        reason: (outcome == TurnOutcome::Failed).then(|| "runtime_error".into()),
        summary: turn["error"]["message"]
            .as_str()
            .map(|s| s.chars().filter(|c| !c.is_control()).take(320).collect()),
        at: 0,
    });
    Some((
        event,
        Observation {
            turn_id: Some(id),
            result,
            ..Observation::default()
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_results_are_independent_of_idle_and_never_assume_success() {
        for (status, outcome) in [
            ("failed", TurnOutcome::Failed),
            ("interrupted", TurnOutcome::Interrupted),
            ("future", TurnOutcome::Unknown),
        ] {
            let (event, observation) =
                desktop_observation(&json!({"turnId":"t","status":status})).unwrap();
            assert_eq!(event, LifecycleEvent::TurnEnd);
            assert_eq!(observation.result.unwrap().outcome, outcome);
        }
        assert!(desktop_observation(&json!({"status":"completed"})).is_none());
    }
    #[test]
    fn continuation_binds_the_result_identity_and_attention_outcome() {
        let mut rec = json!({"lastTurnResult":{"id":"r", "outcome":"failed"}});
        assert!(valid_result(&rec, "r"));
        assert!(!valid_result(&rec, "old"));
        rec["lastTurnResult"]["outcome"] = json!("interrupted");
        assert!(!valid_result(&rec, "r"));
    }
}
