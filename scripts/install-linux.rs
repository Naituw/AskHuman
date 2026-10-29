//! Standalone Linux publisher, built by install.sh with the existing Rust toolchain.
//! Retained executables must not be removed while a process may still need them.
use std::collections::{HashMap, HashSet};
use std::ffi::{c_char, CString, OsStr, OsString};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::os::fd::AsRawFd;
use std::os::unix::{ffi::OsStrExt, fs::MetadataExt, fs::OpenOptionsExt};
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

const RETAINED_PREFIX: &str = ".askhuman-install-";
const EXECUTABLE_NAME: &str = "AskHuman";

/// Device and inode identify a file across mount namespaces and path aliases.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct FileId {
    dev: u64,
    ino: u64,
}

impl FileId {
    fn of(path: &Path) -> io::Result<Self> {
        let meta = fs::metadata(path)?;
        Ok(Self {
            dev: meta.dev(),
            ino: meta.ino(),
        })
    }
}

struct Retained {
    directory: PathBuf,
    binary: PathBuf,
    state: Option<PathBuf>,
    nonce: u128,
}

/// A directory this publisher left behind: `<prefix><pid>-<nonce>/{AskHuman,state?}`.
/// Anything unexpected is ignored here and reported, never deleted.
fn retained_entries(directory: &Path) -> io::Result<Vec<Retained>> {
    let mut retained = Vec::new();
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        let Some(rest) = name.strip_prefix(RETAINED_PREFIX) else {
            continue;
        };
        let Some((pid, nonce)) = rest.split_once('-') else {
            continue;
        };
        if pid.is_empty()
            || nonce.is_empty()
            || !pid.bytes().all(|b| b.is_ascii_digit())
            || !nonce.bytes().all(|b| b.is_ascii_digit())
        {
            continue;
        }
        let Ok(nonce) = nonce.parse::<u128>() else {
            continue;
        };
        let path = entry.path();
        if !fs::symlink_metadata(&path)?.file_type().is_dir() {
            continue;
        }
        let binary = path.join("AskHuman");
        let state = path.join("state");
        let mut has_binary = false;
        let mut has_state = false;
        let mut unknown = false;
        let items = match fs::read_dir(&path) {
            Ok(items) => items,
            Err(error) => {
                eprintln!(
                    "kept retained executable: {} (cannot inspect directory: {error})",
                    binary.display()
                );
                continue;
            }
        };
        for item in items {
            let item = item?;
            let name = item.file_name();
            let name = name.to_string_lossy();
            let meta = fs::symlink_metadata(item.path())?;
            match name.as_ref() {
                "AskHuman" if meta.file_type().is_file() => has_binary = true,
                "state" if meta.file_type().is_file() => has_state = true,
                _ => unknown = true,
            }
        }
        if !has_binary || unknown {
            eprintln!(
                "kept retained executable: {} (unexpected contents; inspect manually)",
                path.display()
            );
            continue;
        }
        retained.push(Retained {
            directory: path,
            binary,
            state: has_state.then_some(state),
            nonce,
        });
    }
    Ok(retained)
}

fn proc_root() -> PathBuf {
    std::env::var_os("ASKHUMAN_INSTALL_PROC_ROOT")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/proc"))
}

struct References {
    pids: HashMap<FileId, u32>,
    complete: bool,
}

/// Find processes executing any candidate image. A scan is complete only when every visible
/// process could be inspected: a single unreadable `/proc/<pid>/exe` makes deletion unsafe.
fn live_references(ids: &HashSet<FileId>) -> References {
    let mut references = References {
        pids: HashMap::new(),
        complete: true,
    };
    if ids.is_empty() {
        return references;
    }
    let proc_root = proc_root();
    let entries = match fs::read_dir(&proc_root) {
        Ok(entries) => entries,
        Err(error) => {
            references.complete = false;
            eprintln!(
                "retained executable pruning skipped: cannot inspect {}: {error}",
                proc_root.display()
            );
            return references;
        }
    };
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                references.complete = false;
                eprintln!(
                    "retained executable pruning skipped: cannot inspect {}: {error}",
                    proc_root.display()
                );
                continue;
            }
        };
        let name = entry.file_name();
        let Some(pid) = name.to_str().and_then(|value| value.parse::<u32>().ok()) else {
            continue;
        };
        let exe = entry.path().join("exe");
        match fs::metadata(&exe) {
            Ok(meta) => {
                let id = FileId {
                    dev: meta.dev(),
                    ino: meta.ino(),
                };
                if ids.contains(&id) {
                    references.pids.entry(id).or_insert(pid);
                }
            }
            // The process exited or became a zombie; it can no longer spawn the image.
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => {
                // Hardened or sandboxed systems can deny stat on another process's exe even
                // for the same uid. The kernel names every process after the executable it
                // is running, so a different name is a safe negative signal; only a process
                // that could be this binary blocks pruning.
                let comm = entry.path().join("comm");
                match fs::read_to_string(&comm) {
                    Ok(name) if name.trim_end() != EXECUTABLE_NAME => {}
                    Ok(_) => {
                        references.complete = false;
                        eprintln!(
                            "retained executable pruning skipped: cannot inspect {} (process is named {EXECUTABLE_NAME}): {error}",
                            exe.display()
                        );
                    }
                    Err(comm_error) => {
                        references.complete = false;
                        eprintln!(
                            "retained executable pruning skipped: cannot inspect {}: {error}; cannot read {}: {comm_error}",
                            exe.display(),
                            comm.display()
                        );
                    }
                }
            }
        }
    }
    references
}

