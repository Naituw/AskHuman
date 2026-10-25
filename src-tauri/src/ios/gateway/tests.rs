use std::time::Duration;

use super::testkit::*;
use super::*;
use crate::ios::wire::{self, t};

const SECOND: Duration = Duration::from_secs(1);

// ---------------------------------------------------------------- pairing

#[tokio::test(start_paused = true)]
async fn pairing_activates_the_phone_after_the_user_allows() {
    let fx = Fixture::new().await;
    let phone = fx.pair_phone().await;

    let state = fx.gateway.state().await.unwrap();
    assert_eq!(state.phones.len(), 1);
    assert_eq!(state.phones[0].phone_id, phone.phone_id);
    assert_eq!(state.phones[0].state, "active");
    assert_eq!(state.phones[0].name, "Test iPhone");
    assert_eq!(state.phones[0].push.as_deref(), Some("ok"));
    assert!(state.pairing.is_none(), "ticket is consumed");
    assert!(fx.gateway.is_ready());
}

#[tokio::test(start_paused = true)]
async fn pairing_rejection_forgets_everything() {
    let fx = Fixture::new().await;
    let mut phone = TestPhone::connect(&fx).await;
    let info = fx.gateway.pair_start().await.unwrap();
    phone.knock(&info.code).await;
    phone.recv_pair(t::PAIR_PENDING).await;

    fx.gateway.pair_decision(&info.ticket_id, false);
    let reject = phone.recv_pair(t::PAIR_REJECT).await;
    assert_eq!(reject.b["reason"], "rejected");
    fx.settle().await;

    let state = fx.gateway.state().await.unwrap();
    assert!(state.phones.is_empty());
    assert!(state.pairing.is_none());
    assert!(!fx.gateway.is_ready());
}

#[tokio::test(start_paused = true)]
async fn a_stranger_without_the_ticket_secret_cannot_knock() {
    let fx = Fixture::new().await;
    let info = fx.gateway.pair_start().await.unwrap();

    // The attacker knows the ticket id (it is part of the topic) but not the secret.
    let mut code = wire::PairCode::decode(&info.code).unwrap();
    code.ts = crypto::b64u(&[9u8; 32]);
    let mut stranger = TestPhone::connect(&fx).await;
    stranger.knock(&code.encode()).await;
    fx.settle().await;
    tokio::time::sleep(SECOND).await;

    let state = fx.gateway.state().await.unwrap();
    assert_eq!(
        state.pairing.unwrap().stage,
        "waiting",
        "the ticket is still available"
    );

    // The real phone can still pair.
    let mut phone = TestPhone::connect(&fx).await;
    phone.knock(&info.code).await;
    phone.recv_pair(t::PAIR_PENDING).await;
}

#[tokio::test(start_paused = true)]
async fn a_ticket_serves_one_phone_only() {
    let fx = Fixture::new().await;
    let info = fx.gateway.pair_start().await.unwrap();
    let mut first = TestPhone::connect(&fx).await;
    first.knock(&info.code).await;
    first.recv_pair(t::PAIR_PENDING).await;

    let mut second = TestPhone::connect(&fx).await;
    second.knock(&info.code).await;
    fx.settle().await;
    let state = fx.gateway.state().await.unwrap();
    assert_eq!(
        state.pairing.unwrap().phone_name.as_deref(),
        Some("Test iPhone")
    );
    assert!(
        second
            .recv_within(t::PAIR_PENDING, Duration::from_secs(1))
            .await
            .is_none(),
        "no second pending"
    );
    // Only the first phone's decision creates a pairing.
    fx.gateway.pair_decision(&info.ticket_id, true);
    first.finish_pairing().await;
    fx.settle().await;
    let state = fx.gateway.state().await.unwrap();
    assert_eq!(state.phones.len(), 1);
    assert_eq!(state.phones[0].phone_id, first.phone_id);
}

#[tokio::test(start_paused = true)]
async fn an_unanswered_ticket_expires() {
    let fx = Fixture::new().await;
    fx.gateway.pair_start().await.unwrap();
    assert!(fx.gateway.state().await.unwrap().pairing.is_some());
    tokio::time::sleep(Duration::from_secs(301)).await;
    assert!(fx.gateway.state().await.unwrap().pairing.is_none());
}

