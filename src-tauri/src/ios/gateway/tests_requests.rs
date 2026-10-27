use std::time::Duration;

use super::requests::{DeliverRequest, Resolution, Sink};
use super::testkit::*;
use super::*;
use crate::ios::envelope::{self, Scope};
use crate::ios::wire::{self, t, AnswerItem, PushPlain};

const SECOND: Duration = Duration::from_secs(1);

fn answer(client_msg_id: &str, request_id: &str, answers: Vec<AnswerItem>) -> wire::AnswerSubmit {
    wire::AnswerSubmit {
        client_msg_id: client_msg_id.into(),
        request_id: request_id.into(),
        action: "send".into(),
        answers,
    }
}

fn item(q: usize, sel: &[usize], text: Option<&str>) -> AnswerItem {
    AnswerItem {
        q,
        sel: sel.to_vec(),
        text: text.map(str::to_string),
        images: Vec::new(),
        files: Vec::new(),
    }
}

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

fn deliver_confirm(fx: &Fixture, id: &str, ttl_ms: i64) -> Arc<FakeConfirm> {
    let sink = FakeConfirm::new();
    fx.gateway.deliver(DeliverRequest {
        view: sample_confirm(id, BASE_MS + ttl_ms),
        sink: Sink::Confirm(sink.clone()),
        ask: None,
        deadline_ms: Some(BASE_MS + ttl_ms),
        assets: Vec::new(),
    });
    sink
}

// ---------------------------------------------------------------- delivery

#[tokio::test(start_paused = true)]
async fn a_question_reaches_a_live_phone_and_the_answer_comes_back() {
    let fx = Fixture::new().await;
    let mut phone = fx.live_phone().await;
    phone.clear_queue();

    let sink = deliver_ask(&fx, "req1");
    let new: wire::RequestNew = phone.recv_body(t::REQUEST_NEW).await;
    assert!(new.full);
    assert_eq!(new.request.id, "req1");
    assert_eq!(new.request.content_hash.len(), 16);
    assert_eq!(new.rev, 1);
    phone
        .send(&wire::RequestAck {
            request_id: "req1".into(),
        })
        .await;

    phone
        .send(&answer(
            "c1",
            "req1",
            vec![item(0, &[1], Some("  because  ")), item(1, &[1, 0], None)],
        ))
        .await;
    let ack: wire::AnswerAck = phone.recv_body(t::ANSWER_ACK).await;
    assert_eq!(
        (ack.status.as_str(), ack.client_msg_id.as_str()),
        ("accepted", "c1")
    );

    {
        let results = sink.results.lock().unwrap();
        assert_eq!(results.len(), 1);
        let result = &results[0];
        assert_eq!(result.source_channel_id, "ios");
        assert!(matches!(result.action, crate::models::ChannelAction::Send));
        assert_eq!(result.answers[0].selected_options, ["production"]);
        assert_eq!(result.answers[0].user_input.as_deref(), Some("because"));
        assert_eq!(result.answers[1].selected_options, ["no", "yes"]);
        assert_eq!(result.answers[1].user_input, None);
    }

    let resolved: wire::RequestResolved = phone.recv_body(t::REQUEST_RESOLVED).await;
    assert_eq!(
        (resolved.request_id.as_str(), resolved.how.as_str()),
        ("req1", "answered")
    );
    assert_eq!(fx.gateway.state().await.unwrap().pending_requests, 0);
    assert_eq!(
        fx.pusher.count(),
        0,
        "an acknowledged request needs no push"
    );
}

#[tokio::test(start_paused = true)]
async fn cancel_is_forwarded_as_a_cancel() {
    let fx = Fixture::new().await;
    let mut phone = fx.live_phone().await;
    let sink = deliver_ask(&fx, "req1");
    phone.recv(t::REQUEST_NEW).await;

    phone
        .send(&wire::AnswerSubmit {
            client_msg_id: "c1".into(),
            request_id: "req1".into(),
            action: "cancel".into(),
            answers: Vec::new(),
        })
        .await;
    let ack: wire::AnswerAck = phone.recv_body(t::ANSWER_ACK).await;
    assert_eq!(ack.status, "accepted");
    let results = sink.results.lock().unwrap();
    assert!(matches!(
        results[0].action,
        crate::models::ChannelAction::Cancel
    ));
    assert!(results[0].answers.is_empty());
}

#[tokio::test(start_paused = true)]
async fn losing_the_race_reports_who_won() {
    let fx = Fixture::new().await;
    let mut phone = fx.live_phone().await;
    let sink = deliver_ask(&fx, "req1");
    sink.win.store(false, std::sync::atomic::Ordering::SeqCst);
    phone.recv(t::REQUEST_NEW).await;

    phone
        .send(&answer("c1", "req1", vec![item(0, &[0], None)]))
        .await;
    let ack: wire::AnswerAck = phone.recv_body(t::ANSWER_ACK).await;
    assert_eq!(ack.status, "lost");
    assert_eq!(ack.reason.as_deref(), Some("popup"));
    // The request stays pending until the daemon resolves it.
    assert_eq!(fx.gateway.state().await.unwrap().pending_requests, 1);
}

