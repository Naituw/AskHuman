//! Local change detection (spec todo-sync §5.5, plan §4.1): turn "what the user did to `todos.json`
//! since the last sync" into entity updates.
//!
//! The file is the source of truth for *local intent*; the entities are the converged project state.
//! Each linked directory (key) is compared with the snapshot recorded when the sync core last wrote or
//! read it, and the differences become events: add, execute, delete, restore, edit, reorder, clear
//! history. Two properties make the whole thing crash-safe without a transaction:
//!
//! * An event is only generated when the entity does not already say the same thing, so re-running the
//!   pass after a half-finished tick (entities saved, file or snapshot not) is a no-op.
//! * Reorders are recognised against the snapshot **and** against the entities' own order: a file that
//!   merely lags behind merged remote changes is never mistaken for a local drag.

use super::hlc;
use super::model::*;
use super::order_key;
use crate::todo_attachments::TodoAttachment;
use crate::todos::{DoneTodoEntry, TodoEntry, TodoFile};
use std::collections::{HashMap, HashSet};

/// Most attachment names carried in the hint (the hint is only a hint).
const HINT_NAMES: usize = 10;

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct DiffReport {
    /// Entities created or advanced by this pass (0 ⇒ the state needs no save on their account).
    pub touched: usize,
    /// Entries given a new id because their old one belongs to another project (file edited).
    pub regenerated: usize,
    /// Projects whose order keys were rebuilt because a key grew too long.
    pub renumbered: usize,
}

/// Apply the local events found in `file` to `state.entities`.
///
/// Only linked keys are looked at; `file` is edited in exactly one case, id regeneration.
pub fn diff(file: &mut TodoFile, state: &mut State, now_ms: u64) -> DiffReport {
    let mut report = DiffReport::default();
    let keys: Vec<(String, String)> = state
        .links
        .iter()
        .filter_map(|(k, l)| l.p.clone().map(|p| (k.clone(), p)))
        .collect();

    // Clearing history is global, so it is resolved before anything else: the entities it tombstones
    // are exactly the ones that were done before this pass.
    for (key, p) in &keys {
        clear_history(file, state, &keys, key, p, now_ms, &mut report);
    }
    for (key, p) in &keys {
        process_key(file, state, key, p, now_ms, &mut report);
    }
    report
}

// ---- entity construction ---------------------------------------------------

fn att_names(attachments: &[TodoAttachment]) -> Vec<String> {
    attachments
        .iter()
        .take(HINT_NAMES)
        .map(|a| a.name.clone())
        .collect()
}

fn att_hint(attachments: &[TodoAttachment], dev: &str, h: &str) -> Option<Att> {
    (!attachments.is_empty()).then(|| Att {
        n: attachments.len() as u32,
        names: att_names(attachments),
        dev: dev.to_string(),
        h: h.to_string(),
    })
}

/// A fresh entity. `ord` starts empty: the key is minted by the ordering pass once the position is known.
#[allow(clippy::too_many_arguments)]
fn new_entity(
    state: &mut State,
    p: &str,
    text: &str,
    st: St,
    created_ms: u64,
    ak: Option<&str>,
    done: Option<DoneInfo>,
    attachments: &[TodoAttachment],
    now_ms: u64,
) -> Entity {
    let h = state.stamp(now_ms);
    let dev = state.device_id.clone();
    Entity {
        v: PAYLOAD_VERSION,
        p: p.to_string(),
        text: Reg {
            v: text.to_string(),
            h: h.clone(),
        },
        ord: Reg {
            v: String::new(),
            h: h.clone(),
        },
        st: Reg {
            v: st,
            h: h.clone(),
        },
        c: Created {
            ms: if created_ms == 0 { now_ms } else { created_ms },
            dev: dev.clone(),
            ak: ak.map(str::to_string),
        },
        dn: done,
        att: if st == St::Pending {
            att_hint(attachments, &dev, &h)
        } else {
            None
        },
        extra: Default::default(),
    }
}

fn set_state(state: &mut State, id: &str, st: St, done: Option<DoneInfo>, now_ms: u64) {
    let h = state.stamp(now_ms);
    if let Some(rec) = state.entities.get_mut(id) {
        rec.e.st = Reg { v: st, h };
        rec.e.dn = done;
        rec.dirty = true;
    }
}

fn wall_of(h: &str) -> u64 {
    hlc::parse(h).map(|(wall, _, _)| wall).unwrap_or(0)
}

// ---- T16: clearing history is global ---------------------------------------

fn clear_history(
    file: &TodoFile,
    state: &mut State,
    keys: &[(String, String)],
    key: &str,
    p: &str,
    now_ms: u64,
    report: &mut DiffReport,
) {
    let Some(snap) = state.snapshot.get(key) else {
        return;
    };
    let hist_now: HashSet<&str> = file
        .history
        .get(key)
        .map(|h| h.iter().map(|d| d.id.as_str()).collect())
        .unwrap_or_default();
    // Trimming by the history limit only ever drops the *oldest* entries while adding a new one, so a
    // history that went from non-empty to empty was cleared by the user.
    if snap.history.is_empty() || !hist_now.is_empty() {
        return;
    }
    // Entries restored to the queue before the clear are not part of it; if that explains every
    // vanished entry, nothing was cleared.
    let queued: HashSet<&str> = keys
        .iter()
        .filter(|(_, other)| other == p)
        .flat_map(|(k, _)| {
            file.projects
                .get(k)
                .into_iter()
                .flat_map(|q| q.iter().map(|t| t.id.as_str()))
        })
        .collect();
    if snap.history.iter().all(|id| queued.contains(id.as_str())) {
        return;
    }
    let victims: Vec<String> = state
        .entities
        .iter()
        .filter(|(id, r)| r.e.p == p && r.e.st.v == St::Done && !queued.contains(id.as_str()))
        .map(|(id, _)| id.clone())
        .collect();
    for id in victims {
        set_state(state, &id, St::Deleted, None, now_ms);
        report.touched += 1;
    }
}

