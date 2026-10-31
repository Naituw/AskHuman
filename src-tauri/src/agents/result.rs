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
    let subagent = kind == AgentKind::Grok && text("subagentType").is_some_and(|s| !s.is_empty());
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
                .or_else(|| text("reasonDetails")),
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
