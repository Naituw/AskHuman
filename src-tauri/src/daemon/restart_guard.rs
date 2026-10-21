//! Why this daemon gave way, and when it must stop giving way.
//!
//! A daemon yields to any client whose binary *content* differs from its own (spec
//! `docs/specs/daemon-graceful-drain.md`). That is right for an upgrade and wrong when two
//! differently built copies are in use at the same time: each copy's clients make the running
//! daemon exit, the client respawns it from its own binary, and the other copy's clients do the
//! same a minute later. Every swap costs an IM reconnect and parks new asks behind a drain.
//!
//! This module keeps two things:
//! * a small persisted history of every yield together with the client that caused it
//!   (`daemon.log` gets the same facts), and
//! * the decision to *refuse* one kind of yield once the same two binaries have swapped places
//!   twice within ten minutes: a client whose binary differs while the daemon's own binary is
//!   untouched and the IPC protocol version matches. The daemon then serves that client as it is
//!   (the "hold") until the conflict goes quiet.
//!
//! Everything else keeps giving way: the daemon's own binary changing on disk is an upgrade, and a
//! client on another protocol version cannot share a daemon safely.

use super::lifecycle::Fingerprint;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// Window in which swaps between the same two binaries are counted.
pub const WINDOW_MS: u64 = 10 * 60 * 1000;
/// Swaps of one pair inside the window after which the next one is refused.
pub const HOLD_AFTER: usize = 2;
/// A hold ends once no differing client has said Hello for this long.
pub const HOLD_QUIET_MS: u64 = 30 * 60 * 1000;
/// While held, the log gets a line at most this often.
pub const HOLD_LOG_EVERY_MS: u64 = 5 * 60 * 1000;
/// History retention: entries older than this are dropped on write.
const KEEP_MS: u64 = 24 * 60 * 60 * 1000;
/// History retention: at most this many entries are kept.
const KEEP_MAX: usize = 50;

/// What made the daemon give way.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Trigger {
    /// A client running a different binary said Hello.
    Client,
    /// The daemon's own executable changed on disk (in-app update, npm, `install.sh`).
    Disk,
    /// A client speaks another IPC protocol version.
    Protocol,
}

impl Trigger {
    /// Stable label used in `daemon.log`.
    pub fn label(self) -> &'static str {
        match self {
            Trigger::Client => "client binary differs",
            Trigger::Disk => "own binary changed on disk",
            Trigger::Protocol => "protocol version differs",
        }
    }
}

/// The client behind a Hello, as the handshake describes it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Peer {
    pub version: String,
    pub protocol_version: u32,
    pub path: String,
    pub pid: u32,
    pub fingerprint: Fingerprint,
}

impl Peer {
    pub fn from_hello(hello: &crate::ipc::ClientHello) -> Self {
        Self {
            version: hello.client_version.clone(),
            protocol_version: hello.protocol_version,
            path: hello.binary_path.clone(),
            pid: hello.pid,
            fingerprint: hello.fingerprint,
        }
    }

    /// One-line description for logs.
    pub fn describe(&self) -> String {
        format!(
            "client v{} protocol={} pid={} path={}",
            self.version, self.protocol_version, self.pid, self.path
        )
    }
}

/// One time the daemon gave way (it exits at once, or drains first when requests are active).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Yield {
    pub at_ms: u64,
    pub trigger: Trigger,
    /// In-flight requests turned the exit into a drain.
    pub drained: bool,
    pub daemon_pid: u32,
    pub daemon_version: String,
    /// Content of the binary that gave way, and of the one it gave way to.
    pub from: Fingerprint,
    pub to: Fingerprint,
    /// The Hello that caused it; absent for disk changes noticed by the daemon itself.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client: Option<Peer>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Store {
    #[serde(default)]
    yields: Vec<Yield>,
}

/// `~/.askhuman/state/daemon-restarts.json`.
pub fn store_path() -> PathBuf {
    crate::paths::state_dir().join("daemon-restarts.json")
}

/// Recorded yields, oldest first. A missing or unreadable file is an empty history.
pub fn load() -> Vec<Yield> {
    load_from(&store_path())
}

/// Append one yield (best-effort: the daemon must give way even when the disk refuses).
pub fn record(entry: Yield) {
    let _ = record_to(&store_path(), entry);
}

