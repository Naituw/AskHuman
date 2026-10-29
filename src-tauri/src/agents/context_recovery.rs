//! Runtime for the integration-mode-owned context recovery hook.
//!
//! For Codex the SessionStart hook is also the carrier of the interaction protocol (spec
//! `docs/specs/codex-delegated-run-silence.md` D8): it classifies the run once, and injects the
//! protocol only into runs a human attends.

use super::delegated::{self, Probes};
use super::AgentKind;
use crate::integrations::agent_mode::Mode;
use serde_json::Value;
use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use std::time::{Duration, Instant};

pub fn run(args: &[String]) {
    let Some(kind) = args.first().and_then(|value| AgentKind::parse(value)) else {
        return;
    };
    if kind == AgentKind::Antigravity {
        return;
    }
    let Some(event) = args.get(1).map(String::as_str) else {
        return;
    };
    let env = super::detect::env_snapshot();
    if super::report::should_skip(kind, &env) {
        return;
    }
    let input = super::report::read_stdin_json();
    if let Some(input) = &input {
        super::session_info::capture_hook(kind, input, &env);
    }
    let output = match event {
        "session-start" => session_start_output(kind, mode(kind), input.as_ref()),
        "pre-tool-use" => pre_tool_output(kind, input.as_ref(), &env),
        _ => None,
    };
    if let Some(output) = output {
        println!("{output}");
    }
}

fn mode(kind: AgentKind) -> Mode {
    crate::integrations::agent_mode::current(match kind {
        AgentKind::Claude => crate::integrations::agent_rules::AgentTarget::ClaudeCode,
        AgentKind::Codex => crate::integrations::agent_rules::AgentTarget::Codex,
        AgentKind::Cursor => crate::integrations::agent_rules::AgentTarget::Cursor,
        AgentKind::Grok => crate::integrations::agent_rules::AgentTarget::Grok,
        AgentKind::Pi => crate::integrations::agent_rules::AgentTarget::Pi,
        AgentKind::Antigravity => crate::integrations::agent_rules::AgentTarget::Antigravity,
    })
}

fn session_start_output(kind: AgentKind, mode: Mode, input: Option<&Value>) -> Option<String> {
    match kind {
        AgentKind::Claude => compact_recovery_output(mode, input?),
        AgentKind::Codex => codex_session_start(mode, input?, &CodexEnv::real()),
        _ => None,
    }
}

/// Claude: the SessionStart hook only matters after compaction, where it re-points the model at the
/// last AskHuman exchange.
fn compact_recovery_output(mode: Mode, input: &Value) -> Option<String> {
    if !is_compact_session_start(input) {
        return None;
    }
    let prompt = match mode {
        Mode::Cli => crate::prompts::compact_recovery_cli_prompt(),
        Mode::Mcp => crate::prompts::compact_recovery_mcp_prompt(),
        Mode::None => return None,
    };
    Some(context_output(prompt))
}

fn context_output(additional_context: &str) -> String {
    serde_json::json!({
        "hookSpecificOutput": {
            "hookEventName": "SessionStart",
            "additionalContext": additional_context,
        }
    })
    .to_string()
}

/// Everything the Codex SessionStart flow reads from the outside world, so the flow itself is
/// testable without a process table, a Codex home, or the user's configuration.
struct CodexEnv<'a> {
    probes: &'a dyn Probes,
    /// `~/.codex/AGENTS.md` still carries the legacy managed block (not migrated yet).
    legacy_block: &'a dyn Fn() -> bool,
    /// `(mcp, compact)` → the protocol text and its marker.
    protocol: &'a dyn Fn(bool, bool) -> crate::prompts::CodexProtocol,
    /// Whether the rollout at the path already carries the marker since its last compaction.
    injected_before: &'a dyn Fn(&Path, &str) -> bool,
    /// `(action, reason, session_id)` → `askhuman_guard` audit line.
    audit: &'a dyn Fn(&'static str, &str, &str),
}

impl CodexEnv<'static> {
    fn real() -> Self {
        Self {
            probes: &delegated::System,
            legacy_block: &crate::integrations::codex_protocol::has_legacy_block,
            protocol: &crate::prompts::codex_protocol,
            injected_before: &injected_since_last_compaction,
            audit: &audit_protocol_hook,
        }
    }
}

fn audit_protocol_hook(action: &'static str, reason: &str, session_id: &str) {
    crate::daemon::lifecycle::log_guard_audit(crate::daemon::lifecycle::GuardAudit {
        component: "protocol_hook",
        action,
        reason,
        tool: None,
        agent: Some("codex"),
        thread_source: None,
        session_id: (!session_id.is_empty()).then_some(session_id),
        thread_id: None,
        turn_id: None,
    });
}

