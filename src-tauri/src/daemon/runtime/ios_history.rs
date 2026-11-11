//! A session's earlier exchanges for the phone, Mac side (docs/specs/ios-session-history.md §6.2).
//!
//! Reads the reply history (`history.rs`) of one exact native Agent session, the same
//! definition of "a real session" as `reply-history.md` F1, and projects every entry with the pure
//! `ios::history_view`. The read is blocking file IO, so it runs in `spawn_blocking`.

use crate::history::SendsPage;
use crate::ios::history_view::exchange_of;
use crate::ios::views::split_agent_key;
use crate::ios::wire::{self, SESSION_HISTORY_PAGE_MAX};

/// The page for one `history.session.get`. An unreadable key, a session without records and a
/// history that is switched off all give an empty page, never an error.
pub(super) async fn session_history(get: &wire::SessionHistoryGet) -> wire::SessionHistory {
    let get = get.clone();
    let fallback = wire::SessionHistory::empty(&get.req_id, &get.agent_key);
    tokio::task::spawn_blocking(move || {
        // Only the general settings are needed, so the keychain is not read.
        let enabled = crate::config::AppConfig::load_without_secrets()
            .general
            .history_limit
            > 0;
        page_with(&get, enabled, crate::history::sends_for_session_before)
    })
    .await
    .unwrap_or(fallback)
}

/// `read` fetches the exchanges older than a cursor: `(agent kind, session id, before_ms, before_id,
/// limit)`. The daemon passes the history file's reader; the tests pass a script.
fn page_with(
    get: &wire::SessionHistoryGet,
    enabled: bool,
    read: impl FnOnce(&str, &str, i64, Option<&str>, usize) -> SendsPage,
) -> wire::SessionHistory {
    let mut page = wire::SessionHistory::empty(&get.req_id, &get.agent_key);
    let Some((kind, session_id)) = split_agent_key(&get.agent_key) else {
        return page;
    };
    if !enabled {
        return page;
    }
    let limit = (get.limit as usize).clamp(1, SESSION_HISTORY_PAGE_MAX);
    let found = read(
        kind.as_str(),
        session_id,
        get.before_ms,
        get.before_id.as_deref(),
        limit,
    );
    page.items = found.entries.iter().map(exchange_of).collect();
    page.more = found.more;
    page
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::*;
    use crate::history::HistoryEntry;
    use crate::models::{ChannelAction, MessagePrompt, Question};

    fn get(key: &str, limit: u32) -> wire::SessionHistoryGet {
        wire::SessionHistoryGet {
            req_id: "7".into(),
            agent_key: key.into(),
            before_ms: 1_000,
            before_id: Some("cursor-id".into()),
            limit,
        }
    }

    fn entry(id: &str, at: i64) -> HistoryEntry {
        HistoryEntry {
            id: id.into(),
            timestamp_ms: at,
            project: "/work".into(),
            source: "Codex".into(),
            agent_kind: Some("codex".into()),
            agent_session_id: Some("s:1".into()),
            mcp_instance_id: None,
            channel: "popup".into(),
            action: ChannelAction::Send,
            is_markdown: false,
            message: MessagePrompt::new("hello".into(), Vec::new()),
            questions: vec![Question::new("ok?".into(), Vec::new())],
            answers: Vec::new(),
            annotations: Vec::new(),
        }
    }

    type Call = (String, String, i64, Option<String>, usize);

    /// Runs `page_with` against a script and returns the page with the calls the reader saw.
    fn run(
        get: &wire::SessionHistoryGet,
        enabled: bool,
        found: SendsPage,
    ) -> (wire::SessionHistory, Vec<Call>) {
        let calls = RefCell::new(Vec::new());
        let page = page_with(get, enabled, |kind, session, ms, id, limit| {
            calls.borrow_mut().push((
                kind.to_string(),
                session.to_string(),
                ms,
                id.map(str::to_string),
                limit,
            ));
            found
        });
        (page, calls.into_inner())
    }

    fn found(ids: &[&str], more: bool) -> SendsPage {
        SendsPage {
            entries: ids
                .iter()
                .enumerate()
                .map(|(index, id)| entry(id, 500 - index as i64))
                .collect(),
            more,
        }
    }

    #[test]
    fn the_cursor_and_the_exact_session_reach_the_reader_and_the_page_echoes_the_request() {
        // The session id keeps its own colons.
        let (page, calls) = run(&get("codex:s:1", 8), true, found(&["b", "a"], true));
        assert_eq!(
            calls,
            [(
                "codex".to_string(),
                "s:1".to_string(),
                1_000,
                Some("cursor-id".to_string()),
                8
            )]
        );
        assert_eq!(page.req_id, "7");
        assert_eq!(page.agent_key, "codex:s:1");
        assert_eq!(
            page.items
                .iter()
                .map(|item| item.id.as_str())
                .collect::<Vec<_>>(),
            ["b", "a"]
        );
        assert!(page.more);
        assert_eq!(page.items[0].message, "hello");
        assert_eq!(page.items[0].qs[0].text, "ok?");
    }

    #[test]
    fn the_limit_is_kept_within_range() {
        let (_, calls) = run(&get("codex:s:1", 0), true, found(&[], false));
        assert_eq!(calls[0].4, 1);
        let (_, calls) = run(&get("codex:s:1", 500), true, found(&[], false));
        assert_eq!(calls[0].4, SESSION_HISTORY_PAGE_MAX);
    }

    #[test]
    fn a_key_that_is_not_a_session_gives_an_empty_page_and_reads_nothing() {
        for key in ["", "codex", "codex:", ":s1", "nope:s1", "mcp:instance"] {
            let (page, calls) = run(&get(key, 8), true, found(&["a"], true));
            assert!(calls.is_empty(), "{key} was read");
            assert!(page.items.is_empty() && !page.more, "{key} is not empty");
            assert_eq!(page.req_id, "7");
            assert_eq!(page.agent_key, key);
        }
    }

    #[test]
    fn a_switched_off_history_gives_an_empty_page_and_reads_nothing() {
        let (page, calls) = run(&get("codex:s1", 8), false, found(&["a"], true));
        assert!(calls.is_empty());
        assert!(page.items.is_empty() && !page.more);
    }
}
