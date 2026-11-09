//! Durable request identities, terminal results, and channel checkpoints across daemon updates.

mod owned;
mod process;
pub use owned::{OwnedCall, OwnedEffect};
pub(crate) use process::{alive as process_alive, birth as process_birth};

use anyhow::{bail, Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub const VERSION: u32 = 1;
const SCHEMA_VERSION: u32 = 5;
pub fn capabilities() -> Value {
    serde_json::json!({"recovery":VERSION, "minSchema":1, "maxSchema":SCHEMA_VERSION})
}
static DAEMON_GENERATION: AtomicUsize = AtomicUsize::new(0);
static DAEMON_PROCESS: AtomicBool = AtomicBool::new(false);
static HANDOFF: AtomicBool = AtomicBool::new(false);

pub fn enter_daemon() {
    DAEMON_GENERATION.fetch_add(1, Ordering::SeqCst);
    DAEMON_PROCESS.store(true, Ordering::SeqCst);
}

pub fn daemon_generation() -> usize {
    DAEMON_GENERATION.load(Ordering::SeqCst)
}

pub fn set_handoff(active: bool) {
    HANDOFF.store(active, Ordering::SeqCst);
}

pub fn handoff_in_progress() -> bool {
    HANDOFF.load(Ordering::SeqCst)
}

/// A caller keeps this identity for the entire submission, including pre-accept retries.
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Call {
    pub id: String,
    pub secret: String,
    pub version: u32,
    pub pid: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub birth: Option<String>,
    #[serde(default)]
    pub daemon_owned: bool,
    #[serde(default)]
    pub legacy: bool,
}

impl std::fmt::Debug for Call {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Call")
            .field("id", &self.id)
            .field("version", &self.version)
            .finish_non_exhaustive()
    }
}

impl Call {
    pub fn new() -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            secret: uuid::Uuid::new_v4().to_string(),
            version: VERSION,
            pid: std::process::id(),
            birth: (!DAEMON_PROCESS.load(Ordering::SeqCst))
                .then(|| process::birth(std::process::id()))
                .flatten(),
            daemon_owned: DAEMON_PROCESS.load(Ordering::SeqCst),
            legacy: false,
        }
    }

    fn validate(&self) -> Result<()> {
        if self.version != VERSION
            || uuid::Uuid::parse_str(&self.id).is_err()
            || uuid::Uuid::parse_str(&self.secret).is_err()
        {
            bail!("unsupported or invalid recovery identity");
        }
        Ok(())
    }

    fn secret_hash(&self) -> String {
        format!("{:x}", Sha256::digest(self.secret.as_bytes()))
    }

    pub fn persist_cancellation(&self) -> Result<()> {
        self.validate()?;
        crate::integrations::hook_edit::atomic_write_private(
            &cancellation_directory().join(format!("{}.json", self.id)),
            &serde_json::to_vec(self)?,
        )
    }

    pub fn forget_cancellation(&self) {
        if self.validate().is_ok() {
            let _ =
                std::fs::remove_file(cancellation_directory().join(format!("{}.json", self.id)));
        }
    }
}

fn cancellation_directory() -> std::path::PathBuf {
    crate::paths::config_dir().join("state/request-cancellations")
}

