//! Tool detail summarization: convert raw tool_name + tool_input into
//! human-readable body and optional structured details.

use serde_json::Value;

pub struct Summary {
    pub body: String,
    pub details: Option<String>,
}

pub fn summarize(tool_name: &str, tool_input: &Value) -> Summary {
    let mut body = match tool_name.to_lowercase().as_str() {
        "bash" | "shell" | "execute" | "terminal" => summarize_bash(tool_input),
        "write" | "write_file" | "create" | "edit" | "str_replace_editor" | "apply_patch" => {
            summarize_file_op(tool_name, tool_input)
        }
        "mcp__" => summarize_mcp(tool_input),
        _ if tool_name.starts_with("mcp__") => summarize_mcp_named(tool_name, tool_input),
        _ => summarize_generic(tool_name, tool_input),
    };
    // Always provide full tool_input as details for review.
    if body.details.is_none() && !tool_input.is_null() {
        body.details = Some(pretty_json_truncated(tool_input, 8000));
    }
    body
}

fn summarize_bash(input: &Value) -> Summary {
    let command = input
        .get("command")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    let body = if command.is_empty() {
        "Execute a shell command".to_string()
    } else {
        format!("```\n{}\n```", truncate_str(command, 2048))
    };
    Summary {
        body,
        details: Some(pretty_json_truncated(input, 8000)),
    }
}

fn summarize_file_op(tool_name: &str, input: &Value) -> Summary {
    let path = input
        .get("path")
        .or_else(|| input.get("file_path"))
        .or_else(|| input.get("target"))
        .and_then(|v| v.as_str())
        .unwrap_or("(unknown path)");
    let body = format!("**{}** `{}`", tool_name, path);
    Summary {
        body,
        details: Some(pretty_json_truncated(input, 8000)),
    }
}

fn summarize_mcp(input: &Value) -> Summary {
    let server = input
        .get("server")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    let tool = input
        .get("tool")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    let body = format!("MCP call: **{}** / `{}`", server, tool);
    Summary {
        body,
        details: Some(pretty_json_truncated(input, 8000)),
    }
}

fn summarize_mcp_named(tool_name: &str, input: &Value) -> Summary {
    let parts: Vec<&str> = tool_name.splitn(3, "__").collect();
    let (server, tool) = if parts.len() >= 3 {
        (parts[1], parts[2])
    } else {
        ("unknown", tool_name)
    };
    let body = format!("MCP call: **{}** / `{}`", server, tool);
    Summary {
        body,
        details: Some(pretty_json_truncated(input, 8000)),
    }
}

fn summarize_generic(tool_name: &str, input: &Value) -> Summary {
    let body = format!("Tool: **{}**", tool_name);
    Summary {
        body,
        details: if tool_input_is_meaningful(input) {
            Some(pretty_json_truncated(input, 8000))
        } else {
            None
        },
    }
}

fn tool_input_is_meaningful(v: &Value) -> bool {
    !v.is_null() && v != &Value::Object(serde_json::Map::new())
}

fn pretty_json_truncated(v: &Value, max_chars: usize) -> String {
    let s = serde_json::to_string_pretty(v).unwrap_or_else(|_| v.to_string());
    truncate_str(&s, max_chars)
}

fn truncate_str(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let head: String = s.chars().take(max).collect();
        format!("{head}\n… (truncated)")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn bash_shows_command() {
        let s = summarize("Bash", &json!({"command": "ls -la"}));
        assert!(s.body.contains("ls -la"));
        assert!(s.details.is_some());
        assert!(s.details.unwrap().contains("ls -la"));
    }

    #[test]
    fn file_op_shows_path() {
        let s = summarize(
            "write_file",
            &json!({"path": "/tmp/foo.txt", "content": "hi"}),
        );
        assert!(s.body.contains("/tmp/foo.txt"));
        assert!(s.details.is_some());
    }

    #[test]
    fn mcp_prefixed_tool() {
        let s = summarize("mcp__myserver__do_thing", &json!({"arg": 1}));
        assert!(s.body.contains("myserver"));
        assert!(s.body.contains("do_thing"));
    }
}