#[tokio::test(start_paused = true)]
async fn the_phone_that_lost_to_another_phone_is_told_who_answered() {
    let fx = Fixture::new().await;
    let mut first = fx.live_phone().await;
    let mut second = fx.live_phone().await;
    deliver_ask(&fx, "req1");
    first.recv(t::REQUEST_NEW).await;
    second.recv(t::REQUEST_NEW).await;

    first
        .send(&answer("c1", "req1", vec![item(0, &[0], None)]))
        .await;
    let ack: wire::AnswerAck = first.recv_body(t::ANSWER_ACK).await;
    assert_eq!(ack.status, "accepted");

    let winner: wire::RequestResolved = first.recv_body(t::REQUEST_RESOLVED).await;
    assert_eq!(winner.how, "answered");
    assert_eq!(winner.by, None);

    let loser: wire::RequestResolved = second.recv_body(t::REQUEST_RESOLVED).await;
    assert_eq!(loser.how, "answeredElsewhere");
    let winner_name = fx
        .gateway
        .state()
        .await
        .unwrap()
        .phones
        .iter()
        .find(|p| p.phone_id == first.phone_id)
        .map(|p| p.name.clone())
        .unwrap();
    assert_eq!(loser.by.as_deref(), Some(winner_name.as_str()));

    // Asking again later gives each phone the same account of what happened.
    second
        .send(&wire::RequestGet {
            request_id: "req1".into(),
        })
        .await;
    let again: wire::RequestResolved = second.recv_body(t::REQUEST_RESOLVED).await;
    assert_eq!(again.how, "answeredElsewhere");
    first
        .send(&wire::RequestGet {
            request_id: "req1".into(),
        })
        .await;
    let own: wire::RequestResolved = first.recv_body(t::REQUEST_RESOLVED).await;
    assert_eq!(own.how, "answered");
}

#[tokio::test(start_paused = true)]
async fn invalid_answers_are_rejected_without_touching_the_request() {
    let fx = Fixture::new().await;
    let mut phone = fx.live_phone().await;
    let sink = deliver_ask(&fx, "req1");
    phone.recv(t::REQUEST_NEW).await;

    let bad = [
        vec![item(5, &[], None)],                               // no such question
        vec![item(0, &[9], None)],                              // no such option
        vec![item(0, &[0, 0], None)],                           // duplicate option
        vec![item(0, &[], Some("a")), item(0, &[], Some("b"))], // question twice
    ];
    for (n, answers) in bad.into_iter().enumerate() {
        phone
            .send(&answer(&format!("bad{n}"), "req1", answers))
            .await;
        let ack: wire::AnswerAck = phone.recv_body(t::ANSWER_ACK).await;
        assert_eq!(ack.status, "invalid", "case {n}");
        assert!(ack.reason.is_some());
    }
    assert!(sink.results.lock().unwrap().is_empty());
    assert_eq!(fx.gateway.state().await.unwrap().pending_requests, 1);
}

#[tokio::test(start_paused = true)]
async fn answers_referencing_unknown_uploads_are_invalid() {
    let fx = Fixture::new().await;
    let mut phone = fx.live_phone().await;
    deliver_ask(&fx, "req1");
    phone.recv(t::REQUEST_NEW).await;
    let mut with_image = item(0, &[], None);
    with_image.images = vec!["u1".into()];
    phone.send(&answer("c1", "req1", vec![with_image])).await;
    let ack: wire::AnswerAck = phone.recv_body(t::ANSWER_ACK).await;
    assert_eq!(ack.status, "invalid");
}

#[tokio::test(start_paused = true)]
async fn a_repeated_client_message_id_replays_the_first_ack() {
    let fx = Fixture::new().await;
    let mut phone = fx.live_phone().await;
    let sink = deliver_ask(&fx, "req1");
    phone.recv(t::REQUEST_NEW).await;

    let submit = answer("c1", "req1", vec![item(0, &[0], None)]);
    phone.send(&submit).await;
    let first: wire::AnswerAck = phone.recv_body(t::ANSWER_ACK).await;
    assert_eq!(first.status, "accepted");

    // The phone never saw the ack and retries (new message id, same client message id).
    phone.send(&submit).await;
    let second: wire::AnswerAck = phone.recv_body(t::ANSWER_ACK).await;
    assert_eq!(second, first);
    assert_eq!(sink.results.lock().unwrap().len(), 1, "submitted once");

    // Same id with different content is a conflict.
    let other = answer("c1", "req1", vec![item(0, &[1], None)]);
    phone.send(&other).await;
    let conflict: wire::AnswerAck = phone.recv_body(t::ANSWER_ACK).await;
    assert_eq!(conflict.status, "invalid");
    assert_eq!(conflict.reason.as_deref(), Some("conflict"));
}

#[tokio::test(start_paused = true)]
async fn a_retry_after_a_gateway_restart_still_gets_the_original_ack() {
    let fx = Fixture::new().await;
    let mut phone = fx.live_phone().await;
    let sink = deliver_ask(&fx, "req1");
    phone.recv(t::REQUEST_NEW).await;
    let submit = answer("c1", "req1", vec![item(0, &[0], None)]);
    phone.send(&submit).await;
    let first: wire::AnswerAck = phone.recv_body(t::ANSWER_ACK).await;
    assert_eq!(first.status, "accepted");

    let fx = fx.restart().await;
    phone.set_presence(true, true).await;
    phone.hello().await;
    phone.clear_queue();
    phone.send(&submit).await;
    let replay: wire::AnswerAck = phone.recv_body(t::ANSWER_ACK).await;
    assert_eq!(replay, first);
    assert_eq!(sink.results.lock().unwrap().len(), 1);
    assert_eq!(fx.gateway.state().await.unwrap().pending_requests, 0);
}

