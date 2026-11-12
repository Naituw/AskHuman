use std::sync::Arc;

use sha2::{Digest, Sha256};

use super::requests::{DeliverRequest, Resolution, Sink};
use super::testkit::*;
use super::*;
use crate::ios::views::AssetSource;
use crate::ios::wire::{self, t, AnswerItem, ASSET_CHUNK_BYTES, MAX_ASSET_BYTES};

fn bytes(len: usize, seed: u8) -> Vec<u8> {
    (0..len)
        .map(|i| (i as u8).wrapping_mul(31).wrapping_add(seed))
        .collect()
}

fn sha_hex(data: &[u8]) -> String {
    Sha256::digest(data)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn deliver_with_assets(fx: &Fixture, id: &str, assets: Vec<AssetSource>) -> Arc<FakeAsk> {
    let (ask, view) = sample_ask(id);
    let sink = FakeAsk::new();
    fx.gateway.deliver(DeliverRequest {
        view,
        sink: Sink::Ask(sink.clone()),
        ask: Some(ask),
        deadline_ms: None,
        assets,
    });
    sink
}

fn asset_file(dir: &std::path::Path, name: &str, data: &[u8]) -> AssetSource {
    let path = dir.join(name);
    std::fs::write(&path, data).unwrap();
    AssetSource {
        asset_id: crypto::new_id(),
        path,
        name: name.to_string(),
        mime: "application/octet-stream".into(),
        size: data.len() as u64,
    }
}

fn get(asset_id: &str, from: u64, count: u64) -> wire::AssetGet {
    wire::AssetGet {
        asset_id: asset_id.into(),
        from,
        count,
    }
}

async fn recv_chunks(phone: &mut TestPhone, n: usize) -> Vec<wire::AssetChunk> {
    let mut chunks = Vec::new();
    for _ in 0..n {
        chunks.push(phone.recv_body(t::ASSET_CHUNK).await);
    }
    chunks
}

// ---------------------------------------------------------------- downloads

#[tokio::test(start_paused = true)]
async fn an_attachment_is_downloaded_in_chunks_and_verifies() {
    let fx = Fixture::new().await;
    let mut phone = fx.live_phone().await;
    let data = bytes(ASSET_CHUNK_BYTES * 2 + 5_000, 1);
    let asset = asset_file(fx.dir.path(), "report.bin", &data);
    deliver_with_assets(&fx, "req1", vec![asset.clone()]);
    phone.recv(t::REQUEST_NEW).await;

    phone.send(&get(&asset.asset_id, 0, 8)).await;
    let chunks = recv_chunks(&mut phone, 3).await;
    assert_eq!(
        chunks.iter().map(|c| c.index).collect::<Vec<_>>(),
        [0, 1, 2]
    );
    assert!(chunks.iter().all(|c| c.total == 3));
    assert!(chunks.iter().all(|c| c.size == data.len() as u64));
    assert_eq!(chunks[0].sha256.as_deref(), Some(sha_hex(&data).as_str()));
    assert!(chunks[1].sha256.is_none() && chunks[2].sha256.is_none());
    let mut joined = Vec::new();
    for chunk in &chunks {
        joined.extend(crypto::b64u_decode(&chunk.data).unwrap());
    }
    assert_eq!(joined, data);
}

#[tokio::test(start_paused = true)]
async fn a_window_returns_only_the_requested_chunks() {
    let fx = Fixture::new().await;
    let mut phone = fx.live_phone().await;
    let data = bytes(ASSET_CHUNK_BYTES * 3, 2);
    let asset = asset_file(fx.dir.path(), "big.bin", &data);
    deliver_with_assets(&fx, "req1", vec![asset.clone()]);
    phone.recv(t::REQUEST_NEW).await;

    phone.send(&get(&asset.asset_id, 1, 1)).await;
    let chunk: wire::AssetChunk = phone.recv_body(t::ASSET_CHUNK).await;
    assert_eq!((chunk.index, chunk.total), (1, 3));
    assert!(chunk.sha256.is_none());
    assert!(
        !phone
            .gets(t::ASSET_CHUNK, std::time::Duration::from_secs(1))
            .await
    );
}

#[tokio::test(start_paused = true)]
async fn unknown_oversized_and_finished_assets_are_refused() {
    let fx = Fixture::new().await;
    let mut phone = fx.live_phone().await;
    let mut huge = asset_file(fx.dir.path(), "huge.bin", b"x");
    huge.size = MAX_ASSET_BYTES + 1;
    let small = asset_file(fx.dir.path(), "small.bin", b"hello");
    deliver_with_assets(&fx, "req1", vec![huge.clone(), small.clone()]);
    phone.recv(t::REQUEST_NEW).await;

    for (asset_id, reason) in [
        (crypto::new_id(), "notFound"),
        (huge.asset_id.clone(), "tooLarge"),
    ] {
        phone.send(&get(&asset_id, 0, 1)).await;
        let error: wire::AssetError = phone.recv_body(t::ASSET_ERROR).await;
        assert_eq!((error.asset_id, error.reason.as_str()), (asset_id, reason));
    }

    // Once the request is over its files are no longer served.
    fx.gateway.resolve("req1", Resolution::Expired);
    phone.recv(t::REQUEST_RESOLVED).await;
    phone.send(&get(&small.asset_id, 0, 1)).await;
    let error: wire::AssetError = phone.recv_body(t::ASSET_ERROR).await;
    assert_eq!(error.reason, "notFound");
}

#[tokio::test(start_paused = true)]
async fn a_file_that_disappeared_is_not_found() {
    let fx = Fixture::new().await;
    let mut phone = fx.live_phone().await;
    let asset = asset_file(fx.dir.path(), "gone.bin", b"data");
    deliver_with_assets(&fx, "req1", vec![asset.clone()]);
    phone.recv(t::REQUEST_NEW).await;
    std::fs::remove_file(&asset.path).unwrap();

    phone.send(&get(&asset.asset_id, 0, 1)).await;
    let error: wire::AssetError = phone.recv_body(t::ASSET_ERROR).await;
    assert_eq!(error.reason, "notFound");
}

// ---------------------------------------------------------------- uploads

fn begin(
    id: &str,
    name: &str,
    mime: &str,
    data: &[u8],
    request_id: Option<&str>,
) -> wire::UploadBegin {
    wire::UploadBegin {
        upload_id: id.into(),
        name: name.into(),
        mime: mime.into(),
        size: data.len() as u64,
        chunks: (data.len() as u64)
            .div_ceil(ASSET_CHUNK_BYTES as u64)
            .max(1),
        sha256: sha_hex(data),
        scope: wire::UploadScope {
            task_id: None,
            request_id: request_id.map(str::to_string),
            agent_key: None,
        },
    }
}

fn chunk(id: &str, index: u64, data: &[u8]) -> wire::UploadChunk {
    wire::UploadChunk {
        upload_id: id.into(),
        index,
        data: crypto::b64u(data),
    }
}

async fn ack(phone: &mut TestPhone) -> wire::UploadAck {
    phone.recv_body(t::UPLOAD_ACK).await
}

/// Uploads `data` completely and returns the final acknowledgement.
async fn upload(
    phone: &mut TestPhone,
    id: &str,
    name: &str,
    mime: &str,
    data: &[u8],
    request_id: &str,
) -> wire::UploadAck {
    phone
        .send(&begin(id, name, mime, data, Some(request_id)))
        .await;
    assert_eq!(ack(phone).await.status, "ok");
    for (index, piece) in data.chunks(ASSET_CHUNK_BYTES).enumerate() {
        phone.send(&chunk(id, index as u64, piece)).await;
    }
    loop {
        let reply = ack(phone).await;
        if reply.status != "ok" {
            return reply;
        }
    }
}

fn with_attachments(q: usize, images: &[&str], files: &[&str]) -> AnswerItem {
    AnswerItem {
        q,
        sel: vec![0],
        text: Some("see attached".into()),
        images: images.iter().map(|s| s.to_string()).collect(),
        files: files.iter().map(|s| s.to_string()).collect(),
    }
}

fn submit(client_msg_id: &str, request_id: &str, items: Vec<AnswerItem>) -> wire::AnswerSubmit {
    wire::AnswerSubmit {
        client_msg_id: client_msg_id.into(),
        request_id: request_id.into(),
        action: "send".into(),
        answers: items,
        todo_ids: Vec::new(),
        directive: None,
        annotations: Vec::new(),
    }
}

#[tokio::test(start_paused = true)]
async fn uploads_become_images_and_files_of_the_answer() {
    let fx = Fixture::new().await;
    let mut phone = fx.live_phone().await;
    let sink = deliver_with_assets(&fx, "req1", Vec::new());
    phone.recv(t::REQUEST_NEW).await;

    let image = bytes(ASSET_CHUNK_BYTES * 4 + 777, 3);
    let image_id = crypto::new_id();
    // Five chunks: acknowledged after the fourth (`ok`), then `complete`.
    phone
        .send(&begin(
            &image_id,
            "photo.png",
            "image/png",
            &image,
            Some("req1"),
        ))
        .await;
    assert_eq!(ack(&mut phone).await.status, "ok");
    for (index, piece) in image.chunks(ASSET_CHUNK_BYTES).enumerate() {
        phone.send(&chunk(&image_id, index as u64, piece)).await;
    }
    let mid = ack(&mut phone).await;
    assert_eq!((mid.status.as_str(), mid.next_index), ("ok", 4));
    let done = ack(&mut phone).await;
    assert_eq!((done.status.as_str(), done.next_index), ("complete", 5));

    let notes = b"remember the milk".to_vec();
    let file_id = crypto::new_id();
    let done = upload(
        &mut phone,
        &file_id,
        "notes.txt",
        "text/plain",
        &notes,
        "req1",
    )
    .await;
    assert_eq!(done.status, "complete");

    phone
        .send(&submit(
            "c1",
            "req1",
            vec![with_attachments(0, &[&image_id], &[&file_id])],
        ))
        .await;
    let reply: wire::AnswerAck = phone.recv_body(t::ANSWER_ACK).await;
    assert_eq!(reply.status, "accepted", "{:?}", reply.reason);

    let results = sink.results.lock().unwrap();
    let answer = &results[0].answers[0];
    use base64::Engine;
    assert_eq!(answer.images.len(), 1);
    assert_eq!(answer.images[0].media_type, "image/png");
    assert_eq!(answer.images[0].filename.as_deref(), Some("photo.png"));
    assert_eq!(
        base64::engine::general_purpose::STANDARD
            .decode(&answer.images[0].data)
            .unwrap(),
        image
    );
    assert_eq!(answer.files.len(), 1);
    assert_eq!(std::fs::read(&answer.files[0]).unwrap(), notes);
    assert!(answer.files[0].ends_with("notes.txt"));
    // The image travelled inside the answer, so its staged copy is gone; the file stays for the
    // Agent to read.
    assert!(!fx.dir.path().join("uploads").join(&image_id).exists());
    assert!(fx.dir.path().join("uploads").join(&file_id).exists());
}

#[tokio::test(start_paused = true)]
async fn a_corrupted_upload_is_rejected_and_removed() {
    let fx = Fixture::new().await;
    let mut phone = fx.live_phone().await;
    deliver_with_assets(&fx, "req1", Vec::new());
    phone.recv(t::REQUEST_NEW).await;

    let data = bytes(1_000, 4);
    let id = crypto::new_id();
    let mut declared = begin(
        &id,
        "a.bin",
        "application/octet-stream",
        &data,
        Some("req1"),
    );
    declared.sha256 = sha_hex(b"something else");
    phone.send(&declared).await;
    assert_eq!(ack(&mut phone).await.status, "ok");
    phone.send(&chunk(&id, 0, &data)).await;
    let reply = ack(&mut phone).await;
    assert_eq!(
        (reply.status.as_str(), reply.reason.as_deref()),
        ("error", Some("checksum"))
    );
    assert!(!fx.dir.path().join("uploads").join(&id).exists());
    // The upload is gone: referencing it fails the answer.
    phone
        .send(&submit(
            "c1",
            "req1",
            vec![with_attachments(0, &[], &[&id])],
        ))
        .await;
    let reply: wire::AnswerAck = phone.recv_body(t::ANSWER_ACK).await;
    assert_eq!(reply.status, "invalid");
}

#[tokio::test(start_paused = true)]
async fn upload_begin_is_validated() {
    let fx = Fixture::new().await;
    let mut phone = fx.live_phone().await;
    deliver_with_assets(&fx, "req1", Vec::new());
    phone.recv(t::REQUEST_NEW).await;
    let data = bytes(100, 5);

    let mut cases: Vec<(wire::UploadBegin, &str)> = Vec::new();
    cases.push((
        begin(&crypto::new_id(), "a", "image/png", &data, Some("unknown")),
        "gone",
    ));
    cases.push((
        begin(&crypto::new_id(), "a", "image/png", &data, None),
        "invalid",
    ));
    cases.push((
        begin("not-an-id", "a", "image/png", &data, Some("req1")),
        "invalid",
    ));
    let mut too_big = begin(&crypto::new_id(), "a", "image/png", &data, Some("req1"));
    too_big.size = MAX_ASSET_BYTES + 1;
    cases.push((too_big, "tooLarge"));
    let mut wrong_chunks = begin(&crypto::new_id(), "a", "image/png", &data, Some("req1"));
    wrong_chunks.chunks = 7;
    cases.push((wrong_chunks, "invalid"));
    let mut bad_hash = begin(&crypto::new_id(), "a", "image/png", &data, Some("req1"));
    bad_hash.sha256 = "zz".into();
    cases.push((bad_hash, "invalid"));

    for (case, reason) in cases {
        phone.send(&case).await;
        let reply = ack(&mut phone).await;
        assert_eq!(
            (reply.status.as_str(), reply.reason.as_deref()),
            ("error", Some(reason)),
            "{}",
            case.upload_id
        );
    }
}

#[tokio::test(start_paused = true)]
async fn at_most_five_attachments_per_request() {
    let fx = Fixture::new().await;
    let mut phone = fx.live_phone().await;
    deliver_with_assets(&fx, "req1", Vec::new());
    phone.recv(t::REQUEST_NEW).await;
    let data = bytes(10, 6);
    for _ in 0..5 {
        phone
            .send(&begin(
                &crypto::new_id(),
                "a",
                "text/plain",
                &data,
                Some("req1"),
            ))
            .await;
        assert_eq!(ack(&mut phone).await.status, "ok");
    }
    phone
        .send(&begin(
            &crypto::new_id(),
            "a",
            "text/plain",
            &data,
            Some("req1"),
        ))
        .await;
    let reply = ack(&mut phone).await;
    assert_eq!(reply.reason.as_deref(), Some("tooMany"));
}

#[tokio::test(start_paused = true)]
async fn out_of_order_chunks_report_where_to_continue() {
    let fx = Fixture::new().await;
    let mut phone = fx.live_phone().await;
    deliver_with_assets(&fx, "req1", Vec::new());
    phone.recv(t::REQUEST_NEW).await;

    let data = bytes(ASSET_CHUNK_BYTES * 2, 7);
    let id = crypto::new_id();
    phone
        .send(&begin(
            &id,
            "a.bin",
            "application/octet-stream",
            &data,
            Some("req1"),
        ))
        .await;
    assert_eq!(ack(&mut phone).await.status, "ok");
    // Chunk 1 arrives before chunk 0.
    phone.send(&chunk(&id, 1, &data[ASSET_CHUNK_BYTES..])).await;
    let reply = ack(&mut phone).await;
    assert_eq!((reply.status.as_str(), reply.next_index), ("ok", 0));
    // A repeated begin (lost ack) reports the same position.
    phone
        .send(&begin(
            &id,
            "a.bin",
            "application/octet-stream",
            &data,
            Some("req1"),
        ))
        .await;
    assert_eq!(ack(&mut phone).await.next_index, 0);
    phone.send(&chunk(&id, 0, &data[..ASSET_CHUNK_BYTES])).await;
    phone.send(&chunk(&id, 1, &data[ASSET_CHUNK_BYTES..])).await;
    assert_eq!(ack(&mut phone).await.status, "complete");
}

#[tokio::test(start_paused = true)]
async fn a_rejected_answer_keeps_its_uploads_for_the_retry() {
    let fx = Fixture::new().await;
    let mut phone = fx.live_phone().await;
    let sink = deliver_with_assets(&fx, "req1", Vec::new());
    phone.recv(t::REQUEST_NEW).await;
    let notes = bytes(300, 8);
    let id = crypto::new_id();
    assert_eq!(
        upload(&mut phone, &id, "n.txt", "text/plain", &notes, "req1")
            .await
            .status,
        "complete"
    );

    // Question 9 does not exist: the answer is invalid and must not consume the upload.
    let mut broken = with_attachments(9, &[], &[&id]);
    broken.sel.clear();
    phone.send(&submit("c1", "req1", vec![broken])).await;
    let reply: wire::AnswerAck = phone.recv_body(t::ANSWER_ACK).await;
    assert_eq!(reply.status, "invalid");
    assert!(sink.results.lock().unwrap().is_empty());

    phone
        .send(&submit(
            "c2",
            "req1",
            vec![with_attachments(0, &[], &[&id])],
        ))
        .await;
    let reply: wire::AnswerAck = phone.recv_body(t::ANSWER_ACK).await;
    assert_eq!(reply.status, "accepted", "{:?}", reply.reason);
}

#[tokio::test(start_paused = true)]
async fn another_phone_cannot_use_an_upload() {
    let fx = Fixture::new().await;
    let mut owner = fx.live_phone().await;
    let mut other = fx.live_phone().await;
    deliver_with_assets(&fx, "req1", Vec::new());
    owner.recv(t::REQUEST_NEW).await;
    other.recv(t::REQUEST_NEW).await;
    let data = bytes(50, 9);
    let id = crypto::new_id();
    assert_eq!(
        upload(&mut owner, &id, "a.txt", "text/plain", &data, "req1")
            .await
            .status,
        "complete"
    );

    other
        .send(&submit(
            "c1",
            "req1",
            vec![with_attachments(0, &[], &[&id])],
        ))
        .await;
    let reply: wire::AnswerAck = other.recv_body(t::ANSWER_ACK).await;
    assert_eq!(reply.status, "invalid");
    // Nor can it feed chunks into somebody else's transfer.
    other.send(&chunk(&id, 0, &data)).await;
    let reply = ack(&mut other).await;
    assert_eq!(reply.reason.as_deref(), Some("gone"));
}

#[tokio::test(start_paused = true)]
async fn uploads_of_finished_requests_and_idle_uploads_are_cleaned_up() {
    let fx = Fixture::new().await;
    let mut phone = fx.live_phone().await;
    deliver_with_assets(&fx, "req1", Vec::new());
    phone.recv(t::REQUEST_NEW).await;
    let data = bytes(50, 10);
    let id = crypto::new_id();
    assert_eq!(
        upload(&mut phone, &id, "a.txt", "text/plain", &data, "req1")
            .await
            .status,
        "complete"
    );
    assert!(fx.dir.path().join("uploads").join(&id).exists());

    // The request ends (answered elsewhere): its attachments are no longer needed.
    fx.gateway
        .resolve("req1", Resolution::AnsweredElsewhere("Popup".into()));
    phone.recv(t::REQUEST_RESOLVED).await;
    fx.settle().await;
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    assert!(!fx.dir.path().join("uploads").join(&id).exists());
}

#[tokio::test(start_paused = true)]
async fn revoking_a_phone_discards_its_uploads() {
    let fx = Fixture::new().await;
    let mut phone = fx.live_phone().await;
    deliver_with_assets(&fx, "req1", Vec::new());
    phone.recv(t::REQUEST_NEW).await;
    let data = bytes(50, 11);
    let id = crypto::new_id();
    assert_eq!(
        upload(&mut phone, &id, "a.txt", "text/plain", &data, "req1")
            .await
            .status,
        "complete"
    );
    assert!(fx.gateway.revoke(&phone.phone_id).await);
    assert!(!fx.dir.path().join("uploads").join(&id).exists());
}

#[tokio::test(start_paused = true)]
async fn upload_file_names_cannot_escape_their_directory() {
    let fx = Fixture::new().await;
    let mut phone = fx.live_phone().await;
    let sink = deliver_with_assets(&fx, "req1", Vec::new());
    phone.recv(t::REQUEST_NEW).await;
    let data = bytes(20, 12);
    let id = crypto::new_id();
    assert_eq!(
        upload(
            &mut phone,
            &id,
            "../../etc/.passwd",
            "text/plain",
            &data,
            "req1"
        )
        .await
        .status,
        "complete"
    );
    phone
        .send(&submit(
            "c1",
            "req1",
            vec![with_attachments(0, &[], &[&id])],
        ))
        .await;
    let reply: wire::AnswerAck = phone.recv_body(t::ANSWER_ACK).await;
    assert_eq!(reply.status, "accepted");
    let path = sink.results.lock().unwrap()[0].answers[0].files[0].clone();
    let expected = fx.dir.path().join("uploads").join(&id).join("passwd");
    assert_eq!(std::path::PathBuf::from(path), expected);
}

#[tokio::test(start_paused = true)]
async fn an_upload_resumes_at_its_saved_chunk_after_a_daemon_handoff() {
    let fx = Fixture::new().await;
    let mut phone = fx.live_phone().await;
    deliver_with_assets(&fx, "req1", Vec::new());
    phone.recv(t::REQUEST_NEW).await;
    let data = bytes(ASSET_CHUNK_BYTES * 5 + 17, 23);
    let id = crypto::new_id();
    let begin = begin(
        &id,
        "resume.bin",
        "application/octet-stream",
        &data,
        Some("req1"),
    );
    phone.send(&begin).await;
    assert_eq!(ack(&mut phone).await.next_index, 0);
    for (index, piece) in data.chunks(ASSET_CHUNK_BYTES).take(4).enumerate() {
        phone.send(&chunk(&id, index as u64, piece)).await;
    }
    assert_eq!(ack(&mut phone).await.next_index, 4);
    fx.host.pending_ios.lock().unwrap().push("req1".into());
    let fx = fx.handoff().await;
    let sink = deliver_with_assets(&fx, "req1", Vec::new());
    phone.clear_queue();
    phone.hello().await;
    phone.send(&begin).await;
    assert_eq!(ack(&mut phone).await.next_index, 4);
    for (index, piece) in data.chunks(ASSET_CHUNK_BYTES).enumerate().skip(4) {
        phone.send(&chunk(&id, index as u64, piece)).await;
    }
    assert_eq!(ack(&mut phone).await.status, "complete");
    let fx = fx.handoff().await;
    let _ = sink;
    let sink = deliver_with_assets(&fx, "req1", Vec::new());
    phone.clear_queue();
    phone.hello().await;
    phone.send(&begin).await;
    assert_eq!(ack(&mut phone).await.status, "complete");
    phone
        .send(&submit(
            "restored-upload",
            "req1",
            vec![with_attachments(0, &[], &[&id])],
        ))
        .await;
    let result: wire::AnswerAck = phone.recv_body(t::ANSWER_ACK).await;
    assert_eq!(result.status, "accepted");
    let results = sink.results.lock().unwrap();
    assert_eq!(
        std::fs::read(&results[0].answers[0].files[0]).unwrap(),
        data
    );
}

#[tokio::test(start_paused = true)]
async fn an_uncommitted_submission_keeps_its_upload_for_the_same_submission_retry() {
    let fx = Fixture::new().await;
    let mut phone = fx.live_phone().await;
    let sink = deliver_with_assets(&fx, "req1", Vec::new());
    phone.recv(t::REQUEST_NEW).await;
    let data = b"keep until accepted";
    let id = crypto::new_id();
    upload(&mut phone, &id, "keep.txt", "text/plain", data, "req1").await;
    let submission = submit("same-retry", "req1", vec![with_attachments(0, &[], &[&id])]);
    sink.unavailable
        .store(true, std::sync::atomic::Ordering::SeqCst);
    phone.send(&submission).await;
    assert!(
        !phone
            .gets(t::ANSWER_ACK, std::time::Duration::from_millis(500))
            .await
    );
    assert!(sink.results.lock().unwrap().is_empty());
    sink.unavailable
        .store(false, std::sync::atomic::Ordering::SeqCst);
    phone.send(&submission).await;
    let ack: wire::AnswerAck = phone.recv_body(t::ANSWER_ACK).await;
    assert_eq!(ack.status, "accepted");
    assert_eq!(ack.client_msg_id, "same-retry");
    let results = sink.results.lock().unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(
        std::fs::read(&results[0].answers[0].files[0]).unwrap(),
        data
    );
}

// ---------------------------------------------------------------- pictures of the history

/// A picture of the history that the scripted host finds by its asset id.
fn history_picture(fx: &Fixture, name: &str, data: &[u8]) -> AssetSource {
    let source = asset_file(fx.dir.path(), name, data);
    fx.host
        .history_pictures
        .lock()
        .unwrap()
        .insert(source.asset_id.clone(), source.clone());
    source
}

fn asked_of_the_host(fx: &Fixture) -> Vec<String> {
    fx.host.history_picture_calls.lock().unwrap().clone()
}

async fn fetch_first_chunk(phone: &mut TestPhone, asset_id: &str) {
    phone.send(&get(asset_id, 0, 1)).await;
    let _: wire::AssetChunk = phone.recv_body(t::ASSET_CHUNK).await;
}

#[tokio::test(start_paused = true)]
async fn a_picture_of_the_history_is_downloaded_through_the_host_and_remembered() {
    let fx = Fixture::new().await;
    let mut phone = fx.live_phone().await;
    let data = bytes(ASSET_CHUNK_BYTES + 1_000, 3);
    let picture = history_picture(&fx, "shot.png", &data);

    // No request is pending, so the id is not one of a request's files: the host is asked.
    phone.send(&get(&picture.asset_id, 0, 1)).await;
    let first: wire::AssetChunk = phone.recv_body(t::ASSET_CHUNK).await;
    assert_eq!(
        (first.index, first.total, first.size),
        (0, 2, data.len() as u64)
    );
    assert_eq!(first.sha256.as_deref(), Some(sha_hex(&data).as_str()));

    phone.send(&get(&picture.asset_id, 1, 1)).await;
    let second: wire::AssetChunk = phone.recv_body(t::ASSET_CHUNK).await;
    assert_eq!(second.index, 1);
    assert!(second.sha256.is_none());
    let mut joined = crypto::b64u_decode(&first.data).unwrap();
    joined.extend(crypto::b64u_decode(&second.data).unwrap());
    assert_eq!(joined, data);

    // The host looked for the file once; the second window came from the gateway's memory.
    assert_eq!(asked_of_the_host(&fx), vec![picture.asset_id.clone()]);
}

#[tokio::test(start_paused = true)]
async fn a_picture_nobody_knows_is_not_found_and_an_id_of_the_wrong_shape_is_not_even_asked_about()
{
    let fx = Fixture::new().await;
    let mut phone = fx.live_phone().await;
    let stranger = crypto::new_id();
    phone.send(&get(&stranger, 0, 1)).await;
    let error: wire::AssetError = phone.recv_body(t::ASSET_ERROR).await;
    assert_eq!(
        (error.asset_id.as_str(), error.reason.as_str()),
        (stranger.as_str(), "notFound")
    );

    for malformed in ["not-an-id", "", &"a".repeat(80), "../etc/passwd"] {
        phone.send(&get(malformed, 0, 1)).await;
        let error: wire::AssetError = phone.recv_body(t::ASSET_ERROR).await;
        assert_eq!(error.reason, "notFound", "{malformed:?}");
        assert_eq!(error.asset_id, malformed);
    }
    assert_eq!(asked_of_the_host(&fx), [stranger]);
}

#[tokio::test(start_paused = true)]
async fn a_picture_whose_file_vanished_or_is_too_large_is_refused_like_any_asset() {
    let fx = Fixture::new().await;
    let mut phone = fx.live_phone().await;

    let vanished = history_picture(&fx, "vanished.png", b"pixels");
    std::fs::remove_file(&vanished.path).unwrap();
    phone.send(&get(&vanished.asset_id, 0, 1)).await;
    let error: wire::AssetError = phone.recv_body(t::ASSET_ERROR).await;
    assert_eq!(error.reason, "notFound");

    let mut huge = history_picture(&fx, "huge.png", b"x");
    huge.size = MAX_ASSET_BYTES + 1;
    fx.host
        .history_pictures
        .lock()
        .unwrap()
        .insert(huge.asset_id.clone(), huge.clone());
    phone.send(&get(&huge.asset_id, 0, 1)).await;
    let error: wire::AssetError = phone.recv_body(t::ASSET_ERROR).await;
    assert_eq!(error.reason, "tooLarge");
}

#[tokio::test(start_paused = true)]
async fn the_gateway_forgets_a_picture_after_a_while_and_asks_the_host_again() {
    let fx = Fixture::new().await;
    let mut phone = fx.live_phone().await;
    let picture = history_picture(&fx, "shot.png", &bytes(2_000, 4));

    fetch_first_chunk(&mut phone, &picture.asset_id).await;
    tokio::time::sleep(std::time::Duration::from_secs(9 * 60)).await;
    fetch_first_chunk(&mut phone, &picture.asset_id).await;
    assert_eq!(
        asked_of_the_host(&fx).len(),
        1,
        "still remembered after nine minutes"
    );

    tokio::time::sleep(std::time::Duration::from_secs(2 * 60)).await;
    fetch_first_chunk(&mut phone, &picture.asset_id).await;
    assert_eq!(asked_of_the_host(&fx).len(), 2, "forgotten after eleven");
}

#[tokio::test(start_paused = true)]
async fn a_file_of_a_pending_request_is_served_without_asking_the_host() {
    let fx = Fixture::new().await;
    let mut phone = fx.live_phone().await;
    let asset = asset_file(fx.dir.path(), "attached.bin", b"hello");
    deliver_with_assets(&fx, "req1", vec![asset.clone()]);
    phone.recv(t::REQUEST_NEW).await;
    phone.send(&get(&asset.asset_id, 0, 1)).await;
    let chunk: wire::AssetChunk = phone.recv_body(t::ASSET_CHUNK).await;
    assert_eq!(crypto::b64u_decode(&chunk.data).unwrap(), b"hello");
    assert!(asked_of_the_host(&fx).is_empty());
}

#[test]
fn the_gateway_remembers_at_most_a_few_hundred_pictures_and_drops_the_oldest() {
    let source = |n: usize| AssetSource {
        asset_id: format!("id{n}"),
        path: format!("/p/{n}.png").into(),
        name: format!("{n}.png"),
        mime: "image/png".into(),
        size: 1,
    };
    let mut memory = assets::HistoryAssets::default();
    for n in 0..300 {
        memory.insert(source(n), 1_000);
    }
    assert!(memory.get("id0", 1_000).is_none(), "the oldest is gone");
    assert!(memory.get("id43", 1_000).is_none());
    assert!(memory.get("id44", 1_000).is_some());
    assert!(memory.get("id299", 1_000).is_some());
    // Remembering one again does not make room for it by forgetting another.
    memory.insert(source(299), 2_000);
    assert!(memory.get("id44", 2_000).is_some());
    // And a picture is only believed for ten minutes from when it was remembered (again).
    assert!(memory.get("id299", 2_000 + 10 * 60 * 1000).is_some());
    assert!(memory.get("id299", 2_000 + 10 * 60 * 1000 + 1).is_none());
}
