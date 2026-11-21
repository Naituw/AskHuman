//! The Mac's own account of attachment traffic (`docs/specs/ios-attachment-traffic.md` §6.6, AT5): the chunks it sent told
//! apart as preview or file and as first time or repeat, and the `asset.get` messages it acted on by what they asked for.
//! Like the rest of the breakdown it stays on this Mac (`channel ios status --json`) and is not a message.
//!
//! These run in real time for the same reason as the preview tests (`tests_preview.rs`): the maker of a preview runs on a
//! blocking thread.

use std::collections::BTreeMap;

use serde_json::json;

use super::assets::{SentChunks, SentKey};
use super::testkit::*;
use super::tests_preview::*;
use crate::ios::crypto;
use crate::ios::envelope::Plain;
use crate::ios::store::KindCount;
use crate::ios::views::AssetSource;
use crate::ios::wire::{t, ASSET_CHUNK_BYTES};

/// What the Mac counted of the attachment traffic so far.
struct Counts {
    kinds: BTreeMap<String, KindCount>,
    assets: BTreeMap<String, KindCount>,
}

impl Counts {
    fn part(&self, key: &str) -> KindCount {
        self.assets.get(key).copied().unwrap_or_default()
    }

    fn kind(&self, key: &str) -> KindCount {
        self.kinds.get(key).copied().unwrap_or_default()
    }

    /// The four kinds of chunk put together.
    fn chunk_parts(&self) -> KindCount {
        [
            "chunk.preview",
            "chunk.preview.repeat",
            "chunk.original",
            "chunk.original.repeat",
        ]
        .iter()
        .map(|key| self.part(key))
        .fold(KindCount::default(), |sum, part| KindCount {
            msgs: sum.msgs + part.msgs,
            bytes: sum.bytes + part.bytes,
        })
    }

    /// The three sources of an `asset.get` put together.
    fn get_parts(&self) -> KindCount {
        ["get.request", "get.history", "get.unresolved"]
            .iter()
            .map(|key| self.part(key))
            .fold(KindCount::default(), |sum, part| KindCount {
                msgs: sum.msgs + part.msgs,
                bytes: sum.bytes + part.bytes,
            })
    }
}

async fn counts(fx: &Fixture) -> Counts {
    let state = fx.gateway.state().await.unwrap();
    Counts {
        kinds: state.traffic_breakdown.kinds,
        assets: state.traffic_breakdown.assets,
    }
}

/// The wire bytes of what the phone was sent after `mark`.
fn wire_bytes_since(phone: &TestPhone, mark: usize) -> u64 {
    phone.received[mark..].iter().map(|r| r.bytes as u64).sum()
}

fn file_of(chunks: usize, seed: u8) -> Vec<u8> {
    (0..ASSET_CHUNK_BYTES * (chunks - 1) + 1_000)
        .map(|i| (i as u8).wrapping_mul(7).wrapping_add(seed))
        .collect()
}

// ---------------------------------------------------------------- chunks

#[tokio::test]
async fn the_parts_of_the_chunks_add_up_to_what_the_type_says_each_byte_in_one_of_them() {
    let fx = Fixture::new().await;
    let picture = source_of(fx.dir.path(), "shot.png", &picture_bytes(1), true);
    let file = source_of(fx.dir.path(), "notes.bin", &file_of(2, 2), true);
    let mut phone = with_request(&fx, &[picture.clone(), file.clone()]).await;
    fx.host.previews.make_previews_of(ASSET_CHUNK_BYTES + 100);

    // A preview of two chunks, the picture itself in three, and a file of two.
    let mark = phone.received.len();
    phone.send(&preview_of(&picture.asset_id)).await;
    chunks_of(&mut phone, 2).await;
    let preview_bytes = wire_bytes_since(&phone, mark);

    let mark = phone.received.len();
    phone.send(&ask_for(&picture.asset_id, 0, 8, None)).await;
    chunks_of(&mut phone, 3).await;
    phone.send(&ask_for(&file.asset_id, 0, 8, None)).await;
    chunks_of(&mut phone, 2).await;
    let original_bytes = wire_bytes_since(&phone, mark);

    let counted = counts(&fx).await;
    assert_eq!(
        counted.part("chunk.preview"),
        KindCount {
            msgs: 2,
            bytes: preview_bytes
        }
    );
    assert_eq!(
        counted.part("chunk.original"),
        KindCount {
            msgs: 5,
            bytes: original_bytes
        }
    );
    assert_eq!(counted.part("chunk.preview.repeat"), KindCount::default());
    assert_eq!(counted.part("chunk.original.repeat"), KindCount::default());
    assert_eq!(
        counted.chunk_parts(),
        counted.kind("out:asset.chunk"),
        "the parts are the type's total, told apart: {:?}",
        counted.assets
    );
    assert_eq!(counted.kind("out:asset.chunk").msgs, 7);
}