#[tokio::test(start_paused = true)]
async fn answers_to_finished_requests_say_why_they_came_too_late() {
    let fx = Fixture::new().await;
    let mut phone = fx.live_phone().await;
    deliver_ask(&fx, "gone");
    deliver_ask(&fx, "late");
    phone.recv(t::REQUEST_NEW).await;
    phone.recv(t::REQUEST_NEW).await;
    fx.gateway
        .resolve_and_wait("gone", Resolution::AnsweredElsewhere("Feishu".into()))
        .await;
    let resolved: wire::RequestResolved = phone.recv_body(t::REQUEST_RESOLVED).await;
    assert_eq!(resolved.how, "answeredElsewhere");
    assert_eq!(resolved.by.as_deref(), Some("Feishu"));

    phone
        .send(&answer("c1", "gone", vec![item(0, &[0], None)]))
        .await;
    let ack: wire::AnswerAck = phone.recv_body(t::ANSWER_ACK).await;
    assert_eq!(ack.status, "lost");
    assert_eq!(ack.reason.as_deref(), Some("Feishu"));

    fx.gateway
        .resolve_and_wait("late", Resolution::Expired)
        .await;
    phone
        .send(&answer("c2", "late", vec![item(0, &[0], None)]))
        .await;
    let ack: wire::AnswerAck = phone.recv_body(t::ANSWER_ACK).await;
    assert_eq!(ack.status, "expired");

    phone
        .send(&answer("c3", "never-heard-of", vec![item(0, &[0], None)]))
        .await;
    let ack: wire::AnswerAck = phone.recv_body(t::ANSWER_ACK).await;
    assert_eq!(ack.status, "expired");
}

#[tokio::test(start_paused = true)]
async fn hello_lists_pending_requests_and_request_get_returns_the_full_content() {
    let fx = Fixture::new().await;
    let mut phone = fx.pair_phone().await;
    deliver_ask(&fx, "req1");
    phone.set_presence(true, true).await;
    phone.hello().await;

    let inbox: wire::InboxSnapshot = phone.recv_body(t::INBOX_SNAPSHOT).await;
    assert_eq!(inbox.items.len(), 1);
    let summary = &inbox.items[0];
    assert_eq!(summary.id, "req1");
    assert_eq!(summary.kind, "ask");
    assert_eq!(summary.preview, "Pick a deployment strategy");
    assert_eq!(summary.agent.label, "Codex");

    phone
        .send(&wire::RequestGet {
            request_id: "req1".into(),
        })
        .await;
    let full: wire::RequestFull = phone.recv_body(t::REQUEST_FULL).await;
    assert_eq!(full.request.content_hash, summary.content_hash);
    assert_eq!(full.request.ask.unwrap().questions.len(), 2);

    phone
        .send(&wire::RequestGet {
            request_id: "unknown".into(),
        })
        .await;
    let error: wire::ErrorBody = phone.recv_body(t::ERROR).await;
    assert_eq!(error.code, "not_found");
}

#[tokio::test(start_paused = true)]
async fn large_requests_arrive_as_a_summary_first_and_huge_ones_are_truncated() {
    let fx = Fixture::new().await;
    let mut phone = fx.live_phone().await;
    phone.clear_queue();

    let (ask, mut view) = sample_ask("big");
    view.ask.as_mut().unwrap().message.text = "x".repeat(200 * 1024);
    fx.gateway.deliver(DeliverRequest {
        view,
        sink: Sink::Ask(FakeAsk::new()),
        ask: Some(ask),
        deadline_ms: None,
        assets: Vec::new(),
    });
    let new: wire::RequestNew = phone.recv_body(t::REQUEST_NEW).await;
    assert!(!new.full, "too large to send inline");
    assert!(
        new.request
            .ask
            .as_ref()
            .unwrap()
            .message
            .text
            .chars()
            .count()
            <= 1_001
    );
    phone
        .send(&wire::RequestGet {
            request_id: "big".into(),
        })
        .await;
    let full: wire::RequestFull = phone.recv_body(t::REQUEST_FULL).await;
    assert_eq!(full.request.ask.unwrap().message.text.len(), 200 * 1024);
    assert!(!full.request.truncated);

    let (ask, mut view) = sample_ask("huge");
    view.ask.as_mut().unwrap().message.text = "y".repeat(2 * 1024 * 1024);
    fx.gateway.deliver(DeliverRequest {
        view,
        sink: Sink::Ask(FakeAsk::new()),
        ask: Some(ask),
        deadline_ms: None,
        assets: Vec::new(),
    });
    phone.recv(t::REQUEST_NEW).await;
    phone
        .send(&wire::RequestGet {
            request_id: "huge".into(),
        })
        .await;
    let full: wire::RequestFull = phone.recv_body(t::REQUEST_FULL).await;
    assert!(full.request.truncated);
    assert!(full.request.ask.unwrap().message.text.len() < 500 * 1024);
}

// ---------------------------------------------------------------- push

