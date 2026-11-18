//! The shared connection of one channel: an IM router (Slack, Telegram, Feishu, DingTalk) or the
//! iOS gateway.
//!
//! A connection is made lazily and then reused by every request. Making it is network (or at least
//! subprocess and disk) work and takes as long as that takes. Everything that has to answer
//! promptly, such as the daemon's `Status` reply, the tray state and shutdown, therefore reads the
//! slot without ever waiting for a connect. The installed connection sits behind a
//! `std::sync::Mutex` that is never held across an await; only the act of connecting is serialised,
//! by a separate async gate that those readers never touch.
//!
//! The previous shape was one lock held across the whole connect. When a Slack WebSocket handshake
//! hung after a live update, `Status` (which listed the live connections) queued behind that lock
//! and the daemon looked dead to every client.
//!
//! Two kinds of caller do wait for a connect on purpose, through [`ConnSlot::settled`]: live-update
//! preparation, which has to pause what is about to be installed before the process is replaced,
//! and detection, which reuses the shared router instead of competing with it for the same account.
//! Both are bounded by the connect's own timeouts and neither is on the `Status` path.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

pub(super) struct ConnSlot<T> {
    state: Mutex<State<T>>,
    /// Held for the duration of one connect so two requests do not connect the same channel twice.
    gate: tokio::sync::Mutex<()>,
}

struct State<T> {
    conn: Option<Arc<T>>,
    /// Bumped by every invalidation. A connect whose caller arrived before it must not install.
    generation: u64,
}

impl<T> Default for ConnSlot<T> {
    fn default() -> Self {
        Self {
            state: Mutex::new(State {
                conn: None,
                generation: 0,
            }),
            gate: tokio::sync::Mutex::new(()),
        }
    }
}

impl<T> ConnSlot<T> {
    fn state(&self) -> MutexGuard<'_, State<T>> {
        // Nothing under this lock can panic halfway: a poisoned guard still holds a valid state.
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The installed connection, alive or not. Never waits for a connect that is in progress.
    pub(super) fn current(&self) -> Option<Arc<T>> {
        self.state().conn.clone()
    }

    /// The installed connection once the connect in progress, if any, has finished. A connect that
    /// finds a handoff under way pauses its own connection before it lets go of the gate, so what
    /// this returns is everything the handoff has to pause.
    pub(super) async fn settled(&self) -> Option<Arc<T>> {
        let _gate = self.gate.lock().await;
        self.current()
    }

    /// Forget the installed connection (credentials changed, channel disabled, handoff, shutdown)
    /// and make every connect whose caller arrived before this call install nothing. Never waits
    /// for a connect. The removed connection is returned so a caller that has to shut it down
    /// properly can; dropping it is enough for a router.
    pub(super) fn invalidate(&self) -> Option<Arc<T>> {
        let mut state = self.state();
        state.generation += 1;
        state.conn.take()
    }

    /// Wait for the right to connect. Callers that want a connection queue here; readers of the
    /// slot never do.
    pub(super) async fn begin_connect(&self) -> ConnectAttempt<'_, T> {
        // Read before queueing: an invalidation that lands while this caller waits for the gate is
        // a change its arguments predate, so what it connects with must not become the shared one.
        let generation = self.state().generation;
        let gate = self.gate.lock().await;
        ConnectAttempt {
            slot: self,
            generation,
            _gate: gate,
        }
    }
}

/// The right to connect one connection. It is held until the attempt is dropped, so a caller that
/// has more to do with the connection after installing it (pausing it during a handoff) keeps
/// every other connect, and live-update preparation, waiting for that.
pub(super) struct ConnectAttempt<'a, T> {
    slot: &'a ConnSlot<T>,
    generation: u64,
    _gate: tokio::sync::MutexGuard<'a, ()>,
}

/// What became of a connection that was made.
pub(super) enum Installed<T> {
    /// It is now the shared connection of the slot. `replaced` is what it displaced.
    Shared { replaced: Option<Arc<T>> },
    /// The slot was invalidated since the caller arrived (new credentials, channel disabled): the
    /// connection serves the request that made it and nothing else.
    Private,
    /// A live-update handoff began. The connection must be paused, not used, so it cannot consume
    /// messages the successor daemon is about to own.
    Suspended,
}