#[tokio::test]
async fn a_phone_that_takes_blob_frames_is_counted_the_same_way() {
    let fx = Fixture::new().await;
    let asset = source_of(fx.dir.path(), "notes.bin", &file_of(3, 3), true);
    let mut phone = fx.live_phone_with(|p| p.binary = true).await;
    deliver(&fx, "req1", vec![asset.clone()]);
    phone.recv(t::REQUEST_NEW).await;

    let mark = phone.received.len();
    phone.send(&ask_for(&asset.asset_id, 0, 8, None)).await;
    chunks_of(&mut phone, 3).await;
    assert!(phone.received[mark..].iter().any(|r| r.blob));
    let wire = phone.received[mark..]
        .iter()
        .filter(|r| r.ty == t::ASSET_CHUNK)
        .map(|r| r.bytes as u64)
        .sum::<u64>();

    let counted = counts(&fx).await;
    assert_eq!(
        counted.part("chunk.original"),
        KindCount {
            msgs: 3,
            bytes: wire
        }
    );
    assert_eq!(counted.chunk_parts(), counted.kind("out:asset.chunk"));
}

#[tokio::test]
async fn a_second_fetch_of_content_the_phone_was_sent_is_a_repeat_even_under_a_new_asset_id() {
    let fx = Fixture::new().await;
    // The same screenshot in two requests: two asset ids, one hash.
    let data = picture_bytes(4);
    let first = source_of(fx.dir.path(), "a.png", &data, true);
    let second = source_of(fx.dir.path(), "b.png", &data, true);
    assert_ne!(first.asset_id, second.asset_id);
    assert_eq!(first.sha256, second.sha256);
    let mut phone = fx.live_phone().await;
    deliver(&fx, "req1", vec![first.clone()]);
    phone.recv(t::REQUEST_NEW).await;
    deliver(&fx, "req2", vec![second.clone()]);
    phone.recv(t::REQUEST_NEW).await;

    phone.send(&ask_for(&first.asset_id, 0, 8, None)).await;
    chunks_of(&mut phone, 3).await;
    let counted = counts(&fx).await;
    assert_eq!(counted.part("chunk.original").msgs, 3);
    assert_eq!(counted.part("chunk.original.repeat").msgs, 0);

    phone.send(&ask_for(&second.asset_id, 0, 8, None)).await;
    chunks_of(&mut phone, 3).await;
    let counted = counts(&fx).await;
    assert_eq!(
        counted.part("chunk.original").msgs,
        3,
        "no more first-time bytes"
    );
    assert_eq!(
        counted.part("chunk.original.repeat").msgs,
        3,
        "the phone was sent this content in full before"
    );
    assert!(counted.part("chunk.original.repeat").bytes > 0);
    assert_eq!(counted.chunk_parts(), counted.kind("out:asset.chunk"));

    // A preview of it is a first time for the preview: what the phone has of the file does not stand for it here.
    fx.host.previews.make_previews_of(1_000);
    phone.send(&preview_of(&second.asset_id)).await;
    chunks_of(&mut phone, 1).await;
    let counted = counts(&fx).await;
    assert_eq!(counted.part("chunk.preview").msgs, 1);
    assert_eq!(counted.part("chunk.preview.repeat").msgs, 0);
    phone.send(&preview_of(&first.asset_id)).await;
    chunks_of(&mut phone, 1).await;
    let counted = counts(&fx).await;
    assert_eq!(counted.part("chunk.preview").msgs, 1);
    assert_eq!(
        counted.part("chunk.preview.repeat").msgs,
        1,
        "the same preview, asked for under the other asset id"
    );
    assert_eq!(counted.chunk_parts(), counted.kind("out:asset.chunk"));
}

#[tokio::test]
async fn a_fetch_that_is_not_complete_yet_is_not_a_repeat_whatever_is_asked_for_again() {
    let fx = Fixture::new().await;
    // Nineteen chunks: three windows.
    let asset = source_of(fx.dir.path(), "big.bin", &file_of(19, 5), true);
    let mut phone = with_request(&fx, std::slice::from_ref(&asset)).await;

    phone.send(&ask_for(&asset.asset_id, 0, 8, None)).await;
    chunks_of(&mut phone, 8).await;
    // The window is asked for again (a lost answer), then the fetch goes on.
    phone.send(&ask_for(&asset.asset_id, 0, 8, None)).await;
    chunks_of(&mut phone, 8).await;
    phone.send(&ask_for(&asset.asset_id, 8, 8, None)).await;
    chunks_of(&mut phone, 8).await;
    phone.send(&ask_for(&asset.asset_id, 16, 8, None)).await;
    chunks_of(&mut phone, 3).await;
    let counted = counts(&fx).await;
    assert_eq!(counted.part("chunk.original").msgs, 27);
    assert_eq!(counted.part("chunk.original.repeat").msgs, 0);

    // Now the whole file has been sent: the next fetch is a repeat, all of it.
    phone.send(&ask_for(&asset.asset_id, 0, 8, None)).await;
    chunks_of(&mut phone, 8).await;
    let counted = counts(&fx).await;
    assert_eq!(counted.part("chunk.original").msgs, 27);
    assert_eq!(counted.part("chunk.original.repeat").msgs, 8);
    assert_eq!(counted.chunk_parts(), counted.kind("out:asset.chunk"));
}