#[tokio::test(start_paused = true)]
async fn a_phone_in_the_background_is_reached_by_push_only() {
    let fx = Fixture::new().await;
    let mut phone = fx.pair_phone().await;
    phone.set_presence(true, false).await;
    phone.clear_queue();

    deliver_ask(&fx, "req1");
    fx.settle().await;
    assert_eq!(fx.pusher.count(), 1);
    assert!(
        !phone.gets(t::REQUEST_NEW, SECOND).await,
        "no MQTT message for a background app"
    );

    let push = fx.pusher.last().unwrap();
    assert_eq!(push.handle, "handle-1");
    assert_eq!(push.kind, "alert");
    assert_eq!(push.loc_key, "PUSH_ASK");
    assert_eq!(push.collapse_id, "rq-req1");
    assert_eq!(push.thread_id, fx.mac_id);
    // The phone can read it with the pairing key; the relay can only see ciphertext.
    let keys = phone.keys.clone().unwrap();
    let plain = envelope::open(&keys.m2p, Scope::Push, &push.ah).unwrap();
    let plain: PushPlain = serde_json::from_slice(&plain).unwrap();
    assert_eq!(plain.k, "ask");
    assert_eq!(plain.rid.as_deref(), Some("req1"));
    assert_eq!(plain.mac, "Test Mac");
    assert_eq!(plain.agent, "Codex");
    assert_eq!(plain.project, "proj");
    assert_eq!(plain.text, "Pick a deployment strategy");
    let wire_json = serde_json::to_string(&push).unwrap();
    assert!(!wire_json.contains("deployment"));
}

#[tokio::test(start_paused = true)]
async fn an_offline_phone_is_pushed_and_finds_the_request_when_it_returns() {
    let fx = Fixture::new().await;
    let mut phone = fx.pair_phone().await;
    deliver_ask(&fx, "req1");
    fx.settle().await;
    assert_eq!(fx.pusher.count(), 1);

    phone.set_presence(true, true).await;
    phone.hello().await;
    let inbox: wire::InboxSnapshot = phone.recv_body(t::INBOX_SNAPSHOT).await;
    assert_eq!(inbox.items[0].id, "req1");
}

#[tokio::test(start_paused = true)]
async fn a_request_that_came_between_hello_and_the_foreground_presence_is_caught_up() {
    // The app says "foreground" only after its first sync. A request created before that was not
    // sent live (the phone is not live yet) and the hello snapshots predate it.
    let fx = Fixture::new().await;
    let mut phone = fx.pair_phone_without_push().await;
    phone.set_presence(true, false).await;
    phone.hello().await;
    phone.clear_queue();

    deliver_ask(&fx, "req1");
    assert!(
        !phone.gets(t::REQUEST_NEW, SECOND).await,
        "the phone is not live yet"
    );

    phone.set_presence(true, true).await;
    let inbox: wire::InboxSnapshot = phone.recv_body(t::INBOX_SNAPSHOT).await;
    assert_eq!(inbox.items.len(), 1);
    assert_eq!(inbox.items[0].id, "req1");

    // From then on the request stream is live: no second catch-up, deltas as usual.
    phone.clear_queue();
    deliver_ask(&fx, "req2");
    let new: wire::RequestNew = phone.recv_body(t::REQUEST_NEW).await;
    assert_eq!(new.request.id, "req2");
}

#[tokio::test(start_paused = true)]
async fn a_phone_that_says_foreground_again_without_a_change_is_not_sent_snapshots() {
    let fx = Fixture::new().await;
    let mut phone = fx.live_phone().await;
    // Let the one-off resync the Mac does a second after it connected come and go.
    tokio::time::sleep(Duration::from_millis(1_300)).await;
    phone.clear_queue();
    // The retained presence arrives again (a Mac reconnecting to the broker sees it): the phone
    // was live all along, so nothing is repeated.
    phone.set_presence(true, true).await;
    assert!(!phone.gets(t::INBOX_SNAPSHOT, SECOND).await);
}

#[tokio::test(start_paused = true)]
async fn the_push_fallback_fires_when_the_foreground_phone_does_not_acknowledge() {
    let fx = Fixture::new().await;
    let mut phone = fx.live_phone().await;
    deliver_ask(&fx, "req1");
    phone.recv(t::REQUEST_NEW).await;

    tokio::time::sleep(Duration::from_millis(2_500)).await;
    assert_eq!(fx.pusher.count(), 0);
    tokio::time::sleep(Duration::from_millis(1_500)).await;
    assert_eq!(
        fx.pusher.count(),
        1,
        "pushed once after three silent seconds"
    );
    tokio::time::sleep(Duration::from_secs(10)).await;
    assert_eq!(fx.pusher.count(), 1, "and only once");
}

#[tokio::test(start_paused = true)]
async fn an_acknowledgement_cancels_the_push_fallback() {
    let fx = Fixture::new().await;
    let mut phone = fx.live_phone().await;
    deliver_ask(&fx, "req1");
    phone.recv(t::REQUEST_NEW).await;
    phone
        .send(&wire::RequestAck {
            request_id: "req1".into(),
        })
        .await;
    tokio::time::sleep(Duration::from_secs(10)).await;
    assert_eq!(fx.pusher.count(), 0);
}

#[tokio::test(start_paused = true)]
async fn resolving_a_pushed_request_replaces_its_notification() {
    let fx = Fixture::new().await;
    let phone = fx.pair_phone().await;
    phone.set_presence(true, false).await;
    deliver_ask(&fx, "req1");
    fx.settle().await;
    assert_eq!(fx.pusher.count(), 1);

    fx.gateway
        .resolve_and_wait("req1", Resolution::AnsweredElsewhere("Popup".into()))
        .await;
    fx.settle().await;
    assert_eq!(fx.pusher.count(), 2);
    let push = fx.pusher.last().unwrap();
    assert_eq!(push.kind, "replace");
    assert_eq!(push.loc_key, "PUSH_RESOLVED");
    assert_eq!(
        push.collapse_id, "rq-req1",
        "same collapse id replaces the alert"
    );
    let keys = phone.keys.clone().unwrap();
    let plain: PushPlain =
        serde_json::from_slice(&envelope::open(&keys.m2p, Scope::Push, &push.ah).unwrap()).unwrap();
    assert_eq!(plain.k, "resolved");
}

