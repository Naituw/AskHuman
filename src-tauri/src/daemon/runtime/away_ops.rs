//! Daemon-side glue of away mode and mute (`docs/specs/special-replies.md`): the automatic reply
//! to a question, the operations every surface sends, the directive that travels with an answer,
//! and the fan-out of a change. The decisions themselves live in `daemon::away`.

use super::*;
use crate::app::RenderOutcome;
use crate::cli::output;
use crate::daemon::away::{self, AutoReply, AwayOp, AwaySnapshot};

/// The state as every surface should see it: the engine's picture plus the escalated questions
/// that are still waiting for an answer.
pub(super) fn away_snapshot(state: &Arc<ServerState>) -> AwaySnapshot {
    let mut snapshot = state.away.snapshot();
    snapshot.escalated_waiting = state.registry.escalated_waiting() as u32;
    snapshot
}

/// Fan-out after the away state changed: the console and the menu bar rebuild their pictures.
pub(super) fn broadcast_away(state: &Arc<ServerState>) {
    broadcast_agents_state(state);
}

/// Applies one operation from a surface (tray, console, phone, CLI) and returns the new picture.
pub(super) fn apply_away_op(state: &Arc<ServerState>, op: &AwayOp) -> AwaySnapshot {
    let outcome = state.away.apply(op, now_ms());
    if outcome.changed {
        log(&format!("away state changed by {op:?}"));
        if let Some(key) = outcome.unmuted_session.as_deref() {
            queue_unmute_notice(state, key);
        }
        broadcast_away(state);
    }
    away_snapshot(state)
}

/// Tells a session it may use AskHuman again, through the interject queue only (spec D15): a
/// message sent through another channel could start a new billed turn. A session without an Agent
/// session id (MCP instance keys) has no queue and is simply no longer intercepted.
fn queue_unmute_notice(state: &Arc<ServerState>, session_key: &str) {
    let Some(session_id) = session_key.strip_prefix("sid:") else {
        return;
    };
    interject_append(state, session_id, away::UNMUTE_NOTICE, Vec::new());
}

/// The user asked this session to ask them (tray "Ask Me Now"): its next question is delivered
/// even while away (spec N2). Called for every appended interjection; only the fixed text counts.
pub(super) fn note_requested_ask(state: &Arc<ServerState>, session_id: &str, text: &str) {
    if text.trim() == away::QUICK_ASK_TEXT {
        state
            .away
            .arm_requested_ask(&away::sid_key(session_id), now_ms());
    }
}

/// An Agent session ended: its override and counters go away with it.
pub(super) fn session_ended(state: &Arc<ServerState>, session_id: &str) {
    if state.away.session_ended(&away::sid_key(session_id)) {
        broadcast_away(state);
    }
}

/// Periodic housekeeping (the 15 second agents tick): overrides of sessions nobody has heard of
/// for days are dropped, in case a session end was missed.
pub(super) fn sweep(state: &Arc<ServerState>) {
    let active: Vec<String> = state
        .agents
        .active_session_ids()
        .iter()
        .map(|id| away::sid_key(id))
        .collect();
    if state.away.sweep(&active, now_ms()) {
        broadcast_away(state);
    }
}

/// Answers a question on the user's behalf: the Agent gets the guidance as its result, the user is
/// not interrupted. Same wire path as a replayed answer (`Accepted` + `Final`, exit code 0). The
/// exchange goes into the reply history (spec D13); it never fires the `ask-received` hook and
/// never enters the replay cache (spec N4).
pub(super) async fn finish_auto_reply(
    state: &Arc<ServerState>,
    task: &TaskRequest,
    reply: AutoReply,
    w: &mut OwnedWriteHalf,
) {
    let request_id = uuid::Uuid::new_v4().to_string();
    let json = task.output_format == crate::models::OutputFormat::Json;
    let stdout = if json {
        output::auto_reply_json(reply.kind.channel(), &reply.text)
    } else {
        output::auto_reply_output(&reply.text)
    };
    record_auto_reply(state, task, &reply, &request_id);
    log(&format!(
        "request {request_id} answered automatically ({})",
        reply.kind.channel()
    ));
    let _ = ipc::write_msg(
        w,
        &ServerMsg::Accepted {
            request_id: request_id.clone(),
        },
    )
    .await;
    let _ = ipc::write_msg(
        w,
        &ServerMsg::Final {
            stdout,
            exit_code: 0,
        },
    )
    .await;
    // The counters moved.
    broadcast_away(state);
}

