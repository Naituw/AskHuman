//! Durable, one-shot native messages. Observing an idle agent is not permission to replay a send.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use tokio::sync::Notify;

use super::AgentKind;
use crate::models::FileAttachment;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ControlSignal {
    pub generation: String,
    pub pid: u32,
    pub idle: bool,
    #[serde(default)]
    pub operation_id: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub turn_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ControlCommand {
    pub id: String,
    pub text: String,
    pub files: Vec<FileAttachment>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ControlReply {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operation: Option<ControlCommand>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartedControl {
    pub phone_id: String,
    pub operation_id: String,
    pub turn_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Operation {
    pub id: String,
    pub kind: AgentKind,
    pub session_id: String,
    pub phone_id: String,
    pub generation: Option<String>,
    pub text: String,
    pub files: Vec<FileAttachment>,
    pub expected_result_id: Option<String>,
    pub status: String,
    #[serde(default)]
    pub reason: Option<String>,
    pub turn_id: Option<String>,
}

impl Operation {
    pub fn submitted(&self) -> bool {
        matches!(self.status.as_str(), "accepted" | "started")
    }
    pub fn started_control(&self) -> Option<StartedControl> {
        (self.status == "started")
            .then(|| self.turn_id.clone())
            .flatten()
            .map(|turn_id| StartedControl {
                phone_id: self.phone_id.clone(),
                operation_id: self.id.clone(),
                turn_id,
            })
    }
}

struct Peer {
    generation: String,
    pid: u32,
    idle: bool,
    seen: Instant,
}

#[derive(Default)]
struct Inner {
    operations: HashMap<String, Operation>,
    peers: HashMap<String, Peer>,
    retired: HashSet<(String, String)>,
}

pub struct ControlStore {
    root: PathBuf,
    inner: Mutex<Inner>,
    changed: Notify,
    pub send_gate: tokio::sync::Mutex<()>,
}

impl ControlStore {
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            inner: Mutex::new(Inner::default()),
            changed: Notify::new(),
            send_gate: tokio::sync::Mutex::new(()),
        }
    }

    pub fn load() -> Self {
        let store = Self::new(crate::paths::state_dir().join("agent-controls"));
        let mut files: Vec<_> = std::fs::read_dir(&store.root)
            .into_iter()
            .flatten()
            .filter_map(|entry| entry.ok())
            .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_file()))
            .filter_map(|entry| {
                let path = entry.path();
                (path.extension().and_then(|s| s.to_str()) == Some("json"))
                    .then(|| (entry.metadata().and_then(|meta| meta.modified()).ok(), path))
            })
            .collect();
        files.sort_by(|a, b| b.0.cmp(&a.0));
        for (_, path) in files.into_iter().take(512) {
            if let Some(id) = path.file_stem().and_then(|s| s.to_str()) {
                let _ = store.existing(id);
            }
        }
        store
    }

    fn path(&self, id: &str) -> Result<PathBuf> {
        uuid::Uuid::parse_str(id)?;
        Ok(self.root.join(format!("{id}.json")))
    }

    fn persist(&self, operation: &Operation) -> Result<()> {
        crate::integrations::hook_edit::atomic_write_private(
            &self.path(&operation.id)?,
            &serde_json::to_vec(operation)?,
        )
    }

    fn existing_locked(&self, inner: &mut Inner, id: &str) -> Result<Option<Operation>> {
        if let Some(operation) = inner.operations.get(id) {
            return Ok(Some(operation.clone()));
        }
        let path = self.path(id)?;
        if !path.exists() {
            return Ok(None);
        }
        if std::fs::metadata(&path)?.len() > 1024 * 1024 {
            bail!("control receipt is too large");
        }
        let mut operation: Operation = serde_json::from_slice(&std::fs::read(path)?)?;
        if operation.id != id {
            bail!("control receipt identity differs");
        }
        if matches!(
            operation.status.as_str(),
            "queued" | "dispatched" | "accepted"
        ) {
            operation.status = "unknown".into();
            self.persist(&operation)?;
        }
        inner.operations.insert(id.into(), operation.clone());
        Ok(Some(operation))
    }

    pub fn existing(&self, id: &str) -> Result<Option<Operation>> {
        self.existing_locked(&mut self.inner.lock().unwrap(), id)
    }

    pub fn begin(&self, mut operation: Operation) -> Result<Operation> {
        let mut inner = self.inner.lock().unwrap();
        if let Some(previous) = self.existing_locked(&mut inner, &operation.id)? {
            if previous.kind != operation.kind
                || previous.session_id != operation.session_id
                || previous.phone_id != operation.phone_id
                || previous.text != operation.text
                || previous.expected_result_id != operation.expected_result_id
                || serde_json::to_value(&previous.files)? != serde_json::to_value(&operation.files)?
            {
                bail!("control operation conflicts with its receipt");
            }
            return Ok(previous);
        }
        if inner.operations.values().any(|other| {
            other.kind == operation.kind
                && other.session_id == operation.session_id
                && matches!(other.status.as_str(), "queued" | "dispatched" | "accepted")
        }) {
            bail!("another message is waiting to start");
        }
        operation.status = "queued".into();
        self.persist(&operation)?;
        inner
            .operations
            .insert(operation.id.clone(), operation.clone());
        Ok(operation)
    }

    pub fn finish(&self, id: &str, status: &str, turn_id: Option<String>) -> Result<()> {
        let mut inner = self.inner.lock().unwrap();
        let Some(operation) = inner.operations.get_mut(id) else {
            bail!("unknown control operation");
        };
        if operation.status == "started" {
            return Ok(());
        }
        if matches!(operation.status.as_str(), "rejected" | "unknown") && status != "started" {
            return Ok(());
        }
        operation.status = status.into();
        if turn_id.is_some() {
            operation.turn_id = turn_id;
        }
        self.persist(operation)?;
        self.changed.notify_waiters();
        Ok(())
    }

    pub fn reject(&self, id: &str, reason: &str) -> Result<()> {
        let mut inner = self.inner.lock().unwrap();
        let operation = inner
            .operations
            .get_mut(id)
            .ok_or_else(|| anyhow::anyhow!("unknown control operation"))?;
        if operation.status == "started" {
            return Ok(());
        }
        operation.status = "rejected".into();
        operation.reason = Some(reason.into());
        self.persist(operation)?;
        self.changed.notify_waiters();
        Ok(())
    }

    pub fn started(
        &self,
        kind: AgentKind,
        session_id: &str,
        id: Option<&str>,
        turn_id: Option<&str>,
    ) -> Option<Operation> {
        let mut inner = self.inner.lock().unwrap();
        let operation = inner.operations.values_mut().find(|operation| {
            operation.kind == kind
                && operation.session_id == session_id
                && operation.status != "queued"
                && operation.status != "rejected"
                && (id == Some(operation.id.as_str())
                    || turn_id.is_some() && operation.turn_id.as_deref() == turn_id)
        })?;
        if operation.status == "started" {
            return None;
        }
        operation.status = "started".into();
        operation.turn_id = turn_id
            .map(str::to_owned)
            .or_else(|| operation.turn_id.clone());
        if self.persist(operation).is_err() {
            return None;
        }
        let operation = operation.clone();
        self.changed.notify_waiters();
        Some(operation)
    }

    pub fn cancel_unsubmitted(&self, kind: AgentKind, session_id: &str) {
        let mut inner = self.inner.lock().unwrap();
        for operation in inner.operations.values_mut().filter(|operation| {
            operation.kind == kind
                && operation.session_id == session_id
                && operation.status == "queued"
        }) {
            operation.status = "rejected".into();
            operation.reason = Some("stateChanged".into());
            let _ = self.persist(operation);
        }
        self.changed.notify_waiters();
    }

    pub fn pi_generation(&self, session_id: &str) -> Option<String> {
        self.inner
            .lock()
            .unwrap()
            .peers
            .get(session_id)
            .filter(|peer| peer.idle && peer.seen.elapsed() < Duration::from_secs(10))
            .map(|peer| peer.generation.clone())
    }

    pub fn pending(&self, kind: AgentKind, session_id: &str) -> bool {
        self.inner
            .lock()
            .unwrap()
            .operations
            .values()
            .any(|operation| {
                operation.kind == kind
                    && operation.session_id == session_id
                    && matches!(
                        operation.status.as_str(),
                        "queued" | "dispatched" | "accepted"
                    )
            })
    }

    pub fn message_operation(
        &self,
        kind: AgentKind,
        session_id: &str,
        message_ids: &[&str],
    ) -> Option<String> {
        self.inner
            .lock()
            .unwrap()
            .operations
            .values()
            .find(|operation| {
                operation.kind == kind
                    && operation.session_id == session_id
                    && operation.status != "queued"
                    && operation.status != "rejected"
                    && message_ids.contains(&operation.id.as_str())
            })
            .map(|operation| operation.id.clone())
    }

    pub fn poll(&self, session_id: &str, signal: &ControlSignal) -> Result<ControlReply> {
        if session_id.is_empty()
            || signal.pid == 0
            || uuid::Uuid::parse_str(&signal.generation).is_err()
        {
            bail!("invalid native control peer");
        }
        let mut inner = self.inner.lock().unwrap();
        if inner
            .retired
            .contains(&(session_id.into(), signal.generation.clone()))
        {
            bail!("control peer was replaced");
        }
        if let Some(previous) = inner.peers.get(session_id) {
            if previous.generation != signal.generation {
                let retired = (session_id.into(), previous.generation.clone());
                inner.retired.insert(retired);
                for operation in inner.operations.values_mut().filter(|operation| {
                    operation.session_id == session_id
                        && operation.generation.as_deref() != Some(signal.generation.as_str())
                        && matches!(
                            operation.status.as_str(),
                            "queued" | "dispatched" | "accepted"
                        )
                }) {
                    operation.status = if operation.status == "queued" {
                        "rejected"
                    } else {
                        "unknown"
                    }
                    .into();
                    if operation.status == "rejected" {
                        operation.reason = Some("nativeUnavailable".into());
                    }
                    self.persist(operation)?;
                }
                self.changed.notify_waiters();
            } else if previous.pid != signal.pid {
                bail!("control peer process differs");
            }
        }
        inner.peers.insert(
            session_id.into(),
            Peer {
                generation: signal.generation.clone(),
                pid: signal.pid,
                idle: signal.idle,
                seen: Instant::now(),
            },
        );
        if let Some(id) = signal.operation_id.as_ref() {
            let Some(operation) = inner.operations.get_mut(id) else {
                bail!("unknown control acknowledgment");
            };
            if operation.session_id != session_id
                || operation.generation.as_deref() != Some(signal.generation.as_str())
            {
                bail!("control acknowledgment belongs to another session");
            }
            if operation.status != "started" && operation.status == "dispatched" {
                operation.status = match signal.status.as_deref() {
                    Some("accepted") => "accepted",
                    Some("rejected") => "rejected",
                    _ => "unknown",
                }
                .into();
                if operation.status == "rejected" {
                    operation.reason = Some("stateChanged".into());
                }
                operation.turn_id = signal.turn_id.clone();
                self.persist(operation)?;
                self.changed.notify_waiters();
            }
            return Ok(ControlReply::default());
        }
        if !signal.idle {
            for operation in inner.operations.values_mut().filter(|operation| {
                operation.kind == AgentKind::Pi
                    && operation.session_id == session_id
                    && operation.status == "queued"
            }) {
                operation.status = "rejected".into();
                operation.reason = Some("stateChanged".into());
                self.persist(operation)?;
            }
            self.changed.notify_waiters();
            return Ok(ControlReply::default());
        }
        let Some(operation) = inner.operations.values_mut().find(|operation| {
            operation.kind == AgentKind::Pi
                && operation.session_id == session_id
                && operation.status == "queued"
                && operation.generation.as_deref() == Some(signal.generation.as_str())
        }) else {
            return Ok(ControlReply::default());
        };
        operation.status = "dispatched".into();
        self.persist(operation)?;
        Ok(ControlReply {
            operation: Some(ControlCommand {
                id: operation.id.clone(),
                text: operation.text.clone(),
                files: operation.files.clone(),
            }),
        })
    }

    pub async fn wait_for_submission(&self, id: &str) -> Result<Operation> {
        let until = tokio::time::Instant::now() + Duration::from_secs(5);
        loop {
            let notified = self.changed.notified();
            let operation = self
                .existing(id)?
                .ok_or_else(|| anyhow::anyhow!("unknown control operation"))?;
            if !matches!(
                operation.status.as_str(),
                "queued" | "dispatched" | "accepted"
            ) {
                return Ok(operation);
            }
            if tokio::time::timeout_at(until, notified).await.is_err() {
                {
                    let mut inner = self.inner.lock().unwrap();
                    if let Some(current) = inner.operations.get_mut(id) {
                        if matches!(
                            current.status.as_str(),
                            "queued" | "dispatched" | "accepted"
                        ) {
                            current.status = if current.status == "queued" {
                                "rejected"
                            } else {
                                "unknown"
                            }
                            .into();
                            if current.status == "rejected" {
                                current.reason = Some("nativeUnavailable".into());
                            }
                            self.persist(current)?;
                        }
                    }
                }
                return self
                    .existing(id)?
                    .ok_or_else(|| anyhow::anyhow!("control receipt disappeared"));
            }
        }
    }
}

