//! Questions and confirmations on their way to the phones: delivery (MQTT while the app is in
//! the foreground; otherwise the question waits in the phone's inbox and a push follows once the
//! Mac has been idle long enough, see `docs/specs/ios-alert-timing.md`), answers coming back, and
//! the resolved/expired bookkeeping (plan §4.4–§4.6).

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

use serde_json::json;
use sha2::{Digest, Sha256};

use super::{Actor, TodoPick};
use crate::daemon::away::{Directive, DirectiveKind, Scope};
use crate::ios::envelope;
use crate::ios::push::PushOutcome;
use crate::ios::store::{JournalEntry, JournalLookup, JournalResult};
use crate::ios::views;
use crate::ios::views::AssetSource;
use crate::ios::wire::Msg;
use crate::ios::wire::{
    self, t, AnswerAck, AnswerSubmit, ConfirmAck, ConfirmDecide, PushPlain, RelayPush,
    RequestSummary, RequestView, INLINE_REQUEST_BYTES, MAX_PLAIN_BYTES, PUSH_FALLBACK_SECS,
    PUSH_PREVIEW_MAX_CHARS, PUSH_SESSION_MAX_CHARS,
};
use crate::models::{
    AskRequest, ChannelAction, ChannelResult, ImageAttachment, QuestionAnswer, TodoSelection,
};
use base64::Engine;

/// How long a request may stay undeliverable (broker down, no push) before the surface is
/// reported lost.
const UNREACHABLE_GRACE_MS: i64 = 5_000;
/// A confirmation that outlived its deadline by this much is expired locally even if the
/// daemon never said so.
const EXPIRY_GRACE_MS: i64 = 5_000;
const RECENT_KEEP: usize = 200;
const RECENT_MAX_AGE_MS: i64 = 3_600_000;
const MAX_ANSWER_TEXT_CHARS: usize = 64 * 1024;

// ---------------------------------------------------------------- daemon-facing types

/// The question side of a request: where answers go.
pub trait AskSink: Send + Sync + 'static {
    fn operation(&self) -> Result<Option<crate::daemon::recovery::Operation>, ()> {
        Ok(None)
    }
    /// Feedback belongs in the inbox but must not ring for the same failure again.
    fn quiet_feedback(&self) -> bool {
        false
    }
    /// Submits the answer; `true` when this submission decided the request.
    fn try_submit(&self, result: ChannelResult) -> bool;
    fn try_submit_identified(
        &self,
        result: ChannelResult,
        _receipt: &crate::daemon::recovery::Receipt,
        directive: Option<crate::daemon::away::Directive>,
    ) -> bool {
        let carries_directive = directive.is_some() && result.action == ChannelAction::Send;
        if carries_directive {
            self.set_directive(directive);
        }
        let accepted = self.try_submit(result);
        if !accepted && carries_directive {
            self.set_directive(None);
        }
        accepted
    }
    /// Channel that decided the request, once it is decided.
    fn winner(&self) -> Option<String>;
    /// The surface can no longer reach a phone; the request keeps waiting elsewhere.
    fn surface_lost(&self, reason: &str);
    /// Stashes (or, with `None`, drops) the directive that came with the answer about to be
    /// submitted; the daemon applies it only if that answer wins the request. Sinks of requests
    /// that cannot carry one ignore it.
    fn set_directive(&self, _directive: Option<crate::daemon::away::Directive>) {}
}

/// The confirmation side of a request.
pub trait ConfirmSink: Send + Sync + 'static {
    fn operation(&self) -> Result<Option<crate::daemon::recovery::Operation>, ()> {
        Ok(None)
    }
    /// Explicit development relays cannot wait for the daily idle notification policy.
    fn immediate_notification(&self) -> bool {
        false
    }
    /// `Ok(true)` decided, `Ok(false)` already decided, `Err` invalid input.
    fn decide(&self, wire_index: usize, comment: Option<String>) -> Result<bool, String>;
    fn decide_identified(
        &self,
        wire_index: usize,
        comment: Option<String>,
        _receipt: &crate::daemon::recovery::Receipt,
    ) -> Result<bool, String> {
        self.decide(wire_index, comment)
    }
    fn winner(&self) -> Option<String>;
    fn surface_lost(&self, reason: &str);
}

#[derive(Clone)]
pub enum Sink {
    Ask(Arc<dyn AskSink>),
    Confirm(Arc<dyn ConfirmSink>),
}

impl Sink {
    fn operation(&self) -> Result<Option<crate::daemon::recovery::Operation>, ()> {
        match self {
            Self::Ask(sink) => sink.operation(),
            Self::Confirm(sink) => sink.operation(),
        }
    }
    fn winner(&self) -> Option<String> {
        match self {
            Sink::Ask(s) => s.winner(),
            Sink::Confirm(s) => s.winner(),
        }
    }

    fn surface_lost(&self, reason: &str) {
        match self {
            Sink::Ask(s) => s.surface_lost(reason),
            Sink::Confirm(s) => s.surface_lost(reason),
        }
    }
}

pub struct DeliverRequest {
    pub view: RequestView,
    pub sink: Sink,
    /// The original question (answers are mapped back onto it).
    pub ask: Option<Arc<AskRequest>>,
    /// Unix milliseconds after which the request is void (confirmations).
    pub deadline_ms: Option<i64>,
    /// Files the phone may download while the request is pending.
    pub assets: Vec<AssetSource>,
}

/// How a request ended, as far as the phones are concerned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resolution {
    /// The iOS channel answered it, without knowing which phone did.
    Answered,
    /// A specific phone answered it (its id): that phone is told "answered", the others
    /// "answered elsewhere" with its name.
    AnsweredByPhone(String),
    /// Another channel answered it (display name of that channel).
    AnsweredElsewhere(String),
    Cancelled(String),
    Expired,
}

impl Resolution {
    fn wire(&self) -> (&'static str, Option<String>) {
        match self {
            Resolution::Answered | Resolution::AnsweredByPhone(_) => ("answered", None),
            Resolution::AnsweredElsewhere(by) => ("answeredElsewhere", Some(by.clone())),
            Resolution::Cancelled(by) => ("cancelled", Some(by.clone())),
            Resolution::Expired => ("expired", None),
        }
    }
}

