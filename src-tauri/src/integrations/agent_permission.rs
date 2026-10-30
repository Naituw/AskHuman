//! User-level PermissionRequest hook capability for Claude Code and Codex.

use super::agent_rules::AgentTarget;
use super::hook_edit;
use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::path::Path;

pub const MARKER: &str = "__permission-hook";
pub const TIMEOUT_SECS: u64 = 25 * 60 * 60;
pub const STATUS_MESSAGE: &str = "Waiting for AskHuman permission approval";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PermissionStatus {
    pub supported: bool,
    pub unsupported_reason: Option<String>,
    pub enabled: bool,
    #[serde(rename = "configured")]
    pub installed: bool,
    pub outdated: bool,
    pub needs_update: bool,
    pub known_blocked_reason: Option<String>,
    pub other_handlers_detected: bool,
}

#[derive(Default, Serialize, Deserialize)]
struct Preferences {
    #[serde(default)]
    claude: Option<bool>,
    #[serde(default)]
    codex: Option<bool>,
}

pub fn supported(target: AgentTarget) -> bool {
    matches!(target, AgentTarget::ClaudeCode | AgentTarget::Codex)
}

pub fn enabled(target: AgentTarget) -> bool {
    let preferences = load_preferences();
    match target {
        AgentTarget::ClaudeCode => preferences.claude.unwrap_or(true),
        AgentTarget::Codex => preferences.codex.unwrap_or(true),
        _ => false,
    }
}

pub fn set_enabled(target: AgentTarget, value: bool) -> Result<()> {
    if !supported(target) {
        return Err(anyhow!("permission approval is unsupported for this agent"));
    }
    let _lock = super::mutation_lock::IntegrationMutationLock::acquire()?;
    let mut preferences = load_preferences();
    match target {
        AgentTarget::ClaudeCode => preferences.claude = Some(value),
        AgentTarget::Codex => preferences.codex = Some(value),
        _ => unreachable!(),
    }
    save_preferences(&preferences)?;
    let mode = super::agent_mode::current(target);
    if mode == super::agent_mode::Mode::None || !value {
        uninstall_unlocked(target)
    } else {
        install_unlocked(target)
    }
}

pub fn status(target: AgentTarget) -> PermissionStatus {
    if !supported(target) {
        return PermissionStatus {
            supported: false,
            unsupported_reason: Some("native_permission_request_unsupported".to_string()),
            enabled: false,
            installed: false,
            outdated: false,
            needs_update: false,
            known_blocked_reason: None,
            other_handlers_detected: false,
        };
    }
    let path = hook_path(target);
    let text = std::fs::read_to_string(&path).unwrap_or_else(|_| "{}".into());
    let groups = hook_edit::nested_groups(&text, "PermissionRequest").unwrap_or_default();
    let expected = hook_command(target).unwrap_or_default();
    let expected_windows = (target == AgentTarget::Codex)
        .then(|| windows_hook_command(target))
        .transpose()
        .unwrap_or_default();
    let mut installed = false;
    let mut marker_count = 0usize;
    let mut exact_count = 0usize;
    let mut other_handlers_detected = false;
    for group in &groups {
        if let Some(handlers) = group.get("hooks").and_then(Value::as_array) {
            for handler in handlers {
                let command = handler.get("command").and_then(Value::as_str).unwrap_or("");
                if command.contains(MARKER) {
                    installed = true;
                    marker_count += 1;
                    if hook_edit::command_handler_matches(
                        handler,
                        &expected,
                        expected_windows.as_deref(),
                    ) && handler.get("type").and_then(Value::as_str) == Some("command")
                        && handler.get("timeout").and_then(Value::as_u64) == Some(TIMEOUT_SECS)
                        && handler.get("statusMessage").and_then(Value::as_str)
                            == Some(STATUS_MESSAGE)
                    {
                        exact_count += 1;
                    }
                } else {
                    other_handlers_detected = true;
                }
            }
        }
    }
    let trust_ok = target != AgentTarget::Codex
        || (!installed || codex_marker_trusted(&path, MARKER).unwrap_or(false));
    let enabled = enabled(target);
    let outdated = installed && (marker_count != 1 || exact_count != 1 || !trust_ok);
    let mode = super::agent_mode::current(target);
    let needs_update = if mode == super::agent_mode::Mode::None || !enabled {
        installed
    } else {
        !installed || outdated
    };
    PermissionStatus {
        supported: true,
        unsupported_reason: None,
        enabled,
        installed,
        outdated,
        needs_update,
        known_blocked_reason: known_blocked_reason(target),
        other_handlers_detected,
    }
}