#[tokio::test(start_paused = true)]
async fn an_accepted_pairing_that_never_completes_is_forgotten() {
    let fx = Fixture::new().await;
    let mut phone = TestPhone::connect(&fx).await;
    let info = fx.gateway.pair_start().await.unwrap();
    phone.knock(&info.code).await;
    phone.recv_pair(t::PAIR_PENDING).await;
    fx.gateway.pair_decision(&info.ticket_id, true);
    phone.recv(t::PAIR_ACCEPT).await;

    let state = fx.gateway.state().await.unwrap();
    assert_eq!(state.phones[0].state, "pendingConfirm");
    assert!(
        !fx.gateway.is_ready(),
        "a half-made pairing carries no requests"
    );

    tokio::time::sleep(Duration::from_secs(125)).await;
    let state = fx.gateway.state().await.unwrap();
    assert!(state.phones.is_empty());
    assert!(state.pairing.is_none());
}

#[tokio::test(start_paused = true)]
async fn a_half_made_pairing_cannot_use_other_messages() {
    let fx = Fixture::new().await;
    let mut phone = TestPhone::connect(&fx).await;
    let info = fx.gateway.pair_start().await.unwrap();
    phone.knock(&info.code).await;
    phone.recv_pair(t::PAIR_PENDING).await;
    fx.gateway.pair_decision(&info.ticket_id, true);
    phone.recv(t::PAIR_ACCEPT).await;

    // Before pair.done the phone may not do anything else.
    phone.send(&wire::Ping { t: 1 }).await;
    assert!(!phone.gets(t::PONG, SECOND).await);
}

#[tokio::test(start_paused = true)]
async fn pair_start_requires_a_configured_broker() {
    let fx = Fixture::new().await;
    // Rebuild a gateway with an empty host.
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open_with(dir.path().to_path_buf(), false);
    let (transport, events) = fx.broker.connect("mac-x", None, None);
    let mut cfg = test_config();
    cfg.host = String::new();
    let gateway = IosGateway::start(GatewayDeps {
        host: fx.host.clone(),
        cfg,
        store,
        transport,
        events,
        pusher: fx.pusher.clone(),
    });
    assert!(gateway.pair_start().await.is_err());
}

// ---------------------------------------------------------------- connection and presence

#[tokio::test(start_paused = true)]
async fn the_mac_announces_itself_and_clears_it_on_shutdown() {
    let fx = Fixture::new().await;
    let topic = wire::topic_presence(USER, &fx.mac_id);
    let retained = fx.broker.retained(&topic).expect("presence retained");
    let presence: wire::Presence = serde_json::from_slice(&retained).unwrap();
    assert_eq!((presence.o, presence.r.as_str()), (1, "mac"));
    assert!(presence.e.is_some());

    fx.gateway.shutdown().await;
    let retained = fx.broker.retained(&topic).expect("offline marker");
    let presence: wire::Presence = serde_json::from_slice(&retained).unwrap();
    assert_eq!(presence.o, 0);
}

#[tokio::test(start_paused = true)]
async fn phone_presence_is_tracked() {
    let fx = Fixture::new().await;
    let phone = fx.pair_phone().await;
    let view = |state: &GatewayState| (state.phones[0].online, state.phones[0].foreground);

    assert_eq!(view(&fx.gateway.state().await.unwrap()), (false, false));
    phone.set_presence(true, false).await;
    assert_eq!(view(&fx.gateway.state().await.unwrap()), (true, false));
    phone.set_presence(true, true).await;
    assert_eq!(view(&fx.gateway.state().await.unwrap()), (true, true));
    phone.set_presence(false, false).await;
    assert_eq!(view(&fx.gateway.state().await.unwrap()), (false, false));
}

#[tokio::test(start_paused = true)]
async fn a_broker_outage_is_reported_and_the_gateway_recovers_by_itself() {
    let fx = Fixture::new().await;
    let mut phone = fx.pair_phone().await;
    phone.set_presence(true, true).await;
    assert!(fx.gateway.is_ready());

    fx.broker.drop_connection(&format!("mac-{}", fx.mac_id));
    fx.settle().await;
    let state = fx.gateway.state().await.unwrap();
    assert!(!state.connected);
    assert!(state.last_error.is_some());
    assert!(!fx.gateway.is_ready());
    assert!(
        !state.phones[0].online,
        "presence is unknown while disconnected"
    );

    fx.transport.reconnect();
    fx.settle().await;
    let state = fx.gateway.state().await.unwrap();
    assert!(state.connected);
    assert!(state.last_error.is_none());
    assert!(fx.gateway.is_ready());
    // Subscriptions and presence were re-established: the retained phone presence is back and
    // the phone can talk again.
    assert!(state.phones[0].online && state.phones[0].foreground);
    let retained = fx
        .broker
        .retained(&wire::topic_presence(USER, &fx.mac_id))
        .unwrap();
    let presence: wire::Presence = serde_json::from_slice(&retained).unwrap();
    assert_eq!(presence.o, 1);
    phone.send(&wire::Ping { t: 3 }).await;
    phone.recv(t::PONG).await;
}

