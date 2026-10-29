//! Integration-mode-owned hooks for post-compaction prompting and MCP session binding.

use super::agent_mode::Mode;
use super::agent_rules::AgentTarget;
use super::hook_edit;
use anyhow::{Context, Result};
use serde_json::Value;
use std::path::{Path, PathBuf};

pub const MARKER: &str = "__context-recovery-hook";
pub const TIMEOUT_SECS: u64 = 30;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RecoveryStatus {
    pub installed: bool,
    pub outdated: bool,
}

#[derive(Clone, Copy)]
struct EventSpec {
    event: &'static str,
    runtime_event: &'static str,
    matcher: Option<&'static str>,
    flat: bool,
    /// Codex `additionalContextLimit` expected on the handler (`None` = field absent).
    context_limit: Option<u64>,
}

/// Claude Code: only the post-compaction recovery sentence, so only `compact` sessions matter.
const SESSION_START: EventSpec = EventSpec {
    event: "SessionStart",
    runtime_event: "session-start",
    matcher: Some("compact"),
    flat: false,
    context_limit: None,
};
/// Codex: the SessionStart hook is the sole carrier of the interaction protocol (spec
/// `codex-delegated-run-silence` D8), so it must run for every session source (no matcher) and
/// must not be spilled to a preview when a long custom collaboration style makes the protocol
/// large (`additionalContextLimit: 0` = unlimited).
const CODEX_SESSION_START: EventSpec = EventSpec {
    event: "SessionStart",
    runtime_event: "session-start",
    matcher: None,
    flat: false,
    context_limit: Some(0),
};
const PRE_TOOL_NESTED: EventSpec = EventSpec {
    event: "PreToolUse",
    runtime_event: "pre-tool-use",
    matcher: None,
    flat: false,
    context_limit: None,
};
const PRE_TOOL_CURSOR: EventSpec = EventSpec {
    event: "preToolUse",
    runtime_event: "pre-tool-use",
    matcher: None,
    flat: true,
    context_limit: None,
};

fn all_specs(target: AgentTarget) -> &'static [EventSpec] {
    match target {
        AgentTarget::ClaudeCode => &[SESSION_START, PRE_TOOL_NESTED],
        AgentTarget::Codex => &[CODEX_SESSION_START],
        AgentTarget::Cursor => &[PRE_TOOL_CURSOR],
        AgentTarget::Grok => &[PRE_TOOL_NESTED],
        AgentTarget::Pi | AgentTarget::Antigravity => &[],
    }
}

fn desired(target: AgentTarget, mode: Mode, spec: EventSpec) -> bool {
    match mode {
        Mode::None => false,
        Mode::Cli => {
            matches!(target, AgentTarget::ClaudeCode | AgentTarget::Codex)
                && spec.runtime_event == "session-start"
        }
        Mode::Mcp => match target {
            AgentTarget::ClaudeCode => true,
            AgentTarget::Codex => spec.runtime_event == "session-start",
            AgentTarget::Cursor | AgentTarget::Grok => spec.runtime_event == "pre-tool-use",
            AgentTarget::Pi | AgentTarget::Antigravity => false,
        },
    }
}

pub fn supported() -> bool {
    true
}

/// On-disk locations of Codex's user-level hook configuration. Injectable so migration logic can
/// be tested against temporary files without redirecting `HOME`.
#[derive(Clone, Debug)]
pub(crate) struct CodexFiles {
    pub hooks_json: PathBuf,
    pub config_toml: PathBuf,
}

impl CodexFiles {
    pub(crate) fn real() -> Self {
        Self {
            hooks_json: crate::paths::codex_hooks_json(),
            config_toml: crate::paths::codex_config_toml(),
        }
    }
}

pub fn status(target: AgentTarget, mode: Mode) -> RecoveryStatus {
    status_at(target, mode, &CodexFiles::real())
}

