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
