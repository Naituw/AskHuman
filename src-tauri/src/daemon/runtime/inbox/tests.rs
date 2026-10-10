use super::*;
use base64::Engine;
use std::pin::Pin;
use std::task::{Context, Poll};
use tokio::io::{AsyncRead, AsyncWriteExt, ReadBuf};

fn state() -> Arc<ServerState> {
    let mut config = AppConfig::default();
    config.general.popup_prewarm = false;
    Arc::new(ServerState {
        startup_fp: lifecycle::Fingerprint { size: 0, hash: 0 },
        started_at: 0,
        active: AtomicUsize::new(1),
        last_active: Mutex::new(Instant::now()),
        shutdown: Default::default(),
        draining: AtomicBool::new(true),
        registry: RequestRegistry::new(),
        popup_focus: Mutex::new(PopupFocusArbiter::new()),
        popup_inbox: Mutex::new(Default::default()),
        dd_router: Default::default(),
        fs_router: Default::default(),
        tg_router: Default::default(),
        sl_router: Default::default(),
        config: Mutex::new(config),
        update: Mutex::new(Default::default()),
        agents: Arc::new(AgentRegistry::new()),
        interject: crate::agents::interject::InterjectStore::new(),
        grok_bindings: Default::default(),
        agent_subs: Default::default(),
        tray_subs: Default::default(),
        active_channel: Mutex::new(None),
        inbound_listeners: Default::default(),
        warm_pool: Mutex::new(None),
        warm_spawning: AtomicBool::new(false),
        watch: Default::default(),
        select: Default::default(),
        pending_launches: Default::default(),
        gui_focus: Default::default(),
        last_waiting: Default::default(),
        replay: Mutex::new(crate::daemon::ask_dedup::ReplayCache::new()),
    })
}

fn request(
    state: &ServerState,
) -> (
    Arc<RequestEntry>,
    tokio::sync::mpsc::UnboundedReceiver<crate::app::RenderOutcome>,
) {
    let task = serde_json::from_value(serde_json::json!({
        "message": { "text": "isolated IPC regression" },
        "questions": [{ "message": "answer", "predefinedOptions": ["option"] }],
        "isMarkdown": false, "source": "test", "lang": "en",
        "recordHistory": false, "outputFormat": "json"
    }))
    .unwrap();
    let (entry, rx) = state.registry.create(task, None);
    state.popup_inbox.lock().unwrap().enqueue(
        entry.request_id.clone(),
        entry.seq,
        InteractionEntry::Ask(entry.clone()),
    );
    (entry, rx)
}

struct Observed<R> {
    reader: R,
    consumed: Option<tokio::sync::oneshot::Sender<()>>,
}
impl<R: AsyncRead + Unpin> AsyncRead for Observed<R> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        let before = buf.filled().len();
        let result = Pin::new(&mut self.reader).poll_read(cx, buf);
        if buf.filled().len() > before {
            if let Some(consumed) = self.consumed.take() {
                let _ = consumed.send(());
            }
        }
        result
    }
}

async fn next<R: tokio::io::AsyncBufRead + Unpin>(reader: &mut ipc::MessageReader<R>) -> ServerMsg {
    tokio::time::timeout(Duration::from_secs(2), reader.read())
        .await
        .unwrap()
        .unwrap()
        .unwrap()
}
async fn ack<R: tokio::io::AsyncBufRead + Unpin>(
    reader: &mut ipc::MessageReader<R>,
    id: &str,
) -> String {
    loop {
        if let ServerMsg::PopupSubmissionAck {
            request_id,
            winner,
            error,
        } = next(reader).await
        {
            assert_eq!(request_id, id);
            assert!(error.is_none());
            return winner.unwrap();
        }
    }
}

