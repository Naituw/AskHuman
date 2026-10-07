//! Durable operation receipts. Only a caller's explicit resubmission can resume creation.
use super::{runtime, save};
use serde_json::{json, Value};
use std::path::PathBuf;

pub(super) struct Action {
    path: PathBuf,
    pub value: Value,
}

impl Action {
    pub fn load(path: PathBuf, input: Value) -> Result<Self, String> {
        let value = if path.exists() {
            serde_json::from_slice::<Value>(&std::fs::read(&path).map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())?
        } else {
            json!({"input":input,"status":"prepared"})
        };
        if value["input"] != input {
            return Err("An operation ID cannot be reused for different content".into());
        }
        Ok(Self { path, value })
    }

    pub fn receipt(&self) -> Result<Option<Value>, String> {
        match self.value["status"].as_str() {
            Some("accepted") => Ok(Some(self.value.clone())),
            Some("prepared" | "rejected") if self.session_id().is_none() => Ok(None),
            Some("created") if self.session_id().is_some() => Ok(None),
            // Old unknown records remain unknown even when an ID is available.
            _ => Err(self.describe("Operation outcome is unknown")),
        }
    }

    pub fn session_id(&self) -> Option<&str> {
        self.value["sessionId"].as_str()
    }

    pub fn persist(&self) -> Result<(), String> {
        save(&self.path, &self.value)
    }

    pub fn describe(&self, error: &str) -> String {
        let guidance = match self.value["status"].as_str() {
            Some("rejected" | "prepared") => "No task was submitted. You can retry this operation.",
            Some("created") => "The chat was created, but no task was submitted. Retry this operation to continue in the same chat.",
            _ => "Inspect the original chat before sending another request; the outcome is unknown.",
        };
        let session = self
            .session_id()
            .or_else(|| self.value["failure"]["sessionId"].as_str());
        format!(
            "{error}\nOperation: {}{}\n{guidance}",
            self.value["input"]["id"].as_str().unwrap_or_default(),
            session
                .map(|id| format!("\nSession: {id}"))
                .unwrap_or_default()
        )
    }

    pub fn fail_before_submit(&mut self, stage: &str, error: &str) -> String {
        self.value["status"] = json!(if self.session_id().is_some() {
            "created"
        } else {
            "rejected"
        });
        self.value["failure"] = json!({"stage":stage,"message":error});
        match self.persist() {
            Ok(()) => self.describe(error),
            Err(save_error) => self.describe(&format!(
                "{error}\nCould not save operation state: {save_error}"
            )),
        }
    }

    pub fn ensure_thread(
        &mut self,
        installation: &runtime::Installation,
        cwd: &str,
        title: &str,
        yolo: bool,
    ) -> Result<String, String> {
        self.ensure_thread_with(|existing, on_created| match existing {
            Some(id) => runtime::finish_creation(installation, cwd, title, id),
            None => runtime::create(installation, cwd, title, yolo, on_created),
        })
    }

    fn ensure_thread_with(
        &mut self,
        run: impl FnOnce(
            Option<&str>,
            &mut dyn FnMut(&str) -> Result<(), String>,
        ) -> Result<String, runtime::CreationError>,
    ) -> Result<String, String> {
        let existing = self.session_id().map(str::to_owned);
        if self.value["runtimeReady"] == true {
            if let Some(id) = existing {
                return Ok(id);
            }
        }
        if existing.is_none() {
            self.value["status"] = json!("unknown");
            self.value["stage"] = json!("creating");
            self.persist()?;
        }
        let result = run(existing.as_deref(), &mut |id| {
            self.value["sessionId"] = json!(id);
            self.value["status"] = json!("created");
            self.value["stage"] = json!("prepare-thread");
            self.persist()
        });
        match result {
            Ok(id) => {
                self.value["sessionId"] = json!(id);
                self.value["status"] = json!("created");
                self.value["runtimeReady"] = json!(true);
                self.value["stage"] = json!("activate");
                self.value.as_object_mut().unwrap().remove("failure");
                self.persist()?;
                Ok(id)
            }
            Err(error) => {
                self.value["status"] = json!(error.outcome);
                self.value["failure"] = json!(error);
                // A failed ID write may leave the previous unknown record on disk. Keep
                // its diagnostic ID, but never treat that write failure as resumable.
                if error.outcome == runtime::CreationOutcome::Unknown {
                    self.value.as_object_mut().unwrap().remove("sessionId");
                }
                match self.persist() {
                    Ok(()) => Err(self.describe(&error.message)),
                    Err(save_error) => Err(self.describe(&format!(
                        "{}\nCould not save operation state: {save_error}",
                        error.message
                    ))),
                }
            }
        }
    }

    pub fn submitting(&mut self, session_id: &str, method: &str) -> Result<(), String> {
        self.value["sessionId"] = json!(session_id);
        self.value["status"] = json!("unknown");
        self.value["stage"] = json!(method);
        self.value.as_object_mut().unwrap().remove("failure");
        self.persist()
    }

