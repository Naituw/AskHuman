//! Agent installation detection and the per-Agent rows the wizard shows (spec D6, D19).
//!
//! An Agent counts as installed when **any** of three signals holds: its command is found by the
//! login shell, its configuration directory exists, or (macOS) its app bundle is installed.

use crate::integrations::agent_launch;
use crate::integrations::agent_mode::{self, Mode};
use crate::integrations::codex_protocol;
use crate::onboarding::agents::{self, AgentDef};
use crate::onboarding::plan::{self, PlannedByMode};
use crate::paths;
use serde::Serialize;
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Signals {
    /// The login shell resolves the Agent's command.
    pub cli: bool,
    /// The Agent's configuration directory exists.
    pub config_dir: bool,
    /// The Agent's desktop app bundle is installed (macOS only).
    pub app: bool,
}

impl Signals {
    pub fn detected(&self) -> bool {
        self.cli || self.config_dir || self.app
    }
}

/// What the machine answered, before the per-Agent rules below are applied.
#[derive(Debug, Clone, Copy, Default)]
pub struct RawProbe {
    pub cli_executable: bool,
    pub config_dir_exists: bool,
    pub app_installed: bool,
}

/// Apply the per-Agent rules to a raw probe: Antigravity has no configuration-directory signal of
/// its own, and only Cursor, Codex and Antigravity ship a macOS app the wizard looks for.
pub fn classify(id: &str, raw: RawProbe, macos: bool) -> Signals {
    Signals {
        cli: raw.cli_executable,
        config_dir: id != "antigravity" && raw.config_dir_exists,
        app: macos && matches!(id, "cursor" | "codex" | "antigravity") && raw.app_installed,
    }
}

fn config_dir_for(id: &str) -> Option<PathBuf> {
    match id {
        "cursor" => Some(paths::cursor_dir()),
        "claude" => Some(paths::claude_dir()),
        "codex" => Some(paths::codex_dir()),
        "grok" => Some(paths::grok_dir()),
        "pi" => Some(paths::pi_agent_dir()),
        _ => None,
    }
}

/// `/Applications/<name>.app` or `~/Applications/<name>.app` exists.
fn app_installed(name: &str) -> bool {
    [
        PathBuf::from("/Applications").join(name),
        paths::home().join("Applications").join(name),
    ]
    .iter()
    .any(|path| path.exists())
}

fn app_bundle_for(id: &str) -> Option<&'static str> {
    match id {
        "cursor" => Some("Cursor.app"),
        "codex" => Some("Codex.app"),
        "antigravity" => Some("Antigravity.app"),
        _ => None,
    }
}

/// Probe the real machine for one Agent. `cli_executable` comes from the shared readiness probe so
/// the login shell is asked once per Agent (the probe is cached and parallel).
fn probe(def: &AgentDef, cli_executable: bool) -> Signals {
    let raw = RawProbe {
        cli_executable,
        config_dir_exists: config_dir_for(def.id).is_some_and(|dir| dir.exists()),
        app_installed: cfg!(target_os = "macos")
            && app_bundle_for(def.id).is_some_and(app_installed),
    };
    classify(def.id, raw, cfg!(target_os = "macos"))
}

/// One row of the wizard's Agent page.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentScanRow {
    pub id: String,
    pub title: String,
    pub signals: Signals,
    pub detected: bool,
    /// Current integration mode: `none` | `cli` | `mcp`.
    pub mode: String,
    pub needs_update: bool,
    /// Why the row's state could not be read (Antigravity's strict status read); the row is
    /// shown disabled.
    pub status_error: Option<String>,
    pub supports_cli: bool,
    pub supports_mcp: bool,
    pub recommended: String,
    /// Codex only: protocol delivery state (`notInstalled` | `trusted` | `untrusted` |
    /// `needsUpdate` | `unsupportedVersion`).
    pub delivery: Option<String>,
    pub planned: PlannedByMode,
}

fn row(def: &AgentDef, cli_executable: bool) -> AgentScanRow {
    let signals = probe(def, cli_executable);
    let (mode, status_error) = match agents::current_mode(def) {
        Ok(mode) => (mode, None),
        Err(error) => (Mode::None, Some(error)),
    };
    let needs_update = status_error.is_none() && agents::needs_update(def).unwrap_or(false);
    let delivery = (def.target == crate::integrations::agent_rules::AgentTarget::Codex)
        .then(|| codex_protocol::delivery().as_str().to_string());
    AgentScanRow {
        id: def.id.to_string(),
        title: def.title.to_string(),
        signals,
        detected: signals.detected(),
        mode: mode.as_str().to_string(),
        needs_update,
        status_error,
        supports_cli: def.cli,
        supports_mcp: def.mcp,
        recommended: def.recommended.as_str().to_string(),
        delivery,
        planned: plan::planned_by_mode(def),
    }
}

/// Scan every Agent available on this platform, in display order.
pub fn scan() -> Vec<AgentScanRow> {
    let readiness = agent_launch::all_readiness();
    agents::available()
        .into_iter()
        .map(|def| {
            let kind = agent_mode::kind_for_target(def.target);
            let cli = readiness
                .iter()
                .any(|r| r.kind == kind && r.executable.is_some());
            row(def, cli)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw(cli: bool, dir: bool, app: bool) -> RawProbe {
        RawProbe {
            cli_executable: cli,
            config_dir_exists: dir,
            app_installed: app,
        }
    }

    #[test]
    fn any_single_signal_detects_an_agent() {
        for (probe, expect) in [
            (raw(true, false, false), true),
            (raw(false, true, false), true),
            (raw(false, false, true), true),
            (raw(false, false, false), false),
        ] {
            assert_eq!(classify("cursor", probe, true).detected(), expect);
        }
    }

    #[test]
    fn antigravity_has_no_config_dir_signal() {
        let signals = classify("antigravity", raw(false, true, false), true);
        assert!(!signals.config_dir);
        assert!(!signals.detected());
    }

    #[test]
    fn the_app_signal_is_macos_only_and_limited_to_three_agents() {
        assert!(classify("codex", raw(false, false, true), true).app);
        assert!(!classify("codex", raw(false, false, true), false).app);
        assert!(!classify("claude", raw(false, false, true), true).app);
        assert!(!classify("pi", raw(false, false, true), true).app);
        assert!(!classify("grok", raw(false, false, true), true).app);
    }

    #[test]
    fn app_bundles_exist_only_for_the_apps_we_look_for() {
        assert_eq!(app_bundle_for("cursor"), Some("Cursor.app"));
        assert_eq!(app_bundle_for("claude"), None);
    }

    #[test]
    fn rows_serialize_in_camel_case() {
        let def = agents::find("pi").unwrap();
        let value = serde_json::to_value(row(def, false)).unwrap();
        for key in [
            "id",
            "title",
            "signals",
            "detected",
            "mode",
            "needsUpdate",
            "statusError",
            "supportsCli",
            "supportsMcp",
            "recommended",
            "delivery",
            "planned",
        ] {
            assert!(value.get(key).is_some(), "missing {key}");
        }
        assert_eq!(value["supportsMcp"], false);
        assert_eq!(value["recommended"], "cli");
        assert!(value["delivery"].is_null(), "only Codex reports delivery");
        assert!(value["planned"]["mcp"].as_array().unwrap().is_empty());
    }
}
