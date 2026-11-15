//! The child processes this daemon inherited across a live update.
//!
//! A live update replaces the daemon's image in place (`exec`), which keeps the process id. Every
//! child the previous image started is therefore still a child of this process, but what that
//! image used to deal with them died with it: the tokio `Child` and its reaper task, the
//! `gui-child-reaper` threads. Nobody waits for such a child any more, so once it exits it stays a
//! zombie, and nothing can end it when the daemon gives up on it.
//!
//! The successor asks the operating system which children it has, before it has started any of
//! its own, and takes them over: it waits for each (woken by `SIGCHLD`, never by a timer), tells
//! whoever watches one when it is gone, and can end one nobody wants any more.
//!
//! Ending a process is the dangerous part. A pid names a process only until that process is
//! reaped, so [`Inherited::end`] signals a pid only after proving, in the same step, that it is
//! still an unreaped child of this process: until the exit status has been collected the kernel
//! keeps the pid, whether the child is running or a zombie. That proof needs this process to be
//! the only one that ever waits for these children, and the kernel not to discard their status
//! behind its back, so [`Inherited::collect`] first puts `SIGCHLD` into a state in which neither
//! can happen (see [`sys::make_children_waitable`]).

use super::log;
use std::collections::BTreeMap;
use std::process::ExitStatus;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use tokio::sync::watch;

/// What became of an inherited child.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Gone {
    /// This process waited for it and collected this status.
    Reaped(ExitStatus),
    /// Not a child of this process after all, or one somebody else already waited for. It is never
    /// waited for or signalled from then on.
    NotOurs,
}

/// What [`Inherited::end`] did.
#[derive(Debug)]
pub(super) enum Ending {
    /// The child was still ours and running or unreaped, and has been sent `SIGKILL`. It is gone
    /// once its watcher reports it, not before.
    Signalled,
    /// It had already exited; its status was collected instead.
    AlreadyGone,
    /// Not an inherited child of this process: nothing was sent.
    NotOurs,
    /// The signal could not be sent.
    Failed(std::io::Error),
}

/// The child processes inherited from the previous image, and what is known of each.
pub(super) struct Inherited {
    /// `None` while the child has not been reaped. Every pid in here was proved to be a child of
    /// this process when the table was built, and a slot becomes [`Gone`] the moment that stops
    /// being provable.
    slots: Mutex<BTreeMap<u32, watch::Sender<Option<Gone>>>>,
    started: AtomicBool,
}

/// Completes when one inherited child is gone.
pub(super) struct Watch(watch::Receiver<Option<Gone>>);

impl Watch {
    /// Waits until the child has been reaped, returns at once if it already was. Never completes
    /// if the table is dropped first: nothing can be said about the child then.
    pub(super) async fn exited(&mut self) -> Gone {
        // Copied out of the read guard, which must not be held across the await below.
        let gone = self
            .0
            .wait_for(|gone| gone.is_some())
            .await
            .map(|gone| *gone);
        match gone {
            Ok(Some(gone)) => gone,
            _ => std::future::pending().await,
        }
    }
}

impl Inherited {
    /// The children this process has right now. Call once, at the very start of the daemon: when
    /// this image has no child of its own that is still running or being waited for. A failed
    /// `exec` resumes the same image, which keeps using this table; children it starts afterwards
    /// are not in it and have their own waiters.
    ///
    /// Anything that goes wrong only costs what this module adds, never the daemon's start.
    pub(super) fn collect() -> Arc<Self> {
        let (ignored, blocked) = sys::make_children_waitable();
        if ignored {
            log("SIGCHLD was ignored at startup; restored the default so child processes can be waited for");
        }
        if blocked {
            log("SIGCHLD was blocked at startup; unblocked it so child exits are noticed");
        }
        match sys::children() {
            Ok(pids) => Self::adopting(pids),
            Err(error) => {
                log(&format!("cannot list inherited child processes: {error}"));
                Self::adopting([])
            }
        }
    }

