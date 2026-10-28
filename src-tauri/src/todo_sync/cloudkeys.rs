//! Virtual project keys: how a project that lives in iCloud but has no directory on this Mac shows up.
//!
//! Every reader of `todos.json` (the todos window, the IM `/todo` commands, the CLI) knows projects as
//! directory keys. A project this Mac has no checkout of is therefore kept under a *virtual key*
//! `icloud:<identity>`: the sync core links it to its identity like a directory, so diff, materialize and
//! push need no special case, and the window shows and edits it like any other project.
//!
//! Lifecycle, all inside the local phase of a tick (`tick::reconcile`):
//!
//! 1. [`ensure`] links a virtual key for every identity that has a live entity and no real directory;
//! 2. the usual diff runs, so edits made in a virtual replica are captured as entity events;
//! 3. [`prune`] drops the virtual keys whose project now has a real directory on this Mac (its todos are
//!    materialized there instead) or no live entity left, together with their snapshot and file slice.
//!
//! The order matters: pruning only after the diff means an edit made in the virtual replica a moment
//! before the repository was cloned still reaches the entity table.

use super::model::{Link, St, State};
use crate::todos::TodoFile;
use std::collections::btree_map::Entry;
use std::collections::BTreeSet;

/// Prefix of a virtual project key.
pub const PREFIX: &str = "icloud:";

pub fn key_of(identity: &str) -> String {
    format!("{PREFIX}{identity}")
}

/// The project identity a virtual key stands for; `None` for a real directory key.
pub fn identity_of(key: &str) -> Option<&str> {
    key.strip_prefix(PREFIX).filter(|p| !p.is_empty())
}

pub fn is_virtual(key: &str) -> bool {
    identity_of(key).is_some()
}

/// Identities that need a virtual replica on this Mac: a live entity, no real linked directory.
fn wanted(state: &State) -> BTreeSet<String> {
    let real: BTreeSet<&str> = state
        .links
        .iter()
        .filter(|(key, _)| !is_virtual(key))
        .filter_map(|(_, link)| link.p.as_deref())
        .collect();
    state
        .entities
        .values()
        .filter(|rec| rec.e.st.v != St::Deleted)
        .map(|rec| rec.e.p.as_str())
        .filter(|p| !real.contains(p))
        .map(str::to_string)
        .collect()
}

/// Link the virtual keys that are missing. Returns whether the link table changed.
pub fn ensure(state: &mut State, now_ms: u64) -> bool {
    let mut touched = false;
    for identity in wanted(state) {
        if let Entry::Vacant(slot) = state.links.entry(key_of(&identity)) {
            slot.insert(Link {
                p: Some(identity),
                cfg_mtime_ms: 0,
                checked_ms: now_ms,
            });
            touched = true;
        }
    }
    touched
}

/// Drop the virtual keys that are no longer wanted: link, snapshot and the slice of `file`. Returns
/// whether anything was dropped. Call it after the diff, before materializing.
pub fn prune(state: &mut State, file: &mut TodoFile) -> bool {
    let wanted = wanted(state);
    let stale: Vec<String> = state
        .links
        .keys()
        .filter(|key| identity_of(key).is_some_and(|p| !wanted.contains(p)))
        .cloned()
        .collect();
    for key in &stale {
        state.links.remove(key);
        state.snapshot.remove(key);
        file.projects.remove(key);
        file.history.remove(key);
    }
    !stale.is_empty()
}