/// A managed hook entry exists for `target`: no freshness, mode, or trust check.
pub(crate) fn hook_installed_at(target: AgentTarget, codex: &CodexFiles) -> bool {
    if !supported() || matches!(target, AgentTarget::Pi | AgentTarget::Antigravity) {
        return false;
    }
    let text = std::fs::read_to_string(hook_path_at(target, codex)).unwrap_or_else(|_| "{}".into());
    status_from_text(target, Mode::Cli, &text, true).installed
}

pub(crate) fn status_at(target: AgentTarget, mode: Mode, codex: &CodexFiles) -> RecoveryStatus {
    if !supported() || matches!(target, AgentTarget::Pi | AgentTarget::Antigravity) {
        return RecoveryStatus::default();
    }
    let path = hook_path_at(target, codex);
    let text = std::fs::read_to_string(&path).unwrap_or_else(|_| "{}".into());
    let mut result = status_from_text(target, mode, &text, true);
    if target == AgentTarget::Codex
        && result.installed
        && !super::agent_permission::codex_marker_trusted_at(&path, &codex.config_toml, MARKER)
            .unwrap_or(false)
    {
        result.outdated = true;
    }
    result
}

fn status_from_text(target: AgentTarget, mode: Mode, text: &str, trust_ok: bool) -> RecoveryStatus {
    let Ok(value) =
        jsonc_parser::parse_to_serde_value::<Value>(text, &jsonc_parser::ParseOptions::default())
    else {
        return RecoveryStatus {
            installed: false,
            outdated: true,
        };
    };
    if !value.is_object() {
        return RecoveryStatus {
            installed: false,
            outdated: true,
        };
    }
    let mut marker_count = 0usize;
    let mut exact_count = 0usize;
    let mut desired_count = 0usize;
    for spec in all_specs(target) {
        let entries = value
            .get("hooks")
            .and_then(|hooks| hooks.get(spec.event))
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        for entry in entries {
            let (command, command_windows, timeout, matcher, context_limit) = if spec.flat {
                (
                    entry.get("command").and_then(Value::as_str),
                    None,
                    entry.get("timeout").and_then(Value::as_u64),
                    None,
                    None,
                )
            } else {
                let handler = entry
                    .get("hooks")
                    .and_then(Value::as_array)
                    .and_then(|handlers| handlers.first());
                (
                    handler
                        .and_then(|handler| handler.get("command"))
                        .and_then(Value::as_str),
                    handler
                        .and_then(|handler| handler.get("commandWindows"))
                        .and_then(Value::as_str),
                    handler
                        .and_then(|handler| handler.get("timeout"))
                        .and_then(Value::as_u64),
                    entry.get("matcher").and_then(Value::as_str),
                    handler
                        .and_then(|handler| handler.get("additionalContextLimit"))
                        .and_then(Value::as_u64),
                )
            };
            if !command.is_some_and(|command| command.contains(MARKER)) {
                continue;
            }
            marker_count += 1;
            if desired(target, mode, *spec) {
                let expected = hook_command(target, spec.runtime_event).unwrap_or_default();
                let expected_windows = (target == AgentTarget::Codex)
                    .then(|| windows_hook_command(target, spec.runtime_event))
                    .transpose()
                    .unwrap_or_default();
                let windows_override_ok = expected_windows
                    .as_deref()
                    .is_none_or(|expected_windows| command_windows == Some(expected_windows));
                if command == Some(expected.as_str())
                    && windows_override_ok
                    && timeout == Some(TIMEOUT_SECS)
                    && matcher == spec.matcher
                    && context_limit == spec.context_limit
                {
                    exact_count += 1;
                }
            }
        }
        if desired(target, mode, *spec) {
            desired_count += 1;
        }
    }
    let installed = marker_count > 0;
    RecoveryStatus {
        installed,
        outdated: marker_count != desired_count || exact_count != desired_count || !trust_ok,
    }
}

pub fn needs_update(target: AgentTarget, mode: Mode) -> bool {
    if !supported() || matches!(target, AgentTarget::Pi | AgentTarget::Antigravity) {
        return false;
    }
    status(target, mode).outdated
}

pub(crate) fn reconcile_unlocked(target: AgentTarget, mode: Mode) -> Result<()> {
    reconcile_at(target, mode, &CodexFiles::real())
}

