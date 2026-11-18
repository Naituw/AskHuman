//! What a phone is sent, and how it is sent, once the traffic reduction of
//! `docs/specs/ios-traffic-reduction.md` is in: only what changed when the phone becomes live (R1), a lean Agents
//! list for a working Agent nobody watches (L1), compressed list streams (C), live views that belong to the phone's
//! session (T2) and the Mac's own account of its traffic by message type (T6).

use std::time::Duration;

use serde_json::json;

use super::requests::{DeliverRequest, Sink};
use super::testkit::*;
use super::*;
use crate::ios::envelope::{self, Plain, Scope};
use crate::ios::wire::{self, t};

const SECOND: Duration = Duration::from_secs(1);

fn working(n: usize) -> Vec<AgentView> {
    (0..n)
        .map(|i| sample_agent(&format!("claude:s{i}"), "working"))
        .collect()
}

fn watch(agent_key: &str) -> wire::WatchOpen {
    wire::WatchOpen {
        agent_key: agent_key.into(),
    }
}

/// A fixture whose Mac connected a while ago: its own resync a second after it did is over, so what a phone is sent
/// afterwards is what the phone's own steps bring.
async fn quiet_fixture(agents: Vec<AgentView>) -> Fixture {
    let fx = Fixture::new().await;
    tokio::time::sleep(2 * SECOND).await;
    fx.host.set_agents(agents);
    fx
}

/// A paired phone that said hello and has not reported the foreground yet, as the app is while its first sync
/// runs. What the hello brought is read and dropped.
async fn hello_in_background(fx: &Fixture, configure: impl FnOnce(&mut TestPhone)) -> TestPhone {
    let mut phone = fx.pair_phone().await;
    configure(&mut phone);
    phone.set_presence(true, false).await;
    phone.hello().await;
    phone.recv(t::SLOT_STATE).await;
    tokio::time::sleep(SECOND).await;
    phone.clear_queue();
    phone
}

/// A paired phone that is live (hello said, foreground reported) and has read the first snapshots.
async fn live(fx: &Fixture, configure: impl FnOnce(&mut TestPhone)) -> TestPhone {
    let mut phone = hello_in_background(fx, configure).await;
    phone.set_presence(true, true).await;
    tokio::time::sleep(SECOND).await;
    phone.clear_queue();
    phone
}

/// The types of what the phone was sent after `mark` (a `phone.received.len()` taken earlier), in order.
fn since(phone: &TestPhone, mark: usize) -> Vec<&str> {
    phone.received[mark..]
        .iter()
        .map(|r| r.ty.as_str())
        .collect()
}

fn count(phone: &TestPhone, mark: usize, ty: &str) -> usize {
    since(phone, mark).iter().filter(|t| **t == ty).count()
}

/// The bytes of what the phone was sent after `mark`.
fn bytes_since(phone: &TestPhone, mark: usize) -> usize {
    phone.received[mark..].iter().map(|r| r.bytes).sum()
}

fn deliver_ask(fx: &Fixture, id: &str, text: Option<&str>) {
    let (mut ask, mut view) = sample_ask(id);
    if let Some(text) = text {
        Arc::make_mut(&mut ask).message.text = text.to_string();
        if let Some(v) = view.ask.as_mut() {
            v.message.text = text.to_string();
        }
    }
    fx.gateway.deliver(DeliverRequest {
        view,
        sink: Sink::Ask(FakeAsk::new()),
        ask: Some(ask),
        deadline_ms: None,
        assets: Vec::new(),
    });
}

// ---------------------------------------------------------------- R1: only what changed

#[tokio::test(start_paused = true)]
async fn a_phone_that_becomes_live_with_nothing_changed_is_sent_nothing() {
    let fx = quiet_fixture(working(3)).await;
    let mut phone = hello_in_background(&fx, |_| {}).await;
    let mark = phone.received.len();

    phone.set_presence(true, true).await;
    tokio::time::sleep(3 * SECOND).await;
    phone.clear_queue();
    assert_eq!(since(&phone, mark), Vec::<&str>::new());
}

