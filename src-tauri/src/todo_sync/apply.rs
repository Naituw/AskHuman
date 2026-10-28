//! Materialization (spec todo-sync §5.2, plan §4.3): make every linked directory's queue and history
//! agree with its project's entities.
//!
//! This is a pure function of `(todos.json, entities, history limit)`: no clock, no I/O. Whatever the
//! entities say wins, with three local-only exceptions that are *preserved* rather than synced: the
//! `auto` flag, attachments, and entries that have no entity (oversize text, or an id that belongs to
//! another project). Running it twice yields the same file, which is what makes retries after a crash
//! safe.

use super::model::*;
use crate::todo_attachments::TodoAttachment;
use crate::todos::{DoneTodoEntry, TodoEntry, TodoFile};
use std::collections::{BTreeMap, HashMap, HashSet};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Materialized {
    /// The new file contents (pruned of empty queues, like every write).
    pub file: TodoFile,
    /// Snapshot of each linked key's slice of `file`.
    pub snaps: BTreeMap<String, Snap>,
    /// Attachments of entries that left the file for good; delete their files after the write succeeds.
    pub cleanup: Vec<TodoAttachment>,
}

/// One project's entities, in the order they are shown.
#[derive(Default)]
struct View<'a> {
    /// Pending, ascending by `(key, hlc, id)`.
    pending: Vec<(&'a str, &'a Entity)>,
    /// Done, ascending by `(done time, id)`.
    done: Vec<(&'a str, &'a Entity)>,
}

fn done_ms(e: &Entity) -> u64 {
    e.dn.as_ref().map_or(0, |d| d.ms)
}

fn index(state: &State) -> HashMap<&str, View<'_>> {
    let mut by_project: HashMap<&str, View<'_>> = HashMap::new();
    for (id, rec) in &state.entities {
        let view = by_project.entry(rec.e.p.as_str()).or_default();
        match rec.e.st.v {
            St::Pending => view.pending.push((id.as_str(), &rec.e)),
            St::Done => view.done.push((id.as_str(), &rec.e)),
            St::Deleted => {}
        }
    }
    for view in by_project.values_mut() {
        view.pending
            .sort_by(|a, b| (&a.1.ord.v, &a.1.ord.h, a.0).cmp(&(&b.1.ord.v, &b.1.ord.h, b.0)));
        view.done
            .sort_by(|a, b| (done_ms(a.1), a.0).cmp(&(done_ms(b.1), b.0)));
    }
    by_project
}

pub fn snap_of(queue: &[TodoEntry], history: &[DoneTodoEntry]) -> Snap {
    Snap {
        pending: queue.iter().map(|t| t.id.clone()).collect(),
        history: history.iter().map(|d| d.id.clone()).collect(),
        hash: queue
            .iter()
            .map(|t| (t.id.clone(), text_hash(&t.text)))
            .collect(),
    }
}

/// Compute the materialized file. `history_limit` is this device's `todo_history_limit`
/// (`0` = do not record: remote executions are not added, existing history is left alone).
pub fn materialize(file: &TodoFile, state: &State, history_limit: usize) -> Materialized {
    let views = index(state);
    let empty = View::default();
    let mut out = file.clone();
    let mut snaps = BTreeMap::new();
    let mut cleanup = Vec::new();

    for (key, link) in &state.links {
        let Some(p) = link.p.as_deref() else {
            continue;
        };
        let view = views.get(p).unwrap_or(&empty);
        let old_q = file.projects.get(key).map(Vec::as_slice).unwrap_or(&[]);
        let old_h = file.history.get(key).map(Vec::as_slice).unwrap_or(&[]);
        let (queue, history, gone) = materialize_key(old_q, old_h, p, state, view, history_limit);
        cleanup.extend(gone);
        snaps.insert(key.clone(), snap_of(&queue, &history));
        out.projects.insert(key.clone(), queue);
        out.history.insert(key.clone(), history);
    }
    Materialized {
        file: out.pruned(),
        snaps,
        cleanup,
    }
}

