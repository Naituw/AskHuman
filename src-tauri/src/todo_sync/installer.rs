//! Offline installation of the signed helper carried by the macOS npm / direct package.
//! Installation never touches todo data or CloudKit. Only an explicit enable / repair, or an
//! already-enabled daemon, may call this module (spec `todo-sync-release`).

use std::path::Path;
#[cfg(target_os = "macos")]
use std::path::PathBuf;

include!(concat!(env!("OUT_DIR"), "/todo-sync-resource.rs"));
pub const APP: &str = "AskHumanTodoSync.app";

pub fn supports_version(version: &str) -> bool {
    version
        .trim()
        .split('.')
        .next()
        .and_then(|v| v.parse::<u32>().ok())
        .is_some_and(|v| v >= 14)
}

pub fn supported_os() -> bool {
    #[cfg(target_os = "macos")]
    {
        static SUPPORTED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        *SUPPORTED.get_or_init(|| {
            std::process::Command::new("/usr/bin/sw_vers")
                .arg("-productVersion")
                .output()
                .ok()
                .filter(|v| v.status.success())
                .is_some_and(|v| supports_version(&String::from_utf8_lossy(&v.stdout)))
        })
    }
    #[cfg(not(target_os = "macos"))]
    false
}

pub fn bundled() -> bool {
    BUNDLED_HELPER.is_some()
}

#[cfg(any(target_os = "macos", test))]
fn should_replace(installed: &str, candidate: &str) -> bool {
    crate::update::compare_versions(installed, candidate) <= 0
}

#[cfg(any(target_os = "macos", test))]
fn validate_archive(archive: &Path) -> anyhow::Result<()> {
    let mut zip = zip::ZipArchive::new(std::fs::File::open(archive)?)?;
    let mut size = 0u64;
    for i in 0..zip.len() {
        let item = zip.by_index(i)?;
        let name = item.name()?;
        let path = Path::new(name.as_ref());
        if path.is_absolute()
            || path
                .components()
                .any(|c| !matches!(c, std::path::Component::Normal(_)))
            || !(name.starts_with(&format!("{APP}/")) || name.starts_with("__MACOSX/"))
        {
            anyhow::bail!("invalid path in the bundled sync component");
        }
        if item.unix_mode().is_some_and(|m| m & 0o170000 == 0o120000) {
            anyhow::bail!("unexpected symlink in the sync component");
        }
        size = size.saturating_add(item.size());
        if size > 100 * 1024 * 1024 {
            anyhow::bail!("sync component archive is too large");
        }
    }
    Ok(())
}

/// Pure release diagnostics: no state directories, helper process, or iCloud access.
pub fn resource_info() -> serde_json::Value {
    use sha2::{Digest, Sha256};
    serde_json::json!({
        "bundled": bundled(),
        "sha256": BUNDLED_HELPER.map(|bytes| format!("{:x}", Sha256::digest(bytes))),
        "version": env!("CARGO_PKG_VERSION"),
        "protocol": super::helper_client::PROTOCOL,
    })
}

/// A complete, validated app is already staged on the destination filesystem. Keep the backup
/// outside the temporary directory: a crash between renames must not let TempDir delete it.
#[cfg(any(target_os = "macos", test))]
fn replace_app<T>(
    staged: &Path,
    destination: &Path,
    register: impl FnOnce(&Path) -> anyhow::Result<T>,
) -> anyhow::Result<T> {
    use anyhow::Context;
    let backup = destination.with_file_name("AskHumanTodoSync.previous.app");
    recover(destination)?;
    let had_previous = destination.exists();
    if had_previous {
        if backup.exists() {
            std::fs::remove_dir_all(&backup).context("cannot remove the previous helper backup")?;
        }
        std::fs::rename(destination, &backup)
            .context("cannot back up the installed sync helper")?;
    }
    if let Err(error) = std::fs::rename(staged, destination) {
        if had_previous {
            std::fs::rename(&backup, destination)
                .context("cannot restore the sync helper after a failed move")?;
        }
        return Err(error).context("cannot publish the sync helper");
    }
    match register(destination) {
        Ok(outcome) => Ok(outcome),
        Err(error) => {
            std::fs::remove_dir_all(destination).context("cannot remove the failed sync helper")?;
            if had_previous {
                std::fs::rename(&backup, destination)
                    .context("cannot restore the previous sync helper")?;
            }
            Err(error)
        }
    }
}