// ---------------------------------------------------------------- bookkeeping

#[derive(Clone, Default, serde::Serialize, serde::Deserialize)]
pub(crate) struct PhoneDelivery {
    pub mqtt_sent_ms: Option<i64>,
    pub acked: bool,
    pub pushed: bool,
    pub seen: bool,
}

pub(crate) struct Pending {
    pub view: RequestView,
    pub summary: RequestSummary,
    pub sink: Sink,
    pub ask: Option<Arc<AskRequest>>,
    pub deadline_ms: Option<i64>,
    pub assets: Vec<AssetSource>,
    pub unreachable_since: Option<i64>,
    pub phones: HashMap<String, PhoneDelivery>,
}

struct Recent {
    id: String,
    how: &'static str,
    by: Option<String>,
    /// The phone that answered, when one did.
    winner: Option<String>,
    ts: i64,
}

#[derive(Default)]
pub(crate) struct Requests {
    pub pending: Vec<Pending>,
    recent: VecDeque<Recent>,
}

impl Requests {
    pub fn pending_count(&self) -> usize {
        self.pending.len()
    }

    pub fn find(&self, id: &str) -> Option<&Pending> {
        self.pending.iter().find(|p| p.view.id == id)
    }

    pub fn find_mut(&mut self, id: &str) -> Option<&mut Pending> {
        self.pending.iter_mut().find(|p| p.view.id == id)
    }

    /// A downloadable file of any pending request.
    pub fn find_asset(&self, asset_id: &str) -> Option<&AssetSource> {
        self.pending
            .iter()
            .flat_map(|p| p.assets.iter())
            .find(|a| a.asset_id == asset_id)
    }

    pub fn forget_phone(&mut self, phone_id: &str) {
        for pending in &mut self.pending {
            pending.phones.remove(phone_id);
        }
    }

    fn remember(
        &mut self,
        id: &str,
        how: &'static str,
        by: Option<String>,
        winner: Option<String>,
        now: i64,
    ) {
        self.recent.push_back(Recent {
            id: id.to_string(),
            how,
            by,
            winner,
            ts: now,
        });
        while self.recent.len() > RECENT_KEEP
            || self
                .recent
                .front()
                .is_some_and(|r| now - r.ts > RECENT_MAX_AGE_MS)
        {
            self.recent.pop_front();
        }
    }

    fn recent(&self, id: &str) -> Option<&Recent> {
        self.recent.iter().find(|r| r.id == id)
    }

    pub(super) fn answer_phone(&self, id: &str) -> Option<String> {
        self.recent(id).and_then(|result| result.winner.clone())
    }
}

// ---------------------------------------------------------------- view helpers

fn truncate_chars(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let cut: String = text.chars().take(max).collect();
    format!("{cut}…")
}

/// The session title as a notification carries it: shorter than in the app, one line is all a
/// lock screen gives it.
fn push_session(view: &RequestView) -> Option<String> {
    view.session_title
        .as_deref()
        .map(|title| truncate_chars(title, PUSH_SESSION_MAX_CHARS))
}

/// First non-empty line, shortened for notifications and list rows.
pub(crate) fn preview_of(view: &RequestView) -> String {
    let source = match (&view.ask, &view.confirm) {
        (Some(ask), _) => {
            let from_message = ask.message.text.trim();
            if !from_message.is_empty() {
                from_message.to_string()
            } else {
                ask.questions
                    .first()
                    .map(|q| q.text.clone())
                    .unwrap_or_default()
            }
        }
        (None, Some(confirm)) => {
            if confirm.detail.summary.trim().is_empty() {
                confirm.title.clone()
            } else {
                confirm.detail.summary.clone()
            }
        }
        _ => String::new(),
    };
    let line = source
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("");
    truncate_chars(line, PUSH_PREVIEW_MAX_CHARS)
}

fn short_hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))[..16].to_string()
}

/// Keeps the first `keep` thumbnails of a view; `true` when any was dropped.
fn drop_thumbs(view: &mut RequestView, keep: usize) -> bool {
    let mut dropped = false;
    if let Some(ask) = view.ask.as_mut() {
        let mut kept = 0;
        for file in &mut ask.message.files {
            if file.thumb.is_some() {
                if kept < keep {
                    kept += 1;
                } else {
                    file.thumb = None;
                    dropped = true;
                }
            }
        }
    }
    dropped
}

/// Shrinks a view until its JSON fits comfortably into one message: thumbnails go first (they
/// are only a convenience), then the long texts are halved.
fn fit_view(mut view: RequestView) -> (RequestView, Vec<u8>) {
    let budget = MAX_PLAIN_BYTES - 16 * 1024;
    for _ in 0..24 {
        let json = serde_json::to_vec(&view).expect("view serializes");
        if json.len() <= budget {
            return (view, json);
        }
        if drop_thumbs(&mut view, 6) || drop_thumbs(&mut view, 0) {
            continue;
        }
        view.truncated = true;
        let halve = |text: &mut String| {
            if text.chars().count() > 256 {
                let keep = text.chars().count() / 2;
                *text = truncate_chars(text, keep);
            }
        };
        if let Some(ask) = view.ask.as_mut() {
            halve(&mut ask.message.text);
            for q in &mut ask.questions {
                halve(&mut q.text);
            }
        }
        if let Some(confirm) = view.confirm.as_mut() {
            halve(&mut confirm.detail.body_md);
        }
    }
    let json = serde_json::to_vec(&view).expect("view serializes");
    (view, json)
}

/// The compact form sent inline when the full request is large; the phone fetches the rest.
fn light_view(view: &RequestView) -> RequestView {
    let mut light = view.clone();
    drop_thumbs(&mut light, 0);
    if let Some(ask) = light.ask.as_mut() {
        ask.message.text = truncate_chars(&ask.message.text, 1_000);
        ask.questions.clear();
    }
    if let Some(confirm) = light.confirm.as_mut() {
        confirm.detail.body_md = truncate_chars(&confirm.detail.body_md, 1_000);
    }
    light
}