    /// A table of exactly these pids, each checked once before it is accepted: a child that has
    /// already exited is reaped now, and a pid that is not a child of this process is dropped from
    /// the table's authority (it can never be signalled). The way to build a table in tests, where
    /// other tests keep starting children of their own.
    pub(super) fn adopting(pids: impl IntoIterator<Item = u32>) -> Arc<Self> {
        let slots: BTreeMap<_, _> = pids
            .into_iter()
            // A pid that is zero or does not fit a `pid_t` would mean a process group or "any
            // child" to `waitpid` and `kill`: it must never get as far as a system call.
            .filter(|pid| i32::try_from(*pid).is_ok_and(|pid| pid > 0))
            .map(|pid| (pid, watch::channel(None).0))
            .collect();
        let table = Self {
            slots: Mutex::new(slots),
            started: AtomicBool::new(false),
        };
        let listed: Vec<u32> = table.slots().keys().copied().collect();
        if !listed.is_empty() {
            log(&format!(
                "inherited child processes from the previous image: {listed:?}"
            ));
        }
        // The ones that already exited are collected now, and logged as they are.
        table.sweep();
        Arc::new(table)
    }

    fn slots(&self) -> MutexGuard<'_, BTreeMap<u32, watch::Sender<Option<Gone>>>> {
        // Nothing under this lock can panic halfway: a poisoned guard still holds a valid table.
        self.slots.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Starts waiting for the children that are still running, until none is left. Safe to call
    /// again, as `run_cmd` does when it resumes the same image.
    pub(super) fn start(self: &Arc<Self>) {
        #[cfg(unix)]
        {
            use std::sync::atomic::Ordering;
            use tokio::signal::unix::{signal, SignalKind};
            if self.started.swap(true, Ordering::SeqCst) {
                return;
            }
            let running = self
                .slots()
                .values()
                .filter(|slot| state(slot).is_none())
                .count();
            if running == 0 {
                return;
            }
            // Subscribe before the first sweep: an exit after it still wakes the task below.
            let mut sigchld = match signal(SignalKind::child()) {
                Ok(sigchld) => sigchld,
                Err(error) => {
                    log(&format!(
                        "cannot watch inherited child processes: {error}; they are reaped only when asked about"
                    ));
                    return;
                }
            };
            let table = self.clone();
            tokio::spawn(async move {
                while table.sweep() > 0 {
                    if sigchld.recv().await.is_none() {
                        return;
                    }
                }
            });
        }
    }

    /// Collects every inherited child that has exited by now, without blocking. Returns how many
    /// are still running.
    fn sweep(&self) -> usize {
        let slots = self.slots();
        let mut running = 0;
        for (pid, slot) in slots.iter() {
            if state(slot).is_some() {
                continue;
            }
            match sys::wait(*pid) {
                sys::Wait::Running => running += 1,
                sys::Wait::Exited(status) => record(*pid, slot, Gone::Reaped(status)),
                sys::Wait::NotChild => record(*pid, slot, Gone::NotOurs),
            }
        }
        running
    }

    /// Reports when `pid` is gone. `None` unless `pid` is an inherited child of this process.
    pub(super) fn watch(&self, pid: u32) -> Option<Watch> {
        let slots = self.slots();
        let slot = slots.get(&pid)?;
        if state(slot) == Some(Gone::NotOurs) {
            return None;
        }
        Some(Watch(slot.subscribe()))
    }

