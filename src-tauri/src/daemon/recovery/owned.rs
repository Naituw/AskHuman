//! Durable identities and consumption state for daemon-owned request continuations.
use super::*;

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum OwnedEffect {
    Waiting,
    Submitting,
    Complete,
    Unknown,
    Cancelled,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OwnedCall {
    pub call: Call,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task: Option<Task>,
    pub effect: OwnedEffect,
    pub touched_ms: i64,
}

fn metadata_key(key: &str) -> String {
    format!("owned/{key}")
}

fn read(connection: &Connection, key: &str) -> Result<Option<OwnedCall>> {
    let value: Option<String> = connection
        .query_row(
            "SELECT payload FROM metadata WHERE key=?1",
            [metadata_key(key)],
            |row| row.get(0),
        )
        .optional()?;
    value
        .map(|value| serde_json::from_str(&value).map_err(Into::into))
        .transpose()
}

fn write(connection: &Connection, key: &str, record: &OwnedCall) -> Result<()> {
    connection.execute("INSERT INTO metadata(key,payload) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET payload=excluded.payload", params![metadata_key(key), serde_json::to_string(record)?])?;
    connection.execute("INSERT INTO metadata(key,payload) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET payload=excluded.payload", params![format!("consumer/{}", record.call.id), serde_json::to_string(&record.effect)?])?;
    Ok(())
}

impl Store {
    pub fn recover_owned_effects(&self) -> Result<()> {
        let records = self.owned_records("")?;
        let mut connection = self.connection.lock().unwrap();
        let transaction = connection.transaction()?;
        for (key, mut record) in records {
            if record.effect != OwnedEffect::Submitting {
                continue;
            }
            record.effect = OwnedEffect::Unknown;
            record.task = None;
            record.touched_ms = chrono::Utc::now().timestamp_millis();
            write(&transaction, &key, &record)?;
            transaction.execute(
                "UPDATE calls SET delivered=1 WHERE id=?1",
                [&record.call.id],
            )?;
        }
        transaction.commit()?;
        Ok(())
    }

    pub fn owned(&self, key: &str, task: &Task) -> Result<OwnedCall> {
        let mut connection = self.connection.lock().unwrap();
        let transaction = connection.transaction()?;
        if let Some(saved) = read(&transaction, key)? {
            let cancelled = lookup(&transaction, &saved.call, None)?
                .is_some_and(|call| matches!(call.outcome, Some(Outcome::Cancelled)));
            if saved.effect != OwnedEffect::Cancelled
                && (saved.effect != OwnedEffect::Waiting || !cancelled)
            {
                return Ok(saved);
            }
        }
        let mut call = Call::new();
        call.daemon_owned = true;
        let record = OwnedCall {
            call,
            task: Some(task.clone()),
            effect: OwnedEffect::Waiting,
            touched_ms: chrono::Utc::now().timestamp_millis(),
        };
        write(&transaction, key, &record)?;
        transaction.commit()?;
        Ok(record)
    }

    pub fn owned_exists(&self, key: &str) -> Result<bool> {
        Ok(read(&self.connection.lock().unwrap(), key)?.is_some())
    }

    pub fn cancel_owned(&self, key: &str) -> Result<()> {
        let record = {
            let mut connection = self.connection.lock().unwrap();
            let transaction = connection.transaction()?;
            let Some(mut record) = read(&transaction, key)? else {
                return Ok(());
            };
            if record.effect != OwnedEffect::Waiting {
                return Ok(());
            }
            let decided =
                lookup(&transaction, &record.call, None)?.is_some_and(|call| call.decided);
            record.effect = if decided {
                OwnedEffect::Complete
            } else {
                OwnedEffect::Cancelled
            };
            record.task = None;
            record.touched_ms = chrono::Utc::now().timestamp_millis();
            write(&transaction, key, &record)?;
            transaction.commit()?;
            record
        };
        record.call.persist_cancellation()
    }

    pub fn owned_records(&self, prefix: &str) -> Result<Vec<(String, OwnedCall)>> {
        let connection = self.connection.lock().unwrap();
        let mut statement =
            connection.prepare("SELECT key,payload FROM metadata WHERE key LIKE ?1")?;
        let rows = statement.query_map([format!("owned/{prefix}%")], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        rows.map(|row| {
            let (key, payload) = row?;
            Ok((
                key.trim_start_matches("owned/").to_string(),
                serde_json::from_str(&payload)?,
            ))
        })
        .collect()
    }

    /// Only one continuation may begin the external native operation.
    pub fn claim_owned(&self, key: &str) -> Result<bool> {
        let mut connection = self.connection.lock().unwrap();
        let transaction = connection.transaction()?;
        anyhow::ensure!(!handoff_in_progress(), "daemon update is in progress");
        let mut saved = read(&transaction, key)?.context("owned request is missing")?;
        if saved.effect != OwnedEffect::Waiting {
            return Ok(false);
        }
        saved.effect = OwnedEffect::Submitting;
        saved.touched_ms = chrono::Utc::now().timestamp_millis();
        write(&transaction, key, &saved)?;
        transaction.commit()?;
        Ok(true)
    }

    pub fn complete_owned(&self, key: &str, success: bool) -> Result<()> {
        let mut connection = self.connection.lock().unwrap();
        let transaction = connection.transaction()?;
        let mut saved = read(&transaction, key)?.context("owned request is missing")?;
        let call = lookup(&transaction, &saved.call, None)?.context("owned call is missing")?;
        anyhow::ensure!(call.outcome.is_some(), "owned call has no terminal result");
        saved.effect = if success {
            OwnedEffect::Complete
        } else {
            OwnedEffect::Unknown
        };
        saved.touched_ms = chrono::Utc::now().timestamp_millis();
        saved.task = None;
        write(&transaction, key, &saved)?;
        transaction.execute("UPDATE calls SET delivered=1 WHERE id=?1", [&saved.call.id])?;
        transaction.commit()?;
        Ok(())
    }

    pub fn pending_daemon_results(&self) -> Result<bool> {
        self.connection.lock().unwrap().query_row(
            "SELECT EXISTS(SELECT 1 FROM calls c JOIN requests r ON r.id=c.request_id JOIN metadata m ON m.key='consumer/'||c.id
             WHERE c.daemon_owned=1 AND c.cancelled=0 AND c.delivered=0 AND m.payload=?1 AND (r.decision IS NOT NULL OR r.outcome IS NOT NULL))",
            [serde_json::to_string(&OwnedEffect::Submitting)?], |row| row.get(0),
        ).map_err(Into::into)
    }
}

pub(super) fn prune(connection: &Connection, cutoff: i64) -> Result<()> {
    let records: Vec<(String, String)> = {
        let mut statement =
            connection.prepare("SELECT key,payload FROM metadata WHERE key LIKE 'owned/%'")?;
        let rows = statement.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    for (key, payload) in records {
        let record: OwnedCall = serde_json::from_str(&payload)?;
        if record.touched_ms >= cutoff
            || matches!(
                record.effect,
                OwnedEffect::Waiting | OwnedEffect::Submitting
            )
        {
            continue;
        }
        let waiting: bool = connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM calls WHERE id=?1 AND cancelled=0 AND delivered=0)",
            [&record.call.id],
            |row| row.get(0),
        )?;
        if !waiting {
            connection.execute(
                "DELETE FROM metadata WHERE key=?1 OR key=?2",
                params![key, format!("consumer/{}", record.call.id)],
            )?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn task(text: &str) -> Task {
        Task::Ask(Box::new(
            serde_json::from_value(json!({
                "message":{"text":text},"questions":[],"isMarkdown":true,
                "source":"Codex App","lang":"en","project":"/project"
            }))
            .unwrap(),
        ))
    }

    fn accept(store: &Store, owned: &OwnedCall) {
        let request = Request {
            id: owned.call.id.clone(),
            task: owned.task.clone().unwrap(),
            show: None,
            created_at_ms: 1,
        };
        store.accept(&request, &owned.call, "native task").unwrap();
        store.decide(&request.id, &json!({"answer":"yes"})).unwrap();
        store
            .finish(
                &request.id,
                &Outcome::Ask(crate::app::RenderOutcome {
                    stdout: "yes".into(),
                    stderr: None,
                    exit_code: 0,
                }),
            )
            .unwrap();
    }

    #[test]
    fn a_native_consumer_reopens_the_original_call_and_question() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("requests.sqlite3");
        let first = {
            let store = Store::open(&path).unwrap();
            store.owned("codex/request", &task("original")).unwrap()
        };
        let store = Store::open(&path).unwrap();
        let restored = store
            .owned("codex/request", &task("changed locale or presentation"))
            .unwrap();
        assert_eq!(restored.call.id, first.call.id);
        assert_eq!(
            serde_json::to_value(restored.task).unwrap(),
            serde_json::to_value(first.task).unwrap()
        );
    }

    #[test]
    fn handoff_waits_only_for_a_claimed_native_operation() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("requests.sqlite3");
        let store = Store::open(&path).unwrap();
        let owned = store.owned("codex/request", &task("question")).unwrap();
        accept(&store, &owned);
        assert!(
            !store.pending_daemon_results().unwrap(),
            "a durable answer may wait for its consumer across a handoff"
        );
        assert!(store.claim_owned("codex/request").unwrap());
        assert!(!store.claim_owned("codex/request").unwrap());
        assert!(store.pending_daemon_results().unwrap());
        store.complete_owned("codex/request", true).unwrap();
        assert!(!store.pending_daemon_results().unwrap());
        let completed = store.owned("codex/request", &task("different")).unwrap();
        assert!(completed.task.is_none());
        assert!(completed.effect == OwnedEffect::Complete);
        assert!(store.resume(&owned.call, None).unwrap().unwrap().delivered);
    }

    #[test]
    fn an_interrupted_native_send_is_never_dispatched_again() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("requests.sqlite3");
        {
            let store = Store::open(&path).unwrap();
            let owned = store.owned("codex/request", &task("question")).unwrap();
            accept(&store, &owned);
            assert!(store.claim_owned("codex/request").unwrap());
        }
        let store = Store::open(&path).unwrap();
        store.recover_owned_effects().unwrap();
        let restored = store.owned("codex/request", &task("question")).unwrap();
        assert!(restored.effect == OwnedEffect::Unknown);
        assert!(!store.claim_owned("codex/request").unwrap());
        assert!(!store.pending_daemon_results().unwrap());
    }

    #[test]
    fn an_unanswered_native_request_can_return_with_a_fresh_identity_after_cancellation() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("requests.sqlite3")).unwrap();
        let first = store.owned("codex/request", &task("question")).unwrap();
        let request = Request {
            id: first.call.id.clone(),
            task: first.task.unwrap(),
            show: None,
            created_at_ms: 1,
        };
        store.accept(&request, &first.call, "task").unwrap();
        store.cancel_call(&first.call).unwrap();
        let returned = store.owned("codex/request", &task("question")).unwrap();
        assert_ne!(returned.call.id, first.call.id);
    }
}
