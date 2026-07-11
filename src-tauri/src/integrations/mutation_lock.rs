//! Cross-process serialization for integration artifact edits.

#[cfg(unix)]
use anyhow::Context;
use anyhow::Result;
#[cfg(unix)]
use std::fs::{File, OpenOptions};
#[cfg(unix)]
use std::os::fd::AsRawFd;

#[cfg(unix)]
pub struct IntegrationMutationLock {
    file: File,
}

#[cfg(not(unix))]
pub struct IntegrationMutationLock;

impl IntegrationMutationLock {
    pub fn acquire() -> Result<Self> {
        #[cfg(not(unix))]
        {
            return Ok(Self);
        }

        #[cfg(unix)]
        {
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
}

#[cfg(unix)]
impl Drop for IntegrationMutationLock {
    fn drop(&mut self) {
        unsafe {
            libc::flock(self.file.as_raw_fd(), libc::LOCK_UN);
        }
    }
}