    /// Ends `pid` with `SIGKILL` if, and only if, it is an inherited child of this process that
    /// is still there. The proof and the signal happen under one lock, so no sweep can collect the
    /// child (and let its pid be reused) in between.
    pub(super) fn end(&self, pid: u32) -> Ending {
        let slots = self.slots();
        let Some(slot) = slots.get(&pid) else {
            return Ending::NotOurs;
        };
        match state(slot) {
            Some(Gone::NotOurs) => return Ending::NotOurs,
            Some(Gone::Reaped(_)) => return Ending::AlreadyGone,
            None => {}
        }
        match sys::wait(pid) {
            sys::Wait::Exited(status) => {
                record(pid, slot, Gone::Reaped(status));
                Ending::AlreadyGone
            }
            sys::Wait::NotChild => {
                record(pid, slot, Gone::NotOurs);
                Ending::NotOurs
            }
            sys::Wait::Running => match sys::kill(pid) {
                Ok(()) => Ending::Signalled,
                Err(error) => Ending::Failed(error),
            },
        }
    }
}

/// The slot's value, copied out so that no `watch` read guard outlives the statement (a write to
/// the same channel while one is held would block forever).
fn state(slot: &watch::Sender<Option<Gone>>) -> Option<Gone> {
    *slot.borrow()
}

fn record(pid: u32, slot: &watch::Sender<Option<Gone>>, gone: Gone) {
    match gone {
        Gone::Reaped(status) => log(&format!("inherited child pid={pid} exited with {status}")),
        Gone::NotOurs => log(&format!(
            "pid={pid} was listed as an inherited child but is not one; leaving it alone"
        )),
    }
    slot.send_replace(Some(gone));
}

/// The few system calls this module needs, with an inert version for platforms without them.
#[cfg(unix)]
mod sys {
    use std::process::ExitStatus;

    pub(super) enum Wait {
        Running,
        Exited(ExitStatus),
        /// Not a child of this process (`ECHILD`), or anything unexpected: never touched again.
        NotChild,
    }

    /// A pid that names exactly one process, as `waitpid` and `kill` need.
    fn single(pid: u32) -> Option<libc::pid_t> {
        libc::pid_t::try_from(pid).ok().filter(|pid| *pid > 0)
    }

    /// Waits for `pid` without blocking.
    pub(super) fn wait(pid: u32) -> Wait {
        use std::os::unix::process::ExitStatusExt;
        let Some(pid) = single(pid) else {
            return Wait::NotChild;
        };
        let mut status: libc::c_int = 0;
        loop {
            // SAFETY: `status` is a valid out pointer for the duration of the call.
            let reaped = unsafe { libc::waitpid(pid, &mut status, libc::WNOHANG) };
            if reaped == 0 {
                return Wait::Running;
            }
            if reaped == pid {
                return Wait::Exited(ExitStatus::from_raw(status));
            }
            if reaped == -1
                && std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted
            {
                continue;
            }
            return Wait::NotChild;
        }
    }

    pub(super) fn kill(pid: u32) -> std::io::Result<()> {
        let Some(pid) = single(pid) else {
            return Err(std::io::Error::from(std::io::ErrorKind::InvalidInput));
        };
        // SAFETY: plain system call on a single, positive pid.
        if unsafe { libc::kill(pid, libc::SIGKILL) } == 0 {
            Ok(())
        } else {
            Err(std::io::Error::last_os_error())
        }
    }

    /// Makes `SIGCHLD` safe to build on, returning what had to be undone as `(ignored, blocked)`.
    ///
    /// Both states survive `exec`, so a daemon started by an unusual launcher can have them:
    /// * **Ignored** (or flagged `SA_NOCLDWAIT`): where the kernel then discards a child's status
    ///   instead of leaving a zombie (Linux does), a listed pid could be reused while the table
    ///   still believed it to be its child.
    /// * **Blocked**: the signal that wakes the reaper would never be delivered to any thread.
    ///
    /// Call it while the process is still single threaded, before any runtime thread exists to
    /// inherit the mask.
    pub(super) fn make_children_waitable() -> (bool, bool) {
        // SAFETY: plain libc calls on zero-initialised, fully written structures.
        unsafe {
            let mut ignored = false;
            let mut current: libc::sigaction = std::mem::zeroed();
            if libc::sigaction(libc::SIGCHLD, std::ptr::null(), &mut current) == 0
                && (current.sa_sigaction == libc::SIG_IGN
                    || current.sa_flags & libc::SA_NOCLDWAIT != 0)
            {
                let mut default: libc::sigaction = std::mem::zeroed();
                default.sa_sigaction = libc::SIG_DFL;
                libc::sigemptyset(&mut default.sa_mask);
                ignored = libc::sigaction(libc::SIGCHLD, &default, std::ptr::null_mut()) == 0;
            }
            let mut chld: libc::sigset_t = std::mem::zeroed();
            libc::sigemptyset(&mut chld);
            libc::sigaddset(&mut chld, libc::SIGCHLD);
            let mut before: libc::sigset_t = std::mem::zeroed();
            let blocked = libc::pthread_sigmask(libc::SIG_UNBLOCK, &chld, &mut before) == 0
                && libc::sigismember(&before, libc::SIGCHLD) == 1;
            (ignored, blocked)
        }
    }

