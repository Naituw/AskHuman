//! Generation-scoped ownership of the instance's shared Popup Host.
use crate::ipc::ServerMsg;
use std::collections::{BTreeMap, VecDeque};
use std::time::{Duration, Instant};
use tokio::sync::mpsc::UnboundedSender;

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Lease {
    pub generation: u64,
    pub token: String,
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct SavedHost {
    pub lease: Lease,
    pub pid: u32,
    #[serde(default)]
    pub birth: Option<String>,
}
enum Phase {
    Absent,
    Starting(Lease),
    Connected(Lease, UnboundedSender<ServerMsg>),
}
#[derive(Clone)]
pub struct Assignment<T> {
    pub id: String,
    pub sequence: u64,
    pub value: T,
}
pub enum Recovery<T> {
    Stale,
    Idle,
    Restart(Lease),
    Failed(Vec<Assignment<T>>),
}

/// What asking for a host does right now.
pub enum Launch {
    /// A host was reserved; the caller starts the process for this lease.
    Started(Lease),
    /// One is already starting or connected.
    Running,
    /// Too many hosts failed recently (the same budget recovery uses): start none.
    Refused,
}

/// Hosts that may fail in a row within [`FAILURE_WINDOW`] before no more are started.
const FAILURE_BUDGET: usize = 2;
const FAILURE_WINDOW: Duration = Duration::from_secs(30);

pub struct Host<T> {
    phase: Phase,
    generation: u64,
    requests: BTreeMap<String, Assignment<T>>,
    failures: VecDeque<Instant>,
    recovered: bool,
    focus: Option<String>,
    recovery_pid: Option<u32>,
    recovery_birth: Option<String>,
    /// Ends the host process this daemon started for the lease that is still `Starting`; fired
    /// when the daemon gives that lease up before the host authenticated, dropped when it does.
    /// Present only while `phase` is `Starting` with a host this daemon owns (a host kept across a
    /// live update is not).
    owned_child: Option<std::sync::Arc<tokio::sync::Notify>>,
}
impl<T> Default for Host<T> {
    fn default() -> Self {
        Self {
            phase: Phase::Absent,
            generation: 0,
            requests: BTreeMap::new(),
            failures: VecDeque::new(),
            recovered: false,
            focus: None,
            recovery_pid: None,
            recovery_birth: None,
            owned_child: None,
        }
    }
}
impl<T: Clone> Host<T> {
    pub fn restore(saved: SavedHost) -> Self {
        Self {
            generation: saved.lease.generation,
            phase: Phase::Starting(saved.lease),
            recovery_pid: Some(saved.pid),
            recovery_birth: saved.birth,
            recovered: false,
            ..Self::default()
        }
    }

    pub fn enable_recovery(&mut self, pid: u32) {
        self.recovery_pid = Some(pid);
        self.recovery_birth = crate::daemon::recovery::process_birth(pid);
    }

    pub fn saved(&self) -> Option<SavedHost> {
        let lease = match &self.phase {
            Phase::Starting(lease) | Phase::Connected(lease, _) => lease,
            Phase::Absent => return None,
        };
        Some(SavedHost {
            lease: lease.clone(),
            pid: self.recovery_pid?,
            birth: self.recovery_birth.clone(),
        })
    }

    pub fn can_suspend(&self) -> bool {
        self.requests.is_empty() || self.saved().is_some()
    }

    pub fn detach(&mut self) {
        if let Phase::Connected(lease, _) = &self.phase {
            self.phase = Phase::Starting(lease.clone());
        }
    }

    /// Reserve a host. Only [`Host::launch`] and [`Host::disconnected`] do this outside tests, so
    /// no caller can start hosts without the failure budget applying.
    fn begin(&mut self) -> Option<Lease> {
        if !matches!(self.phase, Phase::Absent) {
            return None;
        }
        self.generation = self.generation.wrapping_add(1);
        let lease = Lease {
            generation: self.generation,
            token: uuid::Uuid::new_v4().to_string(),
        };
        self.phase = Phase::Starting(lease.clone());
        self.recovery_pid = None;
        self.recovery_birth = None;
        self.owned_child = Some(Default::default());
        Some(lease)
    }

    /// Reserve a host regardless of the failure budget, to set up a phase.
    #[cfg(test)]
    pub fn start(&mut self) -> Option<Lease> {
        self.begin()
    }

    /// Start a host for a request or a prewarm, unless one exists or the recent failures say that
    /// starting another would only make things worse. Every way of starting a host except
    /// recovery's own restart goes through here, and recovery applies the same budget in
    /// [`Host::disconnected`], so a failing host cannot be restarted faster by a different path.
    pub fn launch(&mut self, now: Instant) -> Launch {
        if !matches!(self.phase, Phase::Absent) {
            return Launch::Running;
        }
        if self.over_budget(now) {
            return Launch::Refused;
        }
        Launch::Started(self.begin().expect("an absent host starts"))
    }

    /// Whether [`Host::launch`] would refuse right now. For callers that must not commit to a
    /// popup surface (register a channel, spend a delivery attempt) they could not back with a host.
    pub fn would_refuse(&mut self, now: Instant) -> bool {
        matches!(self.phase, Phase::Absent) && self.over_budget(now)
    }

    fn expire_failures(&mut self, now: Instant) {
        while self
            .failures
            .front()
            .is_some_and(|failure| now.duration_since(*failure) >= FAILURE_WINDOW)
        {
            self.failures.pop_front();
        }
    }

    fn over_budget(&mut self, now: Instant) -> bool {
        self.expire_failures(now);
        self.failures.len() > FAILURE_BUDGET
    }

    /// The signal that ends the process started for `generation`, while that lease is still the
    /// one being waited for. `None` once the host authenticated or the lease was given up: a host
    /// that is no longer expected must not be started for it.
    pub fn abandon_signal(&self, generation: u64) -> Option<std::sync::Arc<tokio::sync::Notify>> {
        match (&self.phase, &self.owned_child) {
            (Phase::Starting(lease), Some(signal)) if lease.generation == generation => {
                Some(signal.clone())
            }
            _ => None,
        }
    }

    /// The daemon gives up on the host it started: end its process. Called under the same lock
    /// as every phase change, so a host that authenticates concurrently either got in first (and
    /// has nothing to end) or never will.
    fn abandon_owned(&mut self) {
        if let Some(signal) = self.owned_child.take() {
            signal.notify_one();
        }
    }

    pub fn enqueue(&mut self, id: String, sequence: u64, value: T) -> bool {
        if self.requests.contains_key(&id) {
            return false;
        }
        self.requests.insert(
            id.clone(),
            Assignment {
                id,
                sequence,
                value,
            },
        );
        true
    }

    pub fn connect(&mut self, token: &str, tx: UnboundedSender<ServerMsg>) -> Option<(u64, bool)> {
        let Phase::Starting(lease) = &self.phase else {
            return None;
        };
        if lease.token != token {
            return None;
        }
        let lease = lease.clone();
        let generation = lease.generation;
        // Queue authentication before publishing the sender. Concurrent assignments must never
        // put Show ahead of the handshake expected by the host's bootstrap reader.
        let _ = tx.send(ServerMsg::PopupHostAccepted {
            generation,
            recovered: self.recovered,
        });
        self.phase = Phase::Connected(lease, tx);
        // Authenticated: this host is no longer the daemon's to end on a timeout.
        self.owned_child = None;
        Some((generation, self.recovered))
    }

    pub fn sender(&self) -> Option<UnboundedSender<ServerMsg>> {
        match &self.phase {
            Phase::Connected(_, tx) => Some(tx.clone()),
            _ => None,
        }
    }

    pub fn generation(&self) -> Option<u64> {
        match &self.phase {
            Phase::Absent => None,
            Phase::Starting(lease) | Phase::Connected(lease, _) => Some(lease.generation),
        }
    }

    pub fn get(&self, id: &str) -> Option<T> {
        self.requests.get(id).map(|request| request.value.clone())
    }

    pub fn requests(&self) -> Vec<Assignment<T>> {
        let mut requests: Vec<_> = self.requests.values().cloned().collect();
        requests.sort_by(|a, b| a.sequence.cmp(&b.sequence).then_with(|| a.id.cmp(&b.id)));
        requests
    }

    pub fn request_focus(&mut self, id: &str) {
        self.focus = Some(id.to_owned());
    }
    pub fn take_focus(&mut self) -> Option<String> {
        self.focus
            .take()
            .filter(|id| self.requests.contains_key(id))
    }
    pub fn remove(&mut self, id: &str) {
        self.requests.remove(id);
    }

    /// An idle host may retire only after every daemon-side assignment has ended. A request
    /// arriving before this check keeps the same host; one arriving afterwards starts a new lease.
    pub fn retire_idle(&mut self, generation: u64) -> bool {
        if self.generation() != Some(generation) || !self.requests.is_empty() {
            return false;
        }
        self.abandon_owned();
        self.phase = Phase::Absent;
        self.recovered = false;
        self.failures.clear();
        true
    }

    /// Limit recovery to two restarts in thirty seconds. Neither an old connection's EOF nor
    /// its startup timeout may retire a newer host or consume its restart budget.
    pub fn disconnected(&mut self, generation: u64, now: Instant, prewarm: bool) -> Recovery<T> {
        if self.generation() != Some(generation) {
            return Recovery::Stale;
        }
        let was_connected =
            matches!(self.phase, Phase::Connected(_, _)) || self.recovery_pid.is_some();
        // A host that never authenticated is still running somewhere; it must not outlive its lease.
        self.abandon_owned();
        self.phase = Phase::Absent;
        if self.requests.is_empty() && !prewarm {
            return Recovery::Idle;
        }
        self.expire_failures(now);
        self.failures.push_back(now);
        if self.failures.len() > FAILURE_BUDGET {
            return Recovery::Failed(std::mem::take(&mut self.requests).into_values().collect());
        }
        self.recovered = !self.requests.is_empty() && (self.recovered || was_connected);
        Recovery::Restart(self.begin().expect("disconnected host is absent"))
    }

    /// The host of `generation` did not authenticate in time. Unlike [`Host::disconnected`] this
    /// acts only on a lease that is still waiting, in the same step that [`Host::connect`] would
    /// have accepted the host in: a host that authenticated a moment before is left alone.
    pub fn startup_timed_out(
        &mut self,
        generation: u64,
        now: Instant,
        prewarm: bool,
    ) -> Recovery<T> {
        match &self.phase {
            Phase::Starting(lease) if lease.generation == generation => {
                self.disconnected(generation, now, prewarm)
            }
            _ => Recovery::Stale,
        }
    }

    pub fn stop_idle(&mut self) -> Option<UnboundedSender<ServerMsg>> {
        if !self.requests.is_empty() {
            return None;
        }
        let tx = self.sender();
        self.abandon_owned();
        self.phase = Phase::Absent;
        tx
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn concurrent_requests_reserve_only_one_host() {
        let host = std::sync::Arc::new(std::sync::Mutex::new(Host::<()>::default()));
        let workers: Vec<_> = (0..12)
            .map(|i| {
                let host = host.clone();
                std::thread::spawn(move || {
                    let mut host = host.lock().unwrap();
                    host.enqueue(format!("r{i}"), i, ());
                    host.start().is_some()
                })
            })
            .collect();
        assert_eq!(
            workers
                .into_iter()
                .map(|worker| worker.join().unwrap())
                .filter(|launched| *launched)
                .count(),
            1
        );
        assert_eq!(host.lock().unwrap().requests().len(), 12);
    }
    #[test]
    fn authentication_is_single_use_and_snapshot_is_ordered() {
        let mut host = Host::default();
        host.enqueue("later".into(), 8, ());
        host.enqueue("first".into(), 3, ());
        let lease = host.start().unwrap();
        let (tx, _) = tokio::sync::mpsc::unbounded_channel();
        assert!(host.connect("wrong", tx.clone()).is_none());
        assert_eq!(
            host.connect(&lease.token, tx.clone()),
            Some((lease.generation, false))
        );
        assert!(host.connect(&lease.token, tx).is_none());
        assert_eq!(
            host.requests()
                .iter()
                .map(|r| r.id.as_str())
                .collect::<Vec<_>>(),
            ["first", "later"]
        );
    }
    #[test]
    fn retaining_a_host_for_handoff_preserves_the_lease_without_a_reopened_notice() {
        let mut host = Host::<()>::restore(SavedHost {
            lease: Lease {
                generation: 7,
                token: "saved".into(),
            },
            pid: std::process::id(),
            birth: None,
        });
        host.enqueue("r".into(), 1, ());
        let (tx, _) = tokio::sync::mpsc::unbounded_channel();
        assert_eq!(host.connect("saved", tx), Some((7, false)));
    }

    #[test]
    fn a_retained_host_that_cannot_reconnect_still_warns_when_rebuilt() {
        let mut host = Host::<()>::restore(SavedHost {
            lease: Lease {
                generation: 7,
                token: "saved".into(),
            },
            pid: std::process::id(),
            birth: None,
        });
        host.enqueue("r".into(), 1, ());
        let Recovery::Restart(lease) = host.disconnected(7, Instant::now(), true) else {
            panic!("host must restart");
        };
        let (tx, _) = tokio::sync::mpsc::unbounded_channel();
        assert_eq!(
            host.connect(&lease.token, tx),
            Some((lease.generation, true))
        );
    }

    #[test]
    fn only_connected_hosts_with_pending_requests_need_a_reopened_notice() {
        for connected in [false, true] {
            let mut host = Host::default();
            host.enqueue("r".into(), 1, ());
            let first = host.start().unwrap();
            if connected {
                let (tx, _) = tokio::sync::mpsc::unbounded_channel();
                host.connect(&first.token, tx).unwrap();
            }
            let Recovery::Restart(second) =
                host.disconnected(first.generation, Instant::now(), true)
            else {
                panic!("host must restart");
            };
            let (tx, _) = tokio::sync::mpsc::unbounded_channel();
            assert_eq!(
                host.connect(&second.token, tx),
                Some((second.generation, connected))
            );
        }
    }

    #[test]
    fn stale_disconnect_cannot_replace_recovery_host() {
        let mut host = Host::default();
        host.enqueue("r".into(), 1, ());
        let first = host.start().unwrap();
        let Recovery::Restart(second) = host.disconnected(first.generation, Instant::now(), false)
        else {
            panic!()
        };
        assert!(matches!(
            host.disconnected(first.generation, Instant::now(), false),
            Recovery::Stale
        ));
        assert_eq!(host.generation(), Some(second.generation));
        assert_eq!(host.requests().len(), 1);
    }
    #[test]
    fn acceptance_precedes_messages_from_the_published_sender() {
        let mut host = Host::<()>::default();
        let lease = host.start().unwrap();
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        host.connect(&lease.token, tx).unwrap();
        host.sender()
            .unwrap()
            .send(ServerMsg::PopupHostShutdown)
            .unwrap();
        assert!(matches!(
            rx.try_recv().unwrap(),
            ServerMsg::PopupHostAccepted { .. }
        ));
        assert!(matches!(
            rx.try_recv().unwrap(),
            ServerMsg::PopupHostShutdown
        ));
    }
    #[test]
    fn idle_close_does_not_drop_an_arriving_request() {
        let mut host = Host::default();
        let first = host.start().unwrap();
        host.enqueue("r".into(), 1, ());
        assert!(!host.retire_idle(first.generation));
        host.remove("r");
        assert!(host.retire_idle(first.generation));
        host.enqueue("next".into(), 2, ());
        let next = host.start().unwrap();
        assert!(!host.retire_idle(first.generation));
        assert_eq!(host.generation(), Some(next.generation));
    }
    /// Whether the signal that ends a host process has been fired (consumes the permit).
    fn fired(signal: &std::sync::Arc<tokio::sync::Notify>) -> bool {
        use futures_util::FutureExt;
        signal.notified().now_or_never().is_some()
    }

    #[test]
    fn a_host_that_never_authenticated_is_ended_with_its_lease() {
        let mut host = Host::<()>::default();
        host.enqueue("r".into(), 1, ());
        let lease = host.start().unwrap();
        let signal = host.abandon_signal(lease.generation).expect("owned");
        assert!(
            !fired(&signal),
            "a host still being waited for is left alone"
        );

        let Recovery::Restart(next) =
            host.startup_timed_out(lease.generation, Instant::now(), false)
        else {
            panic!("the request needs another host");
        };
        assert!(fired(&signal), "the timed-out host's process is ended");

        // The replacement has a signal of its own, which the old lease's host cannot fire.
        let replacement = host.abandon_signal(next.generation).expect("owned");
        assert!(!std::sync::Arc::ptr_eq(&signal, &replacement));
        assert!(!fired(&replacement));
        assert!(host.abandon_signal(lease.generation).is_none());
    }

    #[test]
    fn a_host_that_authenticated_first_is_never_ended_by_the_timeout() {
        let mut host = Host::<()>::default();
        host.enqueue("r".into(), 1, ());
        let lease = host.start().unwrap();
        let signal = host.abandon_signal(lease.generation).unwrap();
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        host.connect(&lease.token, tx).unwrap();

        // The timer fires after the host got in: nothing happens to it.
        assert!(matches!(
            host.startup_timed_out(lease.generation, Instant::now(), false),
            Recovery::Stale
        ));
        assert!(host.sender().is_some(), "still connected");
        assert!(host.abandon_signal(lease.generation).is_none());
        assert!(!fired(&signal));

        // Later it dies on its own: recovery restarts it, and still nobody ends the old process.
        assert!(matches!(
            host.disconnected(lease.generation, Instant::now(), false),
            Recovery::Restart(_)
        ));
        assert!(!fired(&signal));
    }

    #[test]
    fn the_timeout_and_the_authentication_of_one_lease_cannot_both_win() {
        // The host authenticates after the timeout gave its lease up: it is turned away, and the
        // process that would have used the lease was already ordered to end.
        let mut host = Host::<()>::default();
        host.enqueue("r".into(), 1, ());
        let lease = host.start().unwrap();
        let signal = host.abandon_signal(lease.generation).unwrap();
        assert!(matches!(
            host.startup_timed_out(lease.generation, Instant::now(), false),
            Recovery::Restart(_)
        ));
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        assert!(host.connect(&lease.token, tx).is_none());
        assert!(fired(&signal));
    }

    #[test]
    fn retiring_a_host_that_has_not_connected_ends_its_process() {
        let mut host = Host::<()>::default();
        let lease = host.start().unwrap();
        let signal = host.abandon_signal(lease.generation).unwrap();
        // A prewarm host, nobody waiting for it: recycling it must not leave the process behind.
        assert!(host.stop_idle().is_none(), "no connection to tell");
        assert!(fired(&signal));
        assert!(host.generation().is_none());
    }

    #[test]
    fn a_host_kept_across_an_update_is_not_the_daemons_to_end() {
        let mut host = Host::<()>::restore(SavedHost {
            lease: Lease {
                generation: 7,
                token: "saved".into(),
            },
            pid: std::process::id(),
            birth: None,
        });
        assert!(host.abandon_signal(7).is_none());
        // Giving it up still works; there is just no process of ours to end.
        assert!(matches!(
            host.disconnected(7, Instant::now(), true),
            Recovery::Restart(_)
        ));
    }

    #[test]
    fn every_way_of_starting_a_host_shares_the_failure_budget() {
        let mut host = Host::<()>::default();
        host.enqueue("r".into(), 1, ());
        let t0 = Instant::now();
        let Launch::Started(mut lease) = host.launch(t0) else {
            panic!("a first host starts");
        };
        assert!(matches!(host.launch(t0), Launch::Running));
        assert!(!host.would_refuse(t0), "a host is already starting");

        // Two restarts are allowed; the third failure gives the request up.
        for _ in 0..2 {
            let Recovery::Restart(next) = host.disconnected(lease.generation, t0, false) else {
                panic!("within the budget");
            };
            lease = next;
        }
        assert!(matches!(
            host.disconnected(lease.generation, t0, false),
            Recovery::Failed(_)
        ));

        // A new request must not start another host straight away: that is how a broken host
        // used to be respawned every few seconds without limit.
        let soon = t0 + Duration::from_secs(6);
        assert!(host.would_refuse(soon));
        assert!(matches!(host.launch(soon), Launch::Refused));
        assert!(host.generation().is_none(), "nothing was started");

        // The failures age out of the window and starting is allowed again.
        let later = t0 + Duration::from_secs(31);
        assert!(!host.would_refuse(later));
        assert!(matches!(host.launch(later), Launch::Started(_)));
    }

    #[test]
    fn a_host_that_lived_resets_the_failure_budget() {
        let mut host = Host::<()>::default();
        let t0 = Instant::now();
        let mut lease = host.start().unwrap();
        host.enqueue("r".into(), 1, ());
        for _ in 0..2 {
            let Recovery::Restart(next) = host.disconnected(lease.generation, t0, false) else {
                panic!("within the budget");
            };
            lease = next;
        }
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        host.connect(&lease.token, tx).unwrap();
        host.remove("r");
        assert!(host.retire_idle(lease.generation));
        assert!(
            !host.would_refuse(t0),
            "a healthy host cleared the failures"
        );
    }

    #[test]
    fn repeated_crashes_fail_surfaces_after_bounded_recovery() {
        let mut host = Host::default();
        host.enqueue("r".into(), 1, ());
        let now = Instant::now();
        let mut lease = host.start().unwrap();
        for _ in 0..2 {
            let Recovery::Restart(next) = host.disconnected(lease.generation, now, false) else {
                panic!()
            };
            lease = next;
        }
        let Recovery::Failed(requests) = host.disconnected(lease.generation, now, false) else {
            panic!()
        };
        assert_eq!(requests[0].id, "r");
        assert!(host.requests().is_empty());
    }
}
