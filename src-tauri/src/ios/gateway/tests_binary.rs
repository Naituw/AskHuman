//! The binary envelope and blob frames (`docs/specs/ios-attachment-traffic.md` §5): when the Mac sends them, what it
//! takes from a phone, and what it drops. The blob frames of file chunks are tested with the attachments
//! (`tests_assets.rs`).

use std::time::Duration;

use serde_json::json;

use super::testkit::*;
use crate::ios::crypto;
use crate::ios::envelope::{self, Format, Plain, Scope};
use crate::ios::wire::{self, t};

const SECOND: Duration = Duration::from_secs(1);

fn ping(n: i64) -> wire::Ping {
    wire::Ping { t: n }
}

/// What the Mac's ledger counted for one message kind, e.g. `in:rejected`.
async fn counted(fx: &Fixture, kind: &str) -> u64 {
    let state = fx.gateway.state().await.unwrap();
    state
        .traffic_breakdown
        .kinds
        .get(kind)
        .map_or(0, |k| k.msgs)
}

// ---------------------------------------------------------------- negotiation

/// Lets everything that was sent arrive and be read, and returns how many messages the phone has seen so far.
async fn settled(phone: &mut TestPhone) -> usize {
    tokio::time::sleep(SECOND).await;
    phone.clear_queue();
    phone.received.len()
}

#[tokio::test(start_paused = true)]
async fn the_mac_sends_binary_envelopes_to_a_phone_that_declared_them() {
    let fx = Fixture::new().await;
    let mut phone = fx.pair_phone().await;
    phone.binary = true;
    phone.set_presence(true, true).await;
    let mark = settled(&mut phone).await;
    let ack = phone.hello().await;
    assert!(ack.caps.binary, "the Mac says it opens binary frames");
    settled(&mut phone).await;

    // The hello itself was JSON (the phone had not heard from the Mac yet); what answers it is binary already: the
    // ack and every snapshot.
    let answers = &phone.received[mark..];
    let kinds: Vec<&str> = answers.iter().map(|r| r.ty.as_str()).collect();
    assert!(kinds.contains(&t::HELLO_ACK) && kinds.contains(&t::AGENTS_SNAPSHOT));
    assert!(answers.iter().all(|r| r.binary && !r.blob), "{answers:?}");

    // And so is whatever comes later; the phone's own messages go binary now that the Mac said it opens them.
    phone.send(&ping(7)).await;
    let pong: wire::Pong = phone.recv_body(t::PONG).await;
    assert_eq!(pong.t, 7);
    assert!(phone.received.last().unwrap().binary);
    assert_eq!(counted(&fx, "in:ping").await, 1);
}

#[tokio::test(start_paused = true)]
async fn a_phone_that_did_not_declare_binary_is_sent_json_only() {
    let fx = Fixture::new().await;
    let mut phone = fx.pair_phone().await;
    phone.set_presence(true, true).await;
    let mark = settled(&mut phone).await;
    let ack = phone.hello().await;
    // The Mac declares the capability to everyone; whether it is used is the phone's say.
    assert!(ack.caps.binary);
    phone.send(&ping(1)).await;
    phone.recv(t::PONG).await;
    settled(&mut phone).await;
    let answers = &phone.received[mark..];
    assert!(answers.len() >= 5, "{answers:?}");
    assert!(answers.iter().all(|r| !r.binary && !r.blob), "{answers:?}");
}

#[tokio::test(start_paused = true)]
async fn a_mac_that_does_not_declare_binary_sends_json_whatever_the_phone_says() {
    let fx = Fixture::new().await;
    fx.host
        .binary_supported
        .store(false, std::sync::atomic::Ordering::SeqCst);
    let mut phone = fx.pair_phone().await;
    phone.binary = true;
    phone.set_presence(true, true).await;
    let mark = settled(&mut phone).await;
    let ack = phone.hello().await;
    assert!(!ack.caps.binary);
    phone.send(&ping(2)).await;
    phone.recv(t::PONG).await;
    settled(&mut phone).await;
    let answers = &phone.received[mark..];
    assert!(answers.len() >= 5, "{answers:?}");
    assert!(answers.iter().all(|r| !r.binary), "{answers:?}");
    assert_eq!(phone.blobs_sent, 0);
}

#[tokio::test(start_paused = true)]
async fn the_format_follows_the_latest_hello() {
    let fx = Fixture::new().await;
    let mut phone = fx.live_phone_with(|p| p.binary = true).await;
    let mark = settled(&mut phone).await;
    assert!(phone.received.last().unwrap().binary);

    // A hello without the bit takes it back: the answers to it are JSON.
    phone.binary = false;
    phone.hello().await;
    phone.send(&ping(3)).await;
    phone.recv(t::PONG).await;
    let end = settled(&mut phone).await;
    let answers = &phone.received[mark..end];
    assert!(answers.len() >= 6, "{answers:?}");
    assert!(answers.iter().all(|r| !r.binary), "{answers:?}");

    // And a hello with it gives it back.
    phone.binary = true;
    phone.hello().await;
    let end_again = settled(&mut phone).await;
    let answers = &phone.received[end..end_again];
    assert!(answers.len() >= 5, "{answers:?}");
    assert!(answers.iter().all(|r| r.binary), "{answers:?}");
}

