//! 轻量 UI 状态（`~/.askhuman/ui-state.json`）：与用户配置（config.json）分离的界面一次性
//! 标记，如「设置向导里被跳过的域」。字段增量演进（`serde(default)`），读失败视为默认值；
//! 写失败静默（丢标记的代价只是提示多显示一次）。

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// Setup-wizard decisions that cannot be derived from the machine's current state
/// (`docs/specs/onboarding.md` §4). Everything else the wizard shows is derived on demand.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct OnboardingDecisions {
    /// Domains the user explicitly skipped or dismissed: `"agents"`, `"remote"`, and
    /// `"phoneTasks"` (the wizard's offer to start Agent tasks from the phone or IM was declined;
    /// it only keeps a rerun of the wizard from asking again).
    pub skipped: Vec<String>,
    /// The user made a choice about the menu-bar / login-item behaviour (the wizard's "完成", an
    /// explicit `menuBarIcon` change, or the Settings "启用" button).
    pub background_decided: bool,
}

/// The domains that can appear in [`OnboardingDecisions::skipped`].
pub const SKIPPABLE_DOMAINS: [&str; 3] = ["agents", "remote", "phoneTasks"];

impl OnboardingDecisions {
    /// Drop unknown domains and duplicates (keeps first-seen order).
    pub fn normalize(&mut self) {
        let mut seen: Vec<String> = Vec::new();
        for domain in self.skipped.drain(..) {
            if SKIPPABLE_DOMAINS.contains(&domain.as_str()) && !seen.contains(&domain) {
                seen.push(domain);
            }
        }
        self.skipped = seen;
    }

    pub fn is_skipped(&self, domain: &str) -> bool {
        self.skipped.iter().any(|d| d == domain)
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct UiState {
    /// Legacy flag of the removed popup-footer IM tip. Read-only compatibility: it still counts
    /// as "remote answering skipped" (see `onboarding::state`), is preserved on every rewrite and
    /// is never written by new code.
    pub im_tip_dismissed: bool,
    /// Setup-wizard decisions.
    pub onboarding: OnboardingDecisions,
}

fn path() -> PathBuf {
    crate::paths::config_dir().join("ui-state.json")
}

pub fn load() -> UiState {
    load_at(&path())
}

pub fn save(state: &UiState) {
    save_at(&path(), state);
}

/// Load, mutate and save in one step. In-process callers are serialized by a static mutex; the
/// file is not locked across processes: every write is a low-frequency, user-triggered, single
/// field change (the same trade-off the legacy tip flag made).
pub fn update(f: impl FnOnce(&mut UiState)) {
    update_at(&path(), f);
}

static WRITE_LOCK: Mutex<()> = Mutex::new(());

fn load_at(path: &Path) -> UiState {
    let mut state: UiState = std::fs::read(path)
        .ok()
        .and_then(|data| serde_json::from_slice(&data).ok())
        .unwrap_or_default();
    state.onboarding.normalize();
    state
}

fn save_at(path: &Path, state: &UiState) {
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(data) = serde_json::to_vec_pretty(state) {
        let _ = std::fs::write(path, data);
    }
}

fn update_at(path: &Path, f: impl FnOnce(&mut UiState)) {
    let _guard = WRITE_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let mut state = load_at(path);
    f(&mut state);
    state.onboarding.normalize();
    save_at(path, &state);
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn legacy_file_with_only_the_old_flag_is_readable() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("ui-state.json");
        std::fs::write(&path, br#"{"imTipDismissed": true}"#).unwrap();
        let state = load_at(&path);
        assert!(state.im_tip_dismissed);
        assert!(state.onboarding.skipped.is_empty());
        assert!(!state.onboarding.background_decided);
    }

    #[test]
    fn missing_or_corrupt_file_reads_as_default() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("ui-state.json");
        let state = load_at(&path);
        assert!(!state.im_tip_dismissed);
        assert_eq!(state.onboarding, OnboardingDecisions::default());
        std::fs::write(&path, b"not json").unwrap();
        assert_eq!(load_at(&path).onboarding, OnboardingDecisions::default());
    }

    #[test]
    fn update_roundtrip_preserves_the_legacy_flag() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("ui-state.json");
        std::fs::write(&path, br#"{"imTipDismissed": true}"#).unwrap();
        update_at(&path, |s| {
            s.onboarding.skipped.push("remote".to_string());
            s.onboarding.background_decided = true;
        });
        let state = load_at(&path);
        assert!(state.im_tip_dismissed, "legacy flag survives a rewrite");
        assert_eq!(state.onboarding.skipped, vec!["remote".to_string()]);
        assert!(state.onboarding.background_decided);
        let raw = std::fs::read_to_string(&path).unwrap();
        assert!(raw.contains("\"onboarding\""));
        assert!(raw.contains("\"backgroundDecided\""));
    }

    #[test]
    fn skipped_drops_unknown_values_and_duplicates() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("ui-state.json");
        std::fs::write(
            &path,
            br#"{"onboarding": {"skipped": ["remote", "bogus", "agents", "remote"]}}"#,
        )
        .unwrap();
        let state = load_at(&path);
        assert_eq!(
            state.onboarding.skipped,
            vec!["remote".to_string(), "agents".to_string()]
        );
        assert!(state.onboarding.is_skipped("agents"));
        assert!(!state.onboarding.is_skipped("bogus"));
    }

    #[test]
    fn update_normalizes_what_the_closure_wrote() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("ui-state.json");
        update_at(&path, |s| {
            s.onboarding.skipped.push("agents".to_string());
            s.onboarding.skipped.push("agents".to_string());
            s.onboarding.skipped.push("nope".to_string());
        });
        assert_eq!(
            load_at(&path).onboarding.skipped,
            vec!["agents".to_string()]
        );
    }
}