// ---------------------------------------------------------------- hello and sync

#[tokio::test(start_paused = true)]
async fn hello_is_answered_with_the_state_of_the_mac() {
    let fx = Fixture::new().await;
    let mut phone = fx.pair_phone().await;
    phone.set_presence(true, true).await;
    let ack = phone.hello().await;

    assert_eq!(ack.mac_name, "Test Mac");
    assert_eq!(ack.epoch.len(), 16);
    assert!(ack.caps.new_task);
    assert!(ack.slot.active);
    let inbox: wire::InboxSnapshot = phone.recv_body(t::INBOX_SNAPSHOT).await;
    assert!(inbox.items.is_empty());
    assert_eq!(inbox.epoch, ack.epoch);
    let agents: wire::AgentsSnapshot = phone.recv_body(t::AGENTS_SNAPSHOT).await;
    assert_eq!(agents.epoch, ack.epoch);
    let follow: wire::FollowState = phone.recv_body(t::FOLLOW_STATE).await;
    assert!(follow.keys.is_empty());
    let slot: wire::SlotState = phone.recv_body(t::SLOT_STATE).await;
    assert!(slot.active);
}

#[tokio::test(start_paused = true)]
async fn resync_resends_one_stream() {
    let fx = Fixture::new().await;
    let mut phone = fx.live_phone().await;
    phone.clear_queue();
    phone
        .send(&wire::Resync {
            stream: "inbox".into(),
        })
        .await;
    phone.recv(t::INBOX_SNAPSHOT).await;
    assert!(
        !phone
            .gets(t::AGENTS_SNAPSHOT, Duration::from_millis(500))
            .await
    );
}

#[tokio::test(start_paused = true)]
async fn ping_is_answered_with_pong() {
    let fx = Fixture::new().await;
    let mut phone = fx.live_phone().await;
    phone.send(&wire::Ping { t: 42 }).await;
    let pong: wire::Pong = phone.recv_body(t::PONG).await;
    assert_eq!(pong.t, 42);
}

// ---------------------------------------------------------------- inbound security

#[tokio::test(start_paused = true)]
async fn garbage_unknown_senders_and_wrong_keys_are_ignored() {
    let fx = Fixture::new().await;
    let mut phone = fx.live_phone().await;
    phone.clear_queue();

    phone.send_raw_up(b"not json".to_vec()).await;
    phone.send_raw_up(b"{\"v\":1}".to_vec()).await;

    // Right format, wrong key.
    let topic = wire::topic_up(USER, &fx.mac_id);
    let plain = envelope::Plain::new(t::PING, serde_json::json!({"t": 1}), BASE_MS);
    let forged = envelope::seal_plain(
        &[1u8; 32],
        envelope::Scope::Topic(&topic),
        &phone.phone_id,
        1,
        &plain,
    );
    phone.send_raw_up(forged).await;

    // Unknown sender id.
    let keys = phone.keys.clone().unwrap();
    let stranger = envelope::seal_plain(
        &keys.p2m,
        envelope::Scope::Topic(&topic),
        "stranger",
        1,
        &plain,
    );
    phone.send_raw_up(stranger).await;

    assert!(!phone.gets(t::PONG, Duration::from_secs(1)).await);
    // The legitimate phone still works.
    phone.send(&wire::Ping { t: 7 }).await;
    phone.recv(t::PONG).await;
}

#[tokio::test(start_paused = true)]
async fn replayed_messages_are_processed_once() {
    let fx = Fixture::new().await;
    let mut phone = fx.live_phone().await;
    phone.clear_queue();

    let plain = envelope::Plain::new(t::PING, serde_json::json!({"t": 1}), BASE_MS);
    phone.send_plain(plain.clone()).await;
    phone.send_plain(plain).await;
    phone.recv(t::PONG).await;
    assert!(
        !phone.gets(t::PONG, Duration::from_secs(1)).await,
        "duplicate dropped"
    );
}

