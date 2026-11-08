//! Daemon-owned final-failure cards. Passive hooks report without waiting for a decision.

use super::*;
use crate::agents::result::TurnResult;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;

/// How long a card waits for an answer, and how long a saved card stays worth bringing back.
const CARD_LIFETIME: Duration = Duration::from_secs(86400);
const MAX_CARDS: usize = 256;
/// How many failed turns the daemon remembers having dealt with. An entry is a few dozen bytes; the
/// bound only keeps the record from growing for as long as the daemon runs.
const MAX_HANDLED: usize = 512;
/// Sits next to the card record but apart from it: that one empties out as cards are answered,
/// while the point of this one is to outlive them.
const HANDLED_FILE: &str = "failure-turns.json";

#[derive(Default)]
pub(super) struct Cards {
    inner: Mutex<Inner>,
    /// Where unanswered cards are kept so a replacement daemon can bring them back; without a file
    /// the table lives in memory only.
    file: Option<PathBuf>,
    /// Where the failed turns already dealt with are kept; without a file they live in memory only.
    handled_file: Option<PathBuf>,
    /// Set once the daemon is shutting down. Its requests are then cancelled for this process only,
    /// and that must not erase the cards the successor is going to restore.
    frozen: AtomicBool,
}

#[derive(Default)]
struct Inner {
    active: HashMap<(AgentKind, String), (String, Arc<tokio::sync::Notify>)>,
    seen: VecDeque<String>,
    /// Cards waiting for an answer, as persisted: one per session.
    saved: HashMap<(AgentKind, String), Saved>,
    /// Failed native turns the daemon has already dealt with, oldest first. A turn is named by its
    /// native ID, not by the result the registry minted for it: the registry gives the same failed
    /// turn a new result ID every time it accepts it afresh, which happens once it has forgotten
    /// the session and a poll reads the turn again.
    handled: VecDeque<Handled>,
}

impl Inner {
    /// `true` when `handled` was not remembered yet. The oldest entries make room for new ones.
    fn handle(&mut self, handled: Handled) -> bool {
        if self.handled.contains(&handled) {
            return false;
        }
        self.handled.push_back(handled);
        while self.handled.len() > MAX_HANDLED {
            self.handled.pop_front();
        }
        true
    }
}

/// A failed native turn of a session, as the daemon names it.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Handled {
    kind: AgentKind,
    sid: String,
    turn: String,
}