/// Codex SessionStart: classify the run, then deliver the protocol.
///
/// * A delegated run (`codex exec` / `review`, ephemeral threads, recognized app-server clients)
///   gets a marker file and **no output**, so it behaves as if AskHuman were not integrated;
///   anything else clears a stale marker.
/// * While the legacy AGENTS.md block still delivers the protocol, only the post-compaction
///   sentence is emitted (the old behavior); migration removes the block.
/// * `resume` / `fork` replay history that may already contain this exact injection, so they skip
///   it when the rollout carries the marker; `startup`, `clear`, and `compact` always inject
///   (compaction drops the earlier injection).
fn codex_session_start(mode: Mode, input: &Value, env: &CodexEnv<'_>) -> Option<String> {
    if mode == Mode::None {
        return None;
    }
    let source = delegated::session_start_source(input);
    let session_id = delegated::session_id_of(input).unwrap_or_default();

    if !env.probes.allowed() {
        if let Some(signal) =
            delegated::classify_session_start(env.probes, &source, &session_id, input)
        {
            if !session_id.is_empty() {
                env.probes.write_marker(&session_id, signal);
            }
            (env.audit)("suppressed", signal.reason(), &session_id);
            return None;
        }
    }
    if !session_id.is_empty() {
        env.probes.clear_marker(&session_id);
    }

    let compact = source == "compact";
    let mcp = mode == Mode::Mcp;
    if (env.legacy_block)() {
        (env.audit)("passed", "codex_protocol_legacy_block", &session_id);
        if !compact {
            return None;
        }
        let sentence = if mcp {
            crate::prompts::compact_recovery_mcp_prompt()
        } else {
            crate::prompts::compact_recovery_cli_prompt()
        };
        return Some(context_output(sentence));
    }

    let protocol = (env.protocol)(mcp, compact);
    if matches!(source.as_str(), "resume" | "fork") {
        if let Some(path) = transcript_path(input) {
            if (env.injected_before)(Path::new(path), &protocol.needle) {
                (env.audit)("passed", "codex_protocol_already_injected", &session_id);
                return None;
            }
        }
    }
    (env.audit)("passed", "codex_protocol_injected", &session_id);
    Some(context_output(&protocol.text))
}

fn transcript_path(input: &Value) -> Option<&str> {
    input
        .get("transcript_path")
        .or_else(|| input.get("transcriptPath"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|path| !path.is_empty())
}

// ── resume / fork de-duplication ────────────────────────────────────────────────────────────

const SCAN_CHUNK: u64 = 64 * 1024;
/// At most this much of a rollout is read back looking for the marker.
const SCAN_MAX_BYTES: u64 = 64 * 1024 * 1024;
const SCAN_BUDGET: Duration = Duration::from_millis(300);

/// Whether the rollout already carries `needle` in a developer message since its last compaction.
/// Anything unexpected (unreadable file, budget exceeded) answers "no": a duplicate injection is
/// harmless, a missing one is not.
fn injected_since_last_compaction(path: &Path, needle: &str) -> bool {
    scan_for_injection(path, needle, SCAN_MAX_BYTES, SCAN_BUDGET).unwrap_or(false)
}

/// Walk the rollout backwards line by line. A line counts when it is a developer message holding
/// the marker; a `compacted` line ends the search (injections before it were dropped by the
/// compaction, and a marker quoted inside the compaction record itself is not proof).
/// `None` = the scan could not finish within its budget or failed.
fn scan_for_injection(path: &Path, needle: &str, max_bytes: u64, budget: Duration) -> Option<bool> {
    let started = Instant::now();
    let mut file = std::fs::File::open(path).ok()?;
    let length = file.metadata().ok()?.len();
    let mut position = length;
    let mut carry: Vec<u8> = Vec::new();
    let mut read_total = 0u64;

    let classify = |line: &[u8]| -> Option<bool> {
        if contains(line, br#""type":"compacted""#) {
            return Some(false);
        }
        (contains(line, br#""role":"developer""#) && contains(line, needle.as_bytes()))
            .then_some(true)
    };

    while position > 0 {
        if started.elapsed() > budget || read_total >= max_bytes {
            return None;
        }
        let size = SCAN_CHUNK.min(position);
        position -= size;
        file.seek(SeekFrom::Start(position)).ok()?;
        let mut chunk = vec![0u8; size as usize];
        file.read_exact(&mut chunk).ok()?;
        read_total += size;
        // The bytes before the chunk's first newline continue a line that started earlier.
        chunk.extend_from_slice(&carry);
        let mut end = chunk.len();
        let mut lines: Vec<&[u8]> = Vec::new();
        while let Some(newline) = chunk[..end].iter().rposition(|byte| *byte == b'\n') {
            lines.push(&chunk[newline + 1..end]);
            end = newline;
        }
        for line in lines {
            if line.is_empty() {
                continue;
            }
            if let Some(verdict) = classify(line) {
                return Some(verdict);
            }
        }
        carry = chunk[..end].to_vec();
    }
    // The first line of the file has no newline before it.
    if !carry.is_empty() {
        if let Some(verdict) = classify(&carry) {
            return Some(verdict);
        }
    }
    Some(false)
}

/// Substring search over one rollout line. Rollout lines are UTF-8 JSON, so the linear-time
/// `str` search applies; a line that is not valid UTF-8 (a torn write) falls back to a plain scan.
fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    if needle.is_empty() {
        return false;
    }
    match (std::str::from_utf8(haystack), std::str::from_utf8(needle)) {
        (Ok(haystack), Ok(needle)) => haystack.contains(needle),
        _ => haystack
            .windows(needle.len())
            .any(|window| window == needle),
    }
}

fn is_compact_session_start(input: &Value) -> bool {
    ["source", "session_start_source", "sessionStartSource"]
        .into_iter()
        .filter_map(|key| input.get(key).and_then(Value::as_str))
        .any(|source| source.eq_ignore_ascii_case("compact"))
}

fn pre_tool_output(
    kind: AgentKind,
    input: Option<&Value>,
    env: &HashMap<String, String>,
) -> Option<String> {
    pre_tool_output_for_mode(kind, mode(kind), input, env)
}

fn pre_tool_output_for_mode(
    kind: AgentKind,
    current_mode: Mode,
    input: Option<&Value>,
    env: &HashMap<String, String>,
) -> Option<String> {
    if current_mode != Mode::Mcp {
        return None;
    }
    let input = input?;
    if input
        .get("toolInputTruncated")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        return None;
    }
    let raw_name = super::report::tool_name(input)?;
    let tool_name = askhuman_tool_name(kind, &raw_name)?;
    let tool_input = super::report::tool_input(input)?;
    let arguments = arguments_for(kind, &raw_name, &tool_input)?;
    if !input_shape_matches(tool_name, arguments) {
        return None;
    }
    let session_id = session_id_from_hook(kind, input, env)?;

    match kind {
        AgentKind::Claude | AgentKind::Cursor => {
            let token =
                crate::context_binding::create_token(kind.as_str(), &session_id, tool_name)?;
            binding_output(kind, arguments, token)
        }
        AgentKind::Grok => {
            crate::context_binding::record_grok_pending(
                &session_id,
                &raw_name,
                tool_name,
                arguments,
                input,
                env,
            );
            None
        }
        // Pi recovery is injected by the managed extension's `before_agent_start` handler.
        AgentKind::Pi => None,
        AgentKind::Codex | AgentKind::Antigravity => None,
    }
}

fn binding_output(kind: AgentKind, arguments: &Value, token: String) -> Option<String> {
    let mut updated = arguments.as_object()?.clone();
    updated.insert(
        crate::context_binding::HIDDEN_TOKEN_FIELD.to_string(),
        Value::String(token),
    );
    Some(match kind {
        AgentKind::Claude => serde_json::json!({
            "hookSpecificOutput": {
                "hookEventName": "PreToolUse",
                "updatedInput": Value::Object(updated),
            }
        })
        .to_string(),
        AgentKind::Cursor => serde_json::json!({
            "updated_input": {
                (crate::context_binding::HIDDEN_TOKEN_FIELD): updated
                    .remove(crate::context_binding::HIDDEN_TOKEN_FIELD)
                    .expect("inserted token"),
            }
        })
        .to_string(),
        _ => return None,
    })
}

fn askhuman_tool_name(kind: AgentKind, raw: &str) -> Option<&'static str> {
    let prefix = match kind {
        AgentKind::Claude => "mcp__askhuman__",
        AgentKind::Cursor => "MCP:",
        AgentKind::Grok => "askhuman__",
        AgentKind::Codex | AgentKind::Pi | AgentKind::Antigravity => return None,
    };
    let name = raw.strip_prefix(prefix)?;
    match name {
        "ask" => Some("ask"),
        "whats_next" => Some("whats_next"),
        "show_last" => Some("show_last"),
        "todo_add" => Some("todo_add"),
        "todo_list" => Some("todo_list"),
        "todo_update" => Some("todo_update"),
        _ => None,
    }
}