#[tokio::test(start_paused = true)]
async fn only_the_streams_that_changed_are_sent_when_the_phone_becomes_live() {
    let fx = quiet_fixture(working(3)).await;
    let mut phone = hello_in_background(&fx, |_| {}).await;
    let mark = phone.received.len();

    // A request arrives while the phone is not live yet, and one Agent finishes.
    deliver_ask(&fx, "late", None);
    fx.host
        .update_agent("claude:s1", |a| a.state = "idle".into());
    tokio::time::sleep(SECOND).await;
    phone.set_presence(true, true).await;
    tokio::time::sleep(3 * SECOND).await;
    phone.clear_queue();

    assert_eq!(
        since(&phone, mark),
        [t::INBOX_SNAPSHOT, t::AGENTS_DELTA],
        "the follow, slot and away states are what the hello sent"
    );
}

#[tokio::test(start_paused = true)]
async fn the_agents_the_phone_was_sent_are_not_repeated_in_the_catch_up() {
    let fx = quiet_fixture(working(6)).await;
    let mut phone = hello_in_background(&fx, |_| {}).await;
    let mark = phone.received.len();
    fx.host
        .update_agent("claude:s4", |a| a.title = "Renamed".into());
    tokio::time::sleep(SECOND).await;

    phone.set_presence(true, true).await;
    let delta: wire::AgentsDelta = phone.recv_body(t::AGENTS_DELTA).await;
    assert_eq!(delta.upsert.len(), 1, "one of six");
    assert_eq!(delta.upsert[0].key, "claude:s4");
    assert_eq!(delta.upsert[0].title, "Renamed");
    assert_eq!(count(&phone, mark, t::AGENTS_SNAPSHOT), 0);
}

#[tokio::test(start_paused = true)]
async fn a_stream_the_phone_was_never_sent_counts_as_changed() {
    let fx = quiet_fixture(working(1)).await;
    // The Mac had no away mode when the phone said hello, so the hello brought no away state.
    fx.host
        .away_supported
        .store(false, std::sync::atomic::Ordering::SeqCst);
    let mut phone = hello_in_background(&fx, |_| {}).await;
    fx.host
        .away_supported
        .store(true, std::sync::atomic::Ordering::SeqCst);
    let mark = phone.received.len();

    phone.set_presence(true, true).await;
    tokio::time::sleep(2 * SECOND).await;
    phone.clear_queue();
    assert_eq!(since(&phone, mark), [t::AWAY_STATE]);
}

#[tokio::test(start_paused = true)]
async fn the_stream_goes_on_from_the_revision_of_the_catch_up() {
    let fx = quiet_fixture(working(2)).await;
    let mut phone = hello_in_background(&fx, |_| {}).await;
    fx.host
        .update_agent("claude:s0", |a| a.state = "idle".into());
    tokio::time::sleep(SECOND).await;
    phone.set_presence(true, true).await;
    let caught_up: wire::AgentsDelta = phone.recv_body(t::AGENTS_DELTA).await;

    fx.host
        .update_agent("claude:s1", |a| a.state = "idle".into());
    let next: wire::AgentsDelta = phone.recv_body(t::AGENTS_DELTA).await;
    assert_eq!(next.rev, caught_up.rev + 1);
}

// ---------------------------------------------------------------- L1: a lean Agents list

/// Thirty seconds of a working Agent that takes a tool step every second.
async fn thirty_busy_seconds(fx: &Fixture, key: &str) {
    for step in 1..=30u64 {
        fx.host.update_agent(key, |a| {
            a.turn_steps = step;
            a.current_tool = Some(format!("Shell step {step}"));
            a.last_activity_at += 1_000;
        });
        tokio::time::sleep(SECOND).await;
    }
}

