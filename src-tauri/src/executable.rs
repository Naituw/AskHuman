//! Keep the installation entry distinct from the identity of the running image.
use std::{io, path::Path, path::PathBuf, process::Command};

/// A one-shot handoff between native AskHuman launchers, never a user configuration override.
pub(crate) const INTERNAL_LAUNCH_ENTRY_ENV: &str = "ASKHUMAN_INTERNAL_LAUNCH_ENTRY";

#[cfg(target_os = "linux")]
static LAUNCH_PATH: std::sync::OnceLock<Result<PathBuf, String>> = std::sync::OnceLock::new();

/// Capture before dispatch: after an ELF exchange current_exe points at the retained image.
/// `launch_path` also lazily captures for tests and library callers; production calls this from
/// `main` before any potential exchange.
#[cfg(target_os = "linux")]
pub fn capture_launch_path() {
    // Consume even an invalid handoff before GTK, Tokio, Dev re-exec, or external children exist.
    let inherited = take_launch_entry();
    let role = std::env::args_os().nth(1);
    LAUNCH_PATH.get_or_init(|| {
        std::env::current_exe()
            .map(|running| select_launch_path(running, role.as_deref(), inherited.as_deref()))
            .map_err(|e| e.to_string())
    });
}

#[cfg(any(target_os = "linux", test))]
fn take_launch_entry() -> Option<std::ffi::OsString> {
    let inherited = std::env::var_os(INTERNAL_LAUNCH_ENTRY_ENV);
    std::env::remove_var(INTERNAL_LAUNCH_ENTRY_ENV);
    inherited
}

#[cfg(any(target_os = "linux", test))]
fn select_launch_path(
    running: PathBuf,
    role: Option<&std::ffi::OsStr>,
    inherited: Option<&std::ffi::OsStr>,
) -> PathBuf {
    let internal_role = role.is_some_and(|role| {
        ["--popup", "--popup-host", "--gui-host"]
            .iter()
            .any(|expected| role == std::ffi::OsStr::new(expected))
    });
    if let Some(entry) = inherited.map(Path::new).filter(|entry| {
        internal_role
            && entry.is_absolute()
            && !entry
                .components()
                .any(|part| matches!(part, std::path::Component::ParentDir))
            && (*entry == running || retained_entry_matches(&running, entry))
    }) {
        entry.to_path_buf()
    } else {
        running
    }
}

#[cfg(any(target_os = "linux", test))]
fn retained_entry_matches(running: &Path, entry: &Path) -> bool {
    let Some(directory) = running.parent() else {
        return false;
    };
    let retained = directory
        .file_name()
        .and_then(|name| name.to_str())
        .and_then(|name| name.strip_prefix(".askhuman-install-"))
        .and_then(|suffix| suffix.split_once('-'))
        .is_some_and(|(pid, nonce)| {
            !pid.is_empty()
                && !nonce.is_empty()
                && pid.bytes().all(|byte| byte.is_ascii_digit())
                && nonce.bytes().all(|byte| byte.is_ascii_digit())
        });
    // The publisher retains every old image as AskHuman, even when the entry is a symlink to
    // an installation target with another basename. A different build at the entry is expected.
    retained
        && running.file_name() == Some(std::ffi::OsStr::new("AskHuman"))
        && entry.file_name().is_some()
        && entry.parent() == directory.parent()
}

/// Attach the canonical entry while choosing the running image independently in Command::new.
pub(crate) fn inherit_launch_entry(command: &mut Command) -> io::Result<()> {
    #[cfg(target_os = "linux")]
    set_launch_entry(command, &launch_path()?);
    #[cfg(not(target_os = "linux"))]
    clear_launch_context(command);
    Ok(())
}

/// An independent launch uses its explicit target as the child's canonical entry.
pub(crate) fn set_launch_entry(command: &mut Command, entry: &Path) {
    clear_launch_context(command);
    #[cfg(target_os = "linux")]
    command.env(INTERNAL_LAUNCH_ENTRY_ENV, entry);
    #[cfg(not(target_os = "linux"))]
    let _ = entry;
}