fn arguments_for<'a>(kind: AgentKind, raw_name: &str, tool_input: &'a Value) -> Option<&'a Value> {
    if kind != AgentKind::Grok {
        return tool_input.is_object().then_some(tool_input);
    }
    // Grok Build wraps MCP input. Validate both copies of the qualified name before hashing.
    if let Some(wrapper_name) = tool_input.get("tool_name").and_then(Value::as_str) {
        if wrapper_name != raw_name {
            return None;
        }
        return tool_input
            .get("tool_input")
            .filter(|value| value.is_object());
    }
    // Composer/future builds may deliver direct arguments. Accept only an object shape.
    tool_input.is_object().then_some(tool_input)
}

fn input_shape_matches(tool_name: &str, input: &Value) -> bool {
    let Some(object) = input.as_object() else {
        return false;
    };
    let allowed: &[&str] = match tool_name {
        "ask" => &["message", "questions", "files"],
        "whats_next" => &["message", "options", "files"],
        "show_last" => &["count"],
        "todo_add" => &["text", "auto", "files"],
        "todo_list" => &[],
        "todo_update" => &["todoId", "addFiles", "removeAttachmentIds"],
        _ => return false,
    };
    if object.keys().any(|key| !allowed.contains(&key.as_str())) {
        return false;
    }
    match tool_name {
        "ask" => {
            object
                .get("message")
                .and_then(Value::as_str)
                .is_some_and(|value| !value.trim().is_empty())
                || object
                    .get("questions")
                    .and_then(Value::as_array)
                    .is_some_and(|questions| !questions.is_empty())
        }
        "show_last" => match object.get("count") {
            None => true,
            Some(Value::Number(n)) => n.as_u64().is_some_and(|v| (1..=10).contains(&v)),
            _ => false,
        },
        "whats_next" | "todo_list" => true,
        "todo_add" => {
            serde_json::from_value::<crate::mcp::ask::TodoAddParams>(input.clone()).is_ok()
        }
        "todo_update" => {
            serde_json::from_value::<crate::mcp::ask::TodoUpdateParams>(input.clone()).is_ok()
        }
        _ => false,
    }
}