#[tokio::test(start_paused = true)]
async fn a_lean_phone_is_not_told_the_live_activity_of_a_working_agent_it_does_not_watch() {
    let fx = quiet_fixture(working(2)).await;
    let mut phone = live(&fx, |p| p.lean_agents = true).await;
    let mark = phone.received.len();

    thirty_busy_seconds(&fx, "claude:s0").await;
    phone.clear_queue();
    assert_eq!(
        since(&phone, mark),
        Vec::<&str>::new(),
        "tool, step count and activity time are not part of what the row shows"
    );
}

#[tokio::test(start_paused = true)]
async fn a_phone_that_did_not_say_lean_is_told_every_step() {
    let fx = quiet_fixture(working(2)).await;
    let mut phone = live(&fx, |_| {}).await;
    let mark = phone.received.len();

    thirty_busy_seconds(&fx, "claude:s0").await;
    phone.clear_queue();
    assert!(
        count(&phone, mark, t::AGENTS_DELTA) >= 25,
        "{:?}",
        since(&phone, mark)
    );
}

#[tokio::test(start_paused = true)]
async fn the_hello_ack_offers_the_lean_agents_list() {
    // The Mac leaves the activity out only for a phone that asked for it (`Hello::lean_agents`) and when it offered
    // it here (`Caps::lean_agents`): a phone reads this bit to know the Mac understood.
    let fx = quiet_fixture(working(1)).await;
    let mut phone = fx.pair_phone().await;
    phone.lean_agents = true;
    phone.set_presence(true, true).await;
    let ack = phone.hello().await;
    assert!(ack.caps.lean_agents);
}

#[tokio::test(start_paused = true)]
async fn a_lean_phone_is_told_a_change_of_what_the_row_shows_with_the_latest_activity() {
    let fx = quiet_fixture(working(2)).await;
    let mut phone = live(&fx, |p| p.lean_agents = true).await;
    thirty_busy_seconds(&fx, "claude:s0").await;
    phone.clear_queue();

    fx.host
        .update_agent("claude:s0", |a| a.title = "Renamed".into());
    let delta: wire::AgentsDelta = phone.recv_body(t::AGENTS_DELTA).await;
    assert_eq!(delta.upsert.len(), 1);
    let view = &delta.upsert[0];
    assert_eq!(view.title, "Renamed");
    assert_eq!(
        (view.turn_steps, view.current_tool.as_deref()),
        (30, Some("Shell step 30")),
        "the version it is sent is the latest one"
    );
}

#[tokio::test(start_paused = true)]
async fn a_lean_phone_is_told_when_an_agent_stops_working_and_when_it_starts_again() {
    let fx = quiet_fixture(working(1)).await;
    let mut phone = live(&fx, |p| p.lean_agents = true).await;
    thirty_busy_seconds(&fx, "claude:s0").await;
    phone.clear_queue();

    fx.host
        .update_agent("claude:s0", |a| a.state = "idle".into());
    let stopped: wire::AgentsDelta = phone.recv_body(t::AGENTS_DELTA).await;
    assert_eq!(stopped.upsert[0].state, "idle");
    assert_eq!(stopped.upsert[0].turn_steps, 30);

    // An idle Agent is not a working one: every change of it is told.
    fx.host.update_agent("claude:s0", |a| a.turn_steps = 31);
    let changed: wire::AgentsDelta = phone.recv_body(t::AGENTS_DELTA).await;
    assert_eq!(changed.upsert[0].turn_steps, 31);

    fx.host
        .update_agent("claude:s0", |a| a.state = "working".into());
    let started: wire::AgentsDelta = phone.recv_body(t::AGENTS_DELTA).await;
    assert_eq!(started.upsert[0].state, "working");
}

