//! Sending a message to a conversation of the App that is idle
//! (docs/specs/antigravity-deep-integration.md §10.2).
//!
//! A working conversation is reached through its hooks (a `PreInvocation` hook asks the daemon
//! for text to inject). An idle one has no hook waiting, so the message goes the other way: a
//! sidecar's `agentapi send-message` puts it into the conversation, which wakes up and answers.
//! Any connected sidecar can carry it, whatever project it belongs to; when none is connected the
//! daemon brings up the control sidecar (no grants, no folder of the user's) and the App.
//!
//! This only delivers. Whether the message started a turn is proved later, by the conversation's
//! own `PreInvocation` hook, which the daemon matches to the message (agent_control.rs).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

use super::create::{self, Backend, Problem, ONLINE_BUDGET};
use super::launch::{self, UnavailableReason};
use super::link::{Command, LinkInfo, RunError};
use super::sidecar::{self, Change, Kind, Spec};

/// One message for one conversation.
pub struct Message {
    /// The conversation's id.
    pub conversation: String,
    pub text: String,
    /// The conversation's project folder, when it is known: the sidecar of that folder carries the
    /// message when it is connected.
    pub directory: Option<PathBuf>,
}

/// What became of a message that was not delivered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Failure {
    /// Nothing was sent, so sending again cannot deliver it twice.
    NotSent(String),
    /// The message may have been delivered.
    Unclear(String),
}

/// What sending needs beyond what creating a task needs, so tests can stand in for the App.
pub(super) trait Route: Backend {
    /// Why a message cannot go through the App now, if so.
    async fn message_unavailable(&self) -> Option<UnavailableReason>;
    /// The sidecars that are connected now.
    async fn connected(&self) -> HashMap<String, LinkInfo>;
    /// Makes the control sidecar exist and switches it on; returns its id.
    async fn ensure_control(&self) -> Result<(String, Change), Problem>;
}

/// The sidecar to carry a message: the one of the conversation's own folder when it is connected,
/// else the control sidecar, else any other that can run `agentapi`.
fn pick(connected: &HashMap<String, LinkInfo>, own: &[String]) -> Option<String> {
    let capable = |id: &str| connected.get(id).is_some_and(|link| link.agentapi);
    if let Some(id) = own.iter().find(|id| capable(id)) {
        return Some(id.clone());
    }
    let control = Spec {
        kind: Kind::Control,
        directory: PathBuf::new(),
        rules: Vec::new(),
    }
    .id();
    if capable(&control) {
        return Some(control);
    }
    let mut others: Vec<&String> = connected
        .iter()
        .filter(|(_, link)| link.agentapi)
        .map(|(id, _)| id)
        .collect();
    others.sort();
    others.first().map(|id| (*id).clone())
}

/// The ids of the sidecars a folder can have, whichever kind is there.
fn own_sidecars(directory: &Path) -> Vec<String> {
    let Ok(directory) = std::fs::canonicalize(directory) else {
        return Vec::new();
    };
    [Kind::Workspace, Kind::Yolo]
        .into_iter()
        .map(|kind| {
            Spec {
                kind,
                directory: directory.clone(),
                rules: Vec::new(),
            }
            .id()
        })
        .collect()
}

/// Delivers one message, bringing the App and a sidecar up when none is connected.
pub(super) async fn send_with<R: Route>(route: &R, message: Message) -> Result<(), Problem> {
    let command = Command::SendMessage {
        conversation: message.conversation,
        content: message.text,
    };
    // What the sidecar would be asked to run; it refuses what cannot be an argument.
    command.argv().map_err(Problem::Invalid)?;
    if let Some(reason) = route.message_unavailable().await {
        return Err(Problem::Unavailable(reason));
    }
    let started = Instant::now();
    let connected = route.connected().await;
    let own = message
        .directory
        .as_deref()
        .map(own_sidecars)
        .unwrap_or_default();
    let sidecar_id = match pick(&connected, &own) {
        Some(id) => id,
        // Sidecars are connected, but none can run `agentapi`: a control sidecar could not either.
        None if !connected.is_empty() => return Err(Problem::NoAgentapi),
        None => {
            let (id, _) = route.ensure_control().await?;
            let mut app_started = false;
            if route.app_running().await == Some(false) {
                route.start_app().await.map_err(Problem::AppNotStarted)?;
                app_started = true;
            }
            let remaining = ONLINE_BUDGET.saturating_sub(started.elapsed());
            if !route.wait_online(&id, remaining).await {
                return Err(Problem::NotOnline {
                    started: app_started,
                });
            }
            match route.link(&id).await {
                Some(link) if link.agentapi => id,
                Some(_) => return Err(Problem::NoAgentapi),
                // It went away again between the check-in and now.
                None => {
                    return Err(Problem::NotOnline {
                        started: app_started,
                    })
                }
            }
        }
    };
    match route.run(&sidecar_id, command).await {
        Err(RunError::NotSent(detail)) => Err(Problem::NotSent(detail)),
        Err(RunError::Unknown(detail)) => Err(unclear(Some(detail))),
        Ok(outcome) if outcome.succeeded() => Ok(()),
        Ok(outcome) => {
            // `agentapi` that exits with an error delivered nothing. Anything else (it was
            // killed, or timed out) may have.
            let refused = outcome.error.is_none() && outcome.status.is_some_and(|code| code != 0);
            let detail = outcome.failure_text();
            if refused {
                Err(Problem::Rejected(detail))
            } else {
                Err(unclear(Some(detail)))
            }
        }
    }
}