#[tokio::test]
async fn what_one_phone_was_sent_does_not_make_another_phones_fetch_a_repeat() {
    let fx = Fixture::new().await;
    let asset = source_of(fx.dir.path(), "notes.bin", &file_of(2, 6), true);
    let mut first = with_request(&fx, std::slice::from_ref(&asset)).await;
    let mut second = fx.live_phone().await;

    first.send(&ask_for(&asset.asset_id, 0, 8, None)).await;
    chunks_of(&mut first, 2).await;
    second.send(&ask_for(&asset.asset_id, 0, 8, None)).await;
    chunks_of(&mut second, 2).await;

    let counted = counts(&fx).await;
    assert_eq!(counted.part("chunk.original").msgs, 4);
    assert_eq!(counted.part("chunk.original.repeat").msgs, 0);
}

#[tokio::test]
async fn a_preview_request_answered_with_the_file_is_counted_as_the_file() {
    let fx = Fixture::new().await;
    // Too small to get a preview: the Mac sends the file and names it `original`.
    let asset = source_of(fx.dir.path(), "tiny.png", &[7u8; 5_000], true);
    let mut phone = with_request(&fx, std::slice::from_ref(&asset)).await;

    phone.send(&preview_of(&asset.asset_id)).await;
    let chunks = chunks_of(&mut phone, 1).await;
    assert_eq!(chunks[0].variant.as_deref(), Some("original"));
    let counted = counts(&fx).await;
    assert_eq!(counted.part("chunk.original").msgs, 1);
    assert_eq!(counted.part("chunk.preview").msgs, 0);
}

#[tokio::test]
async fn a_picture_of_the_history_is_told_apart_by_its_asset_id() {
    let fx = Fixture::new().await;
    // A picture of the history has no hash: the same bytes under another asset id are another picture to the count.
    let data = picture_bytes(8);
    let one = source_of(fx.dir.path(), "h1.png", &data, false);
    let other = source_of(fx.dir.path(), "h2.png", &data, false);
    for picture in [&one, &other] {
        fx.host
            .history_pictures
            .lock()
            .unwrap()
            .insert(picture.asset_id.clone(), picture.clone());
    }
    let mut phone = fx.live_phone().await;

    for picture in [&one, &one, &other] {
        phone.send(&ask_for(&picture.asset_id, 0, 8, None)).await;
        chunks_of(&mut phone, 3).await;
    }
    let counted = counts(&fx).await;
    assert_eq!(
        counted.part("chunk.original").msgs,
        6,
        "`one` the first time, and `other`"
    );
    assert_eq!(counted.part("chunk.original.repeat").msgs, 3, "`one` again");
    assert_eq!(counted.chunk_parts(), counted.kind("out:asset.chunk"));
}

// ---------------------------------------------------------------- asset.get

#[tokio::test]
async fn asset_gets_are_counted_by_what_they_asked_for() {
    let fx = Fixture::new().await;
    let attachment = source_of(fx.dir.path(), "notes.bin", &file_of(1, 9), true);
    let history = source_of(fx.dir.path(), "history.png", &picture_bytes(9), false);
    fx.host
        .history_pictures
        .lock()
        .unwrap()
        .insert(history.asset_id.clone(), history.clone());
    let mut phone = with_request(&fx, std::slice::from_ref(&attachment)).await;

    phone.send(&ask_for(&attachment.asset_id, 0, 8, None)).await;
    chunks_of(&mut phone, 1).await;
    // The first ask for a picture of the history goes to the host; the second is remembered by the gateway.
    for _ in 0..2 {
        phone.send(&ask_for(&history.asset_id, 0, 8, None)).await;
        chunks_of(&mut phone, 3).await;
    }
    // An id the Mac gives out the shape of that nothing has, one that is not an id at all, and a request that is not one.
    phone.send(&ask_for(&crypto::new_id(), 0, 8, None)).await;
    phone.recv(t::ASSET_ERROR).await;
    phone.send(&ask_for("not-an-id", 0, 8, None)).await;
    phone.recv(t::ASSET_ERROR).await;
    phone
        .send_plain(Plain::new(t::ASSET_GET, json!({"assetId": 5}), BASE_MS))
        .await;
    phone.recv(t::ERROR).await;

    let counted = counts(&fx).await;
    assert_eq!(counted.part("get.request").msgs, 1);
    assert_eq!(counted.part("get.history").msgs, 2);
    assert_eq!(counted.part("get.unresolved").msgs, 3);
    assert!(counted.part("get.request").bytes > 0 && counted.part("get.history").bytes > 0);
    assert_eq!(
        counted.get_parts(),
        counted.kind("in:asset.get"),
        "every message was acted on, so the sources are the type's total, told apart"
    );
}

