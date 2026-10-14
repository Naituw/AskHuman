use super::{ProcessStatus, Status};
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{
    ffi::OsString,
    fs,
    os::unix::{
        fs::{MetadataExt, PermissionsExt},
        process::CommandExt,
    },
    path::{Path, PathBuf},
    time::Duration,
};

const PRELOAD: &str = include_str!("preload.cjs");
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Entry {
    path: PathBuf,
    target: PathBuf,
    wrapper: String,
}

fn dir() -> PathBuf {
    crate::paths::state_dir().join("cursor-cli")
}
fn now() -> u64 {
    chrono::Utc::now().timestamp_millis().max(0) as u64
}
fn revision() -> String {
    format!("{:x}", Sha256::digest(PRELOAD))[..16].into()
}
fn alive(pid: u32) -> bool {
    pid > 0 && pid <= i32::MAX as u32 && unsafe { libc::kill(pid as i32, 0) } == 0
}
fn private_dir(path: &Path) -> Result<()> {
    fs::create_dir_all(path)?;
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_dir() || metadata.uid() != unsafe { libc::getuid() } {
        bail!("Cursor CLI state directory is not owned by the current user");
    }
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    Ok(())
}
fn atomic(path: &Path, data: &[u8], mode: u32) -> Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let temp = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(mode)
        .open(&temp)?;
    let result = (|| {
        file.write_all(data)?;
        file.sync_all()?;
        fs::rename(&temp, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(temp);
    }
    result?;
    fs::File::open(path.parent().context("Invalid atomic file parent")?)?.sync_all()?;
    Ok(())
}
fn read<T: serde::de::DeserializeOwned>(path: &Path) -> Option<T> {
    let meta = fs::symlink_metadata(path).ok()?;
    if !meta.is_file() || meta.len() > 128 * 1024 || meta.uid() != unsafe { libc::getuid() } {
        return None;
    }
    serde_json::from_slice(&fs::read(path).ok()?).ok()
}
fn entries() -> Vec<Entry> {
    read(&dir().join("entries.json")).unwrap_or_default()
}
fn save_entries(value: &[Entry]) -> Result<()> {
    atomic(
        &dir().join("entries.json"),
        &serde_json::to_vec(value)?,
        0o600,
    )
}
fn owned(entry: &Entry) -> bool {
    fs::symlink_metadata(&entry.path).is_ok_and(|m| {
        m.is_file() && m.len() == entry.wrapper.len() as u64 && m.uid() == unsafe { libc::getuid() }
    }) && fs::read_to_string(&entry.path).is_ok_and(|v| v == entry.wrapper)
}
fn verified(entry: &Path) -> Result<PathBuf> {
    let root = dirs::home_dir()
        .context("No user home")?
        .join(".local/share/cursor-agent/versions");
    verified_in(entry, &root)
}
fn verified_in(entry: &Path, root: &Path) -> Result<PathBuf> {
    let root = fs::canonicalize(root)?;
    let target = fs::canonicalize(entry)?;
    let version = target.parent().context("Invalid Cursor CLI target")?;
    if version.parent() != Some(root.as_path())
        || target.file_name().is_none_or(|name| name != "cursor-agent")
    {
        bail!("Entry does not target a supported Cursor CLI installation");
    }
    for file in [&target, &version.join("node"), &version.join("index.js")] {
        let meta = fs::metadata(file)?;
        if !meta.is_file()
            || meta.uid() != unsafe { libc::getuid() }
            || meta.permissions().mode() & 0o022 != 0
        {
            bail!("Cursor CLI installation has unsafe ownership or permissions");
        }
    }
    if fs::metadata(&target)?.len() > 16 * 1024 {
        bail!("Unknown Cursor CLI launcher size");
    }
    let script = fs::read_to_string(&target)?;
    if script.len() > 16 * 1024
        || !script.contains("CURSOR_INVOKED_AS")
        || !script.contains("NODE_BIN=\"$SCRIPT_DIR/node\"")
        || !script.contains("$SCRIPT_DIR/index.js")
    {
        bail!("Unknown Cursor CLI launcher contract");
    }
    Ok(target)
}
fn candidates() -> PathBuf {
    if crate::paths::config_dir() != dirs::home_dir().unwrap_or_default().join(".askhuman") {
        crate::paths::config_dir().join("cursor-cli-bin")
    } else {
        dirs::home_dir().unwrap_or_default().join(".local/bin")
    }
}
fn quote(path: &Path) -> Result<String> {
    let value = path
        .to_str()
        .context("Cursor CLI wrapper path is not UTF-8")?;
    Ok(format!("'{}'", value.replace('\'', "'\\''")))
}
fn wrapper(path: &Path) -> Result<String> {
    wrapper_for(path, &crate::paths::config_dir(), &std::env::current_exe()?)
}
fn wrapper_for(path: &Path, home: &Path, executable: &Path) -> Result<String> {
    Ok(format!(
        "#!/bin/sh\n# AskHuman managed Cursor CLI entry\nexec {} --cursor-cli-run {} {} \"$@\"\n",
        quote(executable)?,
        quote(home)?,
        quote(path)?
    ))
}
fn processes(cleanup: bool) -> Vec<ProcessStatus> {
    let mut result = Vec::new();
    if let Ok(files) = fs::read_dir(dir().join("processes")) {
        for file in files.flatten().take(256) {
            if let Some(mut status) = read::<ProcessStatus>(&file.path()) {
                if !alive(status.pid) {
                    if cleanup {
                        let _ = fs::remove_file(file.path());
                    }
                    continue;
                }
                if now().saturating_sub(status.updated_at) > 6000 {
                    status.phase = "unresponsive".into();
                }
                result.push(status);
            }
        }
    }
    result
}
pub(super) fn needs_cleanup() -> bool {
    (dir().join("entries.json").exists()
        && read::<Vec<Entry>>(&dir().join("entries.json")).is_none())
        || !entries().is_empty()
        || processes(false).iter().any(|p| p.active)
}
fn phase(
    enabled: bool,
    entries: &[String],
    processes: &[ProcessStatus],
    error: bool,
) -> &'static str {
    if error {
        "error"
    } else if !enabled && processes.iter().any(|p| p.active) {
        "detaching"
    } else if !enabled {
        "disabled"
    } else if processes
        .iter()
        .any(|p| p.phase == "unresponsive" || p.error.is_some())
    {
        "error"
    } else if entries.is_empty() && processes.iter().any(|p| p.active) {
        "partial"
    } else if !processes.is_empty() && processes.iter().all(|p| p.active) {
        "active"
    } else if processes.iter().any(|p| p.active) {
        "partial"
    } else if !processes.is_empty() {
        "starting"
    } else if entries.is_empty() {
        "missing"
    } else {
        "ready"
    }
}
pub(super) fn status() -> Status {
    let enabled = crate::cursor_desktop::active(
        &crate::config::AppConfig::load_without_secrets().cursor_desktop,
    );
    let mut saved: Status = read(&dir().join("status.json")).unwrap_or_default();
    saved.supported = true;
    saved.enabled = enabled;
    saved.processes = processes(false);
    saved.entries = entries()
        .iter()
        .filter(|entry| owned(entry))
        .map(|entry| entry.path.to_string_lossy().into_owned())
        .collect();
    saved.phase =
        if now().saturating_sub(saved.updated_at) > 6000 && (enabled || needs_cleanup()) {
            "offline"
        } else {
            phase(
                enabled,
                &saved.entries,
                &saved.processes,
                saved.error.is_some(),
            )
        }
        .into();
    saved
}
fn restore_owned(entry: &Entry) -> Result<()> {
    if !owned(entry) {
        return Ok(());
    }
    let temp = entry
        .path
        .with_extension(format!("{}.restore", uuid::Uuid::new_v4()));
    let result = std::os::unix::fs::symlink(&entry.target, &temp)
        .and_then(|_| fs::rename(&temp, &entry.path));
    if result.is_err() {
        let _ = fs::remove_file(temp);
    }
    result?;
    fs::File::open(entry.path.parent().context("Invalid entry parent")?)?.sync_all()?;
    Ok(())
}
fn reconcile(enabled: bool) -> Result<Vec<String>> {
    private_dir(&dir())?;
    private_dir(&dir().join("processes"))?;
    let preload = dir().join(format!("preload-{}.cjs", revision()));
    if fs::read_to_string(&preload).ok().as_deref() != Some(PRELOAD) {
        atomic(&preload, PRELOAD.as_bytes(), 0o600)?;
    }
    // Publish disable before restoring entry points. Existing sessions then restore independently.
    atomic(
        &dir().join("policy.json"),
        &serde_json::to_vec(&json!({"enabled":enabled}))?,
        0o600,
    )?;
    if dir().join("entries.json").exists()
        && read::<Vec<Entry>>(&dir().join("entries.json")).is_none()
    {
        bail!("Cursor CLI recovery manifest is unreadable; retaining it for repair");
    }
    let mut managed = entries();
    let mut errors = Vec::new();
    let mut keep = Vec::new();
    for entry in managed.drain(..) {
        if enabled && owned(&entry) {
            let target = if entry.target.is_absolute() {
                entry.target.clone()
            } else {
                entry
                    .path
                    .parent()
                    .context("Invalid entry")?
                    .join(&entry.target)
            };
            match verified(&target) {
                Ok(_) => {
                    keep.push(entry);
                    continue;
                }
                Err(error) => errors.push(error.to_string()),
            }
        }
        if let Err(error) = restore_owned(&entry) {
            errors.push(error.to_string());
            keep.push(entry);
        }
        // A replaced/deleted entry is no longer ours; never resurrect or overwrite it.
    }
    save_entries(&keep)?;
    if enabled {
        for name in ["cursor-agent", "agent"] {
            let path = candidates().join(name);
            if keep.iter().any(|entry| entry.path == path) || fs::read_link(&path).is_err() {
                continue;
            }
            if let Err(error) = verified(&path) {
                let root = dirs::home_dir()
                    .unwrap_or_default()
                    .join(".local/share/cursor-agent/versions");
                if fs::canonicalize(&path).is_ok_and(|target| target.starts_with(&root)) {
                    errors.push(format!("{name}: {error}"));
                }
                continue;
            }
            let entry = Entry {
                target: fs::read_link(&path)?,
                wrapper: wrapper(&path)?,
                path,
            };
            keep.push(entry.clone());
            save_entries(&keep)?;
            // Check again after writing recovery metadata, before replacing the verified symlink.
            if fs::read_link(&entry.path).ok().as_ref() != Some(&entry.target) {
                keep.pop();
                save_entries(&keep)?;
                continue;
            }
            if let Err(error) = atomic(&entry.path, entry.wrapper.as_bytes(), 0o755) {
                errors.push(error.to_string());
            }
        }
    }
    if !errors.is_empty() {
        bail!("{}", errors.join("; "));
    }
    Ok(keep
        .iter()
        .filter(|entry| owned(entry))
        .map(|entry| entry.path.to_string_lossy().into_owned())
        .collect())
}
pub(super) async fn run() {
    loop {
        let enabled = crate::cursor_desktop::active(
            &crate::config::AppConfig::load_without_secrets().cursor_desktop,
        );
        if enabled || needs_cleanup() || dir().exists() {
            let result = reconcile(enabled);
            let processes = processes(true);
            let entries = entries()
                .iter()
                .filter(|entry| owned(entry))
                .map(|entry| entry.path.to_string_lossy().into_owned())
                .collect::<Vec<_>>();
            let error = result.err().map(|error| error.to_string());
            let phase = phase(enabled, &entries, &processes, error.is_some());
            let value = Status {
                supported: true,
                enabled,
                phase: phase.into(),
                entries,
                processes,
                error,
                updated_at: now(),
            };
            let _ = atomic(
                &dir().join("status.json"),
                &serde_json::to_vec(&value).unwrap_or_default(),
                0o600,
            );
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

fn interactive(args: &[OsString]) -> bool {
    let commands = [
        "persist",
        "install-shell-integration",
        "uninstall-shell-integration",
        "login",
        "logout",
        "mcp",
        "plugin",
        "worker",
        "status",
        "whoami",
        "models",
        "team",
        "bedrock",
        "about",
        "update",
        "create-chat",
        "generate-rule",
        "rule",
        "ls",
        "help",
    ];
    let value_options = [
        "--model",
        "-m",
        "--workspace",
        "--directory",
        "--api-key",
        "--mode",
        "--worktree-base",
        "--browser",
        "--sandbox",
        "--header",
        "-H",
        "--endpoint",
        "-e",
        "--add-dir",
        "--plugin-dir",
    ];
    let mut value = false;
    let mut first_positional = true;
    for (index, arg) in args.iter().enumerate() {
        let Some(arg) = arg.to_str() else {
            return false;
        };
        if value {
            value = false;
            continue;
        }
        if [
            "-p",
            "--print",
            "--acp",
            "--headless",
            "--help",
            "-h",
            "--version",
            "-v",
            "--list-models",
            "--output-format",
        ]
        .contains(&arg)
            || arg.starts_with("--print=")
            || arg.starts_with("--output-format=")
            || arg.starts_with("--acp=")
            || (arg.starts_with("-p") && !arg.starts_with("--"))
        {
            return false;
        }
        if arg == "--" {
            break;
        }
        if value_options.contains(&arg)
            || (["--resume", "-r", "--worktree", "-w"].contains(&arg)
                && args
                    .get(index + 1)
                    .is_some_and(|next| !next.to_string_lossy().starts_with('-')))
        {
            value = true;
        } else if !arg.starts_with('-') && first_positional {
            first_positional = false;
            if commands.contains(&arg) {
                return false;
            }
        }
    }
    !value
}
pub(super) fn launch(args: &[OsString]) -> Result<()> {
    let owner_home = args.first().context("Missing Cursor CLI owner")?;
    let previous_home = std::env::var_os(crate::dev_instance::ASKHUMAN_HOME_ENV);
    std::env::set_var(crate::dev_instance::ASKHUMAN_HOME_ENV, owner_home);
    let args = args.get(1..).context("Missing Cursor CLI arguments")?;
    let entry_path = args.first().context("Missing managed Cursor CLI entry")?;
    let entry = entries()
        .into_iter()
        .find(|entry| entry.path.as_os_str() == entry_path)
        .context("Managed Cursor CLI entry no longer exists")?;
    let target = if entry.target.is_absolute() {
        entry.target.clone()
    } else {
        entry
            .path
            .parent()
            .context("Invalid entry")?
            .join(&entry.target)
    };
    let native = fs::canonicalize(&target).context("Original Cursor CLI target is unavailable")?;
    let supported = verified(&target);
    let backup_dir = dir().join("original");
    private_dir(&backup_dir)?;
    let original = backup_dir.join(entry.path.file_name().context("Invalid entry name")?);
    if fs::read_link(&original).ok().as_ref() != Some(&native) {
        let temp = backup_dir.join(format!("{}.tmp", uuid::Uuid::new_v4()));
        std::os::unix::fs::symlink(&native, &temp)?;
        fs::rename(temp, &original)?;
    }
    let mut command = std::process::Command::new(&original);
    command.args(&args[1..]);
    if let Some(home) = previous_home {
        command.env(crate::dev_instance::ASKHUMAN_HOME_ENV, home);
    } else {
        command.env_remove(crate::dev_instance::ASKHUMAN_HOME_ENV);
    }
    if supported.is_err() {
        // A vendor launcher update must not make the user's command unusable.
        let _ = restore_owned(&entry);
        eprintln!("AskHuman: unsupported Cursor CLI launcher; starting without protection");
        unsafe {
            libc::signal(libc::SIGPIPE, libc::SIG_DFL);
        }
        return Err(command.exec()).context("Unable to launch Cursor CLI");
    }
    if {
        use std::io::IsTerminal;
        std::io::stdin().is_terminal() && std::io::stdout().is_terminal()
    } && interactive(&args[1..])
        && crate::cursor_desktop::active(
            &crate::config::AppConfig::load_without_secrets().cursor_desktop,
        )
    {
        // A wrapper may outlive a daemon crash. Reuse the normal serialized startup protocol.
        let service = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?
            .block_on(crate::client::ensure_running());
        let preload = dir().join(format!("preload-{}.cjs", revision()));
        for _ in 0..60 {
            if preload.is_file()
                && read::<serde_json::Value>(&dir().join("policy.json"))
                    .is_some_and(|value| value["enabled"] == true)
            {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        if service.is_err()
            || !preload.is_file()
            || !read::<serde_json::Value>(&dir().join("policy.json"))
                .is_some_and(|value| value["enabled"] == true)
        {
            eprintln!("AskHuman: Cursor CLI protection unavailable; starting the native CLI");
            unsafe {
                libc::signal(libc::SIGPIPE, libc::SIG_DFL);
            }
            return Err(command.exec()).context("Unable to launch Cursor CLI");
        }
        let previous = std::env::var("NODE_OPTIONS").ok();
        let preload_arg = format!("--require {}", serde_json::to_string(&preload)?);
        let node_options = previous.as_ref().map_or(preload_arg.clone(), |value| {
            format!("{value} {preload_arg}")
        });
        let boot = json!({"index":native.parent().context("Invalid vendor target")?.join("index.js"), "policy":dir().join("policy.json"), "status":dir().join("processes").join(format!("{}.json", uuid::Uuid::new_v4())), "revision":revision()});
        command
            .env("NODE_OPTIONS", node_options)
            .env("ASKHUMAN_CURSOR_CLI_BOOT", boot.to_string());
        if let Some(previous) = previous {
            command.env("ASKHUMAN_CURSOR_CLI_NODE_OPTIONS", previous);
        } else {
            command.env_remove("ASKHUMAN_CURSOR_CLI_NODE_OPTIONS");
        }
    }
    // Restore conventional Rust SIGPIPE handling before replacing the process with the vendor CLI.
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }
    Err(command.exec()).context("Unable to launch Cursor CLI")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mode_classifier_excludes_commands_and_headless_values() {
        let args = |values: &[&str]| values.iter().map(OsString::from).collect::<Vec<_>>();
        for values in [
            vec![],
            vec!["--model", "grok-4.7-low"],
            vec!["--resume", "abc"],
            vec!["--resume"],
            vec!["--resume", "--force"],
            vec!["--approve-mcps"],
            vec!["--worktree"],
            vec!["--trust", "--force"],
            vec!["agent", "hello"],
            vec!["hello"],
            vec!["--", "--print"],
        ] {
            assert!(interactive(&args(&values)));
        }
        for values in [
            vec!["--print"],
            vec!["-p", "hello"],
            vec!["--output-format=json"],
            vec!["mcp", "list"],
            vec!["--model"],
        ] {
            assert!(!interactive(&args(&values)));
        }
    }
    #[test]
    fn atomic_wrapper_preserves_literal_paths_and_original_symlink() {
        let root = tempfile::tempdir().unwrap();
        let entry = root.path().join("cursor-agent");
        std::os::unix::fs::symlink("vendor ' with spaces", &entry).unwrap();
        let value = Entry {
            path: entry.clone(),
            target: fs::read_link(&entry).unwrap(),
            wrapper: "#!/bin/sh\nexit 17\n".into(),
        };
        atomic(&entry, value.wrapper.as_bytes(), 0o755).unwrap();
        assert!(owned(&value));
        assert_eq!(
            fs::metadata(&entry).unwrap().permissions().mode() & 0o777,
            0o755
        );
        fs::write(&entry, "foreign").unwrap();
        assert!(!owned(&value));
        assert_eq!(quote(Path::new("a'b $(x)")).unwrap(), "'a'\\''b $(x)'");
    }

    #[test]
    fn restore_keeps_foreign_files_and_deletions_and_recovers_relative_links() {
        let root = tempfile::tempdir().unwrap();
        let entry = Entry {
            path: root.path().join("agent"),
            target: PathBuf::from("../vendor/cursor-agent"),
            wrapper: "managed".into(),
        };
        atomic(&entry.path, entry.wrapper.as_bytes(), 0o700).unwrap();
        restore_owned(&entry).unwrap();
        assert_eq!(fs::read_link(&entry.path).unwrap(), entry.target);
        fs::remove_file(&entry.path).unwrap();
        restore_owned(&entry).unwrap();
        assert!(!entry.path.exists());
        fs::write(&entry.path, "foreign").unwrap();
        restore_owned(&entry).unwrap();
        assert_eq!(fs::read_to_string(&entry.path).unwrap(), "foreign");
    }
    #[test]
    fn wrapper_exec_preserves_literal_arguments_cwd_and_exit_status() {
        let root = tempfile::tempdir().unwrap();
        let executable = root.path().join("AskHuman ' literal");
        atomic(&executable, b"#!/bin/sh\n[ \"$1\" = --cursor-cli-run ] || exit 9\n[ \"$4\" = 'literal $(false)' ] || exit 10\nexit 17\n", 0o700).unwrap();
        let entry = root.path().join("cursor-agent");
        let content = wrapper_for(&entry, root.path(), &executable).unwrap();
        atomic(&entry, content.as_bytes(), 0o700).unwrap();
        let result = std::process::Command::new(&entry)
            .arg("literal $(false)")
            .current_dir(root.path())
            .status()
            .unwrap();
        assert_eq!(result.code(), Some(17));
    }

    #[test]
    fn status_never_calls_an_empty_or_partial_process_set_fully_active() {
        let entries = vec!["cursor-agent".into()];
        assert_eq!(phase(true, &entries, &[], false), "ready");
        let active = ProcessStatus {
            active: true,
            ..ProcessStatus::default()
        };
        let starting = ProcessStatus::default();
        assert_eq!(
            phase(true, &entries, &[active.clone(), starting], false),
            "partial"
        );
        assert_eq!(
            phase(true, &[], std::slice::from_ref(&active), false),
            "partial"
        );
        assert_eq!(phase(false, &[], &[active], false), "detaching");
        assert_eq!(phase(false, &[], &[], false), "disabled");
    }

    #[test]
    fn vendor_verification_rejects_foreign_paths_and_changed_launchers() {
        let root = tempfile::tempdir().unwrap();
        let version = root.path().join("v1");
        fs::create_dir(&version).unwrap();
        let launcher = version.join("cursor-agent");
        atomic(
            &launcher,
            b"CURSOR_INVOKED_AS=x\nNODE_BIN=\"$SCRIPT_DIR/node\"\n$SCRIPT_DIR/index.js\n",
            0o755,
        )
        .unwrap();
        atomic(&version.join("node"), b"node", 0o755).unwrap();
        atomic(&version.join("index.js"), b"index", 0o644).unwrap();
        assert_eq!(
            verified_in(&launcher, root.path()).unwrap(),
            fs::canonicalize(&launcher).unwrap()
        );
        assert!(verified_in(&launcher, &root.path().join("foreign")).is_err());
        fs::set_permissions(version.join("node"), fs::Permissions::from_mode(0o777)).unwrap();
        assert!(verified_in(&launcher, root.path()).is_err());
        fs::set_permissions(version.join("node"), fs::Permissions::from_mode(0o755)).unwrap();
        fs::write(&launcher, "unknown vendor contract").unwrap();
        assert!(verified_in(&launcher, root.path()).is_err());
    }
}
