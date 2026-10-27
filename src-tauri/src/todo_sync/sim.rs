//! Whole-system tests of the sync core: several simulated Macs — each with its own `todos.json` edited
//! through the real `todos::*_at` operations, its own `state.json` sidecar and its own clock — around
//! one in-memory cloud with injectable faults and crash points.
//!
//! The random tests print their seed; everything except the todo uuids is deterministic, so a failing
//! seed can be replayed on its own (`cargo test todo_sync::sim -- --nocapture`).

use super::links::testing::MapResolver;
use super::model::{PauseReason, St, State};
use super::tick::{self, Env, Failpoints, Opts, TickError, TickReport};
use super::transport::mem::{Cloud, Faults, MemTransport};
use super::transport::AccountStatus;
use crate::todos::{self, TodoFile};
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::rc::Rc;
use std::time::Duration;

const P1: &str = "github.com/acme/app";
const P2: &str = "github.com/acme/tools";
/// Large enough that trimming never kicks in unless a test wants it.
const LIMIT: usize = 1000;

fn real_now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock")
        .as_millis() as u64
}

/// xorshift64*: small, deterministic, good enough for choosing operations.
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Rng {
        Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
    }

    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 11
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }

    fn chance(&mut self, percent: u64) -> bool {
        self.next() % 100 < percent
    }
}

/// One simulated Mac.
struct Device {
    name: String,
    _root: tempfile::TempDir,
    env: Env,
    resolver: MapResolver,
    transport: MemTransport,
    /// (project key, identity) of every directory the user has todos in on this Mac.
    dirs: Vec<(String, Option<String>)>,
    keys: Vec<String>,
    now: u64,
}

impl Device {
    fn new(name: &str, cloud: &Rc<RefCell<Cloud>>, dirs: &[(&str, Option<&str>)]) -> Device {
        let root = tempfile::tempdir().expect("tempdir");
        let state_dir = root.path().join("state");
        std::fs::create_dir_all(&state_dir).expect("state dir");
        let env = Env {
            todos_path: state_dir.join("todos.json"),
            todos_lock: state_dir.join("todos.lock"),
            dir: state_dir.join("todo-sync"),
            history_limit: LIMIT,
            device_name: name.to_string(),
        };
        Device {
            name: name.to_string(),
            _root: root,
            env,
            resolver: MapResolver::new(dirs),
            transport: MemTransport::new(cloud),
            dirs: dirs
                .iter()
                .map(|(k, p)| (k.to_string(), p.map(str::to_string)))
                .collect(),
            keys: dirs.iter().map(|(k, _)| k.to_string()).collect(),
            now: real_now_ms(),
        }
    }

    fn enable(&self) {
        tick::enable(&self.env, self.now).expect("enable");
    }

    fn tick_with(&mut self, fp: &Failpoints) -> Result<TickReport, TickError> {
        self.now += 1_000;
        tick::full(
            &self.env,
            &self.resolver,
            &mut self.transport,
            &self.keys,
            self.now,
            fp,
            &Opts::default(),
        )
    }

    fn tick(&mut self) -> TickReport {
        match self.tick_with(&Failpoints::off()) {
            Ok(r) => r,
            Err(e) => panic!("{}: tick failed: {e}", self.name),
        }
    }

    /// The local phase alone (what the CLI / popup do before reading).
    fn read_only(&mut self) -> TickReport {
        self.now += 1;
        tick::local_only(
            &self.env,
            &self.resolver,
            &self.keys,
            self.now,
            &Failpoints::off(),
            Duration::from_millis(50),
        )
        .expect("local_only")
    }

    fn state(&self) -> State {
        tick::peek(&self.env)
            .expect("peek")
            .expect("sync is enabled")
    }

    fn keys_of(&self, identity: &str) -> Vec<String> {
        self.dirs
            .iter()
            .filter(|(_, p)| p.as_deref() == Some(identity))
            .map(|(k, _)| k.clone())
            .collect()
    }

    // ---- what the user sees ----

    fn queue(&self, key: &str) -> Vec<(String, String)> {
        todos::list_at(&self.env.todos_path, key)
            .into_iter()
            .map(|e| (e.id, e.text))
            .collect()
    }

    fn texts(&self, key: &str) -> Vec<String> {
        self.queue(key).into_iter().map(|(_, t)| t).collect()
    }

    fn done(&self, key: &str) -> BTreeSet<(String, String)> {
        todos::history_at(&self.env.todos_path, key)
            .into_iter()
            .map(|e| (e.id, e.text))
            .collect()
    }

    fn done_texts(&self, key: &str) -> BTreeSet<String> {
        self.done(key).into_iter().map(|(_, t)| t).collect()
    }

    fn id_of(&self, key: &str, text: &str) -> String {
        self.queue(key)
            .into_iter()
            .chain(self.done(key))
            .find(|(_, t)| t == text)
            .unwrap_or_else(|| panic!("{}: no todo {text:?} in {key}", self.name))
            .0
    }

    fn file(&self) -> TodoFile {
        todos::read_checked_at(&self.env.todos_path).expect("todos.json")
    }

    // ---- what the user does ----

    fn add(&self, key: &str, text: &str) -> String {
        todos::add_at(&self.env.todos_path, &self.env.todos_lock, key, text)
            .expect("add")
            .id
    }

    fn edit(&self, key: &str, id: &str, text: &str) -> bool {
        todos::set_text_at(&self.env.todos_path, &self.env.todos_lock, key, id, text).is_some()
    }

    fn remove(&self, key: &str, id: &str) -> bool {
        todos::remove_at(&self.env.todos_path, &self.env.todos_lock, key, id)
    }

    fn take(&self, key: &str, ids: &[String]) -> usize {
        todos::take_at(
            &self.env.todos_path,
            &self.env.todos_lock,
            key,
            ids,
            self.env.history_limit,
        )
        .len()
    }

    fn restore(&self, key: &str, id: &str) -> bool {
        todos::restore_at(&self.env.todos_path, &self.env.todos_lock, key, id)
    }

    fn reorder(&self, key: &str, ids: &[String]) -> bool {
        todos::reorder_at(&self.env.todos_path, &self.env.todos_lock, key, ids)
    }

    fn clear_history(&self, key: &str) -> usize {
        todos::clear_history_at(&self.env.todos_path, &self.env.todos_lock, key)
    }

    fn clear_queue(&self, key: &str) -> usize {
        todos::clear_at(&self.env.todos_path, &self.env.todos_lock, key)
    }