/// Mark every live entity of `identity` deleted ("remove this project from iCloud"): the tombstones
/// travel to every Mac, whose next local phase empties its replicas. Returns how many entities were
/// removed, finished ones included.
pub fn remove_project(state: &mut State, identity: &str, now_ms: u64) -> usize {
    let ids: Vec<String> = state
        .entities
        .iter()
        .filter(|(_, rec)| rec.e.p == identity && rec.e.st.v != St::Deleted)
        .map(|(id, _)| id.clone())
        .collect();
    for id in &ids {
        let h = state.stamp(now_ms);
        if let Some(rec) = state.entities.get_mut(id) {
            rec.e.st.v = St::Deleted;
            rec.e.st.h = h;
            rec.e.dn = None;
            rec.dirty = true;
        }
    }
    ids.len()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::todo_sync::model::{Created, Entity, Rec, Reg};

    fn entity(p: &str, st: St) -> Rec {
        Rec {
            e: Entity {
                v: 1,
                p: p.to_string(),
                text: Reg {
                    v: "t".into(),
                    h: "h0".into(),
                },
                ord: Reg {
                    v: "a".into(),
                    h: "h0".into(),
                },
                st: Reg {
                    v: st,
                    h: "h0".into(),
                },
                c: Created {
                    ms: 1,
                    dev: "d".into(),
                    ak: None,
                },
                dn: None,
                att: None,
                extra: Default::default(),
            },
            dirty: false,
            tag: None,
        }
    }

    fn state() -> State {
        State::new("device-1".into(), "Mac".into(), 1)
    }

    fn link(state: &mut State, key: &str, p: Option<&str>) {
        state.links.insert(
            key.to_string(),
            Link {
                p: p.map(str::to_string),
                cfg_mtime_ms: 5,
                checked_ms: 5,
            },
        );
    }

    #[test]
    fn keys_round_trip_and_real_directories_are_not_virtual() {
        assert_eq!(key_of("github.com/me/app"), "icloud:github.com/me/app");
        assert_eq!(
            identity_of("icloud:github.com/me/app"),
            Some("github.com/me/app")
        );
        assert_eq!(identity_of("/Users/me/app"), None);
        assert_eq!(identity_of("icloud:"), None);
        assert!(!is_virtual("/Users/me/icloud:odd"));
    }

    #[test]
    fn a_project_without_a_directory_gets_a_virtual_link() {
        let mut s = state();
        s.entities
            .insert("t1".into(), entity("github.com/me/app", St::Pending));
        assert!(ensure(&mut s, 9));
        let l = &s.links["icloud:github.com/me/app"];
        assert_eq!(l.p.as_deref(), Some("github.com/me/app"));
        assert!(!ensure(&mut s, 10), "idempotent");
    }

    #[test]
    fn a_project_with_a_real_directory_needs_none() {
        let mut s = state();
        s.entities
            .insert("t1".into(), entity("github.com/me/app", St::Pending));
        link(&mut s, "/Users/me/app", Some("github.com/me/app"));
        assert!(!ensure(&mut s, 9));
        assert!(s.links.keys().all(|k| !is_virtual(k)));
    }

    #[test]
    fn local_only_directories_do_not_count_as_a_checkout() {
        let mut s = state();
        s.entities
            .insert("t1".into(), entity("github.com/me/app", St::Done));
        link(&mut s, "/Users/me/scratch", None);
        assert!(ensure(&mut s, 9));
        assert!(s.links.contains_key("icloud:github.com/me/app"));
    }

    #[test]
    fn deleted_entities_do_not_keep_a_project_alive() {
        let mut s = state();
        s.entities
            .insert("t1".into(), entity("github.com/me/app", St::Deleted));
        assert!(!ensure(&mut s, 9));
    }

    #[test]
    fn prune_drops_the_replica_once_a_directory_takes_over() {
        let mut s = state();
        s.entities
            .insert("t1".into(), entity("github.com/me/app", St::Pending));
        ensure(&mut s, 9);
        let mut file = TodoFile::default();
        file.projects
            .insert("icloud:github.com/me/app".into(), Vec::new());
        s.snapshot
            .insert("icloud:github.com/me/app".into(), Default::default());

        assert!(!prune(&mut s, &mut file), "still wanted");

        link(&mut s, "/Users/me/app", Some("github.com/me/app"));
        assert!(prune(&mut s, &mut file));
        assert!(!s.links.contains_key("icloud:github.com/me/app"));
        assert!(!s.snapshot.contains_key("icloud:github.com/me/app"));
        assert!(!file.projects.contains_key("icloud:github.com/me/app"));
        assert!(s.links.contains_key("/Users/me/app"), "real links stay");
    }

    #[test]
    fn prune_drops_the_replica_of_a_project_that_is_gone() {
        let mut s = state();
        s.entities
            .insert("t1".into(), entity("github.com/me/app", St::Pending));
        ensure(&mut s, 9);
        s.entities.get_mut("t1").unwrap().e.st.v = St::Deleted;
        let mut file = TodoFile::default();
        assert!(prune(&mut s, &mut file));
        assert!(s.links.is_empty());
    }

    #[test]
    fn removing_a_project_tombstones_all_of_its_entities_and_only_those() {
        let mut s = state();
        s.entities
            .insert("a".into(), entity("github.com/me/app", St::Pending));
        s.entities
            .insert("b".into(), entity("github.com/me/app", St::Done));
        s.entities
            .insert("c".into(), entity("github.com/me/app", St::Deleted));
        s.entities
            .insert("d".into(), entity("github.com/me/other", St::Pending));

        assert_eq!(remove_project(&mut s, "github.com/me/app", 100), 2);

        for id in ["a", "b"] {
            let rec = &s.entities[id];
            assert_eq!(rec.e.st.v, St::Deleted, "{id}");
            assert!(rec.dirty, "{id} must be pushed");
            assert_ne!(rec.e.st.h, "h0", "{id} needs a fresh stamp");
        }
        assert!(!s.entities["c"].dirty, "already deleted: untouched");
        assert_eq!(s.entities["d"].e.st.v, St::Pending);
        assert_eq!(
            remove_project(&mut s, "github.com/me/app", 101),
            0,
            "idempotent"
        );
    }
}
