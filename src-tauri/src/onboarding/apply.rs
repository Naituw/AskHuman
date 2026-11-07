//! Apply the Agent selections the user confirmed in the wizard, one Agent at a time.
//!
//! Each Agent is independent: a failure on one never stops the others. Every write is verified by
//! reading the mode back, and the outcome is reported per Agent. The wizard never uninstalls: a
//! selection is always `cli` or `mcp`.

use crate::integrations::agent_mode::{self, Mode};
use crate::integrations::agent_rules::AgentTarget;
use crate::integrations::codex_protocol;
use crate::onboarding::agents::{self, AgentDef};
use serde::{Deserialize, Serialize};

/// One confirmed row: apply `mode` (`"cli"` | `"mcp"`) to `agent`.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Selection {
    pub agent: String,
    pub mode: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplyResult {
    pub agent: String,
    pub requested: String,
    pub ok: bool,
    pub error: Option<String>,
    /// The mode read back after the write (`none` | `cli` | `mcp`).
    pub mode_after: String,
    /// Codex only: protocol delivery state after applying. `untrusted` right after installing is
    /// the expected "trust the hook on first launch" state, not a failure.
    pub delivery: Option<String>,
}

/// The side effects `apply` needs, so tests can substitute them.
pub trait Backend {
    fn current_mode(&self, def: &AgentDef) -> Result<Mode, String>;
    fn needs_update(&self, def: &AgentDef) -> Result<bool, String>;
    fn set(&self, def: &AgentDef, mode: Mode) -> Result<(), String>;
    fn update(&self, def: &AgentDef) -> Result<(), String>;
    fn delivery(&self, def: &AgentDef) -> Option<String>;
}

pub struct RealBackend;

impl Backend for RealBackend {
    fn current_mode(&self, def: &AgentDef) -> Result<Mode, String> {
        agents::current_mode(def)
    }

    fn needs_update(&self, def: &AgentDef) -> Result<bool, String> {
        agents::needs_update(def)
    }

    fn set(&self, def: &AgentDef, mode: Mode) -> Result<(), String> {
        agent_mode::set(def.target, mode).map_err(|error| error.to_string())
    }

    fn update(&self, def: &AgentDef) -> Result<(), String> {
        agent_mode::update(def.target).map_err(|error| error.to_string())
    }

    fn delivery(&self, def: &AgentDef) -> Option<String> {
        (def.target == AgentTarget::Codex).then(|| codex_protocol::delivery().as_str().to_string())
    }
}

/// Apply every selection against the real machine.
pub fn apply(selections: &[Selection]) -> Vec<ApplyResult> {
    apply_with(&RealBackend, selections)
}

pub fn apply_with(backend: &dyn Backend, selections: &[Selection]) -> Vec<ApplyResult> {
    selections
        .iter()
        .map(|selection| apply_one(backend, selection))
        .collect()
}

fn failure(selection: &Selection, error: String, mode_after: Mode) -> ApplyResult {
    ApplyResult {
        agent: selection.agent.clone(),
        requested: selection.mode.clone(),
        ok: false,
        error: Some(error),
        mode_after: mode_after.as_str().to_string(),
        delivery: None,
    }
}