pub(crate) fn reconcile_at(target: AgentTarget, mode: Mode, codex: &CodexFiles) -> Result<()> {
    if !supported() || matches!(target, AgentTarget::Pi | AgentTarget::Antigravity) {
        return Ok(());
    }
    let path = hook_path_at(target, codex);
    let original_hooks = std::fs::read(&path).ok();
    let original_config = (target == AgentTarget::Codex)
        .then(|| std::fs::read(&codex.config_toml).ok())
        .flatten();
    let original_text = existing_hook_text(original_hooks.as_deref())?;
    let updated = reconcile_text(target, mode, original_text)?;
    if updated == "{}" && original_hooks.is_none() {
        return Ok(());
    }
    let hooks_changed = updated != original_text;
    if hooks_changed {
        hook_edit::atomic_write(&path, updated.as_bytes())?;
    }
    if target == AgentTarget::Codex {
        let trust_markers: &[&str] = if mode == Mode::None { &[] } else { &[MARKER] };
        if let Err(error) = super::agent_permission::reconcile_codex_trust_at(
            &path,
            &codex.config_toml,
            original_text,
            &updated,
            trust_markers,
        ) {
            if hooks_changed {
                restore(&path, original_hooks.as_deref());
            }
            restore(&codex.config_toml, original_config.as_deref());
            return Err(error);
        }
    }
    Ok(())
}

fn existing_hook_text(bytes: Option<&[u8]>) -> Result<&str> {
    match bytes {
        Some(bytes) => std::str::from_utf8(bytes).context("hook config is not valid UTF-8"),
        None => Ok("{}"),
    }
}

fn reconcile_text(target: AgentTarget, mode: Mode, text: &str) -> Result<String> {
    if !status_from_text(target, mode, text, true).outdated {
        return Ok(text.to_string());
    }
    let mut updated = text.to_string();
    for spec in all_specs(target) {
        if desired(target, mode, *spec) {
            let command = hook_command(target, spec.runtime_event)?;
            let command_windows = (target == AgentTarget::Codex)
                .then(|| windows_hook_command(target, spec.runtime_event))
                .transpose()?;
            updated = if spec.flat {
                hook_edit::upsert_flat_handler(
                    &updated,
                    spec.event,
                    MARKER,
                    &command,
                    TIMEOUT_SECS,
                    false,
                )?
            } else {
                hook_edit::upsert_nested_group_matched_with_extras(
                    &updated,
                    spec.event,
                    MARKER,
                    spec.matcher,
                    &command,
                    TIMEOUT_SECS,
                    hook_edit::HandlerExtras {
                        command_windows: command_windows.as_deref(),
                        additional_context_limit: spec.context_limit,
                    },
                )?
            };
        } else {
            updated = if spec.flat {
                hook_edit::remove_flat_marker(&updated, spec.event, MARKER)?
            } else {
                hook_edit::remove_nested_marker(&updated, spec.event, MARKER)?
            };
        }
    }
    Ok(updated)
}

fn hook_path_at(target: AgentTarget, codex: &CodexFiles) -> PathBuf {
    match target {
        AgentTarget::Codex => codex.hooks_json.clone(),
        other => hook_path(other),
    }
}

fn hook_path(target: AgentTarget) -> PathBuf {
    match target {
        AgentTarget::ClaudeCode => crate::paths::claude_settings_json(),
        AgentTarget::Codex => crate::paths::codex_hooks_json(),
        AgentTarget::Cursor => crate::paths::cursor_hooks_json(),
        AgentTarget::Grok => crate::paths::grok_hooks_json(),
        AgentTarget::Pi => crate::paths::pi_extension_file(),
        AgentTarget::Antigravity => {
            crate::paths::config_dir().join("unsupported-antigravity-hooks.json")
        }
    }
}

