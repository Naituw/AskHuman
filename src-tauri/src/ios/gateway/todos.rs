//! Todos for the phone (docs/specs/ios-todos.md §5–§6): the project list, one project's todos,
//! writes and the "something changed" nudge.
//!
//! The gateway owns the protocol: it validates writes, keeps them idempotent through the journal,
//! sizes lists to the broker's limit and tracks who watches. What a project is and how a todo is
//! stored is the host's business ([`super::Host::todo_apply`] and friends).

use std::collections::BTreeSet;

use serde_json::{json, Value};

use super::tasks::body_hash;
use super::Actor;
use crate::ios::store::JournalLookup;
use crate::ios::wire::{self, t, TODO_MAX_PENDING, TODO_TEXT_MAX_CHARS};

/// How often the todo store is checked for changes while a phone watches.
const POLL_MS: i64 = 1000;

/// Who watches the todos, and what they were last told.
#[derive(Default)]
pub(crate) struct TodoLive {
    pub watchers: BTreeSet<String>,
    /// The store signature the watchers know of.
    pub rev: u64,
    pub polled_at: i64,
}

/// What makes two `todo.op` messages the same write: the journal replays the answer to a repeat
/// and calls anything else under the same id a conflict.
fn identity(op: &wire::TodoOp) -> Value {
    json!({
        "id": op.id,
        "op": op.op,
        "todoId": op.todo_id,
        "text": op.text,
        "auto": op.auto,
        "order": op.order,
    })
}

/// Checks the shape of a write and trims its text. `Err` carries the reason of the refusal.
fn normalize(op: &wire::TodoOp) -> Result<wire::TodoOp, &'static str> {
    let mut op = op.clone();
    let has_id = op.todo_id.as_deref().is_some_and(|id| !id.is_empty());
    match op.op.as_str() {
        "add" | "edit" => {
            if op.op == "edit" && !has_id {
                return Err("invalid");
            }
            let text = op.text.as_deref().unwrap_or("").trim().to_string();
            if text.contains('\0') {
                return Err("invalid");
            }
            if text.is_empty() {
                return Err("empty");
            }
            if text.chars().count() > TODO_TEXT_MAX_CHARS {
                return Err("tooLong");
            }
            op.text = Some(text);
        }
        "complete" | "restore" | "remove" if !has_id => return Err("invalid"),
        "complete" | "restore" | "remove" => {}
        "auto" if !has_id || op.auto.is_none() => return Err("invalid"),
        "auto" => {}
        "reorder" => match &op.order {
            Some(order) if order.len() <= TODO_MAX_PENDING => {}
            _ => return Err("invalid"),
        },
        _ => return Err("unsupported"),
    }
    Ok(op)
}

fn ack_of(op: &wire::TodoOp, status: &str, reason: Option<&str>) -> wire::TodoAck {
    wire::TodoAck {
        client_msg_id: op.client_msg_id.clone(),
        status: status.to_string(),
        reason: reason.map(str::to_string),
        todo_id: None,
        list: None,
    }
}

impl Actor {
    pub(super) fn on_todo_projects_get(&mut self, phone_id: &str) {
        self.host.todo_refresh();
        let host = self.host.clone();
        let phone = phone_id.to_string();
        self.spawn_then(
            async move { host.todo_projects().await },
            move |actor, projects| {
                actor.send(&phone, &projects);
            },
        );
    }

    pub(super) fn on_todo_list_get(&mut self, phone_id: &str, project_id: &str) {
        let host = self.host.clone();
        let phone = phone_id.to_string();
        let project_id = project_id.to_string();
        self.spawn_then(
            async move { host.todo_list(&project_id).await },
            move |actor, list| {
                actor.send_todo_list(&phone, list);
            },
        );
    }

    /// Sends a list, dropping what does not fit under the broker's limit (the totals stay, so the
    /// phone can say how many it does not show).
    fn send_todo_list(&mut self, phone_id: &str, mut list: wire::TodoList) {
        while !self.send(phone_id, &list) {
            if !list.shrink() {
                return;
            }
        }
    }

    fn send_todo_ack(&mut self, phone_id: &str, mut ack: wire::TodoAck) {
        loop {
            if self.send(phone_id, &ack) {
                return;
            }
            let shrunk = ack.list.as_mut().is_some_and(|list| list.shrink());
            if !shrunk {
                // Without a list the ack is a few hundred bytes; it fits.
                ack.list = None;
                self.send(phone_id, &ack);
                return;
            }
        }
    }

    pub(super) async fn on_todo_op(&mut self, phone_id: &str, op: wire::TodoOp) {
        let id = op.client_msg_id.clone();
        let hash = body_hash(&identity(&op));
        if let Some(lookup) = self.replay_or_start(phone_id, &id, t::TODO_OP, &hash) {
            let mut ack = match lookup {
                JournalLookup::Replay(Some(result)) => {
                    serde_json::from_value::<wire::TodoAck>(result.body)
                        .unwrap_or_else(|_| ack_of(&op, "failed", Some("interrupted")))
                }
                JournalLookup::Conflict => ack_of(&op, "rejected", Some("conflict")),
                // The first attempt never finished (the daemon restarted mid-way): the write may
                // or may not have happened, and the list below shows which.
                _ => ack_of(&op, "failed", Some("interrupted")),
            };
            // The journal keeps the verdict, never a list: it would be stale by now.
            let list = self.host.todo_list(&op.id).await;
            ack.list = list.found.then_some(list);
            self.send_todo_ack(phone_id, ack);
            return;
        }
        let ack = match normalize(&op) {
            Err(reason) => {
                let mut ack = ack_of(&op, "rejected", Some(reason));
                // The journal keeps the verdict, never a list.
                self.finish_journal(&id, t::TODO_ACK, &ack);
                let list = self.host.todo_list(&op.id).await;
                ack.list = list.found.then_some(list);
                ack
            }
            Ok(clean) => {
                let applied = self.host.todo_apply(&clean).await;
                let mut ack = ack_of(&op, applied.status, applied.reason.as_deref());
                ack.todo_id = applied.todo_id;
                self.finish_journal(&id, t::TODO_ACK, &ack);
                ack.list = applied.list;
                ack
            }
        };
        self.send_todo_ack(phone_id, ack);
    }

    pub(super) fn on_todo_watch(&mut self, phone_id: &str, on: bool) {
        if !on {
            self.todos.watchers.remove(phone_id);
            return;
        }
        self.host.todo_refresh();
        let rev = self.host.todo_rev();
        if self.todos.watchers.is_empty() {
            self.todos.rev = rev;
            self.todos.polled_at = self.now();
        }
        self.todos.watchers.insert(phone_id.to_string());
        // Whatever changed between the phone's last read and this subscription is caught by
        // comparing this signature with the one it holds.
        self.send(phone_id, &wire::TodoChanged { rev });
    }

    /// Tells the watching phones when the todo store changed.
    pub(super) fn tick_todos(&mut self, now: i64) {
        if self.todos.watchers.is_empty() || now - self.todos.polled_at < POLL_MS {
            return;
        }
        self.todos.polled_at = now;
        let live: BTreeSet<String> = self.live_phones().into_iter().collect();
        self.todos.watchers.retain(|phone| live.contains(phone));
        if self.todos.watchers.is_empty() {
            return;
        }
        let rev = self.host.todo_rev();
        if rev == self.todos.rev {
            return;
        }
        self.todos.rev = rev;
        for phone in self.todos.watchers.clone() {
            self.send(&phone, &wire::TodoChanged { rev });
        }
    }
}
