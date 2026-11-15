//! Antigravity lifecycle reporter.
//!
//! The managed AskHuman plugin registers three Antigravity hooks (spec
//! `antigravity-deep-integration` §6.2): `SessionStart` and `PreInvocation` run
//! `AskHuman __agent-hook antigravity <event>`, and `Stop` runs the shared
//! `AskHuman __stop-hook antigravity track`. Both entry points end up in [`report_for`], which is a
//! pure mapping from the hook payload to one normalized lifecycle event (spec §8):
//!
//! | hook | lifecycle event |
//! | --- | --- |
//! | `SessionStart` | `session-start` |
//! | `PreInvocation`, `invocationNum == 0` | `turn-start` |
//! | `PreInvocation`, `invocationNum > 0` | `activity` |
//! | `Stop`, `fullyIdle == true` | `turn-end` |
//! | `Stop`, `fullyIdle == false` | `activity` |
//!
//! Every conversation, including a sub-agent's, is reported on its own: this module does not decide
//! whether a conversation is a root or a child (spec §8 L3 / L4, decision D17 is still open).
//!
//! The hook contract is silent: exit code 0 and an empty stdout, whatever happens. A failure to
//! reach the daemon is dropped, because tracking must never slow down or break the agent.

use serde_json::Value;
use sha2::{Digest, Sha256};

use super::{detect, AgentKind, LifecycleEvent};
use crate::ipc::ClientMsg;

/// The Antigravity hooks AskHuman registers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookEvent {
    SessionStart,
    PreInvocation,
    Stop,
}

impl HookEvent {
    /// The argument written into the plugin's `hooks.json` command line.
    pub fn as_str(self) -> &'static str {
        match self {
            HookEvent::SessionStart => "session-start",
            HookEvent::PreInvocation => "pre-invocation",
            HookEvent::Stop => "stop",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim() {
            "session-start" => Some(HookEvent::SessionStart),
            "pre-invocation" => Some(HookEvent::PreInvocation),
            "stop" => Some(HookEvent::Stop),
            _ => None,
        }
    }
}

/// Which Antigravity product a conversation belongs to, from the directory of its transcript.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Surface {
    /// The desktop App (`~/.gemini/antigravity/…`).
    App,
    /// The `agy` command line (`~/.gemini/antigravity-cli/…`).
    Cli,
    /// The IDE and its extension (`~/.gemini/antigravity-ide/…`); not supported.
    Ide,
    Unknown,
}

pub fn surface(transcript_path: Option<&str>) -> Surface {
    let Some(path) = transcript_path else {
        return Surface::Unknown;
    };
    let mut found = Surface::Unknown;
    for component in path.split(['/', '\\']) {
        match component {
            "antigravity-cli" => return Surface::Cli,
            "antigravity-ide" => return Surface::Ide,
            "antigravity" => found = Surface::App,
            _ => {}
        }
    }
    found
}

/// What one hook invocation tells the daemon.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    pub event: LifecycleEvent,
    pub session_id: String,
    pub cwd: Option<String>,
    pub transcript_path: Option<String>,
    /// Only on `turn-start`: pairs the turn with a task AskHuman launched (spec §12.8).
    pub prompt_sha256: Option<String>,
    pub failed: bool,
}

fn text<'a>(input: &'a Value, key: &str) -> Option<&'a str> {
    input
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

/// The project of a conversation: the first workspace root, else the hook's `cwd`. The hook
/// process itself runs in the plugin directory, so its own working directory means nothing.
fn project(input: &Value) -> Option<String> {
    input
        .get("workspacePaths")
        .and_then(Value::as_array)
        .and_then(|paths| paths.iter().find_map(Value::as_str))
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .or_else(|| text(input, "cwd"))
        .map(str::to_string)
}

/// Whether the payload's `Stop` ends the whole conversation tree's current work. A `Stop` with
/// `fullyIdle == false` leaves background work or a sub-agent running, and the agent wakes itself
/// again when that finishes (spec §4.1).
pub fn stop_is_final(input: &Value) -> bool {
    input.get("fullyIdle").and_then(Value::as_bool) == Some(true)
}

/// A `Stop` that ended in an error; the only turn outcome that raises a failure card.
pub fn stop_failed(input: &Value) -> bool {
    stop_is_final(input) && text(input, "terminationReason") == Some("ERROR")
}

