//! Away mode and per-session mute (`docs/specs/special-replies.md`).
//!
//! The daemon answers an Agent's question on the user's behalf while the user is away, and tells
//! an Agent that AskHuman was turned off for its session to stop using it. The engine here is the
//! single owner of that state:
//!
//! * `AwayState` is what is persisted (`~/.askhuman/state/away.json`) and what hooks read: a global
//!   away switch plus per-session overrides (away / mute / "always ask me" while globally away).
//! * `judge` is a pure function from the state, the question and the clock to a `Verdict`
//!   (deliver / deliver as an escalation / auto-reply). Everything user-visible (guidance text,
//!   counters, the anti-lockout rule) is decided here so it is testable without a daemon.
//! * `AwayEngine` wraps the state in a mutex, persists on change and exposes the operations the
//!   daemon, the popup directive, the tray and the CLI share (`AwayOp`).
//!
//! Hooks run in short-lived processes and must not touch the daemon on their hot path, so the
//! mute check they need reads the state file directly (`session_muted`).

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::i18n::Lang;
use crate::ipc::TaskRequest;
use crate::paths;

/// On-disk schema version of `away.json`.
pub const STATE_VERSION: u32 = 1;
/// The longest note the user may attach to away mode (characters).
pub const NOTE_MAX_CHARS: usize = 500;
/// Anti-lockout (spec D9): this many consecutive plain asks from one session inside the window are
/// answered; the next one is delivered to the user even though no escalation was requested.
pub const NUDGE_FREE_ASKS: u32 = 3;
/// Window of the anti-lockout rule; a quiet gap longer than this starts the count over.
pub const NUDGE_WINDOW_MS: u64 = 10 * 60 * 1000;
/// A session-level setting is dropped when the session has not been heard from for this long and
/// the registry no longer knows it (the registry's own session-end handling covers the normal case).
pub const SESSION_RETENTION_MS: u64 = 3 * 24 * 60 * 60 * 1000;
/// How long "ask me now" from the tray stays armed for a session.
pub const REQUESTED_ASK_TTL_MS: u64 = 30 * 60 * 1000;
/// Reply-history channel ids of automatic replies.
pub const CHANNEL_AWAY: &str = "away";
pub const CHANNEL_MUTE: &str = "mute";

/// The fixed interjection behind the tray's "Ask Me Now" (`app/gui_host.rs`). The daemon recognises
/// it so the question that follows is delivered even while away (spec N2).
pub const QUICK_ASK_TEXT: &str = "I have a new thought. Before continuing, ask me immediately using the questioning tool described in your instructions.";

// ===== State =====

/// A session-level override. A session without an entry follows the global setting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SessionMode {
    /// Away for this session only.
    Away,
    /// AskHuman is turned off for this session.
    Mute,
    /// Ask the user normally even while globally away.
    Normal,
}

impl SessionMode {
    /// The wire / UI name (`away`, `mute`, `normal`), the same as the serialized form.
    pub fn as_str(self) -> &'static str {
        match self {
            SessionMode::Away => "away",
            SessionMode::Mute => "mute",
            SessionMode::Normal => "normal",
        }
    }
}

/// The global away switch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GlobalAway {
    pub since_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// Automatic replies handed out since the switch went on.
    #[serde(default)]
    pub replies: u32,
    /// The same count per session key (the empty key collects callers without a session).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub replies_by_session: BTreeMap<String, u32>,
}

/// One session-level override.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionEntry {
    pub mode: SessionMode,
    pub since_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    #[serde(default)]
    pub replies: u32,
    /// Last time the session asked or was touched; drives the retention sweep.
    #[serde(default)]
    pub last_seen_ms: u64,
}

/// Everything that is persisted.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AwayState {
    #[serde(default)]
    pub version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub global: Option<GlobalAway>,
    /// Keyed by session key (`sid:<agent session id>` or `mcp:<instance id>`).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub sessions: BTreeMap<String, SessionEntry>,
}

/// Where an effective away setting comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Scope {
    Global,
    Session,
}

/// What applies to one question after combining the global switch and the session override.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effective {
    Normal,
    Away {
        scope: Scope,
        since_ms: u64,
        note: Option<String>,
    },
    Mute {
        since_ms: u64,
    },
}

impl AwayState {
    /// Mute beats away, a session entry beats the global switch (spec N1: mute > away > normal).
    pub fn effective(&self, key: Option<&str>) -> Effective {
        if let Some(entry) = key.and_then(|k| self.sessions.get(k)) {
            return match entry.mode {
                SessionMode::Mute => Effective::Mute {
                    since_ms: entry.since_ms,
                },
                SessionMode::Away => Effective::Away {
                    scope: Scope::Session,
                    since_ms: entry.since_ms,
                    note: entry.note.clone(),
                },
                SessionMode::Normal => Effective::Normal,
            };
        }
        match &self.global {
            Some(global) => Effective::Away {
                scope: Scope::Global,
                since_ms: global.since_ms,
                note: global.note.clone(),
            },
            None => Effective::Normal,
        }
    }

    /// True while the global switch or any session's own "away" setting is on.
    pub fn any_away(&self) -> bool {
        self.global.is_some()
            || self
                .sessions
                .values()
                .any(|entry| entry.mode == SessionMode::Away)
    }

    /// True when any part of the state would change how a question is handled.
    pub fn is_idle(&self) -> bool {
        self.global.is_none() && self.sessions.is_empty()
    }
}

// ===== Operations =====

/// Choice offered for one session (console, tray, iOS).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SessionSetting {
    /// Remove the override: the session follows the global setting.
    Follow,
    Away,
    Mute,
    /// Ask normally even while globally away; the same as `Follow` while not away.
    Normal,
}