fn unclear(detail: Option<String>) -> Problem {
    Problem::Unclear {
        directory: String::new(),
        detail,
    }
}

// MARK: - The real thing

impl Route for create::Live {
    async fn message_unavailable(&self) -> Option<UnavailableReason> {
        let config = crate::config::AppConfig::load_without_secrets().antigravity_desktop;
        launch::message_reason(launch::message_facts(&config))
    }

    async fn connected(&self) -> HashMap<String, LinkInfo> {
        super::manager().online()
    }

    async fn ensure_control(&self) -> Result<(String, Change), Problem> {
        tokio::task::spawn_blocking(|| {
            let executable = super::executable().map_err(|error| create::problem_of(&error))?;
            sidecar::ensure_control(
                &sidecar::Layout::current(),
                &executable,
                &sidecar::control_directory(),
                super::now_ms(),
            )
            .map_err(|error| create::problem_of(&error))
        })
        .await
        .map_err(|error| Problem::Files(error.to_string()))?
    }
}

/// Whether the conversation is one of the App's (not `agy`'s, which has no sidecar), and the
/// route is allowed. Reads the settings and looks for the conversation's log, so callers cache it.
pub fn available(session_id: &str) -> bool {
    let config = crate::config::AppConfig::load_without_secrets().antigravity_desktop;
    launch::message_reason(launch::message_facts(&config)).is_none()
        && is_app_conversation(&crate::paths::integration_home(), session_id)
}

fn is_app_conversation(home: &Path, session_id: &str) -> bool {
    use crate::agents::antigravity_hook::{surface, Surface};
    crate::agents::title::antigravity_transcript_in(home, session_id)
        .and_then(|path| path.to_str().map(str::to_string))
        .is_some_and(|path| surface(Some(&path)) == Surface::App)
}