// ---- per-key events --------------------------------------------------------

fn regenerate_foreign_ids(
    file: &mut TodoFile,
    state: &State,
    key: &str,
    p: &str,
    report: &mut DiffReport,
) {
    let foreign = |id: &str| state.entities.get(id).is_some_and(|r| r.e.p != p);
    if let Some(queue) = file.projects.get_mut(key) {
        for t in queue.iter_mut() {
            // Attachments live under a directory named after the id; renaming would orphan them, so
            // such an entry simply stays local.
            if foreign(&t.id) && t.attachments.is_empty() {
                t.id = uuid::Uuid::new_v4().to_string();
                report.regenerated += 1;
            }
        }
    }
    if let Some(hist) = file.history.get_mut(key) {
        for d in hist.iter_mut() {
            if foreign(&d.id) && d.attachments.is_empty() {
                d.id = uuid::Uuid::new_v4().to_string();
                report.regenerated += 1;
            }
        }
    }
}

fn process_key(
    file: &mut TodoFile,
    state: &mut State,
    key: &str,
    p: &str,
    now_ms: u64,
    report: &mut DiffReport,
) {
    regenerate_foreign_ids(file, state, key, p, report);

    let queue: Vec<TodoEntry> = file.projects.get(key).cloned().unwrap_or_default();
    let hist: Vec<DoneTodoEntry> = file.history.get(key).cloned().unwrap_or_default();
    let snap: Option<Snap> = state.snapshot.get(key).cloned();
    let first_seen = snap.is_none();
    let snap_pending: HashSet<&str> = snap
        .iter()
        .flat_map(|s| s.pending.iter().map(String::as_str))
        .collect();
    let snap_hist: HashSet<&str> = snap
        .iter()
        .flat_map(|s| s.history.iter().map(String::as_str))
        .collect();
    let pending_now: HashSet<&str> = queue.iter().map(|t| t.id.as_str()).collect();
    let hist_now: HashSet<&str> = hist.iter().map(|d| d.id.as_str()).collect();
    let me = state.device_id.clone();

    // Entries that need a fresh order key (new, restored, or never queued).
    let mut need_key: HashSet<String> = HashSet::new();

    // A. Newly executed: in the history now, was not at the last sync.
    for d in &hist {
        if pending_now.contains(d.id.as_str())
            || snap_hist.contains(d.id.as_str())
            || d.text.len() > TEXT_MAX
        {
            continue;
        }
        let done = DoneInfo {
            ms: d.done_at_ms,
            dev: me.clone(),
        };
        match state.entities.get(&d.id) {
            None => {
                let e = new_entity(
                    state,
                    p,
                    &d.text,
                    St::Done,
                    d.created_at_ms,
                    d.agent_kind.as_deref(),
                    Some(done),
                    &d.attachments,
                    now_ms,
                );
                state.entities.insert(
                    d.id.clone(),
                    Rec {
                        e,
                        dirty: true,
                        tag: None,
                    },
                );
                report.touched += 1;
            }
            Some(rec) if rec.e.p != p => {}
            Some(rec) if rec.e.st.v == St::Pending => {
                // Seen for the first time, the entity may be newer than this device's offline
                // execution; only a later execution overrides it.
                if !first_seen || d.done_at_ms > wall_of(&rec.e.st.h) {
                    set_state(state, &d.id, St::Done, Some(done), now_ms);
                    report.touched += 1;
                }
            }
            Some(_) => {}
        }
    }

    // B. Queue entries: new, restored, edited, attachment hint.
    for t in &queue {
        if t.text.len() > TEXT_MAX {
            continue;
        }
        enum Act {
            None,
            Create,
            Resurrect,
        }
        let act = match state.entities.get(&t.id) {
            None => Act::Create,
            Some(rec) if rec.e.p != p => Act::None,
            Some(rec) if rec.e.st.v == St::Done => {
                if !first_seen && !snap_pending.contains(t.id.as_str()) {
                    Act::Resurrect
                } else {
                    Act::None
                }
            }
            Some(rec) if rec.e.st.v == St::Deleted => Act::None,
            Some(_) => Act::None,
        };
        match act {
            Act::Create => {
                let e = new_entity(
                    state,
                    p,
                    &t.text,
                    St::Pending,
                    t.created_at_ms,
                    t.agent_kind.as_deref(),
                    None,
                    &t.attachments,
                    now_ms,
                );
                state.entities.insert(
                    t.id.clone(),
                    Rec {
                        e,
                        dirty: true,
                        tag: None,
                    },
                );
                need_key.insert(t.id.clone());
                report.touched += 1;
                continue;
            }
            Act::Resurrect => {
                set_state(state, &t.id, St::Pending, None, now_ms);
                need_key.insert(t.id.clone());
                report.touched += 1;
                continue;
            }
            Act::None => {}
        }

        // The entity exists, belongs to this project and is pending.
        let Some(rec) = state.entities.get(&t.id) else {
            continue;
        };
        if rec.e.p != p || rec.e.st.v != St::Pending {
            continue;
        }
        if !order_key::is_valid(&rec.e.ord.v) {
            need_key.insert(t.id.clone());
        }

        // Text edit: the file differs from what we last wrote *and* from the entity (a file that only
        // lags behind a merged remote edit is not an edit).
        let edited = snap
            .as_ref()
            .and_then(|s| s.hash.get(&t.id))
            .is_some_and(|old| *old != text_hash(&t.text));
        if edited && rec.e.text.v != t.text {
            let h = state.stamp(now_ms);
            if let Some(rec) = state.entities.get_mut(&t.id) {
                rec.e.text = Reg {
                    v: t.text.clone(),
                    h,
                };
                rec.dirty = true;
            }
            report.touched += 1;
        }

        // Attachment hint. Take over only an absent / empty hint or our own, otherwise two Macs that
        // both hold attachments for one todo would overwrite each other's hint forever.
        let n = t.attachments.len() as u32;
        let names = att_names(&t.attachments);
        let Some(rec) = state.entities.get(&t.id) else {
            continue;
        };
        let want = match &rec.e.att {
            Some(a) if a.dev == me => a.n != n || a.names != names,
            Some(a) if a.n == 0 => n > 0,
            Some(_) => false,
            None => n > 0,
        };
        if want {
            let h = state.stamp(now_ms);
            if let Some(rec) = state.entities.get_mut(&t.id) {
                rec.e.att = Some(Att {
                    n,
                    names,
                    dev: me.clone(),
                    h,
                });
                rec.dirty = true;
            }
            report.touched += 1;
        }
    }

    // C. Deleted: was queued at the last sync, is now neither queued nor executed.
    if let Some(snap) = &snap {
        for id in &snap.pending {
            if pending_now.contains(id.as_str()) || hist_now.contains(id.as_str()) {
                continue;
            }
            let live = state
                .entities
                .get(id)
                .is_some_and(|r| r.e.p == p && r.e.st.v == St::Pending);
            if live {
                set_state(state, id, St::Deleted, None, now_ms);
                report.touched += 1;
            }
        }
    }

    // D. Order keys.
    let current: Vec<String> = queue
        .iter()
        .filter(|t| {
            state
                .entities
                .get(&t.id)
                .is_some_and(|r| r.e.p == p && r.e.st.v == St::Pending)
        })
        .map(|t| t.id.clone())
        .collect();
    let moved = match &snap {
        Some(s) => detect_moved(state, &current, s, &need_key),
        None => HashSet::new(),
    };
    let mut to_key = need_key;
    to_key.extend(moved);
    if to_key.is_empty() {
        return;
    }
    let seq = if first_seen {
        merged_sequence(state, p, &current, &to_key)
    } else {
        current
    };
    let (minted, too_long) = mint_keys(state, &seq, &to_key, now_ms);
    report.touched += minted;
    if too_long {
        renumber_project(state, p, now_ms);
        report.renumbered += 1;
    }
}

