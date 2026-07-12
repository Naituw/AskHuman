//! Stop confirmation hook. It reuses the ordinary single-select Ask flow and emits each agent's
//! native continuation JSON from the human's decision.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::{json, Value};

use super::{AgentKind, LifecycleEvent};
use crate::i18n::Lang;
use crate::ipc::TaskRequest;
use crate::models::{MessagePrompt, OptionItem, OutputFormat, Question};

const MAX_LAST_MESSAGE_CHARS: usize = 2_000;
const MAX_INSTRUCTION_CHARS: usize = 1_000;
const HOOK_WAIT_SECS: u64 = 24 * 60 * 60;

#[derive(Debug, Clone, PartialEq, Eq)]
enum StopDecision {
    End,
    Continue(Option<String>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct LastAssistantMessage {
    display: Option<String>,
    user_confirmed_end_turn: bool,
}

/// Entry point for `AskHuman __stop-hook <agent> [track] [confirm]`.
/// Stdout always contains exactly one JSON object.
pub fn run(args: &[String]) {
    let output = run_inner(args).unwrap_or_else(|| json!({}));
    println!("{output}");
}

fn run_inner(args: &[String]) -> Option<Value> {
    let kind = args.first().and_then(|value| AgentKind::parse(value))?;
    if kind == AgentKind::Grok {
        return None;
    }
    let track = args.get(1).is_some_and(|value| value == "track");
    let confirm = args.iter().skip(1).any(|value| value == "confirm");
    let env: HashMap<String, String> = std::env::vars().collect();
    if super::report::should_skip(kind, &env) {
        return None;
    }
    let input = super::report::read_stdin_json()?;
    let session_id = super::report::resolve_session_id(kind, &env, Some(&input));
    if session_id.is_empty() {
        return None;
    }
    let cwd = super::report::resolve_cwd(&env, Some(&input));

    // Cursor also emits stop for aborted/error turns, but only consumes followup_message for a
    // completed turn. Non-natural stops only retain the existing lifecycle turn-end behavior.
    if !is_natural_stop(kind, &input) {
        if track {
            super::report::report_simple_event(kind, LifecycleEvent::TurnEnd, session_id, cwd);
        }
        return None;
    }

    if !confirm {
        if track {
            super::report::report_simple_event(kind, LifecycleEvent::TurnEnd, session_id, cwd);
        }
        return None;
    }

    let last_message = last_assistant_message(kind, &input);
    if last_message.user_confirmed_end_turn {
        if track {
            super::report::report_simple_event(kind, LifecycleEvent::TurnEnd, session_id, cwd);
        }
        return None;
    }
    let lang = Lang::current();
    let task = build_task(
        kind,
        &session_id,
        cwd.as_deref(),
        last_message.display.as_deref(),
        lang,
    );
    let captured = crate::client::run_ask_capture(task, Duration::from_secs(HOOK_WAIT_SECS));
    let decision = captured
        .as_deref()
        .map(parse_ask_decision)
        .unwrap_or(StopDecision::End);

    match decision {
        StopDecision::Continue(instruction) => Some(continuation_output(
            kind,
            &crate::prompts::stop_continue_prompt(instruction.as_deref()),
        )),
        StopDecision::End => {
            if track {
                super::report::report_simple_event(kind, LifecycleEvent::TurnEnd, session_id, cwd);
            }
            None
        }
    }
}

fn is_natural_stop(kind: AgentKind, input: &Value) -> bool {
    match kind {
        AgentKind::Cursor => input.get("status").and_then(Value::as_str) == Some("completed"),
        AgentKind::Claude | AgentKind::Codex => input
            .get("hook_event_name")
            .or_else(|| input.get("hookEventName"))
            .and_then(Value::as_str)
            .is_some_and(|event| event.eq_ignore_ascii_case("stop")),
        AgentKind::Grok => false,
    }
}

fn last_assistant_message(kind: AgentKind, input: &Value) -> LastAssistantMessage {
    let raw = match kind {
        AgentKind::Claude | AgentKind::Codex => input
            .get("last_assistant_message")
            .or_else(|| input.get("lastAssistantMessage"))
            .and_then(Value::as_str)
            .filter(|text| !text.trim().is_empty())
            .map(str::to_string),
        AgentKind::Cursor => cursor_last_message(input),
        AgentKind::Grok => None,
    };
    normalize_last_assistant_message(raw.as_deref())
}

fn normalize_last_assistant_message(raw: Option<&str>) -> LastAssistantMessage {
    let Some(raw) = raw else {
        return LastAssistantMessage {
            display: None,
            user_confirmed_end_turn: false,
        };
    };
    let (cleaned, user_confirmed_end_turn) = strip_confirmed_end_turn_marker(raw);
    LastAssistantMessage {
        display: (!cleaned.trim().is_empty())
            .then(|| truncate_preserving_layout(&cleaned, MAX_LAST_MESSAGE_CHARS)),
        user_confirmed_end_turn,
    }
}

fn cursor_last_message(input: &Value) -> Option<String> {
    cursor_last_message_under(input, &crate::paths::cursor_dir())
}

fn cursor_last_message_under(input: &Value, root: &Path) -> Option<String> {
    let raw = input.get("transcript_path")?.as_str()?.trim();
    if raw.is_empty() || raw.chars().count() > 8_192 {
        return None;
    }
    let path = canonical_under(Path::new(raw), root)?;
    super::activity::resolve_last_assistant_text_from_path_raw(AgentKind::Cursor, &path)
}

fn strip_confirmed_end_turn_marker(text: &str) -> (String, bool) {
    let trimmed = text.trim_end();
    let marker = crate::prompts::USER_CONFIRMED_END_TURN_MARKER;
    let Some(prefix) = trimmed.strip_suffix(marker) else {
        return (text.to_string(), false);
    };
    if !prefix.is_empty() && !prefix.ends_with(['\n', '\r']) {
        return (text.to_string(), false);
    }
    (prefix.trim_end_matches(['\n', '\r']).to_string(), true)
}

fn canonical_under(path: &Path, root: &Path) -> Option<PathBuf> {
    let path = std::fs::canonicalize(path).ok()?;
    let root = std::fs::canonicalize(root).ok()?;
    (path.is_file() && path.starts_with(root)).then_some(path)
}

fn truncate_preserving_layout(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        text.to_string()
    } else {
        let head: String = text.chars().take(max_chars).collect();
        format!("{}\n\n… [truncated]", head.trim_end())
    }
}

fn build_task(
    kind: AgentKind,
    session_id: &str,
    cwd: Option<&str>,
    last_message: Option<&str>,
    lang: Lang,
) -> TaskRequest {
    let (question, continue_label, end_label, unavailable) = match lang {
        Lang::Zh => (
            "Agent 准备结束本轮对话，接下来怎么做？",
            "继续对话",
            "结束对话",
            "未能读取 Agent 的最后一段回复。",
        ),
        Lang::En => (
            "The Agent is ready to end this turn. What should happen next?",
            "Continue conversation",
            "End conversation",
            "The Agent's last response could not be read.",
        ),
    };
    let message = last_message.unwrap_or(unavailable).to_string();
    TaskRequest {
        message: MessagePrompt::new(message, Vec::new()),
        questions: vec![Question::new(
            question.to_string(),
            vec![
                OptionItem::new(continue_label, true),
                OptionItem::new(end_label, false),
            ],
        )],
        is_markdown: true,
        source: crate::models::source_name_for_agent(Some(kind)),
        lang: lang.code().to_string(),
        project: cwd.unwrap_or_default().to_string(),
        select_only: false,
        single: true,
        output_format: OutputFormat::Json,
        record_history: false,
        agent_kind: Some(kind.as_str().to_string()),
        agent_session_id: Some(session_id.to_string()),
        agent_pid: None,
        caller_pid: std::process::id(),
        from_mcp: false,
        perf_id: String::new(),
        perf_autodismiss: false,
    }
}

fn parse_ask_decision(stdout: &str) -> StopDecision {
    let Ok(value) = serde_json::from_str::<Value>(stdout) else {
        return StopDecision::End;
    };
    if value.get("action").and_then(Value::as_str) != Some("answer") {
        return StopDecision::End;
    }
    let Some(answer) = value
        .get("answers")
        .and_then(Value::as_array)
        .and_then(|answers| answers.first())
    else {
        return StopDecision::End;
    };
    let indices = answer
        .get("selected_indices")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if indices.iter().any(|index| index.as_u64() == Some(1)) {
        return StopDecision::End;
    }
    let instruction = answer
        .get("user_input")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(|text| truncate_preserving_layout(text, MAX_INSTRUCTION_CHARS));
    if indices.iter().any(|index| index.as_u64() == Some(0)) || instruction.is_some() {
        StopDecision::Continue(instruction)
    } else {
        StopDecision::End
    }
}

fn continuation_output(kind: AgentKind, prompt: &str) -> Value {
    match kind {
        AgentKind::Cursor => json!({ "followup_message": prompt }),
        AgentKind::Claude | AgentKind::Codex => {
            json!({ "decision": "block", "reason": prompt })
        }
        AgentKind::Grok => json!({}),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn natural_stop_matrix() {
        assert!(is_natural_stop(
            AgentKind::Claude,
            &json!({"hook_event_name":"Stop"})
        ));
        assert!(is_natural_stop(
            AgentKind::Codex,
            &json!({"hook_event_name":"Stop"})
        ));
        assert!(is_natural_stop(
            AgentKind::Cursor,
            &json!({"status":"completed"})
        ));
        assert!(!is_natural_stop(
            AgentKind::Cursor,
            &json!({"status":"error"})
        ));
        assert!(!is_natural_stop(
            AgentKind::Cursor,
            &json!({"status":"aborted"})
        ));
        assert!(!is_natural_stop(
            AgentKind::Claude,
            &json!({"hook_event_name":"StopFailure"})
        ));
        assert!(!is_natural_stop(AgentKind::Codex, &json!({})));
    }

    #[test]
    fn maps_ask_results_fail_open() {
        assert_eq!(parse_ask_decision("not json"), StopDecision::End);
        assert_eq!(parse_ask_decision("{}"), StopDecision::End);
        assert_eq!(
            parse_ask_decision(r#"{"action":"cancel"}"#),
            StopDecision::End
        );
        assert_eq!(
            parse_ask_decision(
                r#"{"action":"answer","answers":[{"selected_indices":[1],"user_input":"ignored"}]}"#
            ),
            StopDecision::End
        );
        assert_eq!(
            parse_ask_decision(r#"{"action":"answer","answers":[{"selected_indices":[0]}]}"#),
            StopDecision::Continue(None)
        );
        assert_eq!(
            parse_ask_decision(r#"{"action":"answer","answers":[{"user_input":"  next step  "}]}"#),
            StopDecision::Continue(Some("next step".into()))
        );
        assert_eq!(
            parse_ask_decision(
                r#"{"action":"answer","answers":[{"selected_indices":[],"user_input":""}]}"#
            ),
            StopDecision::End
        );
    }

    #[test]
    fn output_shapes_match_agents() {
        let claude = continuation_output(AgentKind::Claude, "continue");
        assert_eq!(claude["decision"], "block");
        assert_eq!(claude["reason"], "continue");
        let codex = continuation_output(AgentKind::Codex, "continue");
        assert_eq!(codex["decision"], "block");
        let cursor = continuation_output(AgentKind::Cursor, "continue");
        assert_eq!(cursor["followup_message"], "continue");
    }

    #[test]
    fn last_message_threshold_preserves_layout_and_unicode() {
        let short = "第一行\n第二行";
        assert_eq!(truncate_preserving_layout(short, 20), short);
        assert_eq!(
            last_assistant_message(
                AgentKind::Codex,
                &json!({"last_assistant_message":"\n  indented\n"})
            )
            .display
            .as_deref(),
            Some("\n  indented\n")
        );
        let long = "你".repeat(2_001);
        let out = truncate_preserving_layout(&long, 2_000);
        assert_eq!(out.matches('你').count(), 2_000);
        assert!(out.ends_with("… [truncated]"));
        let exact = "你".repeat(2_000);
        assert_eq!(truncate_preserving_layout(&exact, 2_000), exact);
        assert_eq!(
            last_assistant_message(AgentKind::Claude, &json!({"last_assistant_message":"  "})),
            LastAssistantMessage {
                display: None,
                user_confirmed_end_turn: false
            }
        );
    }

    #[test]
    fn confirmed_end_turn_marker_requires_an_exact_final_line_and_is_stripped() {
        let marker = crate::prompts::USER_CONFIRMED_END_TURN_MARKER;
        for kind in [AgentKind::Claude, AgentKind::Codex] {
            let confirmed = last_assistant_message(
                kind,
                &json!({"last_assistant_message": format!("final report\n{marker}\n")}),
            );
            assert!(confirmed.user_confirmed_end_turn);
            assert_eq!(confirmed.display.as_deref(), Some("final report"));
        }

        for text in [
            format!("prefix {marker}"),
            format!("{marker}\nmore text"),
            format!("quoted `{marker}`"),
            format!("{marker}-suffix"),
        ] {
            let message =
                last_assistant_message(AgentKind::Codex, &json!({"last_assistant_message": text}));
            assert!(!message.user_confirmed_end_turn);
        }

        let long = format!("{}\n{marker}", "你".repeat(2_001));
        let message =
            last_assistant_message(AgentKind::Claude, &json!({"last_assistant_message": long}));
        assert!(message.user_confirmed_end_turn);
        assert_eq!(message.display.unwrap().matches('你').count(), 2_000);
    }

    #[test]
    fn internal_task_is_single_free_text_and_skips_history() {
        let task = build_task(
            AgentKind::Codex,
            "s1",
            Some("/tmp/p"),
            Some("done"),
            Lang::En,
        );
        assert!(task.single);
        assert!(!task.select_only);
        assert!(!task.record_history);
        assert_eq!(task.output_format, OutputFormat::Json);
        assert_eq!(task.questions[0].predefined_options.len(), 2);
        assert_eq!(task.message.text, "done");
        assert!(task.questions[0].predefined_options[0].recommended);
        assert!(!task.questions[0].predefined_options[1].recommended);
    }

    #[test]
    fn transcript_path_must_resolve_to_a_file_under_expected_root() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let inside_file = root.path().join("conversation.jsonl");
        let outside_file = outside.path().join("conversation.jsonl");
        std::fs::write(&inside_file, "{}").unwrap();
        std::fs::write(&outside_file, "{}").unwrap();
        assert_eq!(
            canonical_under(&inside_file, root.path()),
            std::fs::canonicalize(&inside_file).ok()
        );
        assert!(canonical_under(&outside_file, root.path()).is_none());
        assert!(canonical_under(&root.path().join("missing"), root.path()).is_none());
        assert!(canonical_under(root.path(), root.path()).is_none());
    }

    #[test]
    fn cursor_transcript_marker_is_detected_before_display_truncation() {
        let root = tempfile::tempdir().unwrap();
        let transcript = root.path().join("conversation.jsonl");
        let marker = crate::prompts::USER_CONFIRMED_END_TURN_MARKER;
        let event = json!({
            "role": "assistant",
            "message": {"content": [
                {"type": "text", "text": "你".repeat(2_001)},
                {"type": "text", "text": marker}
            ]}
        });
        std::fs::write(&transcript, format!("{event}\n")).unwrap();
        let raw = cursor_last_message_under(&json!({"transcript_path": transcript}), root.path());
        let message = normalize_last_assistant_message(raw.as_deref());
        assert!(message.user_confirmed_end_turn);
        assert_eq!(message.display.unwrap().matches('你').count(), 2_000);
    }
}
