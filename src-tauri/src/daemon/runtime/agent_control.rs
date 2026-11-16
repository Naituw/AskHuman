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

/// What the daemon asks of the native message bridges: Cursor Desktop's, and the sidecars of the
/// Antigravity App. Everything around them (which sessions may be messaged, how a refusal reads,
/// what the registry does afterwards) is tested against a stand-in, so no test ever reaches a real
/// Cursor or a real Antigravity.
#[allow(async_fn_in_trait)]
pub(super) trait NativeChat {
    /// Whether the session's chat can be messaged at all, judged from files alone (never by
    /// attaching to anything).
    fn available(&self, kind: AgentKind, session_id: &str) -> bool;
    async fn send_cursor(
        &self,
        session_id: &str,
        id: &str,
        text: &str,
    ) -> crate::cursor_desktop::MessageResult;
    /// Puts `text` into an idle conversation of the Antigravity App. `directory` is the
    /// conversation's project folder when the registry knows it.
    async fn send_antigravity(
        &self,
        session_id: &str,
        text: &str,
        directory: Option<&str>,
    ) -> AntigravitySend;
}

/// How the Antigravity App's route answered one message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum AntigravitySend {
    /// The sidecar's `agentapi send-message` succeeded. That is delivery, not a turn: the
    /// conversation's own hook proves the turn started, or nothing does.
    Delivered,
    /// Nothing was sent.
    NotSent,
    /// It may have been sent.
    Unknown,
}

/// The real bridges.
pub(super) struct LiveChats;

impl NativeChat for LiveChats {
    fn available(&self, kind: AgentKind, session_id: &str) -> bool {
        match kind {
            AgentKind::Cursor => crate::cursor_desktop::messages_available(session_id),
            AgentKind::Antigravity => crate::antigravity_desktop::message_available(session_id),
            _ => false,
        }
    }

    async fn send_cursor(
        &self,
        session_id: &str,
        id: &str,
        text: &str,
    ) -> crate::cursor_desktop::MessageResult {
        crate::cursor_desktop::send_message(session_id, id, text).await
    }

