//! Parse PermissionRequest stdin from Claude Code / Codex.

use super::{AgentFamily, PermissionInput, FIELD_MAX_CHARS, STDIN_MAX_BYTES};
use serde_json::Value;

/// Read stdin and parse the PermissionRequest event.
/// Returns `None` on any parse failure (hook should exit silently with no output).
pub fn parse_stdin() -> Option<(AgentFamily, PermissionInput)> {
    let raw = read_stdin_limited()?;
    let v: Value = serde_json::from_str(&raw).ok()?;

    // Claude uses `hook_event_name` (snake_case) at top level.
    // Codex wraps in `event.event`.
    let event_name = v
        .pointer("/event/event")
        .or_else(|| v.get("hook_event_name"))
        .or_else(|| v.get("hookEventName"))
        .and_then(|v| v.as_str())?;

    if event_name != "PermissionRequest" {
        return None;
    }

    // Claude: top-level `tool_name` / `tool_input`.
    // Codex: nested `event.tool_name` / `event.tool_input`.
    let tool_name = v
        .pointer("/event/tool_name")
        .or_else(|| v.get("tool_name"))
        .or_else(|| v.get("toolName"))
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string();

    let tool_input = v
        .pointer("/event/tool_input")
        .or_else(|| v.get("tool_input"))
        .or_else(|| v.get("toolInput"))
        .cloned()
        .unwrap_or(Value::Null);

    let cwd = v
        .pointer("/event/cwd")
        .or_else(|| v.get("cwd"))
        .and_then(|v| v.as_str())
        .map(|s| truncate(s, FIELD_MAX_CHARS));

    let session_id = v
        .get("session_id")
        .or_else(|| v.get("sessionId"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    // Detect agent family from payload shape.
    let agent = if v.pointer("/event/event").is_some() {
        AgentFamily::Codex
    } else {
        AgentFamily::Claude
    };

    Some((
        agent,
        PermissionInput {
            agent,
            tool_name,
            tool_input,
            cwd,
            session_id,
        },
    ))
}

fn read_stdin_limited() -> Option<String> {
    use std::io::Read;
    let mut buf = String::new();
    let stdin = std::io::stdin();
    let mut handle = stdin.lock();
    match handle
        .by_ref()
        .take(STDIN_MAX_BYTES as u64)
        .read_to_string(&mut buf)
    {
        Ok(0) => None,
        Ok(_) => Some(buf),
        Err(_) => None,
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let head: String = s.chars().take(max).collect();
        format!("{head}…")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_claude_permission_request() {
        let input = r#"{
            "session_id": "sess-123",
            "cwd": "/home/user/project",
            "hook_event_name": "PermissionRequest",
            "tool_name": "Bash",
            "tool_input": {"command": "rm -rf /tmp/foo"}
        }"#;
        let parsed = parse_from_str(input).unwrap();
        assert_eq!(parsed.1.agent, AgentFamily::Claude);
        assert_eq!(parsed.1.tool_name, "Bash");
        assert_eq!(parsed.1.session_id.as_deref(), Some("sess-123"));
    }

    #[test]
    fn parse_claude_permission_request_camel_case_compat() {
        let input = r#"{
            "hookEventName": "PermissionRequest",
            "toolName": "Bash",
            "toolInput": {"command": "rm -rf /tmp/foo"},
            "cwd": "/home/user/project",
            "sessionId": "sess-456"
        }"#;
        let parsed = parse_from_str(input).unwrap();
        assert_eq!(parsed.1.agent, AgentFamily::Claude);
        assert_eq!(parsed.1.tool_name, "Bash");
        assert_eq!(parsed.1.session_id.as_deref(), Some("sess-456"));
    }

    #[test]
    fn parse_codex_permission_request() {
        let input = r#"{
            "event": {"event": "PermissionRequest", "tool_name": "write_file", "tool_input": {"path": "/tmp/x"}, "cwd": "/home"},
            "session_id": "s1"
        }"#;
        let parsed = parse_from_str(input).unwrap();
        assert_eq!(parsed.1.agent, AgentFamily::Codex);
        assert_eq!(parsed.1.tool_name, "write_file");
    }

    #[test]
    fn wrong_event_returns_none() {
        let input = r#"{"hook_event_name": "PreToolUse", "tool_name": "Bash"}"#;
        assert!(parse_from_str(input).is_none());
    }

    fn parse_from_str(s: &str) -> Option<(AgentFamily, PermissionInput)> {
        let v: Value = serde_json::from_str(s).ok()?;
        let event_name = v
            .pointer("/event/event")
            .or_else(|| v.get("hook_event_name").and_then(|x| x.as_str().map(|_| x)))
            .or_else(|| v.get("hookEventName"))
            .and_then(|v| v.as_str())?;
        if event_name != "PermissionRequest" {
            return None;
        }
        let tool_name = v
            .pointer("/event/tool_name")
            .or_else(|| v.get("tool_name"))
            .or_else(|| v.get("toolName"))
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string();
        let tool_input = v
            .pointer("/event/tool_input")
            .or_else(|| v.get("tool_input"))
            .or_else(|| v.get("toolInput"))
            .cloned()
            .unwrap_or(Value::Null);
        let cwd = v
            .pointer("/event/cwd")
            .or_else(|| v.get("cwd"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let session_id = v
            .get("session_id")
            .or_else(|| v.get("sessionId"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let agent = if v.pointer("/event/event").is_some() {
            AgentFamily::Codex
        } else {
            AgentFamily::Claude
        };
        Some((
            agent,
            PermissionInput {
                agent,
                tool_name,
                tool_input,
                cwd,
                session_id,
            },
        ))
    }
}
