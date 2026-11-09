//! One-shot replacement of this daemon's on-disk installation, independent of new clients.
use super::lifecycle::{Fingerprint, LockGuard};
use serde::{Deserialize, Serialize};
#[cfg(windows)]
use std::process::Command;

#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub(super) enum Intent {
    #[default]
    Exit,
    YieldToClient,
    ReplaceOwnBinary,
    Stop,
}

impl Intent {
    pub(super) fn request(&mut self, next: Self) {
        // An explicit stop/signal wins over disk polling and late Hello messages.
        if *self != Self::Stop && (next != Self::YieldToClient || *self == Self::Exit) {
            *self = next;
        }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Record {
    id: String,
    #[serde(default)]
    at_ms: u64,
    target: Fingerprint,
    failed: bool,
}

fn path() -> std::path::PathBuf {
    crate::paths::config_dir().join("daemon-handoff.json")
}

fn save(record: &Record) {
    if let Ok(bytes) = serde_json::to_vec(record) {
        let _ = crate::integrations::hook_edit::atomic_write_private(&path(), &bytes);
    }
}

fn load() -> Option<Record> {
    let file = path();
    if file.metadata().ok()?.len() > 4096 {
        return None;
    }
    serde_json::from_slice(&std::fs::read(file).ok()?).ok()
}

pub fn failed() -> bool {
    load().is_some_and(|record| record.failed)
}

pub(super) fn healthy(current: Fingerprint) {
    if load().is_some_and(|record| record.target == current) {
        eprintln!("askhuman: update handoff healthy fp={}", current.short());
        let _ = std::fs::remove_file(path());
    }
}

#[cfg(windows)]
static PREPARED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

#[cfg(windows)]
pub(super) fn prepare() -> std::io::Result<()> {
    use std::process::Stdio;
    let target = crate::executable::launch_path()?;
    let record = Record {
        id: uuid::Uuid::new_v4().to_string(),
        at_ms: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64,
        target: super::lifecycle::fingerprint_at(&target),
        failed: false,
    };
    save(&record);
    let mut command = Command::new(target);
    crate::development::clear_client_context(&mut command);
    super::spawn::configure_background(&mut command);
    command
        .args(["daemon", "run", "--background", "--wait-for-handoff"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    match command.spawn() {
        Ok(_) => {
            PREPARED.store(true, std::sync::atomic::Ordering::SeqCst);
            Ok(())
        }
        Err(error) => {
            save(&Record {
                failed: true,
                ..record
            });
            Err(error)
        }
    }
}

#[cfg(windows)]
pub(super) fn timed_out() {
    if let Some(record) = load() {
        save(&Record {
            failed: true,
            ..record
        });
    }
}

#[cfg(unix)]
fn exec_native(target: &std::path::Path) -> std::io::Error {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    let invalid = || {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "handoff contains a NUL byte",
        )
    };
    let Ok(program) = CString::new(target.as_os_str().as_bytes()) else {
        return invalid();
    };
    let args = [
        program.clone(),
        CString::new("daemon").unwrap(),
        CString::new("run").unwrap(),
    ];
    let mut environment = Vec::new();
    for (key, value) in std::env::vars_os() {
        if crate::development::client_environment_key(key.to_str().unwrap_or_default()) {
            continue;
        }
        let mut bytes = key.as_bytes().to_vec();
        bytes.push(b'=');
        bytes.extend_from_slice(value.as_bytes());
        let Ok(entry) = CString::new(bytes) else {
            return invalid();
        };
        environment.push(entry);
    }
    let argv: Vec<_> = args
        .iter()
        .map(|arg| arg.as_ptr())
        .chain(std::iter::once(std::ptr::null()))
        .collect();
    let envp: Vec<_> = environment
        .iter()
        .map(|entry| entry.as_ptr())
        .chain(std::iter::once(std::ptr::null()))
        .collect();
    // All arrays are NUL-terminated and their owned strings outlive this syscall. Use execve,
    // not execvp: a corrupt image must return ENOEXEC, never be interpreted by a shell.
    unsafe {
        libc::execve(program.as_ptr(), argv.as_ptr(), envp.as_ptr());
    }
    std::io::Error::last_os_error()
}

/// The caller retains the singleton lock throughout cleanup and until the exec boundary.
/// Windows' child waits on that same lock; it never kills or replaces a sibling's daemon.
pub(super) fn replace(lock: &LockGuard, _background: bool) -> i32 {
    #[cfg(windows)]
    if PREPARED.load(std::sync::atomic::Ordering::SeqCst) {
        return 0;
    }
    let target = match crate::executable::launch_path() {
        Ok(path) => path,
        Err(error) => {
            eprintln!("askhuman: update handoff failed: {error}");
            return 1;
        }
    };
    let mut record = Record {
        id: uuid::Uuid::new_v4().to_string(),
        at_ms: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64,
        target: super::lifecycle::fingerprint_at(&target),
        failed: false,
    };
    save(&record);
    eprintln!(
        "askhuman: update handoff starting id={} fp={}",
        record.id,
        record.target.short()
    );
    #[cfg(unix)]
    {
        if let Err(error) = lock.close_on_exec() {
            eprintln!("askhuman: update handoff lock setup failed: {error}");
        } else {
            for attempt in 0..3 {
                let error = exec_native(&target);
                eprintln!(
                    "askhuman: update handoff exec attempt={} failed: {error}",
                    attempt + 1
                );
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
        }
    }
    record.failed = true;
    save(&record);
    1
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn a_stop_cannot_be_overridden_by_update_polling() {
        let mut intent = Intent::default();
        intent.request(Intent::ReplaceOwnBinary);
        intent.request(Intent::YieldToClient);
        assert_eq!(intent, Intent::ReplaceOwnBinary);
        intent.request(Intent::Stop);
        intent.request(Intent::ReplaceOwnBinary);
        assert_eq!(intent, Intent::Stop);
    }
    #[cfg(unix)]
    #[test]
    fn invalid_native_image_is_never_run_as_a_shell_script() {
        use std::os::unix::fs::PermissionsExt;
        let directory = tempfile::tempdir().unwrap();
        let image = directory.path().join("AskHuman");
        std::fs::write(&image, "exit 0\n").unwrap();
        std::fs::set_permissions(&image, std::fs::Permissions::from_mode(0o700)).unwrap();
        assert_eq!(exec_native(&image).raw_os_error(), Some(libc::ENOEXEC));
    }
}