fn remove_retained(entry: &Retained) -> io::Result<()> {
    if let Some(state) = &entry.state {
        fs::remove_file(state)?;
    }
    fs::remove_file(&entry.binary)?;
    fs::remove_dir(&entry.directory)
}

/// Reclaim old images only when the newest one is kept and no live process executes them.
/// `just_retained` is the image this install just swapped out; it is always kept so a process
/// that received its path but has not exec'd yet cannot lose it.
fn prune_retained(directory: &Path, just_retained: Option<&Path>) -> io::Result<()> {
    let mut entries = retained_entries(directory)?;
    if entries.len() < 2 && just_retained.is_none() {
        return Ok(());
    }
    entries.sort_by(|left, right| right.nonce.cmp(&left.nonce));
    let newest_nonce = entries.first().map(|entry| entry.nonce);
    let mut protected: HashSet<PathBuf> = entries
        .iter()
        .filter(|entry| Some(entry.nonce) == newest_nonce)
        .map(|entry| entry.directory.clone())
        .collect();
    if let Some(path) = just_retained {
        protected.insert(path.to_path_buf());
    }
    let candidates: Vec<&Retained> = entries
        .iter()
        .filter(|entry| !protected.contains(&entry.directory))
        .collect();
    if candidates.is_empty() {
        return Ok(());
    }

    let mut ids = HashSet::new();
    let mut identified = Vec::new();
    for entry in &candidates {
        match FileId::of(&entry.binary) {
            Ok(id) => {
                ids.insert(id);
                identified.push((*entry, id));
            }
            Err(error) => eprintln!(
                "kept retained executable: {} (cannot identify: {error})",
                entry.binary.display()
            ),
        }
    }
    let references = live_references(&ids);
    let mut removed = false;
    for (entry, id) in identified {
        if let Some(pid) = references.pids.get(&id) {
            println!(
                "kept retained executable: {} (in use by pid {pid})",
                entry.binary.display()
            );
            continue;
        }
        if !references.complete {
            println!(
                "kept retained executable: {} (cannot verify process references)",
                entry.binary.display()
            );
            continue;
        }
        match remove_retained(entry) {
            Ok(()) => {
                removed = true;
                println!(
                    "removed unreferenced retained executable: {}",
                    entry.binary.display()
                );
            }
            Err(error) => eprintln!(
                "kept retained executable: {} (cannot remove: {error})",
                entry.binary.display()
            ),
        }
    }
    if removed {
        if let Err(error) = File::open(directory).and_then(|dir| dir.sync_all()) {
            eprintln!(
                "retained executable pruning could not sync {}: {error}",
                directory.display()
            );
        }
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

struct TempState(PathBuf);
impl Drop for TempState {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

fn valid_hash(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|b| b.is_ascii_hexdigit())
}

fn publish(
    staged: &Path,
    destination: &Path,
    state_path: &Path,
    source_hash: &str,
    installed_hash: &str,
) -> io::Result<()> {
    if !valid_hash(source_hash) || !valid_hash(installed_hash) {
        return Err(io::Error::other(
            "invalid expected SHA-256 for installation",
        ));
    }
    let directory = destination.parent().ok_or_else(|| {
        io::Error::other(format!(
            "install target has no parent directory: {}",
            destination.display()
        ))
    })?;
    // install.sh resolves the entry link and stages beside the resolved target, so the
    // publication lock must live in that directory rather than in the entry directory.
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
    let exists = regular_destination(destination)?;
    match fs::symlink_metadata(staged) {
        Ok(meta) if meta.file_type().is_file() => {}
        Ok(_) => {
            return Err(io::Error::other(format!(
                "refusing non-regular staged install artifact: {}",
                staged.display()
            )));
        }
        Err(error) => return Err(error),
    }
    // install.sh copied the staged artifact; make its contents durable before hashing and exchange.
    File::open(staged)?.sync_all()?;
    let staged_hash = hash(staged)?;
    if staged_hash != installed_hash {
        return Err(io::Error::other(
            "staged install artifact changed after hashing; retry installation",
        ));
    }
    if source_hash != installed_hash {
        return Err(io::Error::other(
            "build artifact changed while staging; retry installation",
        ));
    }
    let expected = format!("source={source_hash}\ninstalled={installed_hash}\n");
    if exists {
        let destination_hash = hash(destination)?;
        if destination_hash == installed_hash
            && fs::read_to_string(state_path).ok().as_deref() == Some(&expected)
        {
            println!("    installed executable unchanged; skipping publication");
            if let Err(error) = fs::remove_file(staged) {
                eprintln!(
                    "could not remove staged install artifact {}: {error}",
                    staged.display()
                );
            }
            if let Err(error) = prune_retained(directory, None) {
                eprintln!("retained executable pruning skipped: {error}");
            }
            return Ok(());
        }
    }

    let mut state_name = OsString::new();
    state_name.push(
        state_path
            .file_name()
            .unwrap_or_else(|| OsStr::new("state")),
    );
    state_name.push(format!(".tmp.{}", std::process::id()));
    let state_tmp = state_path.with_file_name(state_name);
    let mut state = File::create(&state_tmp)?;
    let state_guard = TempState(state_tmp.clone());
    write!(state, "{expected}")?;
    state.sync_all()?;
    drop(state);

    let retained = if exists {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(io::Error::other)?
            .as_nanos();
        let stage_path =
            directory.join(format!(".askhuman-install-{}-{nonce}", std::process::id()));
        fs::create_dir(&stage_path)?;
        let stage = Stage(stage_path);
        let staged_binary = stage.0.join("AskHuman");
        // The staged file is already a verified copy beside the target; move it into the
        // retained directory so exchange leaves the old executable at a stable path.
        fs::rename(staged, &staged_binary)?;
        // Validate again immediately before publication; concurrent cooperating installers hold flock.
        if regular_destination(destination)? != exists {
            return Err(io::Error::other(
                "install target changed while staging; retry installation",
            ));
        }
        rename(&staged_binary, destination, 2)?;
        // From here on, never remove the old executable, even if writing state fails.
        let retained = stage.0.clone();
        std::mem::forget(stage);
        println!(
            "    retained previous executable: {}",
            staged_binary.display()
        );
        Some(retained)
    } else {
        // Validate again immediately before publication; concurrent cooperating installers hold flock.
        if regular_destination(destination)? != exists {
            return Err(io::Error::other(
                "install target changed while staging; retry installation",
            ));
        }
        rename(staged, destination, 1)?;
        None
    };

    if let Err(error) = fs::rename(&state_tmp, state_path) {
        return Err(io::Error::other(format!(
            "executable installed, but install-state publication failed: {error}; retry installation; retained directory: {}",
            retained
                .as_ref()
                .map_or_else(|| "none".to_owned(), |path| path.display().to_string())
        )));
    }
    std::mem::forget(state_guard);
    if let Some(retained) = &retained {
        File::open(retained)?.sync_all()?;
    }
    File::open(directory)?.sync_all()?;
    if let Err(error) = prune_retained(directory, retained.as_deref()) {
        eprintln!("retained executable pruning skipped: {error}");
    }
    Ok(())
}

fn main() {
    let args: Vec<_> = std::env::args_os().collect();
    if args.len() != 7 || args[1] != OsStr::new("publish") {
        eprintln!("usage: install-linux publish STAGED TARGET STATE SOURCE_HASH INSTALLED_HASH");
        std::process::exit(2);
    }
    let Some(source_hash) = args[5].to_str() else {
        eprintln!("source hash is not valid UTF-8");
        std::process::exit(2);
    };
    let Some(installed_hash) = args[6].to_str() else {
        eprintln!("installed hash is not valid UTF-8");
        std::process::exit(2);
    };
    if let Err(e) = publish(
        Path::new(&args[2]),
        Path::new(&args[3]),
        Path::new(&args[4]),
        source_hash,
        installed_hash,
    ) {
        eprintln!("Linux executable installation failed: {e}");
        std::process::exit(1);
    }
}