/// One state-changing request, from the popup directive, the tray, the console, iOS or the CLI.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum AwayOp {
    /// Read-only snapshot.
    State,
    /// Turn global away on (or change its note).
    Start {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        note: Option<String>,
    },
    /// Turn global away off.
    End,
    /// Set or clear one session's override.
    Session {
        session_key: String,
        setting: SessionSetting,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        note: Option<String>,
    },
}

pub use crate::models::EscalationReason;

/// What a one-shot directive asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DirectiveKind {
    /// Turn away mode on.
    Away,
    /// Turn AskHuman off for the asking session.
    Mute,
}

/// A one-shot instruction that travels with an answer (the popup's quick-reply menu, the phone).
/// The daemon applies it when the answer is accepted and appends the matching guidance to what the
/// Agent receives (spec N3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Directive {
    pub kind: DirectiveKind,
    /// `global` or `session`; mute is always about the asking session.
    pub scope: Scope,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

impl Directive {
    /// The operation that makes this directive true for the asking session. `None` when it cannot
    /// be honoured (mute without a session key): the Agent still receives the guidance.
    pub fn to_op(&self, session_key: Option<&str>) -> Option<AwayOp> {
        let note = clean_note(self.note.as_deref());
        match (self.kind, self.scope, session_key) {
            (DirectiveKind::Away, Scope::Global, _)
            | (DirectiveKind::Away, Scope::Session, None) => Some(AwayOp::Start { note }),
            (DirectiveKind::Away, Scope::Session, Some(key)) => Some(AwayOp::Session {
                session_key: key.to_string(),
                setting: SessionSetting::Away,
                note,
            }),
            (DirectiveKind::Mute, _, Some(key)) => Some(AwayOp::Session {
                session_key: key.to_string(),
                setting: SessionSetting::Mute,
                note: None,
            }),
            (DirectiveKind::Mute, _, None) => None,
        }
    }

    /// The guidance appended to the answer (spec N3): the away text of the question's kind, or the
    /// mute text.
    pub fn guidance(&self, whats_next: bool, lang: Lang) -> String {
        match self.kind {
            DirectiveKind::Away => guidance(
                AutoKind::Away,
                whats_next,
                clean_note(self.note.as_deref()).as_deref(),
                lang,
            ),
            DirectiveKind::Mute => guidance(AutoKind::Mute, whats_next, None, lang),
        }
    }
}

/// Which automatic reply was produced.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AutoKind {
    Away,
    Mute,
}

impl AutoKind {
    /// Reply-history channel id of this kind.
    pub fn channel(self) -> &'static str {
        match self {
            AutoKind::Away => CHANNEL_AWAY,
            AutoKind::Mute => CHANNEL_MUTE,
        }
    }
}

/// An automatic reply for the daemon to hand to the asking Agent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AutoReply {
    pub kind: AutoKind,
    /// The guidance text the Agent receives (without any wire marker).
    pub text: String,
}

/// What to do with one question.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// Show the question as usual.
    Deliver,
    /// Show it, flagged as an escalation (the popup / iOS card says why it came through).
    Escalated(EscalationReason),
    /// Answer it automatically; the user is not interrupted.
    Reply(AutoReply),
}

/// The facts of one question the engine needs.
#[derive(Debug, Clone, Copy)]
pub struct AskFacts<'a> {
    pub session_key: Option<&'a str>,
    pub whats_next: bool,
    pub escalate: bool,
    pub lang: Lang,
}

/// The effective setting of one session as shown by the surfaces.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionView {
    pub session_key: String,
    pub mode: SessionMode,
    pub since_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    pub replies: u32,
}

/// The global setting as shown by the surfaces.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GlobalView {
    pub since_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    pub replies: u32,
    /// Automatic replies per session key while globally away.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub replies_by_session: BTreeMap<String, u32>,
}

/// A read-only picture of the state, sent to the tray, console, settings and iOS.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AwaySnapshot {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub global: Option<GlobalView>,
    #[serde(default)]
    pub sessions: Vec<SessionView>,
    /// Escalated questions waiting for an answer; filled in by the daemon from its registry.
    #[serde(default)]
    pub escalated_waiting: u32,
}

impl AwaySnapshot {
    pub fn from_state(state: &AwayState) -> Self {
        Self {
            global: state.global.as_ref().map(|g| GlobalView {
                since_ms: g.since_ms,
                note: g.note.clone(),
                replies: g.replies,
                replies_by_session: g.replies_by_session.clone(),
            }),
            sessions: state
                .sessions
                .iter()
                .map(|(key, e)| SessionView {
                    session_key: key.clone(),
                    mode: e.mode,
                    since_ms: e.since_ms,
                    note: e.note.clone(),
                    replies: e.replies,
                })
                .collect(),
            escalated_waiting: 0,
        }
    }

    /// True while the global switch is on.
    pub fn is_away(&self) -> bool {
        self.global.is_some()
    }

    /// True when neither the global switch nor any session override is set: nothing to show.
    pub fn is_empty(&self) -> bool {
        self.global.is_none() && self.sessions.is_empty()
    }

    /// The override of one session (`None` = it follows the global switch).
    pub fn session_mode(&self, key: &str) -> Option<SessionMode> {
        self.sessions
            .iter()
            .find(|view| view.session_key == key)
            .map(|view| view.mode)
    }
}

// ===== Session keys =====

/// Session key of a registered Agent session (the same key `ask_dedup::session_key` derives, so
/// "this session" means the same thing for history, coalescing and away mode).
pub fn sid_key(session_id: &str) -> String {
    format!("sid:{session_id}")
}

/// The session a question belongs to; `None` for callers without a session (they only follow the
/// global switch, spec N5).
pub fn session_key_of(task: &TaskRequest) -> Option<String> {
    super::ask_dedup::session_key(task)
}