pub(crate) fn reconcile_unlocked(target: AgentTarget, mode: super::agent_mode::Mode) -> Result<()> {
    if !supported(target) {
        return Ok(());
    }
    if mode == super::agent_mode::Mode::None || !enabled(target) {
        uninstall_unlocked(target)
    } else {
        install_unlocked(target)
    }
}

pub(crate) fn install_unlocked(target: AgentTarget) -> Result<()> {
    if !supported(target) {
        return Ok(());
    }
    let path = hook_path(target);
    let original_hooks = std::fs::read(&path).ok();
    let original_config = (target == AgentTarget::Codex)
        .then(|| std::fs::read(crate::paths::codex_config_toml()).ok())
        .flatten();
    let existing = original_hooks
        .as_deref()
        .and_then(|bytes| std::str::from_utf8(bytes).ok())
        .unwrap_or("{}");
    let command = hook_command(target)?;
    let command_windows = (target == AgentTarget::Codex)
        .then(|| windows_hook_command(target))
        .transpose()?;
    let updated = hook_edit::upsert_nested_group_with_windows(
        existing,
        "PermissionRequest",
        MARKER,
        &command,
        command_windows.as_deref(),
        TIMEOUT_SECS,
        Some(STATUS_MESSAGE),
    )?;
    hook_edit::atomic_write(&path, updated.as_bytes())?;
    if target == AgentTarget::Codex {
        if let Err(error) = reconcile_codex_trust(existing, &updated, &[MARKER]) {
            restore(&path, original_hooks.as_deref());
            restore(
                &crate::paths::codex_config_toml(),
                original_config.as_deref(),
            );
            return Err(error);
        }
    }
    Ok(())
}

pub(crate) fn uninstall_unlocked(target: AgentTarget) -> Result<()> {
    if !supported(target) {
        return Ok(());
    }
    let path = hook_path(target);
    let Ok(existing) = std::fs::read_to_string(&path) else {
        return Ok(());
    };
    let original_hooks = std::fs::read(&path).ok();
    let original_config = (target == AgentTarget::Codex)
        .then(|| std::fs::read(crate::paths::codex_config_toml()).ok())
        .flatten();
    let updated = hook_edit::remove_nested_marker(&existing, "PermissionRequest", MARKER)?;
    hook_edit::atomic_write(&path, updated.as_bytes())?;
    if target == AgentTarget::Codex {
        if let Err(error) = reconcile_codex_trust(&existing, &updated, &[]) {
            restore(&path, original_hooks.as_deref());
            restore(
                &crate::paths::codex_config_toml(),
                original_config.as_deref(),
            );
            return Err(error);
        }
    }
    Ok(())
}

fn hook_path(target: AgentTarget) -> std::path::PathBuf {
    match target {
        AgentTarget::ClaudeCode => crate::paths::claude_settings_json(),
        AgentTarget::Codex => crate::paths::codex_hooks_json(),
        _ => crate::paths::config_dir().join("unsupported-hooks.json"),
    }
}

fn hook_command(target: AgentTarget) -> Result<String> {
    let executable = crate::development::routing::program_path()
        .context("failed to resolve current executable")?;
    let agent = match target {
        AgentTarget::ClaudeCode => "claude",
        AgentTarget::Codex => "codex",
        _ => return Err(anyhow!("unsupported permission target")),
    };
    Ok(format!(
        "{} {MARKER} {agent}",
        crate::development::routing::integration_prefix(&executable.to_string_lossy())
    ))
}

fn windows_hook_command(target: AgentTarget) -> Result<String> {
    let executable = crate::development::routing::program_path()
        .context("failed to resolve current executable")?;
    let agent = match target {
        AgentTarget::ClaudeCode => "claude",
        AgentTarget::Codex => "codex",
        _ => return Err(anyhow!("unsupported permission target")),
    };
    Ok(hook_edit::powershell_command(
        &executable.to_string_lossy(),
        &[MARKER, agent],
    ))
}