async fn fragmented_answer(external_terminal: bool) {
    let state = state();
    let (first, mut first_result) = request(&state);
    let (second, mut second_result) = request(&state);
    let (other, _other_result) = request(&state);
    let lease = {
        let mut host = state.popup_inbox.lock().unwrap();
        let mut lease = host.start().unwrap();
        // A failing regression must not launch a child process as part of recovery.
        for _ in 0..2 {
            lease = match host.disconnected(lease.generation, Instant::now(), true) {
                Recovery::Restart(lease) => lease,
                _ => panic!("expected reserved recovery lease"),
            };
        }
        lease
    };
    let generation = lease.generation;
    let (server, client) = tokio::io::duplex(8192);
    let (read, write) = tokio::io::split(server);
    let (consumed_tx, consumed_rx) = tokio::sync::oneshot::channel();
    let reader = BufReader::new(Observed {
        reader: read,
        consumed: Some(consumed_tx),
    });
    let run_state = state.clone();
    let handler =
        tokio::spawn(
            async move { handle_popup_host(lease.token, reader, write, &run_state).await },
        );
    let (read, mut write) = tokio::io::split(client);
    let mut reader = ipc::MessageReader::new(BufReader::new(read));
    assert!(
        matches!(next(&mut reader).await, ServerMsg::PopupHostAccepted { generation: g, .. } if g == generation)
    );
    while !matches!(next(&mut reader).await, ServerMsg::UpdateState { .. }) {}

    let mut image = include_bytes!("../../../../icons/32x32.png").to_vec();
    image.resize(328_082, 0);
    let answer = ClientMsg::Answer {
        request_id: first.request_id.clone(),
        action: ChannelAction::Send,
        answers: vec![crate::models::QuestionAnswer {
            selected_options: vec!["option".into()],
            user_input: Some("typed 中文 text".into()),
            images: vec![crate::models::ImageAttachment {
                data: base64::engine::general_purpose::STANDARD.encode(&image),
                media_type: "image/png".into(),
                filename: Some("screenshot.png".into()),
            }],
            ..Default::default()
        }],
    };
    let mut bytes = serde_json::to_vec(&answer).unwrap();
    bytes.push(b'\n');
    write.write_all(&bytes[..1024]).await.unwrap();
    consumed_rx.await.unwrap();
    if external_terminal {
        assert!(other
            .coordinator
            .try_submit(ChannelResult::cancel("feishu")));
    }
    tokio::time::advance(Duration::from_millis(350)).await;
    tokio::task::yield_now().await;
    if external_terminal {
        loop {
            let ServerMsg::Cancel { request_id, winner } = next(&mut reader).await else {
                panic!("expected the external terminal to cancel its popup");
            };
            assert_eq!(request_id, other.request_id);
            if winner == "feishu" {
                break;
            }
        }
        assert!(state
            .popup_inbox
            .lock()
            .unwrap()
            .get(&other.request_id)
            .is_none());
    }
    write.write_all(&bytes[1024..]).await.unwrap();
    assert_eq!(ack(&mut reader, &first.request_id).await, "popup");
    let result = first_result.recv().await.unwrap();
    assert_eq!(result.exit_code, 0);
    let json: serde_json::Value = serde_json::from_str(&result.stdout).unwrap();
    assert_eq!(json["answers"][0]["user_input"], "typed 中文 text");
    assert_eq!(json["answers"][0]["selected_options"][0], "option");
    let path = json["answers"][0]["files"][0].as_str().unwrap();
    assert_eq!(std::fs::read(path).unwrap(), image);
    assert_eq!(
        state.popup_inbox.lock().unwrap().generation(),
        Some(generation)
    );

    // A duplicate cannot win again, and the next request still uses the same connection.
    ipc::write_msg(&mut write, &answer).await.unwrap();
    assert_eq!(ack(&mut reader, &first.request_id).await, "system");
    assert!(first_result.try_recv().is_err());
    ipc::write_msg(
        &mut write,
        &ClientMsg::Answer {
            request_id: second.request_id.clone(),
            action: ChannelAction::Send,
            answers: vec![crate::models::QuestionAnswer {
                user_input: Some("second".into()),
                ..Default::default()
            }],
        },
    )
    .await
    .unwrap();
    assert_eq!(ack(&mut reader, &second.request_id).await, "popup");
    assert!(second_result
        .recv()
        .await
        .unwrap()
        .stdout
        .contains("second"));
    if !external_terminal {
        other.coordinator.submit(ChannelResult::cancel("timeout"));
    }
    ipc::write_msg(&mut write, &ClientMsg::PopupHostIdle { generation })
        .await
        .unwrap();
    loop {
        if let ServerMsg::PopupHostIdleAck {
            generation: g,
            keep_warm,
        } = next(&mut reader).await
        {
            assert_eq!(g, generation);
            assert!(!keep_warm);
            break;
        }
    }
    handler.await.unwrap();
    assert!(state.popup_inbox.lock().unwrap().generation().is_none());
    let _ = std::fs::remove_dir_all(crate::paths::request_temp_dir(&first.request_id));
}

