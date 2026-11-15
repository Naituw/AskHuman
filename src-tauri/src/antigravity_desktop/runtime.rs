//! The macOS side of the App route: where the Antigravity App is installed, whether it is running,
//! and starting it without taking the focus.

use anyhow::Result;
use std::path::{Path, PathBuf};

/// How the App's main process appears in `ps`. Its helper processes live under `Frameworks/` and
/// carry other names, so this matches the App itself only.
const APP_PROCESS_SUFFIX: &str = "/Antigravity.app/Contents/MacOS/Antigravity";

fn app_candidates(home: &Path) -> [PathBuf; 2] {
    [
        PathBuf::from("/Applications/Antigravity.app"),
        home.join("Applications/Antigravity.app"),
    ]
}

/// The installed App bundle, if there is one.
#[cfg(target_os = "macos")]
pub(crate) fn app_path() -> Option<PathBuf> {
    app_candidates(&crate::paths::integration_home())
        .into_iter()
        .find(|path| path.is_dir())
}

#[cfg(not(target_os = "macos"))]
pub(crate) fn app_path() -> Option<PathBuf> {
    None
}

pub(crate) fn app_installed() -> bool {
    app_path().is_some()
}

fn lists_the_app(processes: &str) -> bool {
    processes
        .lines()
        .any(|line| line.trim_end().ends_with(APP_PROCESS_SUFFIX))
}

/// Whether this user's App is running; `None` when the processes could not be listed.
#[cfg(target_os = "macos")]
pub(crate) fn app_running() -> Option<bool> {
    // `-x` without `-a` lists the current user's processes only, whatever their terminal.
    let output = std::process::Command::new("/bin/ps")
        .args(["-xo", "comm="])
        .output()
        .ok()?;
    if !output.status.success() || output.stdout.len() > 4 * 1024 * 1024 {
        return None;
    }
    Some(lists_the_app(&String::from_utf8_lossy(&output.stdout)))
}

#[cfg(not(target_os = "macos"))]
pub(crate) fn app_running() -> Option<bool> {
    None
}

/// Starts the App in the background (`open -g`: it does not come to the front). Only for an App
/// that is not running: opening a running one can make it open another window.
#[cfg(target_os = "macos")]
pub(crate) fn start_app() -> Result<()> {
    use anyhow::Context;
    let app = app_path().context("The Antigravity App is not installed")?;
    let status = std::process::Command::new("/usr/bin/open")
        .arg("-g")
        .arg(&app)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .context("Could not run open")?;
    if !status.success() {
        anyhow::bail!("Could not start the Antigravity App");
    }
    Ok(())
}

#[cfg(not(target_os = "macos"))]
pub(crate) fn start_app() -> Result<()> {
    anyhow::bail!("The Antigravity App route is only available on macOS")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_app_is_looked_for_in_both_applications_folders() {
        let paths = app_candidates(Path::new("/Users/someone"));
        assert_eq!(paths[0], Path::new("/Applications/Antigravity.app"));
        assert_eq!(
            paths[1],
            Path::new("/Users/someone/Applications/Antigravity.app")
        );
    }

    #[test]
    fn only_the_apps_own_process_counts_as_the_app_running() {
        let running = "\
/sbin/launchd
/Applications/Antigravity.app/Contents/MacOS/Antigravity
/Applications/Antigravity.app/Contents/Frameworks/Antigravity Helper (Renderer).app/Contents/MacOS/Antigravity Helper (Renderer)
";
        assert!(lists_the_app(running));
        assert!(lists_the_app(
            "/Users/someone/Applications/Antigravity.app/Contents/MacOS/Antigravity\n"
        ));
        // Helpers, the language server and an unrelated program are not the App.
        let only_others = "\
/Applications/Antigravity.app/Contents/Frameworks/Antigravity Helper (Renderer).app/Contents/MacOS/Antigravity Helper (Renderer)
/Applications/Antigravity.app/Contents/Resources/app/bin/language_server_macos_arm
/usr/local/bin/antigravity-notes
";
        assert!(!lists_the_app(only_others));
        assert!(!lists_the_app(""));
    }
}