fn load_preferences() -> Preferences {
    std::fs::read(crate::paths::permission_preferences_file())
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

fn save_preferences(preferences: &Preferences) -> Result<()> {
    let bytes = serde_json::to_vec_pretty(preferences)?;
    hook_edit::atomic_write(&crate::paths::permission_preferences_file(), &bytes)
}

fn known_blocked_reason(target: AgentTarget) -> Option<String> {
    match target {
        AgentTarget::ClaudeCode => {
            let text = std::fs::read_to_string(crate::paths::claude_settings_json()).ok()?;
            let value = jsonc_parser::parse_to_serde_value::<Value>(
                &text,
                &jsonc_parser::ParseOptions::default(),
            )
            .ok()?;
            if value.get("disableAllHooks").and_then(Value::as_bool) == Some(true) {
                Some("disable_all_hooks".to_string())
            } else if value.get("allowManagedHooksOnly").and_then(Value::as_bool) == Some(true) {
                Some("allow_managed_hooks_only".to_string())
            } else {
                None
            }
        }
        AgentTarget::Codex => {
            let text = std::fs::read_to_string(crate::paths::codex_config_toml()).ok()?;
            let document = text.parse::<toml_edit::DocumentMut>().ok()?;
            if document
                .get("features")
                .and_then(|features| features.get("hooks"))
                .and_then(toml_edit::Item::as_bool)
                == Some(false)
            {
                Some("hooks_feature_disabled".to_string())
            } else if document
                .get("allow_managed_hooks_only")
                .and_then(toml_edit::Item::as_bool)
                == Some(true)
            {
                Some("allow_managed_hooks_only".to_string())
            } else {
                None
            }
        }
        _ => None,
    }
}

fn restore(path: &Path, bytes: Option<&[u8]>) {
    match bytes {
        Some(bytes) => {
            let _ = hook_edit::atomic_write(path, bytes);
        }
        None => {
            let _ = std::fs::remove_file(path);
        }
    }
}

#[derive(Clone)]
struct TrustEntry {
    key: String,
    hash: String,
    command: String,
}

fn hook_label(event: &str) -> Option<&'static str> {
    match event {
        "PermissionRequest" => Some("permission_request"),
        "SessionStart" => Some("session_start"),
        "UserPromptSubmit" => Some("user_prompt_submit"),
        "PreToolUse" => Some("pre_tool_use"),
        "PostToolUse" => Some("post_tool_use"),
        "PreCompact" => Some("pre_compact"),
        "PostCompact" => Some("post_compact"),
        "SubagentStart" => Some("subagent_start"),
        "SubagentStop" => Some("subagent_stop"),
        "Stop" => Some("stop"),
        _ => None,
    }
}

fn event_uses_matcher(event: &str) -> bool {
    matches!(
        event,
        "PreToolUse"
            | "PermissionRequest"
            | "PostToolUse"
            | "PreCompact"
            | "PostCompact"
            | "SessionStart"
            | "SubagentStart"
            | "SubagentStop"
    )
}

fn trust_entries(path: &Path, text: &str) -> Result<Vec<TrustEntry>> {
    let root =
        jsonc_parser::parse_to_serde_value::<Value>(text, &jsonc_parser::ParseOptions::default())
            .map_err(|error| anyhow!("failed to parse hooks.json: {error}"))?;
    let mut entries = Vec::new();
    let Some(hooks) = root.get("hooks").and_then(Value::as_object) else {
        return Ok(entries);
    };
    for (event, groups) in hooks {
        let Some(label) = hook_label(event) else {
            continue;
        };
        let Some(groups) = groups.as_array() else {
            continue;
        };
        for (group_index, group) in groups.iter().enumerate() {
            let matcher = event_uses_matcher(event)
                .then(|| group.get("matcher").and_then(Value::as_str))
                .flatten();
            let Some(handlers) = group.get("hooks").and_then(Value::as_array) else {
                continue;
            };
            for (handler_index, handler) in handlers.iter().enumerate() {
                if handler.get("type").and_then(Value::as_str) != Some("command") {
                    continue;
                }
                if handler.get("async").and_then(Value::as_bool) == Some(true) {
                    continue;
                }
                #[cfg(windows)]
                let command = handler
                    .get("commandWindows")
                    .or_else(|| handler.get("command"))
                    .and_then(Value::as_str);
                #[cfg(not(windows))]
                let command = handler.get("command").and_then(Value::as_str);
                let Some(command) = command else {
                    continue;
                };
                // Ownership markers live in the portable command. The Windows override may be an
                // encoded PowerShell program, so it is authoritative for hashing but not marker
                // discovery.
                let marker_command = handler
                    .get("command")
                    .and_then(Value::as_str)
                    .unwrap_or(command);
                let timeout = handler
                    .get("timeout")
                    .and_then(Value::as_u64)
                    .unwrap_or(600)
                    .max(1);
                let status_message = handler.get("statusMessage").and_then(Value::as_str);
                // Codex only honors (and hashes) the limit on events that can emit
                // `additionalContext`; elsewhere it is ignored with a warning.
                let additional_context_limit = event_supports_context_limit(event)
                    .then(|| {
                        handler
                            .get("additionalContextLimit")
                            .and_then(Value::as_u64)
                    })
                    .flatten();
                let key = format!(
                    "{}:{label}:{group_index}:{handler_index}",
                    path.to_string_lossy()
                );
                entries.push(TrustEntry {
                    key,
                    hash: trusted_hash(
                        label,
                        matcher,
                        command,
                        timeout,
                        status_message,
                        additional_context_limit,
                    ),
                    command: marker_command.to_string(),
                });
            }
        }
    }
    Ok(entries)
}