impl<T> ConnectAttempt<'_, T> {
    /// The connection another request installed while this attempt waited for its turn, if any.
    pub(super) fn current(&self) -> Option<Arc<T>> {
        self.slot.current()
    }

    /// Install `conn` unless a handoff began or the slot was invalidated since the caller arrived.
    /// `suspended` is evaluated under the slot lock, so it is ordered against the handoff, which
    /// raises its flag before it reads the slot: either the handoff sees this connection and pauses
    /// it, or this call sees the flag.
    pub(super) fn install(&self, conn: &Arc<T>, suspended: impl FnOnce() -> bool) -> Installed<T> {
        let mut state = self.slot.state();
        if suspended() {
            return Installed::Suspended;
        }
        if state.generation != self.generation {
            return Installed::Private;
        }
        Installed::Shared {
            replaced: state.conn.replace(conn.clone()),
        }
    }

    /// Empty the slot because this caller found that no connection should exist (the channel was
    /// switched off). Unlike [`ConnSlot::invalidate`] it leaves the generation alone: every other
    /// connect is queued behind this attempt and decides afresh from whatever is current by then,
    /// so none of them is made stale by this removal. The removed connection is returned.
    pub(super) fn clear(&self) -> Option<Arc<T>> {
        self.slot.state().conn.take()
    }

    /// The connect failed: drop the dead connection that made it necessary, unless the slot was
    /// invalidated meanwhile (then it is already gone).
    pub(super) fn failed(&self) {
        let removed = {
            let mut state = self.slot.state();
            if state.generation == self.generation {
                state.conn.take()
            } else {
                None
            }
        };
        // Dropped after the lock is released: dropping a router stops its tasks.
        drop(removed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    struct Fake(&'static str);

    fn shared<T>(outcome: Installed<T>) -> Option<Arc<T>> {
        match outcome {
            Installed::Shared { replaced } => replaced,
            Installed::Private => panic!("expected the connection to become shared, got Private"),
            Installed::Suspended => {
                panic!("expected the connection to become shared, got Suspended")
            }
        }
    }

    #[tokio::test]
    async fn readers_never_wait_for_a_connect_in_progress() {
        let slot = ConnSlot::<Fake>::default();
        let attempt = slot.begin_connect().await;

        // These are plain synchronous calls: there is nothing to await, so they cannot queue.
        assert!(slot.current().is_none());
        slot.invalidate();
        assert!(attempt.current().is_none());

        // A second connect does queue, until the first attempt is over.
        let waiting = tokio::time::timeout(Duration::from_millis(50), slot.begin_connect()).await;
        assert!(waiting.is_err(), "only one connect runs at a time");
        drop(attempt);
        let _next = tokio::time::timeout(Duration::from_secs(5), slot.begin_connect())
            .await
            .expect("the gate is free once the attempt is dropped");
    }

    #[tokio::test]
    async fn an_installed_connection_is_shared_and_found_by_the_next_attempt() {
        let slot = ConnSlot::default();
        let conn = Arc::new(Fake("first"));

        let attempt = slot.begin_connect().await;
        assert!(attempt.current().is_none());
        assert!(shared(attempt.install(&conn, || false)).is_none());
        drop(attempt);

        assert!(Arc::ptr_eq(&slot.current().unwrap(), &conn));
        let second = slot.begin_connect().await;
        assert!(Arc::ptr_eq(&second.current().unwrap(), &conn));
    }

    #[tokio::test]
    async fn installing_hands_back_what_it_displaced() {
        let slot = ConnSlot::default();
        let first = Arc::new(Fake("first"));
        let second = Arc::new(Fake("second"));

        let attempt = slot.begin_connect().await;
        shared(attempt.install(&first, || false));
        let replaced = shared(attempt.install(&second, || false)).expect("the first one");

        assert!(Arc::ptr_eq(&replaced, &first));
        assert!(Arc::ptr_eq(&slot.current().unwrap(), &second));
    }

    #[tokio::test]
    async fn a_slot_invalidated_while_connecting_keeps_the_connection_private() {
        let slot = ConnSlot::default();
        let conn = Arc::new(Fake("old credentials"));

        let attempt = slot.begin_connect().await;
        slot.invalidate();

        assert!(matches!(
            attempt.install(&conn, || false),
            Installed::Private
        ));
        assert!(slot.current().is_none());
        assert_eq!(
            conn.0, "old credentials",
            "the connecting request still has it"
        );
    }

    /// The caller that queued for the gate behind a connect, then saw the configuration change,
    /// holds arguments the change has outdated. Counting the generation from the moment it
    /// arrived, not from the moment it got the gate, keeps what it connects from becoming shared.
    #[tokio::test]
    async fn a_caller_that_queued_before_an_invalidation_never_installs() {
        let slot = ConnSlot::default();
        let first = slot.begin_connect().await;

        let queued = {
            let slot = &slot;
            let mut waiting = Box::pin(slot.begin_connect());
            // Poll once so the caller has arrived (and read the generation) but still waits.
            assert!(
                tokio::time::timeout(Duration::from_millis(20), &mut waiting)
                    .await
                    .is_err()
            );
            waiting
        };

        slot.invalidate();
        drop(first);
        let attempt = queued.await;
        let stale = Arc::new(Fake("connected with the old configuration"));

        assert!(matches!(
            attempt.install(&stale, || false),
            Installed::Private
        ));
        assert!(
            slot.current().is_none(),
            "no stale connection became shared"
        );

        // A caller that arrives after the invalidation is current.
        drop(attempt);
        let later = slot.begin_connect().await;
        let fresh = Arc::new(Fake("new configuration"));
        assert!(shared(later.install(&fresh, || false)).is_none());
    }

    #[tokio::test]
    async fn a_handoff_while_connecting_installs_nothing_even_if_the_slot_was_invalidated() {
        let slot = ConnSlot::default();

        let attempt = slot.begin_connect().await;
        let conn = Arc::new(Fake("late"));
        assert!(matches!(
            attempt.install(&conn, || true),
            Installed::Suspended
        ));
        assert!(slot.current().is_none());
        drop(attempt);

        // A request that connected under a handoff is never "private": the creator must pause it,
        // because nothing else will.
        let attempt = slot.begin_connect().await;
        slot.invalidate();
        assert!(matches!(
            attempt.install(&conn, || true),
            Installed::Suspended
        ));
        assert!(slot.current().is_none());
    }

    #[tokio::test]
    async fn the_handoff_flag_is_read_under_the_slot_lock() {
        let slot = ConnSlot::default();
        let conn = Arc::new(Fake("conn"));
        let attempt = slot.begin_connect().await;

        let mut observed_lock_held = false;
        let outcome = attempt.install(&conn, || {
            // While the flag is evaluated the slot lock is held, so a handoff reading the slot
            // (`current`) cannot interleave between the flag check and the install.
            observed_lock_held = slot.state.try_lock().is_err();
            false
        });

        assert!(observed_lock_held);
        assert!(matches!(outcome, Installed::Shared { .. }));
    }

    #[tokio::test]
    async fn a_failed_connect_drops_the_dead_connection_it_replaces() {
        let slot = ConnSlot::default();
        let dead = Arc::new(Fake("dead"));
        let attempt = slot.begin_connect().await;
        shared(attempt.install(&dead, || false));
        drop(attempt);

        let reconnect = slot.begin_connect().await;
        assert!(reconnect.current().is_some());
        reconnect.failed();

        assert!(slot.current().is_none());
        assert_eq!(Arc::strong_count(&dead), 1, "the slot let go of it");
    }

    #[tokio::test]
    async fn a_failed_connect_leaves_an_invalidated_slot_alone() {
        let slot = ConnSlot::default();
        let attempt = slot.begin_connect().await;
        slot.invalidate();
        attempt.failed();
        assert!(slot.current().is_none());
        drop(attempt);
        // And the slot still works afterwards.
        let conn = Arc::new(Fake("next"));
        let attempt = slot.begin_connect().await;
        assert!(shared(attempt.install(&conn, || false)).is_none());
    }

    #[tokio::test]
    async fn invalidate_returns_what_it_removed() {
        let slot = ConnSlot::default();
        assert!(slot.invalidate().is_none());

        let conn = Arc::new(Fake("running"));
        let attempt = slot.begin_connect().await;
        shared(attempt.install(&conn, || false));
        drop(attempt);

        let removed = slot.invalidate().expect("it was installed");
        assert!(Arc::ptr_eq(&removed, &conn));
        assert!(slot.current().is_none());
    }

    #[tokio::test]
    async fn clearing_empties_the_slot_without_making_queued_callers_stale() {
        let slot = ConnSlot::default();
        let conn = Arc::new(Fake("running"));
        let attempt = slot.begin_connect().await;
        shared(attempt.install(&conn, || false));
        drop(attempt);

        // A caller that is already queued when another one clears the slot (the channel was off).
        let clearing = slot.begin_connect().await;
        let mut queued = Box::pin(slot.begin_connect());
        assert!(tokio::time::timeout(Duration::from_millis(20), &mut queued)
            .await
            .is_err());

        let removed = clearing.clear().expect("it was installed");
        assert!(Arc::ptr_eq(&removed, &conn));
        assert!(slot.current().is_none());
        drop(clearing);

        // The queued caller reads its inputs afresh, so what it builds is current, not stale.
        let next = queued.await;
        let fresh = Arc::new(Fake("switched back on"));
        assert!(shared(next.install(&fresh, || false)).is_none());
        assert!(Arc::ptr_eq(&slot.current().unwrap(), &fresh));
    }

    /// Live-update preparation and detection read through `settled`: they wait out the connect in
    /// progress and then see what it left, while `current` (Status, tray) answers at once.
    #[tokio::test]
    async fn settled_waits_for_the_connect_in_progress_and_current_does_not() {
        let slot = Arc::new(ConnSlot::default());
        let attempt = slot.begin_connect().await;

        assert!(slot.current().is_none(), "a reader never waits");
        let waiter = {
            let slot = slot.clone();
            tokio::spawn(async move { slot.settled().await })
        };
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(!waiter.is_finished(), "settled is waiting for the connect");

        let conn = Arc::new(Fake("connected"));
        shared(attempt.install(&conn, || false));
        // The creator may still pause what it installed: the gate stays closed until it is done.
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(!waiter.is_finished(), "the attempt still holds the gate");
        drop(attempt);

        let seen = tokio::time::timeout(Duration::from_secs(5), waiter)
            .await
            .expect("settled returns once the gate is free")
            .unwrap()
            .expect("it sees what the connect installed");
        assert!(Arc::ptr_eq(&seen, &conn));
    }
}