#[tokio::test(start_paused = true)]
async fn a_lean_phone_is_told_every_change_of_an_agent_it_watches() {
    let fx = quiet_fixture(working(2)).await;
    fx.host.set_frame("claude:s0", "a", "Working", false);
    let mut phone = live(&fx, |p| p.lean_agents = true).await;
    phone.send(&watch("claude:s0")).await;
    phone.recv(t::WATCH_FRAME).await;
    let mark = phone.received.len();

    for step in 1..=5u64 {
        fx.host.update_agent("claude:s0", |a| a.turn_steps = step);
        tokio::time::sleep(2 * SECOND).await;
    }
    phone.clear_queue();
    assert_eq!(
        count(&phone, mark, t::AGENTS_DELTA),
        5,
        "the live view's watchdog reads the list for it"
    );

    // Another Agent of the same list is still not told.
    for step in 1..=5u64 {
        fx.host.update_agent("claude:s1", |a| a.turn_steps = step);
        tokio::time::sleep(2 * SECOND).await;
    }
    phone.clear_queue();
    assert_eq!(count(&phone, mark, t::AGENTS_DELTA), 5);
}

#[tokio::test(start_paused = true)]
async fn watching_an_agent_brings_the_latest_activity_that_a_lean_phone_was_not_sent() {
    let fx = quiet_fixture(working(1)).await;
    fx.host.set_frame("claude:s0", "a", "Working", false);
    let mut phone = live(&fx, |p| p.lean_agents = true).await;
    thirty_busy_seconds(&fx, "claude:s0").await;
    phone.clear_queue();
    let mark = phone.received.len();

    phone.send(&watch("claude:s0")).await;
    let delta: wire::AgentsDelta = phone.recv_body(t::AGENTS_DELTA).await;
    assert_eq!(delta.upsert[0].turn_steps, 30);

    // Closing the view makes it an ordinary row again.
    phone
        .send(&wire::WatchClose {
            agent_key: "claude:s0".into(),
        })
        .await;
    fx.host.update_agent("claude:s0", |a| a.turn_steps = 31);
    tokio::time::sleep(3 * SECOND).await;
    phone.clear_queue();
    assert_eq!(count(&phone, mark, t::AGENTS_DELTA), 1);
}

// ---------------------------------------------------------------- C: compression

#[tokio::test(start_paused = true)]
async fn the_list_streams_go_compressed_to_a_phone_that_asked_and_nothing_else_does() {
    let fx = quiet_fixture(working(12)).await;
    let mut phone = fx.pair_phone().await;
    // Enough pending requests that the inbox is worth compressing too.
    for i in 0..8 {
        deliver_ask(&fx, &format!("r{i}"), None);
    }
    tokio::time::sleep(SECOND).await;
    phone.compression = true;
    phone.set_presence(true, true).await;
    phone.hello().await;
    phone.recv(t::SLOT_STATE).await;
    tokio::time::sleep(SECOND).await;
    phone.clear_queue();

    let compressed: Vec<&str> = phone
        .received
        .iter()
        .filter(|r| r.compressed)
        .map(|r| r.ty.as_str())
        .collect();
    assert!(
        compressed.contains(&t::AGENTS_SNAPSHOT),
        "{:?}",
        phone.received
    );
    assert!(
        compressed.contains(&t::INBOX_SNAPSHOT),
        "{:?}",
        phone.received
    );
    for ty in &compressed {
        assert!(
            wire::is_compressible(ty),
            "`{ty}` is not a list stream, yet it went compressed"
        );
    }
    for plain in [t::HELLO_ACK, t::FOLLOW_STATE, t::SLOT_STATE] {
        assert!(phone.received.iter().any(|r| r.ty == plain));
    }
}

