//! Daemon-owned final-failure cards. Passive hooks report without waiting for a decision.

use super::*;
use crate::agents::result::TurnResult;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;

/// How long a card waits for an answer, and how long a saved card stays worth bringing back.
const CARD_LIFETIME: Duration = Duration::from_secs(86400);
const MAX_CARDS: usize = 256;

#[derive(Default)]
pub(super) struct Cards {
    inner: Mutex<Inner>,
    /// Where unanswered cards are kept so a replacement daemon can bring them back; without a file
    /// the table lives in memory only.
    file: Option<PathBuf>,
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
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Saved {
    result: String,
    /// Anchors the card's lifetime across daemon replacements.
    created_ms: u64,
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
        Self {
            inner: Mutex::new(Inner {
                saved: load(&file, now_ms()),
                ..Inner::default()
            }),
            file: Some(file),
            frozen: AtomicBool::new(false),
        }
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
        inner.saved.insert(
            (kind, sid.into()),
            Saved {
                result: result.into(),
                created_ms,
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

    /// Writes the record: removals are skipped once frozen, but never what was added after.
    fn persist(&self, inner: &Inner) {
        let Some(file) = &self.file else {
            return;
        };
        let mut cards: Vec<SavedCard> = inner
            .saved
            .iter()
            .map(|((kind, sid), saved)| SavedCard {
                kind: kind.as_str().into(),
                session_id: sid.clone(),
                result_id: saved.result.clone(),
                created_ms: saved.created_ms,
            })
            .collect();
        cards.sort_by(|a, b| (&a.kind, &a.session_id).cmp(&(&b.kind, &b.session_id)));
        write(file, cards);
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
                },
            ))
        })
        .take(MAX_CARDS)
        .collect()
}

/// Atomic and best-effort: a failed write only costs the restoration of a card.
fn write(file: &std::path::Path, cards: Vec<SavedCard>) {
    if cards.is_empty() {
        let _ = std::fs::remove_file(file);
        return;
    }
    let Ok(json) = serde_json::to_vec_pretty(&SavedFile { cards }) else {
        return;
    };
    if let Some(parent) = file.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let tmp = file.with_extension(format!("json.tmp-{}", uuid::Uuid::new_v4()));
    if std::fs::write(&tmp, json).is_err() || std::fs::rename(&tmp, file).is_err() {
        let _ = std::fs::remove_file(&tmp);
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
        || !enabled(kind, sid)
        || !crate::development::session::native_binding_allowed(kind, sid)
        || kind == AgentKind::Codex && crate::agents::delegated::marker_silenced(sid).is_some()
    {
        return;
    }
    let Some(result) = state
        .agents
        .last_turn_result(kind, sid)
        .filter(|r| r.outcome.needs_attention() && !r.id.is_empty())
    else {
        return;
    };
    if !current(state, kind, sid, &result.id) {
        return;
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
    for Pending { kind, sid, result } in state.failure_confirmations.pending() {
        // Keep the session from aging out before the card's own refresh takes over.
        state.agents.touch_activity(kind, &sid, None);
        let card = state
            .agents
            .last_turn_result(kind, &sid)
            .filter(|r| r.id == result && r.outcome.needs_attention())
            .filter(|r| enabled(kind, &sid) && current(state, kind, &sid, &r.id));
        match card {
            Some(card) => start_card(state, kind, &sid, card),
            None => state.failure_confirmations.finish(kind, &sid, &result),
        }
    }
}

fn start_card(state: &Arc<ServerState>, kind: AgentKind, sid: &str, result: TurnResult) {
    let Some((cancel, created_ms)) = state.failure_confirmations.begin(kind, sid, &result.id)
    else {
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
    // A restored card has already spent part of its day.
    let remaining = CARD_LIFETIME
        .saturating_sub(Duration::from_millis(now_ms().saturating_sub(created_ms)))
        .max(Duration::from_secs(1));
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
            answer = tokio::time::timeout(remaining, ask) => answer.ok(),
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
                        feedback.native_request_id =
                            Some(crate::agents::failure::feedback_id(&result.id));
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
}