pub fn run() {
    let Some(input) = super::report::read_stdin_json() else {
        println!("{{}}");
        return;
    };
    let session_id = input
        .get("session_id")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let output = serde_json::from_value::<ControlSignal>(input.clone())
        .ok()
        .and_then(|signal| crate::client::agent_control_request(session_id, signal))
        .unwrap_or_default();
    println!(
        "{}",
        serde_json::to_string(&output).unwrap_or_else(|_| "{}".into())
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn operation(id: String, generation: &str) -> Operation {
        Operation {
            id,
            kind: AgentKind::Pi,
            session_id: "s".into(),
            phone_id: "phone".into(),
            generation: Some(generation.into()),
            text: "continue".into(),
            files: vec![],
            expected_result_id: Some("result-id".into()),
            status: "queued".into(),
            reason: None,
            turn_id: None,
        }
    }

    #[test]
    fn a_competing_turn_cancels_the_undispatched_message_instead_of_queuing_it_behind_work() {
        let dir = tempfile::tempdir().unwrap();
        let store = ControlStore::new(dir.path().into());
        let generation = uuid::Uuid::new_v4().to_string();
        let mut signal = ControlSignal {
            generation: generation.clone(),
            pid: 123,
            idle: true,
            operation_id: None,
            status: None,
            turn_id: None,
        };
        store.poll("s", &signal).unwrap();
        let id = uuid::Uuid::new_v4().to_string();
        store.begin(operation(id.clone(), &generation)).unwrap();
        store.cancel_unsubmitted(AgentKind::Pi, "s");
        assert_eq!(store.existing(&id).unwrap().unwrap().status, "rejected");
        assert!(store.poll("s", &signal).unwrap().operation.is_none());
        let id = uuid::Uuid::new_v4().to_string();
        store.begin(operation(id.clone(), &generation)).unwrap();
        signal.idle = false;
        store.poll("s", &signal).unwrap();
        signal.idle = true;
        assert!(store.poll("s", &signal).unwrap().operation.is_none());
        assert_eq!(
            store.existing(&id).unwrap().unwrap().reason.as_deref(),
            Some("stateChanged")
        );
    }

    #[test]
    fn a_command_is_dispatched_once_and_an_ack_cannot_regress_a_real_start() {
        let dir = tempfile::tempdir().unwrap();
        let store = ControlStore::new(dir.path().into());
        let generation = uuid::Uuid::new_v4().to_string();
        let mut signal = ControlSignal {
            generation: generation.clone(),
            pid: 123,
            idle: true,
            operation_id: None,
            status: None,
            turn_id: None,
        };
        store.poll("s", &signal).unwrap();
        let id = uuid::Uuid::new_v4().to_string();
        store.begin(operation(id.clone(), &generation)).unwrap();
        assert_eq!(store.poll("s", &signal).unwrap().operation.unwrap().id, id);
        assert!(store.poll("s", &signal).unwrap().operation.is_none());
        store
            .started(AgentKind::Pi, "s", Some(&id), Some("turn"))
            .unwrap();
        signal.operation_id = Some(id.clone());
        signal.status = Some("accepted".into());
        store.poll("s", &signal).unwrap();
        assert_eq!(store.existing(&id).unwrap().unwrap().status, "started");
    }

    #[test]
    fn restarting_does_not_replay_a_dispatched_command() {
        let dir = tempfile::tempdir().unwrap();
        let store = ControlStore::new(dir.path().into());
        let generation = uuid::Uuid::new_v4().to_string();
        let signal = ControlSignal {
            generation: generation.clone(),
            pid: 123,
            idle: true,
            operation_id: None,
            status: None,
            turn_id: None,
        };
        store.poll("s", &signal).unwrap();
        let id = uuid::Uuid::new_v4().to_string();
        store.begin(operation(id.clone(), &generation)).unwrap();
        store.poll("s", &signal).unwrap();
        let recovered = ControlStore::new(dir.path().into());
        assert_eq!(recovered.existing(&id).unwrap().unwrap().status, "unknown");
        assert!(recovered.poll("s", &signal).unwrap().operation.is_none());
    }

    #[test]
    fn busy_and_replaced_sessions_do_not_receive_the_message() {
        let dir = tempfile::tempdir().unwrap();
        let store = ControlStore::new(dir.path().into());
        let generation = uuid::Uuid::new_v4().to_string();
        let mut signal = ControlSignal {
            generation: generation.clone(),
            pid: 123,
            idle: false,
            operation_id: None,
            status: None,
            turn_id: None,
        };
        store.poll("s", &signal).unwrap();
        let id = uuid::Uuid::new_v4().to_string();
        store.begin(operation(id, &generation)).unwrap();
        assert!(store.poll("s", &signal).unwrap().operation.is_none());
        signal.generation = uuid::Uuid::new_v4().to_string();
        signal.idle = true;
        assert!(store.poll("s", &signal).unwrap().operation.is_none());
        signal.generation = generation;
        assert!(store.poll("s", &signal).is_err());
    }
}