/// Forget the history. `daemon stop` / `daemon restart` call it: a person who stops or restarts the
/// daemon on purpose starts over, and the next daemon must not re-enter a hold from old swaps.
pub fn clear() {
    let _ = std::fs::remove_file(store_path());
}

fn load_from(path: &Path) -> Vec<Yield> {
    std::fs::read(path)
        .ok()
        .and_then(|data| serde_json::from_slice::<Store>(&data).ok())
        .map(|store| store.yields)
        .unwrap_or_default()
}

fn record_to(path: &Path, entry: Yield) -> std::io::Result<()> {
    let mut yields = load_from(path);
    // A daemon process gives way once and then exits. Hellos that land while it is shutting down
    // are stale too and would be recorded again, turning one swap into several in the count.
    if yields
        .iter()
        .any(|y| y.daemon_pid == entry.daemon_pid && y.from == entry.from)
    {
        return Ok(());
    }
    let now = entry.at_ms;
    yields.push(entry);
    yields.retain(|y| now.saturating_sub(y.at_ms) <= KEEP_MS);
    if yields.len() > KEEP_MAX {
        let extra = yields.len() - KEEP_MAX;
        yields.drain(..extra);
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let data = serde_json::to_vec_pretty(&Store { yields })
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, data)?;
    std::fs::rename(&tmp, path)
}

/// Which reason to record for a Hello that made the daemon stale. The daemon's own binary changing
/// wins: that is an upgrade whoever happens to say Hello first.
pub fn trigger_for(own_changed: bool, protocol_differs: bool) -> Trigger {
    if own_changed {
        Trigger::Disk
    } else if protocol_differs {
        Trigger::Protocol
    } else {
        Trigger::Client
    }
}

/// Whether a stale Hello may be refused at all: only a differing client binary under the same
/// protocol version, while the daemon's own binary is untouched.
pub fn holdable(own_changed: bool, protocol_differs: bool) -> bool {
    !own_changed && !protocol_differs
}

/// How often these two binaries replaced each other, in either direction, inside the window.
pub fn swaps(history: &[Yield], now_ms: u64, a: Fingerprint, b: Fingerprint) -> usize {
    history
        .iter()
        .filter(|y| now_ms.saturating_sub(y.at_ms) <= WINDOW_MS)
        .filter(|y| (y.from == a && y.to == b) || (y.from == b && y.to == a))
        .count()
}

/// An active hold: the daemon keeps serving clients whose binary differs from its own.
#[derive(Debug, Clone)]
pub struct Hold {
    pub since_ms: u64,
    pub last_ms: u64,
    /// Hellos from differing clients answered since the hold began.
    pub shared: u64,
    /// The latest differing client.
    pub peer: Peer,
    logged_ms: u64,
}

/// What to do with a stale Hello that may be held.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// Give way as usual.
    Yield,
    /// Keep running and answer the client as if nothing differed.
    Hold {
        /// This Hello began the hold.
        started: bool,
        /// A log line is due (the first one, then at most one per [`HOLD_LOG_EVERY_MS`]).
        log: bool,
        shared: u64,
    },
}

/// The daemon's in-memory hold. A new daemon process starts without one and derives it again from
/// the persisted history, which is what keeps a freshly restarted daemon from joining the swap.
#[derive(Debug, Default)]
pub struct HoldState(Mutex<Option<Hold>>);

impl HoldState {
    /// Decide for a client that differs from this daemon's unchanged binary under the same
    /// protocol version. `history` is only read when no hold is active yet.
    pub fn assess(
        &self,
        now_ms: u64,
        daemon: Fingerprint,
        peer: &Peer,
        history: impl FnOnce() -> Vec<Yield>,
    ) -> Verdict {
        let mut slot = self.0.lock().unwrap();
        if slot
            .as_ref()
            .is_some_and(|hold| now_ms.saturating_sub(hold.last_ms) > HOLD_QUIET_MS)
        {
            *slot = None;
        }
        if let Some(hold) = slot.as_mut() {
            hold.last_ms = now_ms;
            hold.shared += 1;
            hold.peer = peer.clone();
            let log = now_ms.saturating_sub(hold.logged_ms) >= HOLD_LOG_EVERY_MS;
            if log {
                hold.logged_ms = now_ms;
            }
            return Verdict::Hold {
                started: false,
                log,
                shared: hold.shared,
            };
        }
        if swaps(&history(), now_ms, daemon, peer.fingerprint) < HOLD_AFTER {
            return Verdict::Yield;
        }
        *slot = Some(Hold {
            since_ms: now_ms,
            last_ms: now_ms,
            shared: 1,
            peer: peer.clone(),
            logged_ms: now_ms,
        });
        Verdict::Hold {
            started: true,
            log: true,
            shared: 1,
        }
    }

