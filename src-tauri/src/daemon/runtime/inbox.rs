//! Shared Popup Host dispatch, authenticated request routing, and bounded recovery.
use super::*;
use crate::daemon::popup_inbox::{Lease, Recovery};

fn terminal(entry: &InteractionEntry) -> bool {
    match entry {
        InteractionEntry::Ask(entry) => entry.coordinator.is_finalizing(),
        InteractionEntry::Confirm(entry) => entry.coordinator.is_terminal(),
    }
}
fn connect_entry(entry: &InteractionEntry, tx: &tokio::sync::mpsc::UnboundedSender<ServerMsg>) {
    match entry {
        InteractionEntry::Ask(entry) => {
            entry.gui_connected.store(true, Ordering::SeqCst);
            *entry.gui.lock().unwrap() = Some(tx.clone());
        }
        InteractionEntry::Confirm(entry) => {
            entry.gui_connected.store(true, Ordering::SeqCst);
            *entry.gui.lock().unwrap() = Some(tx.clone());
        }
    }
}
fn detach_entry(entry: &InteractionEntry, tx: &tokio::sync::mpsc::UnboundedSender<ServerMsg>) {
    let (slot, connected) = match entry {
        InteractionEntry::Ask(entry) => (&entry.gui, &entry.gui_connected),
        InteractionEntry::Confirm(entry) => (&entry.gui, &entry.gui_connected),
    };
    let mut slot = slot.lock().unwrap();
    if slot
        .as_ref()
        .is_some_and(|current| current.same_channel(tx))
    {
        *slot = None;
        connected.store(false, Ordering::SeqCst);
    }
}
fn show_entry(
    entry: &InteractionEntry,
    tx: &tokio::sync::mpsc::UnboundedSender<ServerMsg>,
    state: &Arc<ServerState>,
) {
    if terminal(entry) {
        return;
    }
    connect_entry(entry, tx);
    let mut show = entry.show().clone();
    show.sequence = entry.seq();
    if let InteractionEntry::Ask(entry) = entry {
        if let Some(resolved) = entry.resolved_agent.lock().unwrap().as_ref() {
            show.agent_kind = resolved.kind.clone().or(show.agent_kind);
            show.agent_pid = resolved.pid.or(show.agent_pid);
        }
    }
    let identity = show
        .agent_kind
        .as_deref()
        .and_then(AgentKind::parse)
        .zip(show.agent_session_id.as_deref())
        .filter(|(_, session_id)| !session_id.trim().is_empty())
        .map(|(kind, session_id)| (kind, session_id.to_string()));
    if let Some((kind, session_id)) = &identity {
        show.agent_session_title = state.agents.cached_session_title(*kind, session_id);
    }
    let request_id = show.request_id.clone();
    let resolve_title = show.agent_session_title.is_none();
    let _ = tx.send(ServerMsg::Show(show));
    if let Some((kind, session_id)) = identity.filter(|_| resolve_title) {
        let tx = tx.clone();
        let state = state.clone();
        tokio::task::spawn_blocking(move || {
            let title = state
                .agents
                .cached_session_title(kind, &session_id)
                .or_else(|| crate::agents::title::resolve_title(kind, &session_id))
                .filter(|title| !title.trim().is_empty());
            if let Some(title) = title {
                let _ = tx.send(ServerMsg::PopupSessionTitle { request_id, title });
            }
        });
    }
}
fn prune_inbox(state: &Arc<ServerState>) {
    let mut host = state.popup_inbox.lock().unwrap();
    let tx = host.sender();
    for request in host.requests() {
        if terminal(&request.value) {
            if let Some(tx) = &tx {
                let winner = match &request.value {
                    InteractionEntry::Ask(entry) => entry.coordinator.winner_channel_id(),
                    InteractionEntry::Confirm(entry) => entry.coordinator.winner_channel_id(),
                }
                .unwrap_or_else(|| "system".into());
                let _ = tx.send(ServerMsg::Cancel {
                    request_id: request.id.clone(),
                    winner,
                });
                detach_entry(&request.value, tx);
            }
            host.remove(&request.id);
        }
    }
}
fn fail_surface(entry: &InteractionEntry) {
    match entry {
        InteractionEntry::Ask(entry) => {
            entry
                .coordinator
                .surface_lost("popup", "popup host could not be restored");
        }
        InteractionEntry::Confirm(entry) => {
            if entry.mark_failed("popup", "popup host could not be restored") {
                entry
                    .coordinator
                    .fallback(ConfirmFallbackReason::NoAvailableChannel);
            }
        }
    }
}

