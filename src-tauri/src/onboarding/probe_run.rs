//! Running the setup wizard's optional checks (`docs/plans/onboarding.md` §4.3–4.4).
//!
//! Three checks, none of them part of the wizard's main flow and none able to block it:
//! a question to the desktop popup, a question to exactly one remote channel, and a watch for
//! the first question an Agent asks. Each runs on its own task in the process that hosts the
//! wizard window and reports back through one event. `probe` holds the pure rules.

use crate::config::AppConfig;
use crate::i18n::Lang;
use crate::ipc::{self, transport, ClientMsg, ServerMsg};
use crate::onboarding::probe::{self, Outcome, ProbeTarget};
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;
use tauri::{AppHandle, Emitter};
use tokio::io::BufReader;
use tokio::sync::Notify;

/// The window listens for this; the payload is a [`ProbeResult`].
pub const RESULT_EVENT: &str = "setup-probe-result";
/// How long a question check waits for the person to answer.
pub const ASK_TIMEOUT: Duration = Duration::from_secs(120);
/// How long the Agent check waits for an Agent's first question.
pub const AGENT_TIMEOUT: Duration = Duration::from_secs(300);
/// Pause before the Agent watch looks for the daemon again.
const RECONNECT_DELAY: Duration = Duration::from_secs(2);

/// One of the three checks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Check {
    Popup,
    Channel(&'static str),
    Agent,
}

impl Check {
    /// `kind` is `popup`, `channel` (with the channel id) or `agent`.
    pub fn parse(kind: &str, channel: Option<&str>) -> Result<Check, String> {
        match kind {
            "popup" => Ok(Check::Popup),
            "agent" => Ok(Check::Agent),
            "channel" => match channel.and_then(ProbeTarget::parse) {
                Some(ProbeTarget::Channel(id)) => Ok(Check::Channel(id)),
                _ => Err(format!("unknown channel: {}", channel.unwrap_or_default())),
            },
            other => Err(format!("unknown check: {other}")),
        }
    }

    /// The kind name the window uses, and the slot the check occupies: at most one check of a
    /// kind runs at a time.
    fn slot(self) -> &'static str {
        match self {
            Check::Popup => "popup",
            Check::Channel(_) => "channel",
            Check::Agent => "agent",
        }
    }

    fn channel(self) -> Option<&'static str> {
        match self {
            Check::Channel(id) => Some(id),
            _ => None,
        }
    }
}

/// What the window learns when a check ends.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProbeResult {
    pub probe_id: String,
    pub kind: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub channel: Option<&'static str>,
    pub outcome: Outcome,
    /// The family of the Agent whose question the Agent check saw.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_kind: Option<String>,
}

/// The checks that are running, one per slot.
#[derive(Default)]
struct Registry {
    running: HashMap<&'static str, (String, Arc<Notify>)>,
}

impl Registry {
    /// Make `id` the running check of its slot; a check it replaces is told to stop.
    fn begin(&mut self, slot: &'static str, id: &str) -> Arc<Notify> {
        let cancel = Arc::new(Notify::new());
        if let Some((_, previous)) = self.running.insert(slot, (id.to_string(), cancel.clone())) {
            previous.notify_one();
        }
        cancel
    }

    /// Tell a running check to stop; false when no check has this id.
    fn cancel(&mut self, id: &str) -> bool {
        match self.running.values().find(|(running, _)| running == id) {
            Some((_, notify)) => {
                notify.notify_one();
                true
            }
            None => false,
        }
    }

    /// The check ended. True when it was still the one its slot tracked (a replaced check is
    /// not, and its result is of no interest to the window any more).
    fn finish(&mut self, slot: &'static str, id: &str) -> bool {
        let current = self
            .running
            .get(slot)
            .is_some_and(|(running, _)| running == id);
        if current {
            self.running.remove(slot);
        }
        current
    }
}

static REGISTRY: LazyLock<Mutex<Registry>> = LazyLock::new(Mutex::default);

/// Why a check stopped without finishing.
#[derive(Debug, PartialEq, Eq)]
enum Stop {
    Timeout,
    Cancelled,
}

impl From<Stop> for Outcome {
    fn from(stop: Stop) -> Outcome {
        match stop {
            Stop::Timeout => Outcome::Timeout,
            Stop::Cancelled => Outcome::Cancelled,
        }
    }
}

/// Run `work` until it finishes, `limit` passes or `cancel` is signalled. Dropping `work` is what
/// cancels a question: its connection closes and the daemon withdraws the request everywhere.
async fn drive<T>(
    work: impl Future<Output = T>,
    cancel: &Notify,
    limit: Duration,
) -> Result<T, Stop> {
    tokio::select! {
        done = tokio::time::timeout(limit, work) => done.map_err(|_| Stop::Timeout),
        _ = cancel.notified() => Err(Stop::Cancelled),
    }
}

