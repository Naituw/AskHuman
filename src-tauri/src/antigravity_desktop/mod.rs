//! Starting Antigravity tasks from the phone, IM and the New Task window.
//!
//! Antigravity has two ways to start a task: `agy` in a terminal, and the desktop App. The App has
//! no public way to open a conversation in a project of our choosing, but it runs *sidecars*
//! (`~/.gemini/config/sidecars/<id>/`): processes the App starts for a project, with a fixed set
//! of granted actions and an `agentapi` that can create conversations in that project. This
//! module owns the route's configuration and status, the sidecars AskHuman manages for it, and the
//! task-creation operation (docs/specs/antigravity-deep-integration.md §12).

// The route is built bottom-up over several commits and wired into the entry points (phone, IM,
// New Task window, Settings) in the last one, which removes this attribute.
#![allow(dead_code, unused_imports)]

mod host_config;
mod launch;
mod receipt;
mod runtime;
mod sidecar;

pub use launch::{launch_status, resolve_launch, LaunchStatus, UnavailableReason};

use serde::{Deserialize, Serialize};

/// The same choice of route as Codex's and Cursor's, so IM and iOS carry one kind of target for
/// every Agent: `Desktop` is the App, `Terminal` is `agy`.
pub use crate::codex_desktop::LaunchTarget;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Config {
    /// Which route a new Antigravity task tries first; the other one is the fallback.
    pub launch_preference: LaunchTarget,
    /// Whether AskHuman may create, change and remove the sidecars the App route needs, and the
    /// `sidecars.askhuman-*` entries of Antigravity's `config.json` that enable them. Off until the
    /// user allows it once; turning it off again does not remove what already exists.
    pub manage_sidecars: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            launch_preference: LaunchTarget::Desktop,
            manage_sidecars: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_is_managed_until_the_user_allows_it_and_the_app_goes_first() {
        let config = Config::default();
        assert!(!config.manage_sidecars);
        assert_eq!(config.launch_preference, LaunchTarget::Desktop);
    }

    #[test]
    fn a_config_written_before_the_route_existed_reads_as_the_defaults() {
        let config: Config = serde_json::from_str("{}").unwrap();
        assert!(!config.manage_sidecars);
        assert_eq!(config.launch_preference, LaunchTarget::Desktop);
        let value = serde_json::to_value(Config {
            launch_preference: LaunchTarget::Terminal,
            manage_sidecars: true,
        })
        .unwrap();
        assert_eq!(value["launchPreference"], "terminal");
        assert_eq!(value["manageSidecars"], true);
    }
}