fn spawn_inbox_process(exe: &std::path::Path, token: &str) -> std::io::Result<u32> {
    use std::process::{Command, Stdio};
    let mut command = Command::new(exe);
    command
        .arg("--popup-host")
        .arg("--token")
        .arg(token)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    crate::daemon::spawn::configure_background(&mut command);
    crate::daemon::spawn::spawn_and_reap(&mut command)
}

fn spawn_inbox_host(lease: Lease, state: &Arc<ServerState>) -> bool {
    let result = std::env::current_exe().and_then(|exe| spawn_inbox_process(&exe, &lease.token));
    if let Err(error) = result {
        log(&format!("failed to start popup host: {error}"));
        recover_inbox(state, lease.generation);
        return false;
    }
    let state = state.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(5)).await;
        let waiting = {
            let host = state.popup_inbox.lock().unwrap();
            host.generation() == Some(lease.generation) && host.sender().is_none()
        };
        if waiting {
            log("popup host startup timed out");
            recover_inbox(&state, lease.generation);
        }
    });
    true
}
fn recover_inbox(state: &Arc<ServerState>, generation: u64) {
    prune_inbox(state);
    let prewarm = warm_mode_enabled(state, crate::config::PopupWindowMode::Merged)
        && !state.draining.load(Ordering::SeqCst);
    let recovery =
        state
            .popup_inbox
            .lock()
            .unwrap()
            .disconnected(generation, Instant::now(), prewarm);
    match recovery {
        Recovery::Restart(lease) => {
            spawn_inbox_host(lease, state);
        }
        Recovery::Failed(requests) => {
            log("popup host recovery limit reached");
            for request in requests {
                fail_surface(&request.value);
            }
        }
        Recovery::Stale | Recovery::Idle => {}
    }
}

pub(super) fn dispatch_inbox_popup(entry: InteractionEntry, state: &Arc<ServerState>) -> bool {
    prune_inbox(state);
    let (tx, lease, added) = {
        let mut host = state.popup_inbox.lock().unwrap();
        let added = host.enqueue(entry.request_id().to_string(), entry.seq(), entry.clone());
        (host.sender(), host.start(), added)
    };
    if !added {
        return true;
    }
    if let Some(tx) = tx {
        show_entry(&entry, &tx, state);
    }
    if let Some(lease) = lease {
        spawn_inbox_host(lease, state);
    }
    // Startup failure is delivered through the request's channel-failure contract. Do not
    // cancel it again here, or bypass a recovery attempt that is already in flight.
    true
}
pub(super) fn prewarm_inbox(state: &Arc<ServerState>) {
    if !warm_mode_enabled(state, crate::config::PopupWindowMode::Merged)
        || !has_display()
        || state.draining.load(Ordering::SeqCst)
    {
        return;
    }
    let lease = state.popup_inbox.lock().unwrap().start();
    if let Some(lease) = lease {
        spawn_inbox_host(lease, state);
    }
}
pub(super) fn recycle_inbox(state: &Arc<ServerState>) {
    prune_inbox(state);
    if let Some(tx) = state.popup_inbox.lock().unwrap().stop_idle() {
        let _ = tx.send(ServerMsg::PopupHostShutdown);
    }
}
pub(super) fn focus_inbox_request(state: &Arc<ServerState>, id: &str) -> bool {
    let mut host = state.popup_inbox.lock().unwrap();
    if host.get(id).is_none() {
        return false;
    }
    if let Some(tx) = host.sender() {
        let _ = tx.send(ServerMsg::FocusPopup {
            request_id: id.to_string(),
        });
    } else {
        host.request_focus(id);
    }
    true
}