/// Whether this request is an Agent's own AskHuman question (the only kind the engine acts on).
///
/// CLI, MCP and the Claude `AskUserQuestion` takeover carry no native request id; every other
/// source that reaches the daemon as a task is daemon-owned or native (failure and recheck cards,
/// Codex Desktop questions, development relays) and is never intercepted, and neither are Stop
/// confirmations (spec D5).
pub fn is_agent_ask(task: &TaskRequest) -> bool {
    task.popup_kind() == crate::ipc::PopupRequestKind::Ask
        && task.message.development.is_none()
        && task.native_request_id.is_none()
}

/// Normalises a user-supplied note: trimmed, bounded, `None` when empty.
pub fn clean_note(note: Option<&str>) -> Option<String> {
    let trimmed = note?.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(trimmed.chars().take(NOTE_MAX_CHARS).collect())
}

// ===== Guidance text =====

/// The text an Agent receives instead of an answer (spec §4.1).
pub fn guidance(kind: AutoKind, whats_next: bool, note: Option<&str>, lang: Lang) -> String {
    let key = match (kind, whats_next) {
        (AutoKind::Away, false) => "away.reply.ask",
        (AutoKind::Away, true) => "away.reply.whatsNext",
        (AutoKind::Mute, _) => "away.reply.mute",
    };
    let mut out = crate::i18n::tr(lang, key).to_string();
    if kind == AutoKind::Away {
        if let Some(note) = note.map(str::trim).filter(|n| !n.is_empty()) {
            out.push('\n');
            out.push_str(&crate::i18n::tr(lang, "away.note").replace("{note}", note));
        }
    }
    out
}

/// The interjection queued for a session when it leaves mute. English only, like the other fixed
/// interjections (`QUICK_ASK_TEXT`): it is read by the Agent, not by the user.
pub const UNMUTE_NOTICE: &str = "I turned AskHuman back on for this session. You may use it again.";

// ===== Verdict =====

#[derive(Debug, Clone, Copy, Default)]
struct Nudge {
    count: u32,
    window_start_ms: u64,
}

struct Inner {
    state: AwayState,
    /// Anti-lockout counters per session key (the empty key for callers without a session).
    nudges: HashMap<String, Nudge>,
    /// Sessions the user asked to ask them, with the arming time.
    requested: HashMap<String, u64>,
}

impl Inner {
    fn new(state: AwayState) -> Self {
        Self {
            state,
            nudges: HashMap::new(),
            requested: HashMap::new(),
        }
    }

    /// Decides one question. Returns the verdict and whether persisted state changed.
    fn judge(&mut self, facts: &AskFacts<'_>, now_ms: u64) -> (Verdict, bool) {
        let key = facts.session_key.filter(|k| !k.is_empty());
        let counter_key = key.unwrap_or("").to_string();
        let mut changed = false;
        if let Some(entry) = key.and_then(|k| self.state.sessions.get_mut(k)) {
            entry.last_seen_ms = now_ms;
        }

        // The user asked this session to ask them: that one question is delivered (spec N2).
        let requested = key.is_some_and(|k| {
            self.requested
                .remove(k)
                .is_some_and(|armed| now_ms.saturating_sub(armed) <= REQUESTED_ASK_TTL_MS)
        });

        match self.state.effective(key) {
            Effective::Normal => (Verdict::Deliver, changed),
            Effective::Mute { .. } => {
                // Mute wins over everything, escalation included (spec N1).
                if let Some(entry) = key.and_then(|k| self.state.sessions.get_mut(k)) {
                    entry.replies = entry.replies.saturating_add(1);
                    changed = true;
                }
                (
                    Verdict::Reply(AutoReply {
                        kind: AutoKind::Mute,
                        text: guidance(AutoKind::Mute, facts.whats_next, None, facts.lang),
                    }),
                    changed,
                )
            }
            Effective::Away { scope, note, .. } => {
                if requested {
                    self.nudges.remove(&counter_key);
                    return (Verdict::Escalated(EscalationReason::Requested), changed);
                }
                if facts.escalate {
                    self.nudges.remove(&counter_key);
                    return (Verdict::Escalated(EscalationReason::Asked), changed);
                }
                let nudge = self.nudges.entry(counter_key.clone()).or_default();
                if nudge.count == 0
                    || now_ms.saturating_sub(nudge.window_start_ms) > NUDGE_WINDOW_MS
                {
                    nudge.count = 0;
                    nudge.window_start_ms = now_ms;
                }
                if nudge.count >= NUDGE_FREE_ASKS {
                    // The Agent keeps asking without the escalation flag: let the user see it.
                    self.nudges.remove(&counter_key);
                    return (Verdict::Escalated(EscalationReason::Repeated), changed);
                }
                nudge.count += 1;
                match scope {
                    Scope::Global => {
                        if let Some(global) = self.state.global.as_mut() {
                            global.replies = global.replies.saturating_add(1);
                            let slot = global.replies_by_session.entry(counter_key).or_insert(0);
                            *slot = slot.saturating_add(1);
                            changed = true;
                        }
                    }
                    Scope::Session => {
                        if let Some(entry) = key.and_then(|k| self.state.sessions.get_mut(k)) {
                            entry.replies = entry.replies.saturating_add(1);
                            changed = true;
                        }
                    }
                }
                (
                    Verdict::Reply(AutoReply {
                        kind: AutoKind::Away,
                        text: guidance(
                            AutoKind::Away,
                            facts.whats_next,
                            note.as_deref(),
                            facts.lang,
                        ),
                    }),
                    changed,
                )
            }
        }
    }