#[cfg(any(target_os = "macos", test))]
fn recover(destination: &Path) -> anyhow::Result<()> {
    let backup = destination.with_file_name("AskHumanTodoSync.previous.app");
    if !destination.exists() && backup.exists() {
        std::fs::rename(backup, destination)?;
    }
    Ok(())
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Registration {
    pub ok: bool,
    pub needs_approval: bool,
    pub message: String,
}

#[cfg(target_os = "macos")]
mod mac {
    use super::*;
    use anyhow::{anyhow, bail, Context, Result};
    use sha2::{Digest, Sha256};
    use std::process::{Command, Output, Stdio};
    use std::time::{Duration, Instant};

    const BUNDLE: &str = "com.naituw.humaninloop.todosync";
    const CONTAINER: &str = "iCloud.com.naituw.humaninloop.todosync";

    /// No shell, and a stalled system tool cannot hang an enable or daemon forever. Pipe output to
    /// files so a child cannot deadlock on a full pipe while we enforce its deadline.
    fn run(command: &mut Command) -> Result<Output> {
        let stdout = tempfile::tempfile()?;
        let stderr = tempfile::tempfile()?;
        let mut child = command
            .stdin(Stdio::null())
            .stdout(stdout.try_clone()?)
            .stderr(stderr.try_clone()?)
            .spawn()?;
        let until = Instant::now() + Duration::from_secs(30);
        let status = loop {
            if let Some(status) = child.try_wait()? {
                break status;
            }
            if Instant::now() >= until {
                let _ = child.kill();
                let _ = child.wait();
                bail!("sync helper system operation timed out");
            }
            std::thread::sleep(Duration::from_millis(25));
        };
        use std::io::{Read, Seek, SeekFrom};
        let mut stdout = stdout;
        let mut stderr = stderr;
        stdout.seek(SeekFrom::Start(0))?;
        stderr.seek(SeekFrom::Start(0))?;
        let mut out = Vec::new();
        let mut err = Vec::new();
        stdout.take(1024 * 1024).read_to_end(&mut out)?;
        stderr.take(1024 * 1024).read_to_end(&mut err)?;
        Ok(Output {
            status,
            stdout: out,
            stderr: err,
        })
    }

    fn checked(command: &mut Command) -> Result<Output> {
        let out = run(command)?;
        if !out.status.success() {
            bail!(
                "sync helper operation failed: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            );
        }
        Ok(out)
    }

    fn plist(app: &Path, key: &str) -> Result<String> {
        let out = checked(
            Command::new("/usr/libexec/PlistBuddy")
                .args(["-c", &format!("Print :{key}")])
                .arg(app.join("Contents/Info.plist")),
        )?;
        Ok(String::from_utf8(out.stdout)?.trim().to_string())
    }

    fn verify(app: &Path, notarized: bool) -> Result<()> {
        let requirement = format!("=anchor apple generic and identifier \"{BUNDLE}\" and certificate 1[field.1.2.840.113635.100.6.2.6] exists and certificate leaf[field.1.2.840.113635.100.6.1.13] exists and certificate leaf[subject.OU] = \"{}\"", crate::update::EXPECTED_TEAM_ID);
        // Gatekeeper can accept Developer ID alone under local policy. Require notarization
        // explicitly without depending on Xcode's stapler being installed on the user's Mac.
        let requirement = if notarized {
            format!("{requirement} and notarized")
        } else {
            requirement
        };
        checked(
            Command::new("/usr/bin/codesign")
                .args(["--verify", "--deep", "--strict", "-R", &requirement])
                .arg(app),
        )?;
        if notarized {
            // Gatekeeper uses the stapled ticket. End users must not need xcrun or Xcode.
            checked(
                Command::new("/usr/sbin/spctl")
                    .args(["--assess", "--type", "execute"])
                    .arg(app),
            )?;
            if plist(app, "AskHumanTodoSyncProtocol")?
                != super::super::helper_client::PROTOCOL.to_string()
            {
                bail!("the bundled sync helper speaks an incompatible protocol");
            }
            if plist(app, "HelperContainerID")? != CONTAINER {
                bail!("incorrect sync helper container");
            }
            // Parse the actual signed entitlements, not a caller-supplied sidecar manifest.
            let signed = checked(
                Command::new("/usr/bin/codesign")
                    .args(["-d", "--entitlements", ":-"])
                    .arg(app),
            )?;
            let file = tempfile::NamedTempFile::new()?;
            std::fs::write(file.path(), signed.stdout)?;
            let field = |key: &str| -> Result<String> {
                let out = checked(
                    Command::new("/usr/libexec/PlistBuddy")
                        .args(["-c", &format!("Print :{key}")])
                        .arg(file.path()),
                )?;
                Ok(String::from_utf8(out.stdout)?.trim().to_string())
            };
            if field("com.apple.developer.icloud-container-environment")? != "Production"
                || !field("com.apple.developer.icloud-container-identifiers")?
                    .lines()
                    .any(|v| v.trim() == CONTAINER)
                || !field("com.apple.developer.icloud-services")?
                    .lines()
                    .any(|v| v.trim() == "CloudKit")
                || field("com.apple.developer.aps-environment")? != "production"
            {
                bail!("the bundled sync helper lacks Production CloudKit / push permissions");
            }
        }
        Ok(())
    }

    pub fn register(app: &Path) -> Result<Registration> {
        // Validate before executing a helper that can live in a user-writable Applications folder.
        verify(app, false)?;
        let out = run(Command::new(app.join("Contents/MacOS/AskHumanTodoSync")).arg("--register"))?;
        let needs_approval = out.status.code() == Some(3);
        if !out.status.success() && !needs_approval {
            bail!(
                "cannot register sync helper: {} {}",
                String::from_utf8_lossy(&out.stdout).trim(),
                String::from_utf8_lossy(&out.stderr).trim()
            );
        }
        Ok(Registration {
            ok: out.status.success(),
            needs_approval,
            message: String::from_utf8_lossy(&out.stdout).trim().to_string(),
        })
    }

    pub fn needs_approval(app: &Path) -> bool {
        // This is read only. A bad or missing signature simply leaves the repair action available.
        if verify(app, false).is_err() {
            return false;
        }
        run(Command::new(app.join("Contents/MacOS/AskHumanTodoSync")).arg("--login-item-status"))
            .ok()
            .and_then(|out| serde_json::from_slice::<serde_json::Value>(&out.stdout).ok())
            .is_some_and(|value| value["loginItem"] == "requiresApproval")
    }

    fn unpack_resource(bytes: &[u8], directory: &Path) -> Result<PathBuf> {
        let archive = directory.join("helper.zip");
        std::fs::write(&archive, bytes)?;
        validate_archive(&archive)?;
        checked(
            Command::new("/usr/bin/ditto")
                .args(["-x", "-k"])
                .arg(&archive)
                .arg(directory),
        )?;
        let app = directory.join(APP);
        verify(&app, true)?;
        if plist(&app, "CFBundleShortVersionString")? != env!("CARGO_PKG_VERSION") {
            bail!("the sync component does not match this AskHuman version; reinstall AskHuman");
        }
        Ok(app)
    }

    /// Verify what the executable actually carries, without installing, registering, or executing
    /// the helper. CI can validate the embedded ticket and permissions without an iCloud account.
    pub fn verify_resource() -> Result<()> {
        let bytes =
            BUNDLED_HELPER.ok_or_else(|| anyhow!("this build has no embedded sync component"))?;
        let temporary = tempfile::tempdir()?;
        unpack_resource(bytes, temporary.path())?;
        Ok(())
    }

    fn stop(app: &Path, state_dir: &Path) -> Result<()> {
        verify(app, false)?;
        checked(Command::new(app.join("Contents/MacOS/AskHumanTodoSync")).arg("--unregister"))?;
        let until = Instant::now() + Duration::from_secs(5);
        loop {
            if crate::file_lock::FileLock::try_exclusive(&state_dir.join("helper.lock"))?.is_some()
            {
                return Ok(());
            }
            if Instant::now() >= until {
                bail!("the previous sync helper did not stop; retry after it exits");
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    /// Reuse an existing self-use helper when a source build does not carry a release resource.
    /// Public release packages always carry the archive; no download or Apple-account mutation occurs.
    pub fn prepare(state_dir: &Path) -> Result<Registration> {
        if !super::super::service::supported() {
            bail!("iCloud todo sync requires macOS 14 or later, outside Dev Instances");
        }
        let _lock = crate::file_lock::FileLock::exclusive(&state_dir.join("helper-install.lock"))?;
        let home = std::env::var_os("HOME").ok_or_else(|| anyhow!("home directory unavailable"))?;
        let applications = PathBuf::from(home).join("Applications");
        let destination =
            super::super::service::helper_app_path().unwrap_or_else(|| applications.join(APP));
        let applications = destination
            .parent()
            .ok_or_else(|| anyhow!("invalid helper installation path"))?;
        std::fs::create_dir_all(applications)?;
        recover(&destination)?;
        let Some(bytes) = BUNDLED_HELPER else {
            let existing = super::super::service::helper_app_path().ok_or_else(|| {
                anyhow!("the sync component is missing; reinstall or update AskHuman")
            })?;
            return register(&existing);
        };
        let digest = format!("{:x}", Sha256::digest(bytes));
        let receipt = state_dir.join("helper-install.json");
        let previous = std::fs::read(&receipt)
            .ok()
            .and_then(|v| serde_json::from_slice::<serde_json::Value>(&v).ok());
        if destination.exists() && previous.as_ref().is_some_and(|v| v["sha256"] == digest) {
            return register(&destination);
        }
        let temp = tempfile::Builder::new()
            .prefix(".askhuman-todo-sync-")
            .tempdir_in(applications)?;
        // The archive must contain exactly the expected app, with no path traversal or symlinks.
        // Our helper has no frameworks or symlinks; reject them before invoking ditto.
        let staged = unpack_resource(bytes, temp.path())?;
        let version = plist(&staged, "CFBundleShortVersionString")?;
        if destination.exists() {
            verify(&destination, false)?;
            let installed_version = plist(&destination, "CFBundleShortVersionString")?;
            if !should_replace(&installed_version, &version) {
                return register(&destination);
            }
            if let Err(error) = stop(&destination, state_dir) {
                let _ = register(&destination);
                return Err(error);
            }
        }
        let result = replace_app(&staged, &destination, register);
        if result.is_err() && destination.exists() {
            let _ = register(&destination);
        }
        let outcome = result?;
        let tmp = state_dir.join(format!("helper-install-{}.tmp", uuid::Uuid::new_v4()));
        std::fs::write(
            &tmp,
            serde_json::to_vec(&serde_json::json!({ "sha256": digest, "version": version }))?,
        )?;
        std::fs::rename(tmp, receipt)
            .context("cannot save the sync helper installation receipt")?;
        Ok(outcome)
    }
}

#[cfg(target_os = "macos")]
pub use mac::{needs_approval, prepare, verify_resource};

#[cfg(not(target_os = "macos"))]
pub fn verify_resource() -> anyhow::Result<()> {
    anyhow::bail!("this platform has no embedded sync component")
}

#[cfg(not(target_os = "macos"))]
pub fn prepare(_state_dir: &Path) -> anyhow::Result<Registration> {
    anyhow::bail!("iCloud todo sync requires macOS 14 or later")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_requirement_does_not_raise_the_main_program_requirement() {
        for v in ["11.7", "12.0", "13.6", "", "invalid"] {
            assert!(!supports_version(v));
        }
        for v in ["14.0", "15.6.1\n", "27.2"] {
            assert!(supports_version(v));
        }
    }

    #[test]
    fn old_processes_cannot_downgrade_the_helper() {
        assert!(!should_replace("0.18.0", "0.17.0"));
        assert!(should_replace("0.1.0", "0.17.0"));
        assert!(should_replace("0.17.0", "0.17.0"));
    }

    #[test]
    fn release_archive_rejects_path_traversal_and_unrelated_files() {
        use std::io::Write;
        for name in [
            "../outside",
            "AskHumanTodoSync.app/../../outside",
            "/tmp/outside",
            "another.app/Contents/Info.plist",
        ] {
            let file = tempfile::NamedTempFile::new().unwrap();
            let mut writer = zip::ZipWriter::new(std::fs::File::create(file.path()).unwrap());
            writer
                .start_file(name, zip::write::SimpleFileOptions::default())
                .unwrap();
            writer.write_all(b"untrusted").unwrap();
            writer.finish().unwrap();
            assert!(validate_archive(file.path()).is_err(), "accepted {name}");
        }
    }

    #[test]
    fn release_archive_accepts_the_expected_app_and_apple_resource_metadata() {
        use std::io::Write;
        let file = tempfile::NamedTempFile::new().unwrap();
        let mut writer = zip::ZipWriter::new(std::fs::File::create(file.path()).unwrap());
        for name in [
            "AskHumanTodoSync.app/Contents/Info.plist",
            "__MACOSX/AskHumanTodoSync.app/._Contents",
        ] {
            writer
                .start_file(name, zip::write::SimpleFileOptions::default())
                .unwrap();
            writer.write_all(b"fixture").unwrap();
        }
        writer.finish().unwrap();
        validate_archive(file.path()).unwrap();
    }

    #[test]
    fn failed_registration_restores_the_old_app() {
        let temp = tempfile::tempdir().unwrap();
        let old = temp.path().join(APP);
        let staged = temp.path().join("staged.app");
        std::fs::create_dir(&old).unwrap();
        std::fs::write(old.join("version"), "old").unwrap();
        std::fs::create_dir(&staged).unwrap();
        std::fs::write(staged.join("version"), "new").unwrap();
        let result: anyhow::Result<()> =
            replace_app(&staged, &old, |_| anyhow::bail!("registration failed"));
        assert!(result.is_err());
        assert_eq!(std::fs::read_to_string(old.join("version")).unwrap(), "old");
    }

    #[test]
    fn pending_system_approval_commits_the_new_app_and_keeps_a_backup() {
        let temp = tempfile::tempdir().unwrap();
        let old = temp.path().join(APP);
        let staged = temp.path().join("staged.app");
        std::fs::create_dir(&old).unwrap();
        std::fs::write(old.join("version"), "old").unwrap();
        std::fs::create_dir(&staged).unwrap();
        std::fs::write(staged.join("version"), "new").unwrap();
        replace_app(&staged, &old, |_| Ok(())).unwrap();
        assert_eq!(std::fs::read_to_string(old.join("version")).unwrap(), "new");
        assert_eq!(
            std::fs::read_to_string(temp.path().join("AskHumanTodoSync.previous.app/version"))
                .unwrap(),
            "old"
        );
    }

    #[test]
    fn interrupted_publish_recovers_the_backup() {
        let temp = tempfile::tempdir().unwrap();
        let old = temp.path().join(APP);
        let backup = temp.path().join("AskHumanTodoSync.previous.app");
        std::fs::create_dir(&backup).unwrap();
        recover(&old).unwrap();
        assert!(old.is_dir());
        assert!(!backup.exists());
    }

    #[test]
    fn first_install_failure_does_not_leave_an_app() {
        let temp = tempfile::tempdir().unwrap();
        let destination = temp.path().join(APP);
        let staged = temp.path().join("staged.app");
        std::fs::create_dir(&staged).unwrap();
        let result: anyhow::Result<()> =
            replace_app(&staged, &destination, |_| anyhow::bail!("failed"));
        assert!(result.is_err());
        assert!(!destination.exists());
    }
}