/// Events whose handlers may set `additionalContextLimit` (Codex `discovery.rs`).
fn event_supports_context_limit(event: &str) -> bool {
    matches!(
        event,
        "PreToolUse" | "PostToolUse" | "SessionStart" | "UserPromptSubmit" | "SubagentStart"
    )
}

/// Codex's default `additionalContext` spill threshold; an explicit default is normalized away
/// before hashing, so only other values change the trust identity.
const DEFAULT_ADDITIONAL_CONTEXT_LIMIT: u64 = 2_500;

fn trusted_hash(
    label: &str,
    matcher: Option<&str>,
    command: &str,
    timeout: u64,
    status_message: Option<&str>,
    additional_context_limit: Option<u64>,
) -> String {
    let mut handler = serde_json::Map::new();
    handler.insert("type".to_string(), Value::String("command".to_string()));
    handler.insert("command".to_string(), Value::String(command.to_string()));
    handler.insert("timeout".to_string(), Value::Number(timeout.into()));
    handler.insert("async".to_string(), Value::Bool(false));
    if let Some(status_message) = status_message {
        handler.insert(
            "statusMessage".to_string(),
            Value::String(status_message.to_string()),
        );
    }
    if let Some(limit) =
        additional_context_limit.filter(|limit| *limit != DEFAULT_ADDITIONAL_CONTEXT_LIMIT)
    {
        handler.insert(
            "additionalContextLimit".to_string(),
            Value::Number(limit.into()),
        );
    }
    let mut identity = serde_json::Map::new();
    identity.insert("event_name".to_string(), Value::String(label.to_string()));
    if let Some(matcher) = matcher {
        identity.insert("matcher".to_string(), Value::String(matcher.to_string()));
    }
    identity.insert(
        "hooks".to_string(),
        Value::Array(vec![Value::Object(handler)]),
    );
    let identity = Value::Object(identity);
    fn canonical(value: &Value, output: &mut String) {
        match value {
            Value::Object(object) => {
                let mut keys: Vec<&String> = object.keys().collect();
                keys.sort();
                output.push('{');
                for (index, key) in keys.iter().enumerate() {
                    if index > 0 {
                        output.push(',');
                    }
                    output.push_str(&serde_json::to_string(key).unwrap());
                    output.push(':');
                    canonical(&object[*key], output);
                }
                output.push('}');
            }
            Value::Array(array) => {
                output.push('[');
                for (index, item) in array.iter().enumerate() {
                    if index > 0 {
                        output.push(',');
                    }
                    canonical(item, output);
                }
                output.push(']');
            }
            other => output.push_str(&serde_json::to_string(other).unwrap()),
        }
    }
    let mut canonical_json = String::new();
    canonical(&identity, &mut canonical_json);
    let digest = Sha256::digest(canonical_json.as_bytes());
    format!("sha256:{digest:x}")
}

pub(crate) fn reconcile_codex_trust(
    old_hooks: &str,
    new_hooks: &str,
    trust_markers: &[&str],
) -> Result<()> {
    reconcile_codex_trust_at(
        &crate::paths::codex_hooks_json(),
        &crate::paths::codex_config_toml(),
        old_hooks,
        new_hooks,
        trust_markers,
    )
}