#[tokio::test(start_paused = true)]
async fn a_dead_push_registration_is_marked_and_the_phone_is_told() {
    let fx = Fixture::new().await;
    let mut phone = fx.pair_phone().await;
    phone.set_presence(true, false).await;
    *fx.pusher.outcome.lock().unwrap() = Some(crate::ios::push::PushOutcome::Gone);
    deliver_ask(&fx, "req1");
    fx.settle().await;

    let state = fx.gateway.state().await.unwrap();
    assert_eq!(state.phones[0].push.as_deref(), Some("gone"));
    let notice: wire::PushState = phone.recv_body(t::PUSH_STATE).await;
    assert_eq!(notice.status, "gone");

    // No further pushes go to a dead registration.
    deliver_ask(&fx, "req2");
    fx.settle().await;
    assert_eq!(fx.pusher.count(), 1);
}

// ---------------------------------------------------------------- push timing
//
// A question the phone is not looking at waits in its inbox; the push follows once the Mac has
// been idle for `alert_after_secs` (docs/specs/ios-alert-timing.md).

const MINUTE: Duration = Duration::from_secs(60);

#[tokio::test(start_paused = true)]
async fn a_background_phone_is_not_pushed_while_the_mac_is_in_use() {
    let fx = Fixture::new().await;
    fx.host.set_alert_after(900);
    let mut phone = fx.pair_phone().await;
    phone.set_presence(true, false).await;
    fx.host.touch_input();
    deliver_ask(&fx, "req1");
    tokio::time::sleep(10 * MINUTE).await;
    assert_eq!(
        fx.pusher.count(),
        0,
        "the Mac has been idle for ten minutes only"
    );

    // Nothing is lost: the question is in the inbox the moment the phone looks.
    phone.clear_queue();
    phone.set_presence(true, true).await;
    phone.hello().await;
    let inbox: wire::InboxSnapshot = phone.recv_body(t::INBOX_SNAPSHOT).await;
    assert_eq!(inbox.items.len(), 1);
    assert_eq!(inbox.items[0].id, "req1");
    assert_eq!(fx.pusher.count(), 0);
}

#[tokio::test(start_paused = true)]
async fn the_push_goes_out_once_the_mac_has_been_idle_long_enough_and_never_repeats() {
    let fx = Fixture::new().await;
    fx.host.set_alert_after(900);
    let phone = fx.pair_phone().await;
    phone.set_presence(true, false).await;
    fx.host.touch_input();
    deliver_ask(&fx, "req1");

    tokio::time::sleep(14 * MINUTE).await;
    assert_eq!(fx.pusher.count(), 0);
    tokio::time::sleep(2 * MINUTE).await;
    assert_eq!(fx.pusher.count(), 1, "idle for sixteen minutes by now");
    let push = fx.pusher.last().unwrap();
    assert_eq!(
        (
            push.kind.as_str(),
            push.loc_key.as_str(),
            push.collapse_id.as_str()
        ),
        ("alert", "PUSH_ASK", "rq-req1")
    );

    tokio::time::sleep(60 * MINUTE).await;
    assert_eq!(fx.pusher.count(), 1, "no reminders");
}

#[tokio::test(start_paused = true)]
async fn a_question_that_arrives_after_a_long_idle_is_pushed_at_once() {
    let fx = Fixture::new().await;
    fx.host.set_alert_after(900);
    let phone = fx.pair_phone().await;
    phone.set_presence(true, false).await;
    fx.host.idle_for(20 * 60);
    deliver_ask(&fx, "req1");
    fx.settle().await;
    assert_eq!(fx.pusher.count(), 1);
}

#[tokio::test(start_paused = true)]
async fn input_on_the_mac_restarts_the_wait() {
    let fx = Fixture::new().await;
    fx.host.set_alert_after(600);
    let phone = fx.pair_phone().await;
    phone.set_presence(true, false).await;
    fx.host.touch_input();
    deliver_ask(&fx, "req1");

    tokio::time::sleep(9 * MINUTE).await;
    fx.host.touch_input(); // back at the keyboard for a moment
    tokio::time::sleep(9 * MINUTE).await;
    assert_eq!(fx.pusher.count(), 0, "nine minutes of idle, not eighteen");
    tokio::time::sleep(2 * MINUTE).await;
    assert_eq!(fx.pusher.count(), 1);
}

#[tokio::test(start_paused = true)]
async fn a_zero_wait_pushes_at_once() {
    let fx = Fixture::new().await;
    fx.host.set_alert_after(0);
    let phone = fx.pair_phone().await;
    phone.set_presence(true, false).await;
    fx.host.touch_input();
    deliver_ask(&fx, "req1");
    fx.settle().await;
    assert_eq!(
        fx.pusher.count(),
        1,
        "the Mac is in use, but nobody asked to wait"
    );
}

#[tokio::test(start_paused = true)]
async fn an_unknown_idle_time_pushes_at_once() {
    let fx = Fixture::new().await;
    fx.host.set_alert_after(900); // the test host knows no idle time until told
    let phone = fx.pair_phone().await;
    phone.set_presence(true, false).await;
    deliver_ask(&fx, "req1");
    fx.settle().await;
    assert_eq!(
        fx.pusher.count(),
        1,
        "a Mac that cannot tell still notifies"
    );
}

