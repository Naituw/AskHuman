//! Cross-process serialization for integration artifact edits.

use anyhow::{Context, Result};

pub struct IntegrationMutationLock(crate::file_lock::FileLock);

/// The decision itself, apart from the process state it is read from.
///
/// A Dev instance shares the Agent configuration files with the everyday install, so it may only
/// change them from inside an isolated test profile. Outside one, a Dev window or a project tree
/// that requires a test profile is held back.
fn blocked(
    dev_context: bool,
    in_test_profile: bool,
    dev_window: bool,
    project_requires_test_profile: bool,
) -> bool {
    dev_context && !in_test_profile && (dev_window || project_requires_test_profile)
}

/// Whether this process must leave the everyday Agent integration alone.
///
/// Both the write path ([`IntegrationMutationLock::acquire`]) and the Settings page (which hides
/// the Agents tab) ask this one question, so what the page offers can never differ from what the
/// backend accepts.
pub fn everyday_changes_blocked() -> Result<bool, String> {
    if !crate::dev_instance::is_dev_instance_context() {
        return Ok(false);
    }
    let in_test_profile = crate::development::session::active()?.is_some();
    Ok(blocked(
        true,
        in_test_profile,
        std::env::var_os(crate::development::workbench::WINDOW_ENV).is_some(),
        crate::development::session::requires_test_profile(),
    ))
}

impl IntegrationMutationLock {
    pub fn acquire() -> Result<Self> {
        if everyday_changes_blocked().map_err(anyhow::Error::msg)? {
            return Err(anyhow::anyhow!("Dev cannot change everyday Agent integration; use an isolated test profile or the explicit system entry"));
        }
        let path = crate::paths::integrations_lock_file();
        crate::file_lock::FileLock::exclusive(&path)
            .map(Self)
            .with_context(|| format!("failed to lock {}", path.display()))
    }
}

#[cfg(test)]
mod tests {
    use super::blocked;

    #[test]
    fn the_everyday_install_is_never_blocked() {
        for in_test_profile in [false, true] {
            for dev_window in [false, true] {
                for project in [false, true] {
                    assert!(!blocked(false, in_test_profile, dev_window, project));
                }
            }
        }
    }

    #[test]
    fn a_dev_window_or_project_tree_is_blocked_outside_a_test_profile() {
        assert!(blocked(true, false, true, false));
        assert!(blocked(true, false, false, true));
        assert!(blocked(true, false, true, true));
    }

    #[test]
    fn a_test_profile_may_change_its_own_integration() {
        assert!(!blocked(true, true, true, true));
        assert!(!blocked(true, true, false, true));
    }

    #[test]
    fn a_plain_dev_process_outside_a_dev_window_is_not_held_back() {
        // Neither a Dev window nor a project-scoped Dev instance (no `workspace.json`).
        assert!(!blocked(true, false, false, false));
    }
}