#[tokio::test(start_paused = true)]
async fn fragmented_image_answer_survives_unchanged_pruning_and_next_submission() {
    fragmented_answer(false).await;
}

#[tokio::test(start_paused = true)]
async fn external_terminal_pruning_preserves_another_partial_answer() {
    fragmented_answer(true).await;
}

#[tokio::test]
async fn eof_and_invalid_json_keep_the_idle_disconnect_contract() {
    for bytes in [&b""[..], &b"{invalid json}\n"[..]] {
        let state = state();
        let lease = state.popup_inbox.lock().unwrap().start().unwrap();
        let (mut writer, reader) = tokio::io::duplex(128);
        writer.write_all(bytes).await.unwrap();
        writer.shutdown().await.unwrap();
        handle_popup_host(
            lease.token,
            BufReader::new(reader),
            tokio::io::sink(),
            &state,
        )
        .await;
        assert!(state.popup_inbox.lock().unwrap().generation().is_none());
        assert_eq!(state.active.load(Ordering::SeqCst), 1);
    }
}

struct BrokenWriter;
impl tokio::io::AsyncWrite for BrokenWriter {
    fn poll_write(
        self: Pin<&mut Self>,
        _: &mut Context<'_>,
        _: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        Poll::Ready(Err(std::io::Error::new(
            std::io::ErrorKind::BrokenPipe,
            "isolated failure",
        )))
    }
    fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Poll::Ready(Ok(()))
    }
    fn poll_shutdown(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Poll::Ready(Ok(()))
    }
}

#[tokio::test]
async fn writer_failure_ends_a_pending_read_without_hanging() {
    let state = state();
    let lease = state.popup_inbox.lock().unwrap().start().unwrap();
    let (_peer, reader) = tokio::io::duplex(128);
    tokio::time::timeout(
        Duration::from_secs(2),
        handle_popup_host(lease.token, BufReader::new(reader), BrokenWriter, &state),
    )
    .await
    .unwrap();
    assert!(state.popup_inbox.lock().unwrap().generation().is_none());
}

#[tokio::test]
async fn prewarmed_idle_host_retires_after_drain_without_reconnecting() {
    let state = state();
    state.draining.store(false, Ordering::SeqCst);
    state.config.lock().unwrap().general.popup_prewarm = true;
    let lease = state.popup_inbox.lock().unwrap().start().unwrap();
    let generation = lease.generation;
    let (server, client) = tokio::io::duplex(8192);
    let (read, write) = tokio::io::split(server);
    let run_state = state.clone();
    let handler = tokio::spawn(async move {
        handle_popup_host(lease.token, BufReader::new(read), write, &run_state).await;
    });
    let (read, mut write) = tokio::io::split(client);
    let mut reader = ipc::MessageReader::new(BufReader::new(read));
    assert!(matches!(
        next(&mut reader).await,
        ServerMsg::PopupHostAccepted { .. }
    ));
    assert!(matches!(
        next(&mut reader).await,
        ServerMsg::UpdateState { .. }
    ));
    for keep_warm in [true, false] {
        state.draining.store(!keep_warm, Ordering::SeqCst);
        ipc::write_msg(&mut write, &ClientMsg::PopupHostIdle { generation })
            .await
            .unwrap();
        assert!(
            matches!(next(&mut reader).await, ServerMsg::PopupHostIdleAck { generation: g, keep_warm: w } if g == generation && w == keep_warm)
        );
    }
    handler.await.unwrap();
    assert!(state.popup_inbox.lock().unwrap().generation().is_none());
}