    /// The active hold, or `None` once the conflict has been quiet for [`HOLD_QUIET_MS`].
    pub fn snapshot(&self, now_ms: u64) -> Option<Hold> {
        let mut slot = self.0.lock().unwrap();
        if slot
            .as_ref()
            .is_some_and(|hold| now_ms.saturating_sub(hold.last_ms) > HOLD_QUIET_MS)
        {
            *slot = None;
        }
        slot.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: Fingerprint = Fingerprint { size: 10, hash: 1 };
    const B: Fingerprint = Fingerprint { size: 20, hash: 2 };
    const C: Fingerprint = Fingerprint { size: 30, hash: 3 };
    const MIN: u64 = 60 * 1000;

    fn peer(fp: Fingerprint) -> Peer {
        Peer {
            version: "0.14.5".into(),
            protocol_version: 3,
            path: "/old/AskHuman".into(),
            pid: 4242,
            fingerprint: fp,
        }
    }

    /// One daemon giving way. Every call stands for another daemon process (the pid follows the
    /// time), as in real life where a process gives way once.
    fn swap(at_ms: u64, from: Fingerprint, to: Fingerprint) -> Yield {
        Yield {
            at_ms,
            trigger: Trigger::Client,
            drained: false,
            daemon_pid: at_ms as u32,
            daemon_version: "0.15.0".into(),
            from,
            to,
            client: Some(peer(to)),
        }
    }

    #[test]
    fn swaps_count_both_directions_of_one_pair_inside_the_window() {
        let now = 100 * MIN;
        let history = vec![
            swap(now - 30 * MIN, A, B), // too old
            swap(now - 5 * MIN, A, B),
            swap(now - 3 * MIN, B, A),
            swap(now - 2 * MIN, B, C), // another pair
        ];
        assert_eq!(swaps(&history, now, A, B), 2);
        assert_eq!(swaps(&history, now, B, A), 2);
        assert_eq!(swaps(&history, now, B, C), 1);
        assert_eq!(swaps(&history, now, A, C), 0);
    }

    #[test]
    fn the_third_swap_of_a_pair_is_refused_but_the_second_is_not() {
        let hold = HoldState::default();
        let now = 100 * MIN;

        // A → B happened once: the daemon (B) may still give way to A.
        let one = vec![swap(now - MIN, A, B)];
        assert_eq!(
            hold.assess(now, B, &peer(A), || one.clone()),
            Verdict::Yield
        );

        // A → B → A happened: now the daemon (A) refuses to give way to B again.
        let two = vec![swap(now - 2 * MIN, A, B), swap(now - MIN, B, A)];
        assert_eq!(
            hold.assess(now, A, &peer(B), || two.clone()),
            Verdict::Hold {
                started: true,
                log: true,
                shared: 1
            }
        );
    }

    #[test]
    fn fresh_builds_in_a_row_never_trip_the_hold() {
        // Every `install.sh` yields a new content: no pair ever swaps twice.
        let hold = HoldState::default();
        let now = 100 * MIN;
        let history = vec![swap(now - 3 * MIN, A, B), swap(now - 2 * MIN, B, C)];
        assert_eq!(
            hold.assess(now, C, &peer(Fingerprint { size: 40, hash: 4 }), || history
                .clone()),
            Verdict::Yield
        );
    }

    #[test]
    fn an_active_hold_serves_any_differing_client_and_skips_the_history() {
        let hold = HoldState::default();
        let now = 100 * MIN;
        let history = vec![swap(now - 2 * MIN, A, B), swap(now - MIN, B, A)];
        hold.assess(now, A, &peer(B), || history.clone());

        // A third binary is held as well; the history is not consulted again.
        let verdict = hold.assess(now + MIN, A, &peer(C), || {
            panic!("history must not be read while holding")
        });
        assert_eq!(
            verdict,
            Verdict::Hold {
                started: false,
                log: false,
                shared: 2
            }
        );
        assert_eq!(hold.snapshot(now + MIN).unwrap().peer.fingerprint, C);
    }

    #[test]
    fn hold_logs_again_after_the_log_interval() {
        let hold = HoldState::default();
        let now = 100 * MIN;
        let history = vec![swap(now - 2 * MIN, A, B), swap(now - MIN, B, A)];
        hold.assess(now, A, &peer(B), || history.clone());
        let later = now + HOLD_LOG_EVERY_MS;
        assert_eq!(
            hold.assess(later, A, &peer(B), Vec::new),
            Verdict::Hold {
                started: false,
                log: true,
                shared: 2
            }
        );
    }

    #[test]
    fn hold_expires_after_the_conflict_goes_quiet() {
        let hold = HoldState::default();
        let now = 100 * MIN;
        let history = vec![swap(now - 2 * MIN, A, B), swap(now - MIN, B, A)];
        hold.assess(now, A, &peer(B), || history.clone());
        assert!(hold.snapshot(now + HOLD_QUIET_MS).is_some());
        assert!(hold.snapshot(now + HOLD_QUIET_MS + 1).is_none());

        // Long after, the old swaps fall out of the window too, so the daemon gives way again.
        let after = now + HOLD_QUIET_MS + 1;
        assert_eq!(
            hold.assess(after, A, &peer(B), || history.clone()),
            Verdict::Yield
        );
    }

    #[test]
    fn only_a_differing_client_under_the_same_protocol_can_be_held() {
        assert!(holdable(false, false));
        assert!(!holdable(true, false), "own binary changed = an upgrade");
        assert!(!holdable(false, true), "another protocol cannot share");
        assert!(!holdable(true, true));
    }

    #[test]
    fn disk_change_outranks_the_other_reasons() {
        assert_eq!(trigger_for(true, true), Trigger::Disk);
        assert_eq!(trigger_for(true, false), Trigger::Disk);
        assert_eq!(trigger_for(false, true), Trigger::Protocol);
        assert_eq!(trigger_for(false, false), Trigger::Client);
    }

    #[test]
    fn history_round_trips_and_is_pruned_by_age_and_count() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state").join("daemon-restarts.json");
        assert!(load_from(&path).is_empty());

        let day = 24 * 60 * MIN;
        record_to(&path, swap(10 * day, A, B)).unwrap();
        record_to(&path, swap(10 * day + MIN, B, A)).unwrap();
        let loaded = load_from(&path);
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded[0].to, B);
        assert_eq!(loaded[1].client.as_ref().unwrap().pid, 4242);

