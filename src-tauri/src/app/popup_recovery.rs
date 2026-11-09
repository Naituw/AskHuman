//! A popup keeps its UI and immutable submissions while the daemon changes underneath it.
use crate::daemon::recovery::{ClientOperation, Event, VERSION};
use crate::ipc::{self, transport, ClientMsg, ServerMsg};
use std::collections::{HashMap, HashSet};
use std::pin::Pin;
use std::time::Duration;
use tokio::io::{AsyncBufRead, BufReader};
use tokio::sync::mpsc::{unbounded_channel, UnboundedSender};

type Reader = BufReader<transport::OwnedReadHalf>;
type Writer = transport::OwnedWriteHalf;

pub(super) enum Kind {
    Host {
        token: String,
    },
    Independent {
        token: Option<String>,
        request_id: Option<String>,
    },
}

pub(super) fn operation(operation: ClientOperation) -> ClientMsg {
    ClientMsg::Recover {
        version: VERSION,
        operation: serde_json::to_value(operation).expect("popup recovery operation serializes"),
    }
}

pub(super) async fn host_hello(token: &str, known_ids: Vec<String>) -> (ClientMsg, bool) {
    let supported = crate::client::recovery::available().await;
    let hello = if supported {
        operation(ClientOperation::PopupHost {
            token: token.into(),
            pid: std::process::id(),
            known_ids,
        })
    } else {
        ClientMsg::PopupHostHello {
            token: token.into(),
        }
    };
    (hello, supported)
}

pub(super) async fn independent_hello(token: &str) -> (ClientMsg, bool) {
    let supported = crate::client::recovery::available().await;
    let hello = if supported {
        operation(ClientOperation::Popup {
            token: token.into(),
            pid: std::process::id(),
            request_id: None,
        })
    } else {
        ClientMsg::GuiHello {
            token: token.into(),
        }
    };
    (hello, supported)
}

fn id_of(message: &ClientMsg) -> Option<&str> {
    match message {
        ClientMsg::Answer { request_id, .. }
        | ClientMsg::ConfirmAnswer { request_id, .. }
        | ClientMsg::PopupReady { request_id, .. }
        | ClientMsg::ConfirmReady { request_id } => Some(request_id),
        _ => None,
    }
}

