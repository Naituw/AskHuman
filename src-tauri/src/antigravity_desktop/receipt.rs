//! Where the receipt of one task-creation operation lives. The operation ID names the file, so a
//! retry with the same ID finds what the first attempt got to.

use std::path::PathBuf;

/// `~/.askhuman/antigravity-tasks/`.
pub(super) fn dir() -> PathBuf {
    crate::paths::config_dir().join("antigravity-tasks")
}

/// The receipt of operation `id`. The caller has checked that `id` is a UUID, so it can only name
/// a file inside [`dir`].
pub(super) fn path(id: &str) -> PathBuf {
    dir().join(format!("{id}.json"))
}