#[tokio::test(start_paused = true)]
async fn a_phone_that_went_offline_is_sent_json_until_it_says_hello_again() {
    let fx = Fixture::new().await;
    let mut phone = fx.live_phone_with(|p| p.binary = true).await;
    let mark = settled(&mut phone).await;

    // Its last hello is no longer a promise once the phone has gone: the app may have been replaced while it was away.
    phone.set_presence(false, false).await;
    phone.send(&ping(4)).await;
    phone.recv(t::PONG).await;
    let end = settled(&mut phone).await;
    let answers = &phone.received[mark..end];
    assert!(!answers.is_empty());
    assert!(answers.iter().all(|r| !r.binary), "{answers:?}");

    phone.set_presence(true, true).await;
    phone.hello().await;
    let end_again = settled(&mut phone).await;
    let answers = &phone.received[end..end_again];
    assert!(answers.len() >= 5, "{answers:?}");
    assert!(answers.iter().all(|r| r.binary), "{answers:?}");
}

#[tokio::test(start_paused = true)]
async fn the_traffic_ledger_counts_the_bytes_that_went_over_the_wire() {
    let fx = Fixture::new().await;
    let mut phone = fx.live_phone_with(|p| p.binary = true).await;
    settled(&mut phone).await;

    let state = fx.gateway.state().await.unwrap();
    let ledger = |ty: &str| state.traffic_breakdown.kinds[&format!("out:{ty}")].bytes;
    for ty in [t::HELLO_ACK, t::AGENTS_SNAPSHOT, t::INBOX_SNAPSHOT] {
        let seen: usize = phone
            .received
            .iter()
            .filter(|r| r.ty == ty)
            .map(|r| r.bytes)
            .sum();
        assert!(seen > 0, "{ty}");
        assert_eq!(ledger(ty), seen as u64, "{ty}");
    }
}

#[tokio::test(start_paused = true)]
async fn the_same_snapshots_take_fewer_bytes_in_binary() {
    let fx = Fixture::new().await;
    let mut json_phone = fx.live_phone_with(|_| {}).await;
    let mut binary_phone = fx.live_phone_with(|p| p.binary = true).await;
    settled(&mut json_phone).await;
    settled(&mut binary_phone).await;
    let total = |phone: &TestPhone| phone.received.iter().map(|r| r.bytes).sum::<usize>();
    assert!(
        total(&binary_phone) < total(&json_phone),
        "{} against {}",
        total(&binary_phone),
        total(&json_phone)
    );
}

// ---------------------------------------------------------------- what the Mac takes and drops

#[tokio::test(start_paused = true)]
async fn broken_envelopes_from_a_phone_are_dropped_and_counted_as_rejected() {
    let fx = Fixture::new().await;
    let mut phone = fx.live_phone_with(|p| p.binary = true).await;
    phone.recv(t::SLOT_STATE).await;
    let keys = phone.keys.clone().unwrap();
    let topic = wire::topic_up(USER, &fx.mac_id);
    let scope = Scope::Topic(&topic);
    let big_ping = || {
        Plain::new(
            t::PING,
            json!({"t": 1, "padding": "x".repeat(1_500)}),
            BASE_MS,
        )
    };
    let seal = |key: &[u8; 32], sender: &str, kid: u32, compress: bool, format: Format| {
        envelope::seal_plain_as(key, scope, sender, kid, &big_ping(), compress, format)
    };
    let good = seal(&keys.p2m, &phone.phone_id, keys.kid, false, Format::Binary);
    let compressed = seal(&keys.p2m, &phone.phone_id, keys.kid, true, Format::Binary);
    assert!(
        envelope::parse_frame(&compressed).unwrap().compressed,
        "the test must send a compressed envelope"
    );
    let mut flipped = good.clone();
    *flipped.last_mut().unwrap() ^= 0x01;
    let mut reserved = good.clone();
    reserved[1] = 0x02;
    let cases: Vec<(&str, Vec<u8>)> = vec![
        ("only the magic", vec![0xA1]),
        ("cut short", good[..40].to_vec()),
        ("a reserved flag", reserved),
        ("a compressed one: phones do not compress", compressed),
        ("a flipped byte", flipped),
        (
            "another key id",
            seal(
                &keys.p2m,
                &phone.phone_id,
                keys.kid + 1,
                false,
                Format::Binary,
            ),
        ),
        (
            "another key",
            seal(&[1u8; 32], &phone.phone_id, keys.kid, false, Format::Binary),
        ),
        (
            "a sender nobody paired",
            seal(
                &keys.p2m,
                &crypto::new_id(),
                keys.kid,
                false,
                Format::Binary,
            ),
        ),
        (
            "the Mac's own id",
            seal(&keys.p2m, &fx.mac_id, keys.kid, false, Format::Binary),
        ),
    ];
    let before = counted(&fx, "in:rejected").await;
    for (name, bytes) in &cases {
        phone.send_raw_up(bytes.clone()).await;
        assert!(!phone.gets(t::PONG, SECOND).await, "{name} was answered");
    }
    assert_eq!(
        counted(&fx, "in:rejected").await - before,
        cases.len() as u64
    );

    // The intact one is taken, and so is the same message in JSON.
    phone.send_raw_up(good).await;
    phone.recv(t::PONG).await;
    phone
        .send_raw_up(seal(
            &keys.p2m,
            &phone.phone_id,
            keys.kid,
            false,
            Format::Json,
        ))
        .await;
    phone.recv(t::PONG).await;
}

