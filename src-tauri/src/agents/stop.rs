//! Stop confirmation hook. It reuses the ordinary single-select Ask flow and emits each agent's
//! native continuation JSON from the human's decision.

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
    quiet: bool,
}

/// Entry point for `AskHuman __stop-hook <agent> [track] [confirm]`.
/// Stdout always contains exactly one JSON object.
pub fn run(args: &[String]) {
    let antigravity =
        args.first().and_then(|value| AgentKind::parse(value)) == Some(AgentKind::Antigravity);
    match run_inner(args) {
        Some(output) => println!("{output}"),
        // Antigravity's contract is an empty stdout for "no decision" (spec §8); `{}` is not
        // verified there.
        None if antigravity => {}
        None => println!("{}", json!({})),
    }
}

fn run_inner(args: &[String]) -> Option<Value> {
    let kind = args.first().and_then(|value| AgentKind::parse(value))?;
    if kind == AgentKind::Grok {
        return None;
    }
    let track = args.get(1).is_some_and(|value| value == "track");
    let confirm = args.iter().skip(1).any(|value| value == "confirm");
    let env = super::detect::env_snapshot();
    // Antigravity is never a compatibility-loaded copy of another agent's hook.
    if kind != AgentKind::Antigravity && super::report::should_skip(kind, &env) {
        return None;
    }
    let mut input = super::report::read_stdin_json()?;
    let session_id = super::report::resolve_session_id(kind, &env, Some(&input));
    if session_id.is_empty() {
        return None;
    }
    if let Some(reason) = super::t3_host::classify_and_record_hook(kind, &session_id) {
        super::t3_host::log_suppressed(
            "stop_confirmation",
            Some(kind.as_str()),
            Some(&session_id),
            None,
            reason,
        );
        return None;
    }
    let cwd = stop_cwd(kind, &env, &input);

    // A Stop that leaves a background sub-agent running ends no turn: the session is still working
    // and nobody is waiting to be asked yet. The sub-agent's result starts the next turn, and the
    // Stop that ends that turn is the one to confirm. Until then the session only shows activity.
    if super::result::stop_leaves_subagents_running(kind, &input) {
        if track || confirm {
            super::report::report_simple_event(
                kind,
                LifecycleEvent::Activity,
                session_id,
                cwd,
                &input,
            );
        }
        return None;
    }

    // Cursor also emits stop for aborted/error turns, but only consumes followup_message for a
    // completed turn. Non-natural stops only retain the existing lifecycle turn-end behavior.
    if !is_natural_stop(kind, &input) {
        if track || confirm {
            report_turn_end(kind, session_id, cwd, &input);
        }
        return None;
    }

    if !confirm {
        if track || confirm {
            report_turn_end(kind, session_id, cwd, &input);
        }
        return None;
    }
    if let Some(reason) = confirmation_suppression_reason(kind, &session_id, &input) {
        crate::daemon::lifecycle::log_guard_audit(crate::daemon::lifecycle::GuardAudit {
            component: "stop_confirmation",
            action: "suppressed",
            reason: &reason,
            tool: None,
            agent: Some(kind.as_str()),
            thread_source: None,
            session_id: Some(&session_id),
            thread_id: input
                .get("thread_id")
                .or_else(|| input.get("threadId"))
                .and_then(Value::as_str),
            turn_id: input
                .get("turn_id")
                .or_else(|| input.get("turnId"))
                .and_then(Value::as_str),
        });
        if track || confirm {
            report_turn_end(kind, session_id, cwd, &input);
        }
        return None;
    }

    let last_message = last_assistant_message(kind, &input);
    if last_message.user_confirmed_end_turn || last_message.quiet {
        if track || confirm {
            report_turn_end(kind, session_id, cwd, &input);
        }
        return None;
    }
    if kind == AgentKind::Antigravity {
        // A sub-agent's own Stop, or a pause for work that is still running, ends nothing the human
        // is waiting for; the daemon knows which one this is and may first hold the hook until the
        // sub-agents are done (spec D17).
        match super::antigravity_hook::gate(&input) {
            super::antigravity_hook::Gate::Skip => {
                report_turn_end(kind, session_id, cwd, &input);
                return None;
            }
            super::antigravity_hook::Gate::Card { held } => {
                if held {
                    super::antigravity_hook::mark_idle(&mut input);
                }
            }
        }
    }
    let lang = Lang::current();
    // Todo dispatch on the Stop card (spec todo-whats-next D5): the hook's cwd maps to the git
    // root, whose pending todos become leading options. Dequeue happens centrally in the
    // Coordinator (options carry todo_id), not here. Only the first MAX_OPTION_TODOS entries
    // become options (channel option-count limits, round 14); the overflow note goes into the
    // question text. `parse_ask_decision` gets the same truncated list (index math stays aligned).
    super::session_info::capture_hook(kind, &input, &env);
    let project = crate::project::for_session(
        Some(kind.as_str()),
        Some(&session_id),
        Path::new(cwd.as_deref().unwrap_or("")),
    );
    let mut todos = if project.is_empty() {
        Vec::new()
    } else {
        crate::todo_sync::service::before_read(&project);
        crate::todos::list(&project)
    };
    let total_todos = todos.len();
    todos.truncate(crate::todos::MAX_OPTION_TODOS);
    let mut task = build_task(
        kind,
        &session_id,
        &project,
        &todos,
        total_todos,
        last_message.display.as_deref(),
        lang,
    );
    task.caller_cwd = cwd.clone();
    let captured = crate::client::run_ask_capture(task, Duration::from_secs(HOOK_WAIT_SECS));
    let decision = captured
        .as_deref()
        .map(|stdout| parse_ask_decision(stdout, &todos))
        .unwrap_or(StopDecision::End);

    match decision {
        StopDecision::Continue(instruction) => Some(continuation_output(
            kind,
            &crate::prompts::stop_continue_prompt(kind, instruction.as_deref()),
        )),
        StopDecision::End => {
            if track || confirm {
                report_turn_end(kind, session_id, cwd, &input);
            }
            None
        }
    }
}