/// [`reconcile_codex_trust`] against explicit file locations.
pub(crate) fn reconcile_codex_trust_at(
    hooks_path: &Path,
    config_path: &Path,
    old_hooks: &str,
    new_hooks: &str,
    trust_markers: &[&str],
) -> Result<()> {
    use toml_edit::{DocumentMut, Item, Table};
    let hooks_path = hooks_path.to_path_buf();
    let config_path = config_path.to_path_buf();
    let existing = std::fs::read_to_string(&config_path).unwrap_or_default();
    let mut document = existing
        .parse::<DocumentMut>()
        .map_err(|error| anyhow!("failed to parse Codex config.toml: {error}"))?;
    let old_entries = trust_entries(&hooks_path, old_hooks)?;
    let current_hashes: HashMap<String, String> = document
        .get("hooks")
        .and_then(Item::as_table)
        .and_then(|hooks| hooks.get("state"))
        .and_then(Item::as_table)
        .map(|state| {
            state
                .iter()
                .filter_map(|(key, item)| {
                    item.as_table()
                        .and_then(|table| table.get("trusted_hash"))
                        .and_then(Item::as_str)
                        .map(|hash| (key.to_string(), hash.to_string()))
                })
                .collect()
        })
        .unwrap_or_default();
    let mut trusted_hashes: HashSet<String> = old_entries
        .iter()
        .filter(|entry| current_hashes.get(&entry.key) == Some(&entry.hash))
        .map(|entry| entry.hash.clone())
        .collect();
    trusted_hashes.extend(
        trust_entries(&hooks_path, new_hooks)?
            .into_iter()
            .filter(|entry| {
                trust_markers
                    .iter()
                    .any(|marker| entry.command.contains(marker))
            })
            .map(|entry| entry.hash),
    );
    if !document.as_table().contains_key("hooks") {
        document["hooks"] = Item::Table(Table::new());
    }
    let hooks = document["hooks"]
        .as_table_mut()
        .ok_or_else(|| anyhow!("Codex config hooks is not a table"))?;
    if !hooks.contains_key("state") {
        hooks["state"] = Item::Table(Table::new());
    }
    let state = hooks["state"]
        .as_table_mut()
        .ok_or_else(|| anyhow!("Codex hook state is not a table"))?;
    let prefix = format!("{}:", hooks_path.to_string_lossy());
    state.retain(|key, _| !key.starts_with(&prefix));
    for entry in trust_entries(&hooks_path, new_hooks)? {
        if !trusted_hashes.contains(&entry.hash) {
            continue;
        }
        let mut table = Table::new();
        table.insert("trusted_hash", toml_edit::value(entry.hash));
        state.insert(&entry.key, Item::Table(table));
    }
    hook_edit::atomic_write(&config_path, document.to_string().as_bytes())
}

pub(crate) fn codex_marker_trusted(path: &Path, marker: &str) -> Result<bool> {
    codex_marker_trusted_at(path, &crate::paths::codex_config_toml(), marker)
}

