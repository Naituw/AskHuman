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

/// What one Status probe says about the daemon.
#[derive(Debug, PartialEq, Eq)]
enum Probe {
    /// It answered.
    Alive,
    /// Nothing is listening (absent, or refusing because it is out of room to accept).
    Absent,
    /// It accepted the connection and did not answer in time: alive but wedged.
    Silent,
    /// Anything else (permission denied, a malformed reply): starting another daemon would not help.
    Unknown,
}

fn classify(result: &std::io::Result<Option<crate::ipc::StatusInfo>>) -> Probe {
    match result {
        Ok(Some(_)) => Probe::Alive,
        Ok(None) => Probe::Absent,
        Err(error) if error.kind() == ErrorKind::TimedOut => Probe::Silent,
        Err(_) => Probe::Unknown,
    }
}

/// Which daemon instance owns the endpoint, as far as `daemon.json` can say: process id and start
/// time. A replaced or restarted daemon has a different one.
type Identity = Option<(u32, u64)>;

fn daemon_identity() -> Identity {
    lifecycle::read_meta().map(|meta| (meta.pid, meta.started_at))
}

/// Whether the daemon that was diagnosed is still the one in front of us. Without an identity
/// there is nothing to compare, so the answer is no.
fn same_instance(diagnosed: Identity, now: Identity) -> bool {
    diagnosed.is_some() && diagnosed == now
}

/// Recovery bootstraps through Status, so an older surviving CLI never asks a newer daemon to
/// replace itself merely because their executable fingerprints differ.
pub(crate) async fn start_if_missing() {
    let diagnosed = daemon_identity();
    let first = classify(&request_status_result().await);
    restore(first, diagnosed).await;
}

/// `available`, but a daemon that accepts connections and does not answer gets the chance to be
/// replaced before the caller settles for another transport (or gives up on this one).
pub(crate) async fn available_or_restore() -> bool {
    let diagnosed = daemon_identity();
    match request_status_result().await {
        Ok(Some(status)) => status.recovery_version == VERSION,
        Err(error) if error.kind() == ErrorKind::TimedOut => {
            restore(Probe::Silent, diagnosed).await;
            available().await
        }
        _ => false,
    }
}

/// Act on a first probe that found no usable daemon.
///
/// A daemon that does not answer is not left alone forever, but replacing one is destructive, so
/// it takes more than one bad moment: the second probe happens under the cross-process spawn lock
/// (siblings queue instead of racing, and the holder waits for readiness before releasing it), the
/// lock must really be held, and `daemon.json` must still name the instance that was diagnosed, so
/// a successor a sibling just started is never mistaken for the wedged one. Two silent probes in a
/// row replace it; so does a job that is alive yet stays unreachable after a start. The spawn
/// ladder of `ensure_running` does the same for the Hello path, which this one cannot use because
/// it must not send a Hello.
async fn restore(first: Probe, diagnosed: Identity) {
    if matches!(first, Probe::Alive | Probe::Unknown) {
        return;
    }
    let guard = acquire_spawn_lock().await;
    let second = classify(&request_status_result().await);
    match second {
        Probe::Alive | Probe::Unknown => {}
        Probe::Absent => {
            if let Err(error) = spawn::spawn_detached(spawn::SpawnPolicy::ReuseAlive).await {
                eprintln!("askhuman: waiting to restore the daemon: {error}");
                return;
            }
            if !wait_for_status(Duration::from_secs(5)).await
                && guard.is_some()
                && same_instance(diagnosed, daemon_identity())
            {
                replace().await;
            }
        }
        Probe::Silent => {
            if first == Probe::Silent
                && guard.is_some()
                && same_instance(diagnosed, daemon_identity())
            {
                replace().await;
            }
        }
    }
}

/// Tear the wedged daemon down and start a fresh one. Only a platform service manager can do that
/// (launchd on macOS); elsewhere a second daemon would just lose the single-instance lock, so the
/// caller keeps retrying instead.
async fn replace() {
    if !cfg!(target_os = "macos") {
        return;
    }
    eprintln!("askhuman: the daemon does not answer; replacing it");
    match spawn::spawn_detached(spawn::SpawnPolicy::Replace).await {
        Ok(()) => {
            wait_for_status(Duration::from_secs(5)).await;
        }
        Err(error) => eprintln!("askhuman: cannot replace the daemon: {error}"),
    }
}

/// Poll Status until the daemon answers or `max` elapses.
async fn wait_for_status(max: Duration) -> bool {
    let start = Instant::now();
    while start.elapsed() < max {
        if matches!(request_status_result().await, Ok(Some(_))) {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    false
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

#[cfg(test)]
mod tests {
    use super::*;

    fn status() -> crate::ipc::StatusInfo {
        serde_json::from_value(serde_json::json!({
            "pid": 1, "version": "x", "protocolVersion": 1,
            "uptimeSecs": 0, "socket": "s", "activeRequests": 0
        }))
        .unwrap()
    }

    #[test]
    fn a_probe_tells_a_silent_daemon_from_an_absent_one() {
        assert_eq!(classify(&Ok(Some(status()))), Probe::Alive);
        // Nothing listening: start one.
        assert_eq!(classify(&Ok(None)), Probe::Absent);
        // Accepted and never answered: replace it, do not start a second one next to it.
        assert_eq!(
            classify(&Err(Error::new(ErrorKind::TimedOut, "no answer"))),
            Probe::Silent
        );
        // A daemon we cannot talk to for another reason is not fixed by starting or replacing it.
        for kind in [
            ErrorKind::PermissionDenied,
            ErrorKind::InvalidData,
            ErrorKind::UnexpectedEof,
            ErrorKind::Other,
        ] {
            assert_eq!(
                classify(&Err(Error::from(kind))),
                Probe::Unknown,
                "{kind:?}"
            );
        }
    }

    #[test]
    fn a_daemon_is_only_the_diagnosed_one_if_daemon_json_still_names_it() {
        let old = Some((100, 1_000));
        assert!(same_instance(old, old));
        // A successor a sibling started has another pid, or at least another start time.
        assert!(!same_instance(old, Some((200, 1_000))));
        assert!(!same_instance(old, Some((100, 2_000))));
        // No identity on either side is no evidence, however equal the two nothings are.
        assert!(!same_instance(None, None));
        assert!(!same_instance(old, None));
        assert!(!same_instance(None, old));
    }
}
