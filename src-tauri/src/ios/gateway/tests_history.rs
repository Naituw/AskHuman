//! `history.session.get` (docs/specs/ios-session-history.md §5–§6) against a scripted host.

use std::time::Duration;

use super::testkit::*;
use crate::ios::wire::{self, t, SESSION_HISTORY_BYTES, SESSION_HISTORY_PAGE_MAX};

/// Lets the hello burst and the one the Mac repeats a second after it connected pass.
async fn settle_hello(phone: &mut TestPhone) {
    phone.recv(t::SLOT_STATE).await;
    tokio::time::sleep(Duration::from_millis(1_300)).await;
    phone.clear_queue();
}

async fn live() -> (Fixture, TestPhone) {
    let fx = Fixture::new().await;
    let mut phone = fx.live_phone().await;
    settle_hello(&mut phone).await;
    (fx, phone)
}

fn get(req_id: &str, key: &str, limit: u32) -> wire::SessionHistoryGet {
    wire::SessionHistoryGet {
        req_id: req_id.into(),
        agent_key: key.into(),
        before_ms: 9_000,
        before_id: None,
        limit,
    }
}

fn question(text: &str, input_chars: usize) -> wire::SessionQuestion {
    wire::SessionQuestion {
        text: text.into(),
        selected: Vec::new(),
        input: (input_chars > 0).then(|| "i".repeat(input_chars)),
        images: 0,
        files: 0,
    }
}

fn exchange(
    index: usize,
    message_chars: usize,
    qs: Vec<wire::SessionQuestion>,
) -> wire::SessionExchange {
    wire::SessionExchange {
        id: format!("e{index}"),
        at: 8_000 - index as i64,
        channel: "popup".into(),
        markdown: true,
        message: "m".repeat(message_chars),
        attachments: 0,
        qs,
        notes: 0,
    }
}

fn page_of(key: &str, items: Vec<wire::SessionExchange>, more: bool) -> wire::SessionHistory {
    wire::SessionHistory {
        req_id: String::new(),
        agent_key: key.into(),
        items,
        more,
    }
}

fn items_bytes(page: &wire::SessionHistory) -> usize {
    serde_json::to_vec(&page.items).unwrap().len()
}

#[tokio::test(start_paused = true)]
async fn the_mac_announces_the_session_history_capability() {
    let fx = Fixture::new().await;
    let mut phone = fx.pair_phone().await;
    phone.set_presence(true, true).await;
    let hello = phone.hello().await;
    assert!(hello.caps.session_history);
}

#[tokio::test(start_paused = true)]
async fn a_page_comes_from_the_host_and_hands_back_the_phones_number() {
    let (fx, mut phone) = live().await;
    fx.host.session_pages.lock().unwrap().insert(
        "codex:s1".into(),
        page_of(
            "codex:s1",
            vec![
                exchange(0, 20, vec![question("Which?", 5)]),
                exchange(1, 10, vec![question("Sure?", 0)]),
            ],
            true,
        ),
    );

    let mut request = get("41", "codex:s1", 8);
    request.before_id = Some("cursor".into());
    phone.send(&request).await;
    let page: wire::SessionHistory = phone.recv_body(t::HISTORY_SESSION).await;

    assert_eq!(page.req_id, "41");
    assert_eq!(page.agent_key, "codex:s1");
    assert_eq!(
        page.items
            .iter()
            .map(|item| item.id.as_str())
            .collect::<Vec<_>>(),
        ["e0", "e1"]
    );
    assert!(page.more);
    assert_eq!(page.items[0].qs[0].text, "Which?");
    // The host saw the cursor as the phone sent it.
    let calls = fx.host.session_history_calls.lock().unwrap().clone();
    assert_eq!(calls, [request]);
}

#[tokio::test(start_paused = true)]
async fn the_limit_is_kept_within_range_before_the_host_sees_it() {
    let (fx, mut phone) = live().await;
    phone.send(&get("1", "codex:s1", 0)).await;
    let _: wire::SessionHistory = phone.recv_body(t::HISTORY_SESSION).await;
    phone.send(&get("2", "codex:s1", 5_000)).await;
    let _: wire::SessionHistory = phone.recv_body(t::HISTORY_SESSION).await;

    let limits: Vec<u32> = fx
        .host
        .session_history_calls
        .lock()
        .unwrap()
        .iter()
        .map(|call| call.limit)
        .collect();
    assert_eq!(limits, [1, SESSION_HISTORY_PAGE_MAX as u32]);
}

#[tokio::test(start_paused = true)]
async fn a_session_the_host_knows_nothing_of_gets_an_empty_page_not_an_error() {
    let (_fx, mut phone) = live().await;
    phone.send(&get("9", "claude:nobody", 8)).await;
    let page: wire::SessionHistory = phone.recv_body(t::HISTORY_SESSION).await;
    assert_eq!(page.req_id, "9");
    assert_eq!(page.agent_key, "claude:nobody");
    assert!(page.items.is_empty());
    assert!(!page.more);
}

#[tokio::test(start_paused = true)]
async fn a_page_over_the_budget_gives_up_its_oldest_exchanges_and_says_there_is_more() {
    let (fx, mut phone) = live().await;
    // Twenty exchanges of about 22 KB each: far over the budget of a page.
    let items: Vec<_> = (0..SESSION_HISTORY_PAGE_MAX)
        .map(|index| {
            exchange(
                index,
                4_000,
                (0..6)
                    .map(|_| question(&"q".repeat(1_000), 2_000))
                    .collect(),
            )
        })
        .collect();
    let scripted = page_of("codex:s1", items, false);
    assert!(items_bytes(&scripted) > SESSION_HISTORY_BYTES);
    fx.host
        .session_pages
        .lock()
        .unwrap()
        .insert("codex:s1".into(), scripted);

    phone
        .send(&get("5", "codex:s1", SESSION_HISTORY_PAGE_MAX as u32))
        .await;
    let page: wire::SessionHistory = phone.recv_body(t::HISTORY_SESSION).await;

    assert!(!page.items.is_empty());
    assert!(page.items.len() < SESSION_HISTORY_PAGE_MAX);
    assert!(items_bytes(&page) <= SESSION_HISTORY_BYTES);
    // The newest ones stay, in order, and the phone is told to ask again from the oldest it got.
    for (index, item) in page.items.iter().enumerate() {
        assert_eq!(item.id, format!("e{index}"));
    }
    assert!(page.more);
}

#[tokio::test(start_paused = true)]
async fn one_exchange_that_is_still_too_large_is_cut_down_and_not_dropped() {
    let (fx, mut phone) = live().await;
    // A hundred questions of 3 KB each in a single exchange.
    let huge = exchange(
        0,
        4_000,
        (0..100)
            .map(|_| question(&"q".repeat(1_000), 2_000))
            .collect(),
    );
    let scripted = page_of("codex:s1", vec![huge], false);
    assert!(items_bytes(&scripted) > SESSION_HISTORY_BYTES);
    fx.host
        .session_pages
        .lock()
        .unwrap()
        .insert("codex:s1".into(), scripted);

    phone.send(&get("6", "codex:s1", 8)).await;
    let page: wire::SessionHistory = phone.recv_body(t::HISTORY_SESSION).await;

    assert_eq!(page.items.len(), 1);
    assert!(items_bytes(&page) <= SESSION_HISTORY_BYTES);
    assert!(page.items[0].message.ends_with('…'));
    assert!(
        !page.more,
        "cutting the text of one exchange hides no other"
    );
    assert_eq!(page.items[0].qs.len(), 100, "every question is still there");
}
