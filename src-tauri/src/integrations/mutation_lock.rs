//! Cross-process serialization for integration artifact edits.

use anyhow::{Context, Result};
use std::fs::{File, OpenOptions};
use std::os::fd::AsRawFd;

pub struct IntegrationMutationLock {
    file: File,
}

impl IntegrationMutationLock {
    pub fn acquire() -> Result<Self> {
        let path = crate::paths::integrations_lock_file();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .open(&path)
            .with_context(|| format!("failed to open {}", path.display()))?;
        let result = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) };
        if result != 0 {
            return Err(std::io::Error::last_os_error()).context("failed to lock integrations");
        }
        Ok(Self { file })
    }
}

impl Drop for IntegrationMutationLock {
    fn drop(&mut self) {
        unsafe {
            libc::flock(self.file.as_raw_fd(), libc::LOCK_UN);
        }
    }
}
