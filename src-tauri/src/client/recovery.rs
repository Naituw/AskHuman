//! Stable recovery transport for a caller that stays alive while its daemon is replaced.

use super::*;
use crate::daemon::recovery::{Call, ClientOperation, Event, Outcome, Task, VERSION};

pub(crate) async fn available() -> bool {
    request_status()
        .await
        .is_some_and(|status| status.recovery_version == VERSION)
}

pub(super) fn ask_enabled(task: &crate::ipc::TaskRequest) -> bool {
    task.message.development.is_none()
        && !crate::agents::failure::is_card_id(task.native_request_id.as_deref())
}

pub(super) async fn confirm(task: crate::ipc::ConfirmTask) -> Option<crate::models::ConfirmResult> {
    match run(Task::Confirm(Box::new(task)), None).await {
        Outcome::Confirm(result) => Some(result),
        _ => None,
    }
}

struct CancelOnDrop {
    call: Call,
    finished: bool,
    generation: usize,
}

impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        if self.finished
            || (self.call.daemon_owned
                && (crate::daemon::recovery::handoff_in_progress()
                    || self.generation != crate::daemon::recovery::daemon_generation()))
        {
            return;
        }
        if let Err(error) = self.call.persist_cancellation() {
            eprintln!("askhuman: cannot retain request cancellation: {error}");
        }
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            let call = self.call.clone();
            runtime.spawn(async move {
                loop {
                    if available().await {
                        if let Ok((mut reader, mut writer)) = connect_split().await {
                            if write_operation(
                                &mut writer,
                                ClientOperation::Cancel { call: call.clone() },
                            )
                            .await
                            .is_ok()
                            {
                                if let Ok(Some(ServerMsg::Recovery {
                                    version: VERSION,
                                    event,
                                })) = ipc::read_msg::<_, ServerMsg>(&mut reader).await
                                {
                                    if matches!(
                                        serde_json::from_value::<Event>(event),
                                        Ok(Event::Updated)
                                    ) {
                                        call.forget_cancellation();
                                        return;
                                    }
                                }
                            }
                        }
                    }
                    tokio::time::sleep(Duration::from_secs(5)).await;
                }
            });
        }
    }
}

async fn write_operation(
    writer: &mut OwnedWriteHalf,
    operation: ClientOperation,
) -> std::io::Result<()> {
    ipc::write_msg(
        writer,
        &ClientMsg::Recover {
            version: VERSION,
            operation: serde_json::to_value(operation)?,
        },
    )
    .await
}

/// Recovery bootstraps through Status, so an older surviving CLI never asks a newer daemon to
/// replace itself merely because their executable fingerprints differ.
pub(crate) async fn start_if_missing() {
    if !matches!(request_status_result().await, Ok(None)) {
        return;
    }
    let _guard = acquire_spawn_lock().await;
    if matches!(request_status_result().await, Ok(None)) {
        if let Err(error) = spawn::spawn_detached(spawn::SpawnPolicy::ReuseAlive) {
            eprintln!("askhuman: waiting to restore the daemon: {error}");
        }
    }
}

pub(super) async fn run(
    task: Task,
    accepted: Option<&std::sync::Mutex<Option<String>>>,
) -> Outcome {
    run_call(task, Call::new(), accepted, true).await
}

async fn run_call(
    task: Task,
    call: Call,
    accepted: Option<&std::sync::Mutex<Option<String>>>,
    acknowledge: bool,
) -> Outcome {
    let mut cancellation = CancelOnDrop {
        call: call.clone(),
        finished: false,
        generation: crate::daemon::recovery::daemon_generation(),
    };
    let mut delay = Duration::from_millis(250);
    loop {
        start_if_missing().await;
        if available().await {
            if let Ok((mut reader, mut writer)) = connect_split().await {
                let sent = write_operation(
                    &mut writer,
                    ClientOperation::Submit {
                        call: call.clone(),
                        task: task.clone(),
                    },
                )
                .await;
                if sent.is_ok() {
                    loop {
                        match ipc::read_msg::<_, ServerMsg>(&mut reader).await {
                            Ok(Some(ServerMsg::Recovery {
                                version: VERSION,
                                event,
                            })) => {
                                match serde_json::from_value::<Event>(event) {
                                    Ok(Event::Accepted { request_id }) => {
                                        if let Some(slot) = accepted {
                                            *slot.lock().unwrap() = Some(request_id);
                                        }
                                        delay = Duration::from_millis(250);
                                    }
                                    Ok(Event::Final { outcome }) => {
                                        cancellation.finished = true;
                                        // The result stays durable even if this best-effort receipt is lost.
                                        if acknowledge {
                                            if let Ok((_, mut acknowledgement)) =
                                                connect_split().await
                                            {
                                                let _ = write_operation(
                                                    &mut acknowledgement,
                                                    ClientOperation::Delivered {
                                                        call: call.clone(),
                                                    },
                                                )
                                                .await;
                                            }
                                        }
                                        return outcome;
                                    }
                                    Ok(Event::Unavailable) | Err(_) => break,
                                    Ok(
                                        Event::Updated | Event::Updating | Event::PopupLease { .. },
                                    ) => {}
                                }
                            }
                            Ok(Some(_)) => {}
                            Ok(None) | Err(_) => break,
                        }
                    }
                }
            }
        }
        tokio::time::sleep(delay).await;
        delay = delay.saturating_mul(2).min(Duration::from_secs(5));
    }
}

