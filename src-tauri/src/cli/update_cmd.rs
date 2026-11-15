#[cfg(windows)]
use std::time::{Duration, Instant};

pub fn dispatch(args: &[String]) -> i32 {
    match args.first().map(String::as_str) {
        Some("prepare") if args.len() == 1 => prepare(),
        _ => {
            eprintln!("Usage: AskHuman update prepare");
            1
        }
    }
}

/// What one Status probe says about a daemon that may still hold the installed executable open.
#[cfg(any(windows, test))]
#[derive(Debug)]
enum Daemon {
    /// Nothing there that could be asked to stop: nothing listens, or the exchange failed in a way
    /// that leaves no daemon to talk to.
    Gone,
    /// It answered, with this many requests still in flight.
    Answering { active_requests: usize },
    /// It accepted the connection and did not answer in time. It is alive, so it may well hold
    /// the executable open, yet it cannot be asked to stop. Reading that as `Gone` would report
    /// readiness for a replacement that then fails on a locked file.
    Silent,
}

#[cfg(any(windows, test))]
fn daemon_from(probe: std::io::Result<Option<crate::ipc::StatusInfo>>) -> Daemon {
    match probe {
        Ok(Some(status)) => Daemon::Answering {
            active_requests: status.active_requests,
        },
        Err(error) if error.kind() == std::io::ErrorKind::TimedOut => Daemon::Silent,
        Ok(None) | Err(_) => Daemon::Gone,
    }
}

#[cfg(windows)]
async fn probe_daemon() -> Daemon {
    daemon_from(crate::client::request_status_result().await)
}

#[cfg(windows)]
fn prepare() -> i32 {
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("AskHuman update prepare: failed to create runtime: {error}");
            return 1;
        }
    };

    runtime.block_on(async {
        let kind = crate::update::detect_install_kind();
        let current_exe = std::env::current_exe().ok();

        match probe_daemon().await {
            Daemon::Gone => {}
            Daemon::Silent => {
                eprintln!(
                    "AskHuman update prepare: the daemon accepts connections but does not answer, \
                     so it cannot be asked to stop; end its process and retry"
                );
                return 1;
            }
            Daemon::Answering { .. } => {
                if !crate::client::request_stop(false).await {
                    eprintln!("AskHuman update prepare: failed to request a graceful daemon stop");
                    return 1;
                }
                eprintln!("AskHuman update prepare: waiting for the daemon to drain…");
                let mut last_progress = Instant::now() - Duration::from_secs(30);
                loop {
                    let progress = match probe_daemon().await {
                        Daemon::Gone => break,
                        Daemon::Answering { active_requests } => {
                            format!("{active_requests} active request(s) remaining…")
                        }
                        // Alive, so it still has to exit before the executable can be replaced.
                        Daemon::Silent => {
                            "the daemon is not answering; still waiting for it to exit…".to_string()
                        }
                    };
                    if last_progress.elapsed() >= Duration::from_secs(30) {
                        eprintln!("AskHuman update prepare: {progress}");
                        last_progress = Instant::now();
                    }
                    tokio::time::sleep(Duration::from_secs(1)).await;
                }
            }
        }

        // A Settings Host may be finishing the update check whose HTTP client has a 30-second
        // timeout. Give graceful Tauri shutdown enough room to finish that bounded task.
        let deadline = Instant::now() + Duration::from_secs(45);
        loop {
            let sent = crate::gui_host::shutdown_if_running().await;
            if !sent
                && crate::ipc::transport::connect_role("gui-host")
                    .await
                    .is_err()
            {
                break;
            }
            if Instant::now() >= deadline {
                eprintln!(
                    "AskHuman update prepare: timed out closing the GUI Host at {}",
                    crate::ipc::transport::endpoint_path("gui-host").display()
                );
                return 1;
            }
            // Retry the idempotent request until the pipe disappears. This also covers an old Host
            // that closes without the acknowledgement introduced alongside `update prepare`.
            tokio::time::sleep(Duration::from_millis(100)).await;
        }

        // A concurrent caller could have started the daemon while the Host was closing. Never
        // report readiness while a process that may lock the installed executable is reachable.
        match probe_daemon().await {
            Daemon::Gone => {}
            Daemon::Answering { .. } => {
                eprintln!(
                    "AskHuman update prepare: the daemon restarted during preparation; retry"
                );
                return 1;
            }
            Daemon::Silent => {
                eprintln!(
                    "AskHuman update prepare: a daemon is running but does not answer; \
                     end its process and retry"
                );
                return 1;
            }
        }

        println!("AskHuman update prepare: ready");
        match kind {
            crate::update::InstallKind::Npm => {
                println!("next: {}", crate::update::npm::NpmUpdater::manual_command());
            }
            crate::update::InstallKind::Direct => {
                println!(
                    "release: https://github.com/{}/{}/releases/latest",
                    crate::update::GITHUB_OWNER,
                    crate::update::GITHUB_REPO
                );
                if let Some(path) = current_exe {
                    println!("target: {}", path.display());
                }
                println!("next: extract the Windows zip and replace the target executable");
            }
        }
        0
    })
}

#[cfg(not(windows))]
fn prepare() -> i32 {
    eprintln!("AskHuman update prepare is only required on Windows");
    1
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Error, ErrorKind};

    #[test]
    fn unknown_subcommand_is_rejected() {
        assert_eq!(super::dispatch(&["unknown".to_string()]), 1);
    }

    #[test]
    fn a_daemon_that_accepts_and_stays_silent_is_not_gone() {
        assert!(matches!(
            daemon_from(Err(Error::new(ErrorKind::TimedOut, "no answer"))),
            Daemon::Silent
        ));
        // Nothing listens: nothing can hold the executable open.
        assert!(matches!(daemon_from(Ok(None)), Daemon::Gone));
        // Any other failed exchange keeps the long-standing reading: no daemon to talk to.
        assert!(matches!(
            daemon_from(Err(Error::new(ErrorKind::ConnectionReset, "reset"))),
            Daemon::Gone
        ));
        let status: crate::ipc::StatusInfo = serde_json::from_value(serde_json::json!({
            "pid": 7,
            "version": "test",
            "protocolVersion": 1,
            "uptimeSecs": 0,
            "socket": "daemon.sock",
            "activeRequests": 2,
        }))
        .unwrap();
        assert!(matches!(
            daemon_from(Ok(Some(status))),
            Daemon::Answering { active_requests: 2 }
        ));
    }
}