    async fn send_antigravity(
        &self,
        session_id: &str,
        text: &str,
        directory: Option<&str>,
    ) -> AntigravitySend {
        use crate::antigravity_desktop::{send_message, Failure, Message};
        match send_message(Message {
            conversation: session_id.into(),
            text: text.into(),
            directory: directory.map(std::path::PathBuf::from),
        })
        .await
        {
            Ok(()) => AntigravitySend::Delivered,
            Err(Failure::NotSent(_)) => AntigravitySend::NotSent,
            Err(Failure::Unclear(_)) => AntigravitySend::Unknown,
        }
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

/// How long an answer about a native chat is trusted when it only feeds a snapshot or a button.
/// Each answer reads Cursor's database or Antigravity's settings and logs, and snapshots follow
/// every Agent event. Sending always asks again, so a stale answer can at worst show a composer
/// that then says it is unavailable.
const CHAT_ANSWER_TTL: Duration = Duration::from_secs(20);

static CHAT_ANSWERS: std::sync::LazyLock<
    std::sync::Mutex<std::collections::HashMap<String, (std::time::Instant, bool)>>,
> = std::sync::LazyLock::new(Default::default);

/// [`NativeChat::available`] with the answer remembered for [`CHAT_ANSWER_TTL`].
pub(super) fn cached_chat(kind: AgentKind, session_id: &str) -> bool {
    let now = std::time::Instant::now();
    let key = format!("{}:{session_id}", kind.as_str());
    let answers = CHAT_ANSWERS.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(&(at, ok)) = answers.get(&key) {
        if now.duration_since(at) < CHAT_ANSWER_TTL {
            return ok;
        }
    }
    drop(answers);
    // The lookup reads files: never hold the lock across it.
    let ok = LiveChats.available(kind, session_id);
    let mut answers = CHAT_ANSWERS.lock().unwrap_or_else(|e| e.into_inner());
    if answers.len() > 256 {
        answers.retain(|_, (at, _)| now.duration_since(*at) < CHAT_ANSWER_TTL);
    }
    answers.insert(key, (now, ok));
    ok
}

pub(super) fn can_send(state: &Arc<ServerState>, rec: &Value) -> crate::ios::wire::CanInterject {
    can_send_with(state, rec, &cached_chat)
}

/// `has_chat` says whether the session has a native chat to be messaged (a Cursor Desktop chat, an
/// Antigravity App conversation); it is asked at most once, and only for the states and the Agents
/// that need it.
pub(super) fn can_send_with(
    state: &Arc<ServerState>,
    rec: &Value,
    has_chat: &dyn Fn(AgentKind, &str) -> bool,
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
    let native_chat = || {
        *chat.get_or_init(|| {
            matches!(kind, AgentKind::Cursor | AgentKind::Antigravity) && has_chat(kind, sid)
        })
    };
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
        AgentKind::Cursor | AgentKind::Antigravity => native_chat(),
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
    send_with(state, message, &LiveChats).await
}

pub(super) async fn send_with(
    state: &Arc<ServerState>,
    message: Message<'_>,
    chats: &impl NativeChat,
) -> InterjectResult {
    let guard = state.agent_controls.send_gate.lock().await;
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
    // Registry lifecycle states do not determine whether a native message channel still exists.
    let sendable = matches!(rec["state"].as_str(), Some("idle" | "ended"));
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
    let can = can_send_with(state, rec, &|kind, sid| chats.available(kind, sid));
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
        // The turn an Antigravity message starts is announced by the conversation's own hook,
        // which cannot carry an id of ours: the daemon names the turn here, and the hook's
        // `TurnStart` takes this name (see `claim_turn_start`).
        let turn_id =
            (message.kind == AgentKind::Antigravity).then(|| uuid::Uuid::new_v4().to_string());
        if state
            .agent_controls
            .finish(&id, "dispatched", turn_id)
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
            match chats.send_cursor(message.sid, &id, &text).await {
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
        } else if message.kind == AgentKind::Antigravity {
            let mut text = operation.text.clone();
            for file in &operation.files {
                text.push_str(&format!("\n\nAttached file: {} ({})", file.name, file.path));
            }
            // The host shows this to the model as a system message; the wrapper says it is the
            // user's.
            let text = crate::prompts::antigravity_idle_message(&text);
            let directory = rec["cwd"].as_str();
            // Starting the App can take most of half a minute, and the message is already
            // recorded as dispatched, so another one for this session is refused meanwhile.
            // Messages for other sessions need not wait for this one.
            drop(guard);
            match chats.send_antigravity(message.sid, &text, directory).await {
                // Delivered is not started: the turn's own hook proves that, or nothing does.
                AntigravitySend::Delivered => {}
                AntigravitySend::NotSent => {
                    let _ = state.agent_controls.reject(&id, "nativeUnavailable");
                }
                AntigravitySend::Unknown => {
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
    // How long the conversation's own hook has to announce the turn the message started.
    let window = if message.kind == AgentKind::Antigravity {
        ANTIGRAVITY_START_WINDOW
    } else {
        Duration::from_secs(5)
    };
    match state.agent_controls.wait_for_submission(&id, window).await {
        Ok(operation) => receipt(&operation),
        Err(_) => InterjectResult::rejected("controlUnknown"),
    }
}

/// How long a message to an Antigravity conversation waits for the turn it starts to be announced
/// by that conversation's `PreInvocation` hook. The conversation wakes up on its own after the
/// sidecar delivered the message, so this is the model's start-up, not a network call.
const ANTIGRAVITY_START_WINDOW: Duration = Duration::from_secs(15);

/// Matches the turn an Antigravity hook announces to the message that woke the conversation. The
/// hook cannot carry our operation's id, so the turn that starts while one message is waiting
/// answers it: the event takes the message's id and the turn id the daemon named at dispatch,
/// which is what [`ControlStore::started`] and the phone's follow look for.
pub(super) fn claim_turn_start(
    state: &Arc<ServerState>,
    kind: AgentKind,
    session_id: &str,
    event: LifecycleEvent,
    observation: &mut Observation,
) {
    if kind != AgentKind::Antigravity || event != LifecycleEvent::TurnStart {
        return;
    }
    if let Some(operation) = state.agent_controls.awaiting_start(kind, session_id) {
        observation.control_operation_id = Some(operation.id);
        observation.turn_id = operation.turn_id;
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
    has_chat: &dyn Fn(AgentKind, &str) -> bool,
) {
    let Some(records) = snapshot.as_array_mut() else {
        return;
    };
    for rec in records.iter_mut() {
        let eligible = matches!(rec["state"].as_str(), Some("idle" | "ended"))
            && rec["waitingRequestId"]
                .as_str()
                .is_none_or(|id| id.is_empty());
        if eligible && can_send_with(state, rec, has_chat).ok {
            rec["canMessage"] = Value::Bool(true);
        }
    }
}

/// Explain a rejected native message to the console or its source IM channel.
pub(super) fn message_refusal(reason: &str, lang: crate::i18n::Lang) -> String {
    use crate::i18n::Lang::{En, Zh};
    match (reason, lang) {
        (CHAT_NOT_LOADED, Zh) => "Cursor 里没有打开这个聊天，没有发送。请先在 Cursor 里打开它，再试一次。".into(),
        (CHAT_NOT_LOADED, En) => "Cursor has not opened this chat in any window, so nothing was sent. Open it in Cursor and try again.".into(),
        ("nativeUnavailable" | "idleUnsupported" | "desktopDisconnected", Zh) => "原会话的消息通道暂不可用，没有发送。请在 Agent 中打开原会话后重试。".into(),
        ("nativeUnavailable" | "idleUnsupported" | "desktopDisconnected", En) => "The original session’s message channel is unavailable. Open the session in the Agent and try again.".into(),
        ("stateChanged" | "resultChanged", Zh) => "这个聊天此刻不空闲（正在运行、等你确认，或有排队的消息），没有发送。".into(),
        ("stateChanged" | "resultChanged", En) => "This chat is not idle right now (it is running, waiting for you, or has a queued message), so nothing was sent.".into(),
        ("controlStarting", Zh) => "上一条消息还在等 Agent 开始运行，请稍候再发。".into(),
        ("controlStarting", En) => "The previous message is still waiting for the Agent to start. Try again in a moment.".into(),
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
pub(super) fn message_unknown(lang: crate::i18n::Lang) -> String {
    match lang {
        crate::i18n::Lang::Zh => "消息可能已经发出。请先检查 Agent 里的原会话，再决定要不要再发一条。".into(),
        crate::i18n::Lang::En => "The message may have been sent. Check the original session in the Agent before sending another.".into(),
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
    send_from_console_with(state, message, &LiveChats, crate::i18n::Lang::current()).await
}

pub(super) async fn send_from_console_with(
    state: &Arc<ServerState>,
    message: ConsoleMessage,
    chats: &impl NativeChat,
    lang: crate::i18n::Lang,
) -> Result<Value, String> {
    let rejected = |reason: &str| {
        Ok(
            json!({"status": "rejected", "reason": reason, "message": message_refusal(reason, lang)}),
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
        chats,
    )
    .await;
    match (sent.status, sent.reason.as_deref()) {
        ("delivered", _) => Ok(json!({"status": "accepted"})),
        (_, Some("controlUnknown" | "conflict" | "interrupted" | "desktopUnknown")) => {
            Ok(json!({"status": "unknown", "message": message_unknown(lang)}))
        }
        (_, reason) => rejected(reason.unwrap_or("stateChanged")),
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

pub(super) fn apply(
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
            // The App preserves our UUID separately from the server-generated message id.
            let ids: Vec<&str> = turn["params"]["clientUserMessageId"]
                .as_str()
                .into_iter()
                .chain(
                    crate::codex_desktop::protocol::entities(&turn["items"])
                        .into_iter()
                        .filter(|item| item["type"] == "userMessage")
                        .flat_map(|item| {
                            [
                                item["id"].as_str(),
                                item["clientId"].as_str(),
                                item["clientUserMessageId"].as_str(),
                            ]
                        })
                        .flatten(),
                )
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

    /// The native chats, existing only in the test: they record what they are asked to send and
    /// answer with whatever the test queued. Nothing here can reach a real Cursor or Antigravity.
    struct Chat {
        exists: bool,
        reply: StdMutex<Option<MessageResult>>,
        sent: StdMutex<Vec<(String, String, String)>>,
        /// Whether an Antigravity conversation can be messaged, and what the route answers.
        antigravity_exists: bool,
        antigravity_reply: StdMutex<Option<AntigravitySend>>,
        antigravity_sent: StdMutex<Vec<(String, String, Option<String>)>>,
    }

    impl Chat {
        fn new(exists: bool, reply: Option<MessageResult>) -> Self {
            Self {
                exists,
                reply: StdMutex::new(reply),
                sent: Default::default(),
                antigravity_exists: false,
                antigravity_reply: StdMutex::new(None),
                antigravity_sent: Default::default(),
            }
        }
        fn antigravity(mut self, exists: bool, reply: Option<AntigravitySend>) -> Self {
            self.antigravity_exists = exists;
            self.antigravity_reply = StdMutex::new(reply);
            self
        }
        fn sent(&self) -> Vec<(String, String, String)> {
            self.sent.lock().unwrap().clone()
        }
        fn antigravity_sent(&self) -> Vec<(String, String, Option<String>)> {
            self.antigravity_sent.lock().unwrap().clone()
        }
    }

    impl NativeChat for Chat {
        fn available(&self, kind: AgentKind, _session_id: &str) -> bool {
            match kind {
                AgentKind::Cursor => self.exists,
                AgentKind::Antigravity => self.antigravity_exists,
                _ => false,
            }
        }

        async fn send_cursor(&self, session_id: &str, id: &str, text: &str) -> MessageResult {
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

        async fn send_antigravity(
            &self,
            session_id: &str,
            text: &str,
            directory: Option<&str>,
        ) -> AntigravitySend {
            self.antigravity_sent.lock().unwrap().push((
                session_id.into(),
                text.into(),
                directory.map(str::to_string),
            ));
            self.antigravity_reply
                .lock()
                .unwrap()
                .take()
                .expect("the test queued no answer for this Antigravity send")
        }
    }

    fn state(directory: &std::path::Path) -> Arc<ServerState> {
        super::super::test_state(directory)
    }

    /// A session retained in the registry after its lifecycle ended.
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
        assert!(can_send_with(&state, &rec, &|_, _| true).ok);
        let gone = can_send_with(&state, &rec, &|_, _| false);
        assert!(!gone.ok);
        assert_eq!(gone.reason.as_deref(), Some("idleUnsupported"));
    }

    #[test]
    fn ended_sessions_require_an_available_native_channel() {
        let directory = tempfile::tempdir().unwrap();
        let state = state(directory.path());
        for kind in [AgentKind::Claude, AgentKind::Codex, AgentKind::Pi] {
            let sid = format!("other-{}", kind.as_str());
            ended(&state, kind, &sid);
            let rec = record(&state, &sid);
            // The lookup must not even be asked: it reads Cursor's database.
            let refused =
                can_send_with(&state, &rec, &|_, _| panic!("asked Cursor about {kind:?}"));
            assert!(!refused.ok);
            assert_eq!(
                refused.reason.as_deref(),
                Some("idleUnsupported"),
                "{kind:?}"
            );
        }
    }

    #[test]
    fn desktop_sending_depends_on_the_channel_in_every_lifecycle_state() {
        let directory = tempfile::tempdir().unwrap();
        let state = state(directory.path());
        for lifecycle in ["working", "idle", "ended"] {
            let mut rec = json!({"kind":"codex", "sessionId":"desktop", "state":lifecycle,
                "desktop":{"connected":true}});
            assert!(can_send_with(&state, &rec, &|_, _| panic!("Cursor lookup")).ok);
            rec["desktop"]["connected"] = false.into();
            assert_eq!(
                can_send_with(&state, &rec, &|_, _| panic!("Cursor lookup"))
                    .reason
                    .as_deref(),
                Some("desktopDisconnected")
            );
        }
    }

    #[tokio::test]
    async fn phone_console_and_im_can_continue_an_ended_pi_session() {
        use crate::agents::control::ControlSignal;
        for source in ["phone", "console", "im"] {
            let directory = tempfile::tempdir().unwrap();
            let state = state(directory.path());
            ended(&state, AgentKind::Pi, "pi");
            let signal = ControlSignal {
                generation: uuid::Uuid::new_v4().to_string(),
                pid: std::process::id(),
                idle: true,
                operation_id: None,
                status: None,
                turn_id: None,
            };
            state.agent_controls.poll("pi", &signal).unwrap();
            let mut snapshot = state.agents.snapshot();
            mark_messageable(&state, &mut snapshot, &|_, _| panic!("Cursor lookup"));
            assert_eq!(snapshot[0]["canMessage"], true);
            assert!(super::super::can_message_session(&state, &snapshot, "pi"));
            let options =
                crate::select::msg_options(&snapshot, &Default::default(), 0, Lang::En, |record| {
                    can_send(&state, record).ok
                });
            assert_eq!(options[0].id, "pi");
            let submit = async {
                match source {
                    "phone" => {
                        let sent = ios_agents::interject(
                            &state,
                            "phone",
                            "m1",
                            "pi:pi",
                            "continue",
                            vec![],
                        )
                        .await;
                        assert_eq!(sent.status, "delivered");
                    }
                    "console" => {
                        let mut input = console("m1", "pi", "continue");
                        input.kind = AgentKind::Pi;
                        let sent = send_from_console_with(
                            &state,
                            input,
                            &Chat::new(false, None),
                            Lang::En,
                        )
                        .await
                        .unwrap();
                        assert_eq!(sent["status"], "accepted");
                    }
                    _ => {
                        let text =
                            super::super::deliver_msg(&state, "test", "pi", "continue", Lang::En)
                                .unwrap();
                        assert_eq!(text, crate::i18n::tr(Lang::En, "autoChannel.msgSubmitting"));
                    }
                }
            };
            let consume = async {
                let command = tokio::time::timeout(Duration::from_secs(2), async {
                    loop {
                        if let Some(command) =
                            state.agent_controls.poll("pi", &signal).unwrap().operation
                        {
                            break command;
                        }
                        tokio::time::sleep(Duration::from_millis(1)).await;
                    }
                })
                .await
                .unwrap();
                assert_eq!(command.text, "continue");
                state
                    .agent_controls
                    .finish(&command.id, "accepted", Some("turn".into()))
                    .unwrap();
                apply(
                    &state,
                    AgentKind::Pi,
                    "pi",
                    LifecycleEvent::TurnStart,
                    Observation {
                        turn_id: Some("turn".into()),
                        control_operation_id: Some(command.id),
                        ..Observation::default()
                    },
                    None,
                );
            };
            tokio::join!(submit, consume);
            assert_eq!(record(&state, "pi")["state"], "working");
            assert_eq!(state.interject.pending_count("pi"), 0);
        }
    }

    #[test]
    fn an_idle_cursor_session_with_no_chat_behind_it_is_a_cli_session() {
        let directory = tempfile::tempdir().unwrap();
        let state = state(directory.path());
        let rec = json!({"kind":"cursor","sessionId":"cli","state":"idle"});
        assert!(can_send_with(&state, &rec, &|_, _| true).ok);
        let refused = can_send_with(&state, &rec, &|_, _| false);
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
        let refused = can_send_with(&state, &record(&state, "chat"), &|_, _| true);
        assert_eq!(refused.reason.as_deref(), Some("controlStarting"));
    }

    // MARK: Antigravity

    fn antigravity_message<'a>(id: &'a str, sid: &'a str, text: &'a str) -> Message<'a> {
        Message {
            kind: AgentKind::Antigravity,
            ..message(id, sid, text)
        }
    }

    /// What happens when a message wakes a conversation: its own hook announces a turn, and the
    /// daemon's handler matches that turn to the message before the registry applies it.
    fn conversation_wakes(state: &Arc<ServerState>, sid: &str) {
        let mut observation = Observation::default();
        claim_turn_start(
            state,
            AgentKind::Antigravity,
            sid,
            LifecycleEvent::TurnStart,
            &mut observation,
        );
        apply(
            state,
            AgentKind::Antigravity,
            sid,
            LifecycleEvent::TurnStart,
            observation,
            None,
        );
    }

    async fn until(mut done: impl FnMut() -> bool) {
        tokio::time::timeout(Duration::from_secs(2), async {
            while !done() {
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        })
        .await
        .expect("the condition never came true");
    }

    fn completed() -> Observation {
        Observation {
            result: Some(TurnResult {
                source: Some("test".into()),
                id: String::new(),
                outcome: TurnOutcome::Completed,
                turn_id: None,
                reason: None,
                summary: None,
                at: 0,
            }),
            ..Observation::default()
        }
    }

    #[tokio::test]
    async fn a_message_to_an_idle_antigravity_conversation_is_delivered_once_and_its_turn_answers_it(
    ) {
        let directory = tempfile::tempdir().unwrap();
        let state = state(directory.path());
        ended(&state, AgentKind::Antigravity, "ag");
        let chat = Chat::new(false, None).antigravity(true, Some(AntigravitySend::Delivered));
        let submit = send_with(
            &state,
            antigravity_message("m1", "ag", "please continue"),
            &chat,
        );
        let wake = async {
            until(|| {
                state
                    .agent_controls
                    .awaiting_start(AgentKind::Antigravity, "ag")
                    .is_some()
            })
            .await;
            conversation_wakes(&state, "ag");
        };
        let (sent, ()) = tokio::join!(submit, wake);
        assert_eq!(sent.status, "delivered");
        let started = sent.started_control.expect("the turn answered the message");
        assert_eq!(started.phone_id, "mac-console");
        // The route got the text, worded as the user's, and the conversation's folder, once.
        assert_eq!(
            chat.antigravity_sent(),
            [(
                "ag".to_string(),
                crate::prompts::antigravity_idle_message("please continue"),
                Some("/tmp/project".to_string())
            )]
        );
        // The turn the registry runs is the one the operation was told about, so the phone's
        // follow finds the result of the turn its message started.
        let rec = record(&state, "ag");
        assert_eq!(rec["state"], "working");
        assert_eq!(rec["activeTurnId"], started.turn_id);
        apply(
            &state,
            AgentKind::Antigravity,
            "ag",
            LifecycleEvent::TurnEnd,
            completed(),
            None,
        );
        let result = state
            .agents
            .last_turn_result(AgentKind::Antigravity, "ag")
            .unwrap();
        assert_eq!(result.turn_id.as_deref(), Some(started.turn_id.as_str()));
        // The same message again is answered from the journal; nothing is sent twice.
        let again = send_with(
            &state,
            antigravity_message("m1", "ag", "please continue"),
            &chat,
        )
        .await;
        assert_eq!(again.status, "delivered");
        assert_eq!(chat.antigravity_sent().len(), 1);
    }

    #[tokio::test]
    async fn the_files_of_a_message_to_antigravity_are_named_in_its_text() {
        let directory = tempfile::tempdir().unwrap();
        let state = state(directory.path());
        ended(&state, AgentKind::Antigravity, "ag");
        let chat = Chat::new(false, None).antigravity(true, Some(AntigravitySend::NotSent));
        let mut with_file = antigravity_message("m1", "ag", "look");
        with_file.files = vec![FileAttachment {
            path: "/tmp/a.png".into(),
            name: "a.png".into(),
            size: 3,
            is_image: true,
        }];
        send_with(&state, with_file, &chat).await;
        assert_eq!(
            chat.antigravity_sent()[0].1,
            crate::prompts::antigravity_idle_message("look\n\nAttached file: a.png (/tmp/a.png)")
        );
    }

    #[tokio::test]
    async fn a_message_the_route_did_not_send_is_rejected_and_never_tried_again_under_its_id() {
        let directory = tempfile::tempdir().unwrap();
        let state = state(directory.path());
        ended(&state, AgentKind::Antigravity, "ag");
        let chat = Chat::new(false, None).antigravity(true, Some(AntigravitySend::NotSent));
        let sent = send_with(&state, antigravity_message("m1", "ag", "hello"), &chat).await;
        assert_eq!(sent, InterjectResult::rejected("nativeUnavailable"));
        // The stand-in fails the test if it is asked a second time.
        let again = send_with(&state, antigravity_message("m1", "ag", "hello"), &chat).await;
        assert_eq!(again, sent);
        assert_eq!(chat.antigravity_sent().len(), 1);
        // A refusal leaves nothing waiting, so a new message is a new operation.
        assert!(!state.agent_controls.pending(AgentKind::Antigravity, "ag"));
        assert_eq!(record(&state, "ag")["state"], "ended");
        // The console says it in words.
        let answer = send_from_console_with(
            &state,
            ConsoleMessage {
                kind: AgentKind::Antigravity,
                ..console("m2", "ag", "hello")
            },
            &Chat::new(false, None).antigravity(true, Some(AntigravitySend::NotSent)),
            Lang::En,
        )
        .await
        .unwrap();
        assert_eq!(answer["status"], "rejected");
        assert_eq!(answer["reason"], "nativeUnavailable");
    }

    #[tokio::test]
    async fn a_message_that_may_have_been_sent_is_unknown_and_is_not_claimed_by_a_later_turn() {
        let directory = tempfile::tempdir().unwrap();
        let state = state(directory.path());
        ended(&state, AgentKind::Antigravity, "ag");
        let chat = Chat::new(false, None).antigravity(true, Some(AntigravitySend::Unknown));
        let sent = send_with(&state, antigravity_message("m1", "ag", "hello"), &chat).await;
        assert_eq!(sent, InterjectResult::rejected("controlUnknown"));
        let again = send_with(&state, antigravity_message("m1", "ag", "hello"), &chat).await;
        assert_eq!(again, sent);
        assert_eq!(chat.antigravity_sent().len(), 1);
        assert!(!state.agent_controls.pending(AgentKind::Antigravity, "ag"));
        let mut observation = Observation::default();
        claim_turn_start(
            &state,
            AgentKind::Antigravity,
            "ag",
            LifecycleEvent::TurnStart,
            &mut observation,
        );
        assert_eq!(observation.control_operation_id, None);
        assert_eq!(observation.turn_id, None);
    }

    #[tokio::test(start_paused = true)]
    async fn a_delivered_message_no_turn_answers_becomes_unknown_when_the_window_ends() {
        let directory = tempfile::tempdir().unwrap();
        let state = state(directory.path());
        ended(&state, AgentKind::Antigravity, "ag");
        let chat = Chat::new(false, None).antigravity(true, Some(AntigravitySend::Delivered));
        let sent = send_with(&state, antigravity_message("m1", "ag", "hello"), &chat).await;
        assert_eq!(sent, InterjectResult::rejected("controlUnknown"));
        let operation = state
            .agent_controls
            .existing(&operation_id("mac-console", "m1"))
            .unwrap()
            .unwrap();
        assert_eq!(operation.status, "unknown");
        // A turn that starts after the window is nobody's answer.
        let mut observation = Observation::default();
        claim_turn_start(
            &state,
            AgentKind::Antigravity,
            "ag",
            LifecycleEvent::TurnStart,
            &mut observation,
        );
        assert_eq!(observation.control_operation_id, None);
    }

    #[test]
    fn only_antigravity_turn_starts_are_matched_to_a_waiting_message() {
        let directory = tempfile::tempdir().unwrap();
        let state = state(directory.path());
        let id = operation_id("mac-console", "m1");
        state
            .agent_controls
            .begin(Operation {
                id: id.clone(),
                kind: AgentKind::Antigravity,
                session_id: "ag".into(),
                phone_id: "mac-console".into(),
                generation: None,
                text: "hello".into(),
                files: Vec::new(),
                expected_result_id: None,
                status: "queued".into(),
                reason: None,
                turn_id: None,
            })
            .unwrap();
        state
            .agent_controls
            .finish(&id, "dispatched", Some("turn-x".into()))
            .unwrap();
        let claimed = |kind, session: &str, event| {
            let mut observation = Observation::default();
            claim_turn_start(&state, kind, session, event, &mut observation);
            observation.control_operation_id
        };
        let ag = AgentKind::Antigravity;
        assert_eq!(claimed(ag, "ag", LifecycleEvent::TurnStart), Some(id));
        // Not another event, another session, or another Agent.
        assert_eq!(claimed(ag, "ag", LifecycleEvent::Activity), None);
        assert_eq!(claimed(ag, "ag", LifecycleEvent::TurnEnd), None);
        assert_eq!(claimed(ag, "other", LifecycleEvent::TurnStart), None);
        assert_eq!(
            claimed(AgentKind::Cursor, "ag", LifecycleEvent::TurnStart),
            None
        );
    }

    #[tokio::test]
    async fn nothing_is_sent_to_a_working_antigravity_conversation_or_to_one_the_app_does_not_own()
    {
        let directory = tempfile::tempdir().unwrap();
        let state = state(directory.path());
        for event in [LifecycleEvent::SessionStart, LifecycleEvent::TurnStart] {
            state.agents.apply_event(
                AgentKind::Antigravity,
                event,
                "busy",
                None,
                Some("/tmp/project".into()),
                0,
            );
        }
        // The stand-in has no answer queued: being asked would fail the test.
        let chat = Chat::new(false, None).antigravity(true, None);
        let sent = send_with(&state, antigravity_message("m1", "busy", "hi"), &chat).await;
        assert_eq!(sent, InterjectResult::rejected("stateChanged"));
        // An `agy` conversation is idle and ended like any other, but has no sidecar to carry it.
        ended(&state, AgentKind::Antigravity, "agy");
        let chat = Chat::new(false, None).antigravity(false, None);
        let sent = send_with(&state, antigravity_message("m2", "agy", "hi"), &chat).await;
        assert_eq!(sent, InterjectResult::rejected("idleUnsupported"));
        assert!(chat.antigravity_sent().is_empty());
    }

    #[test]
    fn an_antigravity_conversation_is_looked_up_only_when_it_is_idle_or_ended() {
        let directory = tempfile::tempdir().unwrap();
        let state = state(directory.path());
        let working = json!({"kind":"antigravity","sessionId":"w","state":"working"});
        assert!(can_send_with(&state, &working, &|_, _| panic!("looked up a working one")).ok);
        for lifecycle in ["idle", "ended"] {
            let rec = json!({"kind":"antigravity","sessionId":"i","state":lifecycle});
            let asked = std::cell::RefCell::new(Vec::new());
            let can = can_send_with(&state, &rec, &|kind, sid| {
                asked.borrow_mut().push((kind, sid.to_string()));
                true
            });
            assert!(can.ok, "{lifecycle}");
            assert_eq!(
                asked.into_inner(),
                [(AgentKind::Antigravity, "i".to_string())]
            );
            let refused = can_send_with(&state, &rec, &|_, _| false);
            assert_eq!(refused.reason.as_deref(), Some("idleUnsupported"));
        }
    }

    #[test]
    fn the_console_flags_the_antigravity_conversations_the_app_can_carry_a_message_to() {
        let directory = tempfile::tempdir().unwrap();
        let state = state(directory.path());
        let mut snapshot = json!([
            {"kind":"antigravity","sessionId":"app","state":"idle"},
            {"kind":"antigravity","sessionId":"app-ended","state":"ended"},
            {"kind":"antigravity","sessionId":"agy","state":"idle"},
            {"kind":"antigravity","sessionId":"busy","state":"working"},
            {"kind":"antigravity","sessionId":"asking","state":"idle","waitingRequestId":"r1"},
        ]);
        mark_messageable(&state, &mut snapshot, &|kind, sid| {
            kind == AgentKind::Antigravity && sid.starts_with("app") || sid == "asking"
        });
        let flagged: Vec<&str> = snapshot
            .as_array()
            .unwrap()
            .iter()
            .filter(|rec| rec["canMessage"] == true)
            .filter_map(|rec| rec["sessionId"].as_str())
            .collect();
        assert_eq!(flagged, ["app", "app-ended"]);
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
            {"kind":"codex","sessionId":"desktop","state":"ended","desktop":{"connected":true}},
        ]);
        mark_messageable(&state, &mut snapshot, &|_, sid| sid != "gone");
        let flagged: Vec<&str> = snapshot
            .as_array()
            .unwrap()
            .iter()
            .filter(|rec| rec["canMessage"] == true)
            .filter_map(|rec| rec["sessionId"].as_str())
            .collect();
        assert_eq!(flagged, ["idle", "ended", "desktop"]);
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
            let zh = message_refusal(reason, Lang::Zh);
            let en = message_refusal(reason, Lang::En);
            assert!(!zh.is_empty() && !en.is_empty(), "{reason}");
            assert_ne!(zh, en, "{reason}");
        }
    }

    #[test]
    fn desktop_turns_confirm_the_native_client_message_identity() {
        // Captured Desktop snapshots keep the submitted UUID in params.clientUserMessageId
        // and userMessage.clientId; the item's id is a different, server-generated UUID.
        for status in ["inProgress", "completed"] {
            for identity_in_params in [false, true] {
                let directory = tempfile::tempdir().unwrap();
                let state = state(directory.path());
                let id = uuid::Uuid::new_v4().to_string();
                state
                    .agent_controls
                    .begin(Operation {
                        id: id.clone(),
                        kind: AgentKind::Codex,
                        session_id: "desktop".into(),
                        phone_id: "phone".into(),
                        generation: None,
                        text: "hello".into(),
                        files: vec![],
                        expected_result_id: None,
                        status: "queued".into(),
                        reason: None,
                        turn_id: None,
                    })
                    .unwrap();
                state.agent_controls.finish(&id, "accepted", None).unwrap();
                let mut turn = json!({"turnId":"native-turn", "status":status,
                    "items":[{"type":"userMessage", "id":"server-message", "text":"hello"}]});
                if identity_in_params {
                    turn["params"] = json!({"clientUserMessageId":id});
                } else {
                    turn["items"][0]["clientId"] = json!(id);
                }
                let session = crate::codex_desktop::Session {
                    connected: true,
                    meta: Some(crate::codex_desktop::SessionMeta {
                        id: "desktop".into(),
                        title: "Chat".into(),
                        cwd: "/tmp/project".into(),
                    }),
                    state: Arc::new(json!({"turnHistory":{"kind":"canonical", "history":{
                        "islands":[{"entries":[{"value":"tail"}]}], "entitiesByKey":{"tail":turn}
                    }}})),
                    ..Default::default()
                };
                // A matching message UUID in a different session cannot acknowledge this send.
                observe_desktop(&state, "other-session", &session);
                assert_eq!(
                    state.agent_controls.existing(&id).unwrap().unwrap().status,
                    "accepted"
                );
                observe_desktop(&state, "desktop", &session);
                let operation = state.agent_controls.existing(&id).unwrap().unwrap();
                assert_eq!(
                    operation.status, "started",
                    "{status}, params={identity_in_params}"
                );
                assert_eq!(receipt(&operation).status, "delivered");
                assert_eq!(operation.turn_id.as_deref(), Some("native-turn"));
                assert!(!state.agent_controls.pending(AgentKind::Codex, "desktop"));
            }
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
