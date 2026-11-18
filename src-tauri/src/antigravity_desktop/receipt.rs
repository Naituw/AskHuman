//! Where the receipt of one task-creation operation lives. The operation ID names the file, so a
//! retry with the same ID finds what the first attempt got to.

use std::path::PathBuf;

/// `~/.askhuman/antigravity-tasks/`.
pub(super) fn dir() -> PathBuf {
    crate::paths::config_dir().join("antigravity-tasks")
}