/// Reply-history entry of an automatic reply: the Agent's question and the text it received,
/// recorded under the `away` / `mute` channel so the history can single them out.
fn record_auto_reply(
    state: &Arc<ServerState>,
    task: &TaskRequest,
    reply: &AutoReply,
    request_id: &str,
) {
    let limit = state.config_snapshot().general.history_limit;
    if limit == 0 || !task.record_history {
        return;
    }
    let entry = auto_reply_history_entry(task, reply, request_id, crate::history::now_ms());
    crate::history::record(entry, limit);
}

fn auto_reply_history_entry(
    task: &TaskRequest,
    reply: &AutoReply,
    request_id: &str,
    timestamp_ms: i64,
) -> crate::history::HistoryEntry {
    crate::history::HistoryEntry {
        id: request_id.to_string(),
        timestamp_ms,
        project: task.project.clone(),
        source: task.source.clone(),
        agent_kind: task.agent_kind.clone(),
        agent_session_id: task.agent_session_id.clone(),
        mcp_instance_id: task.mcp_instance_id.clone(),
        channel: reply.kind.channel().to_string(),
        action: ChannelAction::Send,
        is_markdown: task.is_markdown,
        message: task.message.clone(),
        questions: task.questions.clone(),
        answers: vec![crate::history::HistoryAnswer {
            selected_options: Vec::new(),
            user_input: Some(reply.text.clone()),
            images: Vec::new(),
            files: Vec::new(),
        }],
    }
}