fn session_id_from_hook(
    kind: AgentKind,
    input: &Value,
    env: &HashMap<String, String>,
) -> Option<String> {
    for key in [
        "session_id",
        "sessionId",
        "conversation_id",
        "conversationId",
        "thread_id",
        "threadId",
    ] {
        if let Some(value) = input
            .get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            return Some(value.to_string());
        }
    }
    super::detect::session_id_from_env_map(kind, env)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn todo_tools_share_trusted_hook_binding_without_exposing_identity_arguments() {
        for (tool, input) in [
            ("todo_add", json!({"text":"later", "auto":true})),
            ("todo_list", json!({})),
            (
                "todo_update",
                json!({"todoId":"id", "addFiles":["relative.txt"]}),
            ),
        ] {
            assert_eq!(
                askhuman_tool_name(AgentKind::Claude, &format!("mcp__askhuman__{tool}")),
                Some(tool)
            );
            assert!(input_shape_matches(tool, &input));
            assert!(
                crate::context_binding::canonical_tool_arguments_sha256(tool, &input).is_some()
            );
        }
        assert!(!input_shape_matches(
            "todo_list",
            &json!({"session_id":"guessed"})
        ));
    }
    #[test]
    fn claude_compact_prompt_strictly_follows_selected_mode() {
        let input = json!({"source": "compact"});
        let cli = session_start_output(AgentKind::Claude, Mode::Cli, Some(&input)).unwrap();
        assert!(cli.contains("AskHuman --show-last"));
        assert!(!cli.contains("MCP `show_last`"));
        let mcp = session_start_output(AgentKind::Claude, Mode::Mcp, Some(&input)).unwrap();
        assert!(mcp.contains("MCP `show_last`"));
        assert!(!mcp.contains("AskHuman --show-last"));
        assert!(session_start_output(AgentKind::Claude, Mode::None, Some(&input)).is_none());
    }

    #[test]
    fn claude_compact_prompt_requires_compact_source_and_other_agents_stay_quiet() {
        for input in [
            json!({"source": "COMPACT"}),
            json!({"session_start_source": "compact"}),
            json!({"sessionStartSource": "Compact"}),
        ] {
            assert!(session_start_output(AgentKind::Claude, Mode::Cli, Some(&input)).is_some());
        }
        for input in [
            json!({}),
            json!({"source": "startup"}),
            json!({"source": 1}),
        ] {
            assert!(session_start_output(AgentKind::Claude, Mode::Cli, Some(&input)).is_none());
        }
        for kind in [AgentKind::Cursor, AgentKind::Grok, AgentKind::Pi] {
            assert!(
                session_start_output(kind, Mode::Mcp, Some(&json!({"source":"compact"}))).is_none(),
                "{kind:?}"
            );
        }
        assert!(session_start_output(AgentKind::Claude, Mode::Cli, None).is_none());
        // Codex needs its stdin input to know the source; without it nothing is emitted. (Codex
        // with input is covered through `CodexEnv` below, never against the real environment.)
        assert!(session_start_output(AgentKind::Codex, Mode::Cli, None).is_none());
    }

    #[test]
    fn exact_tool_names_and_input_guards() {
        assert_eq!(
            askhuman_tool_name(AgentKind::Claude, "mcp__askhuman__show_last"),
            Some("show_last")
        );
        assert_eq!(
            askhuman_tool_name(AgentKind::Cursor, "MCP:ask"),
            Some("ask")
        );
        assert!(askhuman_tool_name(AgentKind::Cursor, "MCP:other").is_none());
        assert!(input_shape_matches("show_last", &json!({})));
        assert!(input_shape_matches("show_last", &json!({"count": 3})));
        assert!(!input_shape_matches("show_last", &json!({"count": 0})));
        assert!(!input_shape_matches("show_last", &json!({"x": 1})));
        assert!(input_shape_matches("ask", &json!({"message": "hello"})));
        assert!(!input_shape_matches("ask", &json!({"message": ""})));
        assert!(input_shape_matches(
            "ask",
            &json!({"questions": [{"question": "Continue?"}]})
        ));
        assert!(input_shape_matches("whats_next", &json!({})));
        assert!(input_shape_matches(
            "whats_next",
            &json!({"message": "done", "options": [], "files": []})
        ));
        assert!(!input_shape_matches(
            "whats_next",
            &json!({"unexpected": true})
        ));
        assert!(!input_shape_matches("ask", &json!([])));

        for name in ["ask", "whats_next", "show_last"] {
            assert_eq!(
                askhuman_tool_name(AgentKind::Claude, &format!("mcp__askhuman__{name}")),
                Some(name)
            );
            assert_eq!(
                askhuman_tool_name(AgentKind::Cursor, &format!("MCP:{name}")),
                Some(name)
            );
            assert_eq!(
                askhuman_tool_name(AgentKind::Grok, &format!("askhuman__{name}")),
                Some(name)
            );
        }
        assert!(askhuman_tool_name(AgentKind::Codex, "mcp__askhuman__ask").is_none());
        assert!(askhuman_tool_name(AgentKind::Claude, "MCP:ask").is_none());
        assert!(askhuman_tool_name(AgentKind::Cursor, "mcp:ask").is_none());
    }

    #[test]
    fn grok_wrapper_requires_matching_qualified_name() {
        let good = json!({
            "tool_name": "askhuman__show_last",
            "tool_input": {}
        });
        assert_eq!(
            arguments_for(AgentKind::Grok, "askhuman__show_last", &good),
            Some(&json!({}))
        );
        assert!(arguments_for(AgentKind::Grok, "other__show_last", &good).is_none());
        assert_eq!(
            arguments_for(AgentKind::Grok, "askhuman__show_last", &json!({})),
            Some(&json!({}))
        );
        assert!(arguments_for(
            AgentKind::Grok,
            "askhuman__show_last",
            &json!({"tool_name":"askhuman__show_last"})
        )
        .is_none());
        assert_eq!(
            arguments_for(AgentKind::Claude, "mcp__askhuman__show_last", &json!({})),
            Some(&json!({}))
        );
        assert!(
            arguments_for(AgentKind::Cursor, "MCP:show_last", &json!("not an object")).is_none()
        );
    }

    #[test]
    fn claude_binding_never_emits_an_allow_decision() {
        let output = binding_output(AgentKind::Claude, &json!({}), "token".into()).unwrap();
        let value: Value = serde_json::from_str(&output).unwrap();
        assert!(value
            .pointer("/hookSpecificOutput/permissionDecision")
            .is_none());
        assert_eq!(
            value
                .pointer("/hookSpecificOutput/updatedInput/__askhuman_session_token_v1")
                .and_then(Value::as_str),
            Some("token")
        );
    }

    #[test]
    fn cursor_binding_emits_only_the_patch_token() {
        let output = binding_output(
            AgentKind::Cursor,
            &json!({"message": "preserved by Cursor"}),
            "token".into(),
        )
        .unwrap();
        let value: Value = serde_json::from_str(&output).unwrap();
        assert_eq!(
            value,
            json!({"updated_input": {"__askhuman_session_token_v1": "token"}})
        );
    }

    #[test]
    fn session_id_prefers_hook_input_then_falls_back_to_agent_env() {
        let env = HashMap::from([
            ("CLAUDE_CODE_SESSION_ID".into(), "env-claude".into()),
            ("CURSOR_CONVERSATION_ID".into(), "env-cursor".into()),
        ]);
        assert_eq!(
            session_id_from_hook(
                AgentKind::Claude,
                &json!({"sessionId": "  input-session  "}),
                &env
            )
            .as_deref(),
            Some("input-session")
        );
        assert_eq!(
            session_id_from_hook(AgentKind::Cursor, &json!({}), &env).as_deref(),
            Some("env-cursor")
        );
        assert!(session_id_from_hook(AgentKind::Grok, &json!({}), &env).is_none());
    }

    #[test]
    fn pre_tool_guards_run_before_any_token_side_effect() {
        let env = HashMap::new();
        let valid = json!({
            "tool_name": "mcp__askhuman__ask",
            "tool_input": {"message": "hello"},
            "session_id": "session"
        });
        assert!(
            pre_tool_output_for_mode(AgentKind::Claude, Mode::Cli, Some(&valid), &env).is_none()
        );
        let mut truncated = valid.clone();
        truncated["toolInputTruncated"] = json!(true);
        assert!(
            pre_tool_output_for_mode(AgentKind::Claude, Mode::Mcp, Some(&truncated), &env)
                .is_none()
        );
        let other = json!({
            "tool_name": "mcp__other__ask",
            "tool_input": {"message": "hello"},
            "session_id": "session"
        });
        assert!(
            pre_tool_output_for_mode(AgentKind::Claude, Mode::Mcp, Some(&other), &env).is_none()
        );
        assert!(pre_tool_output_for_mode(AgentKind::Claude, Mode::Mcp, None, &env).is_none());
    }
}