    /// The pids of this process's children, running or not yet reaped.
    #[cfg(target_os = "macos")]
    pub(super) fn children() -> std::io::Result<Vec<u32>> {
        // `proc_listchildpids` wraps `proc_listpids(PROC_PPID_ONLY)`: it returns the NUMBER of pids
        // written, and 0 both for "no children" and for an error, so a failure here cannot be told
        // from an empty list. Without a buffer it reports the size of the whole process table, not
        // of the children, which makes it a fair first guess; grow until the answer fits.
        let me = std::process::id() as libc::pid_t;
        // SAFETY: a null buffer of size zero asks only for the size.
        let table = unsafe { libc::proc_listchildpids(me, std::ptr::null_mut(), 0) };
        let mut capacity = usize::try_from(table).unwrap_or(0) + 16;
        loop {
            let mut pids: Vec<libc::pid_t> = vec![0; capacity];
            let bytes = libc::c_int::try_from(pids.len() * std::mem::size_of::<libc::pid_t>())
                .map_err(std::io::Error::other)?;
            // SAFETY: the buffer holds `bytes` bytes.
            let count = unsafe { libc::proc_listchildpids(me, pids.as_mut_ptr().cast(), bytes) };
            let count = usize::try_from(count).unwrap_or(0);
            if count < pids.len() {
                pids.truncate(count);
                return Ok(pids
                    .into_iter()
                    .filter_map(|pid| u32::try_from(pid).ok().filter(|pid| *pid > 0))
                    .collect());
            }
            capacity = capacity
                .checked_mul(2)
                .filter(|c| *c <= 1 << 22)
                .ok_or_else(|| {
                    std::io::Error::other("the process table is larger than expected")
                })?;
        }
    }

    /// The pids of this process's children, running or not yet reaped.
    #[cfg(target_os = "linux")]
    pub(super) fn children() -> std::io::Result<Vec<u32>> {
        super::children_in(std::path::Path::new("/proc"), std::process::id())
    }

    /// Other Unix systems are not supported targets; they start with nothing to take over.
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    pub(super) fn children() -> std::io::Result<Vec<u32>> {
        Ok(Vec::new())
    }
}

#[cfg(not(unix))]
mod sys {
    use std::process::ExitStatus;

    pub(super) enum Wait {
        Running,
        Exited(ExitStatus),
        NotChild,
    }

    /// Windows has no zombies and its update starts a new process instead of replacing the image,
    /// so there is nothing inherited to wait for.
    pub(super) fn wait(_pid: u32) -> Wait {
        Wait::NotChild
    }

    pub(super) fn kill(_pid: u32) -> std::io::Result<()> {
        Err(std::io::Error::from(std::io::ErrorKind::Unsupported))
    }

    pub(super) fn make_children_waitable() -> (bool, bool) {
        (false, false)
    }

    pub(super) fn children() -> std::io::Result<Vec<u32>> {
        Ok(Vec::new())
    }
}