fn summarize(view: &RequestView, size: u64) -> RequestSummary {
    RequestSummary {
        id: view.id.clone(),
        kind: view.kind.clone(),
        project: view.project.clone(),
        agent: view.agent.clone(),
        session_title: view.session_title.clone(),
        created_at: view.created_at,
        preview: preview_of(view),
        content_hash: view.content_hash.clone(),
        size,
        escalated: view.escalated,
    }
}

/// The directive a phone sent with its answer, checked. A directive this Mac cannot read is
/// refused rather than dropped: the user asked for something that would silently not happen.
fn directive_of(submit: &AnswerSubmit) -> Result<Option<Directive>, String> {
    let Some(wire) = &submit.directive else {
        return Ok(None);
    };
    let kind = match wire.kind.as_str() {
        "away" => DirectiveKind::Away,
        "mute" => DirectiveKind::Mute,
        _ => return Err("unknown directive".into()),
    };
    let scope = match wire.scope.as_str() {
        "global" => Scope::Global,
        "session" => Scope::Session,
        _ => return Err("unknown directive scope".into()),
    };
    Ok(Some(Directive {
        kind,
        scope,
        note: wire.note.clone(),
    }))
}

fn answer_hash(submit: &AnswerSubmit) -> String {
    let mut canonical = json!({
        "requestId": submit.request_id,
        "action": submit.action,
        "answers": submit.answers,
    });
    // Left out when empty, so the hash of an answer without todos is what it always was: a repeat
    // that crosses an upgrade of the Mac still replays instead of being called a conflict.
    if !submit.todo_ids.is_empty() {
        canonical["todoIds"] = json!(submit.todo_ids);
    }
    if let Some(directive) = &submit.directive {
        canonical["directive"] = json!(directive);
    }
    // Same rule for notes on the message: an answer without any hashes as it always did.
    if !submit.annotations.is_empty() {
        canonical["annotations"] = json!(submit.annotations);
    }
    short_hash(&serde_json::to_vec(&canonical).expect("answer serializes"))
}

