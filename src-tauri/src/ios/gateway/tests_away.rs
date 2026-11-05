//! Away mode over the wire (docs/specs/special-replies.md D31–D34, N3, N9) against a scripted host.

use std::sync::atomic::Ordering;
use std::time::Duration;

use super::requests::{DeliverRequest, Sink};
use super::testkit::*;
use super::*;
use crate::daemon::away::{AwayOp, Directive, DirectiveKind, Scope, SessionSetting};
use crate::ios::wire::{self, t, AnswerItem};

const SECOND: Duration = Duration::from_secs(1);

/// A phone that said hello, with the first burst (and the one the Mac repeats a second later)
/// out of the way.
async fn live() -> (Fixture, TestPhone) {
    let fx = Fixture::new().await;
    let mut phone = fx.live_phone().await;
    phone.recv(t::SLOT_STATE).await;
    tokio::time::sleep(Duration::from_millis(1_300)).await;
    phone.clear_queue();
    (fx, phone)
}

fn set(on: bool, note: Option<&str>) -> wire::AwaySet {
    wire::AwaySet {
        client_msg_id: "a1".into(),
        on,
        note: note.map(str::to_string),
    }
}

fn session_set(session: &str, setting: &str) -> wire::AwaySessionSet {
    wire::AwaySessionSet {
        client_msg_id: "a2".into(),
        session_id: session.into(),
        setting: setting.into(),
        note: None,
    }
}

// ---------------------------------------------------------------- capability and first state

#[tokio::test(start_paused = true)]
async fn the_mac_announces_away_mode_and_sends_its_state_after_hello() {
    let fx = Fixture::new().await;
    let mut phone = fx.pair_phone().await;
    phone.set_presence(true, true).await;
    let hello = phone.hello().await;
    assert!(hello.caps.away);
    let state: wire::AwayState = phone.recv_body(t::AWAY_STATE).await;
    assert_eq!(state.rev, 0);
    assert!(state.global.is_none());
    assert!(state.sessions.is_empty());
    assert_eq!(state.escalated_waiting, 0);
}

#[tokio::test(start_paused = true)]
async fn a_host_without_away_mode_sends_no_state_and_refuses_the_commands() {
    let fx = Fixture::new().await;
    fx.host.away_supported.store(false, Ordering::SeqCst);
    let mut phone = fx.live_phone().await;
    phone.recv(t::SLOT_STATE).await;
    tokio::time::sleep(Duration::from_millis(1_300)).await;
    assert!(
        !phone.queued_types().contains(&t::AWAY_STATE.to_string()),
        "{:?}",
        phone.queued_types()
    );
    phone.send(&set(true, None)).await;
    let error = phone.recv(t::ERROR).await;
    assert_eq!(error.b["code"], "unsupported_type");
    assert!(fx.host.away_ops.lock().unwrap().is_empty());
}

#[tokio::test(start_paused = true)]
async fn resync_repeats_the_state_of_the_away_stream() {
    let (fx, mut phone) = live().await;
    fx.host
        .away_engine
        .apply(&AwayOp::Start { note: None }, BASE_MS as u64);
    phone
        .send(&wire::Resync {
            stream: "away".into(),
        })
        .await;
    let state: wire::AwayState = phone.recv_body(t::AWAY_STATE).await;
    assert!(state.global.is_some());
}

// ---------------------------------------------------------------- commands

#[tokio::test(start_paused = true)]
async fn a_phone_turns_away_mode_on_with_a_note_and_off_again() {
    let (fx, mut phone) = live().await;
    phone.send(&set(true, Some("keep going, no prod"))).await;
    let on: wire::AwayState = phone.recv_body(t::AWAY_STATE).await;
    let global = on.global.expect("away mode is on");
    assert_eq!(global.note.as_deref(), Some("keep going, no prod"));
    assert_eq!(on.rev, 1, "a change moves the revision on");
    assert_eq!(
        fx.host.away_ops.lock().unwrap().as_slice(),
        [AwayOp::Start {
            note: Some("keep going, no prod".into())
        }]
    );

    phone.send(&set(false, None)).await;
    let off: wire::AwayState = phone.recv_body(t::AWAY_STATE).await;
    assert!(off.global.is_none());
    assert_eq!(off.rev, 2);
}

