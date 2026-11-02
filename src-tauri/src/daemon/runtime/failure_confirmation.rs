//! Daemon-owned final-failure cards. Passive hooks report without waiting for a decision.

use super::*;
use crate::agents::result::TurnOutcome;
use std::collections::{HashMap, VecDeque};

#[derive(Default)]
pub(super) struct Cards {
    inner: Mutex<Inner>,
}

#[derive(Default)]
struct Inner {
    active: HashMap<(AgentKind, String), (String, Arc<tokio::sync::Notify>)>,
    seen: VecDeque<String>,
}

impl Cards {
    fn begin(&self, kind: AgentKind, sid: &str, result: &str) -> Option<Arc<tokio::sync::Notify>> {
        let mut inner = self.inner.lock().unwrap();
        let identity = format!("{}:{sid}:{result}", kind.as_str());
        if inner.seen.contains(&identity)
            || inner.active.len() >= 256 && !inner.active.contains_key(&(kind, sid.into()))
        {
            return None;
        }
        let cancel = Arc::new(tokio::sync::Notify::new());
        if let Some((_, previous)) = inner
            .active
            .insert((kind, sid.into()), (result.into(), cancel.clone()))
        {
            previous.notify_one();
        }
        inner.seen.push_back(identity);
        if inner.seen.len() > 512 {
            inner.seen.pop_front();
        }
        Some(cancel)
    }
    fn cancel(&self, kind: AgentKind, sid: &str) {
        if let Some((_, cancel)) = self
            .inner
            .lock()
            .unwrap()
            .active
            .remove(&(kind, sid.into()))
        {
            cancel.notify_one();
        }
    }
    fn finish(&self, kind: AgentKind, sid: &str, result: &str) {
        let mut inner = self.inner.lock().unwrap();
        let key = (kind, sid.into());
        if inner.active.get(&key).is_some_and(|(id, _)| id == result) {
            inner.active.remove(&key);
        }
    }
}

fn current(state: &ServerState, kind: AgentKind, sid: &str, result: &str) -> bool {
    state
        .agents
        .snapshot()
        .as_array()
        .into_iter()
        .flatten()
        .any(|record| {
            record["kind"] == kind.as_str()
                && record["sessionId"] == sid
                && record["state"] == "idle"
                && record["lastTurnResult"]["id"] == result
                && record["lastTurnResult"]["outcome"] == "failed"
        })
}

pub(super) fn enabled(kind: AgentKind, sid: &str) -> bool {
    if !crate::development::session::requires_test_profile() {
        return crate::integrations::agent_stop::active_in_current_mode(kind);
    }
    // The project daemon has no per-caller profile environment. Never resolve its modes by
    // inspecting the user's ordinary integration files.
    let Some(profile) =
        crate::development::session::for_native(kind, sid).filter(|profile| profile.prepared)
    else {
        return false;
    };
    if !crate::integrations::agent_stop::enabled(kind) {
        return false;
    }
    let relative = match kind {
        AgentKind::Claude => "settings.json",
        AgentKind::Codex => "hooks.json",
        AgentKind::Grok => "hooks/askhuman-lifecycle.json",
        AgentKind::Pi => "extensions/askhuman/index.ts",
        _ => return false,
    };
    let file = profile.directory.join(relative);
    if !file
        .metadata()
        .is_ok_and(|metadata| metadata.len() <= 1024 * 1024)
    {
        return false;
    }
    let Ok(source) = std::fs::read_to_string(file) else {
        return false;
    };
    if kind == AgentKind::Pi {
        return source
            .split("ASKHUMAN_CONFIG_BEGIN */")
            .nth(1)
            .and_then(|text| text.split("/* ASKHUMAN_CONFIG_END").next())
            .and_then(|text| serde_json::from_str::<serde_json::Value>(text.trim()).ok())
            .is_some_and(|config| config["stop"] == true);
    }
    source.contains(crate::integrations::agent_stop::MARKER)
        || source.contains(crate::integrations::agent_stop::FAILURE_MARKER)
        || source.contains(crate::integrations::agent_lifecycle::MARKER)
}

