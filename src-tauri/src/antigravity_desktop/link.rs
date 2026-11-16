//! The private link between the daemon and the sidecars the App runs for it
//! (docs/specs/antigravity-deep-integration.md §12.3).
//!
//! A sidecar is a process the App starts; it can run `agentapi`, which the daemon cannot (the
//! tool only works inside a sidecar's environment). So the sidecar connects to the daemon, checks
//! in, and then does exactly what the daemon asks over that connection: one of a few fixed
//! `agentapi` calls, never a shell command and never anything it read from somewhere else.
//!
//! This module has the frames on the wire, the `agentapi` argument lists they turn into, and the
//! daemon's table of connected sidecars. The process on the sidecar's side is `sidecar_run`.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;
use tokio::sync::{mpsc, oneshot, Notify};

/// Changes whenever a sidecar and a daemon could misread each other's frames. A sidecar of
/// another version is told to restart (the App starts it again from the file on disk) or waits.
pub const LINK_VERSION: u32 = 1;

/// How long the daemon waits for one command's result.
pub const COMMAND_TIMEOUT: Duration = Duration::from_secs(45);

/// `agentapi` takes the prompt as one argument; the operating system's limit for all arguments is
/// about a megabyte, so a prompt is kept well below it.
pub const MAX_PROMPT_BYTES: usize = 200_000;
const MAX_TITLE_CHARS: usize = 200;

/// The conversation `Probe` asks about. It does not exist, so asking costs no model call.
const PROBE_CONVERSATION: &str = "00000000-0000-0000-0000-000000000000";

// MARK: - Frames

/// What a sidecar says first, and the daemon decides from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckIn {
    /// The sidecar's id, which is the name of its directory.
    pub id: String,
    pub link_version: u32,
    /// `ANTIGRAVITY_PROJECT_ID`: the App's project for this sidecar.
    #[serde(default)]
    pub project_id: Option<String>,
    /// The AskHuman version the sidecar process is running.
    #[serde(default)]
    pub version: String,
    /// `agentapi` was found where the sidecar looks for it.
    #[serde(default)]
    pub agentapi: bool,
    #[serde(default)]
    pub pid: u32,
}

/// Sidecar → daemon.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum Up {
    CheckIn(CheckIn),
    /// The result of the command `command_id`.
    Done {
        command_id: String,
        outcome: Outcome,
    },
    /// A frame of a kind this side does not know.
    #[serde(other)]
    Unknown,
}

/// Daemon → sidecar.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum Down {
    /// The check-in was accepted; commands may follow.
    Welcome,
    /// The daemon does not want this sidecar. With `restart` the sidecar should exit so the App
    /// starts it again from the file on disk (it is an older build); otherwise it waits and tries
    /// again later.
    Refused {
        reason: String,
        #[serde(default)]
        restart: bool,
    },
    Run {
        command_id: String,
        command: Command,
    },
    #[serde(other)]
    Unknown,
}

/// What the daemon can have a sidecar do. Each one is one `agentapi` call.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Command {
    /// `agentapi new-conversation`: start a conversation in the sidecar's project.
    NewConversation {
        prompt: String,
        #[serde(default)]
        title: Option<String>,
        #[serde(default)]
        model: Option<String>,
    },
    /// A call that proves `agentapi` works in the sidecar's environment without a model call:
    /// it asks about a conversation that does not exist.
    Probe,
}

/// How a command ended.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Outcome {
    /// The exit code; `None` when the process did not run to its end.
    pub status: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    /// Why there is no exit code: it could not start, did not finish in time, or was killed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl Outcome {
    pub fn succeeded(&self) -> bool {
        self.status == Some(0) && self.error.is_none()
    }

    pub fn failed_to_run(error: impl Into<String>) -> Self {
        Self {
            error: Some(error.into()),
            ..Self::default()
        }
    }

    /// A short account of a failure for a log or a reply: what `agentapi` said, else why it did
    /// not run.
    pub fn failure_text(&self) -> String {
        if let Some(error) = &self.error {
            return error.clone();
        }
        let said = self.stderr.trim();
        let said = if said.is_empty() {
            self.stdout.trim()
        } else {
            said
        };
        let said: String = said.chars().take(300).collect();
        match self.status {
            Some(code) if said.is_empty() => format!("agentapi exited with status {code}"),
            Some(code) => format!("agentapi exited with status {code}: {said}"),
            None => said,
        }
    }
}