// ---------------------------------------------------------------- the memory

#[tokio::test]
async fn the_parts_are_in_the_status_the_command_line_prints() {
    let fx = Fixture::new().await;
    let asset = source_of(fx.dir.path(), "notes.bin", &file_of(2, 10), true);
    let mut phone = with_request(&fx, std::slice::from_ref(&asset)).await;
    phone.send(&ask_for(&asset.asset_id, 0, 8, None)).await;
    chunks_of(&mut phone, 2).await;

    let state = fx.gateway.state().await.unwrap();
    let json = serde_json::to_value(&state).unwrap();
    assert_eq!(
        json["trafficBreakdown"]["assets"]["chunk.original"]["msgs"],
        2
    );
    assert_eq!(json["trafficBreakdown"]["assets"]["get.request"]["msgs"], 1);
    assert!(json["trafficBreakdown"]["kinds"]["out:asset.chunk"]["bytes"].is_u64());
}

fn source(n: usize) -> AssetSource {
    AssetSource {
        asset_id: format!("id{n}"),
        path: format!("/p/{n}.png").into(),
        name: format!("{n}.png"),
        mime: "image/png".into(),
        size: 1,
        sha256: Some(format!("{n:064x}")),
    }
}

#[test]
fn what_was_sent_in_full_is_remembered_per_phone_content_and_kind() {
    let mut sent = SentChunks::default();
    let a = source(1);
    let key =
        |phone: &str, source: &AssetSource, preview: bool| SentKey::new(phone, source, preview);

    // Windows that follow one another make the whole; only then is the next fetch a repeat.
    assert!(!sent.window(key("p1", &a, false), 0, 8, 19));
    assert!(!sent.window(key("p1", &a, false), 0, 8, 19), "asked again");
    assert!(!sent.window(key("p1", &a, false), 8, 16, 19));
    assert!(
        !sent.window(key("p1", &a, false), 16, 19, 19),
        "the last window of the first fetch"
    );
    assert!(
        sent.window(key("p1", &a, false), 0, 8, 19),
        "now all of it has gone"
    );
    assert!(
        sent.window(key("p1", &a, false), 8, 16, 19),
        "and the rest of the new fetch with it"
    );

    // Another phone, the preview of the same content, and other content, each start over.
    assert!(!sent.window(key("p2", &a, false), 0, 8, 19));
    assert!(!sent.window(key("p1", &a, true), 0, 1, 1));
    assert!(sent.window(key("p1", &a, true), 0, 1, 1));
    assert!(!sent.window(key("p1", &source(2), false), 0, 8, 19));
}

#[test]
fn a_window_that_skips_ahead_does_not_make_the_content_complete() {
    let mut sent = SentChunks::default();
    let a = source(1);
    // Something asked for the last window without the ones before it: the phone does not have the content.
    assert!(!sent.window(SentKey::new("p1", &a, false), 16, 19, 19));
    assert!(!sent.window(SentKey::new("p1", &a, false), 0, 8, 19));
}

#[test]
fn content_that_is_not_the_size_it_was_starts_over() {
    let mut sent = SentChunks::default();
    let a = source(1);
    assert!(!sent.window(SentKey::new("p1", &a, false), 0, 3, 3));
    assert!(sent.window(SentKey::new("p1", &a, false), 0, 3, 3));
    // The same name for something of another length (an id the host gave out again) is not what was sent.
    assert!(!sent.window(SentKey::new("p1", &a, false), 0, 4, 4));
}

#[test]
fn the_memory_is_bounded_and_forgets_the_oldest_first() {
    let mut sent = SentChunks::default();
    for n in 0..700 {
        sent.window(SentKey::new("p1", &source(n), false), 0, 1, 1);
    }
    assert_eq!(sent.len(), 512);
    assert!(
        !sent.window(SentKey::new("p1", &source(0), false), 0, 1, 1),
        "the oldest is forgotten, so this is a first time again"
    );
    assert!(
        sent.window(SentKey::new("p1", &source(699), false), 0, 1, 1),
        "the newest is not"
    );
    assert_eq!(sent.len(), 512, "still bounded after being asked about");
}