    fn set_created(&self, key: &str, id: &str, ms: u64) {
        let session = todos::Session::open(&self.env.todos_path, &self.env.todos_lock);
        let mut file = session.read().expect("read");
        for e in file.projects.get_mut(key).into_iter().flatten() {
            if e.id == id {
                e.created_at_ms = ms;
            }
        }
        assert!(session.write(file));
    }
}

/// Faults off, then ticks until nothing moves any more.
fn settle(devs: &mut [Device], cloud: &Rc<RefCell<Cloud>>, ctx: &str) {
    cloud.borrow_mut().faults = Faults::default();
    for _round in 0..16 {
        let mut moved = false;
        for d in devs.iter_mut() {
            let r = d.tick();
            moved |=
                r.file_changed || r.pushed > 0 || r.merged > 0 || r.conflicts > 0 || r.unacked > 0;
        }
        if !moved {
            return;
        }
    }
    panic!("{ctx}: replicas did not settle");
}

/// Two rounds so every change reaches everybody, then a third that must find nothing to do.
fn sync_all(devs: &mut [Device], ctx: &str) {
    for _round in 0..2 {
        for d in devs.iter_mut() {
            d.tick();
        }
    }
    for d in devs.iter_mut() {
        let r = d.tick();
        assert!(
            !r.file_changed && r.pushed == 0 && r.merged == 0 && r.conflicts == 0 && r.unacked == 0,
            "{ctx}: {} was not idle after syncing: {r:?}",
            d.name
        );
    }
}

fn entity_view(s: &State) -> BTreeMap<String, super::model::Entity> {
    s.entities
        .iter()
        .map(|(id, r)| (id.clone(), r.e.clone()))
        .collect()
}

/// After settling: every replica of a project shows the same queue and history, every device holds
/// the same entities, nothing is waiting for upload, and the files agree with the entities.
/// (who, queue, history) of the first replica looked at.
type Replica = (String, Vec<(String, String)>, BTreeSet<(String, String)>);

fn assert_converged(devs: &[Device], projects: &[&str], ctx: &str) {
    let reference = entity_view(&devs[0].state());
    for d in devs {
        let s = d.state();
        assert!(
            s.entities.values().all(|r| !r.dirty),
            "{ctx}: {} still has unacknowledged entities",
            d.name
        );
        assert_eq!(
            entity_view(&s),
            reference,
            "{ctx}: entities of {} differ from {}",
            d.name,
            devs[0].name
        );
    }
    for p in projects {
        let pending: BTreeSet<String> = reference
            .iter()
            .filter(|(_, e)| e.p == *p && e.st.v == St::Pending)
            .map(|(id, _)| id.clone())
            .collect();
        let done: BTreeSet<String> = reference
            .iter()
            .filter(|(_, e)| e.p == *p && e.st.v == St::Done)
            .map(|(id, _)| id.clone())
            .collect();
        let mut first: Option<Replica> = None;
        for d in devs {
            for key in d.keys_of(p) {
                let q = d.queue(&key);
                let h = d.done(&key);
                let ids: BTreeSet<String> = q.iter().map(|(id, _)| id.clone()).collect();
                assert_eq!(
                    ids.len(),
                    q.len(),
                    "{ctx}: duplicate ids in {}:{key}",
                    d.name
                );
                assert_eq!(
                    ids, pending,
                    "{ctx}: {}:{key} queue differs from the pending entities",
                    d.name
                );
                let hids: BTreeSet<String> = h.iter().map(|(id, _)| id.clone()).collect();
                assert_eq!(
                    hids, done,
                    "{ctx}: {}:{key} history differs from the done entities",
                    d.name
                );
                match &first {
                    None => first = Some((format!("{}:{key}", d.name), q, h)),
                    Some((who, fq, fh)) => {
                        assert_eq!(
                            &q, fq,
                            "{ctx}: queue of {}:{key} differs from {who}",
                            d.name
                        );
                        assert_eq!(
                            &h, fh,
                            "{ctx}: history of {}:{key} differs from {who}",
                            d.name
                        );
                    }
                }
            }
        }
    }
}

fn three_macs(cloud: &Rc<RefCell<Cloud>>) -> Vec<Device> {
    let mut devs = vec![
        Device::new(
            "d0",
            cloud,
            &[
                ("/d0/a", Some(P1)),
                ("/d0/b", Some(P1)),
                ("/d0/p2", Some(P2)),
                ("/d0/solo", None),
            ],
        ),
        Device::new("d1", cloud, &[("/d1/a", Some(P1)), ("/d1/p2", Some(P2))]),
        Device::new("d2", cloud, &[("/d2/a", Some(P1)), ("/d2/solo", None)]),
    ];
    // Clocks that disagree by minutes: ordering must come from the HLCs, not from wall clocks.
    devs[1].now -= 4 * 60_000;
    devs[2].now += 9 * 60_000;
    for d in &devs {
        d.enable();
    }
    devs
}

// ---- scenarios ------------------------------------------------------------------------------------

#[test]
fn a_device_that_never_enabled_sync_is_untouched() {
    let cloud = Cloud::shared(1);
    let mut a = Device::new("A", &cloud, &[("/a", Some(P1))]);
    a.add("/a", "one");
    let before = std::fs::read(&a.env.todos_path).unwrap();

    let r = a.read_only();
    assert!(r.disabled && !r.file_changed);
    let r = a.tick();
    assert!(r.disabled);
    tick::restart_from_local(&a.env).unwrap();
    tick::clear_cloud(&a.env, &mut a.transport, a.now).unwrap();

    assert_eq!(std::fs::read(&a.env.todos_path).unwrap(), before);
    assert!(
        !a.env.dir.exists(),
        "no sidecar may appear before sync is enabled"
    );
    let c = cloud.borrow();
    assert_eq!((c.saves, c.fetches), (0, 0));
    assert!(c.records.is_empty());
}

#[test]
fn first_sync_round_trip_between_two_devices() {
    let cloud = Cloud::shared(1);
    let mut a = Device::new("Studio", &cloud, &[("/a", Some(P1))]);
    let mut b = Device::new("Air", &cloud, &[("/b", Some(P1))]);
    a.enable();
    b.enable();
    a.add("/a", "one");
    a.add("/a", "two");

    let ra = a.tick();
    assert!(ra.pulled && ra.pushed == 2, "{ra:?}");
    let rb = b.tick();
    assert!(rb.pulled && rb.file_changed, "{rb:?}");
    assert_eq!(b.texts("/b"), ["one", "two"]);

    b.add("/b", "three");
    b.tick();
    a.tick();
    assert_eq!(a.texts("/a"), ["one", "two", "three"]);

    // The device records travel too.
    let names: BTreeSet<String> = a.state().devices.values().map(|d| d.name.clone()).collect();
    assert!(
        names.contains("Studio") && names.contains("Air"),
        "{names:?}"
    );
}