// MARK: - The arguments of `agentapi`

impl Command {
    /// The arguments after `agentapi`. The task text is one argument after `--`, so text that
    /// starts with a dash is not read as an option; `=` ties a value to its option for the same
    /// reason.
    pub fn argv(&self) -> Result<Vec<String>, String> {
        match self {
            Command::NewConversation {
                prompt,
                title,
                model,
            } => {
                if prompt.trim().is_empty() {
                    return Err("The task text is empty".into());
                }
                if prompt.len() > MAX_PROMPT_BYTES {
                    return Err(format!(
                        "The task text is {} bytes; at most {MAX_PROMPT_BYTES} fit in one call",
                        prompt.len()
                    ));
                }
                if prompt.contains('\0') {
                    return Err("The task text contains a NUL character".into());
                }
                let mut argv = vec!["new-conversation".to_string()];
                if let Some(model) = model {
                    if !crate::agents::model_catalog::valid_model_id(model) {
                        return Err(format!("{model:?} is not a model id"));
                    }
                    argv.push(format!("--model={model}"));
                }
                if let Some(title) = title {
                    if title.chars().count() > MAX_TITLE_CHARS
                        || title.chars().any(|c| c.is_control())
                    {
                        return Err("The title is too long or has control characters".into());
                    }
                    if !title.trim().is_empty() {
                        argv.push(format!("--title={title}"));
                    }
                }
                argv.push("--".into());
                argv.push(prompt.clone());
                Ok(argv)
            }
            Command::Probe => Ok(vec![
                "get-conversation-metadata".into(),
                PROBE_CONVERSATION.into(),
            ]),
        }
    }
}

/// The conversation `agentapi new-conversation` made, from what it printed
/// (`{"response":{"newConversation":{"conversationId":"…"}}}`), or why there is none.
pub fn conversation_id(outcome: &Outcome) -> Result<String, String> {
    if !outcome.succeeded() {
        return Err(outcome.failure_text());
    }
    let value: serde_json::Value = serde_json::from_str(outcome.stdout.trim())
        .map_err(|_| "agentapi's answer was not JSON".to_string())?;
    let id = value["response"]["newConversation"]["conversationId"]
        .as_str()
        .ok_or_else(|| "agentapi's answer had no conversation id".to_string())?;
    if id.len() != 36 || uuid::Uuid::parse_str(id).is_err() {
        return Err("agentapi's answer had a conversation id that is not an id".into());
    }
    Ok(id.to_string())
}

// MARK: - The daemon's table of connected sidecars

/// A frame for a sidecar's connection and, optionally, how the write went.
pub struct Outbound {
    pub frame: Down,
    /// Told whether the frame was written. A command that was not written was not sent.
    pub written: Option<oneshot::Sender<bool>>,
}

/// What the daemon knows about a connected sidecar.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LinkInfo {
    pub project_id: Option<String>,
    pub version: String,
    pub agentapi: bool,
    pub pid: u32,
    pub connected_at: u64,
}

/// A command that did not come back with a result, and whether it can have run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunError {
    /// It was not sent: the sidecar is not connected, or the frame was not written. Trying again
    /// cannot start two conversations.
    NotSent(String),
    /// It was sent and no result came back. It may have run.
    Unknown(String),
}

impl std::fmt::Display for RunError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RunError::NotSent(text) | RunError::Unknown(text) => f.write_str(text),
        }
    }
}

impl std::error::Error for RunError {}

struct Link {
    /// Tells a connection that was replaced from the one that replaced it.
    generation: u64,
    outbound: mpsc::UnboundedSender<Outbound>,
    info: LinkInfo,
    waiting: HashMap<String, oneshot::Sender<Outcome>>,
}

#[derive(Default)]
struct Table {
    links: HashMap<String, Link>,
    generations: u64,
}

/// The connected sidecars. One process-wide instance serves the daemon; tests make their own.
pub struct Manager {
    table: Mutex<Table>,
    /// Wakes whoever waits for a sidecar to check in.
    changed: Notify,
}

