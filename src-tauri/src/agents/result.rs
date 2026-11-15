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
    let subagent = kind == AgentKind::Grok && text("subagentType").is_some_and(|s| !s.is_empty())
        || kind == AgentKind::Claude && text("agent_id").is_some_and(|s| !s.is_empty());
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
