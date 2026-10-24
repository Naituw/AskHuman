//! Standalone Linux publisher, built by install.sh with the existing Rust toolchain.
//! Retained executables must not be removed while a process may still need them.
use std::ffi::{c_char, CString, OsStr};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::os::fd::AsRawFd;
use std::os::unix::{ffi::OsStrExt, fs::OpenOptionsExt, fs::PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::Command;

unsafe extern "C" {
    fn flock(fd: i32, operation: i32) -> i32;
    fn renameat2(oldfd: i32, old: *const c_char, newfd: i32, new: *const c_char, flags: u32)
        -> i32;
}

fn hash(path: &Path) -> io::Result<String> {
    for (program, args) in [("sha256sum", &[][..]), ("shasum", &["-a", "256"][..])] {
        match Command::new(program).args(args).arg(path).output() {
            Ok(output) if output.status.success() => {
                let value = String::from_utf8_lossy(&output.stdout)
                    .split_whitespace()
                    .next()
                    .unwrap_or("")
                    .to_owned();
                if value.len() == 64 && value.bytes().all(|b| b.is_ascii_hexdigit()) {
                    return Ok(value);
                }
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
            _ => {}
        }
    }
    Err(io::Error::other("cannot compute executable SHA-256"))
}

fn regular_destination(path: &Path) -> io::Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_file() => Ok(true),
        Ok(_) => Err(io::Error::other(format!(
            "refusing non-regular install target: {}",
            path.display()
        ))),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e),
    }
}

fn rename(source: &Path, target: &Path, flags: u32) -> io::Result<()> {
    let source = CString::new(source.as_os_str().as_bytes())?;
    let target = CString::new(target.as_os_str().as_bytes())?;
    // AT_FDCWD; RENAME_NOREPLACE=1, RENAME_EXCHANGE=2. No unlink/copy fallback.
    if unsafe { renameat2(-100, source.as_ptr(), -100, target.as_ptr(), flags) } != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

struct Stage(PathBuf);
impl Drop for Stage {
    fn drop(&mut self) {
        // Only unpublished artifacts are removed. After exchange the old ELF is retained.
        let _ = fs::remove_file(self.0.join("AskHuman"));
        let _ = fs::remove_file(self.0.join("state"));
        let _ = fs::remove_dir(&self.0);
    }
}

fn install(source: &Path, directory: &Path) -> io::Result<()> {
    fs::create_dir_all(directory)?;
    // Never remove the lock file: another installer may already be waiting on its inode.
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(0x20000) // O_NOFOLLOW
        .open(directory.join(".askhuman-install.lock"))?;
    if unsafe { flock(lock.as_raw_fd(), 2) } != 0 {
        // LOCK_EX
        return Err(io::Error::last_os_error());
    }
    let destination = directory.join("AskHuman");
    let exists = regular_destination(&destination)?;
    let state_path = directory.join(".askhuman-install-state");
    let source_hash = hash(source)?;
    if exists {
        let installed_hash = hash(&destination)?;
        let expected = format!("source={source_hash}\ninstalled={installed_hash}\n");
        if fs::read_to_string(&state_path).ok().as_deref() == Some(&expected) {
            println!("    installed executable unchanged; skipping publication");
            return Ok(());
        }
    }
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(io::Error::other)?
        .as_nanos();
    let stage_path = directory.join(format!(".askhuman-install-{}-{nonce}", std::process::id()));
    fs::create_dir(&stage_path)?;
    let stage = Stage(stage_path);
    let staged_binary = stage.0.join("AskHuman");
    fs::copy(source, &staged_binary)?;
    fs::set_permissions(&staged_binary, fs::Permissions::from_mode(0o755))?;
    File::open(&staged_binary)?.sync_all()?;
    let installed_hash = hash(&staged_binary)?;
    if source_hash != installed_hash {
        return Err(io::Error::other(
            "build artifact changed while staging; retry installation",
        ));
    }
    let mut state = File::create(stage.0.join("state"))?;
    write!(state, "source={source_hash}\ninstalled={installed_hash}\n")?;
    state.sync_all()?;
    drop(state);
    // Validate again immediately before publication; concurrent cooperating installers hold flock.
    if regular_destination(&destination)? != exists {
        return Err(io::Error::other(
            "install target changed while staging; retry installation",
        ));
    }
    rename(&staged_binary, &destination, if exists { 2 } else { 1 })?;
    // From here on, never remove the old executable, even if writing state fails.
    let retained = stage.0.clone();
    std::mem::forget(stage);
    if exists {
        println!(
            "    retained previous executable: {}",
            staged_binary.display()
        );
    }
    if let Err(e) = fs::rename(retained.join("state"), &state_path) {
        return Err(io::Error::other(format!(
            "executable installed, but install-state publication failed: {e}; retry installation; retained directory: {}", retained.display()
        )));
    }
    let finish = || -> io::Result<()> {
        if exists {
            File::open(&retained)?.sync_all()?;
        } else {
            fs::remove_dir(&retained)?;
        }
        File::open(directory)?.sync_all()
    };
    finish().map_err(|e| io::Error::other(format!(
        "executable and state installed, but final directory synchronization/cleanup failed: {e}; inspect installation before retrying"
    )))?;
    Ok(())
}

fn main() {
    let args: Vec<_> = std::env::args_os().collect();
    if args.len() != 3 || args[1] == OsStr::new("") || args[2] == OsStr::new("") {
        eprintln!("usage: install-linux SOURCE INSTALL_DIR");
        std::process::exit(2);
    }
    if let Err(e) = install(Path::new(&args[1]), Path::new(&args[2])) {
        eprintln!("Linux executable installation failed: {e}");
        std::process::exit(1);
    }
}