#[tokio::test(start_paused = true)]
async fn a_phone_that_looks_before_the_wait_is_over_is_never_pushed() {
    let fx = Fixture::new().await;
    fx.host.set_alert_after(900);
    let mut phone = fx.pair_phone().await;
    phone.set_presence(true, false).await;
    fx.host.touch_input();
    deliver_ask(&fx, "req1");
    tokio::time::sleep(5 * MINUTE).await;

    phone.set_presence(true, true).await;
    phone.hello().await;
    phone.recv(t::INBOX_SNAPSHOT).await;
    tokio::time::sleep(30 * MINUTE).await;
    assert_eq!(
        fx.pusher.count(),
        0,
        "a phone that is looking needs no push, however long the Mac idles"
    );
}

#[tokio::test(start_paused = true)]
async fn a_live_phone_is_never_pushed_however_long_the_mac_idles() {
    let fx = Fixture::new().await;
    fx.host.set_alert_after(900);
    fx.host.idle_for(2 * 3600);
    let mut phone = fx.live_phone().await;
    deliver_ask(&fx, "req1");
    phone.recv(t::REQUEST_NEW).await;
    phone
        .send(&wire::RequestAck {
            request_id: "req1".into(),
        })
        .await;
    tokio::time::sleep(30 * MINUTE).await;
    assert_eq!(fx.pusher.count(), 0);
}

#[tokio::test(start_paused = true)]
async fn a_request_the_phone_opened_is_not_pushed_later() {
    let fx = Fixture::new().await;
    fx.host.set_alert_after(900);
    fx.host.touch_input();
    let mut phone = fx.live_phone().await;
    deliver_ask(&fx, "req1");
    phone.recv(t::REQUEST_NEW).await;
    phone
        .send(&wire::RequestSeen {
            request_id: "req1".into(),
        })
        .await;
    phone.set_presence(true, false).await;

    tokio::time::sleep(20 * MINUTE).await;
    assert_eq!(
        fx.pusher.count(),
        0,
        "the phone opened it; the Mac idling changes nothing"
    );
}

#[tokio::test(start_paused = true)]
async fn a_phone_that_leaves_without_opening_the_question_is_pushed_once_the_mac_is_idle() {
    let fx = Fixture::new().await;
    fx.host.set_alert_after(900);
    fx.host.touch_input();
    let mut phone = fx.live_phone().await;
    deliver_ask(&fx, "req1");
    phone.recv(t::REQUEST_NEW).await;
    phone
        .send(&wire::RequestAck {
            request_id: "req1".into(),
        })
        .await;
    phone.set_presence(true, false).await;

    tokio::time::sleep(14 * MINUTE).await;
    assert_eq!(fx.pusher.count(), 0);
    tokio::time::sleep(2 * MINUTE).await;
    assert_eq!(fx.pusher.count(), 1);
    tokio::time::sleep(30 * MINUTE).await;
    assert_eq!(fx.pusher.count(), 1, "once");
}

#[tokio::test(start_paused = true)]
async fn a_question_answered_before_the_push_sends_nothing_at_all() {
    let fx = Fixture::new().await;
    fx.host.set_alert_after(900);
    let phone = fx.pair_phone().await;
    phone.set_presence(true, false).await;
    fx.host.touch_input();
    deliver_ask(&fx, "req1");
    tokio::time::sleep(5 * MINUTE).await;

    fx.gateway
        .resolve_and_wait("req1", Resolution::AnsweredElsewhere("Popup".into()))
        .await;
    fx.settle().await;
    tokio::time::sleep(30 * MINUTE).await;
    assert_eq!(
        fx.pusher.count(),
        0,
        "never pushed, so there is no notification to replace either"
    );
}

#[tokio::test(start_paused = true)]
async fn a_question_answered_after_the_push_replaces_the_notification() {
    let fx = Fixture::new().await;
    fx.host.set_alert_after(900);
    let phone = fx.pair_phone().await;
    phone.set_presence(true, false).await;
    fx.host.idle_for(20 * 60);
    deliver_ask(&fx, "req1");
    fx.settle().await;
    assert_eq!(fx.pusher.count(), 1);

    fx.gateway
        .resolve_and_wait("req1", Resolution::AnsweredElsewhere("Popup".into()))
        .await;
    fx.settle().await;
    assert_eq!(fx.pusher.count(), 2);
    assert_eq!(fx.pusher.last().unwrap().kind, "replace");
}

#[tokio::test(start_paused = true)]
async fn each_phone_is_judged_on_its_own() {
    let fx = Fixture::new().await;
    fx.host.set_alert_after(900);
    fx.host.idle_for(20 * 60);
    let mut looking = fx.live_phone().await;
    let away = fx.pair_phone().await;
    away.set_presence(true, false).await;

    deliver_ask(&fx, "req1");
    looking.recv(t::REQUEST_NEW).await;
    looking
        .send(&wire::RequestAck {
            request_id: "req1".into(),
        })
        .await;
    fx.settle().await;

    assert_eq!(fx.pusher.count(), 1, "only the phone that is away");
    let push = fx.pusher.last().unwrap();
    let keys = away.keys.clone().unwrap();
    assert!(
        envelope::open(&keys.m2p, Scope::Push, &push.ah).is_ok(),
        "the push is sealed for the phone that is away"
    );
}