/// The project a Stop is reported for. Antigravity's hook runs in the plugin directory, so its
/// payload's workspace is the project; the others use the hook's own `cwd`.
fn stop_cwd(
    kind: AgentKind,
    env: &std::collections::HashMap<String, String>,
    input: &Value,
) -> Option<String> {
    if kind == AgentKind::Antigravity {
        return super::antigravity_hook::project(input);
    }
    super::report::resolve_cwd(env, Some(input))
}

/// Reports the end of a turn. Antigravity's reporter decides from the payload whether the whole
/// conversation went idle (a Stop that left background work running is only activity).
fn report_turn_end(kind: AgentKind, session_id: String, cwd: Option<String>, input: &Value) {
    if kind == AgentKind::Antigravity {
        super::antigravity_hook::report(super::antigravity_hook::HookEvent::Stop, input);
        return;
    }
    super::report::report_simple_event(kind, LifecycleEvent::TurnEnd, session_id, cwd, input);
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
        AgentKind::Antigravity => super::antigravity_hook::stop_is_natural(input),
        AgentKind::Pi => {
            input.get("stop_reason").and_then(Value::as_str) == Some("stop")
                && input
                    .get("hook_event_name")
                    .and_then(Value::as_str)
                    .is_some_and(|event| event.eq_ignore_ascii_case("stop"))
        }
    }
}

/// Whether a hook payload describes a turn that ended in an error (as opposed to finishing or
/// being interrupted by the user). Codex has no failure hook, so it never reports one.
pub(super) fn turn_failed(kind: AgentKind, input: &Value) -> bool {
    let text = |key: &str| input.get(key).and_then(Value::as_str);
    match kind {
        AgentKind::Claude | AgentKind::Grok => text("hook_event_name")
            .or_else(|| text("hookEventName"))
            .is_some_and(|event| event.eq_ignore_ascii_case("stopfailure")),
        // `aborted` is the user pressing stop, not a failure.
        AgentKind::Cursor => text("status") == Some("error"),
        AgentKind::Pi => text("stop_reason") == Some("error"),
        AgentKind::Antigravity => super::antigravity_hook::stop_failed(input),
        AgentKind::Codex => false,
    }
}

/// Why a Codex Stop must not ask the human (`None` = ask as usual).
///
/// Codex's Stop hook input carries no `thread_source` (the `StopCommandInput` fields are identical
/// from 0.144.1 through 0.163.0), so unattended threads are recognized by what the input does
/// carry plus the delegated-run signals (spec `codex-delegated-run-silence`):
///
/// 1. `transcript_path: null` means a thread without a rollout (ambient / system / ephemeral).
///    This holds even when the user allows delegated runs to use AskHuman, because nobody can
///    answer such a thread.
/// 2. A delegated run (`codex exec` / `review`, plugin-driven app-server thread) stays silent,
///    as if AskHuman were not integrated, unless that preference allows it.
fn confirmation_suppression_reason(
    kind: AgentKind,
    session_id: &str,
    input: &Value,
) -> Option<String> {
    suppression_reason_with_mute(
        kind,
        session_id,
        input,
        &crate::daemon::away::session_muted,
        &super::delegated::silenced,
    )
}

/// Audit reason of a Stop confirmation silenced because the user turned AskHuman off for the
/// session (`docs/specs/special-replies.md` D6).
const MUTED_REASON: &str = "session_muted";

/// `confirmation_suppression_reason` with the mute lookup injected. The user's own instruction
/// comes first: while AskHuman is off for the session no card is shown, and the hook reports the
/// turn end like any other unconfirmed stop.
fn suppression_reason_with_mute(
    kind: AgentKind,
    session_id: &str,
    input: &Value,
    muted: &dyn Fn(&str) -> bool,
    delegated_silence: &dyn Fn(&str, Option<&Value>) -> Option<String>,
) -> Option<String> {
    if muted(session_id) {
        return Some(MUTED_REASON.to_string());
    }
    confirmation_suppression_reason_with(kind, session_id, input, delegated_silence)
}

