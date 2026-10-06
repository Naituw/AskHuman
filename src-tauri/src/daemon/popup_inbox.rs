//! Generation-scoped ownership of the instance's shared Popup Host.
use crate::ipc::ServerMsg;
use std::collections::{BTreeMap, VecDeque};
use std::time::{Duration, Instant};
use tokio::sync::mpsc::UnboundedSender;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Lease {
    pub generation: u64,
    pub token: String,
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
pub struct Host<T> {
    phase: Phase,
    generation: u64,
    requests: BTreeMap<String, Assignment<T>>,
    failures: VecDeque<Instant>,
    recovered: bool,
    focus: Option<String>,
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
        }
    }
}
impl<T: Clone> Host<T> {
    pub fn start(&mut self) -> Option<Lease> {
        if !matches!(self.phase, Phase::Absent) {
            return None;
        }
        self.generation = self.generation.wrapping_add(1);
        let lease = Lease {
            generation: self.generation,
            token: uuid::Uuid::new_v4().to_string(),
        };
        self.phase = Phase::Starting(lease.clone());
        Some(lease)
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
        self.phase = Phase::Absent;
        if self.requests.is_empty() && !prewarm {
            return Recovery::Idle;
        }
        while self
            .failures
            .front()
            .is_some_and(|failure| now.duration_since(*failure) >= Duration::from_secs(30))
        {
            self.failures.pop_front();
        }
        self.failures.push_back(now);
        if self.failures.len() > 2 {
            return Recovery::Failed(std::mem::take(&mut self.requests).into_values().collect());
        }
        self.recovered = !self.requests.is_empty();
        Recovery::Restart(self.start().expect("disconnected host is absent"))
    }

    pub fn stop_idle(&mut self) -> Option<UnboundedSender<ServerMsg>> {
        if !self.requests.is_empty() {
            return None;
        }
        let tx = self.sender();
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