// ---- ordering --------------------------------------------------------------

type OrderTuple<'a> = (&'a str, &'a str, &'a str);

fn order_tuple<'a>(state: &'a State, id: &'a str) -> OrderTuple<'a> {
    match state.entities.get(id) {
        Some(r) => (r.e.ord.v.as_str(), r.e.ord.h.as_str(), id),
        None => ("", "", id),
    }
}

/// Entries the user dragged since the last sync (they need new keys); empty when the file is merely
/// stale relative to merged remote changes.
fn detect_moved(
    state: &State,
    current: &[String],
    snap: &Snap,
    need_key: &HashSet<String>,
) -> HashSet<String> {
    // The file already follows the entities' order: nothing was dragged (the usual case after a
    // materialization whose snapshot save was lost to a crash).
    let keyed: Vec<&String> = current
        .iter()
        .filter(|id| {
            !need_key.contains(*id)
                && state
                    .entities
                    .get(*id)
                    .is_some_and(|r| order_key::is_valid(&r.e.ord.v))
        })
        .collect();
    if keyed
        .windows(2)
        .all(|w| order_tuple(state, w[0]) <= order_tuple(state, w[1]))
    {
        return HashSet::new();
    }
    // The file's order is exactly the snapshot's: the entities moved on remotely, the user did not.
    let pos: HashMap<&str, usize> = snap
        .pending
        .iter()
        .enumerate()
        .map(|(i, id)| (id.as_str(), i))
        .collect();
    let survivors: Vec<(&String, usize)> = current
        .iter()
        .filter(|id| !need_key.contains(*id))
        .filter_map(|id| pos.get(id.as_str()).map(|&p| (id, p)))
        .collect();
    if survivors.windows(2).all(|w| w[0].1 < w[1].1) {
        return HashSet::new();
    }
    // Keep the longest run that is still in snapshot order; everything else was moved.
    let positions: Vec<usize> = survivors.iter().map(|s| s.1).collect();
    let keep = lis_members(&positions);
    survivors
        .iter()
        .zip(keep)
        .filter(|(_, kept)| !kept)
        .map(|(s, _)| s.0.clone())
        .collect()
}

/// Membership flags of one longest strictly increasing subsequence.
pub(crate) fn lis_members(seq: &[usize]) -> Vec<bool> {
    let n = seq.len();
    let mut tails: Vec<usize> = Vec::new();
    let mut prev = vec![usize::MAX; n];
    for i in 0..n {
        let at = tails.partition_point(|&t| seq[t] < seq[i]);
        if at > 0 {
            prev[i] = tails[at - 1];
        }
        if at == tails.len() {
            tails.push(i);
        } else {
            tails[at] = i;
        }
    }
    let mut member = vec![false; n];
    if let Some(&last) = tails.last() {
        let mut cur = last;
        loop {
            member[cur] = true;
            if prev[cur] == usize::MAX {
                break;
            }
            cur = prev[cur];
        }
    }
    member
}

