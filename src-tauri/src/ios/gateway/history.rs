//! A session's earlier exchanges for the phone (docs/specs/ios-session-history.md §6.3) and the exchange
//! of one history record with its pictures (docs/specs/ios-history-pictures.md §6).
//!
//! The gateway owns the protocol: it keeps `limit` within range, fits a page under the page budget and
//! the broker's limit, and hands the phone's own `reqId` back. What an Agent session is and where its
//! exchanges are kept is the host's business ([`super::Host::session_history`],
//! [`super::Host::history_exchange`]).

use super::Actor;
use crate::ios::wire::{self, SESSION_HISTORY_BYTES, SESSION_HISTORY_PAGE_MAX};

impl Actor {
    pub(super) fn on_session_history_get(
        &mut self,
        phone_id: &str,
        mut get: wire::SessionHistoryGet,
    ) {
        get.limit = get.limit.clamp(1, SESSION_HISTORY_PAGE_MAX as u32);
        let host = self.host.clone();
        let phone = phone_id.to_string();
        self.spawn_then(
            async move { host.session_history(&get).await },
            move |actor, page| {
                actor.send_session_history(&phone, page);
            },
        );
    }

    /// Sends a page, dropping what does not fit: first the thumbnails of the oldest exchanges that have
    /// some, then the oldest exchanges (`more` turns true, so the phone asks again from the oldest one it
    /// got), then half of the text of the single one left.
    fn send_session_history(&mut self, phone_id: &str, mut page: wire::SessionHistory) {
        while items_bytes(&page) > SESSION_HISTORY_BYTES && page.shrink() {}
        while !self.send(phone_id, &page) {
            if !page.shrink() {
                return;
            }
        }
    }

    pub(super) fn on_history_exchange_get(
        &mut self,
        phone_id: &str,
        get: wire::HistoryExchangeGet,
    ) {
        let host = self.host.clone();
        let phone = phone_id.to_string();
        self.spawn_then(
            async move { host.history_exchange(&get).await },
            move |actor, answer| {
                actor.send_history_exchange(&phone, answer);
            },
        );
    }

    /// Sends the exchange, taking thumbnails and then text off it until it fits like a page does.
    fn send_history_exchange(&mut self, phone_id: &str, mut answer: wire::HistoryExchange) {
        while exchange_bytes(&answer) > SESSION_HISTORY_BYTES && answer.shrink() {}
        while !self.send(phone_id, &answer) {
            if !answer.shrink() {
                return;
            }
        }
    }
}

/// The size of what the budget measures: the exchanges as they go on the wire.
fn items_bytes(page: &wire::SessionHistory) -> usize {
    serde_json::to_vec(&page.items)
        .map(|bytes| bytes.len())
        .unwrap_or(0)
}

fn exchange_bytes(answer: &wire::HistoryExchange) -> usize {
    serde_json::to_vec(&answer.item)
        .map(|bytes| bytes.len())
        .unwrap_or(0)
}
