//! The popup footer's pointer to a wizard page that still has an open question
//! (`docs/specs/onboarding.md`). Table driven: pointing at another page later is one more row.

use crate::onboarding::state::{self, DOMAIN_REMOTE};
use serde::Serialize;

/// What the popup shows: the pointer's id (also the key of its text) and the wizard page it opens.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SetupHint {
    pub id: String,
    pub page: String,
}

struct Entry {
    id: &'static str,
    /// The decision `popup_setup_hint_dismiss` records for it (a `state::mark_skipped` domain).
    domain: &'static str,
    page: &'static str,
    /// Whether its question is still open. Asked lazily, in table order, so a popup never pays
    /// for a domain it does not show.
    open: fn() -> bool,
}

const TABLE: &[Entry] = &[Entry {
    id: "remote",
    domain: DOMAIN_REMOTE,
    page: "remote",
    open: remote_open,
}];

fn remote_open() -> bool {
    !state::remote_decided()
}

fn first_open(table: &[Entry]) -> Option<SetupHint> {
    table
        .iter()
        .find(|entry| (entry.open)())
        .map(|entry| SetupHint {
            id: entry.id.to_string(),
            page: entry.page.to_string(),
        })
}

/// The first pointer whose question is still open.
pub fn next_hint() -> Option<SetupHint> {
    first_open(TABLE)
}

/// The decision domain a dismissed pointer records; `None` for an id the table does not know.
pub fn domain_of(id: &str) -> Option<&'static str> {
    TABLE
        .iter()
        .find(|entry| entry.id == id)
        .map(|entry| entry.domain)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn open() -> bool {
        true
    }

    fn settled() -> bool {
        false
    }

    fn entry(id: &'static str, open: fn() -> bool) -> Entry {
        Entry {
            id,
            domain: "remote",
            page: "remote",
            open,
        }
    }

    #[test]
    fn the_first_open_row_wins_and_settled_rows_are_skipped() {
        let table = [entry("a", settled), entry("b", open), entry("c", open)];
        assert_eq!(
            first_open(&table),
            Some(SetupHint {
                id: "b".into(),
                page: "remote".into()
            })
        );
    }

    #[test]
    fn nothing_is_pointed_at_once_every_question_is_settled() {
        assert_eq!(
            first_open(&[entry("a", settled), entry("b", settled)]),
            None
        );
        assert_eq!(first_open(&[]), None);
    }

    #[test]
    fn later_rows_are_not_asked_once_an_earlier_one_is_open() {
        fn must_not_run() -> bool {
            panic!("a later row was evaluated");
        }
        let table = [entry("a", open), entry("b", must_not_run)];
        assert_eq!(
            first_open(&table).map(|hint| hint.id),
            Some("a".to_string())
        );
    }

    #[test]
    fn the_remote_pointer_is_the_only_one_and_records_the_remote_decision() {
        assert_eq!(domain_of("remote"), Some(DOMAIN_REMOTE));
        assert_eq!(domain_of("agents"), None);
        assert_eq!(domain_of(""), None);
    }

    #[test]
    fn every_row_records_a_domain_the_state_accepts() {
        for entry in TABLE {
            assert!(
                crate::uistate::SKIPPABLE_DOMAINS.contains(&entry.domain),
                "{} records {}, which cannot be skipped",
                entry.id,
                entry.domain
            );
        }
    }

    #[test]
    fn the_hint_serializes_with_its_page() {
        let value = serde_json::to_value(SetupHint {
            id: "remote".into(),
            page: "remote".into(),
        })
        .unwrap();
        assert_eq!(
            value,
            serde_json::json!({ "id": "remote", "page": "remote" })
        );
    }
}