/// The daemon's entry: delivers the message, or says whether it may have gone out.
pub async fn send(message: Message) -> Result<(), Failure> {
    let conversation = message.conversation.clone();
    match send_with(&create::Live, message).await {
        Ok(()) => {
            crate::daemon::lifecycle::log_runtime_event(
                "antigravity_desktop",
                "message_sent",
                Some(&conversation),
            );
            Ok(())
        }
        Err(problem) => {
            crate::daemon::lifecycle::log_runtime_event(
                "antigravity_desktop",
                &format!("message_{}", problem.code()),
                Some(&conversation),
            );
            Err(match problem {
                Problem::Unclear { detail, .. } => Failure::Unclear(detail.unwrap_or_default()),
                other => Failure::NotSent(other.code().to_string()),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::antigravity_desktop::link::Outcome;
    use std::collections::VecDeque;
    use std::sync::Mutex;

    const CONVERSATION: &str = "0b7f2c1e-aaaa-4bbb-8ccc-ddddeeeeffff";

    fn link(agentapi: bool) -> LinkInfo {
        LinkInfo {
            project_id: None,
            version: "test".into(),
            agentapi,
            pid: 1,
            connected_at: 0,
        }
    }

    fn message(directory: Option<&Path>) -> Message {
        Message {
            conversation: CONVERSATION.into(),
            text: "please continue".into(),
            directory: directory.map(Path::to_path_buf),
        }
    }

    fn delivered() -> Outcome {
        Outcome {
            status: Some(0),
            stdout: "{}".into(),
            ..Outcome::default()
        }
    }

    struct Fake {
        unavailable: Option<UnavailableReason>,
        connected: Mutex<HashMap<String, LinkInfo>>,
        app_running: Option<bool>,
        online_after_start: bool,
        link_after_start: Option<LinkInfo>,
        runs: Mutex<VecDeque<Result<Outcome, RunError>>>,
        calls: Mutex<Vec<String>>,
        commands: Mutex<Vec<(String, Command)>>,
    }

    impl Fake {
        fn new() -> Self {
            Self {
                unavailable: None,
                connected: Mutex::new(HashMap::new()),
                app_running: Some(true),
                online_after_start: true,
                link_after_start: Some(link(true)),
                runs: Mutex::new(VecDeque::new()),
                calls: Mutex::new(Vec::new()),
                commands: Mutex::new(Vec::new()),
            }
        }

        fn with(self, ids: &[(&str, bool)]) -> Self {
            *self.connected.lock().unwrap() = ids
                .iter()
                .map(|(id, agentapi)| (id.to_string(), link(*agentapi)))
                .collect();
            self
        }

        fn runs(self, runs: Vec<Result<Outcome, RunError>>) -> Self {
            *self.runs.lock().unwrap() = runs.into();
            self
        }

        fn calls(&self) -> Vec<String> {
            self.calls.lock().unwrap().clone()
        }

        fn note(&self, call: &str) {
            self.calls.lock().unwrap().push(call.to_string());
        }

        fn sent_by(&self) -> Vec<String> {
            self.commands
                .lock()
                .unwrap()
                .iter()
                .map(|(id, _)| id.clone())
                .collect()
        }
    }

    impl Backend for Fake {
        async fn unavailable(&self) -> Option<UnavailableReason> {
            unreachable!("a message does not ask whether a task can be created")
        }

        async fn ensure_sidecar(
            &self,
            _: &Path,
            _: sidecar::Kind,
        ) -> Result<(String, Change), Problem> {
            unreachable!("a message never makes the sidecar of a folder")
        }

        async fn app_running(&self) -> Option<bool> {
            self.note("app_running");
            self.app_running
        }

        async fn start_app(&self) -> Result<(), String> {
            self.note("start_app");
            Ok(())
        }

        async fn wait_online(&self, id: &str, _: std::time::Duration) -> bool {
            self.note(&format!("wait_online {id}"));
            self.online_after_start
        }

        async fn link(&self, _: &str) -> Option<LinkInfo> {
            self.link_after_start.clone()
        }

        async fn run(&self, id: &str, command: Command) -> Result<Outcome, RunError> {
            self.note("run");
            self.commands
                .lock()
                .unwrap()
                .push((id.to_string(), command));
            self.runs
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or_else(|| Ok(delivered()))
        }
    }

    impl Route for Fake {
        async fn message_unavailable(&self) -> Option<UnavailableReason> {
            self.note("message_unavailable");
            self.unavailable
        }

        async fn connected(&self) -> HashMap<String, LinkInfo> {
            self.connected.lock().unwrap().clone()
        }

        async fn ensure_control(&self) -> Result<(String, Change), Problem> {
            self.note("ensure_control");
            Ok(("askhuman-ctl".into(), Change::Created))
        }
    }

    fn own_id(directory: &Path) -> String {
        own_sidecars(directory).remove(0)
    }

    // MARK: choosing the sidecar

    #[tokio::test]
    async fn the_sidecar_of_the_conversations_own_folder_carries_the_message() {
        let folder = tempfile::tempdir().unwrap();
        let own = own_id(folder.path());
        let fake = Fake::new().with(&[
            ("askhuman-ctl", true),
            (&own, true),
            ("askhuman-ws-00000000", true),
        ]);
        send_with(&fake, message(Some(folder.path())))
            .await
            .unwrap();
        assert_eq!(fake.sent_by(), [own]);
        assert!(!fake.calls().contains(&"ensure_control".to_string()));
    }

    #[tokio::test]
    async fn the_yolo_sidecar_of_the_folder_counts_as_its_own() {
        let folder = tempfile::tempdir().unwrap();
        let yolo = own_sidecars(folder.path()).remove(1);
        assert!(yolo.starts_with("askhuman-yolo-"));
        let fake = Fake::new().with(&[("askhuman-ctl", true), (&yolo, true)]);
        send_with(&fake, message(Some(folder.path())))
            .await
            .unwrap();
        assert_eq!(fake.sent_by(), [yolo]);
    }

    #[tokio::test]
    async fn without_its_own_the_control_sidecar_goes_before_another_folders() {
        let folder = tempfile::tempdir().unwrap();
        let fake = Fake::new().with(&[("askhuman-ws-11111111", true), ("askhuman-ctl", true)]);
        send_with(&fake, message(Some(folder.path())))
            .await
            .unwrap();
        assert_eq!(fake.sent_by(), ["askhuman-ctl"]);
        // A folder that is not known at all is the same case.
        let fake = Fake::new().with(&[("askhuman-ws-11111111", true), ("askhuman-ctl", true)]);
        send_with(&fake, message(None)).await.unwrap();
        assert_eq!(fake.sent_by(), ["askhuman-ctl"]);
    }

    #[tokio::test]
    async fn any_connected_sidecar_will_do_and_the_choice_is_stable() {
        let fake = Fake::new().with(&[
            ("askhuman-ws-bbbbbbbb", true),
            ("askhuman-ws-aaaaaaaa", true),
            ("askhuman-ws-cccccccc", false),
        ]);
        send_with(&fake, message(None)).await.unwrap();
        assert_eq!(fake.sent_by(), ["askhuman-ws-aaaaaaaa"]);
    }

    #[tokio::test]
    async fn sidecars_that_cannot_run_agentapi_are_not_used_and_no_control_sidecar_replaces_them() {
        let fake = Fake::new().with(&[("askhuman-ws-aaaaaaaa", false)]);
        let error = send_with(&fake, message(None)).await.unwrap_err();
        assert_eq!(error, Problem::NoAgentapi);
        assert!(fake.sent_by().is_empty());
        assert!(!fake.calls().contains(&"ensure_control".to_string()));
    }

    // MARK: bringing the route up

    #[tokio::test]
    async fn with_nothing_connected_the_control_sidecar_and_the_app_are_brought_up() {
        let mut fake = Fake::new();
        fake.app_running = Some(false);
        send_with(&fake, message(None)).await.unwrap();
        assert_eq!(
            fake.calls(),
            [
                "message_unavailable",
                "ensure_control",
                "app_running",
                "start_app",
                "wait_online askhuman-ctl",
                "run"
            ]
        );
        assert_eq!(fake.sent_by(), ["askhuman-ctl"]);
    }

    #[tokio::test]
    async fn a_running_app_is_not_started_again() {
        let fake = Fake::new();
        send_with(&fake, message(None)).await.unwrap();
        assert!(!fake.calls().contains(&"start_app".to_string()));
        assert_eq!(fake.sent_by(), ["askhuman-ctl"]);
    }

    #[tokio::test]
    async fn a_sidecar_that_never_checks_in_means_nothing_was_sent() {
        let mut fake = Fake::new();
        fake.app_running = Some(false);
        fake.online_after_start = false;
        let error = send_with(&fake, message(None)).await.unwrap_err();
        assert_eq!(error, Problem::NotOnline { started: true });
        assert!(fake.sent_by().is_empty());
    }

    #[tokio::test]
    async fn a_sidecar_that_left_again_or_has_no_agentapi_means_nothing_was_sent() {
        let mut fake = Fake::new();
        fake.link_after_start = None;
        assert_eq!(
            send_with(&fake, message(None)).await.unwrap_err(),
            Problem::NotOnline { started: false }
        );
        let mut fake = Fake::new();
        fake.link_after_start = Some(link(false));
        assert_eq!(
            send_with(&fake, message(None)).await.unwrap_err(),
            Problem::NoAgentapi
        );
        assert!(fake.sent_by().is_empty());
    }

    // MARK: refusing early

    #[tokio::test]
    async fn an_unavailable_route_touches_nothing() {
        let mut fake = Fake::new().with(&[("askhuman-ctl", true)]);
        fake.unavailable = Some(UnavailableReason::SidecarsNotAllowed);
        let error = send_with(&fake, message(None)).await.unwrap_err();
        assert_eq!(
            error,
            Problem::Unavailable(UnavailableReason::SidecarsNotAllowed)
        );
        assert_eq!(fake.calls(), ["message_unavailable"]);
    }

    #[tokio::test]
    async fn a_message_that_cannot_be_an_argument_is_refused_before_anything_is_touched() {
        let fake = Fake::new().with(&[("askhuman-ctl", true)]);
        for (conversation, text) in [
            ("not-an-id", "hello"),
            ("--help", "hello"),
            (CONVERSATION, ""),
            (CONVERSATION, "  \n"),
            (CONVERSATION, "a\0b"),
        ] {
            let error = send_with(
                &fake,
                Message {
                    conversation: conversation.into(),
                    text: text.into(),
                    directory: None,
                },
            )
            .await
            .unwrap_err();
            assert!(
                matches!(error, Problem::Invalid(_)),
                "{conversation:?} {text:?}"
            );
        }
        assert!(fake.calls().is_empty());
    }

    // MARK: what the command said

    #[tokio::test]
    async fn the_command_names_the_conversation_and_carries_the_text_unchanged() {
        let fake = Fake::new().with(&[("askhuman-ctl", true)]);
        let mut sent = message(None);
        sent.text = "- looks odd; $(rm -rf /)".into();
        send_with(&fake, sent).await.unwrap();
        let commands = fake.commands.lock().unwrap();
        let argv = commands[0].1.argv().unwrap();
        assert_eq!(
            argv,
            [
                "send-message",
                "--",
                CONVERSATION,
                "- looks odd; $(rm -rf /)"
            ]
        );
    }

    #[tokio::test]
    async fn how_the_command_ended_decides_whether_it_may_have_been_delivered() {
        let cases: Vec<(Result<Outcome, RunError>, Problem)> = vec![
            (
                Err(RunError::NotSent("sidecar left".into())),
                Problem::NotSent("sidecar left".into()),
            ),
            (
                Err(RunError::Unknown("no answer".into())),
                unclear(Some("no answer".into())),
            ),
            (
                Ok(Outcome {
                    status: Some(1),
                    stderr: "conversation not found".into(),
                    ..Outcome::default()
                }),
                Problem::Rejected("agentapi exited with status 1: conversation not found".into()),
            ),
            (
                Ok(Outcome::failed_to_run("timed out")),
                unclear(Some("timed out".into())),
            ),
        ];
        for (run, expected) in cases {
            let fake = Fake::new().with(&[("askhuman-ctl", true)]).runs(vec![run]);
            assert_eq!(send_with(&fake, message(None)).await.unwrap_err(), expected);
        }
    }

    // MARK: the route's own conditions

    #[test]
    fn a_message_needs_the_app_and_the_users_consent_but_not_the_plugins_mode() {
        let ready = launch::MessageFacts {
            supported: true,
            dev_instance: false,
            app_installed: true,
            manage_sidecars: true,
        };
        assert_eq!(launch::message_reason(ready), None);
        let cases = [
            (
                launch::MessageFacts {
                    supported: false,
                    ..ready
                },
                UnavailableReason::UnsupportedPlatform,
            ),
            (
                launch::MessageFacts {
                    dev_instance: true,
                    ..ready
                },
                UnavailableReason::DevInstance,
            ),
            (
                launch::MessageFacts {
                    app_installed: false,
                    ..ready
                },
                UnavailableReason::AppNotInstalled,
            ),
            (
                launch::MessageFacts {
                    manage_sidecars: false,
                    ..ready
                },
                UnavailableReason::SidecarsNotAllowed,
            ),
        ];
        for (facts, reason) in cases {
            assert_eq!(launch::message_reason(facts), Some(reason));
        }
    }

    #[test]
    fn only_a_conversation_of_the_app_has_a_sidecar_to_carry_a_message() {
        let home = tempfile::tempdir().unwrap();
        let log = |surface: &str, id: &str| {
            let dir = home
                .path()
                .join(".gemini")
                .join(surface)
                .join("brain")
                .join(id)
                .join(".system_generated/logs");
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("transcript_full.jsonl"), "{}\n").unwrap();
        };
        let app = "11111111-1111-4111-8111-111111111111";
        let cli = "22222222-2222-4222-8222-222222222222";
        log("antigravity", app);
        log("antigravity-cli", cli);
        assert!(is_app_conversation(home.path(), app));
        assert!(!is_app_conversation(home.path(), cli));
        // No log, or no id at all: nothing says it belongs to the App.
        assert!(!is_app_conversation(
            home.path(),
            "33333333-3333-4333-8333-333333333333"
        ));
        assert!(!is_app_conversation(home.path(), "../escape"));
    }
}
