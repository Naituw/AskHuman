//! Comment-preserving JSONC edits for nested command-hook groups.

use anyhow::{anyhow, Result};
use jsonc_parser::cst::{CstNode, CstRootNode};
use jsonc_parser::json;
use jsonc_parser::ParseOptions;
use serde_json::Value;

fn command_has_marker(value: &Value, marker: &str) -> bool {
    value
        .get("hooks")
        .and_then(Value::as_array)
        .map(|handlers| {
            handlers.iter().any(|handler| {
                handler
                    .get("command")
                    .and_then(Value::as_str)
                    .map(|command| command.contains(marker))
                    .unwrap_or(false)
            })
        })
        .unwrap_or(false)
}

fn handler_node_has_marker(node: &CstNode, marker: &str) -> bool {
    node.to_serde_value()
        .map(|value: Value| {
            value
                .get("command")
                .and_then(Value::as_str)
                .map(|command| command.contains(marker))
                .unwrap_or(false)
        })
        .unwrap_or(false)
}

pub fn upsert_nested_group(
    text: &str,
    event: &str,
    marker: &str,
    command: &str,
    timeout: u64,
    status_message: Option<&str>,
) -> Result<String> {
    let source = if text.trim().is_empty() { "{}" } else { text };
    let root = CstRootNode::parse(source, &ParseOptions::default())
        .map_err(|error| anyhow!("failed to parse hook config: {error}"))?;
    let root_object = root
        .object_value_or_create()
        .ok_or_else(|| anyhow!("hook config root is not an object"))?;
    let hooks = root_object
        .object_value_or_create("hooks")
        .ok_or_else(|| anyhow!("hook config 'hooks' is not an object"))?;
    let groups = hooks
        .array_value_or_create(event)
        .ok_or_else(|| anyhow!("hook event '{event}' is not an array"))?;
    let replacement_handler = match status_message {
        Some(message) => json!({
            "type": "command",
            "command": command,
            "timeout": timeout,
            "statusMessage": message
        }),
        None => json!({ "type": "command", "command": command, "timeout": timeout }),
    };
    let mut replaced = false;
    for group in groups.elements() {
        let Some(object) = group.as_object() else {
            continue;
        };
        let Some(handlers) = object.array_value("hooks") else {
            continue;
        };
        for handler in handlers.elements() {
            let has_marker = handler_node_has_marker(&handler, marker);
            if !has_marker {
                continue;
            }
            if !replaced {
                if let Some(handler_object) = handler.as_object() {
                    handler_object.replace_with(replacement_handler.clone());
                    replaced = true;
                    continue;
                }
            }
            handler.remove();
        }
    }
    if !replaced {
        groups.ensure_multiline();
        groups.append(json!({ "hooks": [replacement_handler] }));
    }
    Ok(root.to_string())
}

pub fn remove_nested_marker(text: &str, event: &str, marker: &str) -> Result<String> {
    let root = CstRootNode::parse(text, &ParseOptions::default())
        .map_err(|error| anyhow!("failed to parse hook config: {error}"))?;
    let Some(root_object) = root.object_value() else {
        return Ok(root.to_string());
    };
    let Some(hooks) = root_object.object_value("hooks") else {
        return Ok(root.to_string());
    };
    if let Some(groups) = hooks.array_value(event) {
        for group in groups.elements() {
            let Some(object) = group.as_object() else {
                continue;
            };
            let Some(handlers) = object.array_value("hooks") else {
                continue;
            };
            for handler in handlers.elements() {
                let has_marker = handler_node_has_marker(&handler, marker);
                if has_marker {
                    handler.remove();
                }
            }
            if handlers.elements().is_empty() {
                group.remove();
            }
        }
        if groups.elements().is_empty() {
            if let Some(property) = hooks.get(event) {
                property.remove();
            }
        }
    }
    Ok(root.to_string())
}

pub fn nested_groups(text: &str, event: &str) -> Result<Vec<Value>> {
    let value = jsonc_parser::parse_to_serde_value::<Value>(text, &ParseOptions::default())
        .map_err(|error| anyhow!("failed to parse hook config: {error}"))?;
    Ok(value
        .get("hooks")
        .and_then(|hooks| hooks.get(event))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default())
}

pub fn group_has_marker(group: &Value, marker: &str) -> bool {
    command_has_marker(group, marker)
}

pub fn atomic_write(path: &std::path::Path, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let temporary = path.with_extension(format!("tmp-{}", uuid::Uuid::new_v4()));
    std::fs::write(&temporary, bytes)?;
    std::fs::rename(&temporary, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upsert_appends_and_preserves_other_groups_and_comments() {
        let input = r#"{
          // user hook
          "hooks": { "PermissionRequest": [
            { "matcher": "Bash", "hooks": [{"type":"command","command":"user"}] }
          ] }
        }"#;
        let output = upsert_nested_group(
            input,
            "PermissionRequest",
            "__permission-hook",
            "ask __permission-hook claude",
            90000,
            Some("Waiting for AskHuman permission approval"),
        )
        .unwrap();
        assert!(output.contains("// user hook"));
        let groups = nested_groups(&output, "PermissionRequest").unwrap();
        assert_eq!(groups.len(), 2);
        assert!(group_has_marker(&groups[1], "__permission-hook"));
    }
}
