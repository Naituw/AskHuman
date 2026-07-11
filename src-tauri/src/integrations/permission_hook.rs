//! PermissionRequest hook management for Claude Code and Codex.
//!
//! Installs `AskHuman __permission-hook <agent>` into each agent's hook config:
//! - Claude: `~/.claude/settings.json` → `hooks.PermissionRequest[].command`
//! - Codex:  `~/.codex/hooks.json` → `hooks.PermissionRequest[].command`

use crate::integrations::agent_rules::AgentTarget;
use crate::paths;
use anyhow::{Context, Result};

const MARKER: &str = "# AskHuman:permission";
const TIMEOUT_MS: u64 = 86_400_000;

/// Whether this agent supports PermissionRequest hooks.
pub fn supported(target: AgentTarget) -> bool {
    matches!(target, AgentTarget::ClaudeCode | AgentTarget::Codex) && cfg!(unix)
}

/// Whether our PermissionRequest hook is currently installed.
pub fn is_installed(target: AgentTarget) -> bool {
    match target {
        AgentTarget::ClaudeCode => claude_is_installed(),
        AgentTarget::Codex => codex_is_installed(),
        _ => false,
    }
}

/// Whether the installed hook needs updating (command mismatch).
pub fn needs_update(target: AgentTarget) -> bool {
    if !is_installed(target) {
        return false;
    }
    match target {
        AgentTarget::ClaudeCode => claude_needs_update(),
        AgentTarget::Codex => codex_needs_update(),
        _ => false,
    }
}

/// Install (or update) the PermissionRequest hook.
pub fn install(target: AgentTarget) -> Result<()> {
    match target {
        AgentTarget::ClaudeCode => claude_install(),
        AgentTarget::Codex => codex_install(),
        _ => Ok(()),
    }
}

/// Remove the PermissionRequest hook.
pub fn uninstall(target: AgentTarget) -> Result<()> {
    match target {
        AgentTarget::ClaudeCode => claude_uninstall(),
        AgentTarget::Codex => codex_uninstall(),
        _ => Ok(()),
    }
}

fn expected_command() -> String {
    let bin = std::env::current_exe().unwrap_or_else(|_| std::path::PathBuf::from("AskHuman"));
    bin.display().to_string()
}

// ─── Claude ─────────────────────────────────────────────────────────────────

/// Check if an entry in the PermissionRequest array contains our marker.
/// Handles both the correct nested format `{"matcher":"","hooks":[...]}` and
/// the legacy flat format `{"type":"command","command":"..."}`.
fn claude_group_has_marker(entry: &serde_json::Value) -> bool {
    // Nested format: check hooks[*].command
    if let Some(nested) = entry.get("hooks").and_then(|h| h.as_array()) {
        if nested.iter().any(|hook| {
            hook.get("command")
                .and_then(|c| c.as_str())
                .map(|c| c.contains(MARKER))
                .unwrap_or(false)
        }) {
            return true;
        }
    }
    // Legacy flat format: check top-level command
    entry
        .get("command")
        .and_then(|c| c.as_str())
        .map(|c| c.contains(MARKER))
        .unwrap_or(false)
}

fn claude_is_installed() -> bool {
    let path = paths::claude_settings_json();
    let Ok(text) = std::fs::read_to_string(&path) else {
        return false;
    };
    let Ok(root) = serde_json::from_str::<serde_json::Value>(&text) else {
        return false;
    };
    root.get("hooks")
        .and_then(|h| h.get("PermissionRequest"))
        .and_then(|p| p.as_array())
        .map(|arr| arr.iter().any(claude_group_has_marker))
        .unwrap_or(false)
}

fn claude_needs_update() -> bool {
    let path = paths::claude_settings_json();
    let Ok(text) = std::fs::read_to_string(&path) else {
        return true;
    };
    let expected = format!("{} __permission-hook claude", expected_command());
    !text.contains(&expected)
}

fn claude_install() -> Result<()> {
    let path = paths::claude_settings_json();
    let text = std::fs::read_to_string(&path).unwrap_or_else(|_| "{}".to_string());
    let mut root: serde_json::Value = serde_json::from_str(&text).unwrap_or(serde_json::json!({}));

    let cmd = format!("{} __permission-hook claude {}", expected_command(), MARKER);
    // Claude hook format: {"matcher": "", "hooks": [{...}]}
    let group = serde_json::json!({
        "matcher": "",
        "hooks": [{ "type": "command", "command": cmd, "timeout": TIMEOUT_MS / 1000 }],
    });

    let hooks = root
        .as_object_mut()
        .context("settings.json root not object")?
        .entry("hooks")
        .or_insert(serde_json::json!({}));
    let hooks_obj = hooks.as_object_mut().context("hooks not object")?;
    let arr = hooks_obj
        .entry("PermissionRequest")
        .or_insert(serde_json::json!([]));
    let arr_vec = arr.as_array_mut().context("PermissionRequest not array")?;

    arr_vec.retain(|g| !claude_group_has_marker(g));
    arr_vec.push(group);

    let out = serde_json::to_string_pretty(&root)?;
    crate::integrations::claude_hook::atomic_write(&path, out.as_bytes())?;
    Ok(())
}