#[tokio::test(start_paused = true)]
async fn a_session_setting_names_the_session_by_the_agents_own_id() {
    let (fx, mut phone) = live().await;
    phone.send(&session_set("abc-123", "mute")).await;
    let state: wire::AwayState = phone.recv_body(t::AWAY_STATE).await;
    assert_eq!(state.sessions.len(), 1);
    assert_eq!(state.sessions[0].session_id, "abc-123");
    assert_eq!(state.sessions[0].mode, "mute");
    assert_eq!(
        fx.host.away_ops.lock().unwrap().as_slice(),
        [AwayOp::Session {
            session_key: "sid:abc-123".into(),
            setting: SessionSetting::Mute,
            note: None
        }]
    );

    phone.send(&session_set("abc-123", "follow")).await;
    let cleared: wire::AwayState = phone.recv_body(t::AWAY_STATE).await;
    assert!(
        cleared.sessions.is_empty(),
        "following the global setting leaves no entry"
    );
}

#[tokio::test(start_paused = true)]
async fn a_setting_the_mac_does_not_know_is_refused() {
    let (fx, mut phone) = live().await;
    phone.send(&session_set("abc", "pause")).await;
    let error = phone.recv(t::ERROR).await;
    assert_eq!(error.b["code"], "bad_request");
    phone.send(&session_set("  ", "away")).await;
    let error = phone.recv(t::ERROR).await;
    assert_eq!(error.b["code"], "bad_request");
    assert!(fx.host.away_ops.lock().unwrap().is_empty());
}

// ---------------------------------------------------------------- changes from elsewhere

#[tokio::test(start_paused = true)]
async fn a_change_made_on_the_mac_reaches_the_live_phone_within_a_tick() {
    let (fx, mut phone) = live().await;
    fx.host.away_engine.apply(
        &AwayOp::Start {
            note: Some("from the tray".into()),
        },
        BASE_MS as u64,
    );
    tokio::time::sleep(2 * SECOND).await;
    let state: wire::AwayState = phone.recv_body(t::AWAY_STATE).await;
    assert_eq!(
        state.global.and_then(|g| g.note).as_deref(),
        Some("from the tray")
    );
    // Nothing changed since: nothing more is sent.
    phone.clear_queue();
    tokio::time::sleep(3 * SECOND).await;
    assert!(!phone.gets(t::AWAY_STATE, SECOND).await);
}

#[tokio::test(start_paused = true)]
async fn a_waiting_escalation_is_part_of_the_state() {
    let (fx, mut phone) = live().await;
    fx.host.escalated_waiting.store(2, Ordering::SeqCst);
    tokio::time::sleep(2 * SECOND).await;
    let state: wire::AwayState = phone.recv_body(t::AWAY_STATE).await;
    assert_eq!(state.escalated_waiting, 2);
}

#[tokio::test(start_paused = true)]
async fn sessions_without_an_agent_session_id_are_not_listed() {
    let (fx, mut phone) = live().await;
    fx.host.away_engine.apply(
        &AwayOp::Session {
            session_key: "mcp:instance-1".into(),
            setting: SessionSetting::Mute,
            note: None,
        },
        BASE_MS as u64,
    );
    fx.host.away_engine.apply(
        &AwayOp::Session {
            session_key: "sid:s1".into(),
            setting: SessionSetting::Away,
            note: None,
        },
        BASE_MS as u64,
    );
    tokio::time::sleep(2 * SECOND).await;
    let state: wire::AwayState = phone.recv_body(t::AWAY_STATE).await;
    assert_eq!(
        state
            .sessions
            .iter()
            .map(|s| (s.session_id.as_str(), s.mode.as_str()))
            .collect::<Vec<_>>(),
        [("s1", "away")]
    );
}

// ---------------------------------------------------------------- the directive of an answer

fn deliver_ask(fx: &Fixture, id: &str) -> Arc<FakeAsk> {
    let (ask, view) = sample_ask(id);
    let sink = FakeAsk::new();
    fx.gateway.deliver(DeliverRequest {
        view,
        sink: Sink::Ask(sink.clone()),
        ask: Some(ask),
        deadline_ms: None,
        assets: Vec::new(),
    });
    sink
}

fn answer_with(directive: Option<wire::AnswerDirective>, action: &str) -> wire::AnswerSubmit {
    wire::AnswerSubmit {
        client_msg_id: "c1".into(),
        request_id: "req1".into(),
        action: action.into(),
        answers: vec![
            AnswerItem {
                q: 0,
                sel: vec![0],
                text: None,
                images: Vec::new(),
                files: Vec::new(),
            },
            AnswerItem {
                q: 1,
                sel: vec![1],
                text: None,
                images: Vec::new(),
                files: Vec::new(),
            },
        ],
        todo_ids: Vec::new(),
        directive,
    }
}