/// The pids in `proc_root` (a `/proc` mount) whose parent is `parent`, zombies included.
#[cfg(any(target_os = "linux", test))]
fn children_in(proc_root: &std::path::Path, parent: u32) -> std::io::Result<Vec<u32>> {
    let mut pids = Vec::new();
    for entry in std::fs::read_dir(proc_root)? {
        let Ok(entry) = entry else { continue };
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|name| name.parse::<u32>().ok())
        else {
            continue;
        };
        // A process can exit while the directory is being walked, and that says nothing about
        // the others. The contents are read as bytes: a process name need not be UTF-8.
        let Ok(stat) = std::fs::read(entry.path().join("stat")) else {
            continue;
        };
        if parent_pid(&stat) == Some(parent) {
            pids.push(pid);
        }
    }
    Ok(pids)
}

/// The parent pid in the contents of `/proc/<pid>/stat`: the second field after the process name,
/// which is in parentheses and may itself hold spaces and parentheses, so the fields are counted
/// from the last closing one.
#[cfg(any(target_os = "linux", test))]
fn parent_pid(stat: &[u8]) -> Option<u32> {
    let close = stat.iter().rposition(|byte| *byte == b')')?;
    let mut fields = stat[close + 1..]
        .split(|byte| byte.is_ascii_whitespace())
        .filter(|field| !field.is_empty());
    let _state = fields.next()?;
    std::str::from_utf8(fields.next()?).ok()?.parse().ok()
}