pub(super) async fn handle_popup_host(
    token: String,
    mut reader: Reader,
    writer: OwnedWriteHalf,
    state: &Arc<ServerState>,
) {
    // An idle/prewarmed GUI must not keep the daemon alive. The callers waiting for their
    // independent requests provide liveness, including while the host is recovering.
    state.active.fetch_sub(1, Ordering::SeqCst);
    let (tx, mut messages) = tokio::sync::mpsc::unbounded_channel::<ServerMsg>();
    let accepted = state
        .popup_inbox
        .lock()
        .unwrap()
        .connect(&token, tx.clone());
    let Some((generation, _)) = accepted else {
        state.active.fetch_add(1, Ordering::SeqCst);
        log("rejected unreserved popup host connection");
        return;
    };
    let failed = Arc::new(tokio::sync::Notify::new());
    let writer_failed = failed.clone();
    let write_task = tokio::spawn(async move {
        let mut writer = writer;
        while let Some(message) = messages.recv().await {
            if ipc::write_msg(&mut writer, &message).await.is_err() {
                break;
            }
        }
        writer_failed.notify_one();
    });
    prune_inbox(state);
    let requests = state.popup_inbox.lock().unwrap().requests();
    for request in requests {
        show_entry(&request.value, &tx, state);
    }
    {
        let update = state.update.lock().unwrap();
        let _ = tx.send(ServerMsg::UpdateState {
            available: update.available,
            latest_version: update.latest_version.clone(),
            pending: update.pending,
        });
    }
    if let Some(id) = state.popup_inbox.lock().unwrap().take_focus() {
        let _ = tx.send(ServerMsg::FocusPopup { request_id: id });
    }
    let mut retired = false;
    let mut terminals = tokio::time::interval(Duration::from_millis(100));
    terminals.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        let message = tokio::select! {
            message = ipc::read_msg::<_, ClientMsg>(&mut reader) => message,
            _ = failed.notified() => break,
            _ = terminals.tick() => { prune_inbox(state); continue; },
        };
        let Ok(Some(message)) = message else { break };
        match message {
            ClientMsg::PopupHostIdle {
                generation: requested,
            } if requested == generation => {
                prune_inbox(state);
                let keep_warm = warm_mode_enabled(state, crate::config::PopupWindowMode::Merged)
                    && !state.draining.load(Ordering::SeqCst);
                let empty = {
                    let mut host = state.popup_inbox.lock().unwrap();
                    let empty = host.requests().is_empty();
                    if empty && !keep_warm {
                        retired = host.retire_idle(generation);
                    }
                    empty && (keep_warm || retired)
                };
                if empty {
                    let _ = tx.send(ServerMsg::PopupHostIdleAck {
                        generation,
                        keep_warm,
                    });
                    if retired {
                        break;
                    }
                }
            }
            ClientMsg::PopupReady { request_id, .. } => {
                if let Some(entry) = state.popup_inbox.lock().unwrap().get(&request_id) {
                    if let InteractionEntry::Ask(entry) = entry {
                        entry.gui_ready.store(true, Ordering::SeqCst);
                    }
                    let _ = tx.send(ServerMsg::PresentPopup {
                        request_id,
                        presentation: PopupPresentation::Foreground,
                    });
                }
            }
            ClientMsg::ConfirmReady { request_id } => {
                if let Some(InteractionEntry::Confirm(entry)) =
                    state.popup_inbox.lock().unwrap().get(&request_id)
                {
                    if !entry.is_ready("popup") {
                        entry.mark_ready("popup", String::new());
                    }
                }
            }
            ClientMsg::Answer {
                request_id,
                action,
                answers,
            } => {
                let entry = state.popup_inbox.lock().unwrap().get(&request_id);
                let (winner, error) = match entry {
                    Some(InteractionEntry::Ask(entry)) => {
                        let won = entry.coordinator.try_submit(ChannelResult {
                            action,
                            answers,
                            source_channel_id: "popup".into(),
                        });
                        (
                            Some(if won {
                                "popup".into()
                            } else {
                                entry
                                    .coordinator
                                    .winner_channel_id()
                                    .unwrap_or_else(|| "system".into())
                            }),
                            None,
                        )
                    }
                    Some(_) => (
                        None,
                        Some("question answer does not match this request".into()),
                    ),
                    None => (Some("system".into()), None),
                };
                let _ = tx.send(ServerMsg::PopupSubmissionAck {
                    request_id,
                    winner,
                    error,
                });
                prune_inbox(state);
            }
            ClientMsg::ConfirmAnswer {
                request_id,
                choice_index,
                comment,
            } => {
                let entry = state.popup_inbox.lock().unwrap().get(&request_id);
                let (winner, error) = match entry {
                    Some(InteractionEntry::Confirm(entry)) if entry.is_ready("popup") => {
                        match entry
                            .coordinator
                            .submit_wire(choice_index, comment, "popup")
                        {
                            Ok(won) => (
                                Some(if won {
                                    "popup".into()
                                } else {
                                    entry
                                        .coordinator
                                        .winner_channel_id()
                                        .unwrap_or_else(|| "system".into())
                                }),
                                None,
                            ),
                            Err(error) => (None, Some(error)),
                        }
                    }
                    Some(InteractionEntry::Confirm(_)) => {
                        (None, Some("confirmation is not ready".into()))
                    }
                    Some(_) => (
                        None,
                        Some("confirmation answer does not match this request".into()),
                    ),
                    None => (Some("system".into()), None),
                };
                let _ = tx.send(ServerMsg::PopupSubmissionAck {
                    request_id,
                    winner,
                    error,
                });
                prune_inbox(state);
            }
            ClientMsg::PopupFocused { .. } | ClientMsg::PopupDismissed { .. } => {}
            _ => {}
        }
    }
    let requests = state.popup_inbox.lock().unwrap().requests();
    for request in requests {
        detach_entry(&request.value, &tx);
    }
    drop(tx);
    if retired {
        let _ = write_task.await;
    } else {
        write_task.abort();
        recover_inbox(state, generation);
    }
    state.active.fetch_add(1, Ordering::SeqCst);
}