/// The daemon's manager.
pub fn shared() -> &'static Arc<Manager> {
    static SHARED: OnceLock<Arc<Manager>> = OnceLock::new();
    SHARED.get_or_init(|| Arc::new(Manager::new()))
}

impl Default for Manager {
    fn default() -> Self {
        Self::new()
    }
}

impl Manager {
    pub fn new() -> Self {
        Self {
            table: Mutex::new(Table::default()),
            changed: Notify::new(),
        }
    }

    fn table(&self) -> std::sync::MutexGuard<'_, Table> {
        self.table
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// A sidecar's connection was accepted. `None` when that sidecar is connected already: the
    /// connection that was there first stays (a process that was stopped is noticed at once,
    /// because its connection ends), so two processes of one sidecar cannot take it from each
    /// other in turn.
    pub fn attach(
        &self,
        check_in: &CheckIn,
        now_ms: u64,
    ) -> Option<(u64, mpsc::UnboundedReceiver<Outbound>)> {
        let (outbound, receiver) = mpsc::unbounded_channel();
        let generation = {
            let mut table = self.table();
            if table.links.contains_key(&check_in.id) {
                return None;
            }
            table.generations += 1;
            let generation = table.generations;
            table.links.insert(
                check_in.id.clone(),
                Link {
                    generation,
                    outbound,
                    info: LinkInfo {
                        project_id: check_in.project_id.clone(),
                        version: check_in.version.clone(),
                        agentapi: check_in.agentapi,
                        pid: check_in.pid,
                        connected_at: now_ms,
                    },
                    waiting: HashMap::new(),
                },
            );
            generation
        };
        self.changed.notify_waiters();
        Some((generation, receiver))
    }

    /// The connection `generation` of `id` ended. A connection that is not the current one leaves
    /// the current one alone.
    pub fn detach(&self, id: &str, generation: u64) {
        let mut table = self.table();
        if table
            .links
            .get(id)
            .is_some_and(|link| link.generation == generation)
        {
            // Dropping the link drops what its commands wait on, which ends them as unknown.
            table.links.remove(id);
        }
    }

    /// A result arrived on the connection `generation` of `id`. `false` when nothing waited for
    /// it (it was given up on, or the connection had been replaced).
    pub fn complete(&self, id: &str, generation: u64, command_id: &str, outcome: Outcome) -> bool {
        let waiting = {
            let mut table = self.table();
            table
                .links
                .get_mut(id)
                .filter(|link| link.generation == generation)
                .and_then(|link| link.waiting.remove(command_id))
        };
        waiting.is_some_and(|sender| sender.send(outcome).is_ok())
    }

    pub fn is_online(&self, id: &str) -> bool {
        self.table().links.contains_key(id)
    }

    /// Every connected sidecar.
    pub fn online(&self) -> HashMap<String, LinkInfo> {
        self.table()
            .links
            .iter()
            .map(|(id, link)| (id.clone(), link.info.clone()))
            .collect()
    }

    /// Waits until the sidecar `id` is connected, for at most `budget`. It is woken by the
    /// check-in itself, not by looking again and again.
    pub async fn wait_online(&self, id: &str, budget: Duration) -> bool {
        let deadline = tokio::time::Instant::now() + budget;
        loop {
            // Listen before looking, so a check-in between the two is not missed.
            let changed = self.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            if self.is_online(id) {
                return true;
            }
            if tokio::time::timeout_at(deadline, changed).await.is_err() {
                return self.is_online(id);
            }
        }
    }