    /// Applies one state-changing operation. Returns whether persisted state changed and, when a
    /// session just left mute, its key (the daemon queues the unmute notice for it).
    fn apply(&mut self, op: &AwayOp, now_ms: u64) -> (bool, Option<String>) {
        match op {
            AwayOp::State => (false, None),
            AwayOp::Start { note } => {
                let note = clean_note(note.as_deref());
                let next = match self.state.global.take() {
                    // Starting again keeps the original start time and counters; only the note moves.
                    Some(mut global) => {
                        global.note = note;
                        global
                    }
                    None => GlobalAway {
                        since_ms: now_ms,
                        note,
                        replies: 0,
                        replies_by_session: BTreeMap::new(),
                    },
                };
                let changed = self.state.global.as_ref() != Some(&next);
                self.state.global = Some(next);
                self.nudges.clear();
                (changed, None)
            }
            AwayOp::End => {
                let changed = self.state.global.take().is_some();
                // "Ask normally" overrides only mean something while globally away.
                let before = self.state.sessions.len();
                self.state
                    .sessions
                    .retain(|_, entry| entry.mode != SessionMode::Normal);
                self.nudges.clear();
                (changed || self.state.sessions.len() != before, None)
            }
            AwayOp::Session {
                session_key,
                setting,
                note,
            } => {
                let key = session_key.trim();
                if key.is_empty() {
                    return (false, None);
                }
                let was_muted = self
                    .state
                    .sessions
                    .get(key)
                    .is_some_and(|e| e.mode == SessionMode::Mute);
                let global_on = self.state.global.is_some();
                let mode = match setting {
                    SessionSetting::Follow => None,
                    SessionSetting::Away => Some(SessionMode::Away),
                    SessionSetting::Mute => Some(SessionMode::Mute),
                    SessionSetting::Normal if global_on => Some(SessionMode::Normal),
                    SessionSetting::Normal => None,
                };
                let before = self.state.sessions.get(key).cloned();
                match mode {
                    None => {
                        self.state.sessions.remove(key);
                    }
                    Some(mode) => {
                        // Re-selecting the same mode keeps its start time and counters.
                        let note = if mode == SessionMode::Away {
                            clean_note(note.as_deref())
                        } else {
                            None
                        };
                        let entry = match self.state.sessions.get(key) {
                            Some(existing) if existing.mode == mode => SessionEntry {
                                note,
                                last_seen_ms: now_ms,
                                ..existing.clone()
                            },
                            _ => SessionEntry {
                                mode,
                                since_ms: now_ms,
                                note,
                                replies: 0,
                                last_seen_ms: now_ms,
                            },
                        };
                        self.state.sessions.insert(key.to_string(), entry);
                    }
                }
                self.nudges.remove(key);
                let unmuted =
                    (was_muted && mode != Some(SessionMode::Mute)).then(|| key.to_string());
                // Touching `last_seen_ms` alone is not a change worth announcing.
                let settled = |entry: Option<SessionEntry>| {
                    entry.map(|mut entry| {
                        entry.last_seen_ms = 0;
                        entry
                    })
                };
                (
                    settled(self.state.sessions.get(key).cloned()) != settled(before),
                    unmuted,
                )
            }
        }
    }
}

// ===== Engine =====

/// The shared owner of the away state; one lives in the daemon's server state.
pub struct AwayEngine {
    inner: Mutex<Inner>,
    /// Serialises disk writes so the file always ends up holding the latest state.
    io: Mutex<()>,
    path: Option<PathBuf>,
}

impl AwayEngine {
    /// Loads the persisted state (missing or unreadable file means no away, no mute).
    pub fn load() -> Self {
        Self::load_from(&paths::away_file())
    }

    pub fn load_from(path: &Path) -> Self {
        Self {
            inner: Mutex::new(Inner::new(read_state_file_at(path))),
            io: Mutex::new(()),
            path: Some(path.to_path_buf()),
        }
    }

    /// An engine that never touches the disk (tests and tools).
    pub fn in_memory() -> Self {
        Self {
            inner: Mutex::new(Inner::new(AwayState::default())),
            io: Mutex::new(()),
            path: None,
        }
    }

    fn persist(&self) {
        let Some(path) = &self.path else { return };
        let _io = self.io.lock().unwrap();
        let state = self.inner.lock().unwrap().state.clone();
        write_state_file_at(path, &state);
    }

    /// Decides one question of an Agent. Callers check `is_agent_ask` first.
    pub fn judge(&self, facts: &AskFacts<'_>, now_ms: u64) -> Verdict {
        let (verdict, changed) = self.inner.lock().unwrap().judge(facts, now_ms);
        if changed {
            self.persist();
        }
        verdict
    }

    /// Convenience for a task: derives the facts and applies the exemptions of `is_agent_ask`.
    pub fn judge_task(&self, task: &TaskRequest, now_ms: u64) -> Verdict {
        if !is_agent_ask(task) {
            return Verdict::Deliver;
        }
        let key = session_key_of(task);
        self.judge(
            &AskFacts {
                session_key: key.as_deref(),
                whats_next: task.whats_next,
                escalate: task.escalate,
                lang: Lang::resolve(&task.lang),
            },
            now_ms,
        )
    }

    /// Applies one operation; returns the new snapshot, whether anything changed, and the key of a
    /// session that just left mute.
    pub fn apply(&self, op: &AwayOp, now_ms: u64) -> ApplyOutcome {
        let (changed, unmuted, snapshot) = {
            let mut inner = self.inner.lock().unwrap();
            let (changed, unmuted) = inner.apply(op, now_ms);
            (changed, unmuted, AwaySnapshot::from_state(&inner.state))
        };
        if changed {
            self.persist();
        }
        ApplyOutcome {
            changed,
            unmuted_session: unmuted,
            snapshot,
        }
    }

    pub fn snapshot(&self) -> AwaySnapshot {
        AwaySnapshot::from_state(&self.inner.lock().unwrap().state)
    }

    /// The effective setting of one session right now.
    pub fn effective(&self, key: Option<&str>) -> Effective {
        self.inner.lock().unwrap().state.effective(key)
    }

    pub fn is_away(&self) -> bool {
        self.inner.lock().unwrap().state.global.is_some()
    }

    /// Whether the global switch or any session's own setting is "away".
    pub fn any_away(&self) -> bool {
        self.inner.lock().unwrap().state.any_away()
    }

