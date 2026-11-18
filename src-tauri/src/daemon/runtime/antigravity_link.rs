//! The daemon's end of a sidecar's connection (docs/specs/antigravity-deep-integration.md §12.3).
//!
//! The Antigravity App starts a sidecar process for each sidecar directory AskHuman made, and
//! that process connects here. While it is connected, the daemon can have it run `agentapi`
//! (which only works inside a sidecar's environment).

use super::*;
use crate::antigravity_desktop::{self as desktop, Down, Manager, Up};

/// A sidecar's connection, from the check-in to its end.
///
/// **It does not keep the daemon running.** `handle_conn` counted this connection as active;
/// that is taken back for as long as it lasts (the same offset as `handle_tray_sub`), because the
/// App starts every sidecar when it opens and a connection that stays for days must not stop the
/// daemon from exiting when it is idle. A sidecar reconnects when the daemon comes back.
pub(super) async fn handle_sidecar(
    reader: Reader,
    w: Writer,
    state: &Arc<ServerState>,
    check_in: desktop::CheckIn,
) {
    handle_sidecar_with(
        desktop::manager(),
        reader,
        w,
        state,
        check_in,
        desktop::admit,
    )
    .await;
}

async fn handle_sidecar_with(
    manager: &Manager,
    reader: Reader,
    mut w: Writer,
    state: &Arc<ServerState>,
    check_in: desktop::CheckIn,
    admit: impl Fn(&desktop::CheckIn) -> Result<(), Down>,
) {
    state.active.fetch_sub(1, Ordering::SeqCst);
    serve(manager, reader, &mut w, check_in, admit).await;
    state.active.fetch_add(1, Ordering::SeqCst);
}

async fn serve(
    manager: &Manager,
    reader: Reader,
    w: &mut Writer,
    check_in: desktop::CheckIn,
    admit: impl Fn(&desktop::CheckIn) -> Result<(), Down>,
) {
    if let Err(refusal) = admit(&check_in) {
        if let Down::Refused { reason, restart } = &refusal {
            log(&format!(
                "antigravity sidecar refused id={} restart={restart}: {reason}",
                check_in.id
            ));
        }
        let _ = ipc::write_msg(w, &ServerMsg::AntigravitySidecar { frame: refusal }).await;
        return;
    }
    let Some((generation, mut outbound)) = manager.attach(&check_in, now_ms()) else {
        let _ = ipc::write_msg(
            w,
            &ServerMsg::AntigravitySidecar {
                frame: Down::Refused {
                    reason: "This sidecar is already connected".into(),
                    restart: false,
                },
            },
        )
        .await;
        return;
    };
    // Online from here: the table shows it, and commands queue up behind the welcome.
    let welcomed = ipc::write_msg(
        w,
        &ServerMsg::AntigravitySidecar {
            frame: Down::Welcome,
        },
    )
    .await
    .is_ok();
    if welcomed {
        log(&format!(
            "antigravity sidecar online id={} version={} agentapi={}",
            check_in.id, check_in.version, check_in.agentapi
        ));
        talk(manager, reader, w, &check_in.id, generation, &mut outbound).await;
    }
    manager.detach(&check_in.id, generation);
    log(&format!("antigravity sidecar offline id={}", check_in.id));
}