impl Default for Call {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", content = "task", rename_all = "camelCase")]
pub enum Task {
    Ask(Box<crate::ipc::TaskRequest>),
    Confirm(Box<crate::ipc::ConfirmTask>),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum ClientOperation {
    Popup {
        token: String,
        pid: u32,
        request_id: Option<String>,
    },
    PopupHost {
        token: String,
        pid: u32,
        known_ids: Vec<String>,
    },
    WarmPopup {
        pid: u32,
    },
    Submit {
        call: Call,
        task: Task,
    },
    Resume {
        call: Call,
    },
    Cancel {
        call: Call,
    },
    Delivered {
        call: Call,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(
    tag = "event",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum Event {
    PopupLease { token: String },
    Updating,
    Accepted { request_id: String },
    Final { outcome: Outcome },
    Updated,
    Unavailable,
}

/// The original presentation is stored so restoration cannot change the question's meaning.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Request {
    pub id: String,
    pub task: Task,
    pub show: Option<crate::ipc::ShowPayload>,
    pub created_at_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", content = "result", rename_all = "camelCase")]
pub enum Outcome {
    Ask(crate::app::RenderOutcome),
    Confirm(crate::models::ConfirmResult),
    Fallback(crate::models::ConfirmFallbackReason),
    Cancelled,
}

#[derive(Debug, Clone)]
pub struct Resumed {
    pub request_id: String,
    pub outcome: Option<Outcome>,
    pub decided: bool,
    pub cancelled: bool,
    pub delivered: bool,
}

pub struct Saved {
    pub finished_at_ms: Option<u64>,
    pub request: Request,
    pub decision: Option<Value>,
    pub outcome: Option<Outcome>,
}

pub struct ProcessCall {
    pub birth: Option<String>,
    pub id: String,
    pub pid: u32,
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct Receipt {
    pub phone_id: String,
    pub client_msg_id: String,
    pub hash: String,
    pub result: crate::ios::store::JournalResult,
    pub at_ms: i64,
}

#[derive(Default, Serialize, Deserialize)]
struct Routing {
    ids: Vec<String>,
}

/// Shared by a restored coordinator and its delivery surfaces.
pub struct RequestContext {
    pub store: Arc<Store>,
    pub id: String,
    pub restoring: bool,
    suspended: AtomicBool,
    operations: AtomicUsize,
    ready: AtomicBool,
    recovering: AtomicBool,
    pause_draining: AtomicBool,
    pending: Mutex<std::collections::HashMap<String, serde_json::Map<String, Value>>>,
    routing: Mutex<()>,
}

impl std::fmt::Debug for RequestContext {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RequestContext")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}

impl RequestContext {
    pub fn new(store: Arc<Store>, id: String, restoring: bool) -> Arc<Self> {
        Arc::new(Self {
            store,
            id,
            restoring,
            suspended: AtomicBool::new(false),
            operations: AtomicUsize::new(0),
            ready: AtomicBool::new(false),
            recovering: AtomicBool::new(restoring),
            pause_draining: AtomicBool::new(false),
            pending: Mutex::new(std::collections::HashMap::new()),
            routing: Mutex::new(()),
        })
    }

    pub fn load<T: serde::de::DeserializeOwned>(&self, channel: &str) -> Result<Option<T>> {
        let pending = self.pending.lock().unwrap();
        let mut saved = self.store.load_checkpoint(&self.id, channel)?;
        if let Some(patch) = pending.get(channel) {
            let mut state = saved
                .take()
                .and_then(|value| value.as_object().cloned())
                .unwrap_or_default();
            state.extend(patch.clone());
            saved = Some(Value::Object(state));
        }
        saved
            .map(serde_json::from_value)
            .transpose()
            .map_err(Into::into)
    }

    pub fn save<T: Serialize>(&self, channel: &str, state: &T) -> Result<()> {
        let Value::Object(patch) = serde_json::to_value(state)? else {
            bail!("request checkpoints must be objects");
        };
        let mut pending = self.pending.lock().unwrap();
        let combined = pending.entry(channel.to_string()).or_default();
        combined.extend(patch);
        self.store.patch_checkpoint(&self.id, channel, combined)?;
        pending.remove(channel);
        Ok(())
    }

    pub fn flush(&self) -> Result<()> {
        let mut pending = self.pending.lock().unwrap();
        let channels = pending.keys().cloned().collect::<Vec<_>>();
        for channel in channels {
            self.store
                .patch_checkpoint(&self.id, &channel, &pending[&channel])?;
            pending.remove(&channel);
        }
        Ok(())
    }

    pub fn resume(&self) {
        self.suspended.store(false, Ordering::SeqCst);
    }

    pub fn begin_recovery(&self) {
        self.recovering.store(true, Ordering::SeqCst);
    }

    pub fn is_recovering(&self) -> bool {
        self.recovering.load(Ordering::SeqCst)
    }

    pub fn mark_ready(&self) {
        self.ready.store(true, Ordering::SeqCst);
    }

    pub fn is_ready(&self) -> bool {
        self.ready.load(Ordering::SeqCst)
    }

    pub fn channels(&self) -> Result<Vec<String>> {
        Ok(self.load::<Routing>("routing")?.unwrap_or_default().ids)
    }

    pub fn remember_channel(&self, channel: &str) -> Result<()> {
        let _routing = self.routing.lock().unwrap();
        let mut routing = self.load::<Routing>("routing")?.unwrap_or_default();
        if !routing.ids.iter().any(|id| id == channel) {
            routing.ids.push(channel.to_string());
            self.save("routing", &routing)?;
        }
        Ok(())
    }

    pub fn suspend(&self) {
        self.pause_draining.store(true, Ordering::SeqCst);
        self.suspended.store(true, Ordering::SeqCst);
    }

    pub fn can_restart_surfaces(&self) -> bool {
        if self.is_suspended() {
            return false;
        }
        if !self.pause_draining.load(Ordering::SeqCst) {
            return true;
        }
        if !self.is_quiescent() {
            return false;
        }
        self.pause_draining.store(false, Ordering::SeqCst);
        true
    }

    pub fn is_suspended(&self) -> bool {
        self.suspended.load(Ordering::SeqCst)
    }

    pub fn is_quiescent(&self) -> bool {
        self.operations.load(Ordering::SeqCst) == 0
    }

    pub fn operation(self: &Arc<Self>) -> Option<Operation> {
        self.operations.fetch_add(1, Ordering::SeqCst);
        if self.is_suspended() {
            self.operations.fetch_sub(1, Ordering::SeqCst);
            return None;
        }
        Some(Operation(self.clone()))
    }
}

/// An update waits for in-flight submissions and external writes to leave a recoverable boundary.
pub struct Operation(Arc<RequestContext>);

impl Drop for Operation {
    fn drop(&mut self) {
        self.0.operations.fetch_sub(1, Ordering::SeqCst);
    }
}

/// Owned by one daemon. Every acknowledgement follows a committed SQLite transaction.
pub struct Store {
    connection: Mutex<Connection>,
}

impl Store {
    pub fn discard_unowned(&self) -> Result<()> {
        self.connection.lock().unwrap().execute(
            "DELETE FROM requests WHERE decision IS NULL AND NOT EXISTS(SELECT 1 FROM calls WHERE calls.request_id=requests.id)",
            [],
        )?;
        Ok(())
    }

    pub fn open(path: &Path) -> Result<Self> {
        let parent = path.parent().context("recovery database has no parent")?;
        std::fs::create_dir_all(parent)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700))?;
        }
        // Create the database privately before SQLite has a chance to create it with the umask.
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create(true).truncate(false);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        drop(options.open(path)?);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
        }
        let mut connection = Connection::open(path)?;
        connection.busy_timeout(Duration::from_secs(2))?;
        connection.execute_batch("PRAGMA foreign_keys=ON; PRAGMA synchronous=FULL;")?;
        let version: u32 = connection.query_row("PRAGMA user_version", [], |row| row.get(0))?;
        if version > SCHEMA_VERSION {
            bail!("recovery database requires a newer reader (schema {version})");
        }
        if version == 0 {
            let transaction = connection.transaction()?;
            transaction.execute_batch(
                "CREATE TABLE requests (
                    id TEXT PRIMARY KEY,
                    payload TEXT NOT NULL,
                    decision TEXT,
                    outcome TEXT,
                    created_at_ms INTEGER NOT NULL
                );
                CREATE TABLE calls (
                    id TEXT PRIMARY KEY,
                    secret_hash TEXT NOT NULL,
                    request_id TEXT NOT NULL REFERENCES requests(id),
                    task_hash TEXT NOT NULL,
                    caller_pid INTEGER NOT NULL,
                    daemon_owned INTEGER NOT NULL DEFAULT 0,
                    legacy INTEGER NOT NULL DEFAULT 0,
                    cancelled INTEGER NOT NULL DEFAULT 0,
                    delivered INTEGER NOT NULL DEFAULT 0
                );
                CREATE INDEX calls_request ON calls(request_id);
                CREATE TABLE cancelled_calls (
                    id TEXT PRIMARY KEY,
                    secret_hash TEXT NOT NULL,
                    at_ms INTEGER NOT NULL
                );
                CREATE TABLE checkpoints (
                    request_id TEXT NOT NULL REFERENCES requests(id) ON DELETE CASCADE,
                    channel TEXT NOT NULL,
                    payload TEXT NOT NULL,
                    PRIMARY KEY (request_id, channel)
                );
                PRAGMA user_version=1;",
            )?;
            transaction.commit()?;
        }
        if version < 2 {
            connection.execute_batch(
                "BEGIN IMMEDIATE;
                CREATE TABLE metadata (key TEXT PRIMARY KEY, payload TEXT NOT NULL);
                CREATE TABLE receipts (
                    id TEXT PRIMARY KEY, phone_id TEXT NOT NULL, hash TEXT NOT NULL,
                    result TEXT NOT NULL, at_ms INTEGER NOT NULL
                );
                PRAGMA user_version=2;
                COMMIT;",
            )?;
        }
        if version < 3 {
            connection.execute_batch(
                "BEGIN IMMEDIATE;
                CREATE TABLE completed (
                    request_id TEXT PRIMARY KEY REFERENCES requests(id) ON DELETE CASCADE,
                    at_ms INTEGER NOT NULL
                );
                INSERT INTO completed(request_id,at_ms) SELECT id,created_at_ms FROM requests WHERE outcome IS NOT NULL;
                PRAGMA user_version=3;
                COMMIT;",
            )?;
        }
        if version < 4 {
            connection.execute_batch("BEGIN IMMEDIATE; ALTER TABLE calls ADD COLUMN birth TEXT; PRAGMA user_version=4; COMMIT;")?;
        }
        if version < 5 {
            // Early recovery builds used schema 1 before these fields were added.
            // Repair those databases even if later migrations already advanced them.
            let transaction = connection.transaction()?;
            for column in ["daemon_owned", "legacy"] {
                let exists: bool = transaction.query_row(
                    "SELECT EXISTS(SELECT 1 FROM pragma_table_info('calls') WHERE name=?1)",
                    [column],
                    |row| row.get(0),
                )?;
                if !exists {
                    transaction.execute_batch(&format!(
                        "ALTER TABLE calls ADD COLUMN {column} INTEGER NOT NULL DEFAULT 0;"
                    ))?;
                }
            }
            transaction.execute_batch(
                "CREATE TABLE IF NOT EXISTS cancelled_calls (
                    id TEXT PRIMARY KEY,
                    secret_hash TEXT NOT NULL,
                    at_ms INTEGER NOT NULL
                );
                CREATE INDEX IF NOT EXISTS calls_request ON calls(request_id);
                PRAGMA user_version=5;",
            )?;
            transaction.commit()?;
        }
        Ok(Self {
            connection: Mutex::new(connection),
        })
    }