#[tokio::test(start_paused = true)]
async fn a_blob_frame_is_only_taken_for_an_upload_chunk() {
    let fx = Fixture::new().await;
    let mut phone = fx.live_phone_with(|p| p.binary = true).await;
    phone.recv(t::SLOT_STATE).await;
    let keys = phone.keys.clone().unwrap();
    let phone_id = phone.phone_id.clone();
    let topic = wire::topic_up(USER, &fx.mac_id);
    let blob = |head: Plain, bytes: &[u8]| {
        envelope::seal_blob(
            &keys.p2m,
            Scope::Topic(&topic),
            &phone_id,
            keys.kid,
            &head,
            bytes,
        )
        .expect("the phone's id fits the binary header")
    };
    let upload_id = crypto::new_id();
    let cases: Vec<(&str, Vec<u8>)> = vec![
        (
            "a ping",
            blob(Plain::new(t::PING, json!({"t": 9}), BASE_MS), b"abc"),
        ),
        (
            "an asset request",
            blob(
                Plain::new(
                    t::ASSET_GET,
                    json!({"assetId": crypto::new_id(), "from": 0, "count": 1}),
                    BASE_MS,
                ),
                b"abc",
            ),
        ),
        (
            "an upload chunk that has its bytes twice",
            blob(
                Plain::new(
                    t::UPLOAD_CHUNK,
                    json!({"uploadId": upload_id, "index": 0, "data": "YWJj"}),
                    BASE_MS,
                ),
                b"abc",
            ),
        ),
    ];
    let before = counted(&fx, "in:rejected").await;
    for (name, bytes) in &cases {
        phone.send_raw_up(bytes.clone()).await;
        assert!(
            !phone.gets(t::PONG, SECOND).await && !phone.gets(t::UPLOAD_ACK, SECOND).await,
            "{name} was answered"
        );
    }
    assert_eq!(
        counted(&fx, "in:rejected").await - before,
        cases.len() as u64
    );

    // An upload chunk of the right shape reaches the upload logic: nobody began this upload, so it is told it is gone.
    phone
        .send_raw_up(blob(
            Plain::new(
                t::UPLOAD_CHUNK,
                json!({"uploadId": upload_id, "index": 0}),
                BASE_MS,
            ),
            b"abc",
        ))
        .await;
    let ack: wire::UploadAck = phone.recv_body(t::UPLOAD_ACK).await;
    assert_eq!(
        (
            ack.upload_id.as_str(),
            ack.status.as_str(),
            ack.reason.as_deref()
        ),
        (upload_id.as_str(), "error", Some("gone"))
    );
}

#[tokio::test(start_paused = true)]
async fn the_pairing_channel_takes_json_only() {
    let fx = Fixture::new().await;
    let mut phone = TestPhone::connect(&fx).await;
    let info = fx.gateway.pair_start().await.expect("pair start");

    // A pairing request in the binary envelope is not read: the ticket is still waiting for one.
    phone.pair_in_binary = true;
    phone.knock(&info.code).await;
    tokio::time::sleep(SECOND).await;
    let view = fx.gateway.state().await.unwrap().pairing.unwrap();
    assert_eq!(view.stage, "waiting");

    // The same request in JSON is, which also shows that the first one did not use the ticket up.
    phone.pair_in_binary = false;
    phone.knock(&info.code).await;
    let _ = phone.recv_pair(t::PAIR_PENDING).await;
    let view = fx.gateway.state().await.unwrap().pairing.unwrap();
    assert_eq!(view.stage, "pending");
}
