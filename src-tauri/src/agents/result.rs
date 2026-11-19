//! Turn outcomes are independent of whether the session is running, idle, or closed.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::AgentKind;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TurnOutcome {
    Completed,
    Failed,
    Interrupted,
    Incomplete,
    #[serde(other)]
    Unknown,
}

impl TurnOutcome {
    pub fn needs_attention(self) -> bool {
        matches!(self, Self::Failed | Self::Incomplete)
    }

    pub fn supersedes(self, previous: Self) -> bool {
        self != previous
            && (previous == Self::Unknown
                || previous == Self::Completed
                    && matches!(self, Self::Failed | Self::Interrupted | Self::Incomplete))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnResult {
    #[serde(default)]
    pub id: String,
    pub outcome: TurnOutcome,
    /// Codex heartbeat intent. Only completion notifications consume it; failures keep their
    /// existing behavior. Absence preserves the legacy result and its serialized shape.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notify: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(default)]
    pub at: u64,
}

/// Metadata is additive on the existing AgentEvent wire message. Old reporters omit it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Observation {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<TurnResult>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub control_operation_id: Option<String>,
    #[serde(default)]
    pub provisional: bool,
    #[serde(default)]
    pub subagent: bool,
    #[serde(default)]
    pub tool_started: bool,
    #[serde(default)]
    pub tool_finished: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub control: Option<Box<super::control::ControlSignal>>,
    /// A `Stop` that leaves work running (a sub-agent, a background task) ends no turn yet. It
    /// carries the end it would be once that work is done, for the daemon that groups an
    /// Antigravity conversation's sub-agents under it (spec D17). The registry never reads it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deferred_end: Option<TurnResult>,
}

fn bounded(value: Option<&str>, maximum: usize) -> Option<String> {
    value.map(str::trim).filter(|s| !s.is_empty()).map(|s| {
        s.chars()
            .filter(|c| !c.is_control() || *c == '\n' || *c == '\t')
            .take(maximum)
            .collect()
    })
}

fn event_name(input: &Value) -> String {
    input
        .get("hook_event_name")
        .or_else(|| input.get("hookEventName"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

/// Whether a hook payload comes from a sub-agent of the session rather than from the session
/// itself. Grok marks its sub-agents with `subagentType`; Claude Code tags every hook input
/// raised inside a sub-agent with that sub-agent's `agent_id`.
pub(super) fn is_subagent_event(kind: AgentKind, input: &Value) -> bool {
    let text = |key: &str| {
        input
            .get(key)
            .and_then(Value::as_str)
            .is_some_and(|s| !s.is_empty())
    };
    kind == AgentKind::Grok && text("subagentType") || kind == AgentKind::Claude && text("agent_id")
}

/// Whether a Claude `Stop` leaves a background sub-agent running.
///
/// The main agent's turn is over but the session's work is not: Claude Code lists the work still
/// in flight in the `Stop` payload's `background_tasks` (only running or pending ones, each with a
/// `type` such as `subagent`, `shell` or `monitor`). When the sub-agent finishes, Claude starts the
/// next turn with its result, and the `Stop` that ends *that* turn is the one that ends the work.
/// Only sub-agents count: a dev server or a watcher in the list may never finish.
///
/// A payload without the list (an older Claude Code) or with an unexpected shape counts as "no
/// sub-agent running", which is the plain turn end.
pub(super) fn stop_leaves_subagents_running(kind: AgentKind, input: &Value) -> bool {
    kind == AgentKind::Claude
        && event_name(input) == "stop"
        // A sub-agent's own stop (`SubagentStop` carries an `agent_id`) is not the session's.
        && !is_subagent_event(kind, input)
        && input
            .get("background_tasks")
            .and_then(Value::as_array)
            .is_some_and(|tasks| {
                tasks.iter().any(|task| {
                    task.get("type").and_then(Value::as_str) == Some("subagent")
                        && matches!(
                            task.get("status").and_then(Value::as_str),
                            Some("running" | "pending")
                        )
                })
            })
}

pub fn observation(
    kind: AgentKind,
    event: super::LifecycleEvent,
    input: Option<&Value>,
) -> Observation {
    let Some(input) = input else {
        return Observation::default();
    };
    let text = |key: &str| input.get(key).and_then(Value::as_str);
    let turn_id = bounded(
        text("turn_id")
            .or_else(|| text("turnId"))
            .or_else(|| text("promptId"))
            .or_else(|| text("generation_id"))
            .or_else(|| text("generationId")),
        256,
    );
    let name = event_name(input);
    let subagent = is_subagent_event(kind, input);
    let mut observation = Observation {
        turn_id: turn_id.clone(),
        subagent,
        control_operation_id: bounded(text("control_operation_id"), 64),
        tool_started: name == "pretooluse",
        tool_finished: matches!(name.as_str(), "posttooluse" | "posttoolusefailure"),
        provisional: kind == AgentKind::Grok
            && name == "stop"
            && !matches!(text("reason"), Some("channel_closed" | "shutdown")),
        ..Observation::default()
    };
    if event != super::LifecycleEvent::TurnEnd {
        return observation;
    }
    let (outcome, reason) = match kind {
        AgentKind::Claude | AgentKind::Grok if name == "stopfailure" => {
            (TurnOutcome::Failed, text("error").or(Some("runtime_error")))
        }
        AgentKind::Grok if name == "stopcancelled" => {
            let reason = text("reason").unwrap_or("unknown");
            let outcome = match reason {
                "user_interrupt" | "permission_rejected" | "permission_cancelled" => {
                    TurnOutcome::Interrupted
                }
                "max_turns" | "no_progress" => TurnOutcome::Incomplete,
                _ if text("cancelledBy") == Some("user") => TurnOutcome::Interrupted,
                _ => TurnOutcome::Unknown,
            };
            (outcome, Some(reason))
        }
        AgentKind::Grok
            if name == "notification"
                || matches!(text("reason"), Some("channel_closed" | "shutdown")) =>
        {
            (TurnOutcome::Unknown, Some("idle_observed"))
        }
        AgentKind::Codex if name == "interrupt" => {
            (TurnOutcome::Interrupted, Some("user_interrupt"))
        }
        AgentKind::Cursor => match text("status") {
            Some("completed") => (TurnOutcome::Completed, None),
            Some("error") => (TurnOutcome::Failed, Some("runtime_error")),
            Some("aborted") => (TurnOutcome::Interrupted, Some("user_interrupt")),
            _ => (TurnOutcome::Unknown, Some("unknown")),
        },
        AgentKind::Pi => match text("stop_reason") {
            Some("stop") => (TurnOutcome::Completed, None),
            Some("error") => (TurnOutcome::Failed, Some("runtime_error")),
            Some("aborted") => (TurnOutcome::Interrupted, Some("user_interrupt")),
            _ => (TurnOutcome::Unknown, Some("unknown")),
        },
        AgentKind::Claude | AgentKind::Codex | AgentKind::Grok if name == "stop" => {
            (TurnOutcome::Completed, None)
        }
        // Only a `Stop` with `fullyIdle` reaches here (the reporter sends it as the turn end).
        AgentKind::Antigravity => match text("terminationReason") {
            Some("NO_TOOL_CALL") => (TurnOutcome::Completed, None),
            Some("ERROR") => (TurnOutcome::Failed, Some("runtime_error")),
            Some("MAX_INVOCATIONS") => (TurnOutcome::Incomplete, Some("max_invocations")),
            Some("MAX_FORCED_INVOCATIONS") => {
                (TurnOutcome::Incomplete, Some("max_forced_invocations"))
            }
            Some("MAX_TOKEN_BUDGET_EXCEEDED") => {
                (TurnOutcome::Incomplete, Some("max_token_budget_exceeded"))
            }
            Some("USER_CANCELED") => (TurnOutcome::Interrupted, Some("user_interrupt")),
            _ => (TurnOutcome::Unknown, Some("unknown")),
        },
        _ => (TurnOutcome::Unknown, Some("unknown")),
    };
    observation.result = Some(TurnResult {
        notify: (kind == AgentKind::Codex && outcome == TurnOutcome::Completed)
            .then(|| {
                text("last_assistant_message")
                    .or_else(|| text("lastAssistantMessage"))
                    .and_then(super::codex_heartbeat::parse)
                    .map(|output| output.notify)
            })
            .flatten(),
        source: Some(
            if kind == AgentKind::Pi {
                "piExtension"
            } else {
                "hook"
            }
            .into(),
        ),
        id: String::new(),
        outcome,
        turn_id,
        reason: bounded(reason, 80),
        summary: bounded(
            text("error_details")
                .or_else(|| text("errorDetails"))
                .or_else(|| text("reasonDetails"))
                // Antigravity's Stop payload carries the error text in `error` ("" otherwise).
                .or_else(|| {
                    (kind == AgentKind::Antigravity)
                        .then(|| text("error"))
                        .flatten()
                })
                .or_else(|| {
                    if name == "stopfailure" {
                        text("last_assistant_message").or_else(|| text("lastAssistantMessage"))
                    } else {
                        None
                    }
                }),
            320,
        ),
        at: 0,
    });
    observation
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn heartbeat_hook_result_preserves_completion_and_old_serialization() {
        let input = serde_json::json!({"hook_event_name":"Stop", "turn_id":"t",
            "last_assistant_message": super::super::codex_heartbeat::QUIET});
        let result = observation(
            AgentKind::Codex,
            super::super::LifecycleEvent::TurnEnd,
            Some(&input),
        )
        .result
        .unwrap();
        assert_eq!(result.outcome, TurnOutcome::Completed);
        assert_eq!(result.notify, Some(false));
        let old = serde_json::json!({"id":"r", "outcome":"completed", "at":123});
        let restored: TurnResult = serde_json::from_value(old.clone()).unwrap();
        assert_eq!(restored.notify, None);
        assert_eq!(serde_json::to_value(restored).unwrap(), old);
    }

    #[test]
    fn a_claude_subagent_failure_is_not_a_main_session_failure_card() {
        let input = json!({"hook_event_name":"StopFailure","agent_id":"child","error":"overloaded","last_assistant_message":"API overloaded"});
        let result = observation(
            AgentKind::Claude,
            super::super::LifecycleEvent::TurnEnd,
            Some(&input),
        );
        assert!(result.subagent);
        assert_eq!(
            result.result.unwrap().summary.as_deref(),
            Some("API overloaded")
        );
    }

    /// A Claude Code `Stop` input carrying the given `background_tasks`: the shape observed with
    /// Claude Code 2.1.321 and found in the bundled source of 2.1.226.
    fn claude_stop(tasks: Value) -> Value {
        json!({
            "session_id": "s",
            "hook_event_name": "Stop",
            "stop_hook_active": false,
            "last_assistant_message": "The sub-agent is running.",
            "background_tasks": tasks,
        })
    }

    fn task(kind: &str, status: &str) -> Value {
        json!({"id": "ab68", "type": kind, "status": status, "description": "run the checks"})
    }

    #[test]
    fn a_claude_stop_with_a_running_subagent_ends_no_turn() {
        for status in ["running", "pending"] {
            let input = claude_stop(json!([task("subagent", status)]));
            assert!(
                stop_leaves_subagents_running(AgentKind::Claude, &input),
                "{status}"
            );
        }
        // Next to other background work it still counts.
        let mixed = claude_stop(json!([
            task("shell", "running"),
            task("subagent", "running")
        ]));
        assert!(stop_leaves_subagents_running(AgentKind::Claude, &mixed));
    }

    #[test]
    fn only_a_running_subagent_keeps_a_claude_stop_from_ending_the_turn() {
        // Nothing left; background commands and watchers, which may never finish; sub-agents that
        // are done; and the shapes an older Claude Code or a malformed payload would send.
        for tasks in [
            json!([]),
            json!([task("shell", "running")]),
            json!([
                task("monitor", "running"),
                task("workflow", "running"),
                task("teammate", "running")
            ]),
            json!([task("subagent", "completed")]),
            json!([task("subagent", "killed"), task("subagent", "failed")]),
            json!([{"id": "x", "type": "subagent"}]),
            json!({"type": "subagent", "status": "running"}),
            json!("subagent"),
            Value::Null,
        ] {
            let input = claude_stop(tasks.clone());
            assert!(
                !stop_leaves_subagents_running(AgentKind::Claude, &input),
                "{tasks}"
            );
        }
        let without_list = json!({"hook_event_name": "Stop", "session_id": "s"});
        assert!(!stop_leaves_subagents_running(
            AgentKind::Claude,
            &without_list
        ));
    }

    #[test]
    fn only_the_main_claude_stop_can_be_held_open_by_a_subagent() {
        let running = json!([task("subagent", "running")]);
        // Other agents are not read with Claude Code's payload shape.
        for kind in [
            AgentKind::Codex,
            AgentKind::Cursor,
            AgentKind::Grok,
            AgentKind::Pi,
            AgentKind::Antigravity,
        ] {
            assert!(
                !stop_leaves_subagents_running(kind, &claude_stop(running.clone())),
                "{}",
                kind.as_str()
            );
        }
        // A sub-agent's own stop and the other ways Claude Code ends a turn are not held open.
        let mut subagent_stop = claude_stop(running.clone());
        subagent_stop["hook_event_name"] = json!("SubagentStop");
        subagent_stop["agent_id"] = json!("ab68");
        assert!(!stop_leaves_subagents_running(
            AgentKind::Claude,
            &subagent_stop
        ));
        let mut tagged = claude_stop(running.clone());
        tagged["agent_id"] = json!("ab68");
        assert!(!stop_leaves_subagents_running(AgentKind::Claude, &tagged));
        let mut failure = claude_stop(running);
        failure["hook_event_name"] = json!("StopFailure");
        assert!(!stop_leaves_subagents_running(AgentKind::Claude, &failure));
    }

    #[test]
    fn a_hook_input_from_inside_a_subagent_carries_its_own_agents_marker() {
        let claude = |input: Value| is_subagent_event(AgentKind::Claude, &input);
        assert!(claude(
            json!({"hook_event_name": "PreToolUse", "agent_id": "ab68"})
        ));
        assert!(!claude(
            json!({"hook_event_name": "PreToolUse", "agent_id": ""})
        ));
        assert!(!claude(json!({"hook_event_name": "PreToolUse"})));
        // A marker belongs to one agent: Claude's is not Grok's, and Codex has neither.
        assert!(is_subagent_event(
            AgentKind::Grok,
            &json!({"subagentType": "explore"})
        ));
        assert!(!claude(json!({"subagentType": "explore"})));
        assert!(!is_subagent_event(
            AgentKind::Grok,
            &json!({"agent_id": "ab68"})
        ));
        assert!(!is_subagent_event(
            AgentKind::Codex,
            &json!({"agent_id": "ab68", "subagentType": "explore"})
        ));
    }

    #[test]
    fn cancellation_is_not_success_or_failure() {
        for (kind, input) in [
            (
                AgentKind::Codex,
                json!({"hook_event_name":"Interrupt", "turn_id":"t"}),
            ),
            (
                AgentKind::Grok,
                json!({"hookEventName":"stop_cancelled", "promptId":"t", "reason":"permission_rejected"}),
            ),
            (AgentKind::Cursor, json!({"status":"aborted"})),
            (AgentKind::Pi, json!({"stop_reason":"aborted"})),
        ] {
            let result = observation(kind, super::super::LifecycleEvent::TurnEnd, Some(&input))
                .result
                .unwrap();
            assert_eq!(result.outcome, TurnOutcome::Interrupted);
            assert!(!result.outcome.needs_attention());
        }
    }

    #[test]
    fn an_antigravity_stop_maps_its_termination_reason_to_one_outcome() {
        let outcome = |reason: &str, error: &str| {
            let input = json!({
                "hookEventName": "Stop", "fullyIdle": true,
                "terminationReason": reason, "error": error,
            });
            observation(
                AgentKind::Antigravity,
                super::super::LifecycleEvent::TurnEnd,
                Some(&input),
            )
            .result
            .unwrap()
        };
        assert_eq!(outcome("NO_TOOL_CALL", "").outcome, TurnOutcome::Completed);
        let failed = outcome("ERROR", "model overloaded");
        assert_eq!(failed.outcome, TurnOutcome::Failed);
        assert_eq!(failed.summary.as_deref(), Some("model overloaded"));
        assert_eq!(outcome("ERROR", "").summary, None);
        for (reason, expected) in [
            ("MAX_INVOCATIONS", "max_invocations"),
            ("MAX_FORCED_INVOCATIONS", "max_forced_invocations"),
            ("MAX_TOKEN_BUDGET_EXCEEDED", "max_token_budget_exceeded"),
        ] {
            let result = outcome(reason, "");
            assert_eq!(result.outcome, TurnOutcome::Incomplete, "{reason}");
            assert_eq!(result.reason.as_deref(), Some(expected));
            assert!(result.outcome.needs_attention());
        }
        let cancelled = outcome("USER_CANCELED", "");
        assert_eq!(cancelled.outcome, TurnOutcome::Interrupted);
        assert!(!cancelled.outcome.needs_attention());
        // Terminal steps and hooks, injected responses and future reasons stay unknown.
        for reason in [
            "TERMINAL_STEP_TYPE",
            "TERMINAL_CUSTOM_HOOK",
            "INJECTED_RESPONSE",
            "NEW",
        ] {
            assert_eq!(
                outcome(reason, "").outcome,
                TurnOutcome::Unknown,
                "{reason}"
            );
        }
        // Only the turn end carries a result; other events never claim one.
        let activity = observation(
            AgentKind::Antigravity,
            super::super::LifecycleEvent::Activity,
            Some(&json!({"hookEventName": "Stop", "fullyIdle": false})),
        );
        assert!(activity.result.is_none() && !activity.subagent);
    }

    #[test]
    fn runtime_limits_are_incomplete_and_unknown_remains_unknown() {
        for reason in ["max_turns", "no_progress", "a_future_reason"] {
            let input = json!({"hook_event_name":"StopCancelled", "reason":reason});
            let result = observation(
                AgentKind::Grok,
                super::super::LifecycleEvent::TurnEnd,
                Some(&input),
            )
            .result
            .unwrap();
            assert_eq!(result.reason.as_deref(), Some(reason));
            assert_eq!(
                result.outcome,
                if reason == "a_future_reason" {
                    TurnOutcome::Unknown
                } else {
                    TurnOutcome::Incomplete
                }
            );
        }
    }
}