    /// Atomically publish a request and its initial waiter. A retry cannot create another card.
    pub fn accept(&self, request: &Request, call: &Call, task_hash: &str) -> Result<Resumed> {
        call.validate()?;
        let payload = serde_json::to_string(request)?;
        let mut connection = self.connection.lock().unwrap();
        let transaction = connection.transaction()?;
        require_uncancelled(&transaction, call)?;
        if let Some(existing) = lookup(&transaction, call, Some(task_hash))? {
            return Ok(existing);
        }
        transaction.execute(
            "INSERT INTO requests(id,payload,created_at_ms) VALUES(?1,?2,?3)",
            params![request.id, payload, request.created_at_ms],
        )?;
        transaction.execute(
            "INSERT INTO calls(id,secret_hash,request_id,task_hash,caller_pid,daemon_owned,legacy,birth) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",
            params![call.id, call.secret_hash(), request.id, task_hash, call.pid, call.daemon_owned, call.legacy, call.birth],
        )?;
        transaction.commit()?;
        Ok(Resumed {
            request_id: request.id.clone(),
            outcome: None,
            decided: false,
            cancelled: false,
            delivered: false,
        })
    }

    /// Attach an independently authenticated caller to an existing coalesced request.
    pub fn attach(&self, request_id: &str, call: &Call, task_hash: &str) -> Result<Resumed> {
        call.validate()?;
        let mut connection = self.connection.lock().unwrap();
        let transaction = connection.transaction()?;
        require_uncancelled(&transaction, call)?;
        if let Some(existing) = lookup(&transaction, call, Some(task_hash))? {
            if existing.request_id != request_id {
                bail!("recovery call already belongs to another request");
            }
            return Ok(existing);
        }
        transaction.execute(
            "INSERT INTO calls(id,secret_hash,request_id,task_hash,caller_pid,daemon_owned,legacy,birth) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",
            params![call.id, call.secret_hash(), request_id, task_hash, call.pid, call.daemon_owned, call.legacy, call.birth],
        )?;
        let resumed = lookup(&transaction, call, Some(task_hash))?
            .context("attached recovery call is missing")?;
        transaction.commit()?;
        Ok(resumed)
    }

    pub fn resume(&self, call: &Call, task_hash: Option<&str>) -> Result<Option<Resumed>> {
        call.validate()?;
        lookup(&self.connection.lock().unwrap(), call, task_hash)
    }

    pub fn present(&self, request: &Request) -> Result<()> {
        let payload = serde_json::to_string(request)?;
        let connection = self.connection.lock().unwrap();
        let changed = connection.execute(
            "UPDATE requests SET payload=?2 WHERE id=?1 AND outcome IS NULL",
            params![request.id, payload],
        )?;
        if changed != 1 {
            bail!("cannot present an unknown or finished recovery request");
        }
        Ok(())
    }

    pub fn active_calls(&self, request_id: &str) -> Result<u64> {
        self.connection
            .lock()
            .unwrap()
            .query_row(
                "SELECT COUNT(*) FROM calls WHERE request_id=?1 AND cancelled=0 AND delivered=0",
                [request_id],
                |row| row.get(0),
            )
            .map_err(Into::into)
    }

    pub fn awaiting_channel(&self, channel: &str) -> Result<Vec<String>> {
        let connection = self.connection.lock().unwrap();
        let mut statement = connection.prepare("SELECT r.id,c.payload FROM requests r JOIN checkpoints c ON c.request_id=r.id AND c.channel='routing' WHERE r.outcome IS NULL AND r.decision IS NULL AND EXISTS(SELECT 1 FROM calls w WHERE w.request_id=r.id AND w.cancelled=0 AND w.delivered=0)")?;
        let rows = statement.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        let mut ids = Vec::new();
        for row in rows {
            let (id, payload) = row?;
            let routing: Routing = serde_json::from_str(&payload)?;
            if routing.ids.iter().any(|id| id == channel) {
                ids.push(id);
            }
        }
        Ok(ids)
    }