pub(super) fn bridge(
    reader: Reader,
    writer: Writer,
    mut kind: Kind,
    supported: bool,
) -> (
    UnboundedSender<ClientMsg>,
    Pin<Box<dyn AsyncBufRead + Send>>,
) {
    let (tx, mut messages) = unbounded_channel::<ClientMsg>();
    let (output, mut input) = tokio::io::duplex(128 * 1024);
    tauri::async_runtime::spawn(async move {
        let mut reader = ipc::MessageReader::new(reader);
        let mut writer = writer;
        let mut known = HashSet::<String>::new();
        if let Kind::Independent {
            request_id: Some(id),
            ..
        } = &kind
        {
            known.insert(id.clone());
        }
        let mut pending = HashMap::<String, ClientMsg>::new();
        let mut ready = HashMap::<String, Vec<ClientMsg>>::new();
        let mut updating = false;
        let mut retry = tokio::time::interval(Duration::from_secs(1));
        retry.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            let incoming = tokio::select! {
                _ = retry.tick(), if supported && !pending.is_empty() => {
                    let mut disconnected = false;
                    for message in pending.values() { if ipc::write_msg(&mut writer, message).await.is_err() { disconnected = true; break; } }
                    if !disconnected { continue; }
                    continue;
                }
                message = reader.read::<ServerMsg>() => message,
                outgoing = messages.recv() => {
                    let Some(message) = outgoing else { return; };
                    if let Some(id) = id_of(&message).map(str::to_owned) {
                        match &message {
                            ClientMsg::Answer { .. } | ClientMsg::ConfirmAnswer { .. } => { pending.insert(id, message.clone()); }
                            ClientMsg::PopupReady { .. } | ClientMsg::ConfirmReady { .. } => {
                                let list = ready.entry(id).or_default();
                                let discriminant = std::mem::discriminant(&message);
                                list.retain(|previous| std::mem::discriminant(previous) != discriminant);
                                list.push(message.clone());
                            }
                            _ => {}
                        }
                    }
                    if ipc::write_msg(&mut writer, &message).await.is_ok() { continue; }
                    continue;
                }
            };
            let message = match incoming {
                Ok(Some(message)) => message,
                Ok(None) | Err(_) => {
                    if !supported || !updating || known.is_empty() {
                        return;
                    }
                    let mut delay = Duration::from_millis(100);
                    loop {
                        crate::client::recovery::start_if_missing().await;
                        if crate::client::recovery::available().await {
                            if let Ok(stream) = transport::connect().await {
                                let (read, mut write) = stream.into_split();
                                let hello = match &kind {
                                    Kind::Host { token } => operation(ClientOperation::PopupHost {
                                        token: token.clone(),
                                        pid: std::process::id(),
                                        known_ids: known.iter().cloned().collect(),
                                    }),
                                    Kind::Independent {
                                        token: Some(token),
                                        request_id,
                                    } => operation(ClientOperation::Popup {
                                        token: token.clone(),
                                        pid: std::process::id(),
                                        request_id: request_id.clone(),
                                    }),
                                    Kind::Independent { token: None, .. } => return,
                                };
                                if ipc::write_msg(&mut write, &hello).await.is_ok() {
                                    reader = ipc::MessageReader::new(BufReader::new(read));
                                    writer = write;
                                    for list in ready.values() {
                                        for message in list {
                                            let _ = ipc::write_msg(&mut writer, message).await;
                                        }
                                    }
                                    for message in pending.values() {
                                        let _ = ipc::write_msg(&mut writer, message).await;
                                    }
                                    break;
                                }
                            }
                        }
                        tokio::time::sleep(delay).await;
                        delay = delay.saturating_mul(2).min(Duration::from_secs(3));
                    }
                    continue;
                }
            };
            let mut finished = false;
            match &message {
                ServerMsg::Recovery {
                    version: VERSION,
                    event,
                } => match serde_json::from_value::<Event>(event.clone()) {
                    Ok(Event::Updating) => updating = true,
                    Ok(Event::Updated) => updating = false,
                    Ok(Event::PopupLease { token }) => {
                        if let Kind::Independent { token: current, .. } = &mut kind {
                            *current = Some(token);
                        }
                        continue;
                    }
                    _ => {}
                },
                ServerMsg::Show(show) => {
                    if let Kind::Independent { request_id, .. } = &mut kind {
                        *request_id = Some(show.request_id.clone());
                    }
                    let existed = !known.insert(show.request_id.clone());
                    if updating {
                        let updated = ServerMsg::Recovery {
                            version: VERSION,
                            event: serde_json::to_value(Event::Updated).unwrap(),
                        };
                        if ipc::write_msg(&mut input, &updated).await.is_err() {
                            return;
                        }
                        updating = false;
                    }
                    if existed {
                        continue;
                    }
                }
                ServerMsg::PopupHostAccepted { .. } => {
                    if updating {
                        let updated = ServerMsg::Recovery {
                            version: VERSION,
                            event: serde_json::to_value(Event::Updated).unwrap(),
                        };
                        if ipc::write_msg(&mut input, &updated).await.is_err() {
                            return;
                        }
                        updating = false;
                    }
                    continue;
                }
                ServerMsg::PopupSubmissionAck {
                    request_id, error, ..
                } => {
                    pending.remove(request_id);
                    if error.is_none() {
                        known.remove(request_id);
                        ready.remove(request_id);
                    }
                }
                ServerMsg::Cancel { request_id, .. } => {
                    known.remove(request_id);
                    pending.remove(request_id);
                    ready.remove(request_id);
                    finished = matches!(kind, Kind::Independent { .. });
                }
                ServerMsg::PopupHostShutdown
                | ServerMsg::PopupHostIdleAck {
                    keep_warm: false, ..
                } => finished = true,
                _ => {}
            }
            if ipc::write_msg(&mut input, &message).await.is_err() || finished {
                return;
            }
        }
    });
    (tx, Box::pin(BufReader::new(output)))
}