#[tokio::test(start_paused = true)]
async fn confirmations_follow_the_same_rule() {
    let fx = Fixture::new().await;
    fx.host.set_alert_after(900);
    let phone = fx.pair_phone().await;
    phone.set_presence(true, false).await;
    fx.host.touch_input();
    deliver_confirm(&fx, "c1", 24 * 3_600_000);

    tokio::time::sleep(10 * MINUTE).await;
    assert_eq!(fx.pusher.count(), 0);
    tokio::time::sleep(6 * MINUTE).await;
    assert_eq!(fx.pusher.count(), 1);
    assert_eq!(fx.pusher.last().unwrap().loc_key, "PUSH_CONFIRM");
}

#[tokio::test(start_paused = true)]
async fn a_phone_without_a_push_registration_is_never_pushed() {
    let fx = Fixture::new().await;
    fx.host.set_alert_after(900);
    fx.host.idle_for(2 * 3600);
    let phone = fx.pair_phone_without_push().await;
    phone.set_presence(true, false).await;
    deliver_ask(&fx, "req1");
    tokio::time::sleep(30 * MINUTE).await;
    assert_eq!(fx.pusher.count(), 0, "there is nowhere to push to");
    assert_eq!(
        fx.gateway.state().await.unwrap().pending_requests,
        1,
        "the question still waits in the inbox"
    );
}

#[tokio::test(start_paused = true)]
async fn the_push_fallback_ignores_the_wait_when_the_foreground_phone_does_not_acknowledge() {
    // The phone was in the foreground but never confirmed receiving the question: it must be
    // told another way now, whatever the Mac is doing.
    let fx = Fixture::new().await;
    fx.host.set_alert_after(900);
    fx.host.touch_input();
    let mut phone = fx.live_phone().await;
    deliver_ask(&fx, "req1");
    phone.recv(t::REQUEST_NEW).await;

    tokio::time::sleep(Duration::from_millis(4_000)).await;
    assert_eq!(fx.pusher.count(), 1);
}

#[tokio::test(start_paused = true)]
async fn the_state_shows_how_long_the_mac_has_been_idle() {
    let fx = Fixture::new().await;
    assert_eq!(fx.gateway.state().await.unwrap().mac_idle_secs, None);
    fx.host.idle_for(125);
    let idle = fx.gateway.state().await.unwrap().mac_idle_secs;
    assert!(matches!(idle, Some(125..=126)), "{idle:?}");
}

// ---------------------------------------------------------------- unreachable phones

#[tokio::test(start_paused = true)]
async fn without_any_paired_phone_the_surface_is_lost_immediately() {
    let fx = Fixture::new().await;
    let sink = deliver_ask(&fx, "req1");
    fx.settle().await;
    assert_eq!(sink.lost.lock().unwrap().len(), 1);
    assert_eq!(fx.gateway.state().await.unwrap().pending_requests, 0);
}

#[tokio::test(start_paused = true)]
async fn a_request_waits_for_an_offline_phone_while_the_broker_is_up() {
    let fx = Fixture::new().await;
    fx.pair_phone_without_push().await;
    let sink = deliver_ask(&fx, "req1");
    tokio::time::sleep(Duration::from_secs(30)).await;
    assert!(
        sink.lost.lock().unwrap().is_empty(),
        "queued for the phone's next visit"
    );
    assert_eq!(fx.gateway.state().await.unwrap().pending_requests, 1);
}

#[tokio::test(start_paused = true)]
async fn a_broker_outage_without_a_push_path_loses_the_surface_after_a_grace_period() {
    let fx = Fixture::new().await;
    fx.pair_phone_without_push().await;
    fx.broker.drop_connection(&format!("mac-{}", fx.mac_id));
    fx.settle().await;

    let sink = deliver_ask(&fx, "req1");
    tokio::time::sleep(Duration::from_secs(3)).await;
    assert!(sink.lost.lock().unwrap().is_empty(), "grace period");
    tokio::time::sleep(Duration::from_secs(3)).await;
    assert_eq!(sink.lost.lock().unwrap().len(), 1);
    assert_eq!(fx.gateway.state().await.unwrap().pending_requests, 0);
}

#[tokio::test(start_paused = true)]
async fn a_quick_reconnect_keeps_the_request() {
    let fx = Fixture::new().await;
    fx.pair_phone_without_push().await;
    fx.broker.drop_connection(&format!("mac-{}", fx.mac_id));
    fx.settle().await;
    let sink = deliver_ask(&fx, "req1");
    tokio::time::sleep(Duration::from_secs(2)).await;
    fx.transport.reconnect();
    tokio::time::sleep(Duration::from_secs(10)).await;
    assert!(sink.lost.lock().unwrap().is_empty());
    assert_eq!(fx.gateway.state().await.unwrap().pending_requests, 1);
}

// ---------------------------------------------------------------- confirmations

#[tokio::test(start_paused = true)]
async fn a_confirmation_decision_reaches_the_daemon() {
    let fx = Fixture::new().await;
    let mut phone = fx.live_phone().await;
    let sink = deliver_confirm(&fx, "conf1", 60_000);
    let new: wire::RequestNew = phone.recv_body(t::REQUEST_NEW).await;
    assert_eq!(new.request.kind, "confirm");

    phone
        .send(&wire::ConfirmDecide {
            client_msg_id: "d1".into(),
            request_id: "conf1".into(),
            wire_index: Some(2),
            dismiss: false,
            input: Some("  not now  ".into()),
        })
        .await;
    let ack: wire::ConfirmAck = phone.recv_body(t::CONFIRM_ACK).await;
    assert_eq!(ack.status, "accepted");
    assert_eq!(
        sink.decisions.lock().unwrap().as_slice(),
        [(2usize, Some("not now".to_string()))]
    );
    phone.recv(t::REQUEST_RESOLVED).await;
}