#[test]
fn concurrent_edits_to_different_fields_both_survive() {
    let cloud = Cloud::shared(2);
    let mut a = Device::new("A", &cloud, &[("/a", Some(P1))]);
    let mut b = Device::new("B", &cloud, &[("/b", Some(P1))]);
    a.enable();
    b.enable();
    a.add("/a", "one");
    a.add("/a", "two");
    a.add("/a", "three");
    a.tick();
    b.tick();
    a.tick();

    // A rewrites a text, B reorders: neither may undo the other.
    let one = a.id_of("/a", "one");
    a.edit("/a", &one, "one!");
    let three = b.id_of("/b", "three");
    let two = b.id_of("/b", "two");
    let one_b = b.id_of("/b", "one");
    assert!(b.reorder("/b", &[three.clone(), one_b, two]));

    a.tick();
    b.tick();
    a.tick();
    b.tick();
    assert_eq!(a.texts("/a"), ["three", "one!", "two"]);
    assert_eq!(b.texts("/b"), ["three", "one!", "two"]);
    assert_converged(&[a, b], &[P1], "different fields");
}

#[test]
fn first_sync_interleaves_by_creation_time() {
    let cloud = Cloud::shared(3);
    let mut a = Device::new("A", &cloud, &[("/a", Some(P1))]);
    let mut b = Device::new("B", &cloud, &[("/b", Some(P1))]);
    a.enable();
    b.enable();
    for (text, at) in [("a1", 10_000), ("a2", 30_000)] {
        let id = a.add("/a", text);
        a.set_created("/a", &id, at);
    }
    for (text, at) in [("b1", 20_000), ("b2", 40_000)] {
        let id = b.add("/b", text);
        b.set_created("/b", &id, at);
    }
    a.tick();
    b.tick();
    a.tick();
    assert_eq!(b.texts("/b"), ["a1", "b1", "a2", "b2"]);
    assert_eq!(a.texts("/a"), ["a1", "b1", "a2", "b2"]);
}

#[test]
fn offline_edits_wait_and_a_first_sync_that_cannot_connect_changes_nothing() {
    let cloud = Cloud::shared(4);
    let mut a = Device::new("A", &cloud, &[("/a", Some(P1))]);
    let mut b = Device::new("B", &cloud, &[("/b", Some(P1))]);
    a.enable();
    b.enable();
    a.add("/a", "one");
    a.tick();

    cloud.borrow_mut().faults.offline = 100;
    b.add("/b", "mine");
    let before = std::fs::read(&b.env.todos_path).unwrap();
    let r = b.tick();
    assert!(r.offline && !r.pulled, "{r:?}");
    // Not even the local phase may run before the first pull (order keys are minted against the cloud).
    assert!(!b.read_only().file_changed);
    assert_eq!(std::fs::read(&b.env.todos_path).unwrap(), before);

    a.add("/a", "two");
    cloud.borrow_mut().faults = Faults::default();
    a.tick();
    b.tick();
    a.tick();
    b.tick();
    let expected: BTreeSet<String> = ["one", "two", "mine"].map(String::from).into();
    assert_eq!(b.texts("/b").into_iter().collect::<BTreeSet<_>>(), expected);
    assert_eq!(a.texts("/a"), b.texts("/b"));
    assert_converged(&[a, b], &[P1], "offline first sync");
}

#[test]
fn an_expired_token_restarts_from_nothing() {
    let cloud = Cloud::shared(5);
    let mut a = Device::new("A", &cloud, &[("/a", Some(P1))]);
    let mut b = Device::new("B", &cloud, &[("/b", Some(P1))]);
    a.enable();
    b.enable();
    for i in 0..6 {
        a.add("/a", &format!("t{i}"));
    }
    a.tick();
    b.tick();
    // Every outstanding token dies; B only knows an old one.
    cloud.borrow_mut().epoch += 1;
    a.add("/a", "later");
    a.tick();
    let r = b.tick();
    assert!(r.pulled && r.fetched >= 7, "{r:?}");
    assert_eq!(b.texts("/b"), a.texts("/a"));
    assert_eq!(b.texts("/b").len(), 7);
}

#[test]
fn a_lost_acknowledgement_never_duplicates_and_clears_the_dirty_flag() {
    let cloud = Cloud::shared(6);
    let mut a = Device::new("A", &cloud, &[("/a", Some(P1))]);
    let mut b = Device::new("B", &cloud, &[("/b", Some(P1))]);
    a.enable();
    b.enable();
    a.add("/a", "one");
    a.tick();

    a.add("/a", "two");
    cloud.borrow_mut().faults.lost_ack = 100;
    let r = a.tick();
    assert!(r.offline, "{r:?}");
    assert_eq!(a.state().entities.values().filter(|r| r.dirty).count(), 1);
    cloud.borrow_mut().faults = Faults::default();
    // The server already has it: our own record comes back with the next pull and settles it.
    let r = a.tick();
    assert_eq!(r.unacked, 0, "{r:?}");
    assert_eq!(a.state().entities.values().filter(|r| r.dirty).count(), 0);

    b.tick();
    assert_eq!(b.texts("/b"), ["one", "two"]);
    let todo_records = cloud
        .borrow()
        .records
        .values()
        .filter(|r| r.rtype == "Todo")
        .count();
    assert_eq!(todo_records, 2);
    assert_converged(&[a, b], &[P1], "lost ack");
}

#[test]
fn deleting_a_record_on_the_server_is_never_a_user_deletion() {
    let cloud = Cloud::shared(7);
    let mut a = Device::new("A", &cloud, &[("/a", Some(P1))]);
    let mut b = Device::new("B", &cloud, &[("/b", Some(P1))]);
    a.enable();
    b.enable();
    a.add("/a", "one");
    a.add("/a", "two");
    a.tick();
    b.tick();

    let id = a.id_of("/a", "one");
    cloud.borrow_mut().delete_record(&id);
    // A learns the record vanished: it keeps its copy and uploads it again.
    a.tick();
    a.tick();
    assert_eq!(a.texts("/a"), ["one", "two"]);
    assert!(cloud.borrow().records.contains_key(&id));
    b.tick();
    assert_eq!(b.texts("/b"), ["one", "two"]);
}