/// What tests need to play a previous image's children.
#[cfg(all(test, unix))]
pub(super) mod testing {
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};

    /// Starts a child and drops its handle on the spot. std never waits for a dropped child,
    /// so from here on it is what a previous image's child is to its successor: nobody's to
    /// wait for. The returned pipe holds it in a `read` until it is dropped.
    // Never waiting is the point: the code under test collects these children, and whatever a
    // failing test leaves behind goes to init when the test process exits.
    #[allow(clippy::zombie_processes)]
    pub fn orphaned(script: &str) -> (u32, std::process::ChildStdin) {
        let mut child = Command::new("/bin/sh")
            .args(["-c", script])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            // A child that outlives a failed test must not hold the test runner's pipes open.
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let stdin = child.stdin.take().unwrap();
        (child.id(), stdin)
    }

    pub fn exists(pid: u32) -> bool {
        unsafe { libc::kill(pid as libc::pid_t, 0) == 0 }
    }

    fn is_zombie(pid: u32) -> bool {
        let output = Command::new("ps")
            .args(["-o", "stat=", "-p", &pid.to_string()])
            .output()
            .unwrap();
        String::from_utf8_lossy(&output.stdout)
            .trim()
            .starts_with('Z')
    }

    /// A zombie is a child that exited and has not been waited for.
    pub fn wait_until_zombie(pid: u32) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !is_zombie(pid) {
            assert!(Instant::now() < deadline, "child {pid} never exited");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// A running process that is nobody's child of ours: the shell that starts it exits, which
    /// hands it to the system's init. Killed when dropped, so no test leaves it behind.
    pub struct Stranger(pub u32);

    impl Stranger {
        pub fn start() -> Self {
            let output = Command::new("/bin/sh")
                .args(["-c", "sleep 600 </dev/null >/dev/null 2>&1 & echo $!"])
                .output()
                .unwrap();
            Self(
                String::from_utf8(output.stdout)
                    .unwrap()
                    .trim()
                    .parse()
                    .unwrap(),
            )
        }
    }

    impl Drop for Stranger {
        fn drop(&mut self) {
            unsafe { libc::kill(self.0 as libc::pid_t, libc::SIGKILL) };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_parent_is_read_from_after_the_last_parenthesis() {
        assert_eq!(
            parent_pid(b"123 (bash) S 45 123 123 0 -1 4194560"),
            Some(45)
        );
        // A process name can hold spaces and parentheses; counting fields from the front would
        // read a piece of the name instead.
        assert_eq!(parent_pid(b"123 (a (b) c) R 678 1 1 0"), Some(678));
        assert_eq!(parent_pid(b"123 ()) Z 9 1"), Some(9));
        for malformed in [
            &b""[..],
            b"123 bash S 45",
            b"123 (bash)",
            b"123 (bash) S",
            b"123 (bash) S x 1",
            b"123 (bash) S -1 1",
        ] {
            assert_eq!(parent_pid(malformed), None, "{malformed:?}");
        }
    }

    #[test]
    fn the_children_in_a_proc_directory_are_the_pids_whose_parent_is_asked_for() {
        let proc = tempfile::tempdir().unwrap();
        let process = |name: &str, stat: &[u8]| {
            let dir = proc.path().join(name);
            std::fs::create_dir(&dir).unwrap();
            std::fs::write(dir.join("stat"), stat).unwrap();
        };
        process("100", b"100 (child) S 7 100 100 0");
        process("101", b"101 (a (b) c) R 7 1 1 0");
        process("102", b"102 (somebody else's) S 8 1 1 0");
        // Not UTF-8, as a process may name itself.
        process("103", b"103 (\xff\xfe) Z 7 1 1 0");
        process("self", b"1 (not a pid) S 7 1 1 0");
        // Gone while the directory was being walked: nothing to read.
        std::fs::create_dir(proc.path().join("104")).unwrap();
        std::fs::write(proc.path().join("version"), "Linux").unwrap();

        let mut found = children_in(proc.path(), 7).unwrap();
        found.sort_unstable();
        assert_eq!(found, [100, 101, 103]);
        assert!(children_in(&proc.path().join("missing"), 7).is_err());
    }

    #[test]
    fn pids_that_cannot_name_one_process_are_never_taken() {
        let table = Inherited::adopting([0, u32::MAX, i32::MAX as u32 + 1]);
        assert!(table.slots().is_empty());
        for pid in [0, u32::MAX, i32::MAX as u32 + 1] {
            assert!(table.watch(pid).is_none());
            assert!(matches!(table.end(pid), Ending::NotOurs));
        }
    }

    #[cfg(unix)]
    mod unix {
        use super::super::testing::{exists, orphaned, wait_until_zombie, Stranger};
        use super::*;
        use futures_util::FutureExt;
        use std::os::unix::process::ExitStatusExt;
        use std::process::{Command, Stdio};
        use std::time::{Duration, Instant};

        #[test]
        fn a_child_that_already_exited_is_reaped_when_the_table_is_built() {
            let (pid, _held) = orphaned("exit 3");
            wait_until_zombie(pid);

            let table = Inherited::adopting([pid]);

            let mut watch = table.watch(pid).expect("a child of ours");
            let Some(Gone::Reaped(status)) = watch.exited().now_or_never() else {
                panic!("a zombie is collected at once");
            };
            assert_eq!(status.code(), Some(3));
            assert!(!exists(pid), "the zombie is gone");
        }

        #[tokio::test]
        async fn a_running_child_is_left_alone_and_reaped_when_it_exits() {
            let (pid, held) = orphaned("read line; exit 0");
            let table = Inherited::adopting([pid]);
            table.start();
            let mut watch = table.watch(pid).unwrap();

            // Nothing is reported, and nothing is touched, while the child runs.
            assert!(
                tokio::time::timeout(Duration::from_millis(300), watch.exited())
                    .await
                    .is_err()
            );
            assert!(exists(pid));

            drop(held);
            let gone = tokio::time::timeout(Duration::from_secs(10), watch.exited())
                .await
                .expect("the exit wakes the reaper");
            assert!(matches!(gone, Gone::Reaped(status) if status.success()));
            assert!(!exists(pid), "no zombie is left");
        }

        #[tokio::test]
        async fn ending_a_running_child_kills_and_reaps_it() {
            let (pid, _held) = orphaned("exec sleep 600");
            let table = Inherited::adopting([pid]);
            table.start();
            let mut watch = table.watch(pid).unwrap();

            assert!(matches!(table.end(pid), Ending::Signalled));

            let Gone::Reaped(status) =
                tokio::time::timeout(Duration::from_secs(10), watch.exited())
                    .await
                    .expect("the killed child is reaped")
            else {
                panic!("a child of ours is reaped");
            };
            assert_eq!(status.signal(), Some(libc::SIGKILL));
            assert!(!exists(pid));
            // Its pid may belong to someone else by now.
            assert!(matches!(table.end(pid), Ending::AlreadyGone));
        }

        #[test]
        fn ending_a_child_that_has_exited_collects_it_instead_of_signalling() {
            let (pid, held) = orphaned("read line; exit 0");
            // No sweep has run since the child exits: the table still believes it is running.
            let table = Inherited::adopting([pid]);
            drop(held);
            wait_until_zombie(pid);

            assert!(matches!(table.end(pid), Ending::AlreadyGone));

            let Some(Gone::Reaped(status)) = table.watch(pid).unwrap().exited().now_or_never()
            else {
                panic!("collected by the attempt to end it");
            };
            assert!(status.success());
        }

        #[test]
        fn a_process_that_is_not_our_child_is_never_signalled() {
            let stranger = Stranger::start();
            assert!(exists(stranger.0));

            // Listed by mistake: the table finds out it is not a child before it ever signals.
            let table = Inherited::adopting([stranger.0]);

            assert!(table.watch(stranger.0).is_none());
            assert!(matches!(table.end(stranger.0), Ending::NotOurs));
            assert!(matches!(table.end(stranger.0 + 1), Ending::NotOurs));
            assert!(exists(stranger.0), "left running");
        }

        #[test]
        fn the_children_of_this_process_are_listed_running_or_not() {
            let (running, held) = orphaned("read line; exit 0");
            let (exited, _gone) = orphaned("exit 0");
            wait_until_zombie(exited);

            let listed = sys::children().unwrap();

            // Other tests start children too, so only these two can be asserted.
            assert!(listed.contains(&running), "{listed:?}");
            assert!(listed.contains(&exited), "{listed:?}");

            // Collect both, so no zombie outlives the test.
            drop(held);
            wait_until_zombie(running);
            Inherited::adopting([running, exited]);
        }

        /// What the reaper must not do: take the exit status of a child that someone else waits for.
        #[tokio::test]
        async fn inherited_children_are_reaped_alongside_every_other_way_of_waiting() {
            let (live, held) = orphaned("read line; exit 4");
            let (dead, _gone) = orphaned("exit 5");
            wait_until_zombie(dead);
            let table = Inherited::adopting([live, dead]);
            table.start();

            let tokio_child = async {
                tokio::process::Command::new("/bin/sh")
                    .args(["-c", "exit 6"])
                    .status()
                    .await
                    .unwrap()
                    .code()
            };
            let std_status = tokio::task::spawn_blocking(|| {
                Command::new("/bin/sh")
                    .args(["-c", "exit 7"])
                    .status()
                    .unwrap()
                    .code()
            });
            let output = tokio::task::spawn_blocking(|| {
                Command::new("/bin/sh")
                    .args(["-c", "printf out; exit 8"])
                    .output()
                    .unwrap()
            });
            let reaper_thread = async {
                let mut command = Command::new("/bin/sh");
                command
                    .args(["-c", "exit 9"])
                    .stdin(Stdio::null())
                    .stdout(Stdio::null());
                let pid = crate::daemon::spawn::spawn_and_reap(&mut command).unwrap();
                let deadline = Instant::now() + Duration::from_secs(10);
                while exists(pid) {
                    assert!(Instant::now() < deadline, "{pid} was not reaped");
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            };
            let release = async {
                tokio::time::sleep(Duration::from_millis(100)).await;
                drop(held);
            };
            let (tokio_code, std_code, output, (), ()) =
                tokio::join!(tokio_child, std_status, output, reaper_thread, release);

            assert_eq!(tokio_code, Some(6));
            assert_eq!(std_code.unwrap(), Some(7));
            let output = output.unwrap();
            assert_eq!(
                (output.status.code(), output.stdout),
                (Some(8), b"out".to_vec())
            );
            for (pid, code) in [(live, 4), (dead, 5)] {
                let mut watch = table.watch(pid).unwrap();
                let gone = tokio::time::timeout(Duration::from_secs(10), watch.exited())
                    .await
                    .expect("inherited children are reaped too");
                assert!(matches!(gone, Gone::Reaped(status) if status.code() == Some(code)));
            }
        }

        // The disposition and the mask are process wide, so this runs in a process of its own.
        const SIGCHLD_FIXTURE_TEST: &str =
            "daemon::server_impl::inherited::tests::unix::sigchld_process_fixture";

        #[test]
        fn a_daemon_started_with_sigchld_ignored_or_blocked_can_still_wait_for_children() {
            let output = Command::new(std::env::current_exe().unwrap())
                .args(["--exact", SIGCHLD_FIXTURE_TEST, "--ignored", "--nocapture"])
                .stdin(Stdio::null())
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
        }

        #[test]
        #[ignore = "subprocess fixture, invoked by the SIGCHLD regression test"]
        fn sigchld_process_fixture() {
            fn disposition() -> libc::sigaction {
                let mut current: libc::sigaction = unsafe { std::mem::zeroed() };
                assert_eq!(
                    unsafe { libc::sigaction(libc::SIGCHLD, std::ptr::null(), &mut current) },
                    0
                );
                current
            }
            fn blocked() -> bool {
                let mut mask: libc::sigset_t = unsafe { std::mem::zeroed() };
                unsafe { libc::pthread_sigmask(libc::SIG_BLOCK, std::ptr::null(), &mut mask) };
                unsafe { libc::sigismember(&mask, libc::SIGCHLD) == 1 }
            }
            fn block() {
                let mut set: libc::sigset_t = unsafe { std::mem::zeroed() };
                unsafe {
                    libc::sigemptyset(&mut set);
                    libc::sigaddset(&mut set, libc::SIGCHLD);
                    libc::pthread_sigmask(libc::SIG_BLOCK, &set, std::ptr::null_mut());
                }
            }
            /// A child that exits must still be there to be waited for.
            fn children_leave_a_status() {
                let child = Command::new("/bin/sh")
                    .args(["-c", "exit 7"])
                    .spawn()
                    .unwrap();
                let pid = child.id();
                drop(child);
                wait_until_zombie(pid);
                let sys::Wait::Exited(status) = sys::wait(pid) else {
                    panic!("the status of child {pid} was discarded");
                };
                assert_eq!(status.code(), Some(7));
            }

            // Whatever the test runner's launcher left; once undone there is nothing more to do.
            sys::make_children_waitable();
            assert_eq!(sys::make_children_waitable(), (false, false));
            children_leave_a_status();

            // Ignored and blocked, as a launcher can leave them across exec.
            unsafe { libc::signal(libc::SIGCHLD, libc::SIG_IGN) };
            block();
            assert_eq!(sys::make_children_waitable(), (true, true));
            assert_eq!(disposition().sa_sigaction, libc::SIG_DFL);
            assert!(!blocked());
            children_leave_a_status();

            // The flag that makes the kernel discard statuses without ignoring the signal.
            let mut flagged: libc::sigaction = unsafe { std::mem::zeroed() };
            flagged.sa_sigaction = libc::SIG_DFL;
            flagged.sa_flags = libc::SA_NOCLDWAIT;
            unsafe { libc::sigaction(libc::SIGCHLD, &flagged, std::ptr::null_mut()) };
            assert_eq!(sys::make_children_waitable(), (true, false));
            assert_eq!(disposition().sa_flags & libc::SA_NOCLDWAIT, 0);
            children_leave_a_status();
        }
    }
}