fn confirmation_suppression_reason_with(
    kind: AgentKind,
    session_id: &str,
    input: &Value,
    delegated_silence: &dyn Fn(&str, Option<&Value>) -> Option<String>,
) -> Option<String> {
    if kind != AgentKind::Codex {
        return None;
    }
    if super::delegated::transcript_path_is_null(input) {
        return Some(
            super::delegated::Signal::TranscriptNull
                .reason()
                .to_string(),
        );
    }
    delegated_silence(session_id, Some(input))
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
        AgentKind::Antigravity => antigravity_last_message(input),
        AgentKind::Grok => None,
        AgentKind::Pi => input
            .get("last_assistant_message")
            .or_else(|| input.get("lastAssistantMessage"))
            .and_then(Value::as_str)
            .filter(|text| !text.trim().is_empty())
            .map(str::to_string),
    };
    let heartbeat = (kind == AgentKind::Codex)
        .then(|| raw.as_deref().and_then(super::codex_heartbeat::parse))
        .flatten();
    let mut message = normalize_last_assistant_message(
        heartbeat
            .as_ref()
            .map(|result| result.display_text)
            .or(raw.as_deref()),
    );
    message.quiet = heartbeat.is_some_and(|result| !result.notify);
    message
}

fn normalize_last_assistant_message(raw: Option<&str>) -> LastAssistantMessage {
    let Some(raw) = raw else {
        return LastAssistantMessage {
            display: None,
            user_confirmed_end_turn: false,
            quiet: false,
        };
    };
    let (cleaned, user_confirmed_end_turn) = strip_confirmed_end_turn_marker(raw);
    LastAssistantMessage {
        display: (!cleaned.trim().is_empty())
            .then(|| truncate_preserving_layout(&cleaned, MAX_LAST_MESSAGE_CHARS)),
        user_confirmed_end_turn,
        quiet: false,
    }
}

/// The agent's last reply, read from the conversation log. Prefers a structurally validated
/// `transcriptPath` from the hook payload (covering isolated ACP homes such as T3 Code even inside
/// this short-lived hook process), and falls back to looking up the log by conversation id.
fn antigravity_last_message(input: &Value) -> Option<String> {
    antigravity_last_message_with(input, &|id| {
        if let Some(raw) = input
            .get("transcriptPath")
            .or_else(|| input.get("transcript_path"))
            .and_then(Value::as_str)
        {
            if let Some(path) =
                super::session_paths::validate_path(AgentKind::Antigravity, id, raw, None)
            {
                return Some(path);
            }
        }
        super::title::transcript_path(AgentKind::Antigravity, id)
    })
}

/// [`antigravity_last_message`] with the lookup of the log injected.
fn antigravity_last_message_with(
    input: &Value,
    log: &dyn Fn(&str) -> Option<PathBuf>,
) -> Option<String> {
    let id = input.get("conversationId")?.as_str()?.trim();
    let path = log(id)?;
    super::activity::resolve_last_assistant_text_from_path_raw(AgentKind::Antigravity, &path)
}

fn cursor_last_message(input: &Value) -> Option<String> {
    let mut roots = vec![crate::paths::cursor_dir()];
    roots.extend(crate::integrations::agent_profiles::enabled_roots(
        AgentKind::Cursor,
    ));
    roots
        .into_iter()
        .find_map(|root| cursor_last_message_under(input, &root))
}

fn cursor_last_message_under(input: &Value, root: &Path) -> Option<String> {
    let raw = input.get("transcript_path")?.as_str()?.trim();
    if raw.is_empty() || raw.chars().count() > 8_192 {
        return None;
    }
    let path = canonical_under(Path::new(raw), root)?;
    super::activity::resolve_last_assistant_text_from_path_raw(AgentKind::Cursor, &path)
}