#[test]
fn history_clear_is_global_and_trimming_stays_local() {
    let cloud = Cloud::shared(8);
    let mut a = Device::new("A", &cloud, &[("/a", Some(P1))]);
    let mut b = Device::new("B", &cloud, &[("/b", Some(P1))]);
    a.env.history_limit = 2;
    a.enable();
    b.enable();
    for t in ["t1", "t2", "t3"] {
        a.add("/a", t);
    }
    a.tick();
    b.tick();
    for t in ["t1", "t2", "t3"] {
        let id = a.id_of("/a", t);
        assert_eq!(a.take("/a", &[id]), 1);
        // A tick in between: the third execution trims t1 out of A's history, which must read as
        // "trimmed here", not as "deleted".
        a.tick();
    }
    b.tick();
    a.tick();
    // A shows only its newest two; the first is just trimmed locally and still done everywhere else.
    assert_eq!(a.done_texts("/a"), ["t2", "t3"].map(String::from).into());
    assert_eq!(
        b.done_texts("/b"),
        ["t1", "t2", "t3"].map(String::from).into()
    );

    // A device that joins later sees the full history too.
    let mut c = Device::new("C", &cloud, &[("/c", Some(P1))]);
    c.enable();
    c.tick();
    assert_eq!(c.done_texts("/c").len(), 3);

    // Clearing the history is global (T16).
    assert_eq!(a.clear_history("/a"), 2);
    a.tick();
    b.tick();
    c.tick();
    a.tick();
    assert!(a.done("/a").is_empty() && b.done("/b").is_empty() && c.done("/c").is_empty());
    let t1 = a
        .state()
        .entities
        .values()
        .find(|r| r.e.text.v == "t1")
        .map(|r| r.e.st.v);
    assert_eq!(
        t1,
        Some(St::Deleted),
        "history that was only trimmed locally is cleared as well"
    );
    let mut devs = vec![a, b, c];
    settle(&mut devs, &cloud, "history clear");
    assert_converged(&devs, &[P1], "history clear");
}

#[test]
fn a_new_directory_of_a_known_project_gets_its_replica_without_the_network() {
    let cloud = Cloud::shared(9);
    let mut a = Device::new("A", &cloud, &[("/a", Some(P1))]);
    a.enable();
    a.add("/a", "one");
    a.add("/a", "two");
    a.tick();

    // A new worktree with the same remote shows the project's todos on its first read.
    a.dirs.push(("/a-wt".into(), Some(P1.into())));
    a.keys.push("/a-wt".into());
    a.resolver.set("/a-wt", Some(P1));
    cloud.borrow_mut().faults.offline = 100;
    let r = a.read_only();
    assert!(r.file_changed, "{r:?}");
    assert_eq!(a.texts("/a-wt"), ["one", "two"]);

    // Editing there is an edit of the shared project.
    a.add("/a-wt", "three");
    cloud.borrow_mut().faults = Faults::default();
    a.tick();
    assert_eq!(a.texts("/a"), ["one", "two", "three"]);
}

#[test]
fn an_account_switch_pauses_and_restarting_from_local_republishes() {
    let cloud = Cloud::shared(10);
    let mut a = Device::new("A", &cloud, &[("/a", Some(P1))]);
    let mut b = Device::new("B", &cloud, &[("/b", Some(P1))]);
    a.enable();
    b.enable();
    a.add("/a", "one");
    a.add("/a", "two");
    a.tick();
    b.tick();

    // This Mac is now signed in with another Apple ID whose zone is empty.
    {
        let mut c = cloud.borrow_mut();
        c.account.hash = Some("account-2".into());
        c.records.clear();
        c.deleted.clear();
        c.zone = false;
        c.epoch += 1;
    }
    let r = a.tick();
    assert_eq!(r.paused, Some(PauseReason::AccountChanged), "{r:?}");
    assert!(
        cloud.borrow().records.is_empty(),
        "nothing may be uploaded into the other account"
    );
    a.add("/a", "three");
    let r = a.read_only();
    assert_eq!(r.paused, Some(PauseReason::AccountChanged));
    assert_eq!(a.texts("/a"), ["one", "two", "three"]);

    // The user decides to publish this Mac's list into the new account.
    tick::restart_from_local(&a.env).unwrap();
    let r = a.tick();
    assert!(r.paused.is_none() && r.pushed == 3, "{r:?}");
    let r = b.tick();
    assert_eq!(r.paused, Some(PauseReason::AccountChanged));
    tick::restart_from_local(&b.env).unwrap();
    b.tick();
    a.tick();
    b.tick();
    assert_eq!(b.texts("/b"), ["one", "two", "three"]);
    assert_eq!(a.texts("/a"), b.texts("/b"));
    assert_converged(&[a, b], &[P1], "account switch");
}

#[test]
fn clearing_icloud_data_pauses_everybody_until_somebody_republishes() {
    let cloud = Cloud::shared(11);
    let mut a = Device::new("A", &cloud, &[("/a", Some(P1))]);
    let mut b = Device::new("B", &cloud, &[("/b", Some(P1))]);
    a.enable();
    b.enable();
    a.add("/a", "one");
    a.tick();
    b.tick();
    b.add("/b", "two");
    b.tick();
    a.tick();

    tick::clear_cloud(&a.env, &mut a.transport, a.now).unwrap();
    assert_eq!(
        a.state().paused.map(|p| p.reason),
        Some(PauseReason::ZoneMissing)
    );
    assert!(cloud.borrow().records.is_empty());

    // B finds the zone gone: it pauses and keeps its todos; it never silently re-creates the zone.
    let before = b.texts("/b");
    let r = b.tick();
    assert_eq!(r.paused, Some(PauseReason::ZoneMissing), "{r:?}");
    assert!(!cloud.borrow().zone);
    assert_eq!(b.texts("/b"), before);

    tick::restart_from_local(&b.env).unwrap();
    let r = b.tick();
    assert!(r.paused.is_none() && r.pushed == 2, "{r:?}");
    tick::restart_from_local(&a.env).unwrap();
    a.tick();
    b.tick();
    assert_eq!(a.texts("/a"), ["one", "two"]);
    assert_converged(&[a, b], &[P1], "cleared iCloud data");
}

#[test]
fn signing_out_pauses_and_signing_back_in_resumes_by_itself() {
    let cloud = Cloud::shared(12);
    let mut a = Device::new("A", &cloud, &[("/a", Some(P1))]);
    a.enable();
    a.add("/a", "one");
    a.tick();

    cloud.borrow_mut().account.status = AccountStatus::NoAccount;
    a.add("/a", "two");
    let r = a.tick();
    assert_eq!(r.paused, Some(PauseReason::NotSignedIn), "{r:?}");
    assert_eq!(
        cloud
            .borrow()
            .records
            .values()
            .filter(|r| r.rtype == "Todo")
            .count(),
        1
    );

    cloud.borrow_mut().account.status = AccountStatus::Available;
    let r = a.tick();
    assert!(r.paused.is_none() && r.pushed == 1, "{r:?}");
    assert!(a.state().paused.is_none());
}