    pub fn has_legacy_waiters(&self) -> Result<bool> {
        self.connection
            .lock()
            .unwrap()
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM calls WHERE legacy=1 AND cancelled=0 AND delivered=0)",
                [],
                |row| row.get(0),
            )
            .map_err(Into::into)
    }

    pub fn coalesce(&self, from: &str, into: &str) -> Result<()> {
        let mut connection = self.connection.lock().unwrap();
        let transaction = connection.transaction()?;
        require_request(&transaction, into)?;
        transaction.execute(
            "UPDATE calls SET request_id=?2 WHERE request_id=?1",
            params![from, into],
        )?;
        transaction.commit()?;
        Ok(())
    }

    /// Record the first terminal submission before telling any answering surface it succeeded.
    pub fn decide(&self, request_id: &str, decision: &Value) -> Result<bool> {
        self.decide_with_receipt(request_id, decision, None)
    }

    pub fn decide_with_receipt(
        &self,
        request_id: &str,
        decision: &Value,
        receipt: Option<&Receipt>,
    ) -> Result<bool> {
        let decision = serde_json::to_string(decision)?;
        let mut connection = self.connection.lock().unwrap();
        let transaction = connection.transaction()?;
        let changed = transaction.execute(
            "UPDATE requests SET decision=?2 WHERE id=?1 AND decision IS NULL AND outcome IS NULL",
            params![request_id, decision],
        )?;
        if changed == 0 {
            require_request(&transaction, request_id)?;
        } else if let Some(receipt) = receipt {
            transaction.execute(
                "INSERT INTO receipts(id,phone_id,hash,result,at_ms) VALUES(?1,?2,?3,?4,?5)",
                params![
                    receipt.client_msg_id,
                    receipt.phone_id,
                    receipt.hash,
                    serde_json::to_string(&receipt.result)?,
                    receipt.at_ms
                ],
            )?;
        }
        transaction.commit()?;
        Ok(changed == 1)
    }

    pub fn receipt(
        &self,
        id: &str,
        phone: &str,
        hash: &str,
    ) -> Result<crate::ios::store::JournalLookup> {
        use crate::ios::store::JournalLookup;
        let connection = self.connection.lock().unwrap();
        let saved: Option<(String, String, String)> = connection
            .query_row(
                "SELECT phone_id,hash,result FROM receipts WHERE id=?1",
                [id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?;
        match saved {
            None => Ok(JournalLookup::New),
            Some((saved_phone, saved_hash, result))
                if saved_phone == phone && saved_hash == hash =>
            {
                Ok(JournalLookup::Replay(Some(serde_json::from_str(&result)?)))
            }
            Some(_) => Ok(JournalLookup::Conflict),
        }
    }

    pub fn metadata<T: serde::de::DeserializeOwned>(&self, key: &str) -> Result<Option<T>> {
        let value: Option<String> = self
            .connection
            .lock()
            .unwrap()
            .query_row("SELECT payload FROM metadata WHERE key=?1", [key], |row| {
                row.get(0)
            })
            .optional()?;
        value
            .map(|value| serde_json::from_str(&value).map_err(Into::into))
            .transpose()
    }

    pub fn set_metadata<T: Serialize>(&self, key: &str, value: &T) -> Result<()> {
        self.connection.lock().unwrap().execute(
            "INSERT INTO metadata(key,payload) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET payload=excluded.payload",
            params![key, serde_json::to_string(value)?],
        )?;
        Ok(())
    }

    /// Final results are immutable, including during a retried finalizer.
    pub fn finish(&self, request_id: &str, outcome: &Outcome) -> Result<()> {
        let encoded = serde_json::to_string(outcome)?;
        let mut connection = self.connection.lock().unwrap();
        let transaction = connection.transaction()?;
        let (existing, decided): (Option<String>, bool) = transaction
            .query_row(
                "SELECT outcome,decision IS NOT NULL FROM requests WHERE id=?1",
                [request_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?
            .context("unknown recovery request")?;
        if matches!(outcome, Outcome::Cancelled) && decided {
            bail!("an accepted answer cannot be cancelled");
        }
        if existing.as_ref().is_some_and(|value| value != &encoded) {
            bail!("recovery request already has a different terminal result");
        }
        transaction.execute(
            "UPDATE requests SET outcome=?2 WHERE id=?1 AND outcome IS NULL",
            params![request_id, encoded],
        )?;
        transaction.execute("INSERT INTO completed(request_id,at_ms) VALUES(?1,?2) ON CONFLICT(request_id) DO NOTHING", params![request_id, chrono::Utc::now().timestamp_millis()])?;
        transaction.commit()?;
        Ok(())
    }

    pub fn requests(&self) -> Result<Vec<Saved>> {
        let connection = self.connection.lock().unwrap();
        let mut statement = connection
            .prepare("SELECT payload,decision,outcome,completed.at_ms FROM requests LEFT JOIN completed ON completed.request_id=requests.id ORDER BY created_at_ms,id")?;
        let rows = statement.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, Option<u64>>(3)?,
            ))
        })?;
        rows.map(|row| {
            let (request, decision, outcome, finished_at_ms) = row?;
            Ok(Saved {
                finished_at_ms,
                request: serde_json::from_str(&request)?,
                decision: decision
                    .map(|value| serde_json::from_str(&value))
                    .transpose()?,
                outcome: outcome
                    .map(|value| serde_json::from_str(&value))
                    .transpose()?,
            })
        })
        .collect()
    }

    pub fn request(&self, request_id: &str) -> Result<Option<Saved>> {
        let row: Option<(String, Option<String>, Option<String>, Option<u64>)> = self
            .connection
            .lock()
            .unwrap()
            .query_row(
                "SELECT payload,decision,outcome,completed.at_ms FROM requests LEFT JOIN completed ON completed.request_id=requests.id WHERE requests.id=?1",
                [request_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .optional()?;
        row.map(|(request, decision, outcome, finished_at_ms)| {
            Ok(Saved {
                finished_at_ms,
                request: serde_json::from_str(&request)?,
                decision: decision
                    .map(|value| serde_json::from_str(&value))
                    .transpose()?,
                outcome: outcome
                    .map(|value| serde_json::from_str(&value))
                    .transpose()?,
            })
        })
        .transpose()
    }

    pub fn checkpoint(&self, request_id: &str, channel: &str, payload: &Value) -> Result<()> {
        let encoded = serde_json::to_string(payload)?;
        self.connection.lock().unwrap().execute(
            "INSERT INTO checkpoints(request_id,channel,payload) VALUES(?1,?2,?3)
             ON CONFLICT(request_id,channel) DO UPDATE SET payload=excluded.payload",
            params![request_id, channel, encoded],
        )?;
        Ok(())
    }

    fn patch_checkpoint(
        &self,
        request_id: &str,
        channel: &str,
        patch: &serde_json::Map<String, Value>,
    ) -> Result<()> {
        let mut connection = self.connection.lock().unwrap();
        let transaction = connection.transaction()?;
        let existing: Option<String> = transaction
            .query_row(
                "SELECT payload FROM checkpoints WHERE request_id=?1 AND channel=?2",
                params![request_id, channel],
                |row| row.get(0),
            )
            .optional()?;
        let mut state: serde_json::Map<String, Value> = match existing {
            Some(value) => serde_json::from_str(&value)?,
            None => Default::default(),
        };
        state.extend(patch.clone());
        transaction.execute(
            "INSERT INTO checkpoints(request_id,channel,payload) VALUES(?1,?2,?3)
             ON CONFLICT(request_id,channel) DO UPDATE SET payload=excluded.payload",
            params![request_id, channel, serde_json::to_string(&state)?],
        )?;
        transaction.commit()?;
        Ok(())
    }

    pub fn load_checkpoint(&self, request_id: &str, channel: &str) -> Result<Option<Value>> {
        let encoded: Option<String> = self
            .connection
            .lock()
            .unwrap()
            .query_row(
                "SELECT payload FROM checkpoints WHERE request_id=?1 AND channel=?2",
                params![request_id, channel],
                |row| row.get(0),
            )
            .optional()?;
        encoded
            .map(|value| serde_json::from_str(&value).map_err(Into::into))
            .transpose()
    }

    pub fn cancel_call(&self, call: &Call) -> Result<Option<String>> {
        call.validate()?;
        let mut connection = self.connection.lock().unwrap();
        let transaction = connection.transaction()?;
        let Some(resumed) = lookup(&transaction, call, None)? else {
            transaction.execute(
                "INSERT INTO cancelled_calls(id,secret_hash,at_ms) VALUES(?1,?2,?3) ON CONFLICT(id) DO NOTHING",
                params![call.id, call.secret_hash(), chrono::Utc::now().timestamp_millis()],
            )?;
            transaction.commit()?;
            return Ok(None);
        };
        let cancelled = cancel_waiter(
            &transaction,
            &call.id,
            &resumed.request_id,
            resumed.outcome.is_some(),
            resumed.decided,
        )?;
        transaction.commit()?;
        Ok(cancelled.then_some(resumed.request_id))
    }

    pub fn process_calls(&self) -> Result<Vec<ProcessCall>> {
        let connection = self.connection.lock().unwrap();
        let mut statement = connection.prepare(
            "SELECT id,caller_pid,birth FROM calls WHERE cancelled=0 AND delivered=0 AND daemon_owned=0 AND legacy=0",
        )?;
        let rows = statement.query_map([], |row| {
            Ok(ProcessCall {
                id: row.get(0)?,
                pid: row.get(1)?,
                birth: row.get(2)?,
            })
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    /// The daemon calls this only after proving the recorded owner process has exited.
    pub fn cancel_absent_call(&self, id: &str) -> Result<Option<String>> {
        let mut connection = self.connection.lock().unwrap();
        let transaction = connection.transaction()?;
        let record: Option<(String, bool, bool)> = transaction
            .query_row(
                "SELECT c.request_id,r.outcome IS NOT NULL,r.decision IS NOT NULL
             FROM calls c JOIN requests r ON r.id=c.request_id
             WHERE c.id=?1 AND c.cancelled=0 AND c.delivered=0 AND c.daemon_owned=0",
                [id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?;
        let Some((request_id, finished, decided)) = record else {
            return Ok(None);
        };
        let cancelled = cancel_waiter(&transaction, id, &request_id, finished, decided)?;
        transaction.commit()?;
        Ok(cancelled.then_some(request_id))
    }

    pub fn consume_cancellations(&self) -> Result<Vec<String>> {
        self.consume_cancellations_at(&cancellation_directory())
    }

    fn consume_cancellations_at(&self, directory: &Path) -> Result<Vec<String>> {
        let entries = match std::fs::read_dir(directory) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(error.into()),
        };
        let mut cancelled = Vec::new();
        for entry in entries {
            let path = entry?.path();
            if path.extension().and_then(|extension| extension.to_str()) != Some("json") {
                continue;
            }
            if path.metadata()?.len() > 4096 {
                bail!("oversized request cancellation record");
            }
            let call: Call = serde_json::from_slice(&std::fs::read(&path)?)?;
            call.validate()?;
            if path.file_stem().and_then(|stem| stem.to_str()) != Some(call.id.as_str()) {
                bail!("request cancellation identity does not match its file");
            }
            if let Some(id) = self.cancel_call(&call)? {
                cancelled.push(id);
            }
            std::fs::remove_file(path)?;
        }
        Ok(cancelled)
    }

    pub fn prune(&self, now: i64) -> Result<()> {
        let mut connection = self.connection.lock().unwrap();
        let transaction = connection.transaction()?;
        transaction.execute("UPDATE calls SET delivered=1 WHERE daemon_owned=1 AND delivered=0 AND NOT EXISTS(SELECT 1 FROM metadata m WHERE m.key='consumer/'||calls.id) AND EXISTS(SELECT 1 FROM requests r WHERE r.id=calls.request_id AND r.outcome IS NOT NULL)", [])?;
        let ids: Vec<String> = {
            let mut statement = transaction.prepare(
                "SELECT r.id FROM requests r JOIN completed d ON r.id=d.request_id
                 WHERE d.at_ms<?1 AND NOT EXISTS(SELECT 1 FROM calls c WHERE c.request_id=r.id AND c.cancelled=0 AND c.delivered=0)",
            )?;
            let rows = statement.query_map([now - 300_000], |row| row.get(0))?;
            rows.collect::<rusqlite::Result<_>>()?
        };
        for id in ids {
            transaction.execute("DELETE FROM metadata WHERE key IN (SELECT 'consumer/'||id FROM calls WHERE request_id=?1)", [&id])?;
            transaction.execute("DELETE FROM calls WHERE request_id=?1", [&id])?;
            transaction.execute("DELETE FROM requests WHERE id=?1", [&id])?;
        }
        let cutoff = now - crate::ios::wire::JOURNAL_MAX_AGE_DAYS * 24 * 3600 * 1000;
        transaction.execute("DELETE FROM receipts WHERE at_ms<?1", [cutoff])?;
        transaction.execute("DELETE FROM receipts WHERE id IN (SELECT id FROM receipts ORDER BY at_ms DESC LIMIT -1 OFFSET ?1)", [crate::ios::wire::JOURNAL_MAX_ENTRIES as i64])?;
        transaction.execute("DELETE FROM cancelled_calls WHERE at_ms<?1", [cutoff])?;
        owned::prune(&transaction, cutoff)?;
        transaction.commit()?;
        Ok(())
    }

    pub fn delivered(&self, call: &Call) -> Result<()> {
        call.validate()?;
        let mut connection = self.connection.lock().unwrap();
        let transaction = connection.transaction()?;
        let resumed = lookup(&transaction, call, None)?.context("unknown recovery call")?;
        if resumed.outcome.is_none() {
            bail!("cannot acknowledge an unfinished request");
        }
        transaction.execute("UPDATE calls SET delivered=1 WHERE id=?1", [&call.id])?;
        transaction.commit()?;
        Ok(())
    }
}

fn require_request(connection: &Connection, request_id: &str) -> Result<()> {
    let exists: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM requests WHERE id=?1)",
        [request_id],
        |row| row.get(0),
    )?;
    if !exists {
        bail!("unknown recovery request");
    }
    Ok(())
}

fn require_uncancelled(connection: &Connection, call: &Call) -> Result<()> {
    let cancelled: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM cancelled_calls WHERE id=?1)",
        [&call.id],
        |row| row.get(0),
    )?;
    if cancelled {
        bail!("the caller cancelled this recovery call");
    }
    Ok(())
}

fn cancel_waiter(
    connection: &Connection,
    call_id: &str,
    request_id: &str,
    finished: bool,
    decided: bool,
) -> Result<bool> {
    connection.execute("UPDATE calls SET cancelled=1 WHERE id=?1", [call_id])?;
    let remaining: u64 = connection.query_row(
        "SELECT COUNT(*) FROM calls WHERE request_id=?1 AND cancelled=0 AND delivered=0",
        [request_id],
        |row| row.get(0),
    )?;
    let cancelled = remaining == 0 && !finished && !decided;
    if cancelled {
        connection.execute(
            "UPDATE requests SET outcome=?2 WHERE id=?1 AND decision IS NULL AND outcome IS NULL",
            params![request_id, serde_json::to_string(&Outcome::Cancelled)?],
        )?;
        connection.execute("INSERT INTO completed(request_id,at_ms) VALUES(?1,?2) ON CONFLICT(request_id) DO NOTHING", params![request_id, chrono::Utc::now().timestamp_millis()])?;
    }
    Ok(cancelled)
}

fn lookup(
    connection: &Connection,
    call: &Call,
    task_hash: Option<&str>,
) -> Result<Option<Resumed>> {
    let row: Option<(String, String, String, bool, bool, Option<String>, bool)> = connection
        .query_row(
            "SELECT c.secret_hash,c.task_hash,c.request_id,c.cancelled,c.delivered,r.outcome,r.decision IS NOT NULL
         FROM calls c JOIN requests r ON r.id=c.request_id WHERE c.id=?1",
            [&call.id],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                    row.get(6)?,
                ))
            },
        )
        .optional()?;
    let Some((secret_hash, saved_hash, request_id, cancelled, delivered, outcome, decided)) = row
    else {
        return Ok(None);
    };
    if secret_hash != call.secret_hash() || task_hash.is_some_and(|hash| hash != saved_hash) {
        bail!("recovery identity or request content does not match");
    }
    Ok(Some(Resumed {
        request_id,
        decided,
        outcome: outcome
            .map(|value| serde_json::from_str(&value))
            .transpose()?,
        cancelled,
        delivered,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn make_request() -> Request {
        let task: crate::ipc::TaskRequest = serde_json::from_value(json!({
            "message": {"text": ""},
            "questions": [{"message": "Question", "predefinedOptions": []}],
            "isMarkdown": false,
            "source": "test",
            "lang": "en"
        }))
        .unwrap();
        let ask =
            crate::models::AskRequest::new(task.message.clone(), task.questions.clone(), false);
        let show = serde_json::from_value(json!({
            "requestId": ask.id,
            "interaction": crate::models::InteractionRequest::Ask(ask.clone()),
            "source": "test", "lang": "en", "createdAtMs": 123
        }))
        .unwrap();
        Request {
            id: ask.id,
            task: Task::Ask(Box::new(task)),
            show: Some(show),
            created_at_ms: 123,
        }
    }

    fn outcome(text: &str) -> Outcome {
        Outcome::Ask(crate::app::RenderOutcome {
            stdout: text.into(),
            stderr: None,
            exit_code: 0,
        })
    }

    #[test]
    fn accepted_mobile_receipts_survive_the_journal_ack_gap() {
        use crate::ios::store::{JournalLookup, JournalResult};
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("requests.sqlite3");
        let request = make_request();
        let receipt = Receipt {
            phone_id: "phone".into(),
            client_msg_id: "message".into(),
            hash: "body".into(),
            at_ms: 100,
            result: JournalResult {
                msg_type: "answer.ack".into(),
                body: json!({"status":"accepted"}),
            },
        };
        {
            let store = Store::open(&path).unwrap();
            store.accept(&request, &Call::new(), "task").unwrap();
            assert!(store
                .decide_with_receipt(&request.id, &json!({"answer":"yes"}), Some(&receipt))
                .unwrap());
            assert!(store.finish(&request.id, &Outcome::Cancelled).is_err());
        }
        let store = Store::open(&path).unwrap();
        assert_eq!(
            store.receipt("message", "phone", "body").unwrap(),
            JournalLookup::Replay(Some(receipt.result))
        );
        assert_eq!(
            store.receipt("message", "other", "body").unwrap(),
            JournalLookup::Conflict
        );
        assert!(!store.decide(&request.id, &json!({"answer":"no"})).unwrap());
    }

    #[test]
    fn completed_results_are_retained_until_waiters_acknowledge_them() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("requests.sqlite3")).unwrap();
        let request = make_request();
        let call = Call::new();
        store.accept(&request, &call, "task").unwrap();
        store.finish(&request.id, &outcome("answer")).unwrap();
        let finished = store
            .request(&request.id)
            .unwrap()
            .unwrap()
            .finished_at_ms
            .unwrap() as i64;
        store.prune(finished + 300_001).unwrap();
        assert!(store.resume(&call, None).unwrap().is_some());
        store.delivered(&call).unwrap();
        store.prune(finished + 299_999).unwrap();
        assert!(store.resume(&call, None).unwrap().is_some());
        store.prune(finished + 300_001).unwrap();
        assert!(store.request(&request.id).unwrap().is_none());
    }

    #[test]
    fn retry_before_accept_ack_finds_the_original_card() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("requests.sqlite3")).unwrap();
        let call = Call::new();
        let original = make_request();
        store.accept(&original, &call, "task").unwrap();
        let retry = store.accept(&make_request(), &call, "task").unwrap();
        assert_eq!(retry.request_id, original.id);
        assert_eq!(store.requests().unwrap().len(), 1);
        let mut impostor = call.clone();
        impostor.secret = Call::new().secret;
        assert!(store.resume(&impostor, None).is_err());
        assert!(store.resume(&call, Some("different task")).is_err());
        assert!(store
            .accept(&make_request(), &call, "different task")
            .is_err());
    }

    #[test]
    fn decisions_results_and_channel_state_survive_reopening() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("requests.sqlite3");
        let call = Call::new();
        let request = make_request();
        {
            let store = Store::open(&path).unwrap();
            store.accept(&request, &call, "task").unwrap();
            assert!(store
                .decide(&request.id, &json!({"answer": "first"}))
                .unwrap());
            assert!(!store
                .decide(&request.id, &json!({"answer": "second"}))
                .unwrap());
            store
                .checkpoint(
                    &request.id,
                    "feishu",
                    &json!({"messageId": "card", "index": 2}),
                )
                .unwrap();
            store.finish(&request.id, &outcome("first")).unwrap();
        }
        let store = Store::open(&path).unwrap();
        let saved = store.requests().unwrap().remove(0);
        assert_eq!(saved.request.id, request.id);
        assert_eq!(saved.decision.unwrap()["answer"], "first");
        assert!(matches!(saved.outcome, Some(Outcome::Ask(_))));
        assert_eq!(
            store
                .load_checkpoint(&request.id, "feishu")
                .unwrap()
                .unwrap()["messageId"],
            "card"
        );
        let resumed = store.resume(&call, Some("task")).unwrap().unwrap();
        assert!(
            matches!(resumed.outcome, Some(Outcome::Ask(ref value)) if value.stdout == "first")
        );
        store.finish(&request.id, &outcome("first")).unwrap();
        assert!(store.finish(&request.id, &outcome("second")).is_err());
        store.delivered(&call).unwrap();
        assert!(store.resume(&call, None).unwrap().unwrap().delivered);
    }

    #[test]
    fn cancelling_a_coalesced_waiter_keeps_other_callers_waiting() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("requests.sqlite3")).unwrap();
        let original = Call::new();
        let follower = Call::new();
        let request = make_request();
        store.accept(&request, &original, "original").unwrap();
        store.attach(&request.id, &follower, "follower").unwrap();
        assert_eq!(store.cancel_call(&original).unwrap(), None);
        assert!(store.resume(&original, None).unwrap().unwrap().cancelled);
        assert!(!store.resume(&follower, None).unwrap().unwrap().cancelled);
        assert!(store.delivered(&follower).is_err());
        assert_eq!(
            store.cancel_call(&follower).unwrap(),
            Some(request.id.clone())
        );
        assert!(store
            .attach(&make_request().id, &follower, "follower")
            .is_err());
    }

    #[test]
    fn early_recovery_schemas_are_repaired_without_losing_pending_or_answered_calls() {
        for version in 1..=5 {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("requests.sqlite3");
            let pending = make_request();
            let pending_call = Call::new();
            let answered = make_request();
            let answered_call = Call::new();
            {
                let connection = Connection::open(&path).unwrap();
                connection
                    .execute_batch(include_str!("recovery/fixtures/schema-v5.sql"))
                    .unwrap();
                for (request, call, hash) in [
                    (&pending, &pending_call, "pending"),
                    (&answered, &answered_call, "answered"),
                ] {
                    connection
                        .execute(
                            "INSERT INTO requests(id,payload,created_at_ms) VALUES(?1,?2,123)",
                            params![request.id, serde_json::to_string(request).unwrap()],
                        )
                        .unwrap();
                    connection.execute("INSERT INTO calls(id,secret_hash,request_id,task_hash,caller_pid,birth) VALUES(?1,?2,?3,?4,?5,?6)",
                        params![call.id, call.secret_hash(), request.id, hash, call.pid, call.birth]).unwrap();
                }
                connection
                    .execute(
                        "UPDATE requests SET decision=?1,outcome=?2 WHERE id=?3",
                        params![
                            json!({"answer": "yes"}).to_string(),
                            serde_json::to_string(&outcome("yes")).unwrap(),
                            answered.id
                        ],
                    )
                    .unwrap();
                connection
                    .execute(
                        "INSERT INTO completed(request_id,at_ms) VALUES(?1,123)",
                        [&answered.id],
                    )
                    .unwrap();
                connection
                    .execute(
                        "INSERT INTO checkpoints(request_id,channel,payload) VALUES(?1,'ios',?2)",
                        params![pending.id, json!({"card": "original"}).to_string()],
                    )
                    .unwrap();
                if version < 5 {
                    connection
                        .execute_batch(
                            "ALTER TABLE calls DROP COLUMN daemon_owned;
                    ALTER TABLE calls DROP COLUMN legacy;
                    DROP TABLE cancelled_calls;
                    DROP INDEX calls_request;",
                        )
                        .unwrap();
                }
                if version < 4 {
                    connection
                        .execute_batch("ALTER TABLE calls DROP COLUMN birth;")
                        .unwrap();
                }
                if version < 3 {
                    connection.execute_batch("DROP TABLE completed;").unwrap();
                }
                if version < 2 {
                    connection
                        .execute_batch("DROP TABLE metadata; DROP TABLE receipts;")
                        .unwrap();
                }
                connection
                    .pragma_update(None, "user_version", version)
                    .unwrap();
            }
            // Opening twice also covers an already repaired database on restart.
            for _ in 0..2 {
                let store = Store::open(&path).unwrap();
                assert_eq!(
                    store
                        .resume(&pending_call, None)
                        .unwrap()
                        .unwrap()
                        .request_id,
                    pending.id
                );
                assert!(store
                    .request(&pending.id)
                    .unwrap()
                    .unwrap()
                    .outcome
                    .is_none());
                assert!(
                    matches!(store.resume(&answered_call, None).unwrap().unwrap().outcome,
                    Some(Outcome::Ask(value)) if value.stdout == "yes")
                );
                assert_eq!(
                    store.load_checkpoint(&pending.id, "ios").unwrap().unwrap()["card"],
                    "original"
                );
                store.prune(crate::history::now_ms() as i64).unwrap();
                let cancelled = Call::new();
                store.cancel_call(&cancelled).unwrap();
                assert!(store.accept(&make_request(), &cancelled, "late").is_err());
                let fresh = Call::new();
                store.accept(&make_request(), &fresh, "new").unwrap();
            }
        }
    }

    #[test]
    fn unknown_request_and_future_schema_fail_explicitly() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("requests.sqlite3");
        {
            let store = Store::open(&path).unwrap();
            assert!(store.decide("missing", &json!({})).is_err());
            assert!(store.finish("missing", &outcome("answer")).is_err());
            assert!(store.checkpoint("missing", "ios", &json!({})).is_err());
        }
        Connection::open(&path)
            .unwrap()
            .pragma_update(None, "user_version", SCHEMA_VERSION + 1)
            .unwrap();
        assert!(Store::open(&path).is_err());
    }

    #[test]
    fn a_cancellation_saved_before_acceptance_prevents_a_late_card() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("requests.sqlite3")).unwrap();
        let call = Call::new();
        assert_eq!(store.cancel_call(&call).unwrap(), None);
        assert!(store.accept(&make_request(), &call, "late").is_err());
        assert!(store.requests().unwrap().is_empty());
    }

    #[test]
    fn cancellation_cannot_replace_a_saved_human_answer() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("requests.sqlite3")).unwrap();
        let call = Call::new();
        let request = make_request();
        store.accept(&request, &call, "task").unwrap();
        assert!(store
            .decide(&request.id, &json!({"answer": "yes"}))
            .unwrap());
        assert_eq!(store.cancel_call(&call).unwrap(), None);
        store.finish(&request.id, &outcome("yes")).unwrap();
        assert!(
            matches!(store.resume(&call, None).unwrap().unwrap().outcome, Some(Outcome::Ask(value)) if value.stdout == "yes")
        );
    }

    #[test]
    fn cancellation_files_survive_an_unavailable_daemon() {
        let dir = tempfile::tempdir().unwrap();
        let cancellations = dir.path().join("cancellations");
        let call = Call::new();
        crate::integrations::hook_edit::atomic_write_private(
            &cancellations.join(format!("{}.json", call.id)),
            &serde_json::to_vec(&call).unwrap(),
        )
        .unwrap();
        let store = Store::open(&dir.path().join("state/requests.sqlite3")).unwrap();
        assert!(store
            .consume_cancellations_at(&cancellations)
            .unwrap()
            .is_empty());
        assert!(store.accept(&make_request(), &call, "late").is_err());
        assert_eq!(std::fs::read_dir(cancellations).unwrap().count(), 0);
    }

    #[test]
    fn suspension_waits_for_operations_and_rejects_new_ones() {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(Store::open(&dir.path().join("requests.sqlite3")).unwrap());
        let context = RequestContext::new(store, "request".into(), false);
        let operation = context.operation().unwrap();
        context.suspend();
        assert!(!context.is_quiescent());
        assert!(context.operation().is_none());
        drop(operation);
        assert!(context.is_quiescent());
    }

    #[test]
    fn debugging_a_call_does_not_expose_its_recovery_secret() {
        let call = Call::new();
        assert!(!format!("{call:?}").contains(&call.secret));
    }

    #[cfg(unix)]
    #[test]
    fn state_files_are_private() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state/requests.sqlite3");
        let _store = Store::open(&path).unwrap();
        assert_eq!(path.metadata().unwrap().permissions().mode() & 0o777, 0o600);
        assert_eq!(
            path.parent()
                .unwrap()
                .metadata()
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
    }
    #[test]
    fn coalesced_shadow_records_are_removed_only_when_a_new_daemon_restores() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("requests.sqlite3")).unwrap();
        let root = make_request();
        let root_call = Call::new();
        store.accept(&root, &root_call, "root").unwrap();
        let shadow = make_request();
        let follower = Call::new();
        store.accept(&shadow, &follower, "follower").unwrap();
        store.coalesce(&shadow.id, &root.id).unwrap();
        store.discard_unowned().unwrap();
        assert!(store.request(&shadow.id).unwrap().is_none());
        assert_eq!(store.active_calls(&root.id).unwrap(), 2);
        store.finish(&root.id, &outcome("one answer")).unwrap();
        let saved = store.resume(&follower, Some("follower")).unwrap().unwrap();
        assert_eq!(saved.request_id, root.id);
        assert!(
            matches!(saved.outcome, Some(Outcome::Ask(result)) if result.stdout == "one answer")
        );
    }
}
