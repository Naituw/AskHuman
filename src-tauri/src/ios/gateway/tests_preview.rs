//! Previews of picture attachments (`docs/specs/ios-attachment-traffic.md` §6.3–§6.5, AT10–AT12, AT17, AT19).
//!
//! A preview is made on a blocking thread, and the clock of a test that has paused time moves on whenever the runtime has
//! nothing to run, which includes the time a blocking thread takes. So these tests run in real time, and the
//! maker they use (`FakePreviews`) is instant unless a test holds it back.

use std::path::Path;
use std::sync::atomic::Ordering;
use std::time::Duration;

use super::requests::{DeliverRequest, Sink};
use super::testkit::*;
use super::*;
use crate::ios::preview::{PREVIEW_MIN_ORIGINAL_BYTES, VARIANT_ORIGINAL};
use crate::ios::views::{mime_for, AssetSource};
use crate::ios::wire::{self, t, ASSET_CHUNK_BYTES};

/// The bytes of a picture file big enough to get a preview.
fn picture_bytes(seed: u8) -> Vec<u8> {
    (0..PREVIEW_MIN_ORIGINAL_BYTES as usize + 1_000)
        .map(|i| (i as u8).wrapping_mul(31).wrapping_add(seed))
        .collect()
}

/// A file a phone may fetch. `hashed` gives it the hash a request records when it is built.
fn source_of(dir: &Path, name: &str, data: &[u8], hashed: bool) -> AssetSource {
    let path = dir.join(name);
    std::fs::write(&path, data).unwrap();
    AssetSource {
        asset_id: crypto::new_id(),
        path,
        name: name.to_string(),
        mime: mime_for(name).to_string(),
        size: data.len() as u64,
        sha256: hashed.then(|| crypto::sha256_hex(data)),
    }
}

fn deliver(fx: &Fixture, id: &str, assets: Vec<AssetSource>) {
    let (ask, view) = sample_ask(id);
    fx.gateway.deliver(DeliverRequest {
        view,
        sink: Sink::Ask(FakeAsk::new()),
        ask: Some(ask),
        deadline_ms: None,
        assets,
    });
}

fn ask_for(asset_id: &str, from: u64, count: u64, variant: Option<&str>) -> wire::AssetGet {
    wire::AssetGet {
        asset_id: asset_id.into(),
        from,
        count,
        variant: variant.map(str::to_string),
    }
}

fn preview_of(asset_id: &str) -> wire::AssetGet {
    ask_for(asset_id, 0, 8, Some("preview"))
}

async fn chunks_of(phone: &mut TestPhone, n: usize) -> Vec<wire::AssetChunk> {
    let mut chunks = Vec::new();
    for _ in 0..n {
        chunks.push(phone.recv_body(t::ASSET_CHUNK).await);
    }
    chunks
}

fn bytes_of(chunks: &[wire::AssetChunk]) -> Vec<u8> {
    chunks
        .iter()
        .flat_map(|chunk| crypto::b64u_decode(&chunk.data).unwrap())
        .collect()
}

fn variants_of(chunks: &[wire::AssetChunk]) -> Vec<Option<&str>> {
    chunks
        .iter()
        .map(|chunk| chunk.variant.as_deref())
        .collect()
}

/// A phone, with a request that has these pictures pending.
async fn with_request(fx: &Fixture, assets: &[AssetSource]) -> TestPhone {
    let mut phone = fx.live_phone().await;
    deliver(fx, "req1", assets.to_vec());
    phone.recv(t::REQUEST_NEW).await;
    phone
}

#[tokio::test]
async fn a_preview_comes_in_chunks_that_describe_the_preview_and_name_it() {
    let fx = Fixture::new().await;
    let original = picture_bytes(1);
    let asset = source_of(fx.dir.path(), "shot.png", &original, true);
    let mut phone = with_request(&fx, std::slice::from_ref(&asset)).await;
    let len = ASSET_CHUNK_BYTES * 2 + 5_000;
    fx.host.previews.make_previews_of(len);
    let preview = FakePreviews::bytes_of(len);

    phone.send(&preview_of(&asset.asset_id)).await;
    let chunks = chunks_of(&mut phone, 3).await;

    assert_eq!(
        chunks.iter().map(|c| c.index).collect::<Vec<_>>(),
        [0, 1, 2]
    );
    assert!(
        chunks.iter().all(|c| c.total == 3 && c.size == len as u64),
        "index, total and size are those of the preview, not of the file: {chunks:?}"
    );
    assert_eq!(
        chunks[0].sha256.as_deref(),
        Some(crypto::sha256_hex(&preview).as_str()),
        "the hash is the preview's"
    );
    assert!(chunks[1].sha256.is_none() && chunks[2].sha256.is_none());
    assert_eq!(variants_of(&chunks), [Some("preview"); 3]);
    assert_eq!(bytes_of(&chunks), preview);
    // The maker was given the file as it is on the disk, once.
    assert_eq!(fx.host.previews.calls.load(Ordering::SeqCst), 1);
    assert_eq!(fx.host.previews.seen.lock().unwrap()[0], original);
}