fn claude_uninstall() -> Result<()> {
    let path = paths::claude_settings_json();
    let Ok(text) = std::fs::read_to_string(&path) else {
        return Ok(());
    };
    let Ok(mut root) = serde_json::from_str::<serde_json::Value>(&text) else {
        return Ok(());
    };

    if let Some(arr) = root
        .pointer_mut("/hooks/PermissionRequest")
        .and_then(|v| v.as_array_mut())
    {
        arr.retain(|g| !claude_group_has_marker(g));
    }

    let out = serde_json::to_string_pretty(&root)?;
    crate::integrations::claude_hook::atomic_write(&path, out.as_bytes())?;
    Ok(())
}

// ─── Codex ──────────────────────────────────────────────────────────────────

/// Check if an entry in the PermissionRequest array contains our marker (same logic as Claude).
fn codex_group_has_marker(entry: &serde_json::Value) -> bool {
    if let Some(nested) = entry.get("hooks").and_then(|h| h.as_array()) {
        if nested.iter().any(|hook| {
            hook.get("command")
                .and_then(|c| c.as_str())
                .map(|c| c.contains(MARKER))
                .unwrap_or(false)
        }) {
            return true;
        }
    }
    entry
        .get("command")
        .and_then(|c| c.as_str())
        .map(|c| c.contains(MARKER))
        .unwrap_or(false)
}

fn codex_is_installed() -> bool {
    let path = paths::codex_hooks_json();
    let Ok(text) = std::fs::read_to_string(&path) else {
        return false;
    };
    let Ok(root) = serde_json::from_str::<serde_json::Value>(&text) else {
        return false;
    };
    root.get("hooks")
        .and_then(|h| h.get("PermissionRequest"))
        .and_then(|p| p.as_array())
        .map(|arr| arr.iter().any(codex_group_has_marker))
        .unwrap_or(false)
}

fn codex_needs_update() -> bool {
    let path = paths::codex_hooks_json();
    let Ok(text) = std::fs::read_to_string(&path) else {
        return true;
    };
    let expected = format!("{} __permission-hook codex", expected_command());
    !text.contains(&expected)
}

fn codex_install() -> Result<()> {
    let path = paths::codex_hooks_json();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let text = std::fs::read_to_string(&path).unwrap_or_else(|_| "{}".to_string());
    let mut root: serde_json::Value = serde_json::from_str(&text).unwrap_or(serde_json::json!({}));

    let cmd = format!("{} __permission-hook codex {}", expected_command(), MARKER);
    let group = serde_json::json!({
        "matcher": "",
        "hooks": [{ "type": "command", "command": cmd, "timeout": TIMEOUT_MS / 1000 }],
    });

    let hooks = root
        .as_object_mut()
        .context("hooks.json root not object")?
        .entry("hooks")
        .or_insert(serde_json::json!({}));
    let hooks_obj = hooks.as_object_mut().context("hooks not object")?;
    let arr = hooks_obj
        .entry("PermissionRequest")
        .or_insert(serde_json::json!([]));
    let arr_vec = arr.as_array_mut().context("PermissionRequest not array")?;

    arr_vec.retain(|g| !codex_group_has_marker(g));
    arr_vec.push(group);

    let out = serde_json::to_string_pretty(&root)?;
    std::fs::write(&path, out.as_bytes())?;
    Ok(())
}

fn codex_uninstall() -> Result<()> {
    let path = paths::codex_hooks_json();
    let Ok(text) = std::fs::read_to_string(&path) else {
        return Ok(());
    };
    let Ok(mut root) = serde_json::from_str::<serde_json::Value>(&text) else {
        return Ok(());
    };

    if let Some(arr) = root
        .pointer_mut("/hooks/PermissionRequest")
        .and_then(|v| v.as_array_mut())
    {
        arr.retain(|g| !codex_group_has_marker(g));
    }

    let out = serde_json::to_string_pretty(&root)?;
    std::fs::write(&path, out.as_bytes())?;
    Ok(())
}