#[tokio::test(start_paused = true)]
async fn a_phone_that_did_not_ask_is_never_sent_a_compressed_message() {
    let fx = quiet_fixture(working(12)).await;
    let mut phone = live(&fx, |_| {}).await;
    for step in 1..=4u64 {
        fx.host
            .update_agent("claude:s0", |a| a.title = format!("Title {step}"));
        tokio::time::sleep(2 * SECOND).await;
    }
    phone.clear_queue();
    assert!(phone.received.len() > 5);
    assert!(
        phone.received.iter().all(|r| !r.compressed),
        "{:?}",
        phone.received
    );
}

#[tokio::test(start_paused = true)]
async fn compression_follows_the_latest_hello() {
    let fx = quiet_fixture(working(12)).await;
    let mut phone = live(&fx, |p| p.compression = true).await;
    let mark = phone.received.len();
    phone.compression = false;
    phone.hello().await;
    phone.recv(t::SLOT_STATE).await;
    tokio::time::sleep(SECOND).await;
    phone.clear_queue();

    let snapshots: Vec<&Received> = phone.received[mark..]
        .iter()
        .filter(|r| r.ty == t::AGENTS_SNAPSHOT)
        .collect();
    assert_eq!(snapshots.len(), 1);
    assert!(!snapshots[0].compressed);
}

#[tokio::test(start_paused = true)]
async fn a_compressed_delta_takes_much_fewer_bytes() {
    let fx = quiet_fixture(working(12)).await;
    let mut asking = live(&fx, |p| p.compression = true).await;
    let mut plain = live(&fx, |_| {}).await;
    let (mark_asking, mark_plain) = (asking.received.len(), plain.received.len());

    // Every Agent changes at once.
    for i in 0..12 {
        fx.host.update_agent(&format!("claude:s{i}"), |a| {
            a.title = format!("A new title {i}")
        });
    }
    tokio::time::sleep(3 * SECOND).await;
    asking.clear_queue();
    plain.clear_queue();

    let delta = |phone: &TestPhone, mark: usize| {
        phone.received[mark..]
            .iter()
            .find(|r| r.ty == t::AGENTS_DELTA)
            .cloned()
            .expect("a delta")
    };
    let (small, large) = (delta(&asking, mark_asking), delta(&plain, mark_plain));
    assert!(small.compressed && !large.compressed);
    assert!(
        small.bytes * 2 < large.bytes,
        "{} compressed against {} bytes",
        small.bytes,
        large.bytes
    );
}

#[tokio::test(start_paused = true)]
async fn the_text_of_a_request_is_never_compressed() {
    let fx = quiet_fixture(working(1)).await;
    let mut phone = live(&fx, |p| p.compression = true).await;
    let mark = phone.received.len();

    deliver_ask(
        &fx,
        "long",
        Some(&"A long question that repeats itself. ".repeat(60)),
    );
    let new: wire::RequestNew = phone.recv_body(t::REQUEST_NEW).await;
    assert_eq!(new.request.id, "long");
    let received = phone.received[mark..]
        .iter()
        .find(|r| r.ty == t::REQUEST_NEW)
        .unwrap();
    assert!(received.bytes > 2_000 && !received.compressed);
}

#[tokio::test(start_paused = true)]
async fn the_mac_refuses_a_compressed_message_from_a_phone() {
    let fx = quiet_fixture(Vec::new()).await;
    let mut phone = live(&fx, |_| {}).await;
    let keys = phone.keys.clone().unwrap();
    let topic = wire::topic_up(USER, &fx.mac_id);
    let ping = || {
        Plain::new(
            t::PING,
            json!({"t": 5, "padding": "x".repeat(1_500)}),
            BASE_MS,
        )
    };

    let compressed = envelope::seal_plain_with(
        &keys.p2m,
        Scope::Topic(&topic),
        &phone.phone_id,
        keys.kid,
        &ping(),
        true,
    );
    assert!(
        envelope::parse(&compressed).unwrap().z.is_some(),
        "the test must send a compressed envelope"
    );
    phone.send_raw_up(compressed).await;
    assert!(
        !phone.gets(t::PONG, 2 * SECOND).await,
        "phones do not compress, and the Mac does not open it"
    );

    // The same message, not compressed, is answered.
    let plain = envelope::seal_plain_with(
        &keys.p2m,
        Scope::Topic(&topic),
        &phone.phone_id,
        keys.kid,
        &ping(),
        false,
    );
    phone.send_raw_up(plain).await;
    phone.recv(t::PONG).await;
}