#[tokio::test]
async fn a_get_without_a_variant_is_the_file_exactly_as_before_even_for_a_picture() {
    let fx = Fixture::new().await;
    let original = picture_bytes(2);
    let asset = source_of(fx.dir.path(), "shot.png", &original, true);
    let mut phone = with_request(&fx, std::slice::from_ref(&asset)).await;

    phone.send(&ask_for(&asset.asset_id, 0, 8, None)).await;
    let mut chunks = Vec::new();
    for _ in 0..3 {
        let message = phone.recv(t::ASSET_CHUNK).await;
        let body = message.b.as_object().unwrap();
        assert!(
            !body.contains_key("variant"),
            "an answer to a request without a variant: {body:?}"
        );
        chunks.push(serde_json::from_value::<wire::AssetChunk>(message.b).unwrap());
    }
    assert_eq!(bytes_of(&chunks), original);
    assert_eq!(
        chunks[0].sha256.as_deref(),
        Some(crypto::sha256_hex(&original).as_str())
    );
    assert_eq!(
        fx.host.previews.calls.load(Ordering::SeqCst),
        0,
        "no preview is made unless asked for"
    );
}

#[tokio::test]
async fn an_unknown_variant_is_answered_with_the_file_and_named_original() {
    let fx = Fixture::new().await;
    let original = picture_bytes(3);
    let asset = source_of(fx.dir.path(), "shot.png", &original, true);
    let mut phone = with_request(&fx, std::slice::from_ref(&asset)).await;

    phone
        .send(&ask_for(&asset.asset_id, 0, 8, Some("thumbnail")))
        .await;
    let chunks = chunks_of(&mut phone, 3).await;
    assert_eq!(variants_of(&chunks), [Some(VARIANT_ORIGINAL); 3]);
    assert_eq!(bytes_of(&chunks), original);
    assert_eq!(fx.host.previews.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn a_file_that_cannot_have_a_preview_is_sent_as_it_is_and_named_original() {
    let fx = Fixture::new().await;
    let big = picture_bytes(4);
    let small = vec![9u8; 1_000];
    let files = [
        source_of(fx.dir.path(), "notes.txt", &big, true),
        source_of(fx.dir.path(), "animation.gif", &big, true),
        source_of(fx.dir.path(), "tiny.png", &small, true),
    ];
    let mut phone = with_request(&fx, &files).await;

    for (file, data) in files.iter().zip([&big, &big, &small]) {
        phone.send(&preview_of(&file.asset_id)).await;
        let count = data.len().div_ceil(ASSET_CHUNK_BYTES);
        let chunks = chunks_of(&mut phone, count).await;
        assert_eq!(
            variants_of(&chunks),
            vec![Some(VARIANT_ORIGINAL); count],
            "{}",
            file.name
        );
        assert_eq!(&bytes_of(&chunks), data, "{}", file.name);
    }
    assert_eq!(fx.host.previews.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn a_mac_that_does_not_declare_previews_sends_the_file() {
    let fx = Fixture::new().await;
    fx.host.preview_supported.store(false, Ordering::SeqCst);
    let original = picture_bytes(5);
    let asset = source_of(fx.dir.path(), "shot.png", &original, true);
    let mut phone = with_request(&fx, std::slice::from_ref(&asset)).await;
    assert!(!phone.hello().await.caps.asset_preview);

    phone.send(&preview_of(&asset.asset_id)).await;
    let chunks = chunks_of(&mut phone, 3).await;
    assert_eq!(variants_of(&chunks), [Some(VARIANT_ORIGINAL); 3]);
    assert_eq!(bytes_of(&chunks), original);
    assert_eq!(fx.host.previews.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn the_hello_ack_declares_previews() {
    let fx = Fixture::new().await;
    let mut phone = fx.pair_phone().await;
    phone.set_presence(true, true).await;
    assert!(phone.hello().await.caps.asset_preview);
}

#[tokio::test]
async fn a_picture_the_maker_gives_up_on_is_sent_as_it_is_and_the_verdict_is_remembered() {
    let fx = Fixture::new().await;
    fx.host.previews.skip_with(ah_preview::Skip::Transparent);
    let original = picture_bytes(6);
    let asset = source_of(fx.dir.path(), "shot.png", &original, true);
    let mut phone = with_request(&fx, std::slice::from_ref(&asset)).await;

    for _ in 0..2 {
        phone.send(&preview_of(&asset.asset_id)).await;
        let chunks = chunks_of(&mut phone, 3).await;
        assert_eq!(variants_of(&chunks), [Some(VARIANT_ORIGINAL); 3]);
        assert_eq!(bytes_of(&chunks), original);
    }
    assert_eq!(
        fx.host.previews.calls.load(Ordering::SeqCst),
        1,
        "the second request did not try again"
    );
}

#[tokio::test]
async fn a_preview_is_remembered_until_the_file_changes() {
    let fx = Fixture::new().await;
    let original = picture_bytes(7);
    // A picture of the history: its file is read as it is, whatever it holds when asked.
    let picture = source_of(fx.dir.path(), "shot.png", &original, false);
    fx.host
        .history_pictures
        .lock()
        .unwrap()
        .insert(picture.asset_id.clone(), picture.clone());
    let mut phone = fx.live_phone().await;
    fx.host.previews.make_previews_of(3_000);

    for _ in 0..2 {
        phone.send(&preview_of(&picture.asset_id)).await;
        let chunk: wire::AssetChunk = phone.recv_body(t::ASSET_CHUNK).await;
        assert_eq!(chunk.variant.as_deref(), Some("preview"));
        assert_eq!(bytes_of(&[chunk]), FakePreviews::bytes_of(3_000));
    }
    assert_eq!(fx.host.previews.calls.load(Ordering::SeqCst), 1);

    // The file is replaced by another one: the preview is made again, from what is there now.
    let mut other = picture_bytes(8);
    other.extend([1, 2, 3]);
    std::fs::write(&picture.path, &other).unwrap();
    phone.send(&preview_of(&picture.asset_id)).await;
    let chunk: wire::AssetChunk = phone.recv_body(t::ASSET_CHUNK).await;
    assert_eq!(chunk.variant.as_deref(), Some("preview"));
    assert_eq!(fx.host.previews.calls.load(Ordering::SeqCst), 2);
    assert_eq!(fx.host.previews.seen.lock().unwrap()[1], other);
}

#[tokio::test]
async fn a_file_that_changed_since_the_request_was_built_is_sent_as_it_is() {
    let fx = Fixture::new().await;
    let built_from = picture_bytes(10);
    let asset = source_of(fx.dir.path(), "shot.png", &built_from, true);
    let mut phone = with_request(&fx, std::slice::from_ref(&asset)).await;
    // Somebody saves another picture under the same name after the request went out.
    let now = picture_bytes(11);
    std::fs::write(&asset.path, &now).unwrap();

    phone.send(&preview_of(&asset.asset_id)).await;
    let chunks = chunks_of(&mut phone, 3).await;
    assert_eq!(
        variants_of(&chunks),
        [Some(VARIANT_ORIGINAL); 3],
        "a preview of other bytes is not this request's picture"
    );
    assert_eq!(
        bytes_of(&chunks),
        now,
        "what is on the disk now is what the phone gets"
    );
    assert_eq!(fx.host.previews.calls.load(Ordering::SeqCst), 1);

    // A request built after the change describes the file as it is now, and gets the preview that was made from it.
    let second = AssetSource {
        asset_id: crypto::new_id(),
        sha256: Some(crypto::sha256_hex(&now)),
        ..asset.clone()
    };
    deliver(&fx, "req2", vec![second.clone()]);
    phone.recv(t::REQUEST_NEW).await;
    phone.send(&preview_of(&second.asset_id)).await;
    let chunk: wire::AssetChunk = phone.recv_body(t::ASSET_CHUNK).await;
    assert_eq!(chunk.variant.as_deref(), Some("preview"));
    assert_eq!(
        fx.host.previews.calls.load(Ordering::SeqCst),
        1,
        "one preview serves every request that describes this file"
    );
}

#[tokio::test]
async fn a_picture_of_the_history_gets_a_preview_too() {
    let fx = Fixture::new().await;
    let original = picture_bytes(12);
    let picture = source_of(fx.dir.path(), "history.png", &original, false);
    fx.host
        .history_pictures
        .lock()
        .unwrap()
        .insert(picture.asset_id.clone(), picture.clone());
    let mut phone = fx.live_phone().await;

    phone.send(&preview_of(&picture.asset_id)).await;
    let chunk: wire::AssetChunk = phone.recv_body(t::ASSET_CHUNK).await;
    assert_eq!(chunk.variant.as_deref(), Some("preview"));
    assert_eq!(chunk.size, 5_000);
    assert_eq!(bytes_of(&[chunk]), FakePreviews::bytes_of(5_000));
}

#[tokio::test]
async fn the_windows_of_a_preview_follow_the_bytes_of_the_preview() {
    let fx = Fixture::new().await;
    let asset = source_of(fx.dir.path(), "shot.png", &picture_bytes(13), true);
    let mut phone = with_request(&fx, std::slice::from_ref(&asset)).await;
    let len = ASSET_CHUNK_BYTES * 2 + 5_000;
    fx.host.previews.make_previews_of(len);
    let preview = FakePreviews::bytes_of(len);

    phone
        .send(&ask_for(&asset.asset_id, 1, 1, Some("preview")))
        .await;
    let middle: wire::AssetChunk = phone.recv_body(t::ASSET_CHUNK).await;
    assert_eq!(
        (middle.index, middle.total, middle.size),
        (1, 3, len as u64)
    );
    assert!(middle.sha256.is_none());
    assert_eq!(middle.variant.as_deref(), Some("preview"));
    assert_eq!(
        bytes_of(&[middle]),
        preview[ASSET_CHUNK_BYTES..ASSET_CHUNK_BYTES * 2]
    );

    // Past the end of the preview (not of the file): an error, like any window past the end.
    phone
        .send(&ask_for(&asset.asset_id, 3, 1, Some("preview")))
        .await;
    let error: wire::AssetError = phone.recv_body(t::ASSET_ERROR).await;
    assert_eq!(error.reason, "internal");
    assert_eq!(fx.host.previews.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn a_missing_picture_is_not_found_for_a_preview_request_too() {
    let fx = Fixture::new().await;
    let asset = source_of(fx.dir.path(), "gone.png", &picture_bytes(14), true);
    let mut phone = with_request(&fx, std::slice::from_ref(&asset)).await;
    std::fs::remove_file(&asset.path).unwrap();

    phone.send(&preview_of(&asset.asset_id)).await;
    let error: wire::AssetError = phone.recv_body(t::ASSET_ERROR).await;
    assert_eq!(error.reason, "notFound");
    assert_eq!(fx.host.previews.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn a_picture_that_is_too_large_is_refused_for_a_preview_request_too() {
    let fx = Fixture::new().await;
    let mut asset = source_of(fx.dir.path(), "huge.png", &picture_bytes(15), true);
    asset.size = wire::MAX_ASSET_BYTES + 1;
    let mut phone = with_request(&fx, std::slice::from_ref(&asset)).await;

    phone.send(&preview_of(&asset.asset_id)).await;
    let error: wire::AssetError = phone.recv_body(t::ASSET_ERROR).await;
    assert_eq!(error.reason, "tooLarge");
    assert_eq!(fx.host.previews.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn phones_asking_for_the_same_picture_together_share_one_preview() {
    let fx = Fixture::new().await;
    let asset = source_of(fx.dir.path(), "shot.png", &picture_bytes(16), true);
    let mut first = fx.live_phone().await;
    let mut second = fx.live_phone().await;
    deliver(&fx, "req1", vec![asset.clone()]);
    first.recv(t::REQUEST_NEW).await;
    second.recv(t::REQUEST_NEW).await;
    fx.host.previews.held.store(true, Ordering::SeqCst);

    first.send(&preview_of(&asset.asset_id)).await;
    fx.host.previews.started(1).await;
    // The second phone asks while the preview is still being made, and so does the first, for another window.
    second.send(&preview_of(&asset.asset_id)).await;
    first
        .send(&ask_for(&asset.asset_id, 0, 1, Some("preview")))
        .await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    fx.host.previews.held.store(false, Ordering::SeqCst);

    let answered = chunks_of(&mut second, 1).await;
    assert_eq!(variants_of(&answered), [Some("preview")]);
    let answered = chunks_of(&mut first, 2).await;
    assert_eq!(variants_of(&answered), [Some("preview"); 2]);
    assert_eq!(
        fx.host.previews.calls.load(Ordering::SeqCst),
        1,
        "the picture was decoded and encoded once"
    );
}

#[tokio::test]
async fn the_gateway_keeps_answering_while_a_preview_is_made() {
    let fx = Fixture::new().await;
    let asset = source_of(fx.dir.path(), "shot.png", &picture_bytes(17), true);
    let mut phone = with_request(&fx, std::slice::from_ref(&asset)).await;
    fx.host.previews.held.store(true, Ordering::SeqCst);

    phone.send(&preview_of(&asset.asset_id)).await;
    fx.host.previews.started(1).await;
    // The preview is not done, and the gateway answers a question of the phone meanwhile.
    phone.send(&wire::Ping { t: 42 }).await;
    let pong: wire::Pong = phone.recv_body(t::PONG).await;
    assert_eq!(pong.t, 42);
    let stranger = crypto::new_id();
    phone.send(&ask_for(&stranger, 0, 1, None)).await;
    let error: wire::AssetError = phone.recv_body(t::ASSET_ERROR).await;
    assert_eq!(error.asset_id, stranger);
    assert!(!phone.gets(t::ASSET_CHUNK, Duration::from_millis(50)).await);

    fx.host.previews.held.store(false, Ordering::SeqCst);
    let chunks = chunks_of(&mut phone, 1).await;
    assert_eq!(variants_of(&chunks), [Some("preview")]);
}

#[tokio::test]
async fn no_more_than_three_previews_are_made_at_once() {
    let fx = Fixture::new().await;
    let files: Vec<AssetSource> = (0..5u8)
        .map(|n| {
            source_of(
                fx.dir.path(),
                &format!("shot{n}.png"),
                &picture_bytes(20 + n),
                true,
            )
        })
        .collect();
    let mut phone = with_request(&fx, &files).await;
    fx.host.previews.held.store(true, Ordering::SeqCst);

    for file in &files {
        phone.send(&preview_of(&file.asset_id)).await;
    }
    fx.host.previews.started(3).await;
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert_eq!(
        fx.host.previews.calls.load(Ordering::SeqCst),
        3,
        "the others wait for their turn"
    );

    fx.host.previews.held.store(false, Ordering::SeqCst);
    let chunks = chunks_of(&mut phone, 5).await;
    assert_eq!(variants_of(&chunks), [Some("preview"); 5]);
    let mut served: Vec<&str> = chunks.iter().map(|c| c.asset_id.as_str()).collect();
    served.sort_unstable();
    let mut asked: Vec<&str> = files.iter().map(|f| f.asset_id.as_str()).collect();
    asked.sort_unstable();
    assert_eq!(served, asked, "every picture got its own preview");
    assert_eq!(fx.host.previews.calls.load(Ordering::SeqCst), 5);
    assert_eq!(fx.host.previews.most_running.load(Ordering::SeqCst), 3);
}

#[tokio::test]
async fn a_preview_goes_to_a_phone_that_takes_binary_as_blob_frames_that_name_the_variant() {
    let fx = Fixture::new().await;
    let asset = source_of(fx.dir.path(), "shot.png", &picture_bytes(30), true);
    let mut phone = fx.live_phone_with(|p| p.binary = true).await;
    deliver(&fx, "req1", vec![asset.clone()]);
    phone.recv(t::REQUEST_NEW).await;
    fx.host.previews.make_previews_of(ASSET_CHUNK_BYTES + 100);

    let mark = phone.received.len();
    phone.send(&preview_of(&asset.asset_id)).await;
    let chunks = chunks_of(&mut phone, 2).await;
    assert_eq!(variants_of(&chunks), [Some("preview"); 2]);
    assert_eq!(
        bytes_of(&chunks),
        FakePreviews::bytes_of(ASSET_CHUNK_BYTES + 100)
    );
    let frames = &phone.received[mark..];
    assert!(
        frames
            .iter()
            .all(|r| r.ty == t::ASSET_CHUNK && r.binary && r.blob),
        "{frames:?}"
    );
    // The bytes of the preview and a few hundred more for each chunk, not of the 400 KB file.
    let on_the_wire: usize = frames.iter().map(|r| r.bytes).sum();
    assert!(
        on_the_wire < ASSET_CHUNK_BYTES + 100 + 2 * 800,
        "{on_the_wire} bytes"
    );
}
