//! `history.session.get` (docs/specs/ios-session-history.md §5–§6) and `history.exchange.get` (docs/specs/
//! ios-history-pictures.md §6) against a scripted host.

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
        pictures: false,
    }
}

fn question(text: &str, input_chars: usize) -> wire::SessionQuestion {
    wire::SessionQuestion {
        text: text.into(),
        selected: Vec::new(),
        input: (input_chars > 0).then(|| "i".repeat(input_chars)),
        images: 0,
        files: 0,
        pictures: Vec::new(),
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
        pictures: Vec::new(),
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

// ---------------------------------------------------------------- pictures

/// A picture reference with a thumbnail of `thumb_chars` characters of base64 (none when 0).
fn picture(name: &str, thumb_chars: usize) -> wire::AssetRef {
    wire::AssetRef {
        asset_id: format!("pic-{name}"),
        name: name.into(),
        size: 100_000,
        mime: "image/png".into(),
        is_image: true,
        thumb: (thumb_chars > 0).then(|| "t".repeat(thumb_chars)),
        too_large: false,
        gone: false,
    }
}

fn thumbs_in(exchange: &wire::SessionExchange) -> usize {
    exchange
        .pictures
        .iter()
        .chain(exchange.qs.iter().flat_map(|q| q.pictures.iter()))
        .filter(|picture| picture.thumb.is_some())
        .count()
}

fn pictures_in(exchange: &wire::SessionExchange) -> usize {
    exchange.pictures.len() + exchange.qs.iter().map(|q| q.pictures.len()).sum::<usize>()
}

#[tokio::test(start_paused = true)]
async fn the_mac_announces_the_history_pictures_capability() {
    let fx = Fixture::new().await;
    let mut phone = fx.pair_phone().await;
    phone.set_presence(true, true).await;
    let hello = phone.hello().await;
    assert!(hello.caps.history_pictures);
}

#[tokio::test(start_paused = true)]
async fn the_pictures_flag_reaches_the_host_as_the_phone_sent_it() {
    let (fx, mut phone) = live().await;
    let mut with = get("1", "codex:s1", 8);
    with.pictures = true;
    phone.send(&with).await;
    let _: wire::SessionHistory = phone.recv_body(t::HISTORY_SESSION).await;
    phone.send(&get("2", "codex:s1", 8)).await;
    let _: wire::SessionHistory = phone.recv_body(t::HISTORY_SESSION).await;
    let flags: Vec<bool> = fx
        .host
        .session_history_calls
        .lock()
        .unwrap()
        .iter()
        .map(|call| call.pictures)
        .collect();
    assert_eq!(flags, [true, false]);
}

#[tokio::test(start_paused = true)]
async fn a_page_over_the_budget_loses_thumbnails_before_it_loses_any_exchange() {
    let (fx, mut phone) = live().await;
    // Ten exchanges with twelve 8 KB thumbnails each: about 1 MB, far over the budget of a page. The text
    // is small, so taking thumbnails off is enough.
    let items: Vec<_> = (0..10)
        .map(|index| {
            let mut item = exchange(index, 200, vec![question("which?", 10)]);
            item.pictures = (0..12)
                .map(|n| picture(&format!("m{index}-{n}.png"), 8_000))
                .collect();
            item
        })
        .collect();
    let scripted = page_of("codex:s1", items, false);
    assert!(items_bytes(&scripted) > 3 * SESSION_HISTORY_BYTES);
    fx.host
        .session_pages
        .lock()
        .unwrap()
        .insert("codex:s1".into(), scripted);

    let mut request = get("5", "codex:s1", SESSION_HISTORY_PAGE_MAX as u32);
    request.pictures = true;
    phone.send(&request).await;
    let page: wire::SessionHistory = phone.recv_body(t::HISTORY_SESSION).await;

    assert_eq!(page.items.len(), 10, "no exchange was given up");
    assert!(!page.more);
    assert!(items_bytes(&page) <= SESSION_HISTORY_BYTES);
    // Every picture is still there, to be shown as a placeholder and opened.
    assert!(page.items.iter().all(|item| pictures_in(item) == 12));
    // The newest exchanges kept their thumbnails, the oldest lost theirs first.
    assert_eq!(thumbs_in(&page.items[0]), 12);
    assert_eq!(thumbs_in(&page.items[9]), 0);
    let stripped = page
        .items
        .iter()
        .filter(|item| thumbs_in(item) == 0)
        .count();
    assert!(stripped > 0 && stripped < 10, "{stripped} stripped");
    // Thumbs come off whole exchanges, oldest first: once one has them none of the newer lacks them.
    let first_stripped = page.items.iter().position(|i| thumbs_in(i) == 0).unwrap();
    assert!(page.items[first_stripped..]
        .iter()
        .all(|i| thumbs_in(i) == 0));
}

fn history_get(req_id: &str, id: &str) -> wire::HistoryExchangeGet {
    wire::HistoryExchangeGet {
        req_id: req_id.into(),
        id: id.into(),
    }
}

#[tokio::test(start_paused = true)]
async fn an_exchange_comes_from_the_host_by_its_id_and_hands_back_the_phones_number() {
    let (fx, mut phone) = live().await;
    let mut record = exchange(0, 30, vec![question("which?", 5)]);
    record.id = "req-1".into();
    record.pictures = vec![picture("a.png", 100)];
    fx.host
        .history_exchanges
        .lock()
        .unwrap()
        .insert("req-1".into(), record.clone());

    let request = history_get("x41", "req-1");
    phone.send(&request).await;
    let answer: wire::HistoryExchange = phone.recv_body(t::HISTORY_EXCHANGE).await;
    assert_eq!(
        (answer.req_id.as_str(), answer.id.as_str()),
        ("x41", "req-1")
    );
    assert_eq!(answer.item, Some(record));
    assert_eq!(*fx.host.history_exchange_calls.lock().unwrap(), [request]);
}

#[tokio::test(start_paused = true)]
async fn an_exchange_the_host_does_not_have_is_an_answer_without_an_item() {
    let (_fx, mut phone) = live().await;
    phone.send(&history_get("x9", "nope")).await;
    let answer: wire::HistoryExchange = phone.recv_body(t::HISTORY_EXCHANGE).await;
    assert_eq!((answer.req_id.as_str(), answer.id.as_str()), ("x9", "nope"));
    assert!(answer.item.is_none());
}

#[tokio::test(start_paused = true)]
async fn an_exchange_over_the_budget_loses_its_thumbnails_and_keeps_its_text() {
    let (fx, mut phone) = live().await;
    // A message with twelve pictures and three questions with twelve each, at 8 KB a thumbnail: about 380 KB.
    let mut record = exchange(0, 500, Vec::new());
    record.id = "req-2".into();
    record.pictures = (0..12)
        .map(|n| picture(&format!("m{n}.png"), 8_000))
        .collect();
    record.qs = (0..3)
        .map(|q| {
            let mut asked = question("which?", 20);
            asked.pictures = (0..12)
                .map(|n| picture(&format!("q{q}-{n}.png"), 8_000))
                .collect();
            asked
        })
        .collect();
    assert!(serde_json::to_vec(&record).unwrap().len() > SESSION_HISTORY_BYTES);
    fx.host
        .history_exchanges
        .lock()
        .unwrap()
        .insert("req-2".into(), record);

    phone.send(&history_get("x3", "req-2")).await;
    let answer: wire::HistoryExchange = phone.recv_body(t::HISTORY_EXCHANGE).await;
    let item = answer.item.expect("the exchange");
    assert!(serde_json::to_vec(&item).unwrap().len() <= SESSION_HISTORY_BYTES);
    assert_eq!(pictures_in(&item), 48, "every picture stays");
    assert_eq!(thumbs_in(&item), 0);
    assert_eq!(item.message.chars().count(), 500, "the text is untouched");
}
