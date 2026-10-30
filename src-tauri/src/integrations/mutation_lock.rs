//! Cross-process serialization for integration artifact edits.

use anyhow::{Context, Result};

pub struct IntegrationMutationLock(crate::file_lock::FileLock);

impl IntegrationMutationLock {
    pub fn acquire() -> Result<Self> {
        if crate::dev_instance::is_dev_instance_context()
            && crate::development::session::active()
                .map_err(anyhow::Error::msg)?
                .is_none()
            && (std::env::var_os(crate::development::workbench::WINDOW_ENV).is_some()
                || crate::development::state::read()
                    .map_err(anyhow::Error::msg)?
                    .mode
                    == crate::development::state::RoutingMode::Explicit)
        {
            return Err(anyhow::anyhow!("Dev cannot change everyday Agent integration; use an isolated test profile or the explicit system entry"));
        }
        let path = crate::paths::integrations_lock_file();
        crate::file_lock::FileLock::exclusive(&path)
            .map(Self)
            .with_context(|| format!("failed to lock {}", path.display()))
    }
}
