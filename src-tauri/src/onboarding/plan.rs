//! The "files that will change" preview shown before the wizard applies an integration.
//!
//! The list is a hand-maintained table of the files `agent_mode::set` writes for each
//! (Agent, mode). A guard test runs the real `set` against a throwaway HOME and compares what it
//! actually created or modified with this table, so the preview cannot drift from the writers.

use crate::integrations::agent_mode::Mode;
use crate::integrations::agent_rules::AgentTarget;
use crate::onboarding::agents::AgentDef;
use crate::paths;
use serde::Serialize;
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlannedFile {
    pub path: PathBuf,
    /// The file already exists, so applying modifies it; otherwise it is created.
    pub exists: bool,
}

/// Both modes' previews for one Agent (an unsupported mode has an empty list).
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlannedByMode {
    pub cli: Vec<PlannedFile>,
    pub mcp: Vec<PlannedFile>,
}

/// The files `agent_mode::set(target, mode)` writes, built from `paths::*` so they follow the
/// `HOME` / `CODEX_HOME` style overrides.
fn files_for(target: AgentTarget, mode: Mode) -> Vec<PathBuf> {
    match (target, mode) {
        (_, Mode::None) => Vec::new(),
        (AgentTarget::Cursor, Mode::Cli) => vec![
            paths::cursor_rule_file(),
            paths::cursor_hooks_json(),
            paths::cursor_hook_script(),
        ],
        (AgentTarget::Cursor, Mode::Mcp) => vec![
            paths::cursor_rule_file(),
            paths::cursor_hooks_json(),
            paths::cursor_mcp_json(),
        ],
        (AgentTarget::ClaudeCode, Mode::Cli) => vec![
            paths::claude_md(),
            paths::claude_settings_json(),
            paths::claude_hook_script(),
        ],
        (AgentTarget::ClaudeCode, Mode::Mcp) => vec![
            paths::claude_md(),
            paths::claude_settings_json(),
            paths::claude_json(),
        ],
        // Both Codex modes carry the protocol hook and enable hooks in `config.toml`; MCP mode
        // additionally writes its server entry into the same file.
        (AgentTarget::Codex, Mode::Cli | Mode::Mcp) => {
            vec![paths::codex_hooks_json(), paths::codex_config_toml()]
        }
        (AgentTarget::Grok, Mode::Mcp) => vec![
            paths::grok_skill_md(),
            paths::grok_config_toml(),
            paths::grok_hooks_json(),
        ],
        (AgentTarget::Grok, Mode::Cli) => Vec::new(),
        (AgentTarget::Pi, Mode::Cli) => vec![paths::pi_agents_md(), paths::pi_extension_file()],
        (AgentTarget::Pi, Mode::Mcp) => Vec::new(),
        (AgentTarget::Antigravity, Mode::Cli) => vec![paths::antigravity_rule_file()],
        (AgentTarget::Antigravity, Mode::Mcp) => vec![
            paths::antigravity_rule_file(),
            paths::antigravity_mcp_config_json(),
        ],
    }
}

/// Preview of what applying `mode` to `def` touches. Empty for a mode the Agent does not offer.
pub fn planned_files(def: &AgentDef, mode: Mode) -> Vec<PlannedFile> {
    if !def.supports(mode) {
        return Vec::new();
    }
    files_for(def.target, mode)
        .into_iter()
        .map(|path| PlannedFile {
            exists: path.exists(),
            path,
        })
        .collect()
}

pub fn planned_by_mode(def: &AgentDef) -> PlannedByMode {
    PlannedByMode {
        cli: planned_files(def, Mode::Cli),
        mcp: planned_files(def, Mode::Mcp),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::onboarding::agents;
    use std::collections::BTreeMap;
    use std::path::Path;

    /// Every regular file under `root` (skipping `skip`) with its content.
    fn snapshot(root: &Path, skip: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
        fn walk(dir: &Path, skip: &Path, out: &mut BTreeMap<PathBuf, Vec<u8>>) {
            let Ok(entries) = std::fs::read_dir(dir) else {
                return;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.starts_with(skip) {
                    continue;
                }
                match entry.file_type() {
                    Ok(kind) if kind.is_dir() => walk(&path, skip, out),
                    Ok(kind) if kind.is_file() => {
                        out.insert(path.clone(), std::fs::read(&path).unwrap_or_default());
                    }
                    _ => {}
                }
            }
        }
        let mut out = BTreeMap::new();
        walk(root, skip, &mut out);
        out
    }

    #[test]
    fn unsupported_modes_have_no_preview() {
        let grok = agents::find("grok").unwrap();
        assert!(planned_files(grok, Mode::Cli).is_empty());
        let pi = agents::find("pi").unwrap();
        assert!(planned_files(pi, Mode::Mcp).is_empty());
        assert!(planned_files(pi, Mode::None).is_empty());
        assert!(!planned_files(pi, Mode::Cli).is_empty());
    }

    #[test]
    fn every_supported_mode_has_a_preview() {
        for def in agents::all() {
            for mode in [Mode::Cli, Mode::Mcp] {
                assert_eq!(
                    !planned_files(def, mode).is_empty(),
                    def.supports(mode),
                    "{} {:?}",
                    def.id,
                    mode
                );
            }
        }
    }

    /// Runs the real `agent_mode::set` for every supported (Agent, mode) in a throwaway HOME and
    /// requires the set of created or modified files to equal the preview table.
    #[cfg(unix)]
    #[test]
    fn preview_matches_what_set_really_writes() {
        // Imported here, not at module level: this is the only unix-only user, and an unused
        // import fails clippy on Windows (`-D warnings`).
        use crate::integrations::agent_mode;
        use crate::onboarding::testutil;
        if !testutil::in_child() {
            testutil::run_in_child(module_path!(), "preview_matches_what_set_really_writes");
            return;
        }
        let home = crate::paths::integration_home();
        let state = crate::paths::config_dir();
        // Each pair starts from an empty HOME (everything but the AskHuman state directory).
        let reset = || {
            for entry in std::fs::read_dir(&home).unwrap().flatten() {
                if entry.path() == state {
                    continue;
                }
                let path = entry.path();
                if path.is_dir() {
                    let _ = std::fs::remove_dir_all(&path);
                } else {
                    let _ = std::fs::remove_file(&path);
                }
            }
        };
        let mut failures = Vec::new();
        for def in agents::available() {
            for mode in [Mode::Cli, Mode::Mcp] {
                if !def.supports(mode) {
                    continue;
                }
                reset();
                let before = snapshot(&home, &state);
                // No login shell and no Agent binary may run: both would leave files of their own.
                let written = crate::integrations::agent_launch::with_no_agent_installed(|| {
                    agent_mode::set(def.target, mode)
                });
                if let Err(error) = written {
                    failures.push(format!("{} {:?}: set failed: {error}", def.id, mode));
                    continue;
                }
                let after = snapshot(&home, &state);
                let changed: Vec<PathBuf> = after
                    .iter()
                    .filter(|(path, content)| before.get(*path) != Some(*content))
                    .map(|(path, _)| path.clone())
                    .collect();
                let mut expected: Vec<PathBuf> = planned_files(def, mode)
                    .into_iter()
                    .map(|file| file.path)
                    .collect();
                let mut actual = changed;
                expected.sort();
                actual.sort();
                if expected != actual {
                    failures.push(format!(
                        "{} {:?}:\n  planned {expected:#?}\n  written {actual:#?}",
                        def.id, mode
                    ));
                }
            }
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }
}