#[tokio::test(start_paused = true)]
async fn messages_outside_the_clock_window_are_refused() {
    let fx = Fixture::new().await;
    let mut phone = fx.live_phone().await;
    phone.clear_queue();

    let stale = envelope::Plain::new(
        t::PING,
        serde_json::json!({"t": 1}),
        BASE_MS - 20 * 60 * 1000,
    );
    phone.send_plain(stale).await;
    let error: wire::ErrorBody = phone.recv_body(t::ERROR).await;
    assert_eq!(error.code, "clock_skew");
    assert!(!phone.gets(t::PONG, Duration::from_millis(500)).await);
}

#[tokio::test(start_paused = true)]
async fn unknown_types_and_malformed_bodies_get_an_error_reply() {
    let fx = Fixture::new().await;
    let mut phone = fx.live_phone().await;
    phone.clear_queue();

    phone
        .send_plain(envelope::Plain::new(
            "no.such.type",
            serde_json::json!({}),
            BASE_MS,
        ))
        .await;
    let error: wire::ErrorBody = phone.recv_body(t::ERROR).await;
    assert_eq!(error.code, "unknown_type");

    phone
        .send_plain(envelope::Plain::new(
            t::PING,
            serde_json::json!({"t": "x"}),
            BASE_MS,
        ))
        .await;
    let error: wire::ErrorBody = phone.recv_body(t::ERROR).await;
    assert_eq!(error.code, "bad_request");
}

// ---------------------------------------------------------------- revoking

#[tokio::test(start_paused = true)]
async fn revoking_tells_the_phone_and_removes_the_pairing() {
    let fx = Fixture::new().await;
    let mut phone = fx.live_phone().await;
    assert!(fx.gateway.revoke(&phone.phone_id).await);
    phone.recv(t::UNPAIRED).await;

    let state = fx.gateway.state().await.unwrap();
    assert!(state.phones.is_empty());
    assert!(!fx.gateway.is_ready());
    // The keys are gone: anything the phone sends now is ignored.
    phone.send(&wire::Ping { t: 1 }).await;
    assert!(!phone.gets(t::PONG, SECOND).await);
    assert!(!fx.gateway.revoke("nobody").await);
}

#[tokio::test(start_paused = true)]
async fn unpair_from_the_phone_removes_the_pairing() {
    let fx = Fixture::new().await;
    let mut phone = fx.live_phone().await;
    phone.send(&wire::Unpair {}).await;
    assert!(fx.gateway.state().await.unwrap().phones.is_empty());
}

#[tokio::test(start_paused = true)]
async fn pairings_survive_a_gateway_restart() {
    let fx = Fixture::new().await;
    let mut phone = fx.pair_phone().await;
    let fx = fx.restart().await;

    let state = fx.gateway.state().await.unwrap();
    assert_eq!(state.phones.len(), 1);
    assert!(fx.gateway.is_ready());

    // The same keys still work and the new epoch differs from the old one.
    phone.set_presence(true, true).await;
    let ack = phone.hello().await;
    assert_eq!(ack.epoch.len(), 16);
}

#[tokio::test(start_paused = true)]
async fn renaming_a_phone_sets_the_alias() {
    let fx = Fixture::new().await;
    let phone = fx.pair_phone().await;
    fx.gateway.rename(&phone.phone_id, "  Work phone\n");
    fx.settle().await;
    let state = fx.gateway.state().await.unwrap();
    assert_eq!(state.phones[0].alias_name.as_deref(), Some("Work phone"));
    assert_eq!(state.phones[0].name, "Test iPhone");
}

#[tokio::test(start_paused = true)]
async fn push_registration_is_stored_and_acknowledged() {
    let fx = Fixture::new().await;
    let mut phone = fx.live_phone().await;
    phone.clear_queue();
    phone
        .send(&wire::PushRegister {
            handle: "new-handle".into(),
            env: "prod".into(),
            expires_at: BASE_MS / 1000 + 1000,
        })
        .await;
    let state: wire::PushState = phone.recv_body(t::PUSH_STATE).await;
    assert_eq!(state.status, "ok");
}

use crate::ios::crypto;
use crate::ios::envelope;
use crate::ios::store::Store;