/// Only accepted observations enter here; an old round cannot cancel a new round's card.
pub(super) fn observe(state: &Arc<ServerState>, kind: AgentKind, sid: &str, event: LifecycleEvent) {
    if matches!(
        event,
        LifecycleEvent::TurnStart | LifecycleEvent::SessionEnd
    ) {
        state.failure_confirmations.cancel(kind, sid);
        return;
    }
    if event != LifecycleEvent::TurnEnd
        || state.draining.load(Ordering::SeqCst)
        || !enabled(kind, sid)
        || !crate::development::session::native_binding_allowed(kind, sid)
        || kind == AgentKind::Codex && crate::agents::delegated::marker_silenced(sid).is_some()
    {
        return;
    }
    let Some(result) = state
        .agents
        .last_turn_result(kind, sid)
        .filter(|r| r.outcome == TurnOutcome::Failed && !r.id.is_empty())
    else {
        return;
    };
    if !current(state, kind, sid, &result.id) {
        return;
    }
    let Some(cancel) = state.failure_confirmations.begin(kind, sid, &result.id) else {
        return;
    };
    let snapshot = state.agents.snapshot();
    let Some(record) = snapshot
        .as_array()
        .into_iter()
        .flatten()
        .find(|r| r["kind"] == kind.as_str() && r["sessionId"] == sid)
    else {
        state.failure_confirmations.finish(kind, sid, &result.id);
        return;
    };
    let availability = super::agent_control::can_send(state, record);
    let can_continue = availability.ok;
    let project = record["cwd"]
        .as_str()
        .map(|cwd| crate::project::detect_from(std::path::Path::new(cwd)))
        .unwrap_or_default();
    let lang = Lang::current();
    let mut task = crate::agents::failure::task(kind, sid, &project, &result, can_continue, lang);
    if !can_continue {
        let reason = match (kind, availability.reason.as_deref()) {
            (AgentKind::Pi, Some("idleUnsupported")) => Some("nativeUnavailable"),
            (_, reason) => reason,
        };
        crate::agents::failure::unavailable(&mut task, reason, lang);
    }
    if crate::development::verification::decorate_native_task(&mut task, kind, sid).is_err() {
        state.failure_confirmations.finish(kind, sid, &result.id);
        return;
    }
    let key = ios_agents::agent_key(kind, sid);
    if let Some(gateway) = ios_gateway(state) {
        gateway.hold_failure(&key, &result.id);
    }
    let state = state.clone();
    let sid = sid.to_string();
    tokio::spawn(async move {
        let accepted = Mutex::new(None::<String>);
        let info = task.message.development.clone();
        let ask = async {
            while state.registry.waiting_for_session(&sid) {
                tokio::time::sleep(Duration::from_millis(250)).await;
            }
            crate::client::internal_ask(task, &accepted).await
        };
        let answer = tokio::select! {
            _ = cancel.notified() => None,
            answer = tokio::time::timeout(Duration::from_secs(86400), ask) => answer.ok(),
            _ = async {
                loop {
                    tokio::time::sleep(Duration::from_secs(2)).await;
                    if !current(&state,kind,&sid,&result.id) || !enabled(kind,&sid) { break; }
                }
            } => None,
        };
        if let Some(info) = info.filter(|info| !info.relay) {
            match &answer {
                Some((stdout, code)) => {
                    let _ = crate::development::verification::complete(&info, stdout, *code);
                }
                None => {
                    let _ = crate::development::verification::interrupted(&info.worktree, &info.id);
                }
            }
        }
        if current(&state, kind, &sid, &result.id)
            && (accepted.lock().unwrap().is_none()
                || answer.as_ref().is_some_and(|(_, code)| *code == 3))
        {
            if let Some(gateway) = ios_gateway(&state) {
                gateway.release_failure(&key, &result.id);
            }
        }
        if accepted.lock().unwrap().is_some()
            && answer.as_ref().is_none_or(|(_, code)| *code != 3)
            && current(&state, kind, &sid, &result.id)
        {
            if let Some(gateway) = ios_gateway(&state) {
                gateway.consume_failure(&key, &result.id);
            }
        }
        if let Some((stdout, 0)) = answer {
            if let Some(instruction) = crate::agents::failure::instruction(&stdout, can_continue) {
                if current(&state, kind, &sid, &result.id) {
                    let request_id = accepted.lock().unwrap().clone();
                    let channel_is_ios = serde_json::from_str::<serde_json::Value>(&stdout)
                        .ok()
                        .is_some_and(|value| value["channel"] == "ios");
                    let phone = if channel_is_ios {
                        if let (Some(gateway), Some(request_id)) = (ios_gateway(&state), request_id)
                        {
                            gateway
                                .answer_phone(&request_id)
                                .await
                                .unwrap_or_else(|| "failure-confirmation".into())
                        } else {
                            "failure-confirmation".into()
                        }
                    } else {
                        "failure-confirmation".into()
                    };
                    let response = super::agent_control::send(
                        &state,
                        super::agent_control::Message {
                            phone: &phone,
                            message_id: &result.id,
                            kind,
                            sid: &sid,
                            text: "",
                            files: vec![],
                            expected_result_id: Some(result.id.clone()),
                            continuation_instruction: Some(&instruction),
                        },
                    )
                    .await;
                    if response.status != "delivered" {
                        let mut feedback = crate::agents::failure::task(
                            kind, &sid, &project, &result, false, lang,
                        );
                        feedback.native_request_id = Some(format!("failure-control:{}", result.id));
                        feedback.questions[0].predefined_options[0].text = match lang {
                            Lang::Zh => "知道了",
                            Lang::En => "Got it",
                        }
                        .into();
                        feedback.message.text = match lang {
                            Lang::Zh=>"未能确认接续已开始。请先检查原会话，再决定是否重试；不要连续重复提交。",
                            Lang::En=>"Continuation could not be confirmed. Check the original chat before retrying; do not repeatedly submit the same instruction.",
                        }.into();
                        if let Some(detail) =
                            crate::agents::failure::control_reason(response.reason.as_deref(), lang)
                        {
                            feedback.message.text.push_str(&format!("\n\n{detail}"));
                        }
                        log(&format!(
                            "failure continuation agent={} reason={}",
                            kind.as_str(),
                            response.reason.as_deref().unwrap_or("controlUnknown")
                        ));
                        if crate::development::verification::decorate_native_task(
                            &mut feedback,
                            kind,
                            &sid,
                        )
                        .is_ok()
                        {
                            let feedback_info = feedback.message.development.clone();
                            let acceptance = Mutex::new(None);
                            let feedback_result = tokio::select! {
                                _ = cancel.notified() => None,
                                result = tokio::time::timeout(Duration::from_secs(86400),crate::client::internal_ask(feedback,&acceptance)) => result.ok(),
                            };
                            if let Some(info) = feedback_info.filter(|info| !info.relay) {
                                match feedback_result {
                                    Some((stdout, code)) => {
                                        let _ = crate::development::verification::complete(
                                            &info, &stdout, code,
                                        );
                                    }
                                    None => {
                                        let _ = crate::development::verification::interrupted(
                                            &info.worktree,
                                            &info.id,
                                        );
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        state.failure_confirmations.finish(kind, &sid, &result.id);
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn duplicate_results_and_old_cleanup_cannot_replace_a_new_card() {
        let cards = Cards::default();
        let old = cards.begin(AgentKind::Codex, "s", "r1").unwrap();
        assert!(cards.begin(AgentKind::Codex, "s", "r1").is_none());
        let new = cards.begin(AgentKind::Codex, "s", "r2").unwrap();
        tokio::time::timeout(Duration::from_millis(10), old.notified())
            .await
            .unwrap();
        cards.finish(AgentKind::Codex, "s", "r1");
        cards.cancel(AgentKind::Codex, "s");
        tokio::time::timeout(Duration::from_millis(10), new.notified())
            .await
            .unwrap();
        assert!(cards.begin(AgentKind::Codex, "s", "r2").is_none());
    }
}
