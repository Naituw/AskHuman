//! One-shot phone messages and native proof that a new turn actually started.
use super::*;
use crate::agents::control::{ConsoleMessage, Operation};
use crate::agents::result::{Observation, TurnOutcome, TurnResult};
use crate::ios::gateway::{AgentEventInfo, InterjectResult};
use crate::models::FileAttachment;
use serde_json::{json, Value};

/// The continuation instruction of a failure card. The `--show-last` pointer stays neutral on
/// purpose: it does not claim anything about what the Agent was doing when the turn stopped.
pub(super) const CONTINUE_PROMPT: &str = "Inspect why the previous attempt stopped and what has already been completed. Continue the current task from its existing progress, without repeating successful operations. If you cannot proceed, explain the blocker and ask for the specific help you need. If needed, you can run `AskHuman --show-last` to see the latest AskHuman question and the answer given to it.";

/// Cursor sends neither `stop` nor `sessionEnd` once it gives up on a turn after exhausting its
/// retries, so the turn would stay "working" until a backstop. Its structured log is the only
/// first-party record of that outcome: tail it read-only and feed the failure into the registry
/// like any other failed `TurnEnd`. This never talks to Cursor and never starts a request
/// (`docs/specs/agent-failure-inbox.md` §4).
pub(super) async fn watch_cursor_log(state: Arc<ServerState>) {
    use crate::agents::cursor_log::{default_root, Tailer};
    // Project Dev Instances never act on the user's real Cursor.
    if crate::development::session::requires_test_profile() {
        return;
    }
    let Some(root) = default_root() else {
        return;
    };
    let mut tailer: Option<Tailer> = None;
    loop {
        tokio::time::sleep(Duration::from_secs(5)).await;
        // Following the log is a few `stat` calls per poll, and only the switch can ask for it. A
        // follower dropped while the switch is off restarts at the end of the logs when it is on
        // again: what happened meanwhile is not news.
        if !crate::integrations::agent_stop::enabled(AgentKind::Cursor) {
            tailer = None;
            continue;
        }
        let mut current = tailer.take().unwrap_or_else(|| Tailer::new(root.clone()));
        let polled = tokio::task::spawn_blocking(move || {
            let now = std::time::Instant::now();
            let failures = current.poll(now);
            let malformed = current.take_malformed(now);
            (current, failures, malformed)
        })
        .await;
        let Ok((current, failures, malformed)) = polled else {
            continue;
        };
        tailer = Some(current);
        if let Some(count) = malformed {
            log(&format!(
                "cursor log: {count} unreadable agent.turn.outcome line(s) in the last hour; the Cursor log format may have changed"
            ));
        }
        for failure in failures {
            apply_cursor_failure(&state, &failure);
        }
    }
}

/// One failed turn from Cursor's log, applied as a failed `TurnEnd` of a session the hooks already
/// registered.
fn apply_cursor_failure(state: &Arc<ServerState>, failure: &crate::agents::cursor_log::Failure) {
    let sid = failure.conversation_id.as_str();
    // The log names every conversation Cursor ran, including ones this daemon never heard of
    // (subagents, other windows, sessions from before the daemon started). Only a session the
    // hooks registered can have a turn to end, and this source must not invent sessions.
    if !state.agents.knows_session(AgentKind::Cursor, sid)
        || !crate::development::session::native_binding_allowed(AgentKind::Cursor, sid)
        || !failure_confirmation::enabled(AgentKind::Cursor, sid)
    {
        return;
    }
    log(&format!(
        "cursor log: failed turn session={} retries={}",
        sid.chars().take(8).collect::<String>(),
        failure
            .retries
            .map_or_else(|| "?".to_string(), |retries| retries.to_string())
    ));
    let observation = Observation {
        turn_id: Some(failure.request_id.clone()),
        result: Some(TurnResult {
            source: Some("cursorLog".into()),
            id: String::new(),
            outcome: TurnOutcome::Failed,
            turn_id: Some(failure.request_id.clone()),
            reason: Some("runtime_error".into()),
            summary: failure.summary(Lang::current()),
            at: 0,
        }),
        ..Observation::default()
    };
    apply(
        state,
        AgentKind::Cursor,
        sid,
        LifecycleEvent::TurnEnd,
        observation,
        None,
    );
    broadcast_agents_state(state);
    state.watch.notify.notify_one();
}