/// First sight of a directory: the project's keyed pending entities (in their own order) merged with
/// this directory's new entries (in theirs) by creation time, so neither sequence is reshuffled.
fn merged_sequence(
    state: &State,
    p: &str,
    current: &[String],
    to_key: &HashSet<String>,
) -> Vec<String> {
    let mut known: Vec<(&str, &Entity)> = state
        .entities
        .iter()
        .filter(|(id, r)| {
            r.e.p == p
                && r.e.st.v == St::Pending
                && !to_key.contains(id.as_str())
                && order_key::is_valid(&r.e.ord.v)
        })
        .map(|(id, r)| (id.as_str(), &r.e))
        .collect();
    known.sort_by(|a, b| (&a.1.ord.v, &a.1.ord.h, a.0).cmp(&(&b.1.ord.v, &b.1.ord.h, b.0)));
    let fresh: Vec<&String> = current.iter().filter(|id| to_key.contains(*id)).collect();
    let created = |id: &str| state.entities.get(id).map_or(0, |r| r.e.c.ms);

    let mut out = Vec::with_capacity(known.len() + fresh.len());
    let (mut i, mut j) = (0, 0);
    while i < known.len() && j < fresh.len() {
        if known[i].1.c.ms <= created(fresh[j]) {
            out.push(known[i].0.to_string());
            i += 1;
        } else {
            out.push(fresh[j].clone());
            j += 1;
        }
    }
    out.extend(known[i..].iter().map(|k| k.0.to_string()));
    out.extend(fresh[j..].iter().map(|f| (*f).clone()));
    out
}

/// Give every id in `to_key` a key between its stable neighbours in `seq`. Returns how many entities
/// were updated and whether a key outgrew [`order_key::MAX_LEN`].
fn mint_keys(
    state: &mut State,
    seq: &[String],
    to_key: &HashSet<String>,
    now_ms: u64,
) -> (usize, bool) {
    let stable: Vec<Option<String>> = seq
        .iter()
        .map(|id| {
            if to_key.contains(id) {
                return None;
            }
            state
                .entities
                .get(id)
                .map(|r| r.e.ord.v.clone())
                .filter(|k| order_key::is_valid(k))
        })
        .collect();
    let n = seq.len();
    let mut minted: Vec<(usize, String)> = Vec::new();
    let mut i = 0;
    while i < n {
        if stable[i].is_some() {
            i += 1;
            continue;
        }
        let mut j = i;
        while j < n && stable[j].is_none() {
            j += 1;
        }
        let right = stable.get(j).and_then(|k| k.as_deref());
        let mut prev: Option<String> = if i > 0 { stable[i - 1].clone() } else { None };
        for x in i..j {
            let key = order_key::between(prev.as_deref(), right);
            prev = Some(key.clone());
            minted.push((x, key));
        }
        i = j;
    }
    let too_long = minted.iter().any(|(_, k)| k.len() > order_key::MAX_LEN);
    let count = minted.len();
    for (x, key) in minted {
        let h = state.stamp(now_ms);
        if let Some(rec) = state.entities.get_mut(&seq[x]) {
            rec.e.ord = Reg { v: key, h };
            rec.dirty = true;
        }
    }
    (count, too_long)
}