    pub fn accepted(&mut self) -> Result<Value, String> {
        self.value["status"] = json!("accepted");
        self.value["stage"] = json!("accepted");
        self.persist().map_err(|e| format!("The App accepted the operation, but its receipt could not be saved: {e}. Check the original chat before retrying."))?;
        Ok(self.value.clone())
    }

    pub async fn submit<F: std::future::Future<Output = Result<Value, String>>>(
        &mut self,
        session_id: &str,
        method: &str,
        send: impl FnOnce() -> F,
    ) -> Result<Value, String> {
        if let Some(receipt) = self.receipt()? {
            return Ok(receipt);
        }
        self.submitting(session_id, method)?;
        if let Err(error) = send().await {
            self.value["failure"] = json!({"stage":"submit","method":method,"message":error});
            let _ = self.persist();
            return Err(self.describe(&error));
        }
        self.accepted()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use runtime::{CreationError, CreationOutcome};

    fn input() -> Value {
        json!({"op":"create","id":"operation","text":"task"})
    }
    fn error(outcome: CreationOutcome, id: Option<&str>) -> CreationError {
        CreationError {
            message: "injected error".into(),
            stage: "response".into(),
            method: Some("thread/start".into()),
            code: Some(-32602),
            session_id: id.map(str::to_owned),
            outcome,
        }
    }

    #[test]
    fn rejected_retry_and_created_resume_never_duplicate_a_thread_or_first_message() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("op.json");
        let mut action = Action::load(path.clone(), input()).unwrap();
        let mut starts = 0;
        action
            .ensure_thread_with(|id, _| {
                assert!(id.is_none());
                starts += 1;
                Err(error(CreationOutcome::Rejected, None))
            })
            .unwrap_err();
        assert_eq!(action.value["status"], "rejected");
        let mut action = Action::load(path.clone(), input()).unwrap();
        assert!(action.receipt().unwrap().is_none());
        action
            .ensure_thread_with(|id, created| {
                assert!(id.is_none());
                starts += 1;
                created("thread").unwrap();
                Err(error(CreationOutcome::Created, Some("thread")))
            })
            .unwrap_err();
        let mut action = Action::load(path.clone(), input()).unwrap();
        assert_eq!(action.value["status"], "created");
        action
            .ensure_thread_with(|id, _| {
                assert_eq!(id, Some("thread"));
                Ok("thread".into())
            })
            .unwrap();
        action.fail_before_submit("activate", "App not ready");
        let mut action = Action::load(path.clone(), input()).unwrap();
        action
            .ensure_thread_with(|_, _| panic!("runtime must not run again after activation failed"))
            .unwrap();
        action
            .submitting("thread", "thread-follower-start-turn")
            .unwrap();
        // Lost acknowledgment cannot cause either a new thread or a repeated message.
        assert!(Action::load(path.clone(), input())
            .unwrap()
            .receipt()
            .is_err());
        action.accepted().unwrap();
        let duplicate = Action::load(path.clone(), input())
            .unwrap()
            .receipt()
            .unwrap()
            .unwrap();
        assert_eq!(duplicate["sessionId"], "thread");
        assert_eq!(starts, 2);
        assert!(Action::load(path, json!({"different":"content"})).is_err());
    }

    #[test]
    fn historical_unknown_and_id_persistence_failure_remain_blocked() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("op.json");
        for id in [Value::Null, json!("thread")] {
            save(
                &path,
                &json!({"input":input(),"status":"unknown","sessionId":id}),
            )
            .unwrap();
            let action = Action::load(path.clone(), input()).unwrap();
            assert!(action.receipt().is_err());
        }
        std::fs::remove_file(&path).unwrap();
        let mut action = Action::load(path.clone(), input()).unwrap();
        action
            .ensure_thread_with(|_, created| {
                created("thread").unwrap();
                Err(error(CreationOutcome::Unknown, Some("thread")))
            })
            .unwrap_err();
        assert_eq!(action.value["failure"]["sessionId"], "thread");
        assert!(Action::load(path, input()).unwrap().receipt().is_err());
    }

    #[tokio::test]
    async fn accepted_duplicates_and_lost_acknowledgments_do_not_resubmit_messages() {
        for lost_ack in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("op.json");
            let mut action = Action::load(path.clone(), input()).unwrap();
            let mut messages = 0;
            let result = action
                .submit("thread", "thread-follower-start-turn", || {
                    messages += 1;
                    std::future::ready(if lost_ack {
                        Err("disconnected".into())
                    } else {
                        Ok(json!({}))
                    })
                })
                .await;
            assert_eq!(result.is_ok(), !lost_ack);
            let mut duplicate = Action::load(path, input()).unwrap();
            let retried = duplicate
                .submit("thread", "thread-follower-start-turn", || {
                    messages += 1;
                    std::future::ready(Ok(json!({})))
                })
                .await;
            assert_eq!(retried.is_ok(), !lost_ack);
            assert_eq!(messages, 1);
        }
    }
}