pub(super) async fn watch_codex_cli(state: Arc<ServerState>) {
    let mut observer = crate::agents::codex_cli::Observer::default();
    loop {
        let enabled = crate::integrations::agent_stop::enabled(AgentKind::Codex)
            && crate::integrations::agent_launch::codex_version().is_some_and(|v| v >= (0, 163, 0));
        let snapshot = state.agents.snapshot();
        let candidates: Vec<_> = snapshot
            .as_array()
            .into_iter()
            .flatten()
            .filter(|record| {
                enabled
                    && record["kind"] == "codex"
                    && record["state"] == "working"
                    && !crate::codex_desktop::is_desktop_session(record)
            })
            .filter_map(|record| {
                let sid = record["sessionId"].as_str()?;
                let turn = record["activeTurnId"].as_str()?;
                (crate::development::session::native_binding_allowed(AgentKind::Codex, sid)
                    && failure_confirmation::enabled(AgentKind::Codex, sid))
                .then(|| (sid.to_string(), turn.to_string()))
            })
            .take(32)
            .collect();
        if candidates.is_empty() {
            observer.close();
        } else if let Some(program) =
            crate::integrations::agent_launch::readiness(AgentKind::Codex).executable
        {
            for (sid, turn) in candidates {
                let home = crate::development::session::for_native(AgentKind::Codex, &sid)
                    .map(|profile| profile.directory)
                    .unwrap_or_else(crate::paths::codex_dir);
                if let Some(observation) = observer.failed_turn(&program, &home, &sid, &turn).await
                {
                    apply(
                        &state,
                        AgentKind::Codex,
                        &sid,
                        LifecycleEvent::TurnEnd,
                        observation,
                        None,
                    );
                    broadcast_agents_state(&state);
                    state.watch.notify.notify_one();
                }
            }
        }
        tokio::time::sleep(Duration::from_secs(5)).await;
    }
}

pub(super) fn operation_id(phone: &str, message: &str) -> String {
    use sha2::{Digest, Sha256};
    let hash = Sha256::digest(serde_json::to_vec(&["ios-interject", phone, message]).unwrap());
    let mut bytes = [0; 16];
    bytes.copy_from_slice(&hash[..16]);
    uuid::Uuid::from_bytes(bytes).to_string()
}

/// What the daemon asks of Cursor Desktop's message bridge. Everything around it (which sessions
/// may be messaged, how a refusal reads, what the registry does afterwards) is tested against a
/// stand-in, so no test ever reaches a real Cursor.
#[allow(async_fn_in_trait)]
pub(super) trait CursorChat {
    /// Whether the chat can be messaged at all, judged from files alone (never by attaching).
    fn available(&self, session_id: &str) -> bool;
    async fn send(
        &self,
        session_id: &str,
        id: &str,
        text: &str,
    ) -> crate::cursor_desktop::MessageResult;
}

/// The real bridge.
pub(super) struct LiveCursor;

impl CursorChat for LiveCursor {
    fn available(&self, session_id: &str) -> bool {
        crate::cursor_desktop::messages_available(session_id)
    }

    async fn send(
        &self,
        session_id: &str,
        id: &str,
        text: &str,
    ) -> crate::cursor_desktop::MessageResult {
        crate::cursor_desktop::send_message(session_id, id, text).await
    }
}

/// What the journal records when no Cursor window has the chat loaded. The bridge says `notFound`
/// for that, but to every caller `notFound` means "the registry does not know this session",
/// which is not what happened, so the journal gets a word of its own and each caller says it in
/// its own way: [`for_phone`], the failure card's text, [`console_refusal`].
pub(super) const CHAT_NOT_LOADED: &str = "chatNotLoaded";

fn cursor_refusal(reason: &str) -> &str {
    match reason {
        "notFound" => CHAT_NOT_LOADED,
        other => other,
    }
}

/// The phone's app knows no `chatNotLoaded`: "the original Agent is unavailable" is the nearest
/// thing it can say truthfully.
pub(super) fn for_phone(mut result: InterjectResult) -> InterjectResult {
    if result.reason.as_deref() == Some(CHAT_NOT_LOADED) {
        result.reason = Some("nativeUnavailable".into());
    }
    result
}

/// How long an answer about a Cursor chat is trusted when it only feeds a snapshot or a button.
/// Each answer reads Cursor's database, and snapshots follow every Agent event. Sending always
/// asks again, so a stale answer can at worst show a composer that then says it is unavailable.
const CHAT_ANSWER_TTL: Duration = Duration::from_secs(20);

static CHAT_ANSWERS: std::sync::LazyLock<
    std::sync::Mutex<std::collections::HashMap<String, (std::time::Instant, bool)>>,
> = std::sync::LazyLock::new(Default::default);