/// Rebuild every pending key of a project, keeping the current order (spec §5.6).
fn renumber_project(state: &mut State, p: &str, now_ms: u64) {
    let mut ids: Vec<String> = state
        .entities
        .iter()
        .filter(|(_, r)| r.e.p == p && r.e.st.v == St::Pending)
        .map(|(id, _)| id.clone())
        .collect();
    ids.sort_by(|a, b| order_tuple(state, a).cmp(&order_tuple(state, b)));
    let keys = order_key::renumber(ids.len());
    for (id, key) in ids.iter().zip(keys) {
        let h = state.stamp(now_ms);
        if let Some(rec) = state.entities.get_mut(id) {
            rec.e.ord = Reg { v: key, h };
            rec.dirty = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::todo_sync::apply;
    use crate::todo_sync::links::testing::MapResolver;
    use crate::todo_sync::{links, merge};

    const P: &str = "github.com/x/y";

    fn state() -> State {
        State::new(
            "aaaaaaaa-0000-0000-0000-000000000000".into(),
            "Studio".into(),
            1,
        )
    }

    fn entry(id: &str, text: &str, created: u64) -> TodoEntry {
        TodoEntry {
            id: id.into(),
            text: text.into(),
            created_at_ms: created,
            agent_kind: None,
            auto: false,
            attachments: vec![],
        }
    }

    fn done(id: &str, text: &str, done_at: u64) -> DoneTodoEntry {
        DoneTodoEntry {
            id: id.into(),
            text: text.into(),
            created_at_ms: 1,
            agent_kind: None,
            done_at_ms: done_at,
            attachments: vec![],
        }
    }

    /// A state with `keys` linked to P, mirroring how `reconcile` prepares it.
    fn linked(keys: &[&str]) -> State {
        let mut s = state();
        let r = MapResolver::new(&keys.iter().map(|k| (*k, Some(P))).collect::<Vec<_>>());
        links::refresh(&mut s, keys.iter().copied(), &r, 1);
        s
    }

    /// One local phase over the file: diff, materialize, refresh snapshots (no persistence).
    fn sync(file: &mut TodoFile, s: &mut State, now: u64, limit: usize) -> DiffReport {
        let report = diff(file, s, now);
        let m = apply::materialize(file, s, limit);
        *file = m.file;
        for (k, snap) in m.snaps {
            s.snapshot.insert(k, snap);
        }
        report
    }

    fn file_with(key: &str, queue: Vec<TodoEntry>, hist: Vec<DoneTodoEntry>) -> TodoFile {
        let mut f = TodoFile::default();
        f.projects.insert(key.into(), queue);
        f.history.insert(key.into(), hist);
        f.pruned()
    }

    fn ids(f: &TodoFile, key: &str) -> Vec<String> {
        f.projects
            .get(key)
            .map(|q| q.iter().map(|t| t.id.clone()).collect())
            .unwrap_or_default()
    }

    fn st(s: &State, id: &str) -> St {
        s.entities[id].e.st.v
    }

    fn pending_order(s: &State) -> Vec<String> {
        let mut all: Vec<(&String, &Rec)> = s
            .entities
            .iter()
            .filter(|(_, r)| r.e.st.v == St::Pending)
            .collect();
        all.sort_by(|a, b| {
            (&a.1.e.ord.v, &a.1.e.ord.h, a.0).cmp(&(&b.1.e.ord.v, &b.1.e.ord.h, b.0))
        });
        all.into_iter().map(|(id, _)| id.clone()).collect()
    }

    #[test]
    fn first_sight_creates_entities_for_queue_and_history() {
        let mut s = linked(&["/a"]);
        let mut f = file_with(
            "/a",
            vec![entry("t1", "one", 10), entry("t2", "two", 20)],
            vec![done("d1", "old", 5)],
        );
        let r = sync(&mut f, &mut s, 1000, 50);
        assert!(
            r.touched >= 3,
            "three entities were created, got {}",
            r.touched
        );
        assert_eq!(st(&s, "t1"), St::Pending);
        assert_eq!(st(&s, "d1"), St::Done);
        assert_eq!(s.entities["d1"].e.dn.as_ref().unwrap().ms, 5);
        assert_eq!(s.entities["t1"].e.c.ms, 10);
        assert!(s.entities.values().all(|r| r.dirty));
        assert_eq!(pending_order(&s), vec!["t1", "t2"]);
        assert!(s.entities["t1"].e.ord.v < s.entities["t2"].e.ord.v);
        // The pass is idempotent.
        let r = sync(&mut f, &mut s, 2000, 50);
        assert_eq!(r.touched, 0);
    }

    #[test]
    fn add_execute_delete_restore_edit_follow_the_table() {
        let mut s = linked(&["/a"]);
        let mut f = file_with(
            "/a",
            vec![entry("t1", "one", 1), entry("t2", "two", 2)],
            vec![],
        );
        sync(&mut f, &mut s, 100, 50);

        // New id at the tail → new entity after the last key.
        f.projects
            .get_mut("/a")
            .unwrap()
            .push(entry("t3", "three", 3));
        sync(&mut f, &mut s, 200, 50);
        assert_eq!(pending_order(&s), vec!["t1", "t2", "t3"]);

        // Leaves the queue and shows up in history → executed, with the history time.
        let q = f.projects.get_mut("/a").unwrap();
        let t1 = q.remove(0);
        f.history
            .entry("/a".into())
            .or_default()
            .push(done(&t1.id, &t1.text, 777));
        sync(&mut f, &mut s, 300, 50);
        assert_eq!(st(&s, "t1"), St::Done);
        assert_eq!(s.entities["t1"].e.dn.as_ref().unwrap().ms, 777);

        // Leaves the queue and is nowhere else → deleted (tombstone).
        f.projects.get_mut("/a").unwrap().retain(|t| t.id != "t2");
        sync(&mut f, &mut s, 400, 50);
        assert_eq!(st(&s, "t2"), St::Deleted);

        // Back from history to the queue → pending again, at the tail.
        f.history.get_mut("/a").unwrap().retain(|d| d.id != "t1");
        f.projects
            .get_mut("/a")
            .unwrap()
            .push(entry("t1", "one", 1));
        sync(&mut f, &mut s, 500, 50);
        assert_eq!(st(&s, "t1"), St::Pending);
        assert!(s.entities["t1"].e.dn.is_none());
        assert_eq!(pending_order(&s), vec!["t3", "t1"]);

        // Text edit → text register only.
        let before = s.entities["t3"].e.clone();
        f.projects.get_mut("/a").unwrap()[0].text = "three!".into();
        sync(&mut f, &mut s, 600, 50);
        let after = &s.entities["t3"].e;
        assert_eq!(after.text.v, "three!");
        assert!(after.text.h > before.text.h);
        assert_eq!(after.ord, before.ord, "an edit must not touch the order");
        assert_eq!(after.st, before.st);
    }

    #[test]
    fn dragging_rekeys_only_the_moved_entry() {
        let mut s = linked(&["/a"]);
        let mut f = file_with(
            "/a",
            (1..=5).map(|i| entry(&format!("t{i}"), "x", i)).collect(),
            vec![],
        );
        sync(&mut f, &mut s, 100, 50);
        let keys_before: Vec<(String, Reg<String>)> = (1..=5)
            .map(|i| (format!("t{i}"), s.entities[&format!("t{i}")].e.ord.clone()))
            .collect();

        // Drag t5 to the front.
        let q = f.projects.get_mut("/a").unwrap();
        let t5 = q.pop().unwrap();
        q.insert(0, t5);
        sync(&mut f, &mut s, 200, 50);
        assert_eq!(pending_order(&s), vec!["t5", "t1", "t2", "t3", "t4"]);
        for (id, ord) in keys_before.iter().filter(|(id, _)| id != "t5") {
            assert_eq!(&s.entities[id].e.ord, ord, "{id} keeps its key");
        }
        assert!(s.entities["t5"].dirty);

        // Drag t2 to the end.
        let q = f.projects.get_mut("/a").unwrap();
        let pos = q.iter().position(|t| t.id == "t2").unwrap();
        let t2 = q.remove(pos);
        q.push(t2);
        sync(&mut f, &mut s, 300, 50);
        assert_eq!(pending_order(&s), vec!["t5", "t1", "t3", "t4", "t2"]);
        assert_eq!(ids(&f, "/a"), pending_order(&s));
    }

    #[test]
    fn a_file_that_lags_behind_merged_remote_changes_is_not_a_local_drag() {
        let mut s = linked(&["/a"]);
        let mut f = file_with(
            "/a",
            (1..=3).map(|i| entry(&format!("t{i}"), "x", i)).collect(),
            vec![],
        );
        sync(&mut f, &mut s, 100, 50);

        // A remote Mac moves t3 to the front; the merge reaches the entities but not (yet) the file.
        let first_key = s.entities["t1"].e.ord.v.clone();
        let key = order_key::before(&first_key);
        let h = s.stamp(500);
        s.entities.get_mut("t3").unwrap().e.ord = Reg { v: key, h };
        assert_eq!(pending_order(&s), vec!["t3", "t1", "t2"]);

        let r = diff(&mut f, &mut s, 600);
        assert_eq!(r.touched, 0, "no local event may come out of a stale file");
        assert_eq!(pending_order(&s), vec!["t3", "t1", "t2"]);

        // And the other half of the crash window: the file already follows the entities, the snapshot
        // does not yet.
        let m = apply::materialize(&f, &s, 50);
        f = m.file;
        assert_eq!(ids(&f, "/a"), vec!["t3", "t1", "t2"]);
        let r = diff(&mut f, &mut s, 700);
        assert_eq!(
            r.touched, 0,
            "stale snapshot + materialized file is not a drag either"
        );
    }

    #[test]
    fn remote_text_edit_materializes_without_echo() {
        let mut s = linked(&["/a"]);
        let mut f = file_with("/a", vec![entry("t1", "one", 1)], vec![]);
        sync(&mut f, &mut s, 100, 50);
        let h = s.stamp(500);
        s.entities.get_mut("t1").unwrap().e.text = Reg {
            v: "remote edit".into(),
            h,
        };
        // File still says "one" (snapshot hash matches it): that is lag, not a local edit.
        let r = diff(&mut f, &mut s, 600);
        assert_eq!(r.touched, 0);
        sync(&mut f, &mut s, 700, 50);
        assert_eq!(f.projects["/a"][0].text, "remote edit");
        assert_eq!(s.entities["t1"].e.text.v, "remote edit");
    }

    #[test]
    fn auto_and_attachment_changes_never_become_text_or_state_events() {
        let mut s = linked(&["/a"]);
        let mut f = file_with("/a", vec![entry("t1", "one", 1)], vec![]);
        sync(&mut f, &mut s, 100, 50);
        let before = s.entities["t1"].e.clone();
        f.projects.get_mut("/a").unwrap()[0].auto = true;
        let r = sync(&mut f, &mut s, 200, 50);
        assert_eq!(r.touched, 0);
        assert_eq!(s.entities["t1"].e, before);
        assert!(
            f.projects["/a"][0].auto,
            "materialize keeps the local auto flag"
        );
    }

    fn attachment(id: &str, name: &str) -> TodoAttachment {
        TodoAttachment {
            id: id.into(),
            name: name.into(),
            size: 1,
            is_image: false,
            source_path: String::new(),
            path: String::new(),
            storage: crate::todo_attachments::TodoAttachmentStorage::Managed,
            thumbnail_path: None,
            source_modified_ms: None,
        }
    }

    #[test]
    fn attachment_hint_is_updated_by_its_owner_and_never_ping_pongs() {
        let mut s = linked(&["/a"]);
        let mut f = file_with("/a", vec![entry("t1", "one", 1)], vec![]);
        sync(&mut f, &mut s, 100, 50);
        assert!(s.entities["t1"].e.att.is_none());

        f.projects.get_mut("/a").unwrap()[0].attachments = vec![attachment("a1", "shot.png")];
        sync(&mut f, &mut s, 200, 50);
        let hint = s.entities["t1"].e.att.clone().unwrap();
        assert_eq!(
            (hint.n, hint.names.clone(), hint.dev.clone()),
            (1, vec!["shot.png".to_string()], s.device_id.clone())
        );

        // Removing them clears our own hint (n = 0, still a Some so it can win the merge).
        f.projects.get_mut("/a").unwrap()[0].attachments.clear();
        sync(&mut f, &mut s, 300, 50);
        assert_eq!(s.entities["t1"].e.att.as_ref().unwrap().n, 0);

        // Another device's hint is respected: we hold attachments too but do not take it over.
        let other = Att {
            n: 2,
            names: vec!["x".into()],
            dev: "bbbbbbbb".into(),
            h: s.stamp(400),
        };
        s.entities.get_mut("t1").unwrap().e.att = Some(other.clone());
        f.projects.get_mut("/a").unwrap()[0].attachments = vec![attachment("a2", "mine.txt")];
        let r = sync(&mut f, &mut s, 500, 50);
        assert_eq!(r.touched, 0);
        assert_eq!(s.entities["t1"].e.att, Some(other));
    }

    #[test]
    fn clear_history_is_global_and_trimming_is_not() {
        let mut s = linked(&["/a"]);
        let mut f = file_with(
            "/a",
            vec![entry("q1", "queued", 1)],
            vec![done("d1", "a", 1), done("d2", "b", 2), done("d3", "c", 3)],
        );
        sync(&mut f, &mut s, 100, 50);
        // Entities for history that this Mac's view has long trimmed (or that never reached it).
        for (id, ms) in [("old1", 10u64), ("old2", 11)] {
            let h = s.stamp(150);
            s.entities.insert(
                id.into(),
                Rec {
                    e: Entity {
                        v: 1,
                        p: P.into(),
                        text: Reg {
                            v: id.into(),
                            h: h.clone(),
                        },
                        ord: Reg {
                            v: String::new(),
                            h: h.clone(),
                        },
                        st: Reg { v: St::Done, h },
                        c: Created {
                            ms,
                            dev: "bbbbbbbb".into(),
                            ak: None,
                        },
                        dn: Some(DoneInfo {
                            ms,
                            dev: "bbbbbbbb".into(),
                        }),
                        att: None,
                        extra: Default::default(),
                    },
                    dirty: false,
                    tag: None,
                },
            );
        }

        // Cap trimming: the oldest entry disappears, history stays non-empty → no event.
        f.history.get_mut("/a").unwrap().remove(0);
        let r = diff(&mut f, &mut s, 200);
        assert_eq!(r.touched, 0, "trimming is local only");
        assert_eq!(st(&s, "d1"), St::Done);

        // Then the user clears what is left (the snapshot still lists d1..d3).
        f.history.remove("/a");
        let r = diff(&mut f, &mut s, 300);
        assert!(
            r.touched >= 5,
            "every done entity of the project is tombstoned, got {}",
            r.touched
        );
        for id in ["d1", "d2", "d3", "old1", "old2"] {
            assert_eq!(st(&s, id), St::Deleted, "{id}");
        }
        assert_eq!(
            st(&s, "q1"),
            St::Pending,
            "the queue is not part of the history"
        );
    }

    #[test]
    fn restore_then_clear_history_keeps_the_restored_entry() {
        let mut s = linked(&["/a"]);
        let mut f = file_with(
            "/a",
            vec![],
            vec![done("d1", "a", 1), done("d2", "b", 2), done("d3", "c", 3)],
        );
        sync(&mut f, &mut s, 100, 50);
        // Within one sync window: restore d1, then clear the rest.
        f.history.remove("/a");
        f.projects.insert("/a".into(), vec![entry("d1", "a", 1)]);
        diff(&mut f, &mut s, 200);
        assert_eq!(st(&s, "d1"), St::Pending);
        assert_eq!(st(&s, "d2"), St::Deleted);
        assert_eq!(st(&s, "d3"), St::Deleted);
        assert_eq!(pending_order(&s), vec!["d1"]);

        // Restoring the only history entries is not a clear.
        let mut s = linked(&["/a"]);
        let mut f = file_with("/a", vec![], vec![done("e1", "a", 1)]);
        sync(&mut f, &mut s, 100, 50);
        f.history.remove("/a");
        f.projects.insert("/a".into(), vec![entry("e1", "a", 1)]);
        diff(&mut f, &mut s, 200);
        assert_eq!(st(&s, "e1"), St::Pending);
    }

    #[test]
    fn limit_zero_dequeue_is_a_delete_and_existing_history_survives() {
        let mut s = linked(&["/a"]);
        let mut f = file_with(
            "/a",
            vec![entry("t1", "one", 1)],
            vec![done("d1", "old", 1)],
        );
        sync(&mut f, &mut s, 100, 0);
        // take_at with limit 0: gone from the queue, not recorded anywhere.
        f.projects.remove("/a");
        sync(&mut f, &mut s, 200, 0);
        assert_eq!(st(&s, "t1"), St::Deleted);
        assert_eq!(st(&s, "d1"), St::Done);
        assert_eq!(
            f.history["/a"].len(),
            1,
            "existing history is kept at limit 0"
        );
    }

    #[test]
    fn a_remote_pending_entity_is_materialized_into_a_second_directory() {
        let mut s = linked(&["/a", "/b"]);
        let mut f = file_with(
            "/a",
            vec![entry("t1", "one", 1), entry("t2", "two", 2)],
            vec![],
        );
        sync(&mut f, &mut s, 100, 50);
        assert_eq!(
            ids(&f, "/b"),
            vec!["t1", "t2"],
            "the sibling directory gets its replica"
        );
        assert!(!f.projects["/b"][0].auto);

        // An edit in /b reaches /a through the entities.
        f.projects.get_mut("/b").unwrap()[1].text = "two (b)".into();
        sync(&mut f, &mut s, 200, 50);
        assert_eq!(f.projects["/a"][1].text, "two (b)");
        // Executing in /a empties /b's queue entry too.
        let q = f.projects.get_mut("/a").unwrap();
        let t1 = q.remove(0);
        f.history
            .entry("/a".into())
            .or_default()
            .push(done(&t1.id, &t1.text, 9));
        sync(&mut f, &mut s, 300, 50);
        assert_eq!(ids(&f, "/b"), vec!["t2"]);
        assert_eq!(f.history["/b"][0].id, "t1");
        assert_eq!(f.history["/b"][0].done_at_ms, 9);
    }

    #[test]
    fn first_sight_merges_two_nonempty_sequences_by_creation_time() {
        // Entities from another Mac, already keyed: r1 (t=10), r2 (t=30).
        let mut s = linked(&["/a"]);
        for (i, (id, ms)) in [("r1", 10u64), ("r2", 30)].into_iter().enumerate() {
            let h = s.stamp(50);
            s.entities.insert(
                id.into(),
                Rec {
                    e: Entity {
                        v: 1,
                        p: P.into(),
                        text: Reg {
                            v: id.into(),
                            h: h.clone(),
                        },
                        ord: Reg {
                            v: order_key::renumber(2)[i].clone(),
                            h: h.clone(),
                        },
                        st: Reg { v: St::Pending, h },
                        c: Created {
                            ms,
                            dev: "bbbbbbbb".into(),
                            ak: None,
                        },
                        dn: None,
                        att: None,
                        extra: Default::default(),
                    },
                    dirty: false,
                    tag: Some("t".into()),
                },
            );
        }
        // Local, never synced: l1 (t=20), l2 (t=40), in this order.
        let mut f = file_with(
            "/a",
            vec![entry("l1", "l1", 20), entry("l2", "l2", 40)],
            vec![],
        );
        sync(&mut f, &mut s, 100, 50);
        assert_eq!(pending_order(&s), vec!["r1", "l1", "r2", "l2"]);
        assert_eq!(ids(&f, "/a"), vec!["r1", "l1", "r2", "l2"]);
        // The remote ones were not touched.
        assert!(!s.entities["r1"].dirty && !s.entities["r2"].dirty);
        assert!(s.entities["l1"].dirty && s.entities["l2"].dirty);
    }

    #[test]
    fn first_sight_adopts_known_ids_instead_of_duplicating() {
        let mut s = linked(&["/a"]);
        let mut f0 = file_with(
            "/a",
            vec![entry("t1", "one", 1), entry("t2", "two", 2)],
            vec![],
        );
        let mut other = linked(&["/a"]);
        sync(&mut f0, &mut other, 100, 50);

        // This device learns the same entities from the cloud before it ever diffs /a.
        for (id, rec) in other.entities.clone() {
            merge::merge_remote(&mut s, &id, rec.e, "tag", 150);
        }
        let mut f = file_with(
            "/a",
            vec![entry("t2", "two", 2), entry("t1", "one", 1)],
            vec![],
        );
        let r = sync(&mut f, &mut s, 200, 50);
        assert_eq!(r.touched, 0, "adopting known ids is not an event");
        assert_eq!(
            ids(&f, "/a"),
            vec!["t1", "t2"],
            "the entity order wins on first sight"
        );
    }

    #[test]
    fn a_resurrected_tombstone_stays_dead() {
        let mut s = linked(&["/a"]);
        let mut f = file_with("/a", vec![entry("t1", "one", 1)], vec![]);
        sync(&mut f, &mut s, 100, 50);
        f.projects.remove("/a");
        sync(&mut f, &mut s, 200, 50);
        assert_eq!(st(&s, "t1"), St::Deleted);
        // The same id turns up in the queue again (stale copy): the entity wins, materialize drops it.
        f.projects.insert("/a".into(), vec![entry("t1", "one", 1)]);
        let r = sync(&mut f, &mut s, 300, 50);
        assert_eq!(r.touched, 0);
        assert!(!f.projects.contains_key("/a"));
    }

    #[test]
    fn foreign_ids_get_regenerated_unless_they_carry_attachments() {
        let mut s = linked(&["/a"]);
        let mut f = file_with("/a", vec![entry("t1", "one", 1)], vec![]);
        sync(&mut f, &mut s, 100, 50);

        // /a moves to another project (its remote was repointed).
        let r = MapResolver::new(&[("/a", Some("github.com/fork/y"))]);
        let out = links::refresh(&mut s, ["/a"], &r, 200);
        assert_eq!(out.identity_changed, vec!["/a".to_string()]);
        let report = sync(&mut f, &mut s, 300, 50);
        assert_eq!(report.regenerated, 1);
        assert_eq!(
            st(&s, "t1"),
            St::Pending,
            "the old project's entity is untouched"
        );
        let new_id = &f.projects["/a"][0].id;
        assert_ne!(new_id, "t1");
        assert_eq!(s.entities[new_id.as_str()].e.p, "github.com/fork/y");
        assert_eq!(s.entities[new_id.as_str()].e.text.v, "one");

        // With attachments the id stays (they live under it) and the entry is left unsynced.
        let mut s = linked(&["/a"]);
        let mut f = file_with("/a", vec![entry("t1", "one", 1)], vec![]);
        sync(&mut f, &mut s, 100, 50);
        f.projects.get_mut("/a").unwrap()[0].attachments = vec![attachment("a1", "f.txt")];
        let r = MapResolver::new(&[("/a", Some("github.com/fork/y"))]);
        links::refresh(&mut s, ["/a"], &r, 200);
        let report = sync(&mut f, &mut s, 300, 50);
        assert_eq!(report.regenerated, 0);
        assert_eq!(f.projects["/a"][0].id, "t1");
        assert_eq!(s.entities["t1"].e.p, P);
    }

    #[test]
    fn local_only_directories_are_never_touched() {
        let mut s = state();
        let r = MapResolver::new(&[("/solo", None)]);
        links::refresh(&mut s, ["/solo"], &r, 1);
        let mut f = file_with(
            "/solo",
            vec![entry("t1", "one", 1)],
            vec![done("d1", "x", 1)],
        );
        let before = f.clone();
        let report = sync(&mut f, &mut s, 100, 50);
        assert_eq!(report, DiffReport::default());
        assert_eq!(f, before);
        assert!(s.entities.is_empty() && s.snapshot.is_empty());
    }

    #[test]
    fn oversize_text_stays_local() {
        let mut s = linked(&["/a"]);
        let big = "x".repeat(TEXT_MAX + 1);
        let mut f = file_with(
            "/a",
            vec![entry("big", &big, 1), entry("t1", "ok", 2)],
            vec![],
        );
        sync(&mut f, &mut s, 100, 50);
        assert!(!s.entities.contains_key("big"));
        assert!(s.entities.contains_key("t1"));
        assert_eq!(
            ids(&f, "/a"),
            vec!["t1", "big"],
            "the unsynced entry is kept (at the tail)"
        );
    }

    #[test]
    fn lis_marks_the_longest_increasing_run() {
        assert_eq!(lis_members(&[]), Vec::<bool>::new());
        assert_eq!(lis_members(&[0, 1, 2]), vec![true; 3]);
        // [4,0,1,2,3] → keep 0..3, move the 4.
        assert_eq!(
            lis_members(&[4, 0, 1, 2, 3]),
            vec![false, true, true, true, true]
        );
        let m = lis_members(&[2, 1, 0]);
        assert_eq!(m.iter().filter(|x| **x).count(), 1);
    }

    #[test]
    fn long_keys_trigger_a_project_renumber() {
        let mut s = linked(&["/a"]);
        let mut f = file_with("/a", vec![entry("t1", "a", 1), entry("t2", "b", 2)], vec![]);
        sync(&mut f, &mut s, 100, 50);
        // Force t1/t2 into a pathological pair of keys, then insert between them.
        let long_a = format!("{}1", "z".repeat(order_key::MAX_LEN - 1));
        let long_b = format!("{}2", "z".repeat(order_key::MAX_LEN - 1));
        s.entities.get_mut("t1").unwrap().e.ord.v = long_a;
        s.entities.get_mut("t2").unwrap().e.ord.v = long_b;
        sync(&mut f, &mut s, 150, 50);
        let q = f.projects.get_mut("/a").unwrap();
        q.insert(1, entry("t3", "c", 3));
        let r = sync(&mut f, &mut s, 200, 50);
        assert_eq!(r.renumbered, 1);
        assert_eq!(pending_order(&s), vec!["t1", "t3", "t2"]);
        assert!(s.entities.values().all(|r| r.e.ord.v.len() <= 2));
    }
}
