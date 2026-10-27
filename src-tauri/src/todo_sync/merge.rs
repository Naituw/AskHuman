//! Field-level merge of entities (spec todo-sync §5.4, plan §4.2).
//!
//! `join` is the lattice join of two replicas of one entity: every register keeps the value with the
//! larger HLC (ties broken by value so the result never depends on argument order). It is commutative,
//! associative and idempotent, which is what lets replicas converge from any delivery order, with
//! duplicates and after crashes. Deleted entities are tombstones: they keep competing on `st` like any
//! other state and are never purged.

use super::model::*;
use serde_json::Value;

/// Keep the register with the larger `(h, v)`; returns whether `a` changed.
fn join_reg<T: Ord + Clone>(a: &mut Reg<T>, b: &Reg<T>) -> bool {
    if (&b.h, &b.v) > (&a.h, &a.v) {
        *a = b.clone();
        true
    } else {
        false
    }
}

/// Merge `remote` into `local`; returns whether `local` changed.
pub fn join(local: &mut Entity, remote: &Entity) -> bool {
    let before = local.clone();

    local.v = local.v.max(remote.v);
    // An entity never changes project; if two replicas disagree (an id collision), every replica
    // deterministically settles on the smaller identity.
    if remote.p < local.p {
        local.p = remote.p.clone();
    }
    join_reg(&mut local.text, &remote.text);
    join_reg(&mut local.ord, &remote.ord);

    // `dn` belongs to the winning `st`.
    let st_before = local.st.clone();
    join_reg(&mut local.st, &remote.st);
    if local.st != st_before {
        local.dn = remote.dn.clone();
    } else if local.st == remote.st {
        local.dn = std::cmp::max(local.dn.take(), remote.dn.clone());
    }

    // Attachment hints compete like registers; `None` loses to any hint.
    local.att = match (local.att.take(), remote.att.clone()) {
        (Some(a), Some(b)) => Some(
            if (&b.h, b.n, &b.names, &b.dev) > (&a.h, a.n, &a.names, &a.dev) {
                b
            } else {
                a
            },
        ),
        (a, b) => a.or(b),
    };

    // Creation info is immutable: if two copies differ, the earlier creation wins.
    let remote_c = (remote.c.ms, &remote.c.dev, &remote.c.ak);
    if remote_c < (local.c.ms, &local.c.dev, &local.c.ak) {
        local.c = remote.c.clone();
    }

    // Fields only a newer build understands: keep both sides', larger serialization wins a clash.
    for (key, value) in &remote.extra {
        let keep = match local.extra.get(key) {
            Some(mine) => larger_serialization(mine, value).clone(),
            None => value.clone(),
        };
        local.extra.insert(key.clone(), keep);
    }

    *local != before
}

/// The deterministic winner of two clashing unknown fields: the one whose JSON text sorts higher.
fn larger_serialization<'a>(a: &'a Value, b: &'a Value) -> &'a Value {
    let text = |v: &Value| serde_json::to_string(v).unwrap_or_default();
    if text(a) >= text(b) {
        a
    } else {
        b
    }
}