/// Maps one hook payload to the event to report; `None` when it must not be reported at all
/// (no conversation id, or a surface AskHuman does not support).
pub fn report_for(event: HookEvent, input: &Value) -> Option<Report> {
    let session_id = text(input, "conversationId")?.to_string();
    let transcript_path = text(input, "transcriptPath").map(str::to_string);
    if surface(transcript_path.as_deref()) == Surface::Ide {
        return None;
    }
    let lifecycle = match event {
        HookEvent::SessionStart => LifecycleEvent::SessionStart,
        HookEvent::PreInvocation => {
            if input.get("invocationNum").and_then(Value::as_u64) == Some(0) {
                LifecycleEvent::TurnStart
            } else {
                LifecycleEvent::Activity
            }
        }
        HookEvent::Stop => {
            if stop_is_final(input) {
                LifecycleEvent::TurnEnd
            } else {
                LifecycleEvent::Activity
            }
        }
    };
    // `lastUserInput` is the latest message the user typed. A turn the agent started by itself
    // (a finished background task) repeats the previous one, which can only ever pair with a
    // launch that is already consumed.
    let prompt_sha256 = (lifecycle == LifecycleEvent::TurnStart)
        .then(|| text(input, "lastUserInput"))
        .flatten()
        .map(|prompt| format!("{:x}", Sha256::digest(prompt.as_bytes())));
    Some(Report {
        event: lifecycle,
        session_id,
        cwd: project(input),
        transcript_path,
        prompt_sha256,
        failed: lifecycle == LifecycleEvent::TurnEnd && stop_failed(input),
    })
}

/// The wire message for a report, shared by both entry points.
fn message(report: Report, input: &Value) -> ClientMsg {
    ClientMsg::AgentEvent {
        agent: AgentKind::Antigravity.as_str().to_string(),
        event: report.event.as_str().to_string(),
        session_id: report.session_id,
        pid: None,
        // The daemon walks up from here: an `agy` session finds its own process, the App's shared
        // `language_server` never matches an agent and leaves the session without a pid.
        hint_pid: detect::parent_pid(std::process::id()),
        cwd: report.cwd,
        transcript_path: report.transcript_path,
        launch_id: std::env::var(crate::integrations::agent_launch::LAUNCH_ID_ENV).ok(),
        prompt_sha256: report.prompt_sha256,
        ts: 0,
        tool: None,
        interject_poll: false,
        failed: report.failed,
        observation: Some(Box::new(super::result::observation(
            AgentKind::Antigravity,
            report.event,
            Some(input),
        ))),
    }
}

/// Reports one hook payload to the daemon. Best effort; returns whether a message was sent.
pub fn report(event: HookEvent, input: &Value) -> bool {
    let Some(report) = report_for(event, input) else {
        return false;
    };
    super::session_info::capture_hook(AgentKind::Antigravity, input, &detect::env_snapshot());
    crate::client::report_agent_event(message(report, input));
    true
}