// ---------------------------------------------------------------- T2: live views follow the session

#[tokio::test(start_paused = true)]
async fn a_late_background_presence_leaves_the_live_views_of_the_phone_registered() {
    // The app opens its live view as soon as it has the Agents list, before it reports the foreground; a presence
    // that still says "background" can come in between.
    let fx = Fixture::new().await;
    fx.host
        .set_agents(vec![sample_agent("claude:s1", "working")]);
    fx.host.set_frame("claude:s1", "a", "Reading files", false);
    let mut phone = fx.pair_phone().await;
    phone.set_presence(true, false).await;
    phone.hello().await;
    phone.send(&watch("claude:s1")).await;
    phone.set_presence(true, false).await;

    phone.set_presence(true, true).await;
    let frame: wire::WatchFrame = phone.recv_body(t::WATCH_FRAME).await;
    assert_eq!(frame.seq, 1);
}

#[tokio::test(start_paused = true)]
async fn leaving_the_foreground_keeps_the_views_registered_but_serves_none() {
    let fx = Fixture::new().await;
    fx.host
        .set_agents(vec![sample_agent("claude:s1", "working")]);
    fx.host.set_frame("claude:s1", "a", "Working", false);
    let mut phone = fx.live_phone().await;
    phone.send(&watch("claude:s1")).await;
    phone.recv(t::WATCH_FRAME).await;
    assert_eq!(fx.gateway.live_watch_count(), 1);

    phone.set_presence(true, false).await;
    assert_eq!(
        fx.gateway.live_watch_count(),
        0,
        "nothing streams to a phone that is not in the foreground"
    );
    fx.host.set_frame("claude:s1", "b", "More work", false);
    assert!(!phone.gets(t::WATCH_FRAME, 6 * SECOND).await);

    // The same session is back in the foreground: its view goes on where it was.
    phone.set_presence(true, true).await;
    let frame: wire::WatchFrame = phone.recv_body(t::WATCH_FRAME).await;
    assert_eq!(frame.seq, 2);
    assert_eq!(frame.frame["text"], "More work");
}

#[tokio::test(start_paused = true)]
async fn a_hello_forgets_the_live_views_of_the_phone() {
    let fx = Fixture::new().await;
    fx.host
        .set_agents(vec![sample_agent("claude:s1", "working")]);
    fx.host.set_frame("claude:s1", "a", "Working", false);
    let mut phone = fx.live_phone().await;
    phone.send(&watch("claude:s1")).await;
    phone.recv(t::WATCH_FRAME).await;

    // The app started again: it holds no view and asks for the ones it still wants.
    phone.hello().await;
    assert_eq!(fx.gateway.live_watch_count(), 0);
    fx.host.set_frame("claude:s1", "b", "More work", false);
    assert!(!phone.gets(t::WATCH_FRAME, 5 * SECOND).await);
    phone.send(&watch("claude:s1")).await;
    let frame: wire::WatchFrame = phone.recv_body(t::WATCH_FRAME).await;
    assert_eq!(frame.seq, 1);
}

// ---------------------------------------------------------------- T6: the Mac's own account