pub(super) async fn ask(
    task: crate::ipc::TaskRequest,
    verbose: bool,
    accepted: Option<&std::sync::Mutex<Option<String>>>,
) -> AskFinal {
    match run(Task::Ask(Box::new(task)), accepted).await {
        Outcome::Ask(result) => {
            if verbose {
                if let Some(warning) = result.stderr {
                    eprintln!("{warning}");
                }
            }
            AskFinal {
                stdout: result.stdout,
                exit_code: result.exit_code,
            }
        }
        _ => client_error(verbose, "askhuman: request cancelled"),
    }
}

pub(crate) struct OwnedHandle {
    store: std::sync::Arc<crate::daemon::recovery::Store>,
    pub call: Call,
    pub effect: crate::daemon::recovery::OwnedEffect,
    key: String,
}

impl OwnedHandle {
    pub async fn claim(&self) -> bool {
        loop {
            match self.store.claim_owned(&self.key) {
                Ok(claimed) => return claimed,
                Err(error) => {
                    eprintln!("askhuman: waiting to save native response intent: {error}")
                }
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    }

    pub async fn complete(&self, success: bool) {
        while let Err(error) = self.store.complete_owned(&self.key, success) {
            eprintln!("askhuman: waiting to save native response receipt: {error}");
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    }
}

pub(crate) fn owned_exists(key: &str) -> anyhow::Result<bool> {
    crate::daemon::recovery::Store::open(
        &crate::paths::config_dir().join("state/requests.sqlite3"),
    )?
    .owned_exists(key)
}

pub(crate) fn cancel_owned(key: &str) -> anyhow::Result<()> {
    crate::daemon::recovery::Store::open(
        &crate::paths::config_dir().join("state/requests.sqlite3"),
    )?
    .cancel_owned(key)
}

pub(crate) async fn run_owned(key: String, task: Task) -> (Outcome, OwnedHandle) {
    run_owned_observed(key, task, None).await
}

pub(crate) fn owned_request_id(key: &str) -> anyhow::Result<Option<String>> {
    let store = crate::daemon::recovery::Store::open(
        &crate::paths::config_dir().join("state/requests.sqlite3"),
    )?;
    let record: Option<crate::daemon::recovery::OwnedCall> =
        store.metadata(&format!("owned/{key}"))?;
    Ok(record
        .and_then(|record| store.resume(&record.call, None).ok().flatten())
        .map(|call| call.request_id))
}

pub(crate) async fn run_owned_observed(
    key: String,
    task: Task,
    accepted: Option<&std::sync::Mutex<Option<String>>>,
) -> (Outcome, OwnedHandle) {
    loop {
        let opened = crate::daemon::recovery::Store::open(
            &crate::paths::config_dir().join("state/requests.sqlite3"),
        );
        if let Ok(store) = opened {
            match store.owned(&key, &task) {
                Ok(saved) => {
                    let handle = OwnedHandle {
                        store: std::sync::Arc::new(store),
                        call: saved.call.clone(),
                        effect: saved.effect.clone(),
                        key: key.clone(),
                    };
                    if matches!(
                        saved.effect,
                        crate::daemon::recovery::OwnedEffect::Complete
                            | crate::daemon::recovery::OwnedEffect::Submitting
                            | crate::daemon::recovery::OwnedEffect::Unknown
                    ) {
                        return (Outcome::Cancelled, handle);
                    }
                    let Some(task) = saved.task else {
                        tokio::time::sleep(Duration::from_secs(1)).await;
                        continue;
                    };
                    let result = run_call(task, saved.call, accepted, false).await;
                    return (result, handle);
                }
                Err(error) => eprintln!("askhuman: waiting to restore native request: {error}"),
            }
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}