#[derive(Serialize, Deserialize)]
struct HandledFile {
    turns: Vec<HandledTurn>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct HandledTurn {
    kind: String,
    session_id: String,
    turn_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Saved {
    result: String,
    /// Anchors the card's lifetime across daemon replacements.
    created_ms: u64,
    attempt: Option<Attempt>,
    attempted: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
struct Attempt {
    phone: String,
    message: String,
}

#[derive(Serialize, Deserialize)]
struct SavedFile {
    cards: Vec<SavedCard>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SavedCard {
    kind: String,
    session_id: String,
    result_id: String,
    created_ms: u64,
    #[serde(default)]
    attempt: Option<Attempt>,
    #[serde(default)]
    attempted: bool,
}

/// A card that was waiting when the previous daemon went away.
struct Pending {
    kind: AgentKind,
    sid: String,
    result: String,
}

impl Cards {
    /// A table that keeps its unanswered cards in `file` and starts with the ones found there.
    pub(super) fn persistent(file: PathBuf) -> Self {
        let handled_file = file.with_file_name(HANDLED_FILE);
        let cards = Self {
            inner: Mutex::new(Inner {
                saved: load(&file, now_ms()),
                handled: load_handled(&handled_file),
                ..Inner::default()
            }),
            file: Some(file),
            handled_file: Some(handled_file),
            frozen: AtomicBool::new(false),
        };
        cards.persist(&cards.inner.lock().unwrap());
        cards
    }

    /// Whether this is the first time the failure of `turn` reaches the daemon. The turn is
    /// remembered either way, so a failure is dealt with once however often it is read again.
    fn first_sight(&self, kind: AgentKind, sid: &str, turn: &str) -> bool {
        let mut inner = self.inner.lock().unwrap();
        let fresh = inner.handle(Handled {
            kind,
            sid: sid.into(),
            turn: turn.into(),
        });
        if fresh {
            self.persist_handled(&inner);
        }
        fresh
    }

    /// Counts failures the registry already holds as dealt with. Whoever recorded them has been
    /// through `first_sight` already, unless it predates the ledger: this keeps those from coming
    /// back as new failures once the registry forgets their sessions.
    fn adopt(&self, turns: Vec<(AgentKind, String, String)>) {
        let mut inner = self.inner.lock().unwrap();
        let mut added = false;
        for (kind, sid, turn) in turns {
            added |= inner.handle(Handled { kind, sid, turn });
        }
        if added {
            self.persist_handled(&inner);
        }
    }

    /// Best-effort: a failed write only costs the protection across a restart.
    fn persist_handled(&self, inner: &Inner) {
        let Some(file) = &self.handled_file else {
            return;
        };
        let turns = inner
            .handled
            .iter()
            .map(|handled| HandledTurn {
                kind: handled.kind.as_str().into(),
                session_id: handled.sid.clone(),
                turn_id: handled.turn.clone(),
            })
            .collect();
        let _ = serde_json::to_vec_pretty(&HandledFile { turns })
            .map_err(std::io::Error::other)
            .and_then(|bytes| {
                crate::integrations::hook_edit::atomic_write_private(file, &bytes)
                    .map_err(std::io::Error::other)
            });
    }

    /// Registers the card for `result`: its cancel handle and creation time, or `None` when that
    /// result already had a card (or the table is full). A card restored from the record keeps its
    /// original creation time.
    fn begin(
        &self,
        kind: AgentKind,
        sid: &str,
        result: &str,
    ) -> Option<(Arc<tokio::sync::Notify>, u64)> {
        let mut inner = self.inner.lock().unwrap();
        let identity = format!("{}:{sid}:{result}", kind.as_str());
        if inner.seen.contains(&identity)
            || inner.saved.len() >= MAX_CARDS && !inner.saved.contains_key(&(kind, sid.into()))
            || inner.active.len() >= MAX_CARDS && !inner.active.contains_key(&(kind, sid.into()))
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
        let created_ms = inner
            .saved
            .get(&(kind, sid.to_string()))
            .filter(|saved| saved.result == result)
            .map_or_else(now_ms, |saved| saved.created_ms);
        let attempt = inner
            .saved
            .get(&(kind, sid.to_string()))
            .filter(|s| s.result == result)
            .and_then(|s| s.attempt.clone());
        let attempted = inner
            .saved
            .get(&(kind, sid.to_string()))
            .filter(|s| s.result == result)
            .is_some_and(|s| s.attempted);
        inner.saved.insert(
            (kind, sid.into()),
            Saved {
                result: result.into(),
                created_ms,
                attempt,
                attempted,
            },
        );
        self.persist(&inner);
        Some((cancel, created_ms))
    }

    /// Keeps a card this daemon will not show, because it is being replaced, for the successor.
    fn remember(&self, kind: AgentKind, sid: &str, result: &str) {
        let mut inner = self.inner.lock().unwrap();
        let key = (kind, sid.to_string());
        if inner
            .saved
            .get(&key)
            .is_some_and(|saved| saved.result == result)
            || inner.saved.len() >= MAX_CARDS && !inner.saved.contains_key(&key)
        {
            return;
        }
        inner.saved.insert(
            key,
            Saved {
                result: result.into(),
                created_ms: now_ms(),
                attempt: None,
                attempted: false,
            },
        );
        self.persist(&inner);
    }

    fn cancel(&self, kind: AgentKind, sid: &str) {
        let mut inner = self.inner.lock().unwrap();
        if let Some((_, cancel)) = inner.active.remove(&(kind, sid.into())) {
            cancel.notify_one();
        }
        if !self.frozen.load(Ordering::SeqCst) && inner.saved.remove(&(kind, sid.into())).is_some()
        {
            self.persist(&inner);
        }
    }

    fn finish(&self, kind: AgentKind, sid: &str, result: &str) {
        let mut inner = self.inner.lock().unwrap();
        let key = (kind, sid.to_string());
        if inner.active.get(&key).is_some_and(|(id, _)| id == result) {
            inner.active.remove(&key);
        }
        if !self.frozen.load(Ordering::SeqCst)
            && inner
                .saved
                .get(&key)
                .is_some_and(|saved| saved.result == result)
        {
            inner.saved.remove(&key);
            self.persist(&inner);
        }
    }

    /// The daemon is going down: from here on, cards ending is the shutdown's doing and says nothing
    /// about whether the user dealt with them, so the record keeps them.
    pub(super) fn freeze(&self) {
        self.frozen.store(true, Ordering::SeqCst);
    }

    /// Cards still worth showing from the record; expired ones are dropped.
    fn pending(&self) -> Vec<Pending> {
        let mut inner = self.inner.lock().unwrap();
        let before = inner.saved.len();
        let now = now_ms();
        inner
            .saved
            .retain(|_, saved| !expired(saved.created_ms, now));
        if inner.saved.len() != before && !self.frozen.load(Ordering::SeqCst) {
            self.persist(&inner);
        }
        inner
            .saved
            .iter()
            .map(|((kind, sid), saved)| Pending {
                kind: *kind,
                sid: sid.clone(),
                result: saved.result.clone(),
            })
            .collect()
    }

    pub(super) fn saved_count(&self) -> usize {
        self.inner.lock().unwrap().saved.len()
    }

    pub(super) fn retention_snapshot(&self) -> Vec<(AgentKind, String, String, u64)> {
        self.inner
            .lock()
            .unwrap()
            .saved
            .iter()
            .filter(|(_, s)| !expired(s.created_ms, now_ms()))
            .map(|((kind, sid), s)| {
                (
                    *kind,
                    sid.clone(),
                    s.result.clone(),
                    s.created_ms
                        .saturating_add(CARD_LIFETIME.as_millis() as u64),
                )
            })
            .collect()
    }

    fn reconcile_retention(&self, agents: &AgentRegistry) {
        // Keep the card table locked while reconciling, so a newly remembered failure cannot
        // lose its lease to an older snapshot. No registry method calls back into Cards.
        let inner = self.inner.lock().unwrap();
        let leases: Vec<_> = inner
            .saved
            .iter()
            .filter(|(_, s)| !expired(s.created_ms, now_ms()))
            .map(|((kind, sid), s)| {
                (
                    *kind,
                    sid.clone(),
                    s.result.clone(),
                    s.created_ms
                        .saturating_add(CARD_LIFETIME.as_millis() as u64),
                )
            })
            .collect();
        agents.reconcile_failure_retention(&leases);
    }

    fn saved(&self, kind: AgentKind, sid: &str) -> Option<Saved> {
        self.inner
            .lock()
            .unwrap()
            .saved
            .get(&(kind, sid.into()))
            .cloned()
    }

    fn attempt(&self, kind: AgentKind, sid: &str, result: &str, attempt: Option<Attempt>) -> bool {
        let mut inner = self.inner.lock().unwrap();
        let Some(saved) = inner
            .saved
            .get_mut(&(kind, sid.into()))
            .filter(|s| s.result == result)
        else {
            return false;
        };
        let previous = (saved.attempt.clone(), saved.attempted);
        saved.attempted |= attempt.is_some();
        saved.attempt = attempt;
        if self.persist(&inner) {
            return true;
        }
        let saved = inner.saved.get_mut(&(kind, sid.into())).unwrap();
        saved.attempt = previous.0;
        saved.attempted = previous.1;
        false
    }

    /// Writes the record: removals are skipped once frozen, but never what was added after.
    fn persist(&self, inner: &Inner) -> bool {
        let Some(file) = &self.file else {
            return true;
        };
        let mut cards: Vec<SavedCard> = inner
            .saved
            .iter()
            .map(|((kind, sid), saved)| SavedCard {
                kind: kind.as_str().into(),
                session_id: sid.clone(),
                result_id: saved.result.clone(),
                created_ms: saved.created_ms,
                attempt: saved.attempt.clone(),
                attempted: saved.attempted,
            })
            .collect();
        cards.sort_by(|a, b| (&a.kind, &a.session_id).cmp(&(&b.kind, &b.session_id)));
        write(file, cards).is_ok()
    }
}

fn expired(created_ms: u64, now: u64) -> bool {
    Duration::from_millis(now.saturating_sub(created_ms)) >= CARD_LIFETIME
}

fn load(file: &std::path::Path, now: u64) -> HashMap<(AgentKind, String), Saved> {
    let Ok(text) = std::fs::read_to_string(file) else {
        return HashMap::new();
    };
    let Ok(parsed) = serde_json::from_str::<SavedFile>(&text) else {
        return HashMap::new();
    };
    parsed
        .cards
        .into_iter()
        .filter_map(|card| {
            let kind = AgentKind::parse(&card.kind)?;
            (!card.session_id.is_empty()
                && !card.result_id.is_empty()
                && !expired(card.created_ms, now))
            .then_some((
                (kind, card.session_id),
                Saved {
                    result: card.result_id,
                    created_ms: card.created_ms,
                    attempt: card.attempt,
                    attempted: card.attempted,
                },
            ))
        })
        .take(MAX_CARDS)
        .collect()
}

fn load_handled(file: &std::path::Path) -> VecDeque<Handled> {
    let Ok(text) = std::fs::read_to_string(file) else {
        return VecDeque::new();
    };
    let Ok(parsed) = serde_json::from_str::<HandledFile>(&text) else {
        return VecDeque::new();
    };
    let mut inner = Inner::default();
    for turn in parsed.turns {
        let Some(kind) = AgentKind::parse(&turn.kind) else {
            continue;
        };
        if !turn.session_id.is_empty() && !turn.turn_id.is_empty() {
            inner.handle(Handled {
                kind,
                sid: turn.session_id,
                turn: turn.turn_id,
            });
        }
    }
    inner.handled
}

/// Atomic and best-effort: a failed write only costs the restoration of a card.
fn write(file: &std::path::Path, cards: Vec<SavedCard>) -> std::io::Result<()> {
    if cards.is_empty() {
        return match std::fs::remove_file(file) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        };
    }
    crate::integrations::hook_edit::atomic_write_private(
        file,
        &serde_json::to_vec_pretty(&SavedFile { cards })?,
    )
    .map_err(std::io::Error::other)
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
                && matches!(record["state"].as_str(), Some("idle" | "ended"))
                && record["lastTurnResult"]["id"] == result
                && matches!(
                    record["lastTurnResult"]["outcome"].as_str(),
                    Some("failed" | "incomplete")
                )
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

/// Each failed native turn is dealt with once, however many times it is read. Some sources are
/// polled for the latest turn of every session, and the registry hands the same failed turn a
/// new result ID each time it accepts it afresh, so the result ID cannot tell a new failure from an
/// old one read again; the native turn ID can. A result without one is taken as new.
fn owes_card(cards: &Cards, kind: AgentKind, sid: &str, result: &TurnResult) -> bool {
    result
        .turn_id
        .as_deref()
        .filter(|turn| !turn.is_empty())
        .is_none_or(|turn| cards.first_sight(kind, sid, turn))
}

/// The failed turns the registry holds a result for, as `(kind, session, turn)`.
fn failed_turns(snapshot: &serde_json::Value) -> Vec<(AgentKind, String, String)> {
    snapshot
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|record| {
            let kind = AgentKind::parse(record["kind"].as_str()?)?;
            let sid = record["sessionId"].as_str().filter(|sid| !sid.is_empty())?;
            let result = &record["lastTurnResult"];
            matches!(result["outcome"].as_str(), Some("failed" | "incomplete")).then_some(())?;
            let turn = result["turnId"].as_str().filter(|turn| !turn.is_empty())?;
            Some((kind, sid.into(), turn.into()))
        })
        .collect()
}

/// Only accepted observations enter here; an old round cannot cancel a new round's card.
pub(super) fn observe(state: &Arc<ServerState>, kind: AgentKind, sid: &str, event: LifecycleEvent) {
    if state.closing.load(Ordering::SeqCst) {
        return;
    }
    if matches!(
        event,
        LifecycleEvent::TurnStart | LifecycleEvent::SessionEnd
    ) {
        state.failure_confirmations.cancel(kind, sid);
        return;
    }
    if event != LifecycleEvent::TurnEnd {
        return;
    }
    let Some(result) = state
        .agents
        .last_turn_result(kind, sid)
        .filter(|r| r.outcome.needs_attention() && !r.id.is_empty())
    else {
        return;
    };
    // Before the gates below: a failure that earned no card, say because the switch was off, is
    // not a new one when it is read again later.
    if !owes_card(&state.failure_confirmations, kind, sid, &result) {
        return;
    }
    if !enabled(kind, sid)
        || !crate::development::session::native_binding_allowed(kind, sid)
        || kind == AgentKind::Codex && crate::agents::delegated::marker_silenced(sid).is_some()
    {
        return;
    }
    if !current(state, kind, sid, &result.id) {
        return;
    }
    state.failure_confirmations.remember(kind, sid, &result.id);
    if let Some(saved) = state.failure_confirmations.saved(kind, sid) {
        state.agents.retain_failure(
            kind,
            sid,
            &result.id,
            saved
                .created_ms
                .saturating_add(CARD_LIFETIME.as_millis() as u64),
        );
        state.agents.persist();
    }
    if state.draining.load(Ordering::SeqCst) {
        // This daemon is about to be replaced and shows no new cards: leave this one to its
        // successor instead of dropping it.
        state.failure_confirmations.remember(kind, sid, &result.id);
        return;
    }
    start_card(state, kind, sid, result);
}

/// Brings back the cards the previous daemon had not seen answered. A card whose result is no
/// longer the session's latest unhandled one is dropped: the user, or the Agent itself, moved on.
pub(super) fn restore(state: &Arc<ServerState>) {
    state
        .failure_confirmations
        .adopt(failed_turns(&state.agents.snapshot()));
    let pending = state.failure_confirmations.pending();
    state
        .failure_confirmations
        .reconcile_retention(&state.agents);
    state.agents.persist();
    for Pending { kind, sid, result } in pending {
        if let Some(saved) = state.failure_confirmations.saved(kind, &sid) {
            state.agents.retain_failure(
                kind,
                &sid,
                &result,
                saved
                    .created_ms
                    .saturating_add(CARD_LIFETIME.as_millis() as u64),
            );
        }
        let card = state
            .agents
            .last_turn_result(kind, &sid)
            .filter(|r| r.id == result && r.outcome.needs_attention())
            .filter(|r| enabled(kind, &sid) && current(state, kind, &sid, &r.id));
        match card {
            Some(card) => start_card(state, kind, &sid, card),
            None => {
                state.failure_confirmations.finish(kind, &sid, &result);
                state.agents.release_failure(kind, &sid, &result);
                state.agents.persist();
            }
        }
    }
}

fn supports_continue(kind: AgentKind, record: &serde_json::Value) -> bool {
    match kind {
        AgentKind::Cursor => {
            cfg!(target_os = "macos")
                && (record["lastTurnResult"]["source"] == "cursorLog"
                    || crate::agents::cursor_vscdb::session_metadata(
                        record["sessionId"].as_str().unwrap_or_default(),
                    )
                    .is_some())
        }
        AgentKind::Pi => true,
        AgentKind::Codex => crate::codex_desktop::is_desktop_session(record),
        _ => false,
    }
}

fn start_card(state: &Arc<ServerState>, kind: AgentKind, sid: &str, result: TurnResult) {
    let Some((cancel, created_ms)) = state.failure_confirmations.begin(kind, sid, &result.id)
    else {
        return;
    };
    let state = state.clone();
    let sid = sid.to_string();
    tokio::spawn(async move {
        let deadline_ms = created_ms.saturating_add(CARD_LIFETIME.as_millis() as u64);
        let mut feedback = String::new();
        loop {
            if state.closing.load(Ordering::SeqCst) {
                return;
            }
            if now_ms() >= deadline_ms
                || !current(&state, kind, &sid, &result.id)
                || !enabled(kind, &sid)
            {
                break;
            }
            let snapshot = state.agents.snapshot();
            let Some(record) = snapshot.as_array().and_then(|records| {
                records
                    .iter()
                    .find(|r| r["kind"] == kind.as_str() && r["sessionId"] == sid)
            }) else {
                break;
            };
            let project = record["cwd"]
                .as_str()
                .map(|cwd| crate::project::detect_from(std::path::Path::new(cwd)))
                .unwrap_or_default();
            let lang = Lang::current();
            let previous = state
                .failure_confirmations
                .saved(kind, &sid)
                .and_then(|s| s.attempt);
            if let Some(operation) = previous
                .as_ref()
                .and_then(|attempt| {
                    state
                        .agent_controls
                        .existing(&super::agent_control::operation_id(
                            &attempt.phone,
                            &attempt.message,
                        ))
                        .ok()
                        .flatten()
                })
                .filter(|operation| operation.status == "started")
            {
                if let Some(turn_id) = operation.turn_id {
                    let observation = crate::agents::result::Observation {
                        turn_id: Some(turn_id),
                        ..Default::default()
                    };
                    state.agents.apply_observed(
                        kind,
                        LifecycleEvent::TurnStart,
                        &sid,
                        None,
                        None,
                        0,
                        Some(&observation),
                    );
                }
                break;
            }
            let uncertain = previous.as_ref().is_some_and(|attempt| {
                let id = super::agent_control::operation_id(&attempt.phone, &attempt.message);
                match state.agent_controls.existing(&id) {
                    Ok(None) => false, // No durable journal means no dispatch could have happened.
                    Ok(Some(op)) => !matches!(op.status.as_str(), "rejected" | "started"),
                    Err(_) => true,
                }
            });
            if previous.is_some() && !uncertain {
                state
                    .failure_confirmations
                    .attempt(kind, &sid, &result.id, None);
            }
            let can_continue = supports_continue(kind, record) && !uncertain;
            let mut task =
                crate::agents::failure::task(kind, &sid, &project, &result, can_continue, lang);
            // A new delivery must not replay the answer to the previous delivery of this failure.
            let quiet_feedback = state
                .failure_confirmations
                .saved(kind, &sid)
                .is_some_and(|s| s.attempted)
                || !feedback.is_empty();
            let identity = if quiet_feedback {
                crate::agents::failure::feedback_id(&result.id)
            } else {
                task.native_request_id.take().unwrap()
            };
            task.native_request_id = Some(format!("{}:{}", identity, uuid::Uuid::new_v4()));
            if uncertain {
                task.questions[0].predefined_options[0].text = match lang {
                    Lang::Zh => "知道了",
                    Lang::En => "Got it",
                }
                .into();
                task.message.text.push_str(match lang {
                    Lang::Zh => "\n\n接续已提交，但尚未确认是否开始。请检查原会话；为避免重复执行，当前不会再次发送。",
                    Lang::En => "\n\nContinuation may have been submitted. Check the original chat; another send is blocked to avoid duplicates.",
                });
            } else if !feedback.is_empty() {
                task.message.text.push_str(&format!("\n\n{feedback}"));
            }
            if crate::development::verification::decorate_native_task(&mut task, kind, &sid)
                .is_err()
            {
                break;
            }
            let info = task.message.development.clone();
            let accepted = Mutex::new(None::<String>);
            let key = ios_agents::agent_key(kind, &sid);
            if let Some(gateway) = ios_gateway(&state) {
                gateway.hold_failure(&key, &result.id);
            }
            let ask = async {
                while state.registry.waiting_for_session(&sid) {
                    tokio::time::sleep(Duration::from_millis(250)).await;
                }
                crate::client::internal_ask(task, &accepted).await
            };
            let answer = tokio::select! {
                _ = cancel.notified() => None,
                answer = tokio::time::timeout(Duration::from_millis(deadline_ms.saturating_sub(now_ms()).max(1)), ask) => answer.ok(),
                _ = async { loop {
                    tokio::time::sleep(Duration::from_secs(2)).await;
                    if state.closing.load(Ordering::SeqCst) || !current(&state, kind, &sid, &result.id) || !enabled(kind, &sid) { break; }
                }} => None,
            };
            if let Some(info) = info.filter(|i| !i.relay) {
                match &answer {
                    Some((stdout, code)) => {
                        let _ = crate::development::verification::complete(&info, stdout, *code);
                    }
                    None => {
                        let _ =
                            crate::development::verification::interrupted(&info.worktree, &info.id);
                    }
                }
            }
            if state.closing.load(Ordering::SeqCst) {
                return;
            }
            let Some((stdout, code)) = answer else {
                break;
            };
            if code != 0 {
                // Infrastructure failure is not an acknowledgment. Leave the persistent card for
                // this daemon's next recovery, or its successor.
                if let Some(gateway) = ios_gateway(&state) {
                    gateway.release_failure(&key, &result.id);
                }
                tokio::select! {
                    _ = cancel.notified() => break,
                    _ = tokio::time::sleep(Duration::from_secs(15)) => {},
                }
                continue;
            }
            let Some(instruction) = crate::agents::failure::instruction(&stdout, can_continue)
            else {
                if let Some(gateway) = ios_gateway(&state) {
                    gateway.consume_failure(&key, &result.id);
                }
                break;
            };
            if !current(&state, kind, &sid, &result.id) {
                break;
            }
            let request_id = accepted.lock().unwrap().clone();
            let channel_is_ios = serde_json::from_str::<serde_json::Value>(&stdout)
                .ok()
                .is_some_and(|v| v["channel"] == "ios");
            let phone = if channel_is_ios {
                if let (Some(gateway), Some(id)) = (ios_gateway(&state), request_id) {
                    gateway
                        .answer_phone(&id)
                        .await
                        .unwrap_or_else(|| "failure-confirmation".into())
                } else {
                    "failure-confirmation".into()
                }
            } else {
                "failure-confirmation".into()
            };
            let attempt = Attempt {
                phone,
                message: uuid::Uuid::new_v4().to_string(),
            };
            if !state
                .failure_confirmations
                .attempt(kind, &sid, &result.id, Some(attempt.clone()))
            {
                feedback = match lang { Lang::Zh => "未能保存接续记录，没有发送。请检查磁盘空间后重试。", Lang::En => "Could not save the continuation record. Nothing was sent; check disk space and retry." }.into();
                continue;
            }
            let response = super::agent_control::send(
                &state,
                super::agent_control::Message {
                    phone: &attempt.phone,
                    message_id: &attempt.message,
                    kind,
                    sid: &sid,
                    text: "",
                    files: vec![],
                    expected_result_id: Some(result.id.clone()),
                    continuation_instruction: Some(&instruction),
                },
            )
            .await;
            if response.status == "delivered" {
                break;
            }
            let unknown = matches!(
                response.reason.as_deref(),
                Some(
                    "controlUnknown"
                        | "conflict"
                        | "interrupted"
                        | "desktopUnknown"
                        | "controlStarting"
                )
            );
            if !unknown {
                state
                    .failure_confirmations
                    .attempt(kind, &sid, &result.id, None);
            }
            feedback = crate::agents::failure::control_reason(response.reason.as_deref(), lang)
                .unwrap_or(match lang {
                    Lang::Zh => "没有发送。请恢复原会话后重试。",
                    Lang::En => "Nothing was sent. Restore the original chat and retry.",
                })
                .to_string();
        }
        state.failure_confirmations.finish(kind, &sid, &result.id);
        state.agents.release_failure(kind, &sid, &result.id);
        state.agents.persist();
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::result::{Observation, TurnOutcome};

    #[tokio::test]
    async fn duplicate_results_and_old_cleanup_cannot_replace_a_new_card() {
        let cards = Cards::default();
        let (old, _) = cards.begin(AgentKind::Codex, "s", "r1").unwrap();
        assert!(cards.begin(AgentKind::Codex, "s", "r1").is_none());
        let (new, _) = cards.begin(AgentKind::Codex, "s", "r2").unwrap();
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

    fn file(directory: &tempfile::TempDir) -> PathBuf {
        directory.path().join("state").join("failure-cards.json")
    }

    fn saved(cards: &Cards) -> Vec<(AgentKind, String, String)> {
        let mut saved: Vec<_> = cards
            .pending()
            .into_iter()
            .map(|pending| (pending.kind, pending.sid, pending.result))
            .collect();
        saved.sort_by(|a, b| a.1.cmp(&b.1));
        saved
    }

    #[test]
    fn an_unanswered_card_outlives_the_daemon_that_created_it() {
        let directory = tempfile::tempdir().unwrap();
        let path = file(&directory);
        let first = Cards::persistent(path.clone());
        let (_, created) = first.begin(AgentKind::Cursor, "s1", "r1").unwrap();
        first.begin(AgentKind::Codex, "s2", "r2").unwrap();
        // The next daemon sees both, and a card restored from the record keeps its age.
        let second = Cards::persistent(path.clone());
        assert_eq!(
            saved(&second),
            [
                (AgentKind::Cursor, "s1".into(), "r1".into()),
                (AgentKind::Codex, "s2".into(), "r2".into())
            ]
        );
        let (_, restored) = second.begin(AgentKind::Cursor, "s1", "r1").unwrap();
        assert_eq!(restored, created);
    }

    #[test]
    fn answered_replaced_and_revoked_cards_leave_the_record() {
        let directory = tempfile::tempdir().unwrap();
        let path = file(&directory);
        let cards = Cards::persistent(path.clone());
        cards.begin(AgentKind::Cursor, "answered", "r1").unwrap();
        cards.begin(AgentKind::Cursor, "replaced", "r1").unwrap();
        cards.begin(AgentKind::Cursor, "revoked", "r1").unwrap();
        cards.begin(AgentKind::Cursor, "kept", "r1").unwrap();
        cards.finish(AgentKind::Cursor, "answered", "r1");
        // A newer result for the session replaces the old card, so the old one's cleanup is moot.
        cards.begin(AgentKind::Cursor, "replaced", "r2").unwrap();
        cards.finish(AgentKind::Cursor, "replaced", "r1");
        cards.cancel(AgentKind::Cursor, "revoked");
        assert_eq!(
            saved(&Cards::persistent(path.clone())),
            [
                (AgentKind::Cursor, "kept".into(), "r1".into()),
                (AgentKind::Cursor, "replaced".into(), "r2".into())
            ]
        );
        cards.finish(AgentKind::Cursor, "kept", "r1");
        cards.cancel(AgentKind::Cursor, "replaced");
        // Nothing left: the record goes away with the last card.
        assert!(!path.exists());
    }

    #[tokio::test]
    async fn the_shutdown_cancelling_cards_does_not_erase_them() {
        let directory = tempfile::tempdir().unwrap();
        let path = file(&directory);
        let cards = Cards::persistent(path.clone());
        let (cancelled, _) = cards.begin(AgentKind::Cursor, "s1", "r1").unwrap();
        cards.freeze();
        // What the shutdown does to every request, as the card tasks see it.
        cards.cancel(AgentKind::Cursor, "s1");
        cards.finish(AgentKind::Cursor, "s1", "r1");
        tokio::time::timeout(Duration::from_millis(10), cancelled.notified())
            .await
            .unwrap();
        assert_eq!(
            saved(&Cards::persistent(path.clone())),
            [(AgentKind::Cursor, "s1".into(), "r1".into())]
        );
        // A card that appears while the daemon is going down is still kept for the successor.
        cards.remember(AgentKind::Codex, "s2", "r2");
        cards.begin(AgentKind::Grok, "s3", "r3").unwrap();
        assert_eq!(saved(&Cards::persistent(path)).len(), 3);
    }

    #[test]
    fn a_draining_daemon_hands_a_card_over_without_showing_it() {
        let directory = tempfile::tempdir().unwrap();
        let path = file(&directory);
        let cards = Cards::persistent(path.clone());
        cards.remember(AgentKind::Cursor, "s1", "r1");
        let first = std::fs::read(&path).unwrap();
        // Remembering the same result again changes nothing, not even its age.
        cards.remember(AgentKind::Cursor, "s1", "r1");
        assert_eq!(std::fs::read(&path).unwrap(), first);
        assert_eq!(
            saved(&Cards::persistent(path.clone())),
            [(AgentKind::Cursor, "s1".into(), "r1".into())]
        );
        // The Agent moving on revokes it.
        cards.cancel(AgentKind::Cursor, "s1");
        assert!(!path.exists());
    }

    #[test]
    fn stale_and_unusable_entries_are_not_restored() {
        let directory = tempfile::tempdir().unwrap();
        let path = file(&directory);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let now = now_ms();
        let day = CARD_LIFETIME.as_millis() as u64;
        let entry = |kind: &str, sid: &str, result: &str, created: u64| {
            serde_json::json!({
                "kind": kind, "sessionId": sid, "resultId": result, "createdMs": created
            })
        };
        std::fs::write(
            &path,
            serde_json::json!({"cards": [
                entry("cursor", "fresh", "r1", now - 1000),
                entry("cursor", "almost", "r1", now - day + 60_000),
                entry("cursor", "expired", "r1", now - day - 1),
                entry("nobody", "unknown-kind", "r1", now),
                entry("cursor", "", "r1", now),
                entry("cursor", "no-result", "", now),
            ]})
            .to_string(),
        )
        .unwrap();
        let cards = Cards::persistent(path.clone());
        assert_eq!(
            saved(&cards),
            [
                (AgentKind::Cursor, "almost".into(), "r1".into()),
                (AgentKind::Cursor, "fresh".into(), "r1".into())
            ]
        );
        for garbage in ["", "not json", "{\"cards\":\"x\"}", "{}"] {
            std::fs::write(&path, garbage).unwrap();
            assert!(
                saved(&Cards::persistent(path.clone())).is_empty(),
                "{garbage}"
            );
        }
        assert!(saved(&Cards::persistent(directory.path().join("missing"))).is_empty());
    }

    #[test]
    fn a_card_that_expires_while_waiting_is_dropped_from_the_record() {
        let directory = tempfile::tempdir().unwrap();
        let path = file(&directory);
        let cards = Cards::persistent(path.clone());
        cards.begin(AgentKind::Cursor, "old", "r1").unwrap();
        cards.begin(AgentKind::Cursor, "new", "r1").unwrap();
        {
            let mut inner = cards.inner.lock().unwrap();
            let key = (AgentKind::Cursor, "old".to_string());
            inner.saved.get_mut(&key).unwrap().created_ms =
                now_ms() - CARD_LIFETIME.as_millis() as u64 - 1;
            cards.persist(&inner);
        }
        assert_eq!(
            saved(&cards),
            [(AgentKind::Cursor, "new".into(), "r1".into())]
        );
        assert_eq!(
            saved(&Cards::persistent(path)),
            [(AgentKind::Cursor, "new".into(), "r1".into())]
        );
    }

    #[test]
    fn the_table_without_a_file_never_touches_the_disk() {
        let cards = Cards::default();
        cards.begin(AgentKind::Cursor, "s1", "r1").unwrap();
        cards.remember(AgentKind::Cursor, "s2", "r2");
        cards.freeze();
        cards.cancel(AgentKind::Cursor, "s1");
        assert!(cards.file.is_none());
    }

    #[test]
    fn the_table_is_bounded() {
        let cards = Cards::default();
        for index in 0..MAX_CARDS {
            cards
                .begin(AgentKind::Cursor, &format!("s{index}"), "r")
                .unwrap();
        }
        assert!(cards
            .begin(AgentKind::Cursor, "one-too-many", "r")
            .is_none());
        // A session that already has a card may still get its newer result.
        assert!(cards.begin(AgentKind::Cursor, "s0", "r2").is_some());
        cards.remember(AgentKind::Cursor, "one-too-many", "r");
        assert_eq!(cards.inner.lock().unwrap().saved.len(), MAX_CARDS);
    }
    #[test]
    fn continuation_attempt_and_original_deadline_survive_replacement() {
        let directory = tempfile::tempdir().unwrap();
        let path = file(&directory);
        let cards = Cards::persistent(path.clone());
        cards.remember(AgentKind::Cursor, "s", "r");
        let created = cards.saved(AgentKind::Cursor, "s").unwrap().created_ms;
        let attempt = Attempt {
            phone: "phone".into(),
            message: "click-one".into(),
        };
        assert!(cards.attempt(AgentKind::Cursor, "s", "r", Some(attempt.clone())));
        let restored = Cards::persistent(path);
        restored.begin(AgentKind::Cursor, "s", "r").unwrap();
        let saved = restored.saved(AgentKind::Cursor, "s").unwrap();
        assert_eq!(saved.attempt, Some(attempt));
        assert_eq!(saved.created_ms, created);
        assert!(!expired(created, created + 86_400_000 - 1));
        assert!(expired(created, created + 86_400_000));
        assert!(restored.attempt(AgentKind::Cursor, "s", "r", None));
        assert_eq!(
            restored.saved(AgentKind::Cursor, "s").unwrap().created_ms,
            created
        );
    }

    #[test]
    fn a_failed_attempt_write_never_authorizes_a_send() {
        let directory = tempfile::tempdir().unwrap();
        let blocker = directory.path().join("not-a-directory");
        std::fs::write(&blocker, "x").unwrap();
        let cards = Cards::persistent(blocker.join("cards.json"));
        cards.remember(AgentKind::Cursor, "s", "r");
        assert!(!cards.attempt(
            AgentKind::Cursor,
            "s",
            "r",
            Some(Attempt {
                phone: "phone".into(),
                message: "click".into()
            })
        ));
        assert!(cards
            .saved(AgentKind::Cursor, "s")
            .unwrap()
            .attempt
            .is_none());
    }

    fn handled_file(directory: &tempfile::TempDir) -> PathBuf {
        directory.path().join("state").join(HANDLED_FILE)
    }

    fn handled_entry(kind: &str, sid: &str, turn: &str) -> serde_json::Value {
        serde_json::json!({"kind": kind, "sessionId": sid, "turnId": turn})
    }

    fn failure(id: &str, turn: Option<&str>) -> TurnResult {
        TurnResult {
            id: id.into(),
            outcome: TurnOutcome::Failed,
            source: Some("codexDesktop".into()),
            turn_id: turn.map(Into::into),
            reason: None,
            summary: None,
            at: 0,
        }
    }

    /// What a Codex Desktop poll reports for a failed turn: its start, then its failure.
    fn poll_failed_turn(registry: &AgentRegistry, sid: &str, turn: &str) -> TurnResult {
        let start = Observation {
            turn_id: Some(turn.into()),
            ..Default::default()
        };
        let end = Observation {
            turn_id: Some(turn.into()),
            result: Some(failure("", Some(turn))),
            ..Default::default()
        };
        for (event, observation) in [
            (LifecycleEvent::TurnStart, &start),
            (LifecycleEvent::TurnEnd, &end),
        ] {
            registry.apply_observed(
                AgentKind::Codex,
                event,
                sid,
                None,
                None,
                0,
                Some(observation),
            );
        }
        registry.last_turn_result(AgentKind::Codex, sid).unwrap()
    }

    #[test]
    fn a_failed_turn_gets_one_card_however_often_it_is_read() {
        let cards = Cards::default();
        let codex = AgentKind::Codex;
        assert!(owes_card(&cards, codex, "s", &failure("r1", Some("t1"))));
        // The same turn again under a new result ID: how the registry takes it in afresh.
        assert!(!owes_card(&cards, codex, "s", &failure("r2", Some("t1"))));
        // A later turn, another session and another Agent are new failures.
        assert!(owes_card(&cards, codex, "s", &failure("r3", Some("t2"))));
        assert!(owes_card(
            &cards,
            codex,
            "other",
            &failure("r4", Some("t1"))
        ));
        assert!(owes_card(
            &cards,
            AgentKind::Cursor,
            "s",
            &failure("r5", Some("t1"))
        ));
        // Without a native turn there is nothing to tell two failures apart by.
        for turn in [None, Some("")] {
            assert!(owes_card(&cards, codex, "s", &failure("r6", turn)));
            assert!(owes_card(&cards, codex, "s", &failure("r7", turn)));
        }
    }

    #[test]
    fn a_session_the_registry_forgot_does_not_make_its_old_failure_new() {
        let cards = Cards::default();
        let registry = AgentRegistry::new();
        let first = poll_failed_turn(&registry, "s", "t1");
        assert!(owes_card(&cards, AgentKind::Codex, "s", &first));
        // While the registry remembers the session, polling the turn again changes nothing.
        assert_eq!(poll_failed_turn(&registry, "s", "t1").id, first.id);
        // Once it has forgotten it, the turn comes in as a failure with a result ID of its own.
        let again = poll_failed_turn(&AgentRegistry::new(), "s", "t1");
        assert_ne!(again.id, first.id);
        assert!(!owes_card(&cards, AgentKind::Codex, "s", &again));
        // A turn that really is new still is.
        let next = poll_failed_turn(&AgentRegistry::new(), "s", "t2");
        assert!(owes_card(&cards, AgentKind::Codex, "s", &next));
    }

    #[test]
    fn handled_turns_outlive_the_daemon_and_the_cards() {
        let directory = tempfile::tempdir().unwrap();
        let path = file(&directory);
        let first = Cards::persistent(path.clone());
        assert!(first.first_sight(AgentKind::Codex, "s", "t1"));
        first.begin(AgentKind::Codex, "s", "r1").unwrap();
        // Answering the card empties the card record, not the turns dealt with.
        first.finish(AgentKind::Codex, "s", "r1");
        assert!(!path.exists());
        let second = Cards::persistent(path);
        assert!(!second.first_sight(AgentKind::Codex, "s", "t1"));
        assert!(second.first_sight(AgentKind::Codex, "s", "t2"));
        assert!(!Cards::persistent(file(&directory)).first_sight(AgentKind::Codex, "s", "t2"));
    }

    #[test]
    fn the_oldest_handled_turns_are_forgotten_first() {
        let directory = tempfile::tempdir().unwrap();
        let path = file(&directory);
        let cards = Cards::persistent(path.clone());
        for index in 0..=MAX_HANDLED {
            assert!(cards.first_sight(AgentKind::Codex, "s", &format!("t{index}")));
        }
        let last = format!("t{MAX_HANDLED}");
        assert!(!cards.first_sight(AgentKind::Codex, "s", "t1"));
        assert!(!cards.first_sight(AgentKind::Codex, "s", &last));
        // The very first one made room for the rest.
        assert!(cards.first_sight(AgentKind::Codex, "s", "t0"));
        let record: HandledFile =
            serde_json::from_slice(&std::fs::read(handled_file(&directory)).unwrap()).unwrap();
        assert_eq!(record.turns.len(), MAX_HANDLED);
        assert!(!Cards::persistent(path).first_sight(AgentKind::Codex, "s", &last));
    }

    #[test]
    fn unusable_handled_entries_are_not_restored() {
        let directory = tempfile::tempdir().unwrap();
        let path = file(&directory);
        let record = handled_file(&directory);
        std::fs::create_dir_all(record.parent().unwrap()).unwrap();
        std::fs::write(
            &record,
            serde_json::json!({"turns": [
                handled_entry("codex", "s", "t1"),
                handled_entry("nobody", "s", "unknown-kind"),
                handled_entry("codex", "", "no-session"),
                handled_entry("codex", "s", ""),
            ]})
            .to_string(),
        )
        .unwrap();
        let cards = Cards::persistent(path.clone());
        assert_eq!(cards.inner.lock().unwrap().handled.len(), 1);
        assert!(!cards.first_sight(AgentKind::Codex, "s", "t1"));
        for garbage in ["", "not json", "{\"turns\":\"x\"}", "{}"] {
            std::fs::write(&record, garbage).unwrap();
            assert!(Cards::persistent(path.clone())
                .inner
                .lock()
                .unwrap()
                .handled
                .is_empty());
        }
    }

    #[test]
    fn failures_already_on_record_are_adopted_as_handled() {
        let snapshot = serde_json::json!([
            {"kind": "codex", "sessionId": "failed", "state": "idle",
                "lastTurnResult": {"id": "r1", "outcome": "failed", "turnId": "t1", "at": 1}},
            {"kind": "cursor", "sessionId": "incomplete", "state": "ended",
                "lastTurnResult": {"id": "r2", "outcome": "incomplete", "turnId": "t2", "at": 1}},
            {"kind": "codex", "sessionId": "completed",
                "lastTurnResult": {"id": "r3", "outcome": "completed", "turnId": "t3", "at": 1}},
            {"kind": "codex", "sessionId": "no-turn",
                "lastTurnResult": {"id": "r4", "outcome": "failed", "at": 1}},
            {"kind": "codex", "sessionId": "no-result"},
            {"kind": "nobody", "sessionId": "unknown-kind",
                "lastTurnResult": {"id": "r5", "outcome": "failed", "turnId": "t5", "at": 1}},
        ]);
        let turns = failed_turns(&snapshot);
        assert_eq!(
            turns,
            [
                (AgentKind::Codex, "failed".into(), "t1".into()),
                (AgentKind::Cursor, "incomplete".into(), "t2".into())
            ]
        );
        let directory = tempfile::tempdir().unwrap();
        let cards = Cards::persistent(file(&directory));
        cards.adopt(turns);
        assert!(!cards.first_sight(AgentKind::Codex, "failed", "t1"));
        assert!(!cards.first_sight(AgentKind::Cursor, "incomplete", "t2"));
        assert!(cards.first_sight(AgentKind::Codex, "completed", "t3"));
        assert!(!Cards::persistent(file(&directory)).first_sight(AgentKind::Codex, "failed", "t1"));
    }

    #[test]
    fn the_registry_snapshot_names_the_failed_turn() {
        let registry = AgentRegistry::new();
        poll_failed_turn(&registry, "s", "t1");
        assert_eq!(
            failed_turns(&registry.snapshot()),
            [(AgentKind::Codex, "s".into(), "t1".into())]
        );
    }

    #[test]
    fn handled_turns_never_touch_the_disk_without_a_file() {
        let cards = Cards::default();
        assert!(cards.first_sight(AgentKind::Codex, "s", "t1"));
        cards.adopt(vec![(AgentKind::Codex, "s".into(), "t2".into())]);
        assert!(cards.handled_file.is_none());
    }
}