#[test]
fn a_full_icloud_pauses_and_an_explicit_resume_retries() {
    let cloud = Cloud::shared(13);
    let mut a = Device::new("A", &cloud, &[("/a", Some(P1))]);
    a.enable();
    a.tick();
    cloud.borrow_mut().quota_full = true;
    a.add("/a", "one");
    let r = a.tick();
    assert_eq!(r.paused, Some(PauseReason::Quota), "{r:?}");
    assert_eq!(a.texts("/a"), ["one"]);

    cloud.borrow_mut().quota_full = false;
    // Still paused: it is the user who says "I made room".
    assert_eq!(a.tick().paused, Some(PauseReason::Quota));
    assert!(tick::resume(&a.env).unwrap());
    let r = a.tick();
    assert!(r.paused.is_none() && r.pushed == 1, "{r:?}");
}

#[test]
fn an_unreadable_sidecar_is_rebuilt_without_duplicating_or_losing_anything() {
    let cloud = Cloud::shared(14);
    let mut a = Device::new("A", &cloud, &[("/a", Some(P1))]);
    let mut b = Device::new("B", &cloud, &[("/b", Some(P1))]);
    a.enable();
    b.enable();
    a.add("/a", "one");
    a.add("/a", "two");
    a.tick();
    b.tick();
    let records_before = cloud.borrow().records.len();

    std::fs::write(super::store::state_file(&a.env.dir), b"{ this is not json").unwrap();
    let r = a.tick();
    assert!(r.rebuilt, "{r:?}");
    assert_eq!(a.texts("/a"), ["one", "two"]);
    a.tick();
    b.tick();
    assert_eq!(b.texts("/b"), ["one", "two"]);
    // The rebuilt device is a new device (it has a new id), nothing else is new.
    assert_eq!(cloud.borrow().records.len(), records_before + 1);
    assert_converged(&[a, b], &[P1], "rebuilt sidecar");
}

#[test]
fn a_sidecar_from_a_newer_build_is_left_alone() {
    let cloud = Cloud::shared(15);
    let mut a = Device::new("A", &cloud, &[("/a", Some(P1))]);
    a.enable();
    a.add("/a", "one");
    a.tick();

    let path = super::store::state_file(&a.env.dir);
    let mut json: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    json["schema"] = serde_json::json!(999);
    let bytes = serde_json::to_vec(&json).unwrap();
    std::fs::write(&path, &bytes).unwrap();

    assert!(matches!(
        a.tick_with(&Failpoints::off()),
        Err(TickError::NewerSchema(999))
    ));
    assert_eq!(
        std::fs::read(&path).unwrap(),
        bytes,
        "a newer sidecar must not be rewritten"
    );
    a.add("/a", "two");
    assert!(matches!(
        tick::local_only(
            &a.env,
            &a.resolver,
            &a.keys,
            a.now,
            &Failpoints::off(),
            Duration::ZERO
        ),
        Err(TickError::NewerSchema(999))
    ));
}

#[test]
fn a_corrupt_todos_file_is_never_read_as_everything_deleted() {
    let cloud = Cloud::shared(16);
    let mut a = Device::new("A", &cloud, &[("/a", Some(P1))]);
    let mut b = Device::new("B", &cloud, &[("/b", Some(P1))]);
    a.enable();
    b.enable();
    a.add("/a", "one");
    a.add("/a", "two");
    a.tick();
    b.tick();

    std::fs::write(&a.env.todos_path, b"{ broken").unwrap();
    let r = a.tick_with(&Failpoints::off());
    assert!(matches!(r, Err(TickError::Todos(_))), "{r:?}");
    assert_eq!(std::fs::read(&a.env.todos_path).unwrap(), b"{ broken");
    b.tick();
    assert_eq!(
        b.texts("/b"),
        ["one", "two"],
        "nothing was deleted elsewhere"
    );
}

#[test]
fn a_directory_that_moves_to_another_project_does_not_delete_in_the_old_one() {
    let cloud = Cloud::shared(17);
    let mut a = Device::new("A", &cloud, &[("/a", Some(P1))]);
    let mut b = Device::new("B", &cloud, &[("/b", Some(P1))]);
    a.enable();
    b.enable();
    a.add("/a", "one");
    a.add("/a", "two");
    a.tick();
    b.tick();

    // The directory's remote now points at another repository.
    a.resolver.set("/a", Some(P2));
    a.dirs[0].1 = Some(P2.into());
    a.tick();
    a.tick();
    b.tick();
    assert_eq!(
        b.texts("/b"),
        ["one", "two"],
        "the old project keeps its todos"
    );
    assert_eq!(a.texts("/a").len(), 2, "the directory keeps what it shows");
    let moved: Vec<_> = a
        .state()
        .entities
        .values()
        .filter(|r| r.e.p == P2 && r.e.st.v == St::Pending)
        .map(|r| r.e.text.v.clone())
        .collect();
    assert_eq!(moved.len(), 2, "they joined the new project under new ids");
}

// ---- crash injection ------------------------------------------------------------------------------

#[derive(Debug, PartialEq, Eq)]
struct Outcome {
    a_queue: Vec<String>,
    a_done: BTreeSet<String>,
    b_queue: Vec<String>,
    b_done: BTreeSet<String>,
    todo_records: usize,
}

/// Two Macs edit concurrently (different fields of the same todos), then `who` ticks — crashing at the
/// given failpoints, one after the other — and everything settles.
fn crash_scenario(who: usize, crashes: &[u32]) -> (Outcome, u64) {
    let cloud = Cloud::shared(21);
    cloud.borrow_mut().page = 4;
    let mut a = Device::new("A", &cloud, &[("/a", Some(P1))]);
    let mut b = Device::new("B", &cloud, &[("/b", Some(P1))]);
    a.enable();
    b.enable();
    for t in ["one", "two", "three"] {
        a.add("/a", t);
    }
    a.tick();
    b.tick();
    a.tick();

    let one = a.id_of("/a", "one");
    a.edit("/a", &one, "one!");
    let two = a.id_of("/a", "two");
    a.take("/a", &[two]);
    a.add("/a", "four");

    let three = b.id_of("/b", "three");
    b.remove("/b", &three);
    let two = b.id_of("/b", "two");
    b.edit("/b", &two, "two!");

    let mut devs = vec![a, b];
    let mut seen = 0;
    for (i, &at) in crashes.iter().enumerate() {
        let fp = Failpoints::crash_at(at);
        let r = devs[who].tick_with(&fp);
        if i == 0 {
            seen = fp.seen();
        }
        match r {
            Ok(_) | Err(TickError::Crash(_)) => {}
            Err(e) => panic!("crash at {crashes:?}: unexpected error {e}"),
        }
    }
    if crashes.is_empty() {
        let fp = Failpoints::off();
        devs[who].tick_with(&fp).unwrap();
        seen = fp.seen();
    }
    settle(
        &mut devs,
        &cloud,
        &format!("crashes {crashes:?} on device {who}"),
    );
    assert_converged(
        &devs,
        &[P1],
        &format!("crashes {crashes:?} on device {who}"),
    );
    let todo_records = cloud
        .borrow()
        .records
        .values()
        .filter(|r| r.rtype == "Todo")
        .count();
    let out = Outcome {
        a_queue: devs[0].texts("/a"),
        a_done: devs[0].done_texts("/a"),
        b_queue: devs[1].texts("/b"),
        b_done: devs[1].done_texts("/b"),
        todo_records,
    };
    (out, seen)
}