#[cfg(test)]
mod codex_flow_tests {
    use super::*;
    use crate::agents::delegated::Signal;
    use serde_json::json;
    use std::cell::RefCell;
    use std::path::PathBuf;

    #[derive(Default)]
    struct FakeProbes {
        allowed: bool,
        exec: bool,
        rollout: Option<Signal>,
        marker: Option<Signal>,
        written: RefCell<Vec<(String, Signal)>>,
        cleared: RefCell<Vec<String>>,
        rollout_paths: RefCell<Vec<String>>,
    }

    impl Probes for FakeProbes {
        fn allowed(&self) -> bool {
            self.allowed
        }
        fn marker(&self, _session_id: &str) -> Option<Signal> {
            self.marker
        }
        fn write_marker(&self, session_id: &str, signal: Signal) {
            self.written.borrow_mut().push((session_id.into(), signal));
        }
        fn clear_marker(&self, session_id: &str) {
            self.cleared.borrow_mut().push(session_id.into());
        }
        fn exec_process(&self) -> bool {
            self.exec
        }
        fn rollout_signal(&self, transcript_path: &str) -> Option<Signal> {
            self.rollout_paths.borrow_mut().push(transcript_path.into());
            self.rollout
        }
    }

    #[derive(Default)]
    struct Harness {
        probes: FakeProbes,
        legacy_block: bool,
        already_injected: bool,
        dedupe_checks: RefCell<Vec<(PathBuf, String)>>,
        audits: RefCell<Vec<(String, String, String)>>,
    }

    impl Harness {
        fn run(&self, mode: Mode, input: Value) -> Option<String> {
            let legacy = || self.legacy_block;
            let protocol = |mcp: bool, compact: bool| crate::prompts::CodexProtocol {
                needle: format!(
                    "askhuman-protocol:{}:deadbeef",
                    if mcp { "mcp" } else { "cli" }
                ),
                text: format!(
                    "PROTOCOL[{}{}]",
                    if mcp { "mcp" } else { "cli" },
                    if compact { "+compact" } else { "" }
                ),
            };
            let injected = |path: &Path, needle: &str| {
                self.dedupe_checks
                    .borrow_mut()
                    .push((path.to_path_buf(), needle.to_string()));
                self.already_injected
            };
            let audit = |action: &'static str, reason: &str, session: &str| {
                self.audits
                    .borrow_mut()
                    .push((action.into(), reason.into(), session.into()));
            };
            let env = CodexEnv {
                probes: &self.probes,
                legacy_block: &legacy,
                protocol: &protocol,
                injected_before: &injected,
                audit: &audit,
            };
            codex_session_start(mode, &input, &env)
        }