/// Start a check on a background task and return its id; the end is reported through
/// [`RESULT_EVENT`]. A check of the same kind that is still running is stopped first.
pub fn start(app: AppHandle, kind: &str, channel: Option<&str>) -> Result<String, String> {
    let check = Check::parse(kind, channel)?;
    let id = uuid::Uuid::new_v4().to_string();
    let cancel = REGISTRY.lock().unwrap().begin(check.slot(), &id);
    let lang = Lang::resolve(&AppConfig::load_without_secrets().general.language);
    let probe_id = id.clone();
    tauri::async_runtime::spawn(async move {
        let (outcome, agent_kind) = run(check, &probe_id, lang, &cancel).await;
        if REGISTRY.lock().unwrap().finish(check.slot(), &probe_id) {
            let _ = app.emit(
                RESULT_EVENT,
                ProbeResult {
                    probe_id,
                    kind: check.slot(),
                    channel: check.channel(),
                    outcome,
                    agent_kind,
                },
            );
        }
    });
    Ok(id)
}

/// Stop a check that is running (the window closing, or the person cancelling).
pub fn cancel(id: &str) -> bool {
    REGISTRY.lock().unwrap().cancel(id)
}

async fn run(check: Check, id: &str, lang: Lang, cancel: &Notify) -> (Outcome, Option<String>) {
    let target = match check {
        Check::Popup => ProbeTarget::Popup,
        Check::Channel(channel) => ProbeTarget::Channel(channel),
        Check::Agent => {
            return match drive(watch_agent_ask(), cancel, AGENT_TIMEOUT).await {
                Ok(kind) => (Outcome::Ok, Some(kind)),
                Err(stop) => (stop.into(), None),
            };
        }
    };
    let task = probe::build_task(target, id, lang);
    let accepted = Mutex::new(None);
    match drive(
        crate::client::internal_ask(task, &accepted),
        cancel,
        ASK_TIMEOUT,
    )
    .await
    {
        Ok((stdout, exit_code)) => (probe::outcome_of(exit_code, &stdout), None),
        Err(stop) => (stop.into(), None),
    }
}