#[test]
fn a_crash_at_any_point_converges_to_the_same_result() {
    for who in [0usize, 1] {
        let (expected, points) = crash_scenario(who, &[]);
        assert_eq!(expected.a_queue, ["one!", "four"], "{expected:?}");
        assert_eq!(expected.a_done, ["two!".to_string()].into(), "{expected:?}");
        assert_eq!(expected.a_queue, expected.b_queue);
        assert!(
            points >= 6,
            "the tick should pass many failpoints, saw {points}"
        );
        for first in 0..points as u32 + 2 {
            let (got, _) = crash_scenario(who, &[first]);
            assert_eq!(got, expected, "crash at failpoint {first} on device {who}");
            // …and a second crash during the recovery.
            for second in [0u32, 1, 2, 4, 6, 9, 13] {
                let (got, _) = crash_scenario(who, &[first, second]);
                assert_eq!(
                    got, expected,
                    "crashes at {first} then {second} on device {who}"
                );
            }
        }
    }
}

// ---- random runs ----------------------------------------------------------------------------------

#[derive(Default, Clone)]
struct Model {
    queue: Vec<(String, String)>,
    done: BTreeMap<String, String>,
}

fn model_done(m: &Model) -> BTreeSet<(String, String)> {
    m.done.iter().map(|(i, t)| (i.clone(), t.clone())).collect()
}

/// One random user action on `key`, applied to the real file and to the model. Returns a description.
fn user_op(
    rng: &mut Rng,
    d: &Device,
    key: &str,
    m: &mut Model,
    counter: &mut usize,
    clears: bool,
) -> String {
    *counter += 1;
    let n = *counter;
    let roll = rng.below(100);
    let choice = match roll {
        0..=29 => "add",
        30..=44 => "remove",
        45..=59 => "edit",
        60..=74 => "reorder",
        75..=89 => "execute",
        90..=96 => "restore",
        97 if clears => "clear-history",
        98..=99 if clears => "clear-queue",
        _ => "add",
    };
    let choice = match choice {
        "remove" | "edit" | "execute" if m.queue.is_empty() => "add",
        "reorder" if m.queue.len() < 2 => "add",
        "restore" if m.done.is_empty() => "add",
        c => c,
    };
    match choice {
        "add" => {
            let text = format!("t{n}");
            let id = d.add(key, &text);
            m.queue.push((id, text.clone()));
            format!("add {text}")
        }
        "remove" => {
            let i = rng.below(m.queue.len());
            let (id, text) = m.queue.remove(i);
            assert!(d.remove(key, &id));
            format!("remove {text}")
        }
        "edit" => {
            let i = rng.below(m.queue.len());
            let text = format!("{}~{n}", m.queue[i].1.split('~').next().unwrap());
            assert!(d.edit(key, &m.queue[i].0, &text));
            m.queue[i].1 = text.clone();
            format!("edit -> {text}")
        }
        "reorder" => {
            let from = rng.below(m.queue.len());
            let to = rng.below(m.queue.len());
            let item = m.queue.remove(from);
            m.queue.insert(to, item);
            let ids: Vec<String> = m.queue.iter().map(|(i, _)| i.clone()).collect();
            d.reorder(key, &ids);
            format!("move {from} -> {to}")
        }
        "execute" => {
            let count = 1 + rng.below(m.queue.len().min(2));
            let mut picked: BTreeSet<usize> = BTreeSet::new();
            while picked.len() < count {
                picked.insert(rng.below(m.queue.len()));
            }
            let ids: Vec<String> = picked.iter().map(|&i| m.queue[i].0.clone()).collect();
            assert_eq!(d.take(key, &ids), count);
            let mut kept = Vec::new();
            let mut names = Vec::new();
            for (i, (id, text)) in m.queue.drain(..).enumerate() {
                if picked.contains(&i) {
                    names.push(text.clone());
                    m.done.insert(id, text);
                } else {
                    kept.push((id, text));
                }
            }
            m.queue = kept;
            format!("execute {names:?}")
        }
        "restore" => {
            let i = rng.below(m.done.len());
            let id = m.done.keys().nth(i).cloned().unwrap();
            let text = m.done.remove(&id).unwrap();
            assert!(d.restore(key, &id));
            m.queue.push((id, text.clone()));
            format!("restore {text}")
        }
        "clear-history" => {
            d.clear_history(key);
            m.done.clear();
            "clear history".into()
        }
        "clear-queue" => {
            d.clear_queue(key);
            m.queue.clear();
            "clear queue".into()
        }
        other => unreachable!("{other}"),
    }
}