        fn context(&self, mode: Mode, input: Value) -> Option<String> {
            let output = self.run(mode, input)?;
            let value: Value = serde_json::from_str(&output).unwrap();
            assert_eq!(value["hookSpecificOutput"]["hookEventName"], "SessionStart");
            Some(
                value["hookSpecificOutput"]["additionalContext"]
                    .as_str()
                    .unwrap()
                    .to_string(),
            )
        }

        fn audit_reasons(&self) -> Vec<(String, String)> {
            self.audits
                .borrow()
                .iter()
                .map(|(action, reason, _)| (action.clone(), reason.clone()))
                .collect()
        }
    }

    fn input(source: &str) -> Value {
        json!({
            "session_id": "sess-1",
            "source": source,
            "transcript_path": "/tmp/rollout.jsonl",
            "cwd": "/work",
        })
    }

    #[test]
    fn integration_mode_none_emits_nothing_and_touches_nothing() {
        let harness = Harness::default();
        assert!(harness.run(Mode::None, input("startup")).is_none());
        assert!(harness.probes.written.borrow().is_empty());
        assert!(harness.probes.cleared.borrow().is_empty());
        assert!(harness.audits.borrow().is_empty());
    }

    #[test]
    fn interactive_sessions_receive_the_protocol_for_their_mode() {
        for source in ["startup", "clear"] {
            let harness = Harness::default();
            assert_eq!(
                harness.context(Mode::Cli, input(source)).as_deref(),
                Some("PROTOCOL[cli]"),
                "{source}"
            );
            assert_eq!(
                harness.context(Mode::Mcp, input(source)).as_deref(),
                Some("PROTOCOL[mcp]"),
                "{source}"
            );
        }
    }

    #[test]
    fn an_interactive_run_clears_a_stale_delegated_marker() {
        let harness = Harness {
            probes: FakeProbes {
                marker: Some(Signal::ExecProcess),
                ..FakeProbes::default()
            },
            ..Harness::default()
        };
        // A human resumed a thread an `exec` run once created: only current-run signals count.
        assert!(harness.context(Mode::Cli, input("resume")).is_some());
        assert_eq!(harness.probes.cleared.borrow().as_slice(), ["sess-1"]);
        assert!(harness.probes.written.borrow().is_empty());
        assert!(
            harness.probes.rollout_paths.borrow().is_empty(),
            "rollout signals are for new threads only"
        );
    }

    #[test]
    fn delegated_runs_get_a_marker_no_output_and_an_audit_line() {
        let cases = [
            (
                "ephemeral thread",
                json!({"session_id":"sess-1","source":"startup","transcript_path":null}),
                FakeProbes::default(),
                Signal::TranscriptNull,
            ),
            (
                "codex exec ancestor",
                input("startup"),
                FakeProbes {
                    exec: true,
                    ..FakeProbes::default()
                },
                Signal::ExecProcess,
            ),
            (
                "rollout says exec",
                input("startup"),
                FakeProbes {
                    rollout: Some(Signal::RolloutSource),
                    ..FakeProbes::default()
                },
                Signal::RolloutSource,
            ),
            (
                "rollout originator on /clear",
                input("clear"),
                FakeProbes {
                    rollout: Some(Signal::RolloutOriginator),
                    ..FakeProbes::default()
                },
                Signal::RolloutOriginator,
            ),
        ];
        for (name, input, probes, signal) in cases {
            let harness = Harness {
                probes,
                ..Harness::default()
            };
            assert!(harness.run(Mode::Mcp, input).is_none(), "{name}");
            assert_eq!(
                harness.probes.written.borrow().as_slice(),
                [("sess-1".to_string(), signal)],
                "{name}"
            );
            assert!(harness.probes.cleared.borrow().is_empty(), "{name}");
            assert_eq!(
                harness.audit_reasons(),
                [("suppressed".to_string(), signal.reason().to_string())],
                "{name}"
            );
            assert_eq!(harness.audits.borrow()[0].2, "sess-1", "{name}");
        }
    }

    #[test]
    fn the_allow_preference_makes_delegated_runs_behave_like_interactive_ones() {
        let harness = Harness {
            probes: FakeProbes {
                allowed: true,
                exec: true,
                ..FakeProbes::default()
            },
            ..Harness::default()
        };
        assert_eq!(
            harness.context(Mode::Cli, input("startup")).as_deref(),
            Some("PROTOCOL[cli]")
        );
        assert!(harness.probes.written.borrow().is_empty());
        assert_eq!(harness.probes.cleared.borrow().as_slice(), ["sess-1"]);
    }

    #[test]
    fn compaction_reinjects_the_whole_protocol_with_the_recovery_sentence() {
        let harness = Harness::default();
        assert_eq!(
            harness.context(Mode::Cli, input("compact")).as_deref(),
            Some("PROTOCOL[cli+compact]")
        );
        assert_eq!(
            harness.context(Mode::Mcp, input("compact")).as_deref(),
            Some("PROTOCOL[mcp+compact]")
        );
        // Compaction never consults the rollout for earlier injections: they are gone.
        assert!(harness.dedupe_checks.borrow().is_empty());
    }

    #[test]
    fn compaction_of_a_delegated_run_stays_silent_through_its_marker() {
        let harness = Harness {
            probes: FakeProbes {
                marker: Some(Signal::RolloutOriginator),
                ..FakeProbes::default()
            },
            ..Harness::default()
        };
        assert!(harness.run(Mode::Cli, input("compact")).is_none());
        assert_eq!(
            harness.probes.written.borrow().as_slice(),
            [("sess-1".to_string(), Signal::RolloutOriginator)]
        );
    }

    #[test]
    fn resume_and_fork_skip_an_injection_the_rollout_already_carries() {
        for source in ["resume", "fork"] {
            let harness = Harness {
                already_injected: true,
                ..Harness::default()
            };
            assert!(harness.run(Mode::Cli, input(source)).is_none(), "{source}");
            assert_eq!(
                harness.dedupe_checks.borrow().as_slice(),
                [(
                    PathBuf::from("/tmp/rollout.jsonl"),
                    "askhuman-protocol:cli:deadbeef".to_string()
                )],
                "{source}"
            );
            assert_eq!(
                harness.audit_reasons(),
                [(
                    "passed".to_string(),
                    "codex_protocol_already_injected".to_string()
                )]
            );
        }
    }

    #[test]
    fn resume_and_fork_inject_when_the_rollout_lacks_the_marker() {
        for source in ["resume", "fork"] {
            let harness = Harness::default();
            assert_eq!(
                harness.context(Mode::Mcp, input(source)).as_deref(),
                Some("PROTOCOL[mcp]"),
                "{source}"
            );
        }
    }

    #[test]
    fn startup_clear_and_compact_never_trust_the_rollout() {
        for source in ["startup", "clear", "compact"] {
            let harness = Harness {
                already_injected: true,
                ..Harness::default()
            };
            assert!(
                harness.context(Mode::Cli, input(source)).is_some(),
                "{source}"
            );
            assert!(harness.dedupe_checks.borrow().is_empty(), "{source}");
        }
    }

    #[test]
    fn resume_without_a_transcript_path_injects() {
        let harness = Harness {
            already_injected: true,
            ..Harness::default()
        };
        let no_path = json!({"session_id":"sess-1","source":"resume"});
        assert!(harness.context(Mode::Cli, no_path).is_some());
        assert!(harness.dedupe_checks.borrow().is_empty());
    }

    #[test]
    fn while_the_legacy_block_delivers_the_protocol_only_compaction_gets_a_sentence() {
        let harness = Harness {
            legacy_block: true,
            ..Harness::default()
        };
        for source in ["startup", "clear", "resume", "fork"] {
            assert!(harness.run(Mode::Cli, input(source)).is_none(), "{source}");
        }
        let cli = harness.context(Mode::Cli, input("compact")).unwrap();
        assert_eq!(cli, crate::prompts::compact_recovery_cli_prompt());
        let mcp = harness.context(Mode::Mcp, input("compact")).unwrap();
        assert_eq!(mcp, crate::prompts::compact_recovery_mcp_prompt());
        assert!(harness.dedupe_checks.borrow().is_empty());
    }

    #[test]
    fn the_legacy_block_does_not_stop_delegated_runs_from_being_classified() {
        let harness = Harness {
            legacy_block: true,
            probes: FakeProbes {
                exec: true,
                ..FakeProbes::default()
            },
            ..Harness::default()
        };
        assert!(harness.run(Mode::Cli, input("startup")).is_none());
        assert_eq!(
            harness.probes.written.borrow().as_slice(),
            [("sess-1".to_string(), Signal::ExecProcess)]
        );
        // A delegated compaction gets no recovery sentence either.
        assert!(harness.run(Mode::Cli, input("compact")).is_none());
    }

    #[test]
    fn a_missing_session_id_never_writes_or_clears_markers() {
        let harness = Harness {
            probes: FakeProbes {
                exec: true,
                ..FakeProbes::default()
            },
            ..Harness::default()
        };
        assert!(harness
            .run(
                Mode::Cli,
                json!({"source":"startup","transcript_path":"/x"})
            )
            .is_none());
        assert!(harness.probes.written.borrow().is_empty());
        let interactive = Harness::default();
        assert!(interactive
            .context(Mode::Cli, json!({"source":"startup"}))
            .is_some());
        assert!(interactive.probes.cleared.borrow().is_empty());
    }

    #[test]
    fn injection_is_audited_for_diagnosis() {
        let harness = Harness::default();
        harness.context(Mode::Cli, input("startup")).unwrap();
        assert_eq!(
            harness.audit_reasons(),
            [("passed".to_string(), "codex_protocol_injected".to_string())]
        );
        let legacy = Harness {
            legacy_block: true,
            ..Harness::default()
        };
        legacy.run(Mode::Cli, input("startup"));
        assert_eq!(
            legacy.audit_reasons(),
            [(
                "passed".to_string(),
                "codex_protocol_legacy_block".to_string()
            )]
        );
    }

    // ── rollout scan ──

    fn developer_line(text: &str) -> String {
        json!({"type":"response_item","payload":{"type":"message","role":"developer",
               "content":[{"type":"input_text","text":text}]}})
        .to_string()
    }

    fn user_line(text: &str) -> String {
        json!({"type":"response_item","payload":{"type":"message","role":"user",
               "content":[{"type":"input_text","text":text}]}})
        .to_string()
    }

    fn compacted_line(retained_text: &str) -> String {
        json!({"type":"compacted","payload":{"message":"","replacement_history":[
            {"type":"message","role":"developer","content":[{"type":"input_text","text":retained_text}]}
        ]}})
        .to_string()
    }

    const NEEDLE: &str = "askhuman-protocol:cli:0a1b2c3d";

    fn injected_text() -> String {
        format!("<!-- {NEEDLE} -->\n<mandatory_interaction_protocol>…")
    }

    fn rollout(lines: &[String]) -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rollout.jsonl");
        std::fs::write(&path, lines.join("\n") + "\n").unwrap();
        (dir, path)
    }

    /// The production scan with the time budget lifted: a debug build under a loaded test machine
    /// can be slow enough to trip the 300 ms production budget, which is tested separately.
    fn scan(path: &Path) -> bool {
        scan_for_injection(path, NEEDLE, SCAN_MAX_BYTES, Duration::from_secs(120)).unwrap_or(false)
    }

    #[test]
    fn the_production_entry_point_uses_the_same_scan() {
        let (_dir, path) = rollout(&[developer_line(&injected_text()), user_line("x")]);
        assert!(injected_since_last_compaction(&path, NEEDLE));
        let (_dir, path) = rollout(&[user_line("x")]);
        assert!(!injected_since_last_compaction(&path, NEEDLE));
    }

    #[test]
    fn scan_finds_the_marker_in_a_developer_message() {
        let (_dir, path) = rollout(&[
            json!({"type":"session_meta","payload":{}}).to_string(),
            developer_line(&injected_text()),
            user_line("hello"),
            user_line("more"),
        ]);
        assert!(scan(&path));
    }

    #[test]
    fn scan_ignores_a_quoted_marker_outside_developer_messages() {
        let (_dir, path) = rollout(&[
            user_line(&format!("please explain {NEEDLE}")),
            developer_line("unrelated developer text"),
        ]);
        assert!(!scan(&path));
    }

    #[test]
    fn scan_ignores_other_modes_and_other_hashes() {
        let (_dir, path) = rollout(&[developer_line(
            "<!-- askhuman-protocol:mcp:0a1b2c3d --> and <!-- askhuman-protocol:cli:ffffffff -->",
        )]);
        assert!(!scan(&path));
    }

    #[test]
    fn scan_stops_at_the_last_compaction() {
        // Marker, then a compaction: the injection was dropped, so inject again.
        let (_dir, path) = rollout(&[
            developer_line(&injected_text()),
            user_line("work"),
            compacted_line("summary"),
            user_line("after"),
        ]);
        assert!(!scan(&path));
        // Re-injected by SessionStart(compact) after the compaction: found.
        let (_dir, path) = rollout(&[
            developer_line(&injected_text()),
            compacted_line("summary"),
            developer_line(&injected_text()),
            user_line("after"),
        ]);
        assert!(scan(&path));
    }

    #[test]
    fn scan_does_not_count_a_marker_inside_the_compaction_record() {
        let (_dir, path) = rollout(&[
            user_line("before"),
            compacted_line(&injected_text()),
            user_line("after"),
        ]);
        assert!(!scan(&path));
    }

    #[test]
    fn scan_handles_files_without_a_trailing_newline_or_with_one_line() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("one.jsonl");
        std::fs::write(&path, developer_line(&injected_text())).unwrap();
        assert!(scan(&path));
        std::fs::write(&path, user_line("nothing here")).unwrap();
        assert!(!scan(&path));
        std::fs::write(&path, "").unwrap();
        assert!(!scan(&path));
    }

    #[test]
    fn scan_finds_the_marker_wherever_chunk_boundaries_fall() {
        // Vary the prefix so the marker line straddles different 64 KiB chunk edges.
        let filler = "x".repeat(997);
        for pad in (0..SCAN_CHUNK as usize + 1500).step_by(1237) {
            let mut lines = vec![user_line(&"p".repeat(pad))];
            lines.push(developer_line(&injected_text()));
            for _ in 0..200 {
                lines.push(user_line(&filler));
            }
            let (_dir, path) = rollout(&lines);
            assert!(
                std::fs::metadata(&path).unwrap().len() > 2 * SCAN_CHUNK,
                "the marker must sit several chunks from the end"
            );
            assert!(scan(&path), "pad {pad}");
        }
    }

    #[test]
    fn scan_never_misreads_a_compaction_split_across_chunks() {
        let filler = "y".repeat(997);
        for pad in (0..SCAN_CHUNK as usize).step_by(4099) {
            let mut lines = vec![developer_line(&injected_text())];
            lines.push(user_line(&"q".repeat(pad)));
            lines.push(compacted_line("summary"));
            for _ in 0..200 {
                lines.push(user_line(&filler));
            }
            let (_dir, path) = rollout(&lines);
            assert!(!scan(&path), "pad {pad}");
        }
    }

    #[test]
    fn scan_falls_back_to_injecting_when_it_cannot_finish_or_read() {
        let mut lines = vec![developer_line(&injected_text())];
        lines.extend((0..400).map(|_| user_line(&"z".repeat(997))));
        let (_dir, path) = rollout(&lines);
        // Byte budget exhausted before the marker is reached.
        assert_eq!(
            scan_for_injection(&path, NEEDLE, SCAN_CHUNK, Duration::from_secs(5)),
            None
        );
        // Time budget exhausted immediately.
        assert_eq!(
            scan_for_injection(&path, NEEDLE, SCAN_MAX_BYTES, Duration::ZERO),
            None
        );
        // A generous budget does find it.
        assert_eq!(
            scan_for_injection(&path, NEEDLE, SCAN_MAX_BYTES, Duration::from_secs(5)),
            Some(true)
        );
        assert!(!injected_since_last_compaction(
            Path::new("/definitely/not/a/rollout.jsonl"),
            NEEDLE
        ));
    }
}
