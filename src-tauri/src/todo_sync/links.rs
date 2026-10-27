//! The link table: local project key (git root) → project identity (spec todo-sync §5.2).
//!
//! The table is a cache the window path can read without touching git. Refreshing it only `stat`s each
//! directory's git config; the config is re-read when its modification time moved.

use super::cloudkeys;
use super::identity::IdentityResolver;
use super::model::{Link, State};

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Refreshed {
    /// The table (or a snapshot) changed and `state.json` needs saving.
    pub touched: bool,
    /// Keys that used to have a link with a different identity.
    pub identity_changed: Vec<String>,
}

/// Re-resolve `keys` and update `state.links`.
///
/// When a key's identity changes (a remote was added, removed or repointed) its snapshot is dropped:
/// the directory is "seen for the first time" again, so entries that vanish from its queue because of
/// the move are never read as deletions in the old project (spec §5.1: they join the new project under
/// new ids; see `diff::regenerate_foreign_ids`).
pub fn refresh<'a>(
    state: &mut State,
    keys: impl IntoIterator<Item = &'a str>,
    resolver: &dyn IdentityResolver,
    now_ms: u64,
) -> Refreshed {
    let mut out = Refreshed::default();
    for key in keys {
        if let Some(identity) = cloudkeys::identity_of(key) {
            // A virtual key (a project without a directory here) stands for its identity: there is
            // nothing on disk to look at.
            if state.links.get(key).and_then(|l| l.p.as_deref()) != Some(identity) {
                state.links.insert(
                    key.to_string(),
                    Link {
                        p: Some(identity.to_string()),
                        cfg_mtime_ms: 0,
                        checked_ms: now_ms,
                    },
                );
                out.touched = true;
            }
            continue;
        }
        let prev = state.links.get(key);
        let next = resolver.resolve(key, prev, now_ms);
        match prev {
            // Unchanged: keep the stored value (and its old `checkedMs`) so that an idle tick
            // does not dirty the state file.
            Some(prev) if prev.p == next.p && prev.cfg_mtime_ms == next.cfg_mtime_ms => continue,
            Some(prev) if prev.p != next.p => {
                out.identity_changed.push(key.to_string());
                state.snapshot.remove(key);
            }
            _ => {}
        }
        state.links.insert(key.to_string(), next);
        out.touched = true;
    }
    out
}

#[cfg(test)]
pub(crate) mod testing {
    use super::*;
    use crate::todo_sync::model::Link;
    use std::cell::RefCell;
    use std::collections::BTreeMap;

    /// A resolver driven by a table, with a switchable identity per key.
    #[derive(Default)]
    pub struct MapResolver {
        pub map: RefCell<BTreeMap<String, Option<String>>>,
    }

    impl MapResolver {
        pub fn new(pairs: &[(&str, Option<&str>)]) -> MapResolver {
            let r = MapResolver::default();
            for (k, p) in pairs {
                r.set(k, *p);
            }
            r
        }

        pub fn set(&self, key: &str, identity: Option<&str>) {
            self.map
                .borrow_mut()
                .insert(key.to_string(), identity.map(str::to_string));
        }
    }

    impl IdentityResolver for MapResolver {
        fn resolve(&self, key: &str, _prev: Option<&Link>, now_ms: u64) -> Link {
            Link {
                p: self.map.borrow().get(key).cloned().flatten(),
                cfg_mtime_ms: 0,
                checked_ms: now_ms,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::testing::MapResolver;
    use super::*;
    use crate::todo_sync::model::Snap;

    fn state() -> State {
        State::new("device-1".into(), "Studio".into(), 1)
    }

    #[test]
    fn new_keys_are_linked_and_idle_refresh_changes_nothing() {
        let r = MapResolver::new(&[("/a", Some("github.com/x/y")), ("/b", None)]);
        let mut s = state();
        let first = refresh(&mut s, ["/a", "/b"], &r, 10);
        assert!(first.touched && first.identity_changed.is_empty());
        assert_eq!(s.project_of("/a"), Some("github.com/x/y"));
        assert_eq!(s.project_of("/b"), None);
        assert!(s.links.contains_key("/b"), "local-only keys are cached too");
        let before = s.links.clone();
        let again = refresh(&mut s, ["/a", "/b"], &r, 99);
        assert!(!again.touched, "an idle refresh must not dirty the state");
        assert_eq!(s.links, before, "checkedMs of an unchanged link stays put");
    }

    #[test]
    fn identity_change_drops_the_snapshot_of_that_key_only() {
        let r = MapResolver::new(&[
            ("/a", Some("github.com/x/y")),
            ("/c", Some("github.com/x/y")),
        ]);
        let mut s = state();
        refresh(&mut s, ["/a", "/c"], &r, 1);
        s.snapshot.insert("/a".into(), Snap::default());
        s.snapshot.insert("/c".into(), Snap::default());

        r.set("/a", Some("github.com/fork/y"));
        let out = refresh(&mut s, ["/a", "/c"], &r, 2);
        assert_eq!(out.identity_changed, vec!["/a".to_string()]);
        assert!(!s.snapshot.contains_key("/a"));
        assert!(s.snapshot.contains_key("/c"));
        assert_eq!(s.project_of("/a"), Some("github.com/fork/y"));

        // Losing the remote unlinks it (local only) and also forgets the snapshot.
        s.snapshot.insert("/a".into(), Snap::default());
        r.set("/a", None);
        let out = refresh(&mut s, ["/a"], &r, 3);
        assert_eq!(out.identity_changed, vec!["/a".to_string()]);
        assert_eq!(s.project_of("/a"), None);
        assert!(!s.snapshot.contains_key("/a"));
    }
}