/// No concurrency at all: after every action everybody syncs, so the replicas must equal a plain
/// model of "one shared list" exactly — order included.
fn run_sequential(seed: u64, steps: usize) -> BTreeMap<String, usize> {
    let mut kinds: BTreeMap<String, usize> = BTreeMap::new();
    let mut rng = Rng::new(seed);
    let cloud = Cloud::shared(seed);
    cloud.borrow_mut().page = 3;
    let mut devs = three_macs(&cloud);
    sync_all(&mut devs, "initial");

    let mut models: BTreeMap<&str, Model> =
        BTreeMap::from([(P1, Model::default()), (P2, Model::default())]);
    let mut solo: BTreeMap<(usize, String), Vec<(String, String)>> = BTreeMap::new();
    let mut log: Vec<String> = Vec::new();
    let mut counter = 0usize;

    for step in 0..steps {
        let ctx = format!("seed {seed} step {step}");
        if rng.chance(8) {
            // A local-only directory (no remote): never synced, never touched.
            let di = if rng.chance(50) { 0 } else { 2 };
            let key = devs[di]
                .dirs
                .iter()
                .find(|(_, p)| p.is_none())
                .unwrap()
                .0
                .clone();
            counter += 1;
            let text = format!("solo{counter}");
            let id = devs[di].add(&key, &text);
            solo.entry((di, key)).or_default().push((id, text));
            log.push(format!("#{step} solo add"));
        } else {
            let p = if rng.chance(75) { P1 } else { P2 };
            let mut options = Vec::new();
            for (di, d) in devs.iter().enumerate() {
                for key in d.keys_of(p) {
                    options.push((di, key));
                }
            }
            let (di, key) = options[rng.below(options.len())].clone();
            let model = models.get_mut(p).unwrap();
            let what = user_op(&mut rng, &devs[di], &key, model, &mut counter, true);
            let kind = what.split_whitespace().next().unwrap_or("?").to_string();
            *kinds.entry(kind).or_default() += 1;
            log.push(format!("#{step} {}:{key} {what}", devs[di].name));
        }
        sync_all(&mut devs, &ctx);

        let recent = || {
            log.iter()
                .rev()
                .take(6)
                .rev()
                .cloned()
                .collect::<Vec<_>>()
                .join("\n  ")
        };
        for (p, model) in &models {
            for d in &devs {
                for key in d.keys_of(p) {
                    assert_eq!(
                        d.queue(&key),
                        model.queue,
                        "{ctx}: queue of {}:{key} differs from the model\n  {}",
                        d.name,
                        recent()
                    );
                    assert_eq!(
                        d.done(&key),
                        model_done(model),
                        "{ctx}: history of {}:{key} differs from the model\n  {}",
                        d.name,
                        recent()
                    );
                }
            }
        }
        for ((di, key), expected) in &solo {
            assert_eq!(
                &devs[*di].queue(key),
                expected,
                "{ctx}: local-only {key} changed"
            );
        }
    }
    assert_converged(&devs, &[P1, P2], &format!("seed {seed} end"));
    let st = devs[0].state();
    for expected in solo.values() {
        for (id, _) in expected {
            assert!(
                !st.entities.contains_key(id),
                "a local-only todo became an entity"
            );
        }
    }
    kinds
}