/// Detect the end-turn marker anywhere in the last assistant message.
///
/// The prompt still asks agents to put it on a final independent line, but in practice they wrap
/// it in markdown, glue punctuation, or write more text after it. Any occurrence of the exact
/// marker substring is treated as user-confirmed end so the Stop card is not shown again.
fn strip_confirmed_end_turn_marker(text: &str) -> (String, bool) {
    let marker = crate::prompts::USER_CONFIRMED_END_TURN_MARKER;
    if !text.contains(marker) {
        return (text.to_string(), false);
    }
    // Remove every occurrence; trim edges left by marker-only lines.
    let cleaned = text.replace(marker, "").trim().to_string();
    (cleaned, true)
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

pub(crate) fn build_task(
    kind: AgentKind,
    session_id: &str,
    project: &str,
    todos: &[crate::todos::TodoEntry],
    total_todos: usize,
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
    // Options: todo chips first (spec D5), then the two original actions. Index math in
    // `parse_ask_decision` relies on this order. Labels carry the "Run todo: " display prefix
    // (same as whats-next); the continuation text comes from the raw entry via index, not the label.
    let mut options: Vec<OptionItem> = todos
        .iter()
        .map(|entry| OptionItem::with_todo_entry(crate::todos::option_label(lang, entry), entry))
        .collect();
    options.push(OptionItem::new(continue_label, true));
    options.push(OptionItem::new(end_label, false));
    // Overflow note (round 14): appended to the question text when the queue exceeds the cap.
    let mut question = question.to_string();
    if let Some(note) = crate::todos::overflow_note(total_todos, lang) {
        question.push_str("\n\n");
        question.push_str(&note);
    }
    TaskRequest {
        caller_cwd: None,
        native_request_id: None,
        message: MessagePrompt::new(message, Vec::new()),
        questions: vec![Question::new(question, options)],
        is_markdown: true,
        source: crate::models::source_name_for_agent(Some(kind)),
        lang: lang.code().to_string(),
        project: project.to_string(),
        select_only: false,
        single: true,
        output_format: OutputFormat::Json,
        record_history: true,
        stop_confirmation: true,
        agent_kind: Some(kind.as_str().to_string()),
        agent_session_id: Some(session_id.to_string()),
        agent_session_candidate: None,
        mcp_instance_id: None,
        agent_pid: None,
        caller_pid: std::process::id(),
        from_mcp: false,
        perf_id: String::new(),
        perf_autodismiss: false,
        whats_next: false,
        escalate: false,
        escalation_reason: None,
    }
}

/// Map the Ask JSON to a decision. Option order is fixed by `build_task`: todo chips occupy
/// indices `0..todos.len()`, then "continue" and "end" (spec D5 mapping):
/// end = end (text discarded) · todo (± text) = continue with that todo (+ supplement) ·
/// continue / free text only = original semantics. Dequeue is not done here (Coordinator does it).
fn parse_ask_decision(stdout: &str, todos: &[crate::todos::TodoEntry]) -> StopDecision {
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
    let continue_index = todos.len() as u64;
    let end_index = continue_index + 1;
    let indices = answer
        .get("selected_indices")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if indices
        .iter()
        .any(|index| index.as_u64() == Some(end_index))
    {
        return StopDecision::End;
    }
    let instruction = answer
        .get("user_input")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(|text| truncate_preserving_layout(text, MAX_INSTRUCTION_CHARS));
    let files: Vec<String> = answer
        .get("files")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_string)
        .collect();
    // A todo picked from the live queue (`todo_queue`, docs/specs/todo-list-actions.md D10) comes as
    // its text in `user_input` plus its id, with no snapshot index. It is a todo like a chip, not a
    // typed instruction: the text goes in full (spec D11, no `MAX_INSTRUCTION_CHARS` cut) and the
    // todo's attachments (already in `files`) go with it.
    let picked_from_queue = answer
        .get("todo_ids")
        .and_then(Value::as_array)
        .is_some_and(|ids| !ids.is_empty());
    if picked_from_queue {
        let prompt = answer
            .get("user_input")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|text| !text.is_empty());
        if let Some(prompt) = prompt {
            return StopDecision::Continue(Some(
                crate::integrations::agent_launch::task_with_attachments(prompt, &files, &[]),
            ));
        }
    }
    if let Some(todo) = indices
        .iter()
        .filter_map(Value::as_u64)
        .find(|index| *index < continue_index)
        .and_then(|index| todos.get(index as usize))
    {
        // Todo text is always delivered in full (spec D11); the typed supplement follows after a
        // blank line, same as whats-next.
        let prompt = match &instruction {
            Some(extra) => format!("{}\n\n{}", todo.text, extra),
            None => todo.text.clone(),
        };
        return StopDecision::Continue(Some(
            crate::integrations::agent_launch::task_with_attachments(&prompt, &files, &[]),
        ));
    }
    if indices
        .iter()
        .any(|index| index.as_u64() == Some(continue_index))
        || instruction.is_some()
    {
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
        // The host runs another invocation with this as the reason it was not allowed to stop.
        AgentKind::Antigravity => json!({ "decision": "continue", "reason": prompt }),
        AgentKind::Pi => json!({ "followup_message": prompt }),
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
        assert!(is_natural_stop(
            AgentKind::Pi,
            &json!({"hook_event_name":"Stop","stop_reason":"stop"})
        ));
        assert!(!is_natural_stop(
            AgentKind::Pi,
            &json!({"hook_event_name":"Stop","stop_reason":"error"})
        ));
    }

    #[test]
    fn an_antigravity_stop_is_natural_only_when_the_model_chose_to_end() {
        let stop = |idle: bool, reason: &str| json!({"conversationId": "c", "fullyIdle": idle, "terminationReason": reason});
        assert!(is_natural_stop(
            AgentKind::Antigravity,
            &stop(true, "NO_TOOL_CALL")
        ));
        // Whether anything is still running is the daemon's call at the gate, not this one's: a
        // Stop that left a sub-agent running is confirmed once the sub-agent is done.
        assert!(is_natural_stop(
            AgentKind::Antigravity,
            &stop(false, "NO_TOOL_CALL")
        ));
        // A limit, an error and a cancel are not the agent finishing.
        for reason in [
            "ERROR",
            "MAX_INVOCATIONS",
            "MAX_FORCED_INVOCATIONS",
            "MAX_TOKEN_BUDGET_EXCEEDED",
            "USER_CANCELED",
            "",
        ] {
            assert!(
                !is_natural_stop(AgentKind::Antigravity, &stop(true, reason)),
                "{reason:?}"
            );
        }
        assert!(!is_natural_stop(AgentKind::Antigravity, &json!({})));
    }

    #[test]
    fn an_antigravity_continuation_names_why_the_agent_may_not_stop() {
        let output = continuation_output(AgentKind::Antigravity, "keep going");
        assert_eq!(
            output,
            json!({"decision": "continue", "reason": "keep going"})
        );
    }

    #[test]
    fn the_antigravity_card_shows_the_last_finished_reply_of_the_logged_conversation() {
        let home = tempfile::tempdir().unwrap();
        let id = "33333333-3333-4333-8333-333333333333";
        let dir = home
            .path()
            .join(".gemini/antigravity/brain")
            .join(id)
            .join(".system_generated/logs");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("transcript_full.jsonl"),
            include_str!("fixtures/antigravity-transcript.jsonl"),
        )
        .unwrap();
        let input = json!({"conversationId": id});
        let lookup = |id: &str| super::super::title::antigravity_transcript_in(home.path(), id);
        let message = antigravity_last_message_with(&input, &lookup);
        assert_eq!(
            message.as_deref(),
            Some("Done: the parser now keeps trailing whitespace and a regression test covers it.")
        );
        // The log is found by the id, never by a path the payload names.
        let pointed = json!({"conversationId": id, "transcriptPath": "/etc/passwd"});
        assert_eq!(
            antigravity_last_message_with(&pointed, &lookup),
            antigravity_last_message_with(&input, &lookup)
        );
        // A conversation without a log, or a payload without an id, has no message to show.
        let other = json!({"conversationId": "44444444-4444-4444-8444-444444444444"});
        assert_eq!(antigravity_last_message_with(&other, &lookup), None);
        assert_eq!(antigravity_last_message_with(&json!({}), &lookup), None);
        // The agent's own marker is read from that text like for any other agent.
        let confirmed = normalize_last_assistant_message(Some(&format!(
            "All done.\n{}",
            crate::prompts::USER_CONFIRMED_END_TURN_MARKER
        )));
        assert!(confirmed.user_confirmed_end_turn);
    }

    fn never_delegated(_: &str, _: Option<&Value>) -> Option<String> {
        None
    }

    #[test]
    fn turn_failed_matrix() {
        for kind in [AgentKind::Claude, AgentKind::Grok] {
            assert!(turn_failed(kind, &json!({"hook_event_name":"StopFailure"})));
            assert!(turn_failed(kind, &json!({"hookEventName":"stopfailure"})));
            assert!(!turn_failed(kind, &json!({"hook_event_name":"Stop"})));
            assert!(!turn_failed(kind, &json!({})));
        }
        assert!(turn_failed(AgentKind::Cursor, &json!({"status":"error"})));
        assert!(!turn_failed(
            AgentKind::Cursor,
            &json!({"status":"aborted"})
        ));
        assert!(!turn_failed(
            AgentKind::Cursor,
            &json!({"status":"completed"})
        ));
        assert!(turn_failed(AgentKind::Pi, &json!({"stop_reason":"error"})));
        assert!(!turn_failed(
            AgentKind::Pi,
            &json!({"stop_reason":"aborted"})
        ));
        assert!(!turn_failed(AgentKind::Pi, &json!({"stop_reason":"stop"})));
        assert!(!turn_failed(AgentKind::Pi, &json!({})));
        // Codex cannot tell: whatever the payload says, it is never a failure.
        assert!(!turn_failed(
            AgentKind::Codex,
            &json!({"hook_event_name":"StopFailure","status":"error","stop_reason":"error"})
        ));
    }

    #[test]
    fn codex_ephemeral_stops_suppress_confirmation_even_when_delegated_runs_are_allowed() {
        // `never_delegated` models the "allow delegated runs" preference being on.
        for input in [
            json!({"transcript_path":null}),
            json!({"transcriptPath":null}),
        ] {
            assert_eq!(
                confirmation_suppression_reason_with(
                    AgentKind::Codex,
                    "session",
                    &input,
                    &never_delegated
                )
                .as_deref(),
                Some("codex_transcript_path_null"),
                "{input:?}"
            );
        }
    }

    #[test]
    fn codex_delegated_stops_suppress_confirmation_with_the_delegated_reason() {
        let delegated = |session: &str, input: Option<&Value>| {
            assert_eq!(session, "session");
            assert!(input.is_some());
            Some("codex_delegated_exec_process".to_string())
        };
        assert_eq!(
            confirmation_suppression_reason_with(
                AgentKind::Codex,
                "session",
                &json!({"transcript_path":"/tmp/rollout.jsonl"}),
                &delegated
            )
            .as_deref(),
            Some("codex_delegated_exec_process")
        );
        // The ephemeral reason keeps priority, so existing audit queries still match.
        assert_eq!(
            confirmation_suppression_reason_with(
                AgentKind::Codex,
                "session",
                &json!({"transcript_path":null}),
                &delegated
            )
            .as_deref(),
            Some("codex_transcript_path_null")
        );
    }

    #[test]
    fn a_muted_session_gets_no_stop_confirmation_whatever_the_agent() {
        let muted = |session: &str| session == "quiet";
        for kind in [
            AgentKind::Claude,
            AgentKind::Codex,
            AgentKind::Cursor,
            AgentKind::Grok,
            AgentKind::Pi,
        ] {
            assert_eq!(
                suppression_reason_with_mute(
                    kind,
                    "quiet",
                    &json!({"transcript_path":"/tmp/rollout.jsonl"}),
                    &muted,
                    &never_delegated
                )
                .as_deref(),
                Some("session_muted"),
                "{kind:?}"
            );
            // Another session of the same agent is untouched.
            assert_eq!(
                suppression_reason_with_mute(
                    kind,
                    "other",
                    &json!({"transcript_path":"/tmp/rollout.jsonl"}),
                    &muted,
                    &never_delegated
                ),
                None,
                "{kind:?}"
            );
        }
        // Mute takes the audit reason over the delegated-run reasons.
        assert_eq!(
            suppression_reason_with_mute(
                AgentKind::Codex,
                "quiet",
                &json!({"transcript_path":null}),
                &muted,
                &never_delegated
            )
            .as_deref(),
            Some("session_muted")
        );
    }

    #[test]
    fn ordinary_and_non_codex_stops_keep_confirmation() {
        // Codex Stop input has no `thread_source`; a stray one must not change the decision.
        for input in [
            json!({"thread_source":"user", "transcript_path":"/tmp/rollout.jsonl"}),
            json!({"thread_source":"system", "transcript_path":"/tmp/rollout.jsonl"}),
            json!({"transcript_path":"/tmp/rollout.jsonl"}),
            json!({}),
        ] {
            assert_eq!(
                confirmation_suppression_reason_with(
                    AgentKind::Codex,
                    "session",
                    &input,
                    &never_delegated
                ),
                None
            );
        }
        let always_delegated = |_: &str, _: Option<&Value>| Some("delegated".to_string());
        for kind in [
            AgentKind::Claude,
            AgentKind::Cursor,
            AgentKind::Grok,
            AgentKind::Pi,
        ] {
            assert_eq!(
                confirmation_suppression_reason_with(
                    kind,
                    "session",
                    &json!({"transcript_path":null}),
                    &always_delegated
                ),
                None
            );
        }
    }

    #[test]
    fn maps_ask_results_fail_open() {
        assert_eq!(parse_ask_decision("not json", &[]), StopDecision::End);
        assert_eq!(parse_ask_decision("{}", &[]), StopDecision::End);
        assert_eq!(
            parse_ask_decision(r#"{"action":"cancel"}"#, &[]),
            StopDecision::End
        );
        assert_eq!(
            parse_ask_decision(
                r#"{"action":"answer","answers":[{"selected_indices":[1],"user_input":"ignored"}]}"#,
                &[]
            ),
            StopDecision::End
        );
        assert_eq!(
            parse_ask_decision(
                r#"{"action":"answer","answers":[{"selected_indices":[0]}]}"#,
                &[]
            ),
            StopDecision::Continue(None)
        );
        assert_eq!(
            parse_ask_decision(
                r#"{"action":"answer","answers":[{"user_input":"  next step  "}]}"#,
                &[]
            ),
            StopDecision::Continue(Some("next step".into()))
        );
        assert_eq!(
            parse_ask_decision(
                r#"{"action":"answer","answers":[{"selected_indices":[],"user_input":""}]}"#,
                &[]
            ),
            StopDecision::End
        );
    }

    fn two_todos() -> Vec<crate::todos::TodoEntry> {
        vec![
            crate::todos::TodoEntry {
                id: "id-1".into(),
                text: "修复登录 bug".into(),
                created_at_ms: 1,
                agent_kind: None,
                auto: false,
                attachments: Vec::new(),
            },
            crate::todos::TodoEntry {
                id: "id-2".into(),
                text: "写发布说明".into(),
                created_at_ms: 2,
                agent_kind: None,
                auto: false,
                attachments: Vec::new(),
            },
        ]
    }

    #[test]
    fn stop_card_todo_mapping_follows_spec_d5() {
        let todos = two_todos();
        // 选待办 → 以该条为 continuation。
        assert_eq!(
            parse_ask_decision(
                r#"{"action":"answer","answers":[{"selected_indices":[0]}]}"#,
                &todos
            ),
            StopDecision::Continue(Some("修复登录 bug".into()))
        );
        // 选待办 + 补充文字 → 按空行拼接。
        assert_eq!(
            parse_ask_decision(
                r#"{"action":"answer","answers":[{"selected_indices":[1],"user_input":" 顺带更新截图 "}]}"#,
                &todos
            ),
            StopDecision::Continue(Some("写发布说明\n\n顺带更新截图".into()))
        );
        // 「继续对话」（索引后移到 todos.len()）→ 原有语义。
        assert_eq!(
            parse_ask_decision(
                r#"{"action":"answer","answers":[{"selected_indices":[2]}]}"#,
                &todos
            ),
            StopDecision::Continue(None)
        );
        // 「结束对话」（末位）→ 结束并丢弃文字。
        assert_eq!(
            parse_ask_decision(
                r#"{"action":"answer","answers":[{"selected_indices":[3],"user_input":"ignored"}]}"#,
                &todos
            ),
            StopDecision::End
        );
        // 只填文字 → 继续（文字为指令），不涉及待办。
        assert_eq!(
            parse_ask_decision(
                r#"{"action":"answer","answers":[{"user_input":"先跑测试"}]}"#,
                &todos
            ),
            StopDecision::Continue(Some("先跑测试".into()))
        );
    }

    #[test]
    fn a_todo_picked_from_the_live_queue_is_delivered_like_a_chip() {
        // The live queue sends the todo's text in `user_input` and its id in `todo_ids`, with no
        // snapshot index; a todo added after the card was made is picked this way too.
        let todos = two_todos();
        let live = |text: &str, files: &str| {
            format!(
                r#"{{"action":"answer","answers":[{{"user_input":{},"files":{files},"todo_ids":["id-9"]}}]}}"#,
                serde_json::to_string(text).unwrap()
            )
        };
        assert_eq!(
            parse_ask_decision(&live("新加的待办", "[]"), &todos),
            StopDecision::Continue(Some("新加的待办".into()))
        );
        // Todo text goes in full (spec D11): the cut that keeps a typed instruction short does not apply.
        let long = "长".repeat(MAX_INSTRUCTION_CHARS + 500);
        assert_eq!(
            parse_ask_decision(&live(&long, "[]"), &todos),
            StopDecision::Continue(Some(long.clone()))
        );
        // The same cut still applies to what was only typed.
        let typed = format!(
            r#"{{"action":"answer","answers":[{{"user_input":{}}}]}}"#,
            serde_json::to_string(&long).unwrap()
        );
        match parse_ask_decision(&typed, &todos) {
            StopDecision::Continue(Some(text)) => assert!(text.ends_with("… [truncated]")),
            other => panic!("unexpected decision: {other:?}"),
        }
        // The todo's attachments travel with it, the way a chip's do.
        let file =
            std::env::temp_dir().join(format!("askhuman-stop-live-{}.txt", std::process::id()));
        std::fs::write(&file, "attachment").unwrap();
        let path = file.to_string_lossy().to_string();
        let with_file = live(
            "看一下附件",
            &serde_json::to_string(std::slice::from_ref(&path)).unwrap(),
        );
        let decision = parse_ask_decision(&with_file, &todos);
        let _ = std::fs::remove_file(&file);
        match decision {
            StopDecision::Continue(Some(text)) => {
                assert!(text.starts_with("看一下附件\n\nAttachments (local file paths):"));
                assert!(text.contains(&serde_json::to_string(&path).unwrap()));
            }
            other => panic!("unexpected decision: {other:?}"),
        }
        // Choosing "End conversation" still wins over anything else on the card.
        assert_eq!(
            parse_ask_decision(
                r#"{"action":"answer","answers":[{"selected_indices":[3],"user_input":"x","todo_ids":["id-9"]}]}"#,
                &todos
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
        let pi = continuation_output(AgentKind::Pi, "continue");
        assert_eq!(pi["followup_message"], "continue");
    }

    #[test]
    fn heartbeat_stop_is_quiet_without_claiming_user_approval() {
        let raw = super::super::codex_heartbeat::QUIET;
        let input = json!({"last_assistant_message": raw});
        let quiet = last_assistant_message(AgentKind::Codex, &input);
        assert!(quiet.quiet);
        assert!(!quiet.user_confirmed_end_turn);
        assert!(!quiet.display.unwrap().contains("<heartbeat>"));
        assert!(!last_assistant_message(AgentKind::Claude, &input).quiet);
        let notify = last_assistant_message(
            AgentKind::Codex,
            &json!({"last_assistant_message": raw.replace("DONT_NOTIFY", "NOTIFY")}),
        );
        assert!(!notify.quiet);
        assert!(!notify.display.unwrap().contains("<heartbeat>"));
        assert!(
            !last_assistant_message(
                AgentKind::Codex,
                &json!({"last_assistant_message": "Next user task is complete."})
            )
            .quiet
        );
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
                user_confirmed_end_turn: false,
                quiet: false,
            }
        );
    }

    #[test]
    fn confirmed_end_turn_marker_matches_any_substring_occurrence_and_is_stripped() {
        let marker = crate::prompts::USER_CONFIRMED_END_TURN_MARKER;
        for kind in [AgentKind::Claude, AgentKind::Codex, AgentKind::Pi] {
            let confirmed = last_assistant_message(
                kind,
                &json!({"last_assistant_message": format!("final report\n{marker}\n")}),
            );
            assert!(confirmed.user_confirmed_end_turn);
            assert_eq!(confirmed.display.as_deref(), Some("final report"));
        }

        // Anywhere in the body counts — including markdown glue and text after the marker.
        for text in [
            format!("done\n  {marker}  \n"),
            format!("prefix {marker}"),
            format!("quoted `{marker}`"),
            format!("**{marker}**"),
            format!("{marker}-suffix"),
            format!("结束。\n\n{marker}\n\n后面还有字"),
        ] {
            let message =
                last_assistant_message(AgentKind::Codex, &json!({"last_assistant_message": text}));
            assert!(message.user_confirmed_end_turn, "should match: {text:?}");
            assert!(
                !message.display.as_deref().unwrap_or("").contains(marker),
                "marker should be stripped from display: {text:?}"
            );
        }

        // Near-misses without the exact substring must not count.
        for text in [
            "user_confirmed_end_turn".to_string(),
            "[user_confirmed_end]".to_string(),
            "no marker here".to_string(),
        ] {
            let message =
                last_assistant_message(AgentKind::Codex, &json!({"last_assistant_message": text}));
            assert!(
                !message.user_confirmed_end_turn,
                "should not match: {text:?}"
            );
        }

        let long = format!("{}\n{marker}", "你".repeat(2_001));
        let message =
            last_assistant_message(AgentKind::Claude, &json!({"last_assistant_message": long}));
        assert!(message.user_confirmed_end_turn);
        assert_eq!(message.display.unwrap().matches('你').count(), 2_000);
    }

    #[test]
    fn internal_task_records_history_without_losing_stop_identity() {
        let task = build_task(
            AgentKind::Codex,
            "s1",
            "/tmp/p",
            &[],
            0,
            Some("done"),
            Lang::En,
        );
        assert!(task.single);
        assert!(!task.select_only);
        assert!(task.record_history);
        assert_eq!(task.popup_kind(), crate::ipc::PopupRequestKind::Stop);
        assert_eq!(task.agent_kind.as_deref(), Some("codex"));
        assert_eq!(task.agent_session_id.as_deref(), Some("s1"));
        assert_eq!(task.output_format, OutputFormat::Json);
        assert_eq!(task.questions[0].predefined_options.len(), 2);
        assert_eq!(task.message.text, "done");
        assert_eq!(task.project, "/tmp/p");
        assert!(task.questions[0].predefined_options[0].recommended);
        assert!(!task.questions[0].predefined_options[1].recommended);
    }

    #[test]
    fn stop_card_prepends_todo_chips_with_ids() {
        // 待办 chip 前置（带 id，供 Coordinator 出队）+ 原「继续/结束」压后（spec D5）。
        let task = build_task(
            AgentKind::Codex,
            "s1",
            "/tmp/p",
            &two_todos(),
            2,
            Some("done"),
            Lang::En,
        );
        let options = &task.questions[0].predefined_options;
        assert_eq!(options.len(), 4);
        // 未超上限 → 问题正文无溢出提示。
        assert!(!task.questions[0].message.contains("not listed"));
        // 展示前缀「Run todo: 」（与 whats-next 一致）；continuation 仍按索引取待办原文。
        assert_eq!(options[0].text, "Run todo: 修复登录 bug");
        assert_eq!(options[0].todo_id.as_deref(), Some("id-1"));
        assert!(!options[0].recommended);
        assert_eq!(options[1].todo_id.as_deref(), Some("id-2"));
        assert_eq!(options[2].text, "Continue conversation");
        assert!(options[2].recommended);
        assert!(options[2].todo_id.is_none());
        assert_eq!(options[3].text, "End conversation");
    }

    #[test]
    fn stop_card_appends_overflow_note_above_cap() {
        // 队列超上限（第 14 轮定案）：run_inner 截断后传入截断列表 + 原始总数；
        // 问题正文带溢出提示，选项数 = 上限 + 2。
        let todos: Vec<crate::todos::TodoEntry> = (0..crate::todos::MAX_OPTION_TODOS)
            .map(|i| crate::todos::TodoEntry {
                id: format!("id-{i}"),
                text: format!("task {i}"),
                created_at_ms: i as u64,
                agent_kind: None,
                auto: false,
                attachments: Vec::new(),
            })
            .collect();
        let task = build_task(
            AgentKind::Codex,
            "s1",
            "/tmp/p",
            &todos,
            crate::todos::MAX_OPTION_TODOS + 4,
            Some("done"),
            Lang::En,
        );
        let options = &task.questions[0].predefined_options;
        assert_eq!(options.len(), crate::todos::MAX_OPTION_TODOS + 2);
        assert!(
            task.questions[0].message.contains("4 more"),
            "{}",
            task.questions[0].message
        );
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

    #[test]
    fn antigravity_last_message_reads_validated_acp_transcript_path() {
        let dir = tempfile::tempdir().unwrap();
        let uuid = "4e200329-9002-48ef-9a74-faf0704f25d4";
        let transcript = dir
            .path()
            .join("providers/antigravity/hash/antigravity-acp/brain")
            .join(uuid)
            .join(".system_generated/logs/transcript_full.jsonl");
        std::fs::create_dir_all(transcript.parent().unwrap()).unwrap();
        let step = json!({
            "type": "PLANNER_RESPONSE",
            "status": "DONE",
            "content": "ACP final reply"
        });
        std::fs::write(&transcript, format!("{step}\n")).unwrap();

        let text = antigravity_last_message(&json!({
            "conversationId": uuid,
            "transcriptPath": transcript,
        }));
        assert_eq!(text.as_deref(), Some("ACP final reply"));

        // Mismatched conversation id cannot read another conversation's log via transcriptPath.
        let rejected = antigravity_last_message(&json!({
            "conversationId": "11111111-1111-4111-8111-111111111111",
            "transcriptPath": transcript,
        }));
        assert!(rejected.is_none());
    }
}