    /// Has the sidecar `id` run `command` and returns how it ended.
    pub async fn run(
        &self,
        id: &str,
        command: Command,
        timeout: Duration,
    ) -> Result<Outcome, RunError> {
        let command_id = uuid::Uuid::new_v4().to_string();
        let (result, answer) = oneshot::channel();
        let (written, was_written) = oneshot::channel();
        let generation = {
            let mut table = self.table();
            let Some(link) = table.links.get_mut(id) else {
                return Err(RunError::NotSent(
                    "The sidecar is not connected to AskHuman".into(),
                ));
            };
            link.waiting.insert(command_id.clone(), result);
            let sent = link.outbound.send(Outbound {
                frame: Down::Run {
                    command_id: command_id.clone(),
                    command,
                },
                written: Some(written),
            });
            if sent.is_err() {
                link.waiting.remove(&command_id);
                return Err(RunError::NotSent(
                    "The sidecar's connection is closing".into(),
                ));
            }
            link.generation
        };
        match was_written.await {
            Ok(true) => {}
            Ok(false) | Err(_) => {
                self.forget(id, generation, &command_id);
                return Err(RunError::NotSent(
                    "The command could not be written to the sidecar".into(),
                ));
            }
        }
        match tokio::time::timeout(timeout, answer).await {
            Ok(Ok(outcome)) => Ok(outcome),
            Ok(Err(_)) => Err(RunError::Unknown(
                "The sidecar disconnected before it answered".into(),
            )),
            Err(_) => {
                self.forget(id, generation, &command_id);
                Err(RunError::Unknown(format!(
                    "The sidecar did not answer within {} seconds",
                    timeout.as_secs()
                )))
            }
        }
    }