    /// The user asked this session to ask them: its next question is delivered (spec N2).
    pub fn arm_requested_ask(&self, key: &str, now_ms: u64) {
        if key.is_empty() {
            return;
        }
        let mut inner = self.inner.lock().unwrap();
        inner.requested.insert(key.to_string(), now_ms);
    }

    /// A session ended: its override and counters go away. Returns whether anything changed.
    pub fn session_ended(&self, key: &str) -> bool {
        let changed = {
            let mut inner = self.inner.lock().unwrap();
            inner.nudges.remove(key);
            inner.requested.remove(key);
            let mut changed = inner.state.sessions.remove(key).is_some();
            if let Some(global) = inner.state.global.as_mut() {
                changed |= global.replies_by_session.remove(key).is_some();
            }
            changed
        };
        if changed {
            self.persist();
        }
        changed
    }

    /// Periodic sweep: drops overrides of sessions that the registry no longer lists and that were
    /// last seen long ago (covers a missed session-end). Returns whether anything changed.
    pub fn sweep(&self, active_keys: &[String], now_ms: u64) -> bool {
        let changed = {
            let mut inner = self.inner.lock().unwrap();
            let before = inner.state.sessions.len();
            inner.state.sessions.retain(|key, entry| {
                active_keys.iter().any(|k| k == key)
                    || now_ms.saturating_sub(entry.last_seen_ms.max(entry.since_ms))
                        <= SESSION_RETENTION_MS
            });
            let mut changed = inner.state.sessions.len() != before;
            if let Some(global) = inner.state.global.as_mut() {
                let keep: Vec<String> = global
                    .replies_by_session
                    .keys()
                    .filter(|k| !k.is_empty() && !active_keys.iter().any(|a| &a == k))
                    .cloned()
                    .collect();
                // Counters of sessions that vanished are dropped with them.
                for key in keep {
                    if now_ms.saturating_sub(global.since_ms) > SESSION_RETENTION_MS {
                        changed |= global.replies_by_session.remove(&key).is_some();
                    }
                }
            }
            inner
                .requested
                .retain(|_, armed| now_ms.saturating_sub(*armed) <= REQUESTED_ASK_TTL_MS);
            changed
        };
        if changed {
            self.persist();
        }
        changed
    }
}

/// The result of `AwayEngine::apply`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplyOutcome {
    pub changed: bool,
    /// Set when the operation moved a session out of mute.
    pub unmuted_session: Option<String>,
    pub snapshot: AwaySnapshot,
}

// ===== Files =====

/// Reads the state file; anything missing, unreadable or from a newer schema is "no away".
pub fn read_state_file_at(path: &Path) -> AwayState {
    let Ok(text) = std::fs::read_to_string(path) else {
        return AwayState::default();
    };
    match serde_json::from_str::<AwayState>(&text) {
        Ok(state) if state.version <= STATE_VERSION => state,
        _ => AwayState::default(),
    }
}

fn write_state_file_at(path: &Path, state: &AwayState) {
    if state.is_idle() {
        // Nothing to remember: keep the state directory free of an empty file.
        let _ = std::fs::remove_file(path);
        return;
    }
    let mut state = state.clone();
    state.version = STATE_VERSION;
    let Ok(json) = serde_json::to_string_pretty(&state) else {
        return;
    };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let tmp = path.with_extension(format!("json.tmp-{}", uuid::Uuid::new_v4()));
    if std::fs::write(&tmp, json.as_bytes()).is_ok() && std::fs::rename(&tmp, path).is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
}

/// Hook helper: has the user turned AskHuman off for this Agent session? Reads the state file on
/// demand because hooks must not wait for the daemon.
pub fn session_muted(session_id: &str) -> bool {
    session_muted_at(&paths::away_file(), session_id)
}

