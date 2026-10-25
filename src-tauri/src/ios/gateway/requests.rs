//! Questions and confirmations on their way to the phones: delivery (MQTT while the app is in
//! the foreground, push otherwise), answers coming back, and the resolved/expired bookkeeping
//! (plan §4.4–§4.6).

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use super::Actor;
use crate::ios::envelope;
use crate::ios::push::PushOutcome;
use crate::ios::store::{JournalEntry, JournalLookup, JournalResult};
use crate::ios::wire::{
    self, t, AnswerAck, AnswerSubmit, ConfirmAck, ConfirmDecide, PushPlain, RelayPush,
    RequestSummary, RequestView, INLINE_REQUEST_BYTES, MAX_PLAIN_BYTES, PUSH_FALLBACK_SECS,
    PUSH_PREVIEW_MAX_CHARS,
};
use crate::models::{AskRequest, ChannelAction, ChannelResult, QuestionAnswer};

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
    /// Submits the answer; `true` when this submission decided the request.
    fn try_submit(&self, result: ChannelResult) -> bool;
    /// Channel that decided the request, once it is decided.
    fn winner(&self) -> Option<String>;
    /// The surface can no longer reach a phone; the request keeps waiting elsewhere.
    fn surface_lost(&self, reason: &str);
}

/// The confirmation side of a request.
pub trait ConfirmSink: Send + Sync + 'static {
    /// `Ok(true)` decided, `Ok(false)` already decided, `Err` invalid input.
    fn decide(&self, wire_index: usize, comment: Option<String>) -> Result<bool, String>;
    fn winner(&self) -> Option<String>;
    fn surface_lost(&self, reason: &str);
}

#[derive(Clone)]
pub enum Sink {
    Ask(Arc<dyn AskSink>),
    Confirm(Arc<dyn ConfirmSink>),
}

impl Sink {
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
}

/// How a request ended, as far as the phones are concerned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resolution {
    /// A phone answered it.
    Answered,
    /// Another channel answered it (display name of that channel).
    AnsweredElsewhere(String),
    Cancelled(String),
    Expired,
}

impl Resolution {
    fn wire(&self) -> (&'static str, Option<String>) {
        match self {
            Resolution::Answered => ("answered", None),
            Resolution::AnsweredElsewhere(by) => ("answeredElsewhere", Some(by.clone())),
            Resolution::Cancelled(by) => ("cancelled", Some(by.clone())),
            Resolution::Expired => ("expired", None),
        }
    }
}

// ---------------------------------------------------------------- bookkeeping

#[derive(Default)]
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
    pub unreachable_since: Option<i64>,
    pub phones: HashMap<String, PhoneDelivery>,
}

struct Recent {
    id: String,
    how: &'static str,
    by: Option<String>,
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

    pub fn forget_phone(&mut self, phone_id: &str) {
        for pending in &mut self.pending {
            pending.phones.remove(phone_id);
        }
    }