fn hook_command(target: AgentTarget, event: &str) -> Result<String> {
    let executable = std::env::current_exe().context("failed to resolve current executable")?;
    let agent = match target {
        AgentTarget::ClaudeCode => "claude",
        AgentTarget::Codex => "codex",
        AgentTarget::Cursor => "cursor",
        AgentTarget::Grok => "grok",
        AgentTarget::Pi => "pi",
        AgentTarget::Antigravity => {
            return Err(anyhow::anyhow!(
                "Antigravity recovery hooks are unsupported"
            ))
        }
    };
    Ok(format!(
        "\"{}\" {MARKER} {agent} {event}",
        executable.to_string_lossy()
    ))
}

fn windows_hook_command(target: AgentTarget, event: &str) -> Result<String> {
    let executable = std::env::current_exe().context("failed to resolve current executable")?;
    let agent = match target {
        AgentTarget::ClaudeCode => "claude",
        AgentTarget::Codex => "codex",
        AgentTarget::Cursor => "cursor",
        AgentTarget::Grok => "grok",
        AgentTarget::Pi => "pi",
        AgentTarget::Antigravity => {
            return Err(anyhow::anyhow!(
                "Antigravity recovery hooks are unsupported"
            ))
        }
    };
    Ok(hook_edit::powershell_command(
        &executable.to_string_lossy(),
        &[MARKER, agent, event],
    ))
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

#[cfg(test)]
mod tests {
    use super::*;

    fn desired_events(target: AgentTarget, mode: Mode) -> Vec<&'static str> {
        all_specs(target)
            .iter()
            .filter(|spec| desired(target, mode, **spec))
            .map(|spec| spec.runtime_event)
            .collect()
    }

    #[test]
    fn desired_matrix_is_mode_specific() {
        let cases = [
            (AgentTarget::ClaudeCode, Mode::None, vec![]),
            (AgentTarget::ClaudeCode, Mode::Cli, vec!["session-start"]),
            (
                AgentTarget::ClaudeCode,
                Mode::Mcp,
                vec!["session-start", "pre-tool-use"],
            ),
            (AgentTarget::Codex, Mode::None, vec![]),
            (AgentTarget::Codex, Mode::Cli, vec!["session-start"]),
            (AgentTarget::Codex, Mode::Mcp, vec!["session-start"]),
            (AgentTarget::Cursor, Mode::None, vec![]),
            (AgentTarget::Cursor, Mode::Cli, vec![]),
            (AgentTarget::Cursor, Mode::Mcp, vec!["pre-tool-use"]),
            (AgentTarget::Grok, Mode::None, vec![]),
            (AgentTarget::Grok, Mode::Cli, vec![]),
            (AgentTarget::Grok, Mode::Mcp, vec!["pre-tool-use"]),
        ];
        for (target, mode, expected) in cases {
            assert_eq!(
                desired_events(target, mode),
                expected,
                "{target:?} {mode:?}"
            );
        }
    }

    #[test]
    fn antigravity_never_reads_or_reconciles_codex_hook_files() {
        let directory = tempfile::tempdir().unwrap();
        let codex = CodexFiles {
            hooks_json: directory.path().join("hooks.json"),
            config_toml: directory.path().join("config.toml"),
        };
        let hooks = b"foreign hook configuration";
        let config = b"foreign trust configuration";
        std::fs::write(&codex.hooks_json, hooks).unwrap();
        std::fs::write(&codex.config_toml, config).unwrap();

        for mode in [Mode::None, Mode::Cli, Mode::Mcp] {
            assert!(desired_events(AgentTarget::Antigravity, mode).is_empty());
            assert_eq!(
                status_at(AgentTarget::Antigravity, mode, &codex),
                RecoveryStatus::default()
            );
            assert!(!hook_installed_at(AgentTarget::Antigravity, &codex));
            reconcile_at(AgentTarget::Antigravity, mode, &codex).unwrap();
            assert_eq!(std::fs::read(&codex.hooks_json).unwrap(), hooks);
            assert_eq!(std::fs::read(&codex.config_toml).unwrap(), config);
        }
    }

    #[test]
    fn reconcile_and_status_cover_every_supported_mode_shape() {
        for (target, mode) in [
            (AgentTarget::ClaudeCode, Mode::Cli),
            (AgentTarget::ClaudeCode, Mode::Mcp),
            (AgentTarget::Codex, Mode::Cli),
            (AgentTarget::Codex, Mode::Mcp),
            (AgentTarget::Cursor, Mode::Mcp),
            (AgentTarget::Grok, Mode::Mcp),
        ] {
            let text = reconcile_text(target, mode, "{}").unwrap();
            assert_eq!(
                status_from_text(target, mode, &text, true),
                RecoveryStatus {
                    installed: true,
                    outdated: false,
                },
                "{target:?} {mode:?}: {text}"
            );
            let removed = reconcile_text(target, Mode::None, &text).unwrap();
            assert_eq!(
                status_from_text(target, Mode::None, &removed, true),
                RecoveryStatus {
                    installed: false,
                    outdated: false,
                },
                "{target:?}: {removed}"
            );
        }
        for target in [AgentTarget::Cursor, AgentTarget::Grok] {
            assert_eq!(
                status_from_text(target, Mode::Cli, "{}", true),
                RecoveryStatus {
                    installed: false,
                    outdated: false,
                }
            );
        }
    }

    #[test]
    fn status_detects_missing_stale_duplicate_and_untrusted_hooks() {
        assert_eq!(
            status_from_text(AgentTarget::ClaudeCode, Mode::Mcp, "{}", true),
            RecoveryStatus {
                installed: false,
                outdated: true,
            }
        );
        let exact = reconcile_text(AgentTarget::Codex, Mode::Mcp, "{}").unwrap();
        assert_eq!(
            status_from_text(AgentTarget::Codex, Mode::Mcp, &exact, false),
            RecoveryStatus {
                installed: true,
                outdated: true,
            }
        );
        let stale = exact.replace(&format!("\"timeout\": {TIMEOUT_SECS}"), "\"timeout\": 1");
        assert!(status_from_text(AgentTarget::Codex, Mode::Mcp, &stale, true).outdated);
        assert!(!status_from_text(AgentTarget::Codex, Mode::Mcp, &exact, true).outdated);
        let mut duplicated: Value = serde_json::from_str(&exact).unwrap();
        let mut second = duplicated["hooks"]["SessionStart"][0].clone();
        second["hooks"][0]["command"] = Value::String(format!("duplicate {MARKER}"));
        duplicated["hooks"]["SessionStart"]
            .as_array_mut()
            .unwrap()
            .push(second);
        assert!(
            status_from_text(AgentTarget::Codex, Mode::Mcp, &duplicated.to_string(), true).outdated
        );
        assert!(status_from_text(AgentTarget::Cursor, Mode::Mcp, "not json", true).outdated);
        assert!(status_from_text(AgentTarget::Cursor, Mode::None, "[]", true).outdated);
    }

    #[test]
    fn codex_protocol_hook_runs_for_every_source_without_a_spill_limit() {
        for mode in [Mode::Cli, Mode::Mcp] {
            let text = reconcile_text(AgentTarget::Codex, mode, "{}").unwrap();
            let groups = hook_edit::nested_groups(&text, "SessionStart").unwrap();
            assert_eq!(groups.len(), 1, "{text}");
            assert!(
                groups[0].get("matcher").is_none(),
                "no matcher = every SessionStart source: {text}"
            );
            let handler = &groups[0]["hooks"][0];
            assert_eq!(handler["additionalContextLimit"], 0, "{text}");
            assert_eq!(handler["timeout"], TIMEOUT_SECS);
            assert!(handler["commandWindows"].as_str().is_some());
        }
        // Claude keeps the compact-only recovery hook and no extra handler field.
        let claude = reconcile_text(AgentTarget::ClaudeCode, Mode::Cli, "{}").unwrap();
        let groups = hook_edit::nested_groups(&claude, "SessionStart").unwrap();
        assert_eq!(groups[0]["matcher"], "compact");
        assert!(groups[0]["hooks"][0]
            .get("additionalContextLimit")
            .is_none());
    }

    #[test]
    fn legacy_compact_only_codex_identity_needs_an_update() {
        let command = hook_command(AgentTarget::Codex, "session-start").unwrap();
        let command_windows = windows_hook_command(AgentTarget::Codex, "session-start").unwrap();
        let handler = |extra: Value| {
            let mut handler = serde_json::json!({
                "type": "command",
                "command": command,
                "commandWindows": command_windows,
                "timeout": TIMEOUT_SECS,
            });
            for (key, value) in extra.as_object().unwrap() {
                handler[key] = value.clone();
            }
            handler
        };
        let build =
            |group: Value| serde_json::json!({"hooks": {"SessionStart": [group]}}).to_string();
        let exact = build(serde_json::json!({
            "hooks": [handler(serde_json::json!({"additionalContextLimit": 0}))]
        }));
        assert!(!status_from_text(AgentTarget::Codex, Mode::Cli, &exact, true).outdated);

        for stale in [
            // Pre-migration identity: compact-only matcher, no limit.
            serde_json::json!({
                "matcher": "compact",
                "hooks": [handler(serde_json::json!({}))]
            }),
            // Right matcher, but the default 2,500-token spill limit would still apply.
            serde_json::json!({"hooks": [handler(serde_json::json!({}))]}),
            // Wrong limit.
            serde_json::json!({
                "hooks": [handler(serde_json::json!({"additionalContextLimit": 4096}))]
            }),
            // Right limit, stray matcher.
            serde_json::json!({
                "matcher": "startup",
                "hooks": [handler(serde_json::json!({"additionalContextLimit": 0}))]
            }),
        ] {
            let text = build(stale);
            let status = status_from_text(AgentTarget::Codex, Mode::Cli, &text, true);
            assert_eq!(
                status,
                RecoveryStatus {
                    installed: true,
                    outdated: true
                },
                "{text}"
            );
            // Reconciling rewrites it to the exact identity.
            let fixed = reconcile_text(AgentTarget::Codex, Mode::Cli, &text).unwrap();
            assert!(!status_from_text(AgentTarget::Codex, Mode::Cli, &fixed, true).outdated);
            assert_eq!(fixed.matches(MARKER).count(), 1);
        }
    }

    #[test]
    fn reconcile_preserves_user_and_lifecycle_hooks_and_is_idempotent() {
        let original = r#"{
          // keep this comment
          "hooks": {
            "SessionStart": [{"matcher":"startup","hooks":[{"type":"command","command":"user-start"}]}],
            "PreToolUse": [{"hooks":[{"type":"command","command":"AskHuman __agent-hook claude activity"}]}]
          }
        }"#;
        let installed = reconcile_text(AgentTarget::ClaudeCode, Mode::Mcp, original).unwrap();
        assert!(installed.contains("// keep this comment"));
        assert!(installed.contains("user-start"));
        assert!(installed.contains("__agent-hook claude activity"));
        assert_eq!(installed.matches(MARKER).count(), 2);
        assert_eq!(
            reconcile_text(AgentTarget::ClaudeCode, Mode::Mcp, &installed).unwrap(),
            installed
        );

        let cli = reconcile_text(AgentTarget::ClaudeCode, Mode::Cli, &installed).unwrap();
        assert_eq!(cli.matches(MARKER).count(), 1);
        assert!(cli.contains("user-start"));
        assert!(cli.contains("__agent-hook claude activity"));

        let removed = reconcile_text(AgentTarget::ClaudeCode, Mode::None, &cli).unwrap();
        assert!(!removed.contains(MARKER));
        assert!(removed.contains("user-start"));
        assert!(removed.contains("__agent-hook claude activity"));
        assert!(reconcile_text(AgentTarget::ClaudeCode, Mode::Mcp, "[]").is_err());
        assert!(reconcile_text(AgentTarget::Cursor, Mode::None, "not json").is_err());
    }

    #[test]
    fn existing_hook_text_rejects_invalid_utf8_instead_of_overwriting_it() {
        assert_eq!(existing_hook_text(None).unwrap(), "{}");
        assert_eq!(
            existing_hook_text(Some(b"{\"hooks\":{}}")).unwrap(),
            r#"{"hooks":{}}"#
        );
        assert!(existing_hook_text(Some(&[0xff, 0xfe])).is_err());
    }
}