    fn forget(&self, id: &str, generation: u64, command_id: &str) {
        if let Some(link) = self
            .table()
            .links
            .get_mut(id)
            .filter(|link| link.generation == generation)
        {
            link.waiting.remove(command_id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn check_in(id: &str) -> CheckIn {
        CheckIn {
            id: id.to_string(),
            link_version: LINK_VERSION,
            project_id: Some("project-1".into()),
            version: "9.9.9".into(),
            agentapi: true,
            pid: 42,
        }
    }

    fn created(prompt: &str) -> Command {
        Command::NewConversation {
            prompt: prompt.to_string(),
            title: None,
            model: None,
        }
    }

    // MARK: frames

    #[test]
    fn frames_have_a_stable_json_shape() {
        let up = Up::Done {
            command_id: "c1".into(),
            outcome: Outcome {
                status: Some(0),
                stdout: "{}".into(),
                stderr: String::new(),
                error: None,
            },
        };
        assert_eq!(
            serde_json::to_value(&up).unwrap(),
            json!({"type": "done", "commandId": "c1",
                   "outcome": {"status": 0, "stdout": "{}", "stderr": ""}})
        );
        let hello = serde_json::to_value(Up::CheckIn(check_in("askhuman-ws-1"))).unwrap();
        assert_eq!(hello["type"], "checkIn");
        assert_eq!(hello["linkVersion"], 1);
        assert_eq!(hello["projectId"], "project-1");
        let run = Down::Run {
            command_id: "c2".into(),
            command: Command::NewConversation {
                prompt: "hi".into(),
                title: Some("t".into()),
                model: None,
            },
        };
        assert_eq!(
            serde_json::to_value(&run).unwrap(),
            json!({"type": "run", "commandId": "c2",
                   "command": {"kind": "newConversation", "prompt": "hi", "title": "t",
                               "model": null}})
        );
        assert_eq!(
            serde_json::to_value(Down::Refused {
                reason: "x".into(),
                restart: true
            })
            .unwrap(),
            json!({"type": "refused", "reason": "x", "restart": true})
        );
        assert_eq!(
            serde_json::to_value(Down::Welcome).unwrap(),
            json!({"type": "welcome"})
        );
    }

    #[test]
    fn a_frame_of_a_kind_nobody_knows_is_not_an_error() {
        let up: Up = serde_json::from_value(json!({"type": "someLaterThing", "a": 1})).unwrap();
        assert_eq!(up, Up::Unknown);
        let down: Down = serde_json::from_value(json!({"type": "someLaterThing"})).unwrap();
        assert_eq!(down, Down::Unknown);
    }

    #[test]
    fn a_check_in_from_a_leaner_sidecar_reads_with_defaults() {
        let up: Up = serde_json::from_value(
            json!({"type": "checkIn", "id": "askhuman-ws-1", "linkVersion": 1}),
        )
        .unwrap();
        let Up::CheckIn(check_in) = up else {
            panic!("not a check-in")
        };
        assert!(!check_in.agentapi);
        assert_eq!(check_in.project_id, None);
        assert_eq!(check_in.pid, 0);
    }

    #[test]
    fn a_refusal_without_the_restart_flag_does_not_restart() {
        let down: Down =
            serde_json::from_value(json!({"type": "refused", "reason": "nope"})).unwrap();
        assert_eq!(
            down,
            Down::Refused {
                reason: "nope".into(),
                restart: false
            }
        );
    }

    // MARK: arguments

    #[test]
    fn the_task_text_is_one_argument_after_a_double_dash() {
        let argv = Command::NewConversation {
            prompt: "- fix it; rm -rf /".into(),
            title: Some("- fix it".into()),
            model: Some("gemini-3.8-flash-low".into()),
        }
        .argv()
        .unwrap();
        assert_eq!(
            argv,
            [
                "new-conversation",
                "--model=gemini-3.8-flash-low",
                "--title=- fix it",
                "--",
                "- fix it; rm -rf /"
            ]
        );
        // No model and no title: nothing but the text.
        assert_eq!(
            created("hello").argv().unwrap(),
            ["new-conversation", "--", "hello"]
        );
    }

    #[test]
    fn an_empty_title_is_left_out() {
        let argv = Command::NewConversation {
            prompt: "x".into(),
            title: Some("   ".into()),
            model: None,
        }
        .argv()
        .unwrap();
        assert_eq!(argv, ["new-conversation", "--", "x"]);
    }

    #[test]
    fn text_that_cannot_be_one_argument_is_refused() {
        assert!(created("").argv().is_err());
        assert!(created("  \n ").argv().is_err());
        assert!(created("a\0b").argv().is_err());
        assert!(created(&"x".repeat(MAX_PROMPT_BYTES + 1)).argv().is_err());
        assert!(created(&"x".repeat(MAX_PROMPT_BYTES)).argv().is_ok());
        for title in ["two\nlines", "tab\there", &"t".repeat(MAX_TITLE_CHARS + 1)] {
            let command = Command::NewConversation {
                prompt: "x".into(),
                title: Some(title.to_string()),
                model: None,
            };
            assert!(command.argv().is_err(), "{title:?}");
        }
        for model in ["", "-flag", "a b", "a:b", "--model=x", "a\nb"] {
            let command = Command::NewConversation {
                prompt: "x".into(),
                title: None,
                model: Some(model.to_string()),
            };
            assert!(command.argv().is_err(), "{model:?}");
        }
    }

    #[test]
    fn the_probe_asks_about_a_conversation_that_does_not_exist() {
        assert_eq!(
            Command::Probe.argv().unwrap(),
            [
                "get-conversation-metadata",
                "00000000-0000-0000-0000-000000000000"
            ]
        );
    }

    // MARK: what came back

    fn answered(stdout: &str) -> Outcome {
        Outcome {
            status: Some(0),
            stdout: stdout.into(),
            ..Outcome::default()
        }
    }

    #[test]
    fn the_conversation_id_comes_from_the_printed_json() {
        let id = "0d4f9a52-7a41-4d6c-9f55-0e0f6d1b9c11";
        let out = answered(&format!(
            "{{\"response\":{{\"newConversation\":{{\"conversationId\":\"{id}\"}}}}}}\n"
        ));
        assert_eq!(conversation_id(&out).unwrap(), id);
        for bad in [
            "not json",
            "{}",
            "{\"response\":{\"newConversation\":{}}}",
            "{\"response\":{\"newConversation\":{\"conversationId\":\"../../x\"}}}",
        ] {
            assert!(conversation_id(&answered(bad)).is_err(), "{bad}");
        }
    }

    #[test]
    fn a_failed_call_says_what_agentapi_said() {
        let failed = Outcome {
            status: Some(1),
            stdout: String::new(),
            stderr: "Unauthenticated: missing CSRF token\n".into(),
            error: None,
        };
        assert_eq!(
            conversation_id(&failed).unwrap_err(),
            "agentapi exited with status 1: Unauthenticated: missing CSRF token"
        );
        let silent = Outcome {
            status: Some(2),
            ..Outcome::default()
        };
        assert_eq!(silent.failure_text(), "agentapi exited with status 2");
        assert_eq!(
            Outcome::failed_to_run("could not start agentapi").failure_text(),
            "could not start agentapi"
        );
        let long = Outcome {
            status: Some(1),
            stderr: "e".repeat(1000),
            ..Outcome::default()
        };
        assert!(long.failure_text().chars().count() < 340);
    }

    // MARK: the table

    /// A connection as the daemon's handler runs it: every command that arrives is answered.
    fn answering(
        manager: &Arc<Manager>,
        id: &str,
        outcome: Outcome,
    ) -> (u64, tokio::task::JoinHandle<()>) {
        let (generation, mut receiver) = manager.attach(&check_in(id), 1).unwrap();
        let manager = manager.clone();
        let id = id.to_string();
        let task = tokio::spawn(async move {
            while let Some(outbound) = receiver.recv().await {
                if let Some(written) = outbound.written {
                    let _ = written.send(true);
                }
                if let Down::Run { command_id, .. } = outbound.frame {
                    manager.complete(&id, generation, &command_id, outcome.clone());
                }
            }
        });
        (generation, task)
    }

    #[tokio::test]
    async fn a_command_goes_to_the_sidecar_and_its_result_comes_back() {
        let manager = Arc::new(Manager::new());
        let (_, _task) = answering(&manager, "askhuman-ws-1", answered("ok"));
        let outcome = manager
            .run("askhuman-ws-1", Command::Probe, Duration::from_secs(5))
            .await
            .unwrap();
        assert_eq!(outcome.stdout, "ok");
    }

    #[tokio::test]
    async fn a_command_for_a_sidecar_that_is_not_connected_was_not_sent() {
        let manager = Manager::new();
        let error = manager
            .run("askhuman-ws-1", Command::Probe, Duration::from_secs(5))
            .await
            .unwrap_err();
        assert!(matches!(error, RunError::NotSent(_)));
    }

    #[tokio::test]
    async fn a_frame_that_was_not_written_was_not_sent() {
        let manager = Manager::new();
        let (_, mut receiver) = manager.attach(&check_in("askhuman-ws-1"), 1).unwrap();
        let writer = tokio::spawn(async move {
            let outbound = receiver.recv().await.unwrap();
            let _ = outbound.written.unwrap().send(false);
            receiver
        });
        let error = manager
            .run("askhuman-ws-1", Command::Probe, Duration::from_secs(5))
            .await
            .unwrap_err();
        assert!(matches!(error, RunError::NotSent(_)), "{error:?}");
        let _ = writer.await;
        // Nothing is left waiting for it.
        assert!(manager.table().links["askhuman-ws-1"].waiting.is_empty());
    }

    #[tokio::test]
    async fn a_connection_that_closes_before_writing_leaves_the_command_unsent() {
        let manager = Manager::new();
        let (_, receiver) = manager.attach(&check_in("askhuman-ws-1"), 1).unwrap();
        drop(receiver);
        let error = manager
            .run("askhuman-ws-1", Command::Probe, Duration::from_secs(5))
            .await
            .unwrap_err();
        assert!(matches!(error, RunError::NotSent(_)), "{error:?}");
    }

    #[tokio::test]
    async fn a_command_that_was_written_and_never_answered_may_have_run() {
        let manager = Arc::new(Manager::new());
        let (generation, mut receiver) = manager.attach(&check_in("askhuman-ws-1"), 1).unwrap();
        let connection = manager.clone();
        let task = tokio::spawn(async move {
            let outbound = receiver.recv().await.unwrap();
            let _ = outbound.written.unwrap().send(true);
            // The connection ends without a result.
            connection.detach("askhuman-ws-1", generation);
        });
        let error = manager
            .run("askhuman-ws-1", Command::Probe, Duration::from_secs(5))
            .await
            .unwrap_err();
        assert!(matches!(error, RunError::Unknown(_)), "{error:?}");
        task.await.unwrap();
    }

    #[tokio::test(start_paused = true)]
    async fn a_command_that_gets_no_answer_in_time_is_unknown_and_forgotten() {
        let manager = Arc::new(Manager::new());
        let (_, mut receiver) = manager.attach(&check_in("askhuman-ws-1"), 1).unwrap();
        tokio::spawn(async move {
            while let Some(outbound) = receiver.recv().await {
                let _ = outbound.written.unwrap().send(true);
            }
        });
        let error = manager
            .run("askhuman-ws-1", Command::Probe, Duration::from_secs(45))
            .await
            .unwrap_err();
        assert!(matches!(error, RunError::Unknown(_)), "{error:?}");
        assert!(manager.table().links["askhuman-ws-1"].waiting.is_empty());
    }

    #[tokio::test]
    async fn a_sidecar_that_is_connected_is_not_taken_over_by_a_second_connection() {
        let manager = Manager::new();
        let (first, mut receiver) = manager.attach(&check_in("askhuman-ws-1"), 1).unwrap();
        assert!(manager.attach(&check_in("askhuman-ws-1"), 2).is_none());
        // The refused one changed nothing: the first connection is still the one.
        assert!(manager.is_online("askhuman-ws-1"));
        assert_eq!(manager.online()["askhuman-ws-1"].connected_at, 1);
        assert!(receiver.try_recv().is_err());
        // Another sidecar is no obstacle.
        assert!(manager.attach(&check_in("askhuman-ws-2"), 3).is_some());
        // A connection that is not the current one cannot end the current one.
        manager.detach("askhuman-ws-1", first + 100);
        assert!(manager.is_online("askhuman-ws-1"));
        assert!(!manager.complete("askhuman-ws-1", first + 100, "c", Outcome::default()));
        // Once the first one has ended, the sidecar can connect again.
        manager.detach("askhuman-ws-1", first);
        assert!(!manager.is_online("askhuman-ws-1"));
        assert!(manager.attach(&check_in("askhuman-ws-1"), 4).is_some());
    }

    #[tokio::test]
    async fn a_result_nobody_waits_for_is_ignored() {
        let manager = Manager::new();
        let (generation, _receiver) = manager.attach(&check_in("askhuman-ws-1"), 1).unwrap();
        assert!(!manager.complete("askhuman-ws-1", generation, "unknown", Outcome::default()));
        assert!(!manager.complete("askhuman-ws-2", 1, "unknown", Outcome::default()));
    }

    #[tokio::test]
    async fn the_online_table_says_what_each_sidecar_reported() {
        let manager = Manager::new();
        let (_, _receiver) = manager.attach(&check_in("askhuman-ws-1"), 7).unwrap();
        let online = manager.online();
        assert_eq!(online.len(), 1);
        let info = &online["askhuman-ws-1"];
        assert_eq!(info.project_id.as_deref(), Some("project-1"));
        assert_eq!(info.version, "9.9.9");
        assert!(info.agentapi);
        assert_eq!(info.pid, 42);
        assert_eq!(info.connected_at, 7);
    }

    #[tokio::test(start_paused = true)]
    async fn waiting_for_a_sidecar_ends_the_moment_it_checks_in() {
        let manager = Arc::new(Manager::new());
        let waiter = {
            let manager = manager.clone();
            tokio::spawn(async move {
                let started = tokio::time::Instant::now();
                let online = manager
                    .wait_online("askhuman-ws-1", Duration::from_secs(30))
                    .await;
                (online, started.elapsed())
            })
        };
        tokio::time::sleep(Duration::from_secs(7)).await;
        let (_generation, _receiver) = manager.attach(&check_in("askhuman-ws-1"), 1).unwrap();
        let (online, waited) = waiter.await.unwrap();
        assert!(online);
        // Woken by the check-in, not by a poll that came round later.
        assert_eq!(waited, Duration::from_secs(7));
    }

    #[tokio::test(start_paused = true)]
    async fn waiting_gives_up_when_the_budget_is_spent() {
        let manager = Manager::new();
        let started = tokio::time::Instant::now();
        assert!(
            !manager
                .wait_online("askhuman-ws-1", Duration::from_secs(30))
                .await
        );
        assert_eq!(started.elapsed(), Duration::from_secs(30));
    }

    #[tokio::test]
    async fn an_online_sidecar_needs_no_waiting() {
        let manager = Manager::new();
        let (_generation, _receiver) = manager.attach(&check_in("askhuman-ws-1"), 1).unwrap();
        assert!(
            manager
                .wait_online("askhuman-ws-1", Duration::from_secs(0))
                .await
        );
    }
}
