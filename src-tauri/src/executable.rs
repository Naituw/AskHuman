//! Keep the installation entry distinct from the identity of the running image.
use std::{io, path::PathBuf};

#[cfg(target_os = "linux")]
static LAUNCH_PATH: std::sync::OnceLock<Result<PathBuf, String>> = std::sync::OnceLock::new();

/// Capture before dispatch: after an ELF exchange current_exe points at the retained image.
/// `launch_path` also lazily captures for tests and library callers; production calls this from
/// `main` before any potential exchange.
#[cfg(target_os = "linux")]
pub fn capture_launch_path() {
    LAUNCH_PATH.get_or_init(|| std::env::current_exe().map_err(|e| e.to_string()));
}

/// The original installation pathname, for disk watching and independent child launches.
pub fn launch_path() -> io::Result<PathBuf> {
    #[cfg(target_os = "linux")]
    {
        match LAUNCH_PATH.get_or_init(|| std::env::current_exe().map_err(|e| e.to_string())) {
            Ok(path) => Ok(path.clone()),
            Err(error) => Err(io::Error::other(error.clone())),
        }
    }
    #[cfg(not(target_os = "linux"))]
    std::env::current_exe()
}