#[tokio::test(start_paused = true)]
async fn the_mac_counts_what_it_sent_and_received_by_message_type() {
    let fx = Fixture::new().await;
    let mut phone = fx.live_phone().await;
    phone.send(&wire::Ping { t: 1 }).await;
    phone.recv(t::PONG).await;
    // A type nobody registered becomes `other`, and garbage is `rejected`.
    phone
        .send_plain(Plain::new("made.up", json!({}), BASE_MS))
        .await;
    phone.recv(t::ERROR).await;
    phone.send_raw_up(b"not an envelope".to_vec()).await;
    fx.settle().await;

    let state = fx.gateway.state().await.unwrap();
    let kinds = &state.traffic_breakdown.kinds;
    let msgs = |key: &str| kinds.get(key).map_or(0, |k| k.msgs);
    assert_eq!(msgs("in:hello"), 1, "{kinds:?}");
    assert_eq!(msgs("out:hello.ack"), 1);
    assert_eq!(msgs("out:agents.snapshot"), 1);
    assert_eq!(msgs("out:inbox.snapshot"), 1);
    assert_eq!(msgs("in:ping"), 1);
    assert_eq!(msgs("out:pong"), 1);
    assert_eq!(msgs("in:other"), 1);
    assert_eq!(msgs("out:error"), 1);
    assert_eq!(msgs("in:rejected"), 1);
    assert!(msgs("in:presence") >= 1);
    assert!(
        kinds.keys().all(|key| !key.contains("made.up")),
        "an unregistered type never becomes a key"
    );
    assert!(kinds["out:agents.snapshot"].bytes > 0);
    assert_eq!(state.traffic_breakdown.month.len(), 7);

    // `channel ios status --json` prints the state as it is.
    let json = serde_json::to_value(&state).unwrap();
    assert_eq!(
        json["trafficBreakdown"]["kinds"]["out:hello.ack"]["msgs"],
        1
    );
}

// ---------------------------------------------------------------- the budget

/// What a phone is sent by the Mac when it enters the foreground and then looks at one question for a minute, with
/// seventeen sessions working on the Mac, every one of them taking a tool step every few seconds.
async fn foreground_and_a_minute(configure: impl FnOnce(&mut TestPhone)) -> (usize, usize) {
    let fx = quiet_fixture(working(17)).await;
    let mut phone = fx.pair_phone().await;
    configure(&mut phone);
    let mark = phone.received.len();

    // The order the app uses: connect, hello, and the foreground once the snapshots are in.
    phone.set_presence(true, false).await;
    phone.hello().await;
    phone.recv(t::SLOT_STATE).await;
    tokio::time::sleep(SECOND).await;
    phone.set_presence(true, true).await;
    tokio::time::sleep(SECOND).await;
    phone.clear_queue();
    let entering = bytes_since(&phone, mark);

    let mark = phone.received.len();
    for step in 1..=12u64 {
        for i in 0..17 {
            fx.host.update_agent(&format!("claude:s{i}"), |a| {
                a.turn_steps = step;
                a.current_tool = Some(format!("Shell step {step}"));
                a.last_activity_at += 5_000;
            });
        }
        tokio::time::sleep(5 * SECOND).await;
    }
    phone.clear_queue();
    (entering, bytes_since(&phone, mark))
}

#[tokio::test(start_paused = true)]
async fn entering_the_foreground_and_a_busy_minute_stay_inside_the_budget() {
    let (old_entering, old_minute) = foreground_and_a_minute(|_| {}).await;
    let (new_entering, new_minute) = foreground_and_a_minute(|p| {
        p.lean_agents = true;
        p.compression = true;
    })
    .await;

    // Entering: one round of snapshots instead of two (the second is gone), and the list compressed.
    assert!(
        new_entering * 3 < old_entering,
        "entering the foreground: {new_entering} bytes, was {old_entering}"
    );
    // The minute: not one byte of activity for rows nobody watches.
    assert_eq!(new_minute, 0, "{new_minute} bytes of activity");
    assert!(
        old_minute > 20_000,
        "the baseline is a busy one: {old_minute}"
    );
    eprintln!(
        "entering {old_entering} -> {new_entering} bytes; a busy minute {old_minute} -> {new_minute} bytes"
    );
}
