//! Per-agent "allow delegated runs to use AskHuman" preference (spec
//! `docs/specs/codex-delegated-run-silence.md`, D2).
//!
//! A delegated run is a non-interactive run that another agent, script, or plugin started (for
//! example `codex exec`). By default AskHuman stays silent there, as if it were not integrated.
//! This is a pure preference: it changes no on-disk integration artifact, only affects runs that
//! start after it is changed, and has no environment override.

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};

use crate::agents::AgentKind;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DelegatedStatus {
    /// Whether the agent has a delegated-run concept AskHuman can act on.
    pub supported: bool,
    /// `true` = delegated runs may use AskHuman. Default `false` (silent).
    pub enabled: bool,
}

#[derive(Clone, Default, Serialize, Deserialize)]
struct Preferences {
    #[serde(default)]
    codex: Option<bool>,
}

pub fn supported(kind: AgentKind) -> bool {
    kind == AgentKind::Codex
}

/// Whether delegated runs of this agent may use AskHuman. Unreadable or missing preferences mean
/// "not allowed" (the silent default).
pub fn allowed(kind: AgentKind) -> bool {
    supported(kind) && load_preferences().codex.unwrap_or(false)
}

pub fn set_allowed(kind: AgentKind, value: bool) -> Result<()> {
    if !supported(kind) {
        return Err(anyhow!(
            "delegated-run handling is unsupported for this agent"
        ));
    }
    let _lock = super::mutation_lock::IntegrationMutationLock::acquire()?;
    let mut preferences = load_preferences();
    preferences.codex = Some(value);
    save_preferences(&preferences)
}

pub fn status(kind: AgentKind) -> DelegatedStatus {
    DelegatedStatus {
        supported: supported(kind),
        enabled: allowed(kind),
    }
}

fn load_preferences() -> Preferences {
    std::fs::read(crate::paths::delegated_preferences_file())
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

fn save_preferences(preferences: &Preferences) -> Result<()> {
    super::hook_edit::atomic_write(
        &crate::paths::delegated_preferences_file(),
        &serde_json::to_vec_pretty(preferences)?,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_codex_has_a_delegated_preference() {
        assert!(supported(AgentKind::Codex));
        for kind in [
            AgentKind::Claude,
            AgentKind::Cursor,
            AgentKind::Grok,
            AgentKind::Pi,
        ] {
            assert!(!supported(kind), "{kind:?}");
            assert!(!allowed(kind), "{kind:?}");
            assert!(set_allowed(kind, true).is_err(), "{kind:?}");
            assert!(!status(kind).supported, "{kind:?}");
        }
    }

    #[test]
    fn preferences_default_to_silent_and_tolerate_garbage() {
        let parsed: Preferences = serde_json::from_str("{}").unwrap();
        assert_eq!(parsed.codex, None);
        assert!(serde_json::from_slice::<Preferences>(b"not json").is_err());
        // `unwrap_or_default` in `load_preferences` turns that parse failure into the silent default.
        assert!(!Preferences::default().codex.unwrap_or(false));
        let round_trip: Preferences = serde_json::from_slice(
            &serde_json::to_vec_pretty(&Preferences { codex: Some(true) }).unwrap(),
        )
        .unwrap();
        assert_eq!(round_trip.codex, Some(true));
    }
}