/// [`codex_marker_trusted`] against an explicit `config.toml`.
pub(crate) fn codex_marker_trusted_at(
    path: &Path,
    config_path: &Path,
    marker: &str,
) -> Result<bool> {
    let hooks = std::fs::read_to_string(path)?;
    let entries: Vec<TrustEntry> = trust_entries(path, &hooks)?
        .into_iter()
        .filter(|entry| entry.command.contains(marker))
        .collect();
    if entries.is_empty() {
        return Ok(false);
    }
    let config = std::fs::read_to_string(config_path).unwrap_or_default();
    let document = config.parse::<toml_edit::DocumentMut>()?;
    Ok(entries.iter().all(|entry| {
        document
            .get("hooks")
            .and_then(|hooks| hooks.get("state"))
            .and_then(|state| state.get(&entry.key))
            .and_then(|table| table.get("trusted_hash"))
            .and_then(toml_edit::Item::as_str)
            == Some(entry.hash.as_str())
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preferences_default_on_only_for_supported_agents() {
        assert!(supported(AgentTarget::ClaudeCode));
        assert!(supported(AgentTarget::Codex));
        assert!(!supported(AgentTarget::Cursor));
    }

    #[test]
    fn permission_group_preserves_other_handlers_in_same_group() {
        let input = r#"{"hooks":{"PermissionRequest":[{"hooks":[
          {"type":"command","command":"user-hook"},
          {"type":"command","command":"old __permission-hook claude","timeout":1}
        ]}]}}"#;
        let output = hook_edit::upsert_nested_group(
            input,
            "PermissionRequest",
            MARKER,
            "new __permission-hook claude",
            TIMEOUT_SECS,
            Some(STATUS_MESSAGE),
        )
        .unwrap();
        assert!(output.contains("user-hook"));
        assert!(output.contains("new __permission-hook claude"));
        assert!(!output.contains("old __permission-hook claude"));
    }

    #[test]
    fn trusted_hash_matches_lifecycle_reference_shape() {
        assert!(
            trusted_hash("permission_request", None, "cmd", TIMEOUT_SECS, None, None)
                .starts_with("sha256:")
        );
    }

    #[test]
    fn trusted_hash_includes_status_message_like_codex() {
        let hash = trusted_hash(
            "permission_request",
            None,
            "\"/Users/wutian/.local/bin/AskHuman\" __permission-hook codex",
            TIMEOUT_SECS,
            Some(STATUS_MESSAGE),
            None,
        );
        assert_eq!(
            hash,
            "sha256:7ef2b2088c8e2c086cd1fb9ab238dfdc4f502d5da25e3b54dbab63ab74299d50"
        );
    }

    #[test]
    fn trusted_hash_includes_effective_matcher() {
        assert_ne!(
            trusted_hash("pre_tool_use", Some("Bash"), "cmd", 600, None, None),
            trusted_hash("pre_tool_use", None, "cmd", 600, None, None)
        );
    }

    #[test]
    fn trusted_hash_covers_additional_context_limit_like_codex() {
        let base = trusted_hash("session_start", None, "cmd", 30, None, None);
        // Codex normalizes an explicit default away before hashing.
        assert_eq!(
            base,
            trusted_hash("session_start", None, "cmd", 30, None, Some(2_500))
        );
        // `0` (unlimited) and any other value are part of the trust identity.
        let unlimited = trusted_hash("session_start", None, "cmd", 30, None, Some(0));
        let custom = trusted_hash("session_start", None, "cmd", 30, None, Some(4_096));
        assert_ne!(base, unlimited);
        assert_ne!(base, custom);
        assert_ne!(unlimited, custom);
    }

    /// Reference vectors: `currentHash` values reported by a real Codex 0.163.0 `app-server`
    /// `hooks/list` (isolated HOME / CODEX_HOME, no model call) for the handlers AskHuman installed.
    /// Codex reported all of them `trusted` with AskHuman's hashes, so the hash must stay equal.
    #[test]
    fn trusted_hash_matches_real_codex_for_the_session_start_handlers() {
        let exe = "\"/Users/wutian/Developer/HumanInLoop/src-tauri/target/debug/AskHuman\"";
        // Protocol hook: `additionalContextLimit: 0`, no matcher.
        assert_eq!(
            trusted_hash(
                "session_start",
                None,
                &format!("{exe} __context-recovery-hook codex session-start"),
                30,
                None,
                Some(0),
            ),
            "sha256:9202196705b26b5efb3373d668a83e91bb22fdef37f1fd6d8942c52769b11aaa"
        );
        // Lifecycle SessionStart handler: no limit field at all.
        assert_eq!(
            trusted_hash(
                "session_start",
                None,
                &format!("{exe} __agent-hook codex session-start"),
                600,
                None,
                None,
            ),
            "sha256:3b47259fef106edb7f5fca45a3cb5d795b77fada16fdf92772c3b3a034531b2e"
        );
    }

    #[test]
    fn trust_entries_read_the_limit_only_on_events_that_can_emit_context() {
        let path = Path::new("/home/u/.codex/hooks.json");
        let text = r#"{"hooks":{
          "SessionStart":[{"hooks":[{"type":"command","command":"a __context-recovery-hook codex session-start","timeout":30,"additionalContextLimit":0}]}],
          "Stop":[{"hooks":[{"type":"command","command":"b __stop-hook codex","timeout":30,"additionalContextLimit":0}]}]
        }}"#;
        let entries = trust_entries(path, text).unwrap();
        let session_start = entries
            .iter()
            .find(|entry| entry.command.contains("__context-recovery-hook"))
            .unwrap();
        let stop = entries
            .iter()
            .find(|entry| entry.command.contains("__stop-hook"))
            .unwrap();
        assert_eq!(
            session_start.hash,
            trusted_hash(
                "session_start",
                None,
                "a __context-recovery-hook codex session-start",
                30,
                None,
                Some(0)
            )
        );
        // Stop cannot emit additionalContext, so Codex ignores the field and so must the hash.
        assert_eq!(
            stop.hash,
            trusted_hash("stop", None, "b __stop-hook codex", 30, None, None)
        );
    }
}