#[cfg(all(test, unix))]
mod spawn_tests {
    use super::spawn_inbox_process;
    use std::os::unix::fs::PermissionsExt;
    use std::time::{Duration, Instant};

    #[test]
    fn shared_host_preserves_arguments_and_reaps_exited_child() {
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join("popup-host");
        std::fs::write(
            &exe,
            "#!/bin/sh\nprintf '%s\\n' \"$@\" > \"$0.args\"\nexec /bin/sleep 0.1\n",
        )
        .unwrap();
        std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o700)).unwrap();

        let pid = spawn_inbox_process(&exe, "token with spaces").unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if unsafe { libc::kill(pid as libc::pid_t, 0) } == -1 {
                assert_eq!(
                    std::io::Error::last_os_error().raw_os_error(),
                    Some(libc::ESRCH)
                );
                break;
            }
            assert!(
                Instant::now() < deadline,
                "shared host {pid} was not reaped"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(
            std::fs::read_to_string(dir.path().join("popup-host.args")).unwrap(),
            "--popup-host\n--token\ntoken with spaces\n"
        );
    }

    #[test]
    fn shared_host_spawn_failure_is_returned() {
        let dir = tempfile::tempdir().unwrap();
        let error = spawn_inbox_process(&dir.path().join("missing-popup-host"), "token")
            .expect_err("missing host executable must fail");
        assert_eq!(error.kind(), std::io::ErrorKind::NotFound);
    }
}