/// [`CursorChat::available`] with the answer remembered for [`CHAT_ANSWER_TTL`].
pub(super) fn cached_chat(session_id: &str) -> bool {
    let now = std::time::Instant::now();
    let answers = CHAT_ANSWERS.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(&(at, ok)) = answers.get(session_id) {
        if now.duration_since(at) < CHAT_ANSWER_TTL {
            return ok;
        }
    }
    drop(answers);
    // The lookup reads a database: never hold the lock across it.
    let ok = LiveCursor.available(session_id);
    let mut answers = CHAT_ANSWERS.lock().unwrap_or_else(|e| e.into_inner());
    if answers.len() > 256 {
        answers.retain(|_, (at, _)| now.duration_since(*at) < CHAT_ANSWER_TTL);
    }
    answers.insert(session_id.into(), (now, ok));
    ok
}

pub(super) fn can_send(state: &Arc<ServerState>, rec: &Value) -> crate::ios::wire::CanInterject {
    can_send_with(state, rec, &cached_chat)
}

/// `has_chat` says whether a Cursor Desktop chat exists to be messaged; it is asked at most once,
/// and only for the states that need it.
pub(super) fn can_send_with(
    state: &Arc<ServerState>,
    rec: &Value,
    has_chat: &dyn Fn(&str) -> bool,
) -> crate::ios::wire::CanInterject {
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
    let chat = std::cell::OnceCell::new();
    let cursor_chat = || *chat.get_or_init(|| kind == AgentKind::Cursor && has_chat(sid));
    // A Cursor Desktop chat has no process whose exit would end it. "Ended" is the registry's
    // verdict after an hour without activity, so while the chat itself is still there it can be
    // continued like an idle one. Every other kind has an end that means it.
    if rec["state"] == "ended" && !cursor_chat() && !retained_result(rec) {
        return refuse("ended");
    }
    if state.agent_controls.pending(kind, sid) {
        return refuse("controlStarting");
    }
    if crate::codex_desktop::is_desktop_session(rec) {
        let mut current = rec.clone();
        if current["state"] == "ended" && retained_result(rec) {
            current["state"] = "idle".into();
        }
        return ios_agents::can_interject(&current);
    }
    if rec["state"] == "working" {
        return ios_agents::can_interject(rec);
    }
    let available = match kind {
        AgentKind::Cursor => cursor_chat(),
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
    pub continuation_instruction: Option<&'a str>,
}

pub(super) async fn send(state: &Arc<ServerState>, message: Message<'_>) -> InterjectResult {
    send_with(state, message, &LiveCursor).await
}

pub(super) async fn send_with(
    state: &Arc<ServerState>,
    message: Message<'_>,
    cursor: &impl CursorChat,
) -> InterjectResult {
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
    // An "ended" Cursor Desktop chat is one the registry gave up on after an hour of silence;
    // `can_send_with` below decides whether the chat itself is still there to be continued.
    let sendable = rec["state"] == "idle"
        || (rec["state"] == "ended"
            && (message.kind == AgentKind::Cursor
                || message
                    .expected_result_id
                    .as_deref()
                    .is_some_and(|id| valid_result(rec, id) && retained_result(rec))));
    if !sendable
        || state.registry.waiting_for_session(message.sid)
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
    let can = can_send_with(state, rec, &|sid| cursor.available(sid));
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
            match message
                .continuation_instruction
                .filter(|text| !text.trim().is_empty())
            {
                Some(text) => format!("{CONTINUE_PROMPT}\n\nUser instruction:\n{text}"),
                None => CONTINUE_PROMPT.into(),
            }
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
            match cursor.send(message.sid, &id, &text).await {
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
                    let _ = state.agent_controls.reject(&id, cursor_refusal(&reason));
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

/// What the journal records as the sender of a message typed in the Mac's own console. No phone
/// ever has this id, so nothing in the phone gateway reacts to such a message.
const CONSOLE_SENDER: &str = "mac-console";
/// The same ceiling as a message from the phone.
const CONSOLE_MAX_CHARS: usize = 20_000;

/// Marks the records of the console's snapshot that the console may message directly:
/// `canMessage: true` on a chat that is idle, or that the registry has called ended, and that
/// [`can_send_with`] says can take a message. The console shows its composer from this flag alone.
pub(super) fn mark_messageable(
    state: &Arc<ServerState>,
    snapshot: &mut Value,
    has_chat: &dyn Fn(&str) -> bool,
) {
    let Some(records) = snapshot.as_array_mut() else {
        return;
    };
    for rec in records.iter_mut() {
        let eligible = rec["kind"] == AgentKind::Cursor.as_str()
            && matches!(rec["state"].as_str(), Some("idle" | "ended"))
            && rec["waitingRequestId"]
                .as_str()
                .is_none_or(|id| id.is_empty());
        if eligible && can_send_with(state, rec, has_chat).ok {
            rec["canMessage"] = Value::Bool(true);
        }
    }
}

/// Words a refusal for the console. All of them mean nothing was sent.
fn console_refusal(reason: &str, lang: crate::i18n::Lang) -> String {
    use crate::i18n::Lang::{En, Zh};
    match (reason, lang) {
        (CHAT_NOT_LOADED, Zh) => "Cursor 里没有打开这个聊天，没有发送。请先在 Cursor 里打开它，再试一次。".into(),
        (CHAT_NOT_LOADED, En) => "Cursor has not opened this chat in any window, so nothing was sent. Open it in Cursor and try again.".into(),
        ("nativeUnavailable" | "idleUnsupported", Zh) => "没能把消息交给 Cursor，没有发送。请确认只开了一个 Cursor、它正在运行，并且这个聊天只在一个窗口里打开。".into(),
        ("nativeUnavailable" | "idleUnsupported", En) => "Could not hand the message to Cursor, so nothing was sent. Make sure a single Cursor is running and this chat is open in only one window.".into(),
        ("stateChanged" | "resultChanged", Zh) => "这个聊天此刻不空闲（正在运行、等你确认，或有排队的消息），没有发送。".into(),
        ("stateChanged" | "resultChanged", En) => "This chat is not idle right now (it is running, waiting for you, or has a queued message), so nothing was sent.".into(),
        ("controlStarting", Zh) => "上一条消息还在等 Cursor 开始运行，请稍候再发。".into(),
        ("controlStarting", En) => "The previous message is still waiting for Cursor to start. Try again in a moment.".into(),
        ("ended", Zh) => "这个会话已经结束，没有发送。".into(),
        ("ended", En) => "This session has ended, so nothing was sent.".into(),
        ("notFound", Zh) => "找不到这个会话，没有发送。".into(),
        ("notFound", En) => "This session cannot be found, so nothing was sent.".into(),
        ("empty", Zh) => "没有可发送的内容。".into(),
        ("empty", En) => "There is nothing to send.".into(),
        ("tooLong", Zh) => format!("消息太长（上限 {CONSOLE_MAX_CHARS} 字），没有发送。"),
        ("tooLong", En) => format!("The message is too long (limit {CONSOLE_MAX_CHARS} characters), so nothing was sent."),
        (other, Zh) => format!("没有发送（{other}）。"),
        (other, En) => format!("Nothing was sent ({other})."),
    }
}

/// The words for an answer that may or may not have reached the chat.
fn console_unknown(lang: crate::i18n::Lang) -> String {
    match lang {
        crate::i18n::Lang::Zh => "消息可能已经发出。请先检查 Cursor 里的原聊天，再决定要不要再发一条。".into(),
        crate::i18n::Lang::En => "The message may have been sent. Check the original chat in Cursor before sending another.".into(),
    }
}

/// A message typed in the Mac's console for a chat that is not working. The answer's `status` is
/// `accepted` (the chat took it and a new turn began), `rejected` (nothing was sent, and the id
/// is spent) or `unknown` (it may have been sent); `message` is what to tell the user. `Err` is
/// reserved for what the caller cannot act on.
pub(super) async fn send_from_console(
    state: &Arc<ServerState>,
    message: ConsoleMessage,
) -> Result<Value, String> {
    send_from_console_with(state, message, &LiveCursor, crate::i18n::Lang::current()).await
}

pub(super) async fn send_from_console_with(
    state: &Arc<ServerState>,
    message: ConsoleMessage,
    cursor: &impl CursorChat,
    lang: crate::i18n::Lang,
) -> Result<Value, String> {
    let rejected = |reason: &str| {
        Ok(
            json!({"status": "rejected", "reason": reason, "message": console_refusal(reason, lang)}),
        )
    };
    if message.id.is_empty() || message.id.len() > 128 {
        return Err("Invalid operation ID".into());
    }
    if message.text.trim().is_empty() && message.files.is_empty() {
        return rejected("empty");
    }
    if message.text.chars().count() > CONSOLE_MAX_CHARS {
        return rejected("tooLong");
    }
    let sent = send_with(
        state,
        Message {
            phone: CONSOLE_SENDER,
            message_id: &message.id,
            kind: message.kind,
            sid: &message.session_id,
            text: &message.text,
            files: message.files,
            expected_result_id: None,
            continuation_instruction: None,
        },
        cursor,
    )
    .await;
    match (sent.status, sent.reason.as_deref()) {
        ("delivered", _) => Ok(json!({"status": "accepted"})),
        (_, Some("controlUnknown" | "conflict" | "interrupted" | "desktopUnknown")) => {
            Ok(json!({"status": "unknown", "message": console_unknown(lang)}))
        }
        (_, reason) => rejected(reason.unwrap_or("stateChanged")),
    }
}

fn retained_result(rec: &Value) -> bool {
    rec["failureRetention"]["expiresMs"]
        .as_u64()
        .is_some_and(|expires| expires > now_ms())
        && rec["failureRetention"]["resultId"] == rec["lastTurnResult"]["id"]
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
    if state.closing.load(Ordering::SeqCst) {
        return;
    }
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
        failure_confirmation::observe(state, kind, sid, event);
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
    use crate::cursor_desktop::MessageResult;
    use crate::i18n::Lang;
    use std::sync::Mutex as StdMutex;

    /// A Cursor that exists only in the test: it records what it is asked to send and answers
    /// with whatever the test queued. Nothing here can reach a real Cursor.
    struct Chat {
        exists: bool,
        reply: StdMutex<Option<MessageResult>>,
        sent: StdMutex<Vec<(String, String, String)>>,
    }

    impl Chat {
        fn new(exists: bool, reply: Option<MessageResult>) -> Self {
            Self {
                exists,
                reply: StdMutex::new(reply),
                sent: Default::default(),
            }
        }
        fn sent(&self) -> Vec<(String, String, String)> {
            self.sent.lock().unwrap().clone()
        }
    }

    impl CursorChat for Chat {
        fn available(&self, _session_id: &str) -> bool {
            self.exists
        }
        async fn send(&self, session_id: &str, id: &str, text: &str) -> MessageResult {
            self.sent
                .lock()
                .unwrap()
                .push((session_id.into(), id.into(), text.into()));
            self.reply
                .lock()
                .unwrap()
                .take()
                .expect("the test queued no answer for this send")
        }
    }

    fn state(directory: &std::path::Path) -> Arc<ServerState> {
        let mut config = AppConfig::default();
        config.general.popup_prewarm = false;
        Arc::new(ServerState {
            failure_confirmations: failure_confirmation::Cards::default(),
            startup_fp: lifecycle::Fingerprint { size: 0, hash: 0 },
            exe_path: String::new(),
            restart_hold: Default::default(),
            started_at: 0,
            active: AtomicUsize::new(1),
            last_active: Mutex::new(Instant::now()),
            shutdown: Default::default(),
            draining: AtomicBool::new(true),
            exit_intent: Mutex::new(Default::default()),
            closing: AtomicBool::new(false),
            registry: RequestRegistry::new(),
            popup_focus: Mutex::new(PopupFocusArbiter::new()),
            popup_inbox: Mutex::new(Default::default()),
            dd_router: Default::default(),
            fs_router: Default::default(),
            tg_router: Default::default(),
            sl_router: Default::default(),
            config: Mutex::new(config),
            update: Mutex::new(Default::default()),
            agents: Arc::new(AgentRegistry::new()),
            agent_controls: crate::agents::control::ControlStore::new(directory.join("controls")),
            interject: crate::agents::interject::InterjectStore::new(),
            grok_bindings: Default::default(),
            agent_subs: Default::default(),
            tray_subs: Default::default(),
            active_channel: Mutex::new(None),
            inbound_listeners: Default::default(),
            warm_pool: Mutex::new(None),
            warm_spawning: AtomicBool::new(false),
            watch: Default::default(),
            select: Default::default(),
            pending_launches: Default::default(),
            gui_focus: Default::default(),
            last_waiting: Default::default(),
            replay: Mutex::new(crate::daemon::ask_dedup::ReplayCache::new()),
            ios: Mutex::new(None),
        })
    }

    /// A session the registry has already given up on, the way a Cursor Desktop chat ends after
    /// an hour of silence.
    fn ended(state: &Arc<ServerState>, kind: AgentKind, sid: &str) {
        for event in [LifecycleEvent::SessionStart, LifecycleEvent::SessionEnd] {
            state
                .agents
                .apply_event(kind, event, sid, None, Some("/tmp/project".into()), 0);
        }
    }

    fn record(state: &Arc<ServerState>, sid: &str) -> Value {
        state
            .agents
            .snapshot()
            .as_array()
            .unwrap()
            .iter()
            .find(|rec| rec["sessionId"] == sid)
            .cloned()
            .expect("the registry has no such session")
    }

    fn message<'a>(id: &'a str, sid: &'a str, text: &'a str) -> Message<'a> {
        Message {
            phone: "mac-console",
            message_id: id,
            kind: AgentKind::Cursor,
            sid,
            text,
            files: Vec::new(),
            expected_result_id: None,
            continuation_instruction: None,
        }
    }

    fn console(id: &str, sid: &str, text: &str) -> ConsoleMessage {
        ConsoleMessage {
            id: id.into(),
            kind: AgentKind::Cursor,
            session_id: sid.into(),
            text: text.into(),
            files: Vec::new(),
        }
    }

    #[test]
    fn an_ended_cursor_chat_can_be_messaged_while_the_chat_still_exists() {
        let directory = tempfile::tempdir().unwrap();
        let state = state(directory.path());
        ended(&state, AgentKind::Cursor, "chat");
        let rec = record(&state, "chat");
        assert_eq!(rec["state"], "ended");
        assert!(can_send_with(&state, &rec, &|_| true).ok);
        let gone = can_send_with(&state, &rec, &|_| false);
        assert!(!gone.ok);
        assert_eq!(gone.reason.as_deref(), Some("ended"));
    }

    #[test]
    fn only_cursor_chats_outlive_the_registrys_verdict() {
        let directory = tempfile::tempdir().unwrap();
        let state = state(directory.path());
        for kind in [AgentKind::Claude, AgentKind::Codex, AgentKind::Pi] {
            let sid = format!("other-{}", kind.as_str());
            ended(&state, kind, &sid);
            let rec = record(&state, &sid);
            // The lookup must not even be asked: it reads Cursor's database.
            let refused = can_send_with(&state, &rec, &|_| panic!("asked Cursor about {kind:?}"));
            assert!(!refused.ok);
            assert_eq!(refused.reason.as_deref(), Some("ended"), "{kind:?}");
        }
    }

    #[test]
    fn an_idle_cursor_session_with_no_chat_behind_it_is_a_cli_session() {
        let directory = tempfile::tempdir().unwrap();
        let state = state(directory.path());
        let rec = json!({"kind":"cursor","sessionId":"cli","state":"idle"});
        assert!(can_send_with(&state, &rec, &|_| true).ok);
        let refused = can_send_with(&state, &rec, &|_| false);
        assert_eq!(refused.reason.as_deref(), Some("idleUnsupported"));
    }

    #[test]
    fn a_second_message_waits_for_the_first_to_start() {
        let directory = tempfile::tempdir().unwrap();
        let state = state(directory.path());
        ended(&state, AgentKind::Cursor, "chat");
        state
            .agent_controls
            .begin(Operation {
                id: operation_id("mac-console", "earlier"),
                kind: AgentKind::Cursor,
                session_id: "chat".into(),
                phone_id: "mac-console".into(),
                generation: None,
                text: "first".into(),
                files: Vec::new(),
                expected_result_id: None,
                status: "queued".into(),
                reason: None,
                turn_id: None,
            })
            .unwrap();
        let refused = can_send_with(&state, &record(&state, "chat"), &|_| true);
        assert_eq!(refused.reason.as_deref(), Some("controlStarting"));
    }

    #[test]
    fn the_console_snapshot_flags_exactly_the_chats_it_can_message() {
        let directory = tempfile::tempdir().unwrap();
        let state = state(directory.path());
        let mut snapshot = json!([
            {"kind":"cursor","sessionId":"idle","state":"idle"},
            {"kind":"cursor","sessionId":"ended","state":"ended"},
            {"kind":"cursor","sessionId":"gone","state":"ended"},
            {"kind":"cursor","sessionId":"busy","state":"working"},
            {"kind":"cursor","sessionId":"asking","state":"idle","waitingRequestId":"r1"},
            {"kind":"claude","sessionId":"claude","state":"idle"},
            {"kind":"codex","sessionId":"codex","state":"ended"},
        ]);
        mark_messageable(&state, &mut snapshot, &|sid| sid != "gone");
        let flagged: Vec<&str> = snapshot
            .as_array()
            .unwrap()
            .iter()
            .filter(|rec| rec["canMessage"] == true)
            .filter_map(|rec| rec["sessionId"].as_str())
            .collect();
        assert_eq!(flagged, ["idle", "ended"]);
    }

    #[tokio::test]
    async fn a_message_to_an_ended_chat_starts_a_turn_and_brings_the_session_back() {
        let directory = tempfile::tempdir().unwrap();
        let state = state(directory.path());
        ended(&state, AgentKind::Cursor, "chat");
        let chat = Chat::new(true, Some(MessageResult::Started("turn-1".into())));
        let sent = send_with(&state, message("m1", "chat", "hello"), &chat).await;
        assert_eq!(sent.status, "delivered");
        assert_eq!(sent.reason, None);
        assert_eq!(sent.started_control.unwrap().turn_id, "turn-1");
        let calls = chat.sent();
        assert_eq!(calls.len(), 1);
        assert_eq!(
            (calls[0].0.as_str(), calls[0].2.as_str()),
            ("chat", "hello")
        );
        let rec = record(&state, "chat");
        assert_eq!(rec["state"], "working");
        assert_eq!(rec["activeTurnId"], "turn-1");
    }

    #[tokio::test]
    async fn a_message_with_files_names_them_in_the_text_cursor_receives() {
        let directory = tempfile::tempdir().unwrap();
        let state = state(directory.path());
        ended(&state, AgentKind::Cursor, "chat");
        let chat = Chat::new(true, Some(MessageResult::Started("turn-1".into())));
        let mut with_file = message("m1", "chat", "look");
        with_file.files = vec![FileAttachment {
            path: "/tmp/a.png".into(),
            name: "a.png".into(),
            size: 3,
            is_image: true,
        }];
        assert_eq!(
            send_with(&state, with_file, &chat).await.status,
            "delivered"
        );
        assert_eq!(
            chat.sent()[0].2,
            "look\n\nAttached file: a.png (/tmp/a.png)"
        );
    }

    #[tokio::test]
    async fn a_chat_no_cursor_window_has_open_is_refused_in_each_callers_words() {
        let directory = tempfile::tempdir().unwrap();
        let state = state(directory.path());
        ended(&state, AgentKind::Cursor, "chat");
        let chat = Chat::new(true, Some(MessageResult::Rejected("notFound".into())));
        let refused = send_with(&state, message("m1", "chat", "hello"), &chat).await;
        assert_eq!(refused.status, "rejected");
        // The journal and the failure card keep the precise word ...
        assert_eq!(refused.reason.as_deref(), Some(CHAT_NOT_LOADED));
        // ... the phone, which has no such word, gets the nearest true one ...
        assert_eq!(
            for_phone(refused.clone()).reason.as_deref(),
            Some("nativeUnavailable")
        );
        // ... and the console says what to do.
        let answer = send_from_console_with(
            &state,
            console("m2", "chat", "hello"),
            &Chat::new(true, Some(MessageResult::Rejected("notFound".into()))),
            Lang::En,
        )
        .await
        .unwrap();
        assert_eq!(answer["status"], "rejected");
        assert_eq!(answer["reason"], CHAT_NOT_LOADED);
        assert!(answer["message"]
            .as_str()
            .unwrap()
            .contains("has not opened this chat"));
        // Nothing was sent, so the session is as it was.
        assert_eq!(record(&state, "chat")["state"], "ended");
    }

    #[tokio::test]
    async fn a_refusal_is_final_for_its_id_and_never_asks_cursor_twice() {
        let directory = tempfile::tempdir().unwrap();
        let state = state(directory.path());
        ended(&state, AgentKind::Cursor, "chat");
        let chat = Chat::new(true, Some(MessageResult::Rejected("busy".into())));
        let first = send_with(&state, message("m1", "chat", "hello"), &chat).await;
        let again = send_with(&state, message("m1", "chat", "hello"), &chat).await;
        assert_eq!(first.reason, again.reason);
        assert_eq!(chat.sent().len(), 1);
    }

    #[tokio::test]
    async fn an_answer_that_never_came_is_reported_as_unknown_and_cannot_be_retried_blindly() {
        let directory = tempfile::tempdir().unwrap();
        let state = state(directory.path());
        ended(&state, AgentKind::Cursor, "chat");
        let chat = Chat::new(true, Some(MessageResult::Unknown));
        let answer =
            send_from_console_with(&state, console("m1", "chat", "hello"), &chat, Lang::En)
                .await
                .unwrap();
        assert_eq!(answer["status"], "unknown");
        assert!(answer["message"]
            .as_str()
            .unwrap()
            .contains("may have been sent"));
        // The same id gets the same verdict and never reaches Cursor again.
        let retry = send_from_console_with(&state, console("m1", "chat", "hello"), &chat, Lang::En)
            .await
            .unwrap();
        assert_eq!(retry["status"], "unknown");
        assert_eq!(chat.sent().len(), 1);
        assert_eq!(record(&state, "chat")["state"], "ended");
    }

    #[tokio::test]
    async fn the_console_accepts_a_message_the_chat_took() {
        let directory = tempfile::tempdir().unwrap();
        let state = state(directory.path());
        ended(&state, AgentKind::Cursor, "chat");
        let chat = Chat::new(true, Some(MessageResult::Started("turn-1".into())));
        let answer =
            send_from_console_with(&state, console("m1", "chat", "hello"), &chat, Lang::Zh)
                .await
                .unwrap();
        assert_eq!(answer, json!({"status": "accepted"}));
        // A retry of an accepted message is the same acceptance, not a second message.
        let retry = send_from_console_with(&state, console("m1", "chat", "hello"), &chat, Lang::Zh)
            .await
            .unwrap();
        assert_eq!(retry, json!({"status": "accepted"}));
        assert_eq!(chat.sent().len(), 1);
    }

    #[tokio::test]
    async fn the_console_checks_its_input_before_it_looks_at_the_chat() {
        let directory = tempfile::tempdir().unwrap();
        let state = state(directory.path());
        ended(&state, AgentKind::Cursor, "chat");
        let chat = Chat::new(true, None);
        let reason =
            |answer: Result<Value, String>| answer.unwrap()["reason"].as_str().unwrap().to_string();
        assert_eq!(
            reason(
                send_from_console_with(&state, console("a", "chat", "  \n"), &chat, Lang::En).await
            ),
            "empty"
        );
        let long = "x".repeat(CONSOLE_MAX_CHARS + 1);
        assert_eq!(
            reason(
                send_from_console_with(&state, console("b", "chat", &long), &chat, Lang::En).await
            ),
            "tooLong"
        );
        assert!(
            send_from_console_with(&state, console("", "chat", "hi"), &chat, Lang::En)
                .await
                .is_err()
        );
        assert!(chat.sent().is_empty());
    }

    #[tokio::test]
    async fn the_console_never_messages_a_chat_that_is_working_or_unknown() {
        let directory = tempfile::tempdir().unwrap();
        let state = state(directory.path());
        state.agents.apply_event(
            AgentKind::Cursor,
            LifecycleEvent::TurnStart,
            "busy",
            None,
            Some("/tmp/project".into()),
            0,
        );
        let chat = Chat::new(true, None);
        for (sid, reason) in [("busy", "stateChanged"), ("nobody", "notFound")] {
            let answer = send_from_console_with(&state, console(sid, sid, "hi"), &chat, Lang::En)
                .await
                .unwrap();
            assert_eq!(answer["status"], "rejected", "{sid}");
            assert_eq!(answer["reason"], reason, "{sid}");
        }
        assert!(chat.sent().is_empty());
    }

    #[test]
    fn every_refusal_the_console_can_get_is_worded_in_both_languages() {
        for reason in [
            CHAT_NOT_LOADED,
            "nativeUnavailable",
            "idleUnsupported",
            "stateChanged",
            "resultChanged",
            "controlStarting",
            "ended",
            "notFound",
            "empty",
            "tooLong",
            "something-new",
        ] {
            let zh = console_refusal(reason, Lang::Zh);
            let en = console_refusal(reason, Lang::En);
            assert!(!zh.is_empty() && !en.is_empty(), "{reason}");
            assert_ne!(zh, en, "{reason}");
        }
    }

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
    #[tokio::test]
    async fn failure_continuation_after_ttl_can_retry_a_definite_rejection_but_not_replay_a_click()
    {
        let directory = tempfile::tempdir().unwrap();
        let state = state(directory.path());
        let result = TurnResult {
            id: "failure".into(),
            outcome: TurnOutcome::Failed,
            source: Some("cursorLog".into()),
            turn_id: Some("failed-turn".into()),
            at: now_ms(),
            reason: None,
            summary: None,
        };
        state.agents.apply_observed(
            AgentKind::Cursor,
            LifecycleEvent::TurnEnd,
            "chat",
            None,
            Some("/tmp/project".into()),
            0,
            Some(&Observation {
                turn_id: Some("failed-turn".into()),
                result: Some(result),
                ..Default::default()
            }),
        );
        let result_id = record(&state, "chat")["lastTurnResult"]["id"]
            .as_str()
            .unwrap()
            .to_string();
        state
            .agents
            .retain_failure(AgentKind::Cursor, "chat", &result_id, now_ms() + 86_400_000);
        state.agents.apply_event(
            AgentKind::Cursor,
            LifecycleEvent::SessionEnd,
            "chat",
            None,
            None,
            0,
        );
        let failure_message = |id| Message {
            expected_result_id: Some(result_id.clone()),
            ..message(id, "chat", "")
        };
        let unavailable = Chat::new(true, Some(MessageResult::Rejected("notFound".into())));
        let first = send_with(&state, failure_message("first-click"), &unavailable).await;
        assert_eq!(first.reason.as_deref(), Some(CHAT_NOT_LOADED));
        let ready = Chat::new(true, Some(MessageResult::Started("new-turn".into())));
        assert_eq!(
            send_with(&state, failure_message("first-click"), &ready)
                .await
                .reason
                .as_deref(),
            Some(CHAT_NOT_LOADED)
        );
        assert!(
            ready.sent().is_empty(),
            "retransmitting one click never sends again"
        );
        assert_eq!(
            send_with(&state, failure_message("second-click"), &ready)
                .await
                .status,
            "delivered"
        );
        assert_eq!(ready.sent().len(), 1);
        assert_eq!(record(&state, "chat")["state"], "working");
    }
}