/// Fold one record received from the cloud into the state. Returns whether the local entity changed
/// (new entity or any register advanced).
pub fn merge_remote(state: &mut State, name: &str, remote: Entity, tag: &str, now_ms: u64) -> bool {
    if remote.p.is_empty() {
        return false;
    }
    for h in [
        Some(&remote.text.h),
        Some(&remote.ord.h),
        Some(&remote.st.h),
        remote.att.as_ref().map(|a| &a.h),
    ]
    .into_iter()
    .flatten()
    {
        state.clock.observe(now_ms, h);
    }
    match state.entities.get_mut(name) {
        Some(rec) => {
            let changed = join(&mut rec.e, &remote);
            rec.tag = Some(tag.to_string());
            // The server already holds exactly the merged value → nothing left to upload.
            rec.dirty = rec.e != remote;
            changed
        }
        None => {
            state.entities.insert(
                name.to_string(),
                Rec {
                    e: remote,
                    dirty: false,
                    tag: Some(tag.to_string()),
                },
            );
            true
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn h(wall: u64, dev: &str) -> String {
        super::super::hlc::format(wall, 0, dev)
    }

    fn base(dev: &str, wall: u64) -> Entity {
        Entity {
            v: 1,
            p: "github.com/a/b".into(),
            text: Reg {
                v: "t".into(),
                h: h(wall, dev),
            },
            ord: Reg {
                v: "V".into(),
                h: h(wall, dev),
            },
            st: Reg {
                v: St::Pending,
                h: h(wall, dev),
            },
            c: Created {
                ms: wall,
                dev: dev.into(),
                ak: None,
            },
            dn: None,
            att: None,
            extra: BTreeMap::new(),
        }
    }

    #[test]
    fn registers_merge_independently_per_field() {
        let mut a = base("aaaaaaaa", 1);
        let mut b = a.clone();
        a.text = Reg {
            v: "edited on A".into(),
            h: h(5, "aaaaaaaa"),
        };
        b.ord = Reg {
            v: "k".into(),
            h: h(6, "bbbbbbbb"),
        };
        let mut left = a.clone();
        assert!(join(&mut left, &b));
        let mut right = b.clone();
        assert!(join(&mut right, &a));
        assert_eq!(left, right, "commutative");
        assert_eq!(left.text.v, "edited on A");
        assert_eq!(left.ord.v, "k", "a concurrent edit and a drag both survive");
    }

    #[test]
    fn state_carries_its_done_info_and_deletion_wins_when_later() {
        let mut a = base("aaaaaaaa", 1);
        let mut b = a.clone();
        a.st = Reg {
            v: St::Done,
            h: h(5, "aaaaaaaa"),
        };
        a.dn = Some(DoneInfo {
            ms: 5,
            dev: "dev-a".into(),
        });
        b.st = Reg {
            v: St::Deleted,
            h: h(7, "bbbbbbbb"),
        };
        let mut left = a.clone();
        join(&mut left, &b);
        let mut right = b.clone();
        join(&mut right, &a);
        assert_eq!(left, right);
        assert_eq!(left.st.v, St::Deleted);
        assert_eq!(left.dn, None, "dn follows the winning state");

        // A restore after a completion clears the done info.
        let mut done = a.clone();
        let mut restored = a.clone();
        restored.st = Reg {
            v: St::Pending,
            h: h(9, "bbbbbbbb"),
        };
        restored.dn = None;
        join(&mut done, &restored);
        assert_eq!(done.st.v, St::Pending);
        assert_eq!(done.dn, None);
    }

    #[test]
    fn join_is_idempotent_and_reports_changes() {
        let a = base("aaaaaaaa", 1);
        let mut x = a.clone();
        assert!(!join(&mut x, &a));
        let mut newer = a.clone();
        newer.text = Reg {
            v: "new".into(),
            h: h(9, "bbbbbbbb"),
        };
        assert!(join(&mut x, &newer));
        assert!(!join(&mut x, &newer));
        assert!(!join(&mut x, &a), "an older copy changes nothing");
    }

    #[test]
    fn attachment_hint_and_creation_info_merge_deterministically() {
        let mut a = base("aaaaaaaa", 1);
        let mut b = a.clone();
        a.att = Some(Att {
            n: 1,
            names: vec!["a.png".into()],
            dev: "dev-a".into(),
            h: h(3, "aaaaaaaa"),
        });
        b.att = Some(Att {
            n: 2,
            names: vec!["a.png".into(), "b.png".into()],
            dev: "dev-b".into(),
            h: h(4, "bbbbbbbb"),
        });
        b.c = Created {
            ms: 0,
            dev: "dev-z".into(),
            ak: Some("cursor".into()),
        };
        let mut l = a.clone();
        join(&mut l, &b);
        let mut r = b.clone();
        join(&mut r, &a);
        assert_eq!(l, r);
        assert_eq!(l.att.as_ref().unwrap().n, 2);
        assert_eq!(l.c.ms, 0, "the earlier creation wins");
    }

    #[test]
    fn unknown_fields_union_with_a_deterministic_tie_break() {
        let mut a = base("aaaaaaaa", 1);
        let mut b = a.clone();
        a.extra.insert("x".into(), serde_json::json!(1));
        b.extra.insert("x".into(), serde_json::json!(2));
        b.extra.insert("y".into(), serde_json::json!("only-b"));
        let mut l = a.clone();
        join(&mut l, &b);
        let mut r = b.clone();
        join(&mut r, &a);
        assert_eq!(l, r);
        assert_eq!(l.extra["y"], "only-b");
    }

    #[test]
    fn join_laws_hold_for_random_entities() {
        // Small deterministic generator: three replicas of one entity with random register values.
        let mut s = 0x1234_5678_9abc_def1u64;
        let mut next = move |n: u64| {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            s % n
        };
        let devs = ["aaaaaaaa", "bbbbbbbb", "cccccccc"];
        for case in 0..300 {
            let mut reps: Vec<Entity> = Vec::new();
            for d in devs {
                let mut e = base(d, 1);
                if next(2) == 0 {
                    e.text = Reg {
                        v: format!("t{}", next(4)),
                        h: h(next(6), d),
                    };
                }
                if next(2) == 0 {
                    e.ord = Reg {
                        v: ["V", "a", "k"][next(3) as usize].into(),
                        h: h(next(6), d),
                    };
                }
                if next(2) == 0 {
                    let st = [St::Pending, St::Done, St::Deleted][next(3) as usize];
                    e.st = Reg {
                        v: st,
                        h: h(next(6), d),
                    };
                    e.dn = (st == St::Done).then(|| DoneInfo {
                        ms: next(9),
                        dev: d.into(),
                    });
                }
                if next(3) == 0 {
                    e.att = Some(Att {
                        n: next(3) as u32,
                        names: vec![],
                        dev: d.into(),
                        h: h(next(6), d),
                    });
                }
                reps.push(e);
            }
            let fold = |order: [usize; 3]| {
                let mut acc = reps[order[0]].clone();
                for &i in &order[1..] {
                    join(&mut acc, &reps[i]);
                }
                acc
            };
            let reference = fold([0, 1, 2]);
            for order in [[0, 2, 1], [1, 0, 2], [1, 2, 0], [2, 0, 1], [2, 1, 0]] {
                assert_eq!(fold(order), reference, "case {case}: order {order:?}");
            }
            // Idempotent: re-applying everything changes nothing.
            let mut again = reference.clone();
            for r in &reps {
                assert!(!join(&mut again, r), "case {case}");
            }
            // Associative: (a ⊔ b) ⊔ c == a ⊔ (b ⊔ c).
            let mut bc = reps[1].clone();
            join(&mut bc, &reps[2]);
            let mut a_bc = reps[0].clone();
            join(&mut a_bc, &bc);
            assert_eq!(a_bc, reference, "case {case}");
        }
    }

    #[test]
    fn merge_remote_marks_dirty_only_when_local_has_something_the_server_lacks() {
        let mut state = State::new("device-1".into(), "n".into(), 0);
        let remote = base("bbbbbbbb", 5);
        assert!(merge_remote(&mut state, "t1", remote.clone(), "tag1", 100));
        assert!(!state.entities["t1"].dirty);

        // Local edit (newer than the server's copy), then the server's copy arrives again.
        state.entities.get_mut("t1").unwrap().e.text = Reg {
            v: "mine".into(),
            h: h(9, "aaaaaaaa"),
        };
        assert!(!merge_remote(&mut state, "t1", remote, "tag2", 100));
        assert!(
            state.entities["t1"].dirty,
            "the server does not have the local edit yet"
        );
        assert_eq!(state.entities["t1"].tag.as_deref(), Some("tag2"));

        // The server catches up: its copy equals ours.
        let ours = state.entities["t1"].e.clone();
        merge_remote(&mut state, "t1", ours, "tag3", 100);
        assert!(!state.entities["t1"].dirty);
    }

    #[test]
    fn merge_remote_observes_remote_timestamps_for_the_local_clock() {
        let mut state = State::new("device-1".into(), "n".into(), 0);
        let mut remote = base("bbbbbbbb", 5);
        remote.text.h = h(5_000_000, "bbbbbbbb");
        merge_remote(&mut state, "t1", remote.clone(), "tag", 1_000);
        let next = state.stamp(1_000);
        assert!(next > remote.text.h, "{next} must follow the remote stamp");
    }
}