#[tokio::test(start_paused = true)]
async fn dismissing_maps_to_the_dismiss_action() {
    let fx = Fixture::new().await;
    let mut phone = fx.live_phone().await;
    let sink = deliver_confirm(&fx, "conf1", 60_000);
    phone.recv(t::REQUEST_NEW).await;
    phone
        .send(&wire::ConfirmDecide {
            client_msg_id: "d1".into(),
            request_id: "conf1".into(),
            wire_index: None,
            dismiss: true,
            input: None,
        })
        .await;
    let ack: wire::ConfirmAck = phone.recv_body(t::CONFIRM_ACK).await;
    assert_eq!(ack.status, "accepted");
    assert_eq!(sink.decisions.lock().unwrap().as_slice(), [(2usize, None)]);
}

#[tokio::test(start_paused = true)]
async fn hidden_or_unknown_choices_and_daemon_errors_are_invalid() {
    let fx = Fixture::new().await;
    let mut phone = fx.live_phone().await;
    let sink = deliver_confirm(&fx, "conf1", 60_000);
    phone.recv(t::REQUEST_NEW).await;

    // Wire index 1 exists in the request but is not offered on this surface.
    phone
        .send(&wire::ConfirmDecide {
            client_msg_id: "d1".into(),
            request_id: "conf1".into(),
            wire_index: Some(1),
            dismiss: false,
            input: None,
        })
        .await;
    let ack: wire::ConfirmAck = phone.recv_body(t::CONFIRM_ACK).await;
    assert_eq!(ack.status, "invalid");
    assert!(sink.decisions.lock().unwrap().is_empty());

    *sink.result.lock().unwrap() = Err("confirm input is required".into());
    phone
        .send(&wire::ConfirmDecide {
            client_msg_id: "d2".into(),
            request_id: "conf1".into(),
            wire_index: Some(0),
            dismiss: false,
            input: None,
        })
        .await;
    let ack: wire::ConfirmAck = phone.recv_body(t::CONFIRM_ACK).await;
    assert_eq!(ack.status, "invalid");
    assert_eq!(ack.reason.as_deref(), Some("confirm input is required"));
    assert_eq!(fx.gateway.state().await.unwrap().pending_requests, 1);
}

#[tokio::test(start_paused = true)]
async fn deciding_after_someone_else_reports_lost() {
    let fx = Fixture::new().await;
    let mut phone = fx.live_phone().await;
    let sink = deliver_confirm(&fx, "conf1", 60_000);
    phone.recv(t::REQUEST_NEW).await;
    *sink.result.lock().unwrap() = Ok(false);
    phone
        .send(&wire::ConfirmDecide {
            client_msg_id: "d1".into(),
            request_id: "conf1".into(),
            wire_index: Some(0),
            dismiss: false,
            input: None,
        })
        .await;
    let ack: wire::ConfirmAck = phone.recv_body(t::CONFIRM_ACK).await;
    assert_eq!(ack.status, "lost");
    assert_eq!(ack.reason.as_deref(), Some("feishu"));
}

#[tokio::test(start_paused = true)]
async fn an_expired_confirmation_is_withdrawn_from_the_phone() {
    let fx = Fixture::new().await;
    let mut phone = fx.live_phone().await;
    deliver_confirm(&fx, "conf1", 10_000);
    phone.recv(t::REQUEST_NEW).await;

    tokio::time::sleep(Duration::from_secs(20)).await;
    let resolved: wire::RequestResolved = phone.recv_body(t::REQUEST_RESOLVED).await;
    assert_eq!(
        (resolved.request_id.as_str(), resolved.how.as_str()),
        ("conf1", "expired")
    );
    assert_eq!(fx.gateway.state().await.unwrap().pending_requests, 0);
}

#[tokio::test(start_paused = true)]
async fn a_wrong_kind_of_answer_is_invalid() {
    let fx = Fixture::new().await;
    let mut phone = fx.live_phone().await;
    deliver_confirm(&fx, "conf1", 60_000);
    deliver_ask(&fx, "ask1");
    phone.recv(t::REQUEST_NEW).await;
    phone.recv(t::REQUEST_NEW).await;

    phone
        .send(&answer("c1", "conf1", vec![item(0, &[0], None)]))
        .await;
    let ack: wire::AnswerAck = phone.recv_body(t::ANSWER_ACK).await;
    assert_eq!(ack.status, "invalid");

    phone
        .send(&wire::ConfirmDecide {
            client_msg_id: "d1".into(),
            request_id: "ask1".into(),
            wire_index: Some(0),
            dismiss: false,
            input: None,
        })
        .await;
    let ack: wire::ConfirmAck = phone.recv_body(t::CONFIRM_ACK).await;
    assert_eq!(ack.status, "invalid");
}

#[tokio::test(start_paused = true)]
async fn revoking_a_phone_stops_deliveries_to_it() {
    let fx = Fixture::new().await;
    let mut phone = fx.live_phone().await;
    assert!(fx.gateway.revoke(&phone.phone_id).await);
    phone.recv(t::UNPAIRED).await;
    phone.clear_queue();

    let sink = deliver_ask(&fx, "req1");
    fx.settle().await;
    assert!(!phone.gets(t::REQUEST_NEW, SECOND).await);
    assert_eq!(sink.lost.lock().unwrap().len(), 1, "nobody left to ask");
}