/// Applies the directive that came with an accepted answer and appends its guidance to what the
/// Agent receives (spec N3). A directive without a real answer behind it is dropped.
pub(super) fn finish_with_directive(
    state: &Arc<ServerState>,
    entry: &request::RequestEntry,
    outcome: RenderOutcome,
) -> RenderOutcome {
    let Some(directive) = entry.take_directive() else {
        return outcome;
    };
    if !entry.coordinator.answered() {
        return outcome;
    }
    let request = entry.request();
    if let Some(op) = directive.to_op(entry.session_key.as_deref()) {
        apply_away_op(state, &op);
    }
    let lang = Lang::resolve(&entry.show.lang);
    let text = directive.guidance(request.whats_next, lang);
    log(&format!(
        "request {} answered with a {:?} directive",
        entry.request_id, directive.kind
    ));
    let kind = match directive.kind {
        away::DirectiveKind::Away => "away",
        away::DirectiveKind::Mute => "mute",
    };
    RenderOutcome {
        stdout: output::append_status(
            &outcome.stdout,
            &text,
            request.output_format == crate::models::OutputFormat::Json,
            Some(kind),
        ),
        ..outcome
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::away::{AutoKind, Directive, DirectiveKind, Scope};

    fn task() -> TaskRequest {
        serde_json::from_value(serde_json::json!({
            "message": {"text": "hello"},
            "questions": [{"message": "Which one?", "predefinedOptions": []}],
            "isMarkdown": false,
            "source": "Claude Code",
            "lang": "en",
            "project": "/work/app",
            "agentKind": "claude",
            "agentSessionId": "abc"
        }))
        .unwrap()
    }

    #[test]
    fn an_automatic_reply_is_recorded_as_the_users_side_of_the_exchange() {
        let reply = AutoReply {
            kind: AutoKind::Away,
            text: "keep going".to_string(),
        };
        let entry = auto_reply_history_entry(&task(), &reply, "req-1", 42);
        assert_eq!(entry.id, "req-1");
        assert_eq!(entry.channel, "away");
        assert_eq!(entry.action, ChannelAction::Send);
        assert_eq!(entry.project, "/work/app");
        assert_eq!(entry.agent_session_id.as_deref(), Some("abc"));
        assert_eq!(entry.questions[0].message, "Which one?");
        assert_eq!(entry.answers[0].user_input.as_deref(), Some("keep going"));
        let mute = AutoReply {
            kind: AutoKind::Mute,
            text: "stop".to_string(),
        };
        assert_eq!(
            auto_reply_history_entry(&task(), &mute, "req-2", 1).channel,
            "mute"
        );
    }

    /// A task that never writes reply history (the tests must not touch the real one).
    fn quiet_task() -> TaskRequest {
        let mut task = task();
        task.record_history = false;
        task
    }

    fn answer(option: &str) -> ChannelResult {
        ChannelResult {
            action: ChannelAction::Send,
            answers: vec![crate::models::QuestionAnswer {
                selected_options: vec![option.to_string()],
                ..Default::default()
            }],
            source_channel_id: "popup".to_string(),
        }
    }

    #[tokio::test]
    async fn an_accepted_answer_applies_its_directive_and_carries_the_guidance() {
        let dir = tempfile::tempdir().unwrap();
        let state = test_state(dir.path());
        let (entry, mut rx) = state.registry.create(quiet_task(), None);
        entry.set_directive(Some(Directive {
            kind: DirectiveKind::Mute,
            scope: Scope::Session,
            note: None,
        }));
        entry.coordinator.submit(answer("A"));
        let outcome = rx.recv().await.expect("a rendered result");
        let out = finish_with_directive(&state, &entry, outcome);
        // The user's answer first, then the guidance in its own status block.
        assert!(
            out.stdout.starts_with("[selected_options]\nA"),
            "{}",
            out.stdout
        );
        assert!(out.stdout.contains("\n\n[status]\n"), "{}", out.stdout);
        assert!(out.stdout.contains("turned AskHuman off"), "{}", out.stdout);
        // And the state really changed: the session is muted from now on.
        assert!(matches!(
            state.away.effective(Some("sid:abc")),
            away::Effective::Mute { .. }
        ));
        assert!(entry.take_directive().is_none(), "a directive applies once");
    }

    #[tokio::test]
    async fn an_away_directive_turns_global_away_on_with_its_note() {
        let dir = tempfile::tempdir().unwrap();
        let state = test_state(dir.path());
        let (entry, mut rx) = state.registry.create(quiet_task(), None);
        entry.set_directive(Some(Directive {
            kind: DirectiveKind::Away,
            scope: Scope::Global,
            note: Some("no prod".to_string()),
        }));
        entry.coordinator.submit(answer("A"));
        let outcome = rx.recv().await.expect("a rendered result");
        let out = finish_with_directive(&state, &entry, outcome);
        assert!(out.stdout.contains("--escalate"), "{}", out.stdout);
        assert!(out.stdout.contains("no prod"), "{}", out.stdout);
        let snapshot = away_snapshot(&state);
        assert_eq!(snapshot.global.unwrap().note.as_deref(), Some("no prod"));
    }

    #[tokio::test]
    async fn a_directive_without_a_real_answer_is_dropped() {
        let dir = tempfile::tempdir().unwrap();
        let state = test_state(dir.path());
        let (entry, mut rx) = state.registry.create(quiet_task(), None);
        entry.set_directive(Some(Directive {
            kind: DirectiveKind::Mute,
            scope: Scope::Session,
            note: None,
        }));
        entry.coordinator.submit(ChannelResult::cancel("popup"));
        let outcome = rx.recv().await.expect("a rendered result");
        let before = outcome.stdout.clone();
        let out = finish_with_directive(&state, &entry, outcome);
        assert_eq!(out.stdout, before);
        assert!(state.away.snapshot().sessions.is_empty());
    }

    #[tokio::test]
    async fn json_answers_get_the_guidance_in_status_and_name_the_directive() {
        let dir = tempfile::tempdir().unwrap();
        let state = test_state(dir.path());
        let mut task = quiet_task();
        task.output_format = crate::models::OutputFormat::Json;
        let (entry, mut rx) = state.registry.create(task, None);
        entry.set_directive(Some(Directive {
            kind: DirectiveKind::Mute,
            scope: Scope::Session,
            note: None,
        }));
        entry.coordinator.submit(answer("A"));
        let outcome = rx.recv().await.expect("a rendered result");
        let out = finish_with_directive(&state, &entry, outcome);
        let value: serde_json::Value = serde_json::from_str(&out.stdout).unwrap();
        assert_eq!(value["action"], "answer");
        assert_eq!(value["directive"], "mute");
        assert!(value["status"]
            .as_str()
            .unwrap()
            .contains("turned AskHuman off"));
    }

    #[test]
    fn escalated_questions_are_counted_while_they_wait() {
        let dir = tempfile::tempdir().unwrap();
        let state = test_state(dir.path());
        assert_eq!(away_snapshot(&state).escalated_waiting, 0);
        let (plain, _plain_rx) = state.registry.create(quiet_task(), None);
        let (escalated, _escalated_rx) = state.registry.create_escalated(
            quiet_task(),
            None,
            Some(crate::models::EscalationReason::Asked),
        );
        assert!(plain.request().escalated.is_none());
        assert_eq!(
            escalated.request().escalated,
            Some(crate::models::EscalationReason::Asked)
        );
        assert_eq!(away_snapshot(&state).escalated_waiting, 1);
        state.registry.remove(&escalated.request_id);
        assert_eq!(away_snapshot(&state).escalated_waiting, 0);
    }

    #[test]
    fn leaving_mute_queues_the_unmute_notice_for_a_registered_session_only() {
        let dir = tempfile::tempdir().unwrap();
        let state = test_state(dir.path());
        let mute = |key: &str, setting| AwayOp::Session {
            session_key: key.to_string(),
            setting,
            note: None,
        };
        apply_away_op(&state, &mute("sid:abc", away::SessionSetting::Mute));
        apply_away_op(&state, &mute("mcp:inst", away::SessionSetting::Mute));
        assert_eq!(state.interject.pending_count("abc"), 0);
        apply_away_op(&state, &mute("sid:abc", away::SessionSetting::Follow));
        apply_away_op(&state, &mute("mcp:inst", away::SessionSetting::Follow));
        assert_eq!(state.interject.full_text("abc"), away::UNMUTE_NOTICE);
        assert_eq!(state.interject.pending_sessions(), vec!["abc".to_string()]);
    }

    #[test]
    fn the_trays_ask_me_now_interjection_arms_one_delivered_question() {
        let dir = tempfile::tempdir().unwrap();
        let state = test_state(dir.path());
        apply_away_op(&state, &AwayOp::Start { note: None });
        // Any other interjection leaves the session intercepted.
        note_requested_ask(&state, "abc", "please check the logs");
        assert!(matches!(
            state.away.judge_task(&task(), 10),
            away::Verdict::Reply(_)
        ));
        note_requested_ask(&state, "abc", away::QUICK_ASK_TEXT);
        assert_eq!(
            state.away.judge_task(&task(), 20),
            away::Verdict::Escalated(crate::models::EscalationReason::Requested)
        );
        assert!(matches!(
            state.away.judge_task(&task(), 30),
            away::Verdict::Reply(_)
        ));
    }

    #[test]
    fn a_session_end_clears_its_override() {
        let dir = tempfile::tempdir().unwrap();
        let state = test_state(dir.path());
        apply_away_op(
            &state,
            &AwayOp::Session {
                session_key: "sid:abc".to_string(),
                setting: away::SessionSetting::Mute,
                note: None,
            },
        );
        session_ended(&state, "abc");
        assert!(state.away.snapshot().sessions.is_empty());
    }

    #[test]
    fn directives_map_to_the_operation_that_makes_them_true() {
        let away_all = Directive {
            kind: DirectiveKind::Away,
            scope: Scope::Global,
            note: Some(" careful ".to_string()),
        };
        assert_eq!(
            away_all.to_op(Some("sid:a")),
            Some(AwayOp::Start {
                note: Some("careful".to_string())
            })
        );
        let away_here = Directive {
            kind: DirectiveKind::Away,
            scope: Scope::Session,
            note: None,
        };
        assert_eq!(
            away_here.to_op(Some("sid:a")),
            Some(AwayOp::Session {
                session_key: "sid:a".to_string(),
                setting: away::SessionSetting::Away,
                note: None
            })
        );
        // Without a session key a session-scoped away falls back to the global switch (spec N5).
        assert_eq!(away_here.to_op(None), Some(AwayOp::Start { note: None }));
        let mute = Directive {
            kind: DirectiveKind::Mute,
            scope: Scope::Global,
            note: Some("ignored".to_string()),
        };
        assert_eq!(
            mute.to_op(Some("sid:a")),
            Some(AwayOp::Session {
                session_key: "sid:a".to_string(),
                setting: away::SessionSetting::Mute,
                note: None
            })
        );
        assert_eq!(mute.to_op(None), None);
    }

    #[test]
    fn directive_guidance_follows_the_kind_of_question_and_carries_the_note() {
        let away_dir = Directive {
            kind: DirectiveKind::Away,
            scope: Scope::Global,
            note: Some("no prod".to_string()),
        };
        let ask = away_dir.guidance(false, Lang::En);
        assert!(ask.contains("--escalate") && ask.contains("no prod"));
        assert!(away_dir.guidance(true, Lang::En).contains("--whats-next"));
        let mute = Directive {
            kind: DirectiveKind::Mute,
            scope: Scope::Session,
            note: None,
        };
        assert!(mute.guidance(false, Lang::Zh).contains("关闭 AskHuman"));
    }
}