fn apply_one(backend: &dyn Backend, selection: &Selection) -> ApplyResult {
    let Some(def) = agents::available()
        .into_iter()
        .find(|def| def.id == selection.agent)
    else {
        return failure(
            selection,
            format!("unknown or unavailable agent: {}", selection.agent),
            Mode::None,
        );
    };
    let mode = match Mode::parse(&selection.mode) {
        Some(mode @ (Mode::Cli | Mode::Mcp)) => mode,
        _ => {
            return failure(
                selection,
                format!("unsupported mode: {}", selection.mode),
                Mode::None,
            )
        }
    };
    if !def.supports(mode) {
        return failure(
            selection,
            format!("{} does not support {} mode", def.title, mode.as_str()),
            Mode::None,
        );
    }

    let current = match backend.current_mode(def) {
        Ok(current) => current,
        Err(error) => return failure(selection, error, Mode::None),
    };
    let needs_update = match backend.needs_update(def) {
        Ok(needs) => needs,
        Err(error) => return failure(selection, error, current),
    };

    let write = if current == mode && !needs_update {
        // Already in the requested mode and current: nothing to write.
        Ok(())
    } else if current == mode {
        backend.update(def)
    } else {
        backend.set(def, mode)
    };

    let mode_after = backend.current_mode(def);
    let (error, shown_mode) = match (write, mode_after) {
        (Err(error), after) => (Some(error), after.unwrap_or(current)),
        (Ok(()), Err(error)) => (Some(error), current),
        (Ok(()), Ok(after)) if after != mode => (
            Some(format!(
                "read-back mismatch: expected {} got {}",
                mode.as_str(),
                after.as_str()
            )),
            after,
        ),
        (Ok(()), Ok(after)) => (None, after),
    };
    ApplyResult {
        agent: selection.agent.clone(),
        requested: selection.mode.clone(),
        ok: error.is_none(),
        error,
        mode_after: shown_mode.as_str().to_string(),
        delivery: backend.delivery(def),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::HashMap;

    /// Scriptable backend: remembers calls, holds a mode per Agent and can fail or lie.
    #[derive(Default)]
    struct Fake {
        modes: RefCell<HashMap<&'static str, Mode>>,
        outdated: RefCell<Vec<&'static str>>,
        calls: RefCell<Vec<String>>,
        fail_set: Vec<&'static str>,
        /// `set` succeeds but leaves this mode behind instead of the requested one.
        wrong_after: HashMap<&'static str, Mode>,
        unreadable: Vec<&'static str>,
    }

    impl Backend for Fake {
        fn current_mode(&self, def: &AgentDef) -> Result<Mode, String> {
            if self.unreadable.contains(&def.id) {
                return Err("cannot read status".to_string());
            }
            Ok(*self.modes.borrow().get(def.id).unwrap_or(&Mode::None))
        }

        fn needs_update(&self, def: &AgentDef) -> Result<bool, String> {
            Ok(self.outdated.borrow().contains(&def.id))
        }

        fn set(&self, def: &AgentDef, mode: Mode) -> Result<(), String> {
            self.calls
                .borrow_mut()
                .push(format!("set {} {}", def.id, mode.as_str()));
            if self.fail_set.contains(&def.id) {
                return Err("disk full".to_string());
            }
            let landed = self.wrong_after.get(def.id).copied().unwrap_or(mode);
            self.modes.borrow_mut().insert(def.id, landed);
            Ok(())
        }

        fn update(&self, def: &AgentDef) -> Result<(), String> {
            self.calls.borrow_mut().push(format!("update {}", def.id));
            self.outdated.borrow_mut().retain(|id| *id != def.id);
            Ok(())
        }

        fn delivery(&self, def: &AgentDef) -> Option<String> {
            (def.id == "codex").then(|| "untrusted".to_string())
        }
    }

    fn select(agent: &str, mode: &str) -> Selection {
        Selection {
            agent: agent.to_string(),
            mode: mode.to_string(),
        }
    }

    #[test]
    fn a_fresh_agent_is_set_and_read_back() {
        let fake = Fake::default();
        let results = apply_with(&fake, &[select("cursor", "cli")]);
        assert_eq!(fake.calls.borrow().as_slice(), ["set cursor cli"]);
        assert!(results[0].ok, "{:?}", results[0]);
        assert_eq!(results[0].mode_after, "cli");
        assert_eq!(results[0].delivery, None);
    }

    #[test]
    fn a_failing_agent_does_not_stop_the_others() {
        let fake = Fake {
            fail_set: vec!["cursor"],
            ..Fake::default()
        };
        let results = apply_with(&fake, &[select("cursor", "cli"), select("claude", "cli")]);
        assert!(!results[0].ok);
        assert_eq!(results[0].error.as_deref(), Some("disk full"));
        assert_eq!(results[0].mode_after, "none");
        assert!(results[1].ok);
        assert_eq!(results[1].mode_after, "cli");
    }

    #[test]
    fn already_current_means_no_write() {
        let fake = Fake::default();
        fake.modes.borrow_mut().insert("claude", Mode::Cli);
        let results = apply_with(&fake, &[select("claude", "cli")]);
        assert!(fake.calls.borrow().is_empty());
        assert!(results[0].ok);
    }

    #[test]
    fn same_mode_but_outdated_runs_update_not_set() {
        let fake = Fake::default();
        fake.modes.borrow_mut().insert("claude", Mode::Cli);
        fake.outdated.borrow_mut().push("claude");
        let results = apply_with(&fake, &[select("claude", "cli")]);
        assert_eq!(fake.calls.borrow().as_slice(), ["update claude"]);
        assert!(results[0].ok);
    }

    #[test]
    fn switching_mode_runs_set() {
        let fake = Fake::default();
        fake.modes.borrow_mut().insert("cursor", Mode::Cli);
        let results = apply_with(&fake, &[select("cursor", "mcp")]);
        assert_eq!(fake.calls.borrow().as_slice(), ["set cursor mcp"]);
        assert_eq!(results[0].mode_after, "mcp");
    }

    #[test]
    fn a_read_back_that_disagrees_is_a_failure() {
        let fake = Fake {
            wrong_after: HashMap::from([("cursor", Mode::None)]),
            ..Fake::default()
        };
        let results = apply_with(&fake, &[select("cursor", "cli")]);
        assert!(!results[0].ok);
        assert_eq!(
            results[0].error.as_deref(),
            Some("read-back mismatch: expected cli got none")
        );
        assert_eq!(results[0].mode_after, "none");
    }

    #[test]
    fn unsupported_requests_never_reach_the_backend() {
        let fake = Fake::default();
        let results = apply_with(
            &fake,
            &[
                select("grok", "cli"),
                select("pi", "mcp"),
                select("cursor", "none"),
                select("cursor", "bogus"),
                select("nobody", "cli"),
            ],
        );
        assert!(fake.calls.borrow().is_empty());
        assert!(results.iter().all(|r| !r.ok && r.error.is_some()));
    }

    #[test]
    fn an_unreadable_status_blocks_only_that_agent() {
        let fake = Fake {
            unreadable: vec!["claude"],
            ..Fake::default()
        };
        let results = apply_with(&fake, &[select("claude", "cli"), select("cursor", "cli")]);
        assert!(!results[0].ok);
        assert_eq!(results[0].error.as_deref(), Some("cannot read status"));
        assert!(results[1].ok);
        assert!(!fake.calls.borrow().iter().any(|c| c.contains("claude")));
    }

    #[test]
    fn codex_reports_delivery_and_untrusted_is_not_a_failure() {
        let fake = Fake::default();
        let results = apply_with(&fake, &[select("codex", "mcp")]);
        assert!(results[0].ok);
        assert_eq!(results[0].delivery.as_deref(), Some("untrusted"));
    }
}