/// Dev re-exec chooses a different installation and must not carry another instance's context.
pub(crate) fn clear_launch_context(command: &mut Command) {
    command.env_remove(INTERNAL_LAUNCH_ENTRY_ENV);
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsStr;

    #[test]
    fn retained_popup_roles_preserve_the_entry_without_changing_the_image() {
        let root = std::env::temp_dir().join("askhuman-entry-test");
        let image = root.join(".askhuman-install-12-345").join("AskHuman");
        // Symlink destinations can have a different basename from the public AskHuman entry.
        let entry = root.join("custom-name");
        for role in ["--popup", "--popup-host", "--gui-host"] {
            assert_eq!(
                select_launch_path(
                    image.clone(),
                    Some(OsStr::new(role)),
                    Some(entry.as_os_str())
                ),
                entry
            );
        }
        assert_eq!(image.file_name(), Some(OsStr::new("AskHuman")));
    }

    #[test]
    fn public_roles_and_unrelated_entries_cannot_override_the_launch_path() {
        let root = std::env::temp_dir().join("askhuman-entry-test");
        let image = root.join(".askhuman-install-12-345").join("AskHuman");
        let entry = root.join("AskHuman");
        for role in [None, Some("mcp"), Some("--settings"), Some("question")] {
            assert_eq!(
                select_launch_path(image.clone(), role.map(OsStr::new), Some(entry.as_os_str())),
                image
            );
        }
        for invalid in [
            PathBuf::from("AskHuman"),
            root.join("elsewhere").join("AskHuman"),
            root.join("..").join("AskHuman"),
        ] {
            let chosen = select_launch_path(
                image.clone(),
                Some(OsStr::new("--popup-host")),
                Some(invalid.as_os_str()),
            );
            assert_eq!(chosen, image, "must not redirect to {}", invalid.display());
        }
        for name in [".askhuman-install-x-123", ".askhuman-install-12-", "other"] {
            let unrelated = root.join(name).join("AskHuman");
            assert_eq!(
                select_launch_path(
                    unrelated.clone(),
                    Some(OsStr::new("--popup")),
                    Some(entry.as_os_str())
                ),
                unrelated
            );
        }
    }

    #[test]
    fn dev_reexec_clears_context_without_changing_the_selected_binary() {
        let selected = std::env::temp_dir()
            .join("other-dev-instance")
            .join("AskHuman");
        let mut command = Command::new(&selected);
        command.env(INTERNAL_LAUNCH_ENTRY_ENV, "another installation");
        clear_launch_context(&mut command);
        assert_eq!(command.get_program(), selected.as_os_str());
        assert!(command
            .get_envs()
            .any(|(key, value)| { key == INTERNAL_LAUNCH_ENTRY_ENV && value.is_none() }));
    }

    #[cfg(unix)]
    #[test]
    fn launch_entry_preserves_non_utf8_installation_paths() {
        use std::os::unix::ffi::OsStringExt;
        let root =
            std::env::temp_dir().join(std::ffi::OsString::from_vec(b"ah-entry-\xff".to_vec()));
        let image = root.join(".askhuman-install-12-345").join("AskHuman");
        let entry = root.join("AskHuman");
        assert_eq!(
            select_launch_path(image, Some(OsStr::new("--popup")), Some(entry.as_os_str())),
            entry
        );
    }

    #[test]
    fn launch_context_is_consumed_before_descendant_launches() {
        const MODE: &str = "ASKHUMAN_LAUNCH_CONTEXT_TEST";
        const TEST: &str =
            "executable::tests::launch_context_is_consumed_before_descendant_launches";
        match std::env::var(MODE).as_deref() {
            Ok("child") => {
                assert_eq!(take_launch_entry(), Some("one-shot".into()));
                assert!(std::env::var_os(INTERNAL_LAUNCH_ENTRY_ENV).is_none());
                let output = Command::new(std::env::current_exe().unwrap())
                    .args(["--exact", TEST, "--nocapture"])
                    .env(MODE, "grandchild")
                    .output()
                    .unwrap();
                assert!(
                    output.status.success(),
                    "{}",
                    String::from_utf8_lossy(&output.stderr)
                );
            }
            Ok("grandchild") => assert!(std::env::var_os(INTERNAL_LAUNCH_ENTRY_ENV).is_none()),
            _ => {
                let output = Command::new(std::env::current_exe().unwrap())
                    .args(["--exact", TEST, "--nocapture"])
                    .env(MODE, "child")
                    .env(INTERNAL_LAUNCH_ENTRY_ENV, "one-shot")
                    .output()
                    .unwrap();
                assert!(
                    output.status.success(),
                    "{}",
                    String::from_utf8_lossy(&output.stderr)
                );
            }
        }
    }
}