/// `AskHuman __agent-hook antigravity <event>`; `args` starts at the event name.
pub fn run(args: &[String]) {
    let Some(event) = args.first().and_then(|value| HookEvent::parse(value)) else {
        return;
    };
    if let Some(input) = super::report::read_stdin_json() {
        report(event, &input);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Real payloads recorded from the App and `agy` on 2026-11-16, sanitized (see each file's
    /// first line for its source).
    const APP_SINGLE_TURN: &str = include_str!("antigravity_hook/fixtures/app-single-turn.jsonl");
    const APP_SUBAGENT: &str = include_str!("antigravity_hook/fixtures/app-subagent-session.jsonl");
    const AGY_SINGLE_TURN: &str = include_str!("antigravity_hook/fixtures/agy-single-turn.jsonl");

    struct Recorded {
        at_ms: u64,
        env_conversation_id: String,
        payload: Value,
    }

    fn recorded(fixture: &str) -> Vec<Recorded> {
        fixture
            .lines()
            .map(|line| serde_json::from_str::<Value>(line).unwrap())
            .filter(|value| value.get("fixture").is_none())
            .map(|value| Recorded {
                at_ms: value["atMs"].as_u64().unwrap(),
                env_conversation_id: value["envConversationId"].as_str().unwrap().to_string(),
                payload: value["payload"].clone(),
            })
            .collect()
    }

    fn hook_event(payload: &Value) -> HookEvent {
        match payload["hookEventName"].as_str().unwrap() {
            "SessionStart" => HookEvent::SessionStart,
            "PreInvocation" => HookEvent::PreInvocation,
            "Stop" => HookEvent::Stop,
            other => panic!("unexpected hook {other}"),
        }
    }

    fn events(fixture: &str) -> Vec<(String, LifecycleEvent)> {
        recorded(fixture)
            .iter()
            .map(|entry| {
                let report = report_for(hook_event(&entry.payload), &entry.payload).unwrap();
                (report.session_id, report.event)
            })
            .collect()
    }

    #[test]
    fn a_turn_with_a_long_tool_call_is_one_working_stretch() {
        // SessionStart, PreInvocation 0, PreInvocation 1 (92 s later), Stop(fullyIdle).
        let sequence: Vec<_> = events(APP_SINGLE_TURN)
            .into_iter()
            .map(|(_, event)| event)
            .collect();
        assert_eq!(
            sequence,
            [
                LifecycleEvent::SessionStart,
                LifecycleEvent::TurnStart,
                LifecycleEvent::Activity,
                LifecycleEvent::TurnEnd,
            ]
        );
    }

    #[test]
    fn the_cli_reports_the_same_events_as_the_app() {
        let sequence: Vec<_> = events(AGY_SINGLE_TURN)
            .into_iter()
            .map(|(_, event)| event)
            .collect();
        assert_eq!(
            sequence,
            [
                LifecycleEvent::SessionStart,
                LifecycleEvent::TurnStart,
                LifecycleEvent::Activity,
                LifecycleEvent::Activity,
                LifecycleEvent::TurnEnd,
            ]
        );
        for entry in recorded(AGY_SINGLE_TURN) {
            assert_eq!(
                surface(entry.payload["transcriptPath"].as_str()),
                Surface::Cli
            );
        }
        for entry in recorded(APP_SINGLE_TURN) {
            assert_eq!(
                surface(entry.payload["transcriptPath"].as_str()),
                Surface::App
            );
        }
    }

    #[test]
    fn each_conversation_of_a_subagent_session_is_reported_on_its_own() {
        const ROOT: &str = "11111111-1111-4111-8111-111111111111";
        const CHILD: &str = "22222222-2222-4222-8222-222222222222";
        let all = events(APP_SUBAGENT);
        let of = |id: &str| -> Vec<LifecycleEvent> {
            all.iter()
                .filter(|(session, _)| session == id)
                .map(|(_, event)| *event)
                .collect()
        };
        // A `Stop(fullyIdle=false)` never ends a turn: the root only refreshes its activity, even
        // though no later root `Stop` ever reports `fullyIdle=true` (spec §8 L4).
        assert!(!of(ROOT).contains(&LifecycleEvent::TurnEnd));
        // The woken root starts a new execution (`invocationNum` 0 again).
        assert_eq!(
            of(ROOT)
                .iter()
                .filter(|event| **event == LifecycleEvent::TurnStart)
                .count(),
            2
        );
        // The child ends its own turn with its own `Stop(fullyIdle=true)`.
        assert_eq!(of(CHILD).last(), Some(&LifecycleEvent::TurnEnd));
        assert_eq!(of(CHILD).first(), Some(&LifecycleEvent::SessionStart));
    }

    #[test]
    fn the_environment_conversation_matches_the_payload_in_every_recording() {
        for fixture in [APP_SINGLE_TURN, APP_SUBAGENT, AGY_SINGLE_TURN] {
            for entry in recorded(fixture) {
                assert_eq!(
                    entry.env_conversation_id,
                    entry.payload["conversationId"].as_str().unwrap()
                );
            }
        }
    }

    #[test]
    fn the_project_is_the_workspace_not_the_plugin_directory() {
        let entry = &recorded(APP_SINGLE_TURN)[1];
        let report = report_for(HookEvent::PreInvocation, &entry.payload).unwrap();
        assert_eq!(report.cwd.as_deref(), Some("/Users/tester/project"));
        assert_eq!(
            report.transcript_path.as_deref(),
            entry.payload["transcriptPath"].as_str()
        );
        // The first workspace root wins over `cwd`, and `cwd` is the fallback.
        let multi = json!({
            "conversationId": "c", "invocationNum": 0, "cwd": "/b",
            "workspacePaths": ["/a", "/b"],
        });
        assert_eq!(
            report_for(HookEvent::PreInvocation, &multi)
                .unwrap()
                .cwd
                .as_deref(),
            Some("/a")
        );
        let bare = json!({"conversationId": "c", "invocationNum": 0, "cwd": "/b"});
        assert_eq!(
            report_for(HookEvent::PreInvocation, &bare)
                .unwrap()
                .cwd
                .as_deref(),
            Some("/b")
        );
    }

    #[test]
    fn only_the_first_invocation_starts_a_turn_and_only_it_carries_the_prompt() {
        let events = recorded(APP_SINGLE_TURN);
        let first = report_for(HookEvent::PreInvocation, &events[1].payload).unwrap();
        let later = report_for(HookEvent::PreInvocation, &events[2].payload).unwrap();
        assert_eq!(first.event, LifecycleEvent::TurnStart);
        assert_eq!(later.event, LifecycleEvent::Activity);
        let expected = format!(
            "{:x}",
            Sha256::digest(
                events[1].payload["lastUserInput"]
                    .as_str()
                    .unwrap()
                    .as_bytes()
            )
        );
        assert_eq!(first.prompt_sha256.as_deref(), Some(expected.as_str()));
        assert_eq!(later.prompt_sha256, None);
        // A missing invocation number cannot claim a new turn.
        let unnumbered = json!({"conversationId": "c", "lastUserInput": "hi"});
        let report = report_for(HookEvent::PreInvocation, &unnumbered).unwrap();
        assert_eq!(report.event, LifecycleEvent::Activity);
        // A conversation nobody typed into (a sub-agent) has no prompt to pair.
        let child = json!({"conversationId": "c", "invocationNum": 0, "lastUserInput": ""});
        let report = report_for(HookEvent::PreInvocation, &child).unwrap();
        assert_eq!(
            (report.event, report.prompt_sha256),
            (LifecycleEvent::TurnStart, None)
        );
    }

    #[test]
    fn a_stop_is_final_only_when_the_whole_conversation_is_idle() {
        let base = |idle: Value, reason: &str| json!({"conversationId": "c", "fullyIdle": idle, "terminationReason": reason});
        let ended = report_for(HookEvent::Stop, &base(json!(true), "NO_TOOL_CALL")).unwrap();
        assert_eq!(
            (ended.event, ended.failed),
            (LifecycleEvent::TurnEnd, false)
        );
        let waiting = report_for(HookEvent::Stop, &base(json!(false), "NO_TOOL_CALL")).unwrap();
        assert_eq!(waiting.event, LifecycleEvent::Activity);
        // An unreadable flag is not idle: the 30-minute backstop settles it instead.
        let unknown = report_for(HookEvent::Stop, &json!({"conversationId": "c"})).unwrap();
        assert_eq!(unknown.event, LifecycleEvent::Activity);
    }

    #[test]
    fn an_error_stop_is_a_failed_turn_only_when_final() {
        // Synthetic: no real run produced `ERROR`; the value comes from the host's documentation.
        let input = |idle: bool| json!({"conversationId": "c", "fullyIdle": idle, "terminationReason": "ERROR", "error": "boom"});
        let failed = report_for(HookEvent::Stop, &input(true)).unwrap();
        assert_eq!(
            (failed.event, failed.failed),
            (LifecycleEvent::TurnEnd, true)
        );
        let not_final = report_for(HookEvent::Stop, &input(false)).unwrap();
        assert_eq!(
            (not_final.event, not_final.failed),
            (LifecycleEvent::Activity, false)
        );
    }

    #[test]
    fn nothing_is_reported_without_a_conversation_or_for_the_ide() {
        for event in [
            HookEvent::SessionStart,
            HookEvent::PreInvocation,
            HookEvent::Stop,
        ] {
            assert!(report_for(event, &json!({})).is_none());
            assert!(report_for(event, &json!({"conversationId": "  "})).is_none());
        }
        let ide = json!({
            "conversationId": "c",
            "transcriptPath": "/Users/tester/.gemini/antigravity-ide/brain/c/.system_generated/logs/transcript_full.jsonl",
        });
        assert!(report_for(HookEvent::SessionStart, &ide).is_none());
    }

    #[test]
    fn hook_names_round_trip_and_unknown_names_are_ignored() {
        for event in [
            HookEvent::SessionStart,
            HookEvent::PreInvocation,
            HookEvent::Stop,
        ] {
            assert_eq!(HookEvent::parse(event.as_str()), Some(event));
        }
        assert_eq!(HookEvent::parse("PreInvocation"), None);
        assert_eq!(HookEvent::parse(""), None);
    }

    #[test]
    fn the_surface_comes_from_the_brain_directory() {
        let path = |dir: &str| format!("/Users/tester/.gemini/{dir}/brain/c/x.jsonl");
        assert_eq!(surface(Some(&path("antigravity"))), Surface::App);
        assert_eq!(surface(Some(&path("antigravity-cli"))), Surface::Cli);
        assert_eq!(surface(Some(&path("antigravity-ide"))), Surface::Ide);
        assert_eq!(surface(Some(&path("elsewhere"))), Surface::Unknown);
        assert_eq!(surface(None), Surface::Unknown);
    }

    /// Feeds a recording through the mapping and the registry exactly as the daemon would, with
    /// the pid left out (the App's hook parent is the shared language server).
    fn replay_into(registry: &super::super::registry::AgentRegistry, fixture: &str, take: usize) {
        for entry in recorded(fixture).iter().take(take) {
            let report = report_for(hook_event(&entry.payload), &entry.payload).unwrap();
            let observation = super::super::result::observation(
                AgentKind::Antigravity,
                report.event,
                Some(&entry.payload),
            );
            registry.apply_observed(
                AgentKind::Antigravity,
                report.event,
                &report.session_id,
                None,
                report.cwd,
                0,
                Some(&observation),
            );
        }
    }

    fn state_of(registry: &super::super::registry::AgentRegistry, session: &str) -> Option<String> {
        registry
            .snapshot()
            .as_array()
            .unwrap()
            .iter()
            .find(|record| record["sessionId"] == session)
            .map(|record| record["state"].as_str().unwrap().to_string())
    }

    #[test]
    fn a_recorded_turn_is_working_until_its_final_stop_and_then_idle() {
        const SESSION: &str = "33333333-3333-4333-8333-333333333333";
        let registry = super::super::registry::AgentRegistry::new();
        // The 92 s wait inside the tool call sits between the second and third event.
        replay_into(&registry, APP_SINGLE_TURN, 3);
        assert_eq!(state_of(&registry, SESSION).as_deref(), Some("working"));
        replay_into(&registry, APP_SINGLE_TURN, 4);
        assert_eq!(state_of(&registry, SESSION).as_deref(), Some("idle"));
        let result = registry
            .last_turn_result(AgentKind::Antigravity, SESSION)
            .unwrap();
        assert_eq!(result.outcome, super::super::result::TurnOutcome::Completed);
    }

    #[test]
    fn a_session_is_registered_by_its_first_hook_without_a_pid() {
        let registry = super::super::registry::AgentRegistry::new();
        replay_into(&registry, AGY_SINGLE_TURN, 1);
        let snapshot = registry.snapshot();
        let record = &snapshot.as_array().unwrap()[0];
        assert_eq!(record["kind"], "antigravity");
        assert_eq!(record["cwd"], "/Users/tester/project");
        // Nobody resolved a pid here; the real resolution is the daemon's walk from `hint_pid`.
        assert!(record["pid"].is_null());
    }

    #[test]
    fn the_subagent_recording_pins_the_flat_behavior_of_the_open_decision() {
        // Spec §8 L4 / D17. Today every conversation stands alone: the child finishes, while the
        // root's last `Stop` said `fullyIdle=false` and no later root `Stop` exists, so the root stays
        // working until the 30-minute backstop. If D17 chooses to merge sub-agents into a tree, this
        // is the assertion that changes (the whole tree is idle after the last event).
        const ROOT: &str = "11111111-1111-4111-8111-111111111111";
        const CHILD: &str = "22222222-2222-4222-8222-222222222222";
        let registry = super::super::registry::AgentRegistry::new();
        replay_into(&registry, APP_SUBAGENT, usize::MAX);
        assert_eq!(state_of(&registry, CHILD).as_deref(), Some("idle"));
        assert_eq!(state_of(&registry, ROOT).as_deref(), Some("working"));
    }

    #[test]
    fn recorded_gaps_match_the_timeline_the_fixtures_claim() {
        // The sub-agent fixture is ordered by time and the child outlives the root's last Stop.
        let events = recorded(APP_SUBAGENT);
        assert!(events.windows(2).all(|pair| pair[0].at_ms <= pair[1].at_ms));
        let last = events.last().unwrap();
        assert_eq!(last.payload["fullyIdle"], json!(true));
        assert_eq!(
            last.payload["conversationId"],
            json!("22222222-2222-4222-8222-222222222222")
        );
    }
}