/// Puts the picked todos into the answers the way the popup does (`collectAnswers` in
/// `usePopupCore.ts`), so the Agent reads the same words whichever device answered, and fills the
/// ids and attachment snapshots the Coordinator dequeues and delivers from.
///
/// A question: the todos go into the last one, after what the person wrote, each on a line of its
/// own. What's next: the todo is the next task itself, so its text leads and what the person wrote
/// follows (the popup's handling of a todo of the refreshed queue).
fn merge_todo_picks(
    ask: &AskRequest,
    answers: &mut [QuestionAnswer],
    picks: Vec<TodoPick>,
    merge_line: impl Fn(&str) -> String,
) {
    let Some(first) = picks.first() else { return };
    let Some(last) = answers.len().checked_sub(1) else {
        return;
    };
    let index = if ask.whats_next { 0 } else { last };
    let answer = &mut answers[index];
    let typed = answer.user_input.take();
    let parts: Vec<String> = if ask.whats_next {
        std::iter::once(first.text.clone()).chain(typed).collect()
    } else {
        typed
            .into_iter()
            .chain(picks.iter().map(|pick| merge_line(&pick.text)))
            .collect()
    };
    let merged = parts
        .into_iter()
        .filter(|part| !part.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n\n");
    answer.user_input = Some(merged).filter(|text| !text.is_empty());
    answer.todo_ids = picks.iter().map(|pick| pick.id.clone()).collect();
    answer.todo_selections = picks
        .into_iter()
        .map(|pick| TodoSelection {
            id: pick.id,
            attachments: pick.attachments,
        })
        .collect();
}

fn decide_hash(decide: &ConfirmDecide) -> String {
    let canonical = json!({
        "requestId": decide.request_id,
        "wireIndex": decide.wire_index,
        "dismiss": decide.dismiss,
        "input": decide.input,
    });
    short_hash(&serde_json::to_vec(&canonical).expect("decision serializes"))
}

// ---------------------------------------------------------------- actor side

impl Actor {
    pub(crate) fn inbox_snapshot(&self) -> wire::InboxSnapshot {
        wire::InboxSnapshot {
            epoch: self.epoch.clone(),
            rev: self.revs.inbox,
            items: self
                .requests
                .pending
                .iter()
                .map(|p| p.summary.clone())
                .collect(),
        }
    }

    pub(super) fn is_live(&self, phone_id: &str) -> bool {
        self.rt
            .get(phone_id)
            .is_some_and(|rt| rt.online && rt.foreground && rt.hello_at.is_some())
    }

    pub(super) async fn deliver(&mut self, req: DeliverRequest) {
        let now = self.now();
        if self.requests.find(&req.view.id).is_some() {
            return;
        }
        // Decided elsewhere while the view was being prepared.
        if req.sink.winner().is_some() {
            return;
        }
        let targets: Vec<String> = self
            .store
            .state
            .active_phones()
            .map(|p| p.phone_id.clone())
            .collect();
        if targets.is_empty() {
            req.sink.surface_lost("no paired phone");
            return;
        }

        let mut view = req.view;
        view.content_hash = String::new();
        let (view, json) = fit_view(view);
        let view = RequestView {
            content_hash: short_hash(&json),
            ..view
        };
        let summary = summarize(&view, json.len() as u64);
        self.revs.inbox += 1;

        let mut pending = Pending {
            view,
            summary,
            sink: req.sink,
            ask: req.ask,
            deadline_ms: req.deadline_ms,
            assets: req.assets,
            unreachable_since: None,
            phones: HashMap::new(),
        };
        let inline = json.len() <= INLINE_REQUEST_BYTES;
        let rev = self.revs.inbox;
        let mut reached = false;
        let mut recovered = self
            .recovered_deliveries
            .remove(&pending.view.id)
            .unwrap_or_default();
        for phone_id in &targets {
            let mut delivery = recovered.remove(phone_id).unwrap_or_default();
            if self.is_live(phone_id) && self.connected {
                let request = if inline {
                    pending.view.clone()
                } else {
                    light_view(&pending.view)
                };
                let sent = self.send(
                    phone_id,
                    &wire::RequestNew {
                        rev,
                        request,
                        full: inline,
                    },
                );
                if sent {
                    delivery.mqtt_sent_ms = Some(now);
                    reached = true;
                }
            } else if self.can_push(phone_id, now) {
                // Not looking at the app: the question waits in the phone's inbox, and the push
                // follows once the Mac has been idle long enough (`flush_due_notifications`).
                reached = true;
            }
            pending.phones.insert(phone_id.clone(), delivery);
        }
        // Phones that are merely offline pick the request up from the snapshot on their next
        // hello. Only a broker outage without any push path counts as a lost surface.
        if !reached && !self.connected {
            pending.unreachable_since = Some(now);
        }
        self.requests.pending.push(pending);
        // With the Mac already idle for long enough (or no waiting configured) the push goes
        // out right here.
        self.flush_due_notifications(now);
    }

    /// Whether the phone is looking at the app: connected, in the foreground and past its hello.
    /// Such a phone gets the question live (and its inbox snapshot), never a push.
    pub(super) fn is_looking(&self, phone_id: &str) -> bool {
        self.is_live(phone_id) && self.connected
    }

    /// Pushes every question a phone has not looked at, once the Mac has been idle for the
    /// configured time (`docs/specs/ios-alert-timing.md`): the phone is not looking, has not
    /// opened the request and was not pushed for it yet. At most one push per request and phone;
    /// nothing is repeated. An unknown idle time counts as long enough, so a Mac that cannot tell
    /// still notifies. Cheap while nothing waits: the system clock is read only for a waiting
    /// request.
    pub(super) fn flush_due_notifications(&mut self, now: i64) {
        let mut waiting: Vec<(String, String, bool)> = Vec::new();
        for pending in &self.requests.pending {
            if matches!(&pending.sink, Sink::Ask(sink) if sink.quiet_feedback()) {
                continue;
            }
            for (phone_id, delivery) in &pending.phones {
                if !delivery.seen
                    && !delivery.pushed
                    && !self.is_looking(phone_id)
                    && self.can_push(phone_id, now)
                {
                    let immediate = pending.ask.as_ref().is_some_and(|ask| {
                        ask.message
                            .development
                            .as_ref()
                            .is_some_and(|info| info.relay)
                    }) || matches!(&pending.sink, Sink::Confirm(sink) if sink.immediate_notification());
                    waiting.push((phone_id.clone(), pending.view.id.clone(), immediate));
                }
            }
        }
        if waiting.is_empty() {
            return;
        }
        let after = self.host.alert_after_secs();
        let delayed = after > 0 && self.host.mac_idle_secs().is_some_and(|idle| idle < after);
        for (phone_id, request_id, immediate) in waiting {
            if delayed && !immediate {
                continue;
            }
            let Some(view) = self.requests.find(&request_id).map(|p| p.view.clone()) else {
                continue;
            };
            self.push_request(&phone_id, &view, now);
            if let Some(delivery) = self
                .requests
                .find_mut(&request_id)
                .and_then(|p| p.phones.get_mut(&phone_id))
            {
                delivery.pushed = true;
            }
        }
    }

    /// Sends the push for a request. `false` when the phone has no usable push registration.
    fn push_request(&mut self, phone_id: &str, view: &RequestView, now: i64) -> bool {
        let plain = PushPlain {
            k: view.kind.clone(),
            rid: Some(view.id.clone()),
            mac: self.mac_name(),
            agent: view.agent.label.clone(),
            project: view.project.name.clone(),
            session: push_session(view),
            text: preview_of(view),
            ts: now,
        };
        let loc_key = if view.kind == "confirm" {
            "PUSH_CONFIRM"
        } else {
            "PUSH_ASK"
        };
        self.spawn_push(
            phone_id,
            "alert",
            loc_key,
            &format!("rq-{}", view.id),
            &plain,
            now,
        )
    }

    fn push_resolved(&mut self, phone_id: &str, view: &RequestView, how: &str, now: i64) {
        let plain = PushPlain {
            k: "resolved".into(),
            rid: Some(view.id.clone()),
            mac: self.mac_name(),
            agent: view.agent.label.clone(),
            project: view.project.name.clone(),
            // The notification it replaces carried the title: keep the same lines above the text.
            session: push_session(view),
            text: how.to_string(),
            ts: now,
        };
        self.spawn_push(
            phone_id,
            "replace",
            "PUSH_RESOLVED",
            &format!("rq-{}", view.id),
            &plain,
            now,
        );
    }

    /// Whether a push to this phone could go out at all: it registered a push handle that is
    /// neither gone nor expired, the relay is configured and the push keys exist.
    pub(super) fn can_push(&self, phone_id: &str, now: i64) -> bool {
        let Some(reg) = self
            .store
            .state
            .phone(phone_id)
            .and_then(|phone| phone.push.as_ref())
        else {
            return false;
        };
        reg.status != "gone"
            && reg.expires_at * 1000 > now
            && !self.cfg.relay_url.trim().is_empty()
            && self.store.keys(phone_id).is_some()
    }

    /// Seals `plain` for the push domain and posts it to the relay in the background.
    pub(crate) fn spawn_push(
        &mut self,
        phone_id: &str,
        kind: &str,
        loc_key: &str,
        collapse_id: &str,
        plain: &PushPlain,
        now: i64,
    ) -> bool {
        if !self.can_push(phone_id, now) {
            return false;
        }
        let Some(reg) = self
            .store
            .state
            .phone(phone_id)
            .and_then(|phone| phone.push.clone())
        else {
            return false;
        };
        let Some(keys) = self.store.keys(phone_id).cloned() else {
            return false;
        };
        let plaintext = serde_json::to_vec(plain).expect("push plaintext serializes");
        let env = envelope::seal(
            &keys.m2p,
            envelope::Scope::Push,
            &self.mac_id,
            keys.kid,
            &plaintext,
        );
        let push = RelayPush {
            handle: reg.handle,
            kind: kind.to_string(),
            loc_key: loc_key.to_string(),
            thread_id: self.mac_id.clone(),
            collapse_id: collapse_id.to_string(),
            ah: env,
        };
        let relay_url = self.cfg.relay_url.clone();
        let pusher = self.pusher.clone();
        let tx = self.cmd_tx.clone();
        let phone_id = phone_id.to_string();
        tokio::spawn(async move {
            let outcome = pusher.send(&relay_url, &push).await;
            let _ = tx.send(super::Cmd::PushResult { phone_id, outcome });
        });
        true
    }

    pub(super) fn on_push_result(&mut self, phone_id: &str, outcome: PushOutcome) {
        let status = match outcome {
            PushOutcome::Accepted => "ok",
            PushOutcome::Gone => "gone",
            PushOutcome::Failed(_) => "error",
        };
        let mut changed = false;
        if let Some(reg) = self
            .store
            .state
            .phone_mut(phone_id)
            .and_then(|p| p.push.as_mut())
        {
            if reg.status != status {
                reg.status = status.to_string();
                changed = true;
            }
        }
        if changed {
            self.store.save_state();
            if status == "gone" {
                self.send(
                    phone_id,
                    &wire::PushState {
                        status: "gone".into(),
                    },
                );
            }
        }
    }

    pub(super) fn on_request_get(&mut self, phone_id: &str, request_id: &str) {
        if let Some(pending) = self.requests.find(request_id) {
            let request = pending.view.clone();
            self.send(phone_id, &wire::RequestFull { request });
            return;
        }
        if let Some(recent) = self.requests.recent(request_id) {
            let (how, by) = self.resolution_for(
                phone_id,
                recent.how,
                &recent.by.clone(),
                recent.winner.as_deref(),
            );
            self.send(
                phone_id,
                &wire::RequestResolved {
                    rev: self.revs.inbox,
                    request_id: request_id.to_string(),
                    how,
                    by,
                },
            );
            return;
        }
        self.send_error(phone_id, request_id, "not_found", None);
    }

    pub(super) fn on_request_ack(&mut self, phone_id: &str, request_id: &str) {
        if let Some(delivery) = self
            .requests
            .find_mut(request_id)
            .and_then(|p| p.phones.get_mut(phone_id))
        {
            delivery.acked = true;
        }
    }

    pub(super) fn on_request_seen(&mut self, phone_id: &str, request_id: &str) {
        if let Some(delivery) = self
            .requests
            .find_mut(request_id)
            .and_then(|p| p.phones.get_mut(phone_id))
        {
            delivery.seen = true;
            delivery.acked = true;
        }
    }

    // ------------------------------------------------------------ answers

    pub(super) fn replay_or_start(
        &mut self,
        phone_id: &str,
        client_msg_id: &str,
        msg_type: &str,
        hash: &str,
    ) -> Option<JournalLookup> {
        let lookup = self.store.journal_lookup(client_msg_id, phone_id, hash);
        // Decisions have their own durable compare-and-set and may safely resume before ACK.
        if matches!(msg_type, t::ANSWER_SUBMIT | t::CONFIRM_DECIDE)
            && matches!(
                lookup,
                JournalLookup::Interrupted | JournalLookup::Replay(None)
            )
        {
            return None;
        }
        if lookup == JournalLookup::New {
            self.store.journal_put(
                client_msg_id,
                JournalEntry {
                    phone_id: phone_id.to_string(),
                    msg_type: msg_type.to_string(),
                    hash: hash.to_string(),
                    status: "started".into(),
                    result: None,
                    ts: self.now(),
                },
            );
            return None;
        }
        Some(lookup)
    }

    pub(super) async fn on_answer_submit(&mut self, phone_id: &str, submit: AnswerSubmit) {
        if !self.inbox_ready() {
            return;
        }
        let _operation = match self
            .requests
            .find(&submit.request_id)
            .map(|pending| pending.sink.operation())
            .transpose()
        {
            Ok(operation) => operation.flatten(),
            Err(()) => return,
        };
        let hash = answer_hash(&submit);
        if self.replay_receipt(
            phone_id,
            &submit.client_msg_id,
            &hash,
            &submit.request_id,
            false,
        ) {
            return;
        }
        if let Some(lookup) =
            self.replay_or_start(phone_id, &submit.client_msg_id, t::ANSWER_SUBMIT, &hash)
        {
            self.send_journal_reply(
                phone_id,
                lookup,
                &submit.client_msg_id,
                &submit.request_id,
                false,
            );
            return;
        }
        let receipt = crate::daemon::recovery::Receipt {
            phone_id: phone_id.to_string(),
            client_msg_id: submit.client_msg_id.clone(),
            hash,
            result: JournalResult {
                msg_type: t::ANSWER_ACK.into(),
                body: AnswerAck {
                    client_msg_id: submit.client_msg_id.clone(),
                    request_id: submit.request_id.clone(),
                    status: "accepted".into(),
                    reason: None,
                }
                .to_value(),
            },
            at_ms: self.now(),
        };
        let (status, reason) = self.try_answer(phone_id, &submit, &receipt);
        if status == "retry" {
            return;
        }
        let ack = AnswerAck {
            client_msg_id: submit.client_msg_id.clone(),
            request_id: submit.request_id.clone(),
            status: status.to_string(),
            reason,
        };
        self.finish_journal(&submit.client_msg_id, t::ANSWER_ACK, &ack);
        self.send(phone_id, &ack);
        if status == "accepted" {
            self.resolve(
                &submit.request_id,
                Resolution::AnsweredByPhone(phone_id.to_string()),
            )
            .await;
        }
    }

    pub(super) async fn on_confirm_decide(&mut self, phone_id: &str, decide: ConfirmDecide) {
        if !self.inbox_ready() {
            return;
        }
        let _operation = match self
            .requests
            .find(&decide.request_id)
            .map(|pending| pending.sink.operation())
            .transpose()
        {
            Ok(operation) => operation.flatten(),
            Err(()) => return,
        };
        let hash = decide_hash(&decide);
        if self.replay_receipt(
            phone_id,
            &decide.client_msg_id,
            &hash,
            &decide.request_id,
            true,
        ) {
            return;
        }
        if let Some(lookup) =
            self.replay_or_start(phone_id, &decide.client_msg_id, t::CONFIRM_DECIDE, &hash)
        {
            self.send_journal_reply(
                phone_id,
                lookup,
                &decide.client_msg_id,
                &decide.request_id,
                true,
            );
            return;
        }
        let receipt = crate::daemon::recovery::Receipt {
            phone_id: phone_id.to_string(),
            client_msg_id: decide.client_msg_id.clone(),
            hash,
            result: JournalResult {
                msg_type: t::CONFIRM_ACK.into(),
                body: ConfirmAck {
                    client_msg_id: decide.client_msg_id.clone(),
                    request_id: decide.request_id.clone(),
                    status: "accepted".into(),
                    reason: None,
                }
                .to_value(),
            },
            at_ms: self.now(),
        };
        let (status, reason) = self.try_decide(&decide, &receipt);
        if status == "retry" {
            return;
        }
        let ack = ConfirmAck {
            client_msg_id: decide.client_msg_id.clone(),
            request_id: decide.request_id.clone(),
            status: status.to_string(),
            reason,
        };
        self.finish_journal(&decide.client_msg_id, t::CONFIRM_ACK, &ack);
        self.send(phone_id, &ack);
        if status == "accepted" {
            self.resolve(
                &decide.request_id,
                Resolution::AnsweredByPhone(phone_id.to_string()),
            )
            .await;
        }
    }

    fn replay_receipt(
        &mut self,
        phone: &str,
        id: &str,
        hash: &str,
        request: &str,
        confirm: bool,
    ) -> bool {
        match self.host.submission_receipt(id, phone, hash) {
            Ok(JournalLookup::New) => false,
            Ok(lookup) => {
                self.send_journal_reply(phone, lookup, id, request, confirm);
                true
            }
            Err(error) => {
                eprintln!("askhuman: waiting to restore mobile submission receipt: {error}");
                true
            }
        }
    }

    pub(super) fn finish_journal<M: wire::Msg>(
        &mut self,
        client_msg_id: &str,
        msg_type: &str,
        ack: &M,
    ) {
        let now = self.now();
        self.store.journal_finish(
            client_msg_id,
            JournalResult {
                msg_type: msg_type.to_string(),
                body: ack.to_value(),
            },
            now,
        );
    }

    /// A repeated `clientMsgId`: replay the first result, never re-run the side effect.
    fn send_journal_reply(
        &mut self,
        phone_id: &str,
        lookup: JournalLookup,
        client_msg_id: &str,
        request_id: &str,
        confirm: bool,
    ) {
        match lookup {
            JournalLookup::Replay(Some(result)) => {
                self.send_value(phone_id, &result.msg_type, result.body);
            }
            JournalLookup::Conflict => {
                self.send_ack(
                    phone_id,
                    confirm,
                    client_msg_id,
                    request_id,
                    "invalid",
                    Some("conflict"),
                );
            }
            // In flight without a result (daemon restarted mid-way) or first run unfinished.
            _ => {
                self.send_ack(
                    phone_id,
                    confirm,
                    client_msg_id,
                    request_id,
                    "expired",
                    None,
                );
            }
        }
    }

    fn send_ack(
        &mut self,
        phone_id: &str,
        confirm: bool,
        client_msg_id: &str,
        request_id: &str,
        status: &str,
        reason: Option<&str>,
    ) {
        let (client_msg_id, request_id, status, reason) = (
            client_msg_id.to_string(),
            request_id.to_string(),
            status.to_string(),
            reason.map(str::to_string),
        );
        if confirm {
            self.send(
                phone_id,
                &ConfirmAck {
                    client_msg_id,
                    request_id,
                    status,
                    reason,
                },
            );
        } else {
            self.send(
                phone_id,
                &AnswerAck {
                    client_msg_id,
                    request_id,
                    status,
                    reason,
                },
            );
        }
    }

    /// Outcome for a request that is no longer pending.
    fn closed_status(&self, request_id: &str) -> (&'static str, Option<String>) {
        match self.requests.recent(request_id) {
            Some(recent) if recent.how == "expired" => ("expired", None),
            Some(recent) => ("lost", recent.by.clone()),
            None => ("expired", None),
        }
    }

    fn try_answer(
        &mut self,
        phone_id: &str,
        submit: &AnswerSubmit,
        receipt: &crate::daemon::recovery::Receipt,
    ) -> (&'static str, Option<String>) {
        let Some(pending) = self.requests.find(&submit.request_id) else {
            return self.closed_status(&submit.request_id);
        };
        let (Sink::Ask(sink), Some(ask)) = (&pending.sink, &pending.ask) else {
            return ("invalid", Some("not a question".into()));
        };
        let (sink, ask) = (sink.clone(), ask.clone());
        let project_key = pending.view.project.key.clone();
        let directive = match directive_of(submit) {
            Ok(directive) => directive,
            Err(reason) => return ("invalid", Some(reason)),
        };
        let result = match submit.action.as_str() {
            "cancel" => ChannelResult::cancel("ios"),
            "send" => match self.build_answers(phone_id, &ask, &project_key, submit) {
                Ok(answers) => ChannelResult {
                    action: ChannelAction::Send,
                    answers,
                    source_channel_id: "ios".into(),
                    // Cleaned for every source by `Coordinator::try_submit`.
                    annotations: submit.annotations.clone(),
                },
                Err(reason) => return ("invalid", Some(reason)),
            },
            _ => return ("invalid", Some("unknown action".into())),
        };
        if sink.try_submit_identified(result, receipt, directive) {
            self.finish_reply_uploads(submit);
            ("accepted", None)
        } else {
            match sink.winner() {
                Some(winner) => ("lost", Some(winner)),
                None => ("retry", None),
            }
        }
    }

    /// The todos a phone picked while answering, looked up on this Mac before anything is consumed
    /// (docs/specs/special-replies.md D28/D29). Empty when none were picked.
    fn resolve_todo_picks(
        &self,
        ask: &AskRequest,
        project_key: &str,
        submit: &AnswerSubmit,
        answers: &[QuestionAnswer],
    ) -> Result<Vec<TodoPick>, String> {
        let ids = &submit.todo_ids;
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        if ask.select_only {
            return Err("this question accepts only the listed options".into());
        }
        if ids.len() > wire::TODO_MAX_PENDING {
            return Err("too many todos".into());
        }
        let mut seen = std::collections::HashSet::new();
        if !ids.iter().all(|id| seen.insert(id.as_str())) {
            return Err("duplicate todo".into());
        }
        if ask.whats_next {
            // The next task is one thing: a todo or one of the options, never both.
            let chosen = answers.first().map_or(0, |a| a.selected_options.len());
            if ids.len() > 1 || chosen > 0 {
                return Err("choose one next task".into());
            }
        }
        let picks = self.host.todo_picks(project_key, ids);
        if picks.len() != ids.len() || picks.iter().zip(ids).any(|(pick, id)| &pick.id != id) {
            return Err(wire::REJECT_TODO_GONE.into());
        }
        Ok(picks)
    }

    fn build_answers(
        &mut self,
        phone_id: &str,
        ask: &AskRequest,
        project_key: &str,
        submit: &AnswerSubmit,
    ) -> Result<Vec<QuestionAnswer>, String> {
        let mut answers = vec![QuestionAnswer::default(); ask.questions.len()];
        let mut seen = vec![false; ask.questions.len()];
        for item in &submit.answers {
            let Some(question) = ask.questions.get(item.q) else {
                return Err(format!("question {} does not exist", item.q));
            };
            if std::mem::replace(&mut seen[item.q], true) {
                return Err(format!("question {} answered twice", item.q));
            }
            let mut sel = item.sel.clone();
            sel.sort_unstable();
            sel.dedup();
            if sel.len() != item.sel.len() {
                return Err("duplicate option".into());
            }
            if ask.single && sel.len() > 1 {
                return Err("only one option may be selected".into());
            }
            let mut selected = Vec::with_capacity(sel.len());
            for index in sel {
                let option = question
                    .predefined_options
                    .get(index)
                    .ok_or_else(|| format!("option {index} does not exist"))?;
                selected.push(option.text.clone());
            }
            let text = item
                .text
                .as_deref()
                .map(str::trim)
                .filter(|t| !t.is_empty());
            if let Some(text) = text {
                if text.chars().count() > MAX_ANSWER_TEXT_CHARS {
                    return Err("answer text too long".into());
                }
            }
            let has_attachments = !item.images.is_empty() || !item.files.is_empty();
            if ask.select_only && (text.is_some() || has_attachments) {
                return Err("this question accepts only the listed options".into());
            }
            answers[item.q] = QuestionAnswer {
                selected_options: selected,
                user_input: text.map(str::to_string),
                ..QuestionAnswer::default()
            };
        }
        let picks = self.resolve_todo_picks(ask, project_key, submit, &answers)?;
        merge_todo_picks(ask, &mut answers, picks, |text| {
            self.host.todo_merge_line(text)
        });
        // Uploads are consumed only once the whole answer is known to be valid, so a rejected
        // answer can be corrected and resubmitted without uploading again.
        let all_ids: Vec<String> = submit
            .answers
            .iter()
            .flat_map(|item| item.images.iter().chain(item.files.iter()))
            .cloned()
            .collect();
        let mut uploads = self
            .reply_uploads(phone_id, &submit.request_id, &all_ids)?
            .into_iter();
        for item in &submit.answers {
            let answer = &mut answers[item.q];
            for _ in &item.images {
                let Some(file) = uploads.next() else { break };
                let bytes = std::fs::read(&file.path)
                    .map_err(|_| "attachment is not available".to_string())?;
                let media_type = if file.mime.starts_with("image/") {
                    file.mime.clone()
                } else {
                    views::mime_for(&file.name).to_string()
                };
                answer.images.push(ImageAttachment {
                    data: base64::engine::general_purpose::STANDARD.encode(bytes),
                    media_type,
                    filename: Some(file.name.clone()),
                });
            }
            for _ in &item.files {
                let Some(file) = uploads.next() else { break };
                answer.files.push(file.path.display().to_string());
            }
        }
        Ok(answers)
    }

    fn try_decide(
        &mut self,
        decide: &ConfirmDecide,
        receipt: &crate::daemon::recovery::Receipt,
    ) -> (&'static str, Option<String>) {
        let Some(pending) = self.requests.find(&decide.request_id) else {
            return self.closed_status(&decide.request_id);
        };
        let (Sink::Confirm(sink), Some(confirm)) = (&pending.sink, &pending.view.confirm) else {
            return ("invalid", Some("not a confirmation".into()));
        };
        let index = if decide.dismiss {
            match confirm.dismiss_index {
                Some(index) => index,
                None => {
                    return (
                        "invalid",
                        Some("this confirmation cannot be dismissed".into()),
                    )
                }
            }
        } else {
            match decide.wire_index {
                Some(index) if confirm.choices.iter().any(|c| c.wire_index == index) => index,
                _ => return ("invalid", Some("unknown choice".into())),
            }
        };
        let comment = decide
            .input
            .as_deref()
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .map(str::to_string);
        let sink = sink.clone();
        match sink.decide_identified(index, comment, receipt) {
            Ok(true) => ("accepted", None),
            Ok(false) => match sink.winner() {
                Some(winner) => ("lost", Some(winner)),
                None => ("retry", None),
            },
            Err(reason)
                if reason.starts_with("unable to save") || reason.starts_with("updating") =>
            {
                ("retry", None)
            }
            Err(reason) => ("invalid", Some(reason)),
        }
    }

    // ------------------------------------------------------------ resolution and timers

    pub(super) async fn resolve(&mut self, request_id: &str, resolution: Resolution) {
        let Some(position) = self
            .requests
            .pending
            .iter()
            .position(|p| p.view.id == request_id)
        else {
            return;
        };
        let pending = self.requests.pending.remove(position);
        let now = self.now();
        let (how, by) = resolution.wire();
        let winner = match &resolution {
            Resolution::AnsweredByPhone(phone_id) => Some(phone_id.clone()),
            _ => None,
        };
        self.revs.inbox += 1;
        self.requests
            .remember(request_id, how, by.clone(), winner.clone(), now);

        let phones: Vec<String> = self
            .store
            .state
            .active_phones()
            .map(|p| p.phone_id.clone())
            .collect();
        for phone_id in phones {
            let (how, by) = self.resolution_for(&phone_id, how, &by, winner.as_deref());
            let online = self.rt.get(&phone_id).is_some_and(|rt| rt.online);
            if online {
                self.send(
                    &phone_id,
                    &wire::RequestResolved {
                        rev: self.revs.inbox,
                        request_id: request_id.to_string(),
                        how: how.clone(),
                        by: by.clone(),
                    },
                );
            }
            // A phone that was pushed and never opened the request still shows the alert.
            let delivery = pending.phones.get(&phone_id);
            if delivery.is_some_and(|d| d.pushed && !d.seen) {
                let text = match &by {
                    Some(by) => format!("{how}: {by}"),
                    None => how.clone(),
                };
                self.push_resolved(&phone_id, &pending.view, &text, now);
            }
        }
    }

    /// What one phone is told about how a request ended: a phone that lost to another phone hears
    /// "answered elsewhere" with the winner's name instead of a plain "answered".
    fn resolution_for(
        &self,
        phone_id: &str,
        how: &str,
        by: &Option<String>,
        winner: Option<&str>,
    ) -> (String, Option<String>) {
        if let Some(winner) = winner.filter(|w| *w != phone_id) {
            let name = self
                .store
                .state
                .phone(winner)
                .map(|p| p.display_name().to_string())
                .unwrap_or_else(|| "iOS".to_string());
            return ("answeredElsewhere".to_string(), Some(name));
        }
        (how.to_string(), by.clone())
    }

    /// Hands every request back to the daemon as a lost surface.
    pub(super) fn drop_requests(&mut self, reason: &str) {
        for pending in self.requests.pending.drain(..) {
            pending.sink.surface_lost(reason);
        }
    }

    pub(super) async fn tick_requests(&mut self, now: i64) {
        // Expired confirmations the daemon did not report.
        let expired: Vec<String> = self
            .requests
            .pending
            .iter()
            .filter(|p| p.deadline_ms.is_some_and(|d| now > d + EXPIRY_GRACE_MS))
            .map(|p| p.view.id.clone())
            .collect();
        for id in expired {
            self.resolve(&id, Resolution::Expired).await;
        }

        // Surfaces that stayed unreachable.
        let connected = self.connected;
        let mut lost = Vec::new();
        for pending in &mut self.requests.pending {
            match pending.unreachable_since {
                Some(_) if connected => pending.unreachable_since = None,
                Some(since) if now - since >= UNREACHABLE_GRACE_MS => {
                    lost.push(pending.view.id.clone());
                }
                _ => {}
            }
        }
        for id in lost {
            if let Some(position) = self.requests.pending.iter().position(|p| p.view.id == id) {
                let pending = self.requests.pending.remove(position);
                self.revs.inbox += 1;
                pending.sink.surface_lost("iOS broker unreachable");
            }
        }

        // Push fallback for requests the foreground phone never acknowledged.
        let mut due: Vec<(String, String)> = Vec::new();
        for pending in &self.requests.pending {
            if matches!(&pending.sink, Sink::Ask(sink) if sink.quiet_feedback()) {
                continue;
            }
            for (phone_id, delivery) in &pending.phones {
                if let Some(sent) = delivery.mqtt_sent_ms {
                    if !delivery.acked
                        && !delivery.pushed
                        && now - sent >= PUSH_FALLBACK_SECS as i64 * 1000
                    {
                        due.push((phone_id.clone(), pending.view.id.clone()));
                    }
                }
            }
        }
        for (phone_id, request_id) in due {
            let Some(view) = self.requests.find(&request_id).map(|p| p.view.clone()) else {
                continue;
            };
            // Without a push path there is nothing more to try; do not retry every tick either way.
            self.push_request(&phone_id, &view, now);
            if let Some(delivery) = self
                .requests
                .find_mut(&request_id)
                .and_then(|p| p.phones.get_mut(&phone_id))
            {
                delivery.pushed = true;
            }
        }

        // Questions a phone is not looking at: pushed once the Mac has been idle long enough.
        self.flush_due_notifications(now);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::annotations::UserAnnotation;

    fn submit(annotations: Vec<UserAnnotation>) -> AnswerSubmit {
        AnswerSubmit {
            client_msg_id: "c1".into(),
            request_id: "req1".into(),
            action: "send".into(),
            answers: Vec::new(),
            todo_ids: Vec::new(),
            directive: None,
            annotations,
        }
    }

    /// An answer without notes hashes exactly as before the field existed, so a retry that
    /// crosses an upgrade of the Mac still replays instead of being called a conflict; notes
    /// change the hash.
    #[test]
    fn the_answer_hash_ignores_absent_notes_and_notices_present_ones() {
        let plain = submit(Vec::new());
        let before = short_hash(
            &serde_json::to_vec(&json!({
                "requestId": plain.request_id,
                "action": plain.action,
                "answers": plain.answers,
            }))
            .unwrap(),
        );
        assert_eq!(answer_hash(&plain), before);

        let note = |text: &str| UserAnnotation {
            quote: "passage".into(),
            note: text.into(),
        };
        let one = answer_hash(&submit(vec![note("why?")]));
        assert_ne!(one, before);
        assert_ne!(one, answer_hash(&submit(vec![note("other")])));
        assert_eq!(one, answer_hash(&submit(vec![note("why?")])));
    }
}