    fn remember(&mut self, id: &str, how: &'static str, by: Option<String>, now: i64) {
        self.recent.push_back(Recent {
            id: id.to_string(),
            how,
            by,
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
}

// ---------------------------------------------------------------- view helpers

fn truncate_chars(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let cut: String = text.chars().take(max).collect();
    format!("{cut}…")
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

/// Shrinks the long texts of a view until its JSON fits comfortably into one message.
fn fit_view(mut view: RequestView) -> (RequestView, Vec<u8>) {
    let budget = MAX_PLAIN_BYTES - 16 * 1024;
    for _ in 0..24 {
        let json = serde_json::to_vec(&view).expect("view serializes");
        if json.len() <= budget {
            return (view, json);
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
        created_at: view.created_at,
        preview: preview_of(view),
        content_hash: view.content_hash.clone(),
        size,
    }
}

fn answer_hash(submit: &AnswerSubmit) -> String {
    let canonical = json!({
        "requestId": submit.request_id,
        "action": submit.action,
        "answers": submit.answers,
    });
    short_hash(&serde_json::to_vec(&canonical).expect("answer serializes"))
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

    fn is_live(&self, phone_id: &str) -> bool {
        self.rt
            .get(phone_id)
            .is_some_and(|rt| rt.online && rt.foreground && rt.hello_at.is_some())
    }

    pub(super) async fn deliver(&mut self, req: DeliverRequest) {
        let now = self.now();
        if self.requests.find(&req.view.id).is_some() {
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
            unreachable_since: None,
            phones: HashMap::new(),
        };
        let inline = json.len() <= INLINE_REQUEST_BYTES;
        let rev = self.revs.inbox;
        let mut reached = false;
        for phone_id in &targets {
            let mut delivery = PhoneDelivery::default();
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
            } else if self.push_request(phone_id, &pending.view, now) {
                delivery.pushed = true;
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
    }

    /// Sends the push for a request. `false` when the phone has no usable push registration.
    fn push_request(&mut self, phone_id: &str, view: &RequestView, now: i64) -> bool {
        let plain = PushPlain {
            k: view.kind.clone(),
            rid: Some(view.id.clone()),
            mac: self.mac_name(),
            agent: view.agent.label.clone(),
            project: view.project.name.clone(),
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
        let Some(phone) = self.store.state.phone(phone_id) else {
            return false;
        };
        let Some(reg) = phone.push.clone() else {
            return false;
        };
        if reg.status == "gone"
            || reg.expires_at * 1000 <= now
            || self.cfg.relay_url.trim().is_empty()
        {
            return false;
        }
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
            let (how, by) = (recent.how, recent.by.clone());
            self.send(
                phone_id,
                &wire::RequestResolved {
                    rev: self.revs.inbox,
                    request_id: request_id.to_string(),
                    how: how.to_string(),
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

    fn replay_or_start(
        &mut self,
        phone_id: &str,
        client_msg_id: &str,
        msg_type: &str,
        hash: &str,
    ) -> Option<JournalLookup> {
        let lookup = self.store.journal_lookup(client_msg_id, phone_id, hash);
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
        let hash = answer_hash(&submit);
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
        let (status, reason) = self.try_answer(phone_id, &submit);
        let ack = AnswerAck {
            client_msg_id: submit.client_msg_id.clone(),
            request_id: submit.request_id.clone(),
            status: status.to_string(),
            reason,
        };
        self.finish_journal(&submit.client_msg_id, t::ANSWER_ACK, &ack);
        self.send(phone_id, &ack);
        if status == "accepted" {
            self.resolve(&submit.request_id, Resolution::Answered).await;
        }
    }

    pub(super) async fn on_confirm_decide(&mut self, phone_id: &str, decide: ConfirmDecide) {
        let hash = decide_hash(&decide);
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
        let (status, reason) = self.try_decide(&decide);
        let ack = ConfirmAck {
            client_msg_id: decide.client_msg_id.clone(),
            request_id: decide.request_id.clone(),
            status: status.to_string(),
            reason,
        };
        self.finish_journal(&decide.client_msg_id, t::CONFIRM_ACK, &ack);
        self.send(phone_id, &ack);
        if status == "accepted" {
            self.resolve(&decide.request_id, Resolution::Answered).await;
        }
    }

    fn finish_journal<M: wire::Msg>(&mut self, client_msg_id: &str, msg_type: &str, ack: &M) {
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
    ) -> (&'static str, Option<String>) {
        let Some(pending) = self.requests.find(&submit.request_id) else {
            return self.closed_status(&submit.request_id);
        };
        let (Sink::Ask(sink), Some(ask)) = (&pending.sink, &pending.ask) else {
            return ("invalid", Some("not a question".into()));
        };
        let (sink, ask) = (sink.clone(), ask.clone());
        let result = match submit.action.as_str() {
            "cancel" => ChannelResult::cancel("ios"),
            "send" => match self.build_answers(phone_id, &ask, submit) {
                Ok(answers) => ChannelResult {
                    action: ChannelAction::Send,
                    answers,
                    source_channel_id: "ios".into(),
                },
                Err(reason) => return ("invalid", Some(reason)),
            },
            _ => return ("invalid", Some("unknown action".into())),
        };
        if sink.try_submit(result) {
            ("accepted", None)
        } else {
            ("lost", sink.winner())
        }
    }

    fn build_answers(
        &mut self,
        _phone_id: &str,
        ask: &AskRequest,
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
            if ask.select_only
                && (text.is_some() || !item.images.is_empty() || !item.files.is_empty())
            {
                return Err("this question accepts only the listed options".into());
            }
            if !item.images.is_empty() || !item.files.is_empty() {
                return Err("attachments are not supported yet".into());
            }
            answers[item.q] = QuestionAnswer {
                selected_options: selected,
                user_input: text.map(str::to_string),
                ..QuestionAnswer::default()
            };
        }
        Ok(answers)
    }

    fn try_decide(&mut self, decide: &ConfirmDecide) -> (&'static str, Option<String>) {
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
        match sink.decide(index, comment) {
            Ok(true) => ("accepted", None),
            Ok(false) => ("lost", sink.winner()),
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
        self.revs.inbox += 1;
        self.requests.remember(request_id, how, by.clone(), now);

        let phones: Vec<String> = self
            .store
            .state
            .active_phones()
            .map(|p| p.phone_id.clone())
            .collect();
        for phone_id in phones {
            let online = self.rt.get(&phone_id).is_some_and(|rt| rt.online);
            if online {
                self.send(
                    &phone_id,
                    &wire::RequestResolved {
                        rev: self.revs.inbox,
                        request_id: request_id.to_string(),
                        how: how.to_string(),
                        by: by.clone(),
                    },
                );
            }
            // A phone that was pushed and never opened the request still shows the alert.
            let delivery = pending.phones.get(&phone_id);
            if delivery.is_some_and(|d| d.pushed && !d.seen) {
                let text = match &by {
                    Some(by) => format!("{how}: {by}"),
                    None => how.to_string(),
                };
                self.push_resolved(&phone_id, &pending.view, &text, now);
            }
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
                Some(since) if connected => {
                    let _ = since;
                    pending.unreachable_since = None;
                }
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
            let pushed = self.push_request(&phone_id, &view, now);
            if let Some(delivery) = self
                .requests
                .find_mut(&request_id)
                .and_then(|p| p.phones.get_mut(&phone_id))
            {
                // Without a push path there is nothing more to try; do not retry every tick.
                delivery.pushed = true;
                let _ = pushed;
            }
        }
    }
}

#[allow(dead_code)]
fn _assert_json(_: &Value) {}