pub fn session_muted_at(path: &Path, session_id: &str) -> bool {
    let session_id = session_id.trim();
    if session_id.is_empty() {
        return false;
    }
    matches!(
        read_state_file_at(path).effective(Some(&sid_key(session_id))),
        Effective::Mute { .. }
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn engine() -> AwayEngine {
        AwayEngine::in_memory()
    }

    fn facts<'a>(key: Option<&'a str>) -> AskFacts<'a> {
        AskFacts {
            session_key: key,
            whats_next: false,
            escalate: false,
            lang: Lang::En,
        }
    }

    fn start(e: &AwayEngine, now: u64) {
        e.apply(&AwayOp::Start { note: None }, now);
    }

    fn session(e: &AwayEngine, key: &str, setting: SessionSetting, now: u64) -> ApplyOutcome {
        e.apply(
            &AwayOp::Session {
                session_key: key.to_string(),
                setting,
                note: None,
            },
            now,
        )
    }

    #[test]
    fn normal_state_delivers_everything() {
        let e = engine();
        assert_eq!(e.judge(&facts(Some("sid:a")), 1), Verdict::Deliver);
        assert_eq!(e.judge(&facts(None), 1), Verdict::Deliver);
    }

    #[test]
    fn global_away_answers_with_guidance_that_names_the_escalation_flag() {
        let e = engine();
        start(&e, 10);
        let Verdict::Reply(reply) = e.judge(&facts(Some("sid:a")), 20) else {
            panic!("expected an automatic reply");
        };
        assert_eq!(reply.kind, AutoKind::Away);
        assert!(reply.text.contains("--escalate"));
        assert!(reply.text.contains("MCP: escalate: true"));
        let snap = e.snapshot();
        let global = snap.global.unwrap();
        assert_eq!(global.replies, 1);
        assert_eq!(global.replies_by_session.get("sid:a"), Some(&1));
    }

    #[test]
    fn whats_next_gets_its_own_guidance() {
        let e = engine();
        start(&e, 10);
        let mut f = facts(Some("sid:a"));
        f.whats_next = true;
        let Verdict::Reply(reply) = e.judge(&f, 20) else {
            panic!("expected an automatic reply");
        };
        assert!(reply.text.contains("next task"));
        assert!(reply.text.contains("--escalate"));
    }

    #[test]
    fn guidance_is_localised() {
        let e = engine();
        start(&e, 10);
        let mut f = facts(Some("sid:a"));
        f.lang = Lang::Zh;
        let Verdict::Reply(reply) = e.judge(&f, 20) else {
            panic!("expected an automatic reply");
        };
        assert!(reply.text.contains("用户已离开"));
        assert!(reply.text.contains("--escalate"));
    }

    #[test]
    fn note_follows_the_guidance_as_a_standing_instruction() {
        let e = engine();
        e.apply(
            &AwayOp::Start {
                note: Some("  keep the migration reversible  ".to_string()),
            },
            10,
        );
        let Verdict::Reply(reply) = e.judge(&facts(None), 20) else {
            panic!("expected an automatic reply");
        };
        assert!(reply.text.contains("keep the migration reversible"));
        assert!(!reply.text.contains("  keep"));
        assert!(
            reply.text.find("--escalate").unwrap() < reply.text.find("keep the migration").unwrap()
        );
    }

    #[test]
    fn note_is_bounded_and_blank_notes_vanish() {
        assert_eq!(clean_note(Some("   ")), None);
        assert_eq!(clean_note(None), None);
        let long = "x".repeat(NOTE_MAX_CHARS + 50);
        assert_eq!(
            clean_note(Some(&long)).unwrap().chars().count(),
            NOTE_MAX_CHARS
        );
    }

    #[test]
    fn escalation_flag_delivers_and_resets_the_anti_lockout_count() {
        let e = engine();
        start(&e, 10);
        assert!(matches!(
            e.judge(&facts(Some("sid:a")), 20),
            Verdict::Reply(_)
        ));
        assert!(matches!(
            e.judge(&facts(Some("sid:a")), 30),
            Verdict::Reply(_)
        ));
        let mut f = facts(Some("sid:a"));
        f.escalate = true;
        assert_eq!(e.judge(&f, 40), Verdict::Escalated(EscalationReason::Asked));
        // The count starts over after an escalation: three more plain asks are answered.
        for t in [50, 60, 70] {
            assert!(matches!(
                e.judge(&facts(Some("sid:a")), t),
                Verdict::Reply(_)
            ));
        }
        assert_eq!(
            e.judge(&facts(Some("sid:a")), 80),
            Verdict::Escalated(EscalationReason::Repeated)
        );
    }

    #[test]
    fn anti_lockout_lets_the_fourth_plain_ask_through_then_starts_over() {
        let e = engine();
        start(&e, 0);
        for t in 1..=3 {
            assert!(matches!(
                e.judge(&facts(Some("sid:a")), t),
                Verdict::Reply(_)
            ));
        }
        assert_eq!(
            e.judge(&facts(Some("sid:a")), 4),
            Verdict::Escalated(EscalationReason::Repeated)
        );
        // After the pass the count starts over.
        assert!(matches!(
            e.judge(&facts(Some("sid:a")), 5),
            Verdict::Reply(_)
        ));
    }

    #[test]
    fn anti_lockout_is_per_session_and_a_quiet_window_resets_it() {
        let e = engine();
        start(&e, 0);
        for t in 1..=3 {
            assert!(matches!(
                e.judge(&facts(Some("sid:a")), t),
                Verdict::Reply(_)
            ));
        }
        // Another session has its own count.
        assert!(matches!(
            e.judge(&facts(Some("sid:b")), 4),
            Verdict::Reply(_)
        ));
        // A long quiet gap starts session a over instead of letting its fourth ask through.
        let later = NUDGE_WINDOW_MS + 100;
        assert!(matches!(
            e.judge(&facts(Some("sid:a")), later),
            Verdict::Reply(_)
        ));
    }

    #[test]
    fn callers_without_a_session_follow_only_the_global_switch() {
        let e = engine();
        session(&e, "sid:a", SessionSetting::Away, 5);
        // Not globally away: a keyless caller is untouched even though a session is away.
        assert_eq!(e.judge(&facts(None), 10), Verdict::Deliver);
        start(&e, 11);
        assert!(matches!(e.judge(&facts(None), 12), Verdict::Reply(_)));
    }

    #[test]
    fn session_away_applies_to_that_session_only() {
        let e = engine();
        session(&e, "sid:a", SessionSetting::Away, 5);
        assert!(matches!(
            e.judge(&facts(Some("sid:a")), 10),
            Verdict::Reply(_)
        ));
        assert_eq!(e.judge(&facts(Some("sid:b")), 10), Verdict::Deliver);
        let snap = e.snapshot();
        assert_eq!(snap.sessions.len(), 1);
        assert_eq!(snap.sessions[0].replies, 1);
        assert!(snap.global.is_none());
    }

    #[test]
    fn mute_replies_with_the_stop_using_text_and_ignores_escalation() {
        let e = engine();
        session(&e, "sid:a", SessionSetting::Mute, 5);
        let mut f = facts(Some("sid:a"));
        f.escalate = true;
        let Verdict::Reply(reply) = e.judge(&f, 10) else {
            panic!("a muted session must get an automatic reply");
        };
        assert_eq!(reply.kind, AutoKind::Mute);
        assert!(reply.text.contains("turned AskHuman off"));
        assert!(!reply.text.contains("--escalate"));
        // whats-next is muted too.
        f.whats_next = true;
        assert!(matches!(e.judge(&f, 11), Verdict::Reply(r) if r.kind == AutoKind::Mute));
    }

    #[test]
    fn mute_is_never_let_through_by_the_anti_lockout_rule() {
        let e = engine();
        session(&e, "sid:a", SessionSetting::Mute, 5);
        for t in 10..30 {
            assert!(matches!(
                e.judge(&facts(Some("sid:a")), t),
                Verdict::Reply(r) if r.kind == AutoKind::Mute
            ));
        }
    }

    #[test]
    fn mute_beats_global_away_for_its_session() {
        let e = engine();
        start(&e, 1);
        session(&e, "sid:a", SessionSetting::Mute, 2);
        assert!(matches!(
            e.judge(&facts(Some("sid:a")), 3),
            Verdict::Reply(r) if r.kind == AutoKind::Mute
        ));
        assert!(matches!(
            e.judge(&facts(Some("sid:b")), 3),
            Verdict::Reply(r) if r.kind == AutoKind::Away
        ));
    }

    #[test]
    fn a_session_can_opt_out_of_global_away() {
        let e = engine();
        start(&e, 1);
        session(&e, "sid:a", SessionSetting::Normal, 2);
        assert_eq!(e.judge(&facts(Some("sid:a")), 3), Verdict::Deliver);
        assert!(matches!(
            e.judge(&facts(Some("sid:b")), 3),
            Verdict::Reply(_)
        ));
        // Ending global away drops the now meaningless override.
        e.apply(&AwayOp::End, 4);
        assert!(e.snapshot().sessions.is_empty());
    }

    #[test]
    fn any_away_counts_the_global_switch_and_sessions_set_to_away_but_not_mute() {
        let e = engine();
        assert!(!e.any_away());
        // Muting a session changes how its questions are handled, not whether the user is away.
        session(&e, "sid:a", SessionSetting::Mute, 1);
        assert!(!e.any_away());
        session(&e, "sid:b", SessionSetting::Away, 2);
        assert!(e.any_away());
        assert!(!e.is_away());
        session(&e, "sid:b", SessionSetting::Follow, 3);
        assert!(!e.any_away());
        start(&e, 4);
        assert!(e.any_away());
        assert!(e.is_away());
    }

    #[test]
    fn normal_without_global_away_is_the_same_as_following_it() {
        let e = engine();
        session(&e, "sid:a", SessionSetting::Away, 1);
        session(&e, "sid:a", SessionSetting::Normal, 2);
        assert!(e.snapshot().sessions.is_empty());
        assert_eq!(e.judge(&facts(Some("sid:a")), 3), Verdict::Deliver);
    }

    #[test]
    fn away_and_mute_are_exclusive_within_one_session() {
        let e = engine();
        session(&e, "sid:a", SessionSetting::Away, 1);
        session(&e, "sid:a", SessionSetting::Mute, 2);
        let snap = e.snapshot();
        assert_eq!(snap.sessions.len(), 1);
        assert_eq!(snap.sessions[0].mode, SessionMode::Mute);
    }

    #[test]
    fn leaving_mute_reports_the_session_for_the_unmute_notice() {
        let e = engine();
        session(&e, "sid:a", SessionSetting::Mute, 1);
        let out = session(&e, "sid:a", SessionSetting::Follow, 2);
        assert_eq!(out.unmuted_session.as_deref(), Some("sid:a"));
        assert!(out.changed);
        // Switching to another mode also leaves mute.
        session(&e, "sid:b", SessionSetting::Mute, 3);
        let out = session(&e, "sid:b", SessionSetting::Away, 4);
        assert_eq!(out.unmuted_session.as_deref(), Some("sid:b"));
        // Re-selecting mute does not.
        session(&e, "sid:c", SessionSetting::Mute, 5);
        let out = session(&e, "sid:c", SessionSetting::Mute, 6);
        assert_eq!(out.unmuted_session, None);
        assert!(!out.changed);
    }

    #[test]
    fn starting_again_keeps_the_start_time_and_counters() {
        let e = engine();
        start(&e, 100);
        e.judge(&facts(Some("sid:a")), 110);
        e.apply(
            &AwayOp::Start {
                note: Some("n".to_string()),
            },
            200,
        );
        let global = e.snapshot().global.unwrap();
        assert_eq!(global.since_ms, 100);
        assert_eq!(global.replies, 1);
        assert_eq!(global.note.as_deref(), Some("n"));
    }

    #[test]
    fn ending_away_clears_the_global_switch_and_reports_no_change_twice() {
        let e = engine();
        start(&e, 1);
        assert!(e.apply(&AwayOp::End, 2).changed);
        assert!(!e.apply(&AwayOp::End, 3).changed);
        assert!(!e.is_away());
        assert_eq!(e.judge(&facts(Some("sid:a")), 4), Verdict::Deliver);
    }

    #[test]
    fn requested_ask_is_delivered_once_even_while_away() {
        let e = engine();
        start(&e, 1);
        e.arm_requested_ask("sid:a", 10);
        assert_eq!(
            e.judge(&facts(Some("sid:a")), 20),
            Verdict::Escalated(EscalationReason::Requested)
        );
        // One-shot.
        assert!(matches!(
            e.judge(&facts(Some("sid:a")), 30),
            Verdict::Reply(_)
        ));
    }

    #[test]
    fn requested_ask_expires_and_does_not_unmute() {
        let e = engine();
        start(&e, 1);
        e.arm_requested_ask("sid:a", 10);
        assert!(matches!(
            e.judge(&facts(Some("sid:a")), 10 + REQUESTED_ASK_TTL_MS + 1),
            Verdict::Reply(_)
        ));
        session(&e, "sid:b", SessionSetting::Mute, 1);
        e.arm_requested_ask("sid:b", 10);
        assert!(matches!(
            e.judge(&facts(Some("sid:b")), 20),
            Verdict::Reply(r) if r.kind == AutoKind::Mute
        ));
    }

    #[test]
    fn session_end_clears_its_override_and_counters() {
        let e = engine();
        start(&e, 1);
        session(&e, "sid:a", SessionSetting::Mute, 2);
        e.judge(&facts(Some("sid:b")), 3);
        assert!(e.session_ended("sid:a"));
        assert!(e.snapshot().sessions.is_empty());
        assert!(e.session_ended("sid:b"));
        assert!(e.snapshot().global.unwrap().replies_by_session.is_empty());
        assert!(!e.session_ended("sid:a"));
    }

    #[test]
    fn sweep_drops_stale_overrides_of_unknown_sessions_only() {
        let e = engine();
        session(&e, "sid:old", SessionSetting::Mute, 1);
        session(&e, "sid:live", SessionSetting::Mute, 1);
        session(&e, "sid:fresh", SessionSetting::Mute, SESSION_RETENTION_MS);
        let now = SESSION_RETENTION_MS + 10;
        assert!(e.sweep(&["sid:live".to_string()], now));
        let keys: Vec<String> = e
            .snapshot()
            .sessions
            .into_iter()
            .map(|s| s.session_key)
            .collect();
        assert_eq!(keys, vec!["sid:fresh".to_string(), "sid:live".to_string()]);
        assert!(!e.sweep(&["sid:live".to_string()], now));
    }

    #[test]
    fn state_round_trips_through_the_file_and_survives_a_restart() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state").join("away.json");
        let e = AwayEngine::load_from(&path);
        e.apply(
            &AwayOp::Start {
                note: Some("be careful".to_string()),
            },
            100,
        );
        session(&e, "sid:a", SessionSetting::Mute, 110);
        e.judge(&facts(Some("sid:b")), 120);
        assert!(path.exists());

        let reloaded = AwayEngine::load_from(&path);
        assert_eq!(reloaded.snapshot(), e.snapshot());
        assert!(matches!(
            reloaded.judge(&facts(Some("sid:a")), 130),
            Verdict::Reply(r) if r.kind == AutoKind::Mute
        ));
        assert!(session_muted_at(&path, "a"));
        assert!(!session_muted_at(&path, "b"));
    }

    #[test]
    fn an_idle_state_leaves_no_file_behind() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("away.json");
        let e = AwayEngine::load_from(&path);
        start(&e, 1);
        assert!(path.exists());
        e.apply(&AwayOp::End, 2);
        assert!(!path.exists());
    }

    #[test]
    fn unreadable_or_newer_state_files_mean_no_away() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("away.json");
        std::fs::write(&path, "{not json").unwrap();
        assert!(read_state_file_at(&path).is_idle());
        std::fs::write(&path, r#"{"version":99,"global":{"sinceMs":1}}"#).unwrap();
        assert!(read_state_file_at(&path).is_idle());
        assert!(!session_muted_at(&path, "a"));
        assert!(!session_muted_at(&dir.path().join("missing.json"), "a"));
        assert!(!session_muted_at(&path, ""));
    }

    #[test]
    fn hooks_see_mute_but_not_session_level_away() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("away.json");
        let e = AwayEngine::load_from(&path);
        session(&e, "sid:away", SessionSetting::Away, 1);
        session(&e, "sid:mute", SessionSetting::Mute, 1);
        assert!(!session_muted_at(&path, "away"));
        assert!(session_muted_at(&path, "mute"));
    }

    #[test]
    fn op_wire_format_is_stable() {
        let op = AwayOp::Session {
            session_key: "sid:a".to_string(),
            setting: SessionSetting::Mute,
            note: None,
        };
        let json = serde_json::to_value(&op).unwrap();
        assert_eq!(
            json,
            serde_json::json!({"op":"session","sessionKey":"sid:a","setting":"mute"})
        );
        let back: AwayOp = serde_json::from_value(json).unwrap();
        assert_eq!(back, op);
        let start: AwayOp = serde_json::from_str(r#"{"op":"start"}"#).unwrap();
        assert_eq!(start, AwayOp::Start { note: None });
    }

    #[test]
    fn session_keys_prefer_the_agent_session_over_the_mcp_instance() {
        let mut task = sample_task();
        assert_eq!(session_key_of(&task), None);
        task.mcp_instance_id = Some("inst".to_string());
        assert_eq!(session_key_of(&task).as_deref(), Some("mcp:inst"));
        task.agent_session_id = Some("abc".to_string());
        assert_eq!(session_key_of(&task).as_deref(), Some("sid:abc"));
        task.agent_session_id = Some("   ".to_string());
        assert_eq!(session_key_of(&task).as_deref(), Some("mcp:inst"));
    }

    #[test]
    fn only_agent_questions_are_judged() {
        let e = engine();
        start(&e, 1);
        let mut task = sample_task();
        assert!(matches!(e.judge_task(&task, 2), Verdict::Reply(_)));

        // Stop confirmations, history-less cards and daemon-owned cards pass untouched.
        task.stop_confirmation = true;
        assert_eq!(e.judge_task(&task, 3), Verdict::Deliver);
        task.stop_confirmation = false;
        task.record_history = false;
        assert_eq!(e.judge_task(&task, 4), Verdict::Deliver);
        task.record_history = true;
        task.native_request_id = Some("failure:cursor:s:r".to_string());
        assert_eq!(e.judge_task(&task, 5), Verdict::Deliver);
    }

    #[test]
    fn judge_task_reads_escalate_whats_next_and_language_from_the_task() {
        let e = engine();
        start(&e, 1);
        let mut task = sample_task();
        task.whats_next = true;
        task.lang = "zh".to_string();
        let Verdict::Reply(reply) = e.judge_task(&task, 2) else {
            panic!("expected an automatic reply");
        };
        assert!(reply.text.contains("下一个任务"));
        task.escalate = true;
        assert_eq!(
            e.judge_task(&task, 3),
            Verdict::Escalated(EscalationReason::Asked)
        );
    }

    fn sample_task() -> TaskRequest {
        serde_json::from_value(serde_json::json!({
            "message": {"text": "hello", "files": []},
            "questions": [{"message": "q", "predefinedOptions": []}],
            "isMarkdown": false,
            "source": "the Loop",
            "lang": "en"
        }))
        .expect("a minimal task parses")
    }
}