fn directive(kind: &str, scope: &str, note: Option<&str>) -> wire::AnswerDirective {
    wire::AnswerDirective {
        kind: kind.into(),
        scope: scope.into(),
        note: note.map(str::to_string),
    }
}

#[tokio::test(start_paused = true)]
async fn an_answer_hands_its_directive_to_the_request_before_it_is_submitted() {
    let (fx, mut phone) = live().await;
    let sink = deliver_ask(&fx, "req1");
    phone.recv(t::REQUEST_NEW).await;
    phone
        .send(&answer_with(
            Some(directive("away", "session", Some("be bold"))),
            "send",
        ))
        .await;
    let ack: wire::AnswerAck = phone.recv_body(t::ANSWER_ACK).await;
    assert_eq!(ack.status, "accepted");
    assert_eq!(
        sink.directives.lock().unwrap().as_slice(),
        [Some(Directive {
            kind: DirectiveKind::Away,
            scope: Scope::Session,
            note: Some("be bold".into())
        })]
    );
    assert_eq!(sink.results.lock().unwrap().len(), 1);
}

#[tokio::test(start_paused = true)]
async fn a_directive_is_dropped_again_when_another_surface_won() {
    let (fx, mut phone) = live().await;
    let sink = deliver_ask(&fx, "req1");
    sink.win.store(false, Ordering::SeqCst);
    phone.recv(t::REQUEST_NEW).await;
    phone
        .send(&answer_with(
            Some(directive("mute", "session", None)),
            "send",
        ))
        .await;
    let ack: wire::AnswerAck = phone.recv_body(t::ANSWER_ACK).await;
    assert_eq!(ack.status, "lost");
    let stashed = sink.directives.lock().unwrap().clone();
    assert_eq!(
        stashed.len(),
        2,
        "stashed for the attempt, then taken back: {stashed:?}"
    );
    assert!(stashed[0].is_some());
    assert!(stashed[1].is_none());
}

#[tokio::test(start_paused = true)]
async fn an_answer_without_a_directive_never_touches_the_slot() {
    let (fx, mut phone) = live().await;
    let sink = deliver_ask(&fx, "req1");
    phone.recv(t::REQUEST_NEW).await;
    phone.send(&answer_with(None, "send")).await;
    let ack: wire::AnswerAck = phone.recv_body(t::ANSWER_ACK).await;
    assert_eq!(ack.status, "accepted");
    assert!(sink.directives.lock().unwrap().is_empty());
}

#[tokio::test(start_paused = true)]
async fn cancelling_carries_no_directive() {
    let (fx, mut phone) = live().await;
    let sink = deliver_ask(&fx, "req1");
    phone.recv(t::REQUEST_NEW).await;
    phone
        .send(&answer_with(
            Some(directive("away", "global", None)),
            "cancel",
        ))
        .await;
    let ack: wire::AnswerAck = phone.recv_body(t::ANSWER_ACK).await;
    assert_eq!(ack.status, "accepted");
    assert!(sink.directives.lock().unwrap().is_empty());
}

#[tokio::test(start_paused = true)]
async fn a_directive_the_mac_cannot_read_refuses_the_answer() {
    let (fx, mut phone) = live().await;
    let sink = deliver_ask(&fx, "req1");
    phone.recv(t::REQUEST_NEW).await;
    phone
        .send(&answer_with(
            Some(directive("shout", "global", None)),
            "send",
        ))
        .await;
    let ack: wire::AnswerAck = phone.recv_body(t::ANSWER_ACK).await;
    assert_eq!(ack.status, "invalid");
    assert!(sink.results.lock().unwrap().is_empty());

    // The same answer with a readable directive still goes through.
    phone
        .send(&wire::AnswerSubmit {
            client_msg_id: "c2".into(),
            ..answer_with(Some(directive("mute", "global", None)), "send")
        })
        .await;
    let ack: wire::AnswerAck = phone.recv_body(t::ANSWER_ACK).await;
    assert_eq!(ack.status, "accepted");
}

#[tokio::test(start_paused = true)]
async fn a_question_that_was_escalated_says_so_to_the_phone() {
    let (fx, mut phone) = live().await;
    let (ask, mut view) = sample_ask("req1");
    view.escalated = true;
    fx.gateway.deliver(DeliverRequest {
        view,
        sink: Sink::Ask(FakeAsk::new()),
        ask: Some(ask),
        deadline_ms: None,
        assets: Vec::new(),
    });
    let new: wire::RequestNew = phone.recv_body(t::REQUEST_NEW).await;
    assert!(new.request.escalated);
}