fn materialize_key(
    old_q: &[TodoEntry],
    old_h: &[DoneTodoEntry],
    p: &str,
    state: &State,
    view: &View<'_>,
    limit: usize,
) -> (Vec<TodoEntry>, Vec<DoneTodoEntry>, Vec<TodoAttachment>) {
    // The entity of an id, if it is one of this project's.
    let ent = |id: &str| state.entities.get(id).filter(|r| r.e.p == p).map(|r| &r.e);
    let in_queue: HashMap<&str, &TodoEntry> = old_q.iter().map(|t| (t.id.as_str(), t)).collect();
    let in_history: HashMap<&str, &DoneTodoEntry> =
        old_h.iter().map(|d| (d.id.as_str(), d)).collect();
    let mut gone: Vec<TodoAttachment> = Vec::new();

    // ---- queue: the project's pending entities, then entries that have no entity ----
    let mut queue: Vec<TodoEntry> = Vec::with_capacity(view.pending.len());
    for (id, e) in &view.pending {
        let entry = if let Some(t) = in_queue.get(id) {
            let mut t = (*t).clone();
            if t.text != e.text.v {
                t.text = e.text.v.clone();
            }
            t
        } else if let Some(d) = in_history.get(id) {
            // Restored on another Mac: moves back, attachments (if any) come along.
            TodoEntry {
                id: d.id.clone(),
                text: e.text.v.clone(),
                created_at_ms: d.created_at_ms,
                agent_kind: d.agent_kind.clone(),
                auto: false,
                attachments: d.attachments.clone(),
            }
        } else {
            TodoEntry {
                id: (*id).to_string(),
                text: e.text.v.clone(),
                created_at_ms: e.c.ms,
                agent_kind: e.c.ak.clone(),
                auto: false,
                attachments: Vec::new(),
            }
        };
        queue.push(entry);
    }
    let mut queued: HashSet<String> = queue.iter().map(|t| t.id.clone()).collect();
    for t in old_q {
        if ent(&t.id).is_none() && queued.insert(t.id.clone()) {
            queue.push(t.clone());
        }
    }

    // ---- history ----
    let mut kept: Vec<DoneTodoEntry> = Vec::new();
    let mut have: HashSet<String> = HashSet::new();
    for d in old_h {
        match ent(&d.id) {
            None => {
                if have.insert(d.id.clone()) {
                    kept.push(d.clone());
                }
            }
            Some(e) if e.st.v == St::Done => {
                if have.insert(d.id.clone()) {
                    let mut d = d.clone();
                    d.text = e.text.v.clone();
                    d.done_at_ms = done_ms(e);
                    kept.push(d);
                }
            }
            // Restored elsewhere: it moved to the queue above.
            Some(e) if e.st.v == St::Pending => {}
            Some(_) => gone.extend(d.attachments.iter().cloned()),
        }
    }
    for t in old_q {
        let Some(e) = ent(&t.id) else { continue };
        match e.st.v {
            St::Done if limit > 0 => {
                if have.insert(t.id.clone()) {
                    kept.push(DoneTodoEntry {
                        id: t.id.clone(),
                        text: e.text.v.clone(),
                        created_at_ms: t.created_at_ms,
                        agent_kind: t.agent_kind.clone(),
                        done_at_ms: done_ms(e),
                        attachments: t.attachments.clone(),
                    });
                }
            }
            // Executed elsewhere while recording is off, or deleted: nothing to keep.
            St::Done | St::Deleted => gone.extend(t.attachments.iter().cloned()),
            St::Pending => {}
        }
    }
    if limit > 0 {
        // Executions from other Macs: only the newest `limit` of the project's history is shown.
        for (id, e) in view.done.iter().rev().take(limit) {
            if have.insert((*id).to_string()) {
                kept.push(DoneTodoEntry {
                    id: (*id).to_string(),
                    text: e.text.v.clone(),
                    created_at_ms: e.c.ms,
                    agent_kind: e.c.ak.clone(),
                    done_at_ms: done_ms(e),
                    attachments: Vec::new(),
                });
            }
        }
        // Chronological, oldest first; the sort is stable so equal times keep their order.
        kept.sort_by_key(|d| d.done_at_ms);
        if kept.len() > limit {
            let overflow = kept.len() - limit;
            for d in kept.drain(..overflow) {
                gone.extend(d.attachments);
            }
        }
    }
    (queue, kept, gone)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::todo_sync::links::testing::MapResolver;
    use crate::todo_sync::{links, order_key};

    const P: &str = "github.com/x/y";

    fn linked(keys: &[(&str, Option<&str>)]) -> State {
        let mut s = State::new("aaaaaaaa-0000".into(), "Studio".into(), 1);
        links::refresh(
            &mut s,
            keys.iter().map(|(k, _)| *k),
            &MapResolver::new(keys),
            1,
        );
        s
    }

    fn put(s: &mut State, id: &str, st: St, key: &str, ms: u64) {
        let h = s.stamp(10);
        s.entities.insert(
            id.to_string(),
            Rec {
                e: Entity {
                    v: 1,
                    p: P.into(),
                    text: Reg {
                        v: format!("text {id}"),
                        h: h.clone(),
                    },
                    ord: Reg {
                        v: key.into(),
                        h: h.clone(),
                    },
                    st: Reg { v: st, h },
                    c: Created {
                        ms,
                        dev: "bbbbbbbb".into(),
                        ak: Some("cursor".into()),
                    },
                    dn: (st == St::Done).then(|| DoneInfo {
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

    fn entry(id: &str, text: &str) -> TodoEntry {
        TodoEntry {
            id: id.into(),
            text: text.into(),
            created_at_ms: 1,
            agent_kind: None,
            auto: false,
            attachments: vec![],
        }
    }

    fn att(name: &str) -> TodoAttachment {
        TodoAttachment {
            id: format!("att-{name}"),
            name: name.into(),
            size: 1,
            is_image: false,
            source_path: String::new(),
            path: format!("/managed/{name}"),
            storage: crate::todo_attachments::TodoAttachmentStorage::Managed,
            thumbnail_path: None,
            source_modified_ms: None,
        }
    }

    fn ids(f: &TodoFile, key: &str) -> Vec<String> {
        f.projects
            .get(key)
            .map(|q| q.iter().map(|t| t.id.clone()).collect())
            .unwrap_or_default()
    }

    fn hist_ids(f: &TodoFile, key: &str) -> Vec<String> {
        f.history
            .get(key)
            .map(|q| q.iter().map(|t| t.id.clone()).collect())
            .unwrap_or_default()
    }

    #[test]
    fn builds_the_queue_in_entity_order_and_preserves_local_state() {
        let mut s = linked(&[("/a", Some(P))]);
        let k = order_key::renumber(3);
        put(&mut s, "t1", St::Pending, &k[0], 1);
        put(&mut s, "t2", St::Pending, &k[1], 2);
        put(&mut s, "t3", St::Pending, &k[2], 3);

        let mut file = TodoFile::default();
        let mut local_t3 = entry("t3", "stale text");
        local_t3.auto = true;
        local_t3.attachments = vec![att("shot.png")];
        file.projects
            .insert("/a".into(), vec![local_t3, entry("solo", "no entity")]);
        let m = materialize(&file, &s, 50);
        assert_eq!(ids(&m.file, "/a"), vec!["t1", "t2", "t3", "solo"]);
        let q = &m.file.projects["/a"];
        assert_eq!(q[0].text, "text t1");
        assert!(
            !q[0].auto && q[0].attachments.is_empty(),
            "remote entries arrive plain"
        );
        assert_eq!(q[0].agent_kind.as_deref(), Some("cursor"));
        assert_eq!(q[2].text, "text t3", "text follows the entity");
        assert!(q[2].auto, "auto stays local");
        assert_eq!(q[2].attachments.len(), 1, "attachments stay local");
        assert_eq!(m.snaps["/a"].pending, vec!["t1", "t2", "t3", "solo"]);
        assert_eq!(m.snaps["/a"].hash["t1"], text_hash("text t1"));
        // Idempotent.
        assert_eq!(materialize(&m.file, &s, 50), m);
    }

    #[test]
    fn moves_between_queue_and_history_carrying_attachments() {
        let mut s = linked(&[("/a", Some(P))]);
        put(&mut s, "ran", St::Done, "", 500);
        put(&mut s, "back", St::Pending, "V", 1);
        let mut file = TodoFile::default();
        let mut ran = entry("ran", "x");
        ran.attachments = vec![att("a.txt")];
        file.projects.insert("/a".into(), vec![ran]);
        file.history.insert(
            "/a".into(),
            vec![DoneTodoEntry {
                id: "back".into(),
                text: "x".into(),
                created_at_ms: 1,
                agent_kind: None,
                done_at_ms: 5,
                attachments: vec![att("b.txt")],
            }],
        );
        let m = materialize(&file, &s, 50);
        assert_eq!(ids(&m.file, "/a"), vec!["back"]);
        assert_eq!(m.file.projects["/a"][0].attachments[0].name, "b.txt");
        assert_eq!(hist_ids(&m.file, "/a"), vec!["ran"]);
        let h = &m.file.history["/a"][0];
        assert_eq!(
            (h.done_at_ms, h.attachments[0].name.as_str()),
            (500, "a.txt")
        );
        assert!(m.cleanup.is_empty(), "nothing left the file");
    }

    #[test]
    fn deleted_entities_leave_and_their_attachments_are_queued_for_cleanup() {
        let mut s = linked(&[("/a", Some(P))]);
        put(&mut s, "dead", St::Deleted, "V", 1);
        let mut file = TodoFile::default();
        let mut dead = entry("dead", "x");
        dead.attachments = vec![att("gone.png")];
        file.projects.insert("/a".into(), vec![dead]);
        let m = materialize(&file, &s, 50);
        assert!(m.file.projects.is_empty());
        assert_eq!(m.cleanup.len(), 1);
        assert_eq!(m.cleanup[0].name, "gone.png");
    }

    #[test]
    fn history_follows_the_limit_and_zero_means_hands_off() {
        let mut s = linked(&[("/a", Some(P))]);
        for i in 1..=6u64 {
            put(&mut s, &format!("d{i}"), St::Done, "", i * 10);
        }
        let file = TodoFile::default();
        let m = materialize(&file, &s, 3);
        assert_eq!(
            hist_ids(&m.file, "/a"),
            vec!["d4", "d5", "d6"],
            "newest three, oldest first"
        );

        // Lowering the limit trims the local view (and reports attachments of dropped entries).
        let mut with_att = m.file.clone();
        with_att.history.get_mut("/a").unwrap()[0].attachments = vec![att("old.png")];
        let m2 = materialize(&with_att, &s, 2);
        assert_eq!(hist_ids(&m2.file, "/a"), vec!["d5", "d6"]);
        assert_eq!(m2.cleanup.len(), 1);

        // Limit 0: remote executions are not added, existing history is kept as it is.
        let m0 = materialize(&file, &s, 0);
        assert!(m0.file.history.is_empty());
        let m0b = materialize(&m.file, &s, 0);
        assert_eq!(hist_ids(&m0b.file, "/a"), vec!["d4", "d5", "d6"]);

        // A remote execution of an entry that is queued here: with recording off the entry just
        // leaves and its attachments are released.
        let mut s0 = linked(&[("/a", Some(P))]);
        put(&mut s0, "run", St::Done, "", 5);
        let mut f0 = TodoFile::default();
        let mut run = entry("run", "x");
        run.attachments = vec![att("r.txt")];
        f0.projects.insert("/a".into(), vec![run]);
        let m = materialize(&f0, &s0, 0);
        assert!(m.file.projects.is_empty() && m.file.history.is_empty());
        assert_eq!(m.cleanup.len(), 1);
    }

    #[test]
    fn a_new_directory_gets_the_whole_project_and_unlinked_ones_are_untouched() {
        let mut s = linked(&[("/new", Some(P)), ("/solo", None)]);
        put(&mut s, "t1", St::Pending, "V", 1);
        put(&mut s, "d1", St::Done, "", 9);
        let mut file = TodoFile::default();
        file.projects
            .insert("/solo".into(), vec![entry("t1", "mine")]);
        let m = materialize(&file, &s, 50);
        assert_eq!(ids(&m.file, "/new"), vec!["t1"]);
        assert_eq!(hist_ids(&m.file, "/new"), vec!["d1"]);
        assert_eq!(ids(&m.file, "/solo"), vec!["t1"]);
        assert_eq!(
            m.file.projects["/solo"][0].text, "mine",
            "a local-only directory is never rewritten"
        );
        assert!(!m.snaps.contains_key("/solo"));
        assert_eq!(m.snaps["/new"].history, vec!["d1"]);
    }

    #[test]
    fn entries_belonging_to_another_project_are_left_alone() {
        let mut s = linked(&[("/a", Some(P))]);
        put(&mut s, "t1", St::Pending, "V", 1);
        s.entities.get_mut("t1").unwrap().e.p = "github.com/other/repo".into();
        let mut file = TodoFile::default();
        file.projects
            .insert("/a".into(), vec![entry("t1", "kept as is")]);
        let m = materialize(&file, &s, 50);
        assert_eq!(m.file.projects["/a"][0].text, "kept as is");
    }
}