/// Writes what the daemon queues for the sidecar and takes in its results, until either side
/// ends. What was queued and not written is dropped with `outbound`, which tells whoever queued
/// it that it was not sent.
async fn talk(
    manager: &Manager,
    reader: Reader,
    w: &mut Writer,
    id: &str,
    generation: u64,
    outbound: &mut tokio::sync::mpsc::UnboundedReceiver<desktop::Outbound>,
) {
    let mut reader = ipc::MessageReader::new(reader);
    loop {
        tokio::select! {
            queued = outbound.recv() => {
                let Some(queued) = queued else { break };
                let written = ipc::write_msg(
                    w,
                    &ServerMsg::AntigravitySidecar { frame: queued.frame },
                )
                .await
                .is_ok();
                if let Some(acknowledge) = queued.written {
                    let _ = acknowledge.send(written);
                }
                if !written {
                    break;
                }
            }
            frame = reader.read::<ClientMsg>() => match frame {
                Ok(Some(ClientMsg::AntigravitySidecar { frame: Up::Done { command_id, outcome } })) => {
                    manager.complete(id, generation, &command_id, outcome);
                }
                Ok(Some(_)) => {}
                Ok(None) | Err(_) => break,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::antigravity_desktop::{CheckIn, Command, Outcome, RunError};
    use std::time::Duration;
    use tokio::io::{AsyncWriteExt, BufReader, DuplexStream};

    fn check_in(id: &str) -> CheckIn {
        CheckIn {
            id: id.to_string(),
            link_version: 1,
            project_id: None,
            version: "1.0.0".into(),
            agentapi: true,
            pid: 7,
        }
    }

    /// The sidecar's end of a connection the daemon serves.
    struct Sidecar {
        reader: ipc::MessageReader<BufReader<DuplexStream>>,
        writer: DuplexStream,
    }

    impl Sidecar {
        async fn frame(&mut self) -> Down {
            match tokio::time::timeout(Duration::from_secs(5), self.reader.read::<ServerMsg>())
                .await
                .expect("the daemon sent nothing")
                .unwrap()
                .expect("the daemon closed the connection")
            {
                ServerMsg::AntigravitySidecar { frame } => frame,
                other => panic!("not a sidecar frame: {other:?}"),
            }
        }

        async fn send(&mut self, frame: Up) {
            ipc::write_msg(&mut self.writer, &ClientMsg::AntigravitySidecar { frame })
                .await
                .unwrap();
        }
    }

    fn connection() -> (Reader, Writer, Sidecar) {
        let (sidecar_out, daemon_in) = tokio::io::duplex(1 << 20);
        let (daemon_out, sidecar_in) = tokio::io::duplex(1 << 20);
        (
            Box::new(BufReader::new(daemon_in)),
            Box::new(daemon_out),
            Sidecar {
                reader: ipc::MessageReader::new(BufReader::new(sidecar_in)),
                writer: sidecar_out,
            },
        )
    }

    fn admit_all(_: &CheckIn) -> Result<(), Down> {
        Ok(())
    }

    async fn online(manager: &Manager, id: &str) {
        assert!(
            manager.wait_online(id, Duration::from_secs(5)).await,
            "{id} did not come online"
        );
    }

    #[tokio::test]
    async fn an_admitted_sidecar_is_welcomed_runs_a_command_and_goes_offline_when_it_ends() {
        let manager = Arc::new(Manager::new());
        let (reader, mut writer, mut sidecar) = connection();
        let serving = {
            let manager = manager.clone();
            tokio::spawn(async move {
                serve(
                    &manager,
                    reader,
                    &mut writer,
                    check_in("askhuman-ws-1"),
                    admit_all,
                )
                .await
            })
        };
        assert_eq!(sidecar.frame().await, Down::Welcome);
        online(&manager, "askhuman-ws-1").await;

        let asking = {
            let manager = manager.clone();
            tokio::spawn(async move {
                manager
                    .run("askhuman-ws-1", Command::Probe, Duration::from_secs(5))
                    .await
            })
        };
        let Down::Run {
            command_id,
            command,
        } = sidecar.frame().await
        else {
            panic!("the daemon did not send the command")
        };
        assert_eq!(command, Command::Probe);
        sidecar
            .send(Up::Done {
                command_id,
                outcome: Outcome {
                    status: Some(0),
                    stdout: "answer".into(),
                    ..Outcome::default()
                },
            })
            .await;
        assert_eq!(asking.await.unwrap().unwrap().stdout, "answer");

        // The sidecar ends: it is no longer online and the handler returns.
        drop(sidecar);
        tokio::time::timeout(Duration::from_secs(5), serving)
            .await
            .expect("the handler did not end with the connection")
            .unwrap();
        assert!(!manager.is_online("askhuman-ws-1"));
    }

    #[tokio::test]
    async fn a_sidecar_the_daemon_does_not_admit_is_told_so_and_is_never_online() {
        let manager = Manager::new();
        let (reader, mut writer, mut sidecar) = connection();
        serve(
            &manager,
            reader,
            &mut writer,
            check_in("askhuman-ws-1"),
            |_| {
                Err(Down::Refused {
                    reason: "no".into(),
                    restart: true,
                })
            },
        )
        .await;
        assert_eq!(
            sidecar.frame().await,
            Down::Refused {
                reason: "no".into(),
                restart: true
            }
        );
        assert!(!manager.is_online("askhuman-ws-1"));
    }

    #[tokio::test]
    async fn a_second_connection_for_a_connected_sidecar_is_refused_and_the_first_goes_on() {
        let manager = Arc::new(Manager::new());
        let (reader, mut writer, mut first) = connection();
        let first_serving = {
            let manager = manager.clone();
            tokio::spawn(async move {
                serve(
                    &manager,
                    reader,
                    &mut writer,
                    check_in("askhuman-ws-1"),
                    admit_all,
                )
                .await
            })
        };
        assert_eq!(first.frame().await, Down::Welcome);
        online(&manager, "askhuman-ws-1").await;

        let (reader, mut writer, mut second) = connection();
        serve(
            &manager,
            reader,
            &mut writer,
            check_in("askhuman-ws-1"),
            admit_all,
        )
        .await;
        assert!(matches!(
            second.frame().await,
            Down::Refused { restart: false, .. }
        ));
        // The first one is still the one that is online and can take a command.
        assert!(manager.is_online("askhuman-ws-1"));
        let outcome = {
            let manager = manager.clone();
            let asking = tokio::spawn(async move {
                manager
                    .run("askhuman-ws-1", Command::Probe, Duration::from_secs(5))
                    .await
            });
            let Down::Run { command_id, .. } = first.frame().await else {
                panic!("the command did not reach the first connection")
            };
            first
                .send(Up::Done {
                    command_id,
                    outcome: Outcome::default(),
                })
                .await;
            asking.await.unwrap()
        };
        assert!(outcome.is_ok());
        drop(first);
        let _ = first_serving.await;
    }

    #[tokio::test]
    async fn a_command_the_connection_cannot_carry_was_not_sent() {
        let manager = Arc::new(Manager::new());
        let (reader, mut writer, sidecar) = connection();
        let Sidecar {
            reader: sidecar_reader,
            writer: sidecar_writer,
        } = sidecar;
        let serving = {
            let manager = manager.clone();
            tokio::spawn(async move {
                serve(
                    &manager,
                    reader,
                    &mut writer,
                    check_in("askhuman-ws-1"),
                    admit_all,
                )
                .await
            })
        };
        online(&manager, "askhuman-ws-1").await;
        // The sidecar's reading end is gone, so nothing written to it arrives (and the write
        // fails), while its writing end is still open: the daemon has not noticed it is gone.
        drop(sidecar_reader);
        let error = manager
            .run("askhuman-ws-1", Command::Probe, Duration::from_secs(5))
            .await
            .unwrap_err();
        assert!(matches!(error, RunError::NotSent(_)), "{error:?}");
        drop(sidecar_writer);
        let _ = tokio::time::timeout(Duration::from_secs(5), serving).await;
        assert!(!manager.is_online("askhuman-ws-1"));
    }

    #[tokio::test]
    async fn a_command_that_was_written_and_never_answered_is_unknown_when_the_sidecar_leaves() {
        let manager = Arc::new(Manager::new());
        let (reader, mut writer, mut sidecar) = connection();
        let serving = {
            let manager = manager.clone();
            tokio::spawn(async move {
                serve(
                    &manager,
                    reader,
                    &mut writer,
                    check_in("askhuman-ws-1"),
                    admit_all,
                )
                .await
            })
        };
        assert_eq!(sidecar.frame().await, Down::Welcome);
        let asking = {
            let manager = manager.clone();
            tokio::spawn(async move {
                manager
                    .run("askhuman-ws-1", Command::Probe, Duration::from_secs(30))
                    .await
            })
        };
        let Down::Run { .. } = sidecar.frame().await else {
            panic!("the command did not arrive")
        };
        // It left without a word: the command may have run.
        let _ = sidecar.writer.shutdown().await;
        drop(sidecar);
        let error = asking.await.unwrap().unwrap_err();
        assert!(matches!(error, RunError::Unknown(_)), "{error:?}");
        let _ = serving.await;
    }

    /// A sidecar's connection is not a reason for the daemon to stay: the count of active
    /// connections is one lower for as long as the sidecar is connected, and back after.
    #[tokio::test]
    async fn a_sidecar_connection_does_not_keep_the_daemon_running() {
        let directory = tempfile::tempdir().unwrap();
        let state = test_state(directory.path());
        let manager = Arc::new(Manager::new());
        // `handle_conn` has counted the connection before it hands it over.
        state.active.fetch_add(1, Ordering::SeqCst);
        let counted = state.active.load(Ordering::SeqCst);
        let (reader, writer, mut sidecar) = connection();
        let serving = {
            let (state, manager) = (state.clone(), manager.clone());
            tokio::spawn(async move {
                handle_sidecar_with(
                    &manager,
                    reader,
                    writer,
                    &state,
                    check_in("askhuman-ws-1"),
                    admit_all,
                )
                .await
            })
        };
        assert_eq!(sidecar.frame().await, Down::Welcome);
        online(&manager, "askhuman-ws-1").await;
        assert_eq!(state.active.load(Ordering::SeqCst), counted - 1);
        drop(sidecar);
        serving.await.unwrap();
        assert_eq!(state.active.load(Ordering::SeqCst), counted);

        // A sidecar that is refused leaves the count as it found it, too.
        let (reader, writer, mut sidecar) = connection();
        handle_sidecar_with(
            &manager,
            reader,
            writer,
            &state,
            check_in("askhuman-ws-2"),
            |_| {
                Err(Down::Refused {
                    reason: "no".into(),
                    restart: false,
                })
            },
        )
        .await;
        assert!(matches!(sidecar.frame().await, Down::Refused { .. }));
        assert_eq!(state.active.load(Ordering::SeqCst), counted);
    }

    /// A sidecar sends no `Hello`, so a sidecar of any build is served by the control loop as it
    /// stands and cannot make the daemon give way.
    #[tokio::test]
    async fn the_control_loop_hands_a_check_in_on_without_a_hello() {
        let directory = tempfile::tempdir().unwrap();
        let state = test_state(directory.path());
        let (mut reader, mut writer, mut sidecar) = connection();
        sidecar.send(Up::CheckIn(check_in("askhuman-ws-1"))).await;
        let control = control_loop(&mut reader, &mut writer, &state).await;
        match control {
            Control::AntigravitySidecar(received) => {
                assert_eq!(received, check_in("askhuman-ws-1"))
            }
            _ => panic!("the check-in was not handed on"),
        }
    }
}