/// How many seeds a randomized test runs: `TODO_SYNC_SOAK=<n>` turns the suite into a soak run.
fn seeds(default: u64) -> u64 {
    std::env::var("TODO_SYNC_SOAK")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

#[test]
fn sequential_runs_match_a_plain_list_model() {
    let mut kinds: BTreeMap<String, usize> = BTreeMap::new();
    for seed in 1..=seeds(6) {
        for (k, n) in run_sequential(seed, 90) {
            *kinds.entry(k).or_default() += n;
        }
    }
    // The run must actually have exercised every kind of action.
    for kind in ["add", "remove", "edit", "move", "execute", "restore"] {
        assert!(
            kinds.get(kind).copied().unwrap_or(0) > 0,
            "no {kind} was generated: {kinds:?}"
        );
    }
}

#[derive(Default)]
struct IdOps {
    project: &'static str,
    removed: bool,
    executed: bool,
    restored: bool,
}

/// Concurrent edits on three Macs with offline spells, lost acknowledgements, redelivery, token expiry,
/// skewed clocks and crashes at random failpoints; afterwards everything must converge, and todos nobody
/// touched (or only removed, or only executed) must be in the state that implies.
fn run_chaos(seed: u64, steps: usize) {
    let mut rng = Rng::new(seed ^ 0x5eed);
    let cloud = Cloud::shared(seed);
    cloud.borrow_mut().page = 3;
    let mut devs = three_macs(&cloud);
    let rough = Faults {
        offline: 12,
        lost_ack: 8,
        redeliver: 20,
        expire: 3,
    };
    cloud.borrow_mut().faults = rough.clone();

    let mut ids: HashMap<String, IdOps> = HashMap::new();
    let mut solo: BTreeMap<(usize, String), Vec<(String, String)>> = BTreeMap::new();
    let mut counter = 0usize;
    let mut log: Vec<String> = Vec::new();
    let mut crashes = 0usize;

    for step in 0..steps {
        let di = rng.below(devs.len());
        match rng.below(100) {
            0..=44 => {
                let (key, identity) = devs[di].dirs[rng.below(devs[di].dirs.len())].clone();
                counter += 1;
                match identity {
                    None => {
                        // Local-only directory: add or remove, tracked exactly.
                        let list = solo.entry((di, key.clone())).or_default();
                        if list.is_empty() || rng.chance(70) {
                            let text = format!("solo{counter}");
                            let id = devs[di].add(&key, &text);
                            list.push((id, text));
                        } else {
                            let (id, _) = list.remove(rng.below(list.len()));
                            assert!(devs[di].remove(&key, &id));
                        }
                    }
                    Some(identity) => {
                        let project: &'static str = if identity == P1 { P1 } else { P2 };
                        // Act on what this Mac currently shows.
                        let mut model = Model {
                            queue: devs[di].queue(&key),
                            done: devs[di].done(&key).into_iter().collect(),
                        };
                        let before = model.clone();
                        let what =
                            user_op(&mut rng, &devs[di], &key, &mut model, &mut counter, false);
                        log.push(format!("#{step} {}:{key} {what}", devs[di].name));
                        // Record which todos were touched how.
                        let now_ids: BTreeSet<&String> =
                            model.queue.iter().map(|(i, _)| i).collect();
                        for (id, _) in &model.queue {
                            let e = ids.entry(id.clone()).or_default();
                            e.project = project;
                            if before.done.contains_key(id) {
                                e.restored = true;
                            }
                        }
                        for (id, _) in &before.queue {
                            if !now_ids.contains(id) {
                                let e = ids.entry(id.clone()).or_default();
                                e.project = project;
                                if model.done.contains_key(id) {
                                    e.executed = true;
                                } else {
                                    e.removed = true;
                                }
                            }
                        }
                    }
                }
            }
            45..=89 => {
                let fp = if rng.chance(10) {
                    Failpoints::crash_at(rng.below(30) as u32)
                } else {
                    Failpoints::off()
                };
                match devs[di].tick_with(&fp) {
                    Ok(_) => {}
                    Err(TickError::Crash(_)) => crashes += 1,
                    Err(TickError::Net(super::transport::NetError::TokenExpired)) => {}
                    Err(e) => panic!(
                        "seed {seed} step {step}: {} tick failed: {e}",
                        devs[di].name
                    ),
                }
            }
            90..=94 => {
                devs[di].now += rng.below(180_000) as u64;
            }
            _ => {
                cloud.borrow_mut().faults = if rng.chance(50) {
                    Faults::default()
                } else {
                    rough.clone()
                };
            }
        }
    }

    let ctx = format!("seed {seed} ({crashes} crashes)");
    settle(&mut devs, &cloud, &ctx);
    assert_converged(&devs, &[P1, P2], &ctx);

    let st = devs[0].state();
    let recent = || {
        log.iter()
            .rev()
            .take(12)
            .rev()
            .cloned()
            .collect::<Vec<_>>()
            .join("\n  ")
    };
    for (id, ops) in &ids {
        let Some(rec) = st.entities.get(id) else {
            // Added and removed again between two syncs: nobody ever needed to hear about it.
            assert!(
                ops.removed && !ops.executed && !ops.restored,
                "{ctx}: todo {id} never reached the entities\n  {}",
                recent()
            );
            continue;
        };
        assert_eq!(rec.e.p, ops.project, "{ctx}: todo {id} changed project");
        let touched = ops.removed || ops.executed || ops.restored;
        if !touched {
            assert_eq!(
                rec.e.st.v,
                St::Pending,
                "{ctx}: untouched todo {id} is not pending\n  {}",
                recent()
            );
        } else if ops.removed && !ops.executed && !ops.restored {
            assert_eq!(
                rec.e.st.v,
                St::Deleted,
                "{ctx}: removed todo {id} came back\n  {}",
                recent()
            );
        } else if ops.executed && !ops.removed && !ops.restored {
            assert_eq!(
                rec.e.st.v,
                St::Done,
                "{ctx}: executed todo {id} is not done\n  {}",
                recent()
            );
        }
    }
    for ((di, key), expected) in &solo {
        assert_eq!(
            &devs[*di].queue(key),
            expected,
            "{ctx}: local-only {key} changed"
        );
        for (id, _) in expected {
            assert!(
                !st.entities.contains_key(id),
                "{ctx}: a local-only todo became an entity"
            );
        }
    }
}

#[test]
fn chaotic_runs_converge() {
    for seed in 1..=seeds(10) {
        run_chaos(seed, 260);
    }
}

#[test]
fn trimming_the_history_locally_never_resurrects_or_deletes_elsewhere() {
    // A keeps two history entries, B keeps all of them; both keep executing and syncing.
    let cloud = Cloud::shared(30);
    let mut a = Device::new("A", &cloud, &[("/a", Some(P1))]);
    let mut b = Device::new("B", &cloud, &[("/b", Some(P1))]);
    a.env.history_limit = 2;
    a.enable();
    b.enable();
    let mut rng = Rng::new(30);
    let mut counter = 0usize;
    for step in 0..40 {
        let on_a = rng.chance(50);
        let (d, key) = if on_a { (&a, "/a") } else { (&b, "/b") };
        // Act on what this Mac shows (A's history is trimmed).
        let mut shown = Model {
            queue: d.queue(key),
            done: d.done(key).into_iter().collect(),
        };
        user_op(&mut rng, d, key, &mut shown, &mut counter, false);
        a.tick();
        b.tick();
        a.tick();
        b.tick();
        assert_eq!(
            a.queue("/a"),
            b.queue("/b"),
            "step {step}: queues must agree"
        );
        let a_done = a.done("/a");
        assert!(
            a_done.len() <= 2,
            "step {step}: A's history exceeds its limit"
        );
        assert!(
            a_done.is_subset(&b.done("/b")),
            "step {step}: A's trimmed history must be a subset of B's"
        );
    }
}

// ---- cost of the local phase ----------------------------------------------------------------------

fn median(mut xs: Vec<f64>) -> f64 {
    xs.sort_by(|a, b| a.partial_cmp(b).expect("no NaN"));
    xs[xs.len() / 2]
}

/// Plan V8: the local phase runs in front of every read (CLI, popup, Stop card, whats-next), so it has to
/// stay cheap on the stress tier of spec §9.9 — 200 projects, 2000 todos, 2000 history entries.
/// Not part of the normal run (the numbers only mean something when optimized):
/// `cargo test --release local_phase -- --ignored --nocapture`
#[test]
#[ignore = "benchmark"]
fn the_local_phase_is_cheap_on_stress_data() {
    use std::time::Instant;

    let cloud = Cloud::shared(99);
    cloud.borrow_mut().page = 500;
    let dirs: Vec<(String, String)> = (0..200)
        .map(|i| (format!("/p/{i}"), format!("github.com/acme/p{i}")))
        .collect();
    let pairs: Vec<(&str, Option<&str>)> = dirs
        .iter()
        .map(|(k, p)| (k.as_str(), Some(p.as_str())))
        .collect();
    let mut a = Device::new("A", &cloud, &pairs);
    a.enable();

    let mut file = TodoFile::default();
    for (n, (key, _)) in dirs.iter().enumerate() {
        let queue: Vec<todos::TodoEntry> = (0..10)
            .map(|j| todos::TodoEntry {
                id: uuid::Uuid::new_v4().to_string(),
                text: format!(
                    "project {n} todo {j}: a sentence about as long as a real task description"
                ),
                created_at_ms: 1_000 + j as u64,
                agent_kind: None,
                auto: false,
                attachments: vec![],
            })
            .collect();
        let history: Vec<todos::DoneTodoEntry> = (0..10)
            .map(|j| todos::DoneTodoEntry {
                id: uuid::Uuid::new_v4().to_string(),
                text: format!(
                    "project {n} done {j}: a sentence about as long as a real task description"
                ),
                created_at_ms: 500 + j as u64,
                agent_kind: None,
                done_at_ms: 2_000 + j as u64,
                attachments: vec![],
            })
            .collect();
        file.projects.insert(key.clone(), queue);
        file.history.insert(key.clone(), history);
    }
    let session = todos::Session::open(&a.env.todos_path, &a.env.todos_lock);
    assert!(session.write(file));
    drop(session);
    let size = std::fs::metadata(&a.env.todos_path).unwrap().len();

    let t = Instant::now();
    a.tick();
    let first = t.elapsed();
    let t = Instant::now();
    a.tick();
    let second = t.elapsed();

    let mut local = Vec::new();
    for _ in 0..30 {
        let t = Instant::now();
        let r = a.read_only();
        local.push(t.elapsed().as_secs_f64() * 1000.0);
        assert!(!r.file_changed);
    }
    let mut baseline = Vec::new();
    for _ in 0..30 {
        let t = Instant::now();
        let f = todos::read_checked_at(&a.env.todos_path).unwrap();
        baseline.push(t.elapsed().as_secs_f64() * 1000.0);
        assert_eq!(f.projects.len(), 200);
    }
    println!(
        "todos.json {} KiB, first sync {:?}, idle tick {:?}",
        size / 1024,
        first,
        second
    );
    println!(
        "local phase (idle): median {:.1} ms; reading todos.json alone: median {:.1} ms",
        median(local),
        median(baseline)
    );
}