/// Wait for the first question an Agent asks, and name the Agent. It never starts a daemon and
/// does not keep one alive: a menu-bar subscription is enough, so it connects whenever a daemon
/// exists. What was already waiting when the watch began is not "first".
async fn watch_agent_ask() -> String {
    let mut first_connection = true;
    loop {
        if let Ok(stream) = transport::connect().await {
            let (reader, mut writer) = stream.into_split();
            let mut reader = BufReader::new(reader);
            if ipc::write_msg(&mut writer, &ClientMsg::TraySubscribe)
                .await
                .is_ok()
            {
                let mut baseline: Option<HashSet<String>> = None;
                while let Ok(Some(msg)) = ipc::read_msg::<_, ServerMsg>(&mut reader).await {
                    let ServerMsg::TrayState {
                        pending_requests, ..
                    } = msg
                    else {
                        continue;
                    };
                    // The first frame of the connection the watch began with is the world as it
                    // was; a daemon that came up later has nothing older than the watch.
                    let known = baseline.get_or_insert_with(|| {
                        if first_connection {
                            probe::ids_of(&pending_requests)
                        } else {
                            HashSet::new()
                        }
                    });
                    if let Some(kind) = probe::first_new_agent_ask(known, &pending_requests) {
                        return kind;
                    }
                }
            }
        }
        first_connection = false;
        tokio::time::sleep(RECONNECT_DELAY).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};

    #[test]
    fn a_check_names_its_kind_and_for_a_channel_which_one() {
        assert_eq!(Check::parse("popup", None), Ok(Check::Popup));
        assert_eq!(Check::parse("agent", None), Ok(Check::Agent));
        assert_eq!(
            Check::parse("channel", Some("telegram")),
            Ok(Check::Channel("telegram"))
        );
        // The channel id is only read for a channel check.
        assert_eq!(Check::parse("popup", Some("slack")), Ok(Check::Popup));
    }

    #[test]
    fn anything_else_is_refused_by_name() {
        assert!(Check::parse("", None).is_err());
        assert!(Check::parse("everything", None).is_err());
        assert!(Check::parse("channel", None)
            .unwrap_err()
            .contains("channel"));
        assert!(Check::parse("channel", Some("ios")).is_err());
        assert!(Check::parse("channel", Some("popup")).is_err());
        assert!(Check::parse("channel", Some("Slack")).is_err());
    }

    #[test]
    fn a_check_occupies_the_slot_of_its_kind() {
        assert_eq!(Check::Popup.slot(), "popup");
        assert_eq!(Check::Agent.slot(), "agent");
        // Any channel shares one slot, so two channel checks never run at once.
        assert_eq!(Check::Channel("slack").slot(), "channel");
        assert_eq!(Check::Channel("feishu").slot(), "channel");
        assert_eq!(Check::Channel("slack").channel(), Some("slack"));
        assert_eq!(Check::Popup.channel(), None);
    }

    #[test]
    fn a_new_check_replaces_the_one_of_its_kind_and_stops_it() {
        let mut registry = Registry::default();
        let first = registry.begin("popup", "one");
        let other_kind = registry.begin("agent", "agent-one");
        let second = registry.begin("popup", "two");
        // The replaced check holds a stop signal; the others do not.
        let waited = futures_now(first.notified());
        assert!(waited, "the replaced check was told to stop");
        assert!(!futures_now(other_kind.notified()));
        assert!(!futures_now(second.notified()));
        // Only the newest check of the slot is current.
        assert!(!registry.finish("popup", "one"));
        assert!(registry.finish("popup", "two"));
        assert!(!registry.finish("popup", "two"));
    }

    #[test]
    fn cancelling_by_id_signals_that_check_and_only_it() {
        let mut registry = Registry::default();
        let popup = registry.begin("popup", "p1");
        let channel = registry.begin("channel", "c1");
        assert!(registry.cancel("c1"));
        assert!(futures_now(channel.notified()));
        assert!(!futures_now(popup.notified()));
        assert!(!registry.cancel("nobody"));
        // Cancelling does not end the check: it ends itself and then finishes.
        assert!(registry.finish("channel", "c1"));
    }

    /// Whether a `Notified` future is ready right now (a stored permit counts).
    fn futures_now(notified: impl Future<Output = ()>) -> bool {
        use std::task::{Context, Poll, Waker};
        let mut notified = std::pin::pin!(notified);
        matches!(
            notified
                .as_mut()
                .poll(&mut Context::from_waker(Waker::noop())),
            Poll::Ready(())
        )
    }

    #[tokio::test(start_paused = true)]
    async fn a_question_that_is_answered_reports_what_it_returned() {
        let cancel = Notify::new();
        let result = drive(
            async { ("answer".to_string(), 0) },
            &cancel,
            Duration::from_secs(120),
        )
        .await;
        assert_eq!(result, Ok(("answer".to_string(), 0)));
    }

    #[tokio::test(start_paused = true)]
    async fn a_question_nobody_answers_times_out() {
        let cancel = Notify::new();
        let result = drive(
            std::future::pending::<()>(),
            &cancel,
            Duration::from_secs(120),
        )
        .await;
        assert_eq!(result, Err(Stop::Timeout));
    }

    #[tokio::test(start_paused = true)]
    async fn the_stop_signal_ends_a_check_and_drops_its_work() {
        struct Flag(Arc<AtomicBool>);
        impl Drop for Flag {
            fn drop(&mut self) {
                self.0.store(true, Ordering::SeqCst);
            }
        }
        let dropped = Arc::new(AtomicBool::new(false));
        let guard = Flag(dropped.clone());
        let cancel = Notify::new();
        // A permit stored before the check started waiting still stops it.
        cancel.notify_one();
        let result = drive(
            async move {
                let _guard = guard;
                std::future::pending::<()>().await
            },
            &cancel,
            Duration::from_secs(120),
        )
        .await;
        assert_eq!(result, Err(Stop::Cancelled));
        // Dropping the work is what closes the question's connection and withdraws it.
        assert!(dropped.load(Ordering::SeqCst));
    }

    #[test]
    fn the_outcome_of_a_stop_matches_its_name() {
        assert_eq!(Outcome::from(Stop::Timeout), Outcome::Timeout);
        assert_eq!(Outcome::from(Stop::Cancelled), Outcome::Cancelled);
    }

    #[test]
    fn the_result_event_carries_what_the_window_needs() {
        let value = serde_json::to_value(ProbeResult {
            probe_id: "p1".into(),
            kind: "channel",
            channel: Some("slack"),
            outcome: Outcome::Timeout,
            agent_kind: None,
        })
        .unwrap();
        assert_eq!(
            value,
            serde_json::json!({
                "probeId": "p1",
                "kind": "channel",
                "channel": "slack",
                "outcome": "timeout",
            })
        );
        let value = serde_json::to_value(ProbeResult {
            probe_id: "p2".into(),
            kind: "agent",
            channel: None,
            outcome: Outcome::Ok,
            agent_kind: Some("claude".into()),
        })
        .unwrap();
        assert_eq!(
            value,
            serde_json::json!({
                "probeId": "p2",
                "kind": "agent",
                "outcome": "ok",
                "agentKind": "claude",
            })
        );
    }

    #[test]
    fn the_waits_are_the_ones_the_spec_names() {
        assert_eq!(ASK_TIMEOUT, Duration::from_secs(120));
        assert_eq!(AGENT_TIMEOUT, Duration::from_secs(300));
    }
}