        // A write more than a day later drops what is older than the retention.
        record_to(&path, swap(12 * day, A, C)).unwrap();
        let loaded = load_from(&path);
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].to, C);

        // The count cap keeps the newest entries.
        for i in 0..(KEEP_MAX as u64 + 5) {
            record_to(&path, swap(12 * day + i + 1, A, B)).unwrap();
        }
        let loaded = load_from(&path);
        assert_eq!(loaded.len(), KEEP_MAX);
        assert_eq!(loaded.last().unwrap().at_ms, 12 * day + KEEP_MAX as u64 + 5);
    }

    #[test]
    fn one_daemon_giving_way_is_one_swap_however_many_hellos_arrive_meanwhile() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("daemon-restarts.json");
        // The daemon (pid 1) is shutting down while two more clients of the same binary say Hello.
        for at_ms in [MIN, MIN + 50, MIN + 90] {
            let mut again = swap(at_ms, A, B);
            again.daemon_pid = 1;
            record_to(&path, again).unwrap();
        }
        assert_eq!(load_from(&path).len(), 1);

        // The next daemon (another pid) giving way is a second swap.
        let mut next = swap(2 * MIN, B, A);
        next.daemon_pid = 2;
        record_to(&path, next).unwrap();
        assert_eq!(load_from(&path).len(), 2);
        assert_eq!(swaps(&load_from(&path), 3 * MIN, A, B), 2);
    }

    #[test]
    fn an_unreadable_history_is_an_empty_history() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("daemon-restarts.json");
        std::fs::write(&path, b"not json").unwrap();
        assert!(load_from(&path).is_empty());
        // ...and the next record replaces it.
        record_to(&path, swap(1, A, B)).unwrap();
        assert_eq!(load_from(&path).len(), 1);
    }
}
