//! The project list a phone sees for todos (docs/specs/ios-todos.md §6.1).
//!
//! A pure port of the todos window's sidebar model (`src/views/todos/todoProjects.ts`): the same
//! labels, the same merging of the directories of one repository, the same sections. It has no IO
//! (the daemon gathers the inputs) and it never puts a path on the wire: a project is named by an
//! opaque id, and `hint` is only as much of the parent folders as it takes to tell same-named
//! projects apart. Change the window's model and this file together.

use std::collections::HashMap;

use sha2::{Digest, Sha256};

use super::wire::{TodoProjectRow, TODO_MAX_PROJECTS};

/// What the Mac knows about one project directory, in the order the window ranks them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub key: String,
    pub name: String,
    /// Pending todos under this key.
    pub pending: usize,
    /// Finished todos under this key.
    pub history: usize,
    /// 0 = an Agent works in the project, 1 = an idle Agent, 2 = none.
    pub activity: u8,
}

/// A synced project, merged across this Mac's directories (mirror of
/// `todo_sync::service::SyncProject`, without the timestamps).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncedProject {
    /// Normalized git remote, e.g. `github.com/acme/app`.
    pub identity: String,
    /// Project keys of this Mac's directories that belong to it.
    pub keys: Vec<String>,
    /// `icloud:<identity>` while no directory of this Mac has the project.
    pub cloud_key: Option<String>,
    /// Names of the Macs that have a directory for it.
    pub macs: Vec<String>,
}

/// Stable id of a project on the wire: the phone never sees (or sends) a path.
pub fn project_id(key: &str) -> String {
    Sha256::digest(key.as_bytes())
        .iter()
        .take(8)
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn split_path(path: &str) -> Vec<String> {
    path.replace('\\', "/")
        .split('/')
        .filter(|part| !part.is_empty())
        .map(str::to_string)
        .collect()
}

/// Last path component (the whole text when it has none).
pub fn basename(path: &str) -> String {
    split_path(path).pop().unwrap_or_else(|| path.to_string())
}

/// Gray suffix per key telling same-named projects apart: the fewest trailing parent folders that
/// make every label of a name distinct (`app · a`, `app · b`). Unique names get none. Port of the
/// window's `labelProjects`.
pub fn label_hints(items: &[(String, String)]) -> HashMap<String, String> {
    let mut hints: HashMap<String, String> = HashMap::new();
    let mut by_name: Vec<(String, Vec<String>)> = Vec::new();
    for (key, name) in items {
        if hints.contains_key(key) {
            continue;
        }
        hints.insert(key.clone(), String::new());
        match by_name.iter_mut().find(|(n, _)| n == name) {
            Some((_, keys)) => keys.push(key.clone()),
            None => by_name.push((name.clone(), vec![key.clone()])),
        }
    }
    for (_, keys) in &by_name {
        if keys.len() < 2 {
            continue;
        }
        // Parent folders, nearest first.
        let parents: Vec<Vec<String>> = keys
            .iter()
            .map(|key| {
                let mut parts = split_path(key);
                parts.pop();
                parts.reverse();
                parts
            })
            .collect();
        let deepest = parents.iter().map(Vec::len).max().unwrap_or(0);
        let hint_at = |chain: &[String], depth: usize| -> String {
            let mut taken: Vec<&str> = chain.iter().take(depth).map(String::as_str).collect();
            taken.reverse();
            taken.join("/")
        };
        let mut depth = 1;
        while depth < deepest {
            let candidates: Vec<String> = parents.iter().map(|c| hint_at(c, depth)).collect();
            let distinct: std::collections::HashSet<&String> = candidates.iter().collect();
            if distinct.len() == candidates.len() {
                break;
            }
            depth += 1;
        }
        for (key, chain) in keys.iter().zip(&parents) {
            hints.insert(key.clone(), hint_at(chain, depth));
        }
    }
    hints
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Scope {
    Synced,
    CloudOnly,
    LocalOnly,
}

impl Scope {
    fn wire(self) -> &'static str {
        match self {
            Scope::Synced => "synced",
            Scope::CloudOnly => "cloudOnly",
            Scope::LocalOnly => "localOnly",
        }
    }
}

/// A project as iCloud mode sees it: directories of one repository merged into one.
struct Merged {
    /// What a click opens.
    key: String,
    pending: usize,
    history: usize,
    activity: u8,
    scope: Scope,
    /// Normalized remote; `None` for projects without one.
    identity: Option<String>,
    name: String,
    macs: Vec<String>,
}

/// Folds `candidates` (in the window's ranking, which puts the most recently used directory of a
/// repository first) into one entry per repository, then appends synced projects the list does
/// not mention (a project whose todos are all finished lives only in the history). Port of
/// `mergeProjects`.
fn merge(candidates: &[Candidate], sync: &[SyncedProject]) -> Vec<Merged> {
    let mut by_sync_key: HashMap<&str, &SyncedProject> = HashMap::new();
    for project in sync {
        for key in &project.keys {
            by_sync_key.insert(key, project);
        }
        if let Some(key) = &project.cloud_key {
            by_sync_key.insert(key, project);
        }
    }
    let mut merged: Vec<Merged> = Vec::new();
    let mut by_identity: HashMap<&str, usize> = HashMap::new();
    let from_sync = |key: &str, pending, history, activity, project: &SyncedProject| Merged {
        key: key.to_string(),
        pending,
        history,
        activity,
        scope: if project.keys.is_empty() {
            Scope::CloudOnly
        } else {
            Scope::Synced
        },
        identity: Some(project.identity.clone()),
        name: basename(&project.identity),
        macs: project.macs.clone(),
    };
    for candidate in candidates {
        let Some(project) = by_sync_key.get(candidate.key.as_str()) else {
            merged.push(Merged {
                key: candidate.key.clone(),
                pending: candidate.pending,
                history: candidate.history,
                activity: candidate.activity,
                scope: Scope::LocalOnly,
                identity: None,
                name: candidate.name.clone(),
                macs: Vec::new(),
            });
            continue;
        };
        if let Some(&index) = by_identity.get(project.identity.as_str()) {
            // Replicas agree once the local phase ran; until then the larger count is the newer one.
            let seen = &mut merged[index];
            seen.pending = seen.pending.max(candidate.pending);
            seen.history = seen.history.max(candidate.history);
            seen.activity = seen.activity.min(candidate.activity);
            continue;
        }
        by_identity.insert(&project.identity, merged.len());
        merged.push(from_sync(
            &candidate.key,
            candidate.pending,
            candidate.history,
            candidate.activity,
            project,
        ));
    }
    for project in sync {
        if by_identity.contains_key(project.identity.as_str()) {
            continue;
        }
        let Some(key) = project.keys.first().or(project.cloud_key.as_ref()) else {
            continue;
        };
        by_identity.insert(&project.identity, merged.len());
        merged.push(from_sync(key, 0, 0, 2, project));
    }
    merged
}

fn agent_of(activity: u8) -> Option<String> {
    match activity {
        0 => Some("working".to_string()),
        1 => Some("idle".to_string()),
        _ => None,
    }
}

fn hint_of(hints: &HashMap<String, String>, key: &str) -> Option<String> {
    hints.get(key).filter(|hint| !hint.is_empty()).cloned()
}

/// The rows of `todo.projects`, and whether more projects existed than the wire carries.
///
/// With nothing synced this is the window's local mode: `withTodos` first, then `recent`. With
/// synced projects it is iCloud mode: one row per repository, `synced` rows (todos pending), the
/// `empty` ones, then the projects without a remote (`localOnly`).
pub fn build_rows(candidates: &[Candidate], sync: &[SyncedProject]) -> (Vec<TodoProjectRow>, bool) {
    let mut rows = if sync.is_empty() {
        local_rows(candidates)
    } else {
        synced_rows(candidates, sync)
    };
    let truncated = rows.len() > TODO_MAX_PROJECTS;
    rows.truncate(TODO_MAX_PROJECTS);
    (rows, truncated)
}

fn local_rows(candidates: &[Candidate]) -> Vec<TodoProjectRow> {
    let hints = label_hints(
        &candidates
            .iter()
            .map(|c| (c.key.clone(), c.name.clone()))
            .collect::<Vec<_>>(),
    );
    let row = |c: &Candidate, section: &str| TodoProjectRow {
        id: project_id(&c.key),
        name: c.name.clone(),
        hint: hint_of(&hints, &c.key),
        repo: None,
        pending: c.pending as u64,
        history: c.history as u64,
        scope: "local".to_string(),
        section: section.to_string(),
        agent: agent_of(c.activity),
        macs: Vec::new(),
    };
    let with_todos = candidates.iter().filter(|c| c.pending > 0);
    let recent = candidates.iter().filter(|c| c.pending == 0);
    with_todos
        .map(|c| row(c, "withTodos"))
        .chain(recent.map(|c| row(c, "recent")))
        .collect()
}

fn synced_rows(candidates: &[Candidate], sync: &[SyncedProject]) -> Vec<TodoProjectRow> {
    let merged = merge(candidates, sync);
    // Synced projects are labelled by repository address (same-named repositories told apart by
    // owner), the others by directory like the local mode.
    let remote = label_hints(
        &merged
            .iter()
            .filter_map(|m| m.identity.as_ref().map(|id| (id.clone(), m.name.clone())))
            .collect::<Vec<_>>(),
    );
    let local = label_hints(
        &merged
            .iter()
            .filter(|m| m.identity.is_none())
            .map(|m| (m.key.clone(), m.name.clone()))
            .collect::<Vec<_>>(),
    );
    let row = |m: &Merged, section: &str| TodoProjectRow {
        id: project_id(&m.key),
        name: m.name.clone(),
        hint: match &m.identity {
            Some(identity) => hint_of(&remote, identity),
            None => hint_of(&local, &m.key),
        },
        repo: m.identity.clone(),
        pending: m.pending as u64,
        history: m.history as u64,
        scope: m.scope.wire().to_string(),
        section: section.to_string(),
        agent: agent_of(m.activity),
        macs: m.macs.clone(),
    };
    let mut synced = Vec::new();
    let mut empty = Vec::new();
    let mut local_only = Vec::new();
    for m in &merged {
        if m.pending == 0 {
            empty.push(row(m, "empty"));
        } else if m.scope == Scope::LocalOnly {
            local_only.push(row(m, "localOnly"));
        } else {
            synced.push(row(m, "synced"));
        }
    }
    synced.into_iter().chain(empty).chain(local_only).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cand(key: &str, pending: usize) -> Candidate {
        Candidate {
            key: key.to_string(),
            name: basename(key),
            pending,
            history: 0,
            activity: 2,
        }
    }

    fn synced(identity: &str, keys: &[&str], cloud_key: Option<&str>) -> SyncedProject {
        SyncedProject {
            identity: identity.to_string(),
            keys: keys.iter().map(|k| k.to_string()).collect(),
            cloud_key: cloud_key.map(str::to_string),
            macs: if keys.is_empty() {
                Vec::new()
            } else {
                vec!["Studio".to_string()]
            },
        }
    }

    fn hints(keys: &[&str]) -> HashMap<String, String> {
        label_hints(
            &keys
                .iter()
                .map(|k| (k.to_string(), basename(k)))
                .collect::<Vec<_>>(),
        )
    }

    const APP: &str = "github.com/acme/app";
    const DOTS: &str = "github.com/acme/dotfiles";
    const CLOUD_DOTS: &str = "icloud:github.com/acme/dotfiles";

    /// Two directories of one repository, a cloud-only project, an empty one and a project without
    /// a remote (the window's `iCloudWorld`).
    fn icloud_world() -> (Vec<Candidate>, Vec<SyncedProject>) {
        (
            vec![
                cand("/Users/me/dev/app-wt", 3),
                cand("/Users/me/dev/app", 3),
                Candidate {
                    name: "dotfiles".into(),
                    ..cand(CLOUD_DOTS, 2)
                },
                cand("/Users/me/Scratch/notes", 2),
                cand("/Users/me/dev/blog", 0),
                cand("/Users/me/dev/lonely", 0),
            ],
            vec![
                synced(APP, &["/Users/me/dev/app", "/Users/me/dev/app-wt"], None),
                synced(DOTS, &[], Some(CLOUD_DOTS)),
                synced("github.com/acme/blog", &["/Users/me/dev/blog"], None),
            ],
        )
    }

    #[test]
    fn project_ids_are_stable_opaque_and_path_free() {
        let id = project_id("/Users/me/dev/app");
        assert_eq!(id.len(), 16);
        assert!(id.bytes().all(|b| b.is_ascii_hexdigit()));
        assert_eq!(id, project_id("/Users/me/dev/app"));
        assert_ne!(id, project_id("/Users/me/dev/app2"));
        assert!(!id.contains("app"));
    }

    #[test]
    fn unique_names_have_no_hint() {
        let hints = hints(&["/a/app", "/a/blog"]);
        assert_eq!(hints["/a/app"], "");
        assert_eq!(hints["/a/blog"], "");
    }

    #[test]
    fn same_named_projects_are_told_apart_by_their_parent_folder() {
        let hints = hints(&["/x/a/app", "/x/b/app", "/x/blog"]);
        assert_eq!(hints["/x/a/app"], "a");
        assert_eq!(hints["/x/b/app"], "b");
        assert_eq!(hints["/x/blog"], "");
    }

    #[test]
    fn hints_go_up_as_far_as_needed_to_be_distinct() {
        let hints = hints(&["/x/a/app", "/y/a/app", "/z/b/app"]);
        assert_eq!(hints["/x/a/app"], "x/a");
        assert_eq!(hints["/y/a/app"], "y/a");
        assert_eq!(hints["/z/b/app"], "z/b");
    }

    #[test]
    fn a_path_with_fewer_parents_than_its_twin_copes() {
        let hints = hints(&["/app", "/a/app"]);
        assert_eq!(hints["/app"], "");
        assert_eq!(hints["/a/app"], "a");
    }

    #[test]
    fn windows_separators_are_understood() {
        assert_eq!(basename("C:\\dev\\app"), "app");
        let hints = hints(&["C:\\x\\app", "C:\\y\\app"]);
        assert_eq!(hints["C:\\x\\app"], "x");
    }

    #[test]
    fn local_mode_puts_projects_with_todos_first_and_keeps_the_ranking() {
        let candidates = vec![
            cand("/w/idle", 0),
            cand("/w/busy", 2),
            Candidate {
                activity: 0,
                ..cand("/w/working", 1)
            },
        ];
        let (rows, truncated) = build_rows(&candidates, &[]);
        assert!(!truncated);
        let summary: Vec<_> = rows
            .iter()
            .map(|r| (r.name.as_str(), r.section.as_str(), r.pending))
            .collect();
        assert_eq!(
            summary,
            [
                ("busy", "withTodos", 2),
                ("working", "withTodos", 1),
                ("idle", "recent", 0)
            ]
        );
        assert!(rows.iter().all(|r| r.scope == "local" && r.repo.is_none()));
        assert_eq!(rows[1].agent.as_deref(), Some("working"));
        assert_eq!(rows[0].agent, None);
    }

    #[test]
    fn local_mode_tells_same_named_directories_apart() {
        let candidates = vec![cand("/x/a/app", 1), cand("/x/b/app", 1)];
        let (rows, _) = build_rows(&candidates, &[]);
        assert_eq!(rows[0].hint.as_deref(), Some("a"));
        assert_eq!(rows[1].hint.as_deref(), Some("b"));
    }

    #[test]
    fn icloud_mode_merges_directories_of_one_repository_into_the_most_recent_row() {
        let (candidates, sync) = icloud_world();
        let (rows, _) = build_rows(&candidates, &sync);
        let synced_rows: Vec<_> = rows.iter().filter(|r| r.section == "synced").collect();
        assert_eq!(
            synced_rows
                .iter()
                .map(|r| (r.name.as_str(), r.id.clone(), r.pending, r.scope.as_str()))
                .collect::<Vec<_>>(),
            [
                ("app", project_id("/Users/me/dev/app-wt"), 3, "synced"),
                ("dotfiles", project_id(CLOUD_DOTS), 2, "cloudOnly"),
            ]
        );
        assert_eq!(synced_rows[0].repo.as_deref(), Some(APP));
        assert_eq!(synced_rows[0].macs, ["Studio"]);
        assert!(synced_rows[1].macs.is_empty());
    }

    #[test]
    fn each_project_counts_once_however_many_directories_it_has() {
        let (candidates, sync) = icloud_world();
        let (rows, _) = build_rows(&candidates, &sync);
        assert_eq!(rows.iter().map(|r| r.pending).sum::<u64>(), 3 + 2 + 2);
    }

    #[test]
    fn icloud_sections_are_synced_then_empty_then_local_only() {
        let (candidates, sync) = icloud_world();
        let (rows, _) = build_rows(&candidates, &sync);
        let order: Vec<_> = rows
            .iter()
            .map(|r| (r.name.as_str(), r.section.as_str()))
            .collect();
        assert_eq!(
            order,
            [
                ("app", "synced"),
                ("dotfiles", "synced"),
                ("blog", "empty"),
                ("lonely", "empty"),
                ("notes", "localOnly"),
            ]
        );
        let notes = rows.iter().find(|r| r.name == "notes").unwrap();
        assert_eq!(notes.scope, "localOnly");
        assert_eq!(notes.repo, None);
    }

    #[test]
    fn same_named_repositories_are_told_apart_by_owner() {
        let candidates = vec![cand("/x/app", 1), cand("/y/app", 2)];
        let sync = vec![
            synced("github.com/acme/app", &["/x/app"], None),
            synced("github.com/other/app", &["/y/app"], None),
        ];
        let (rows, _) = build_rows(&candidates, &sync);
        assert_eq!(
            rows.iter().map(|r| r.hint.clone()).collect::<Vec<_>>(),
            [Some("acme".to_string()), Some("other".to_string())]
        );
    }

    #[test]
    fn synced_projects_the_list_does_not_mention_still_get_a_row() {
        let candidates = vec![cand("/Users/me/dev/app", 1)];
        let sync = vec![
            synced(APP, &["/Users/me/dev/app"], None),
            synced(DOTS, &[], Some(CLOUD_DOTS)),
        ];
        let (rows, _) = build_rows(&candidates, &sync);
        let empty: Vec<_> = rows.iter().filter(|r| r.section == "empty").collect();
        assert_eq!(empty.len(), 1);
        assert_eq!(empty[0].id, project_id(CLOUD_DOTS));
        assert_eq!(empty[0].scope, "cloudOnly");
        assert_eq!(empty[0].pending, 0);
    }

    #[test]
    fn replicas_report_the_larger_count_and_the_busiest_agent() {
        let candidates = vec![
            cand("/Users/me/dev/app-wt", 2),
            Candidate {
                history: 4,
                activity: 1,
                ..cand("/Users/me/dev/app", 5)
            },
        ];
        let sync = vec![synced(
            APP,
            &["/Users/me/dev/app", "/Users/me/dev/app-wt"],
            None,
        )];
        let (rows, _) = build_rows(&candidates, &sync);
        assert_eq!(rows.len(), 1);
        assert_eq!((rows[0].pending, rows[0].history), (5, 4));
        assert_eq!(rows[0].agent.as_deref(), Some("idle"));
    }

    #[test]
    fn nothing_synced_means_the_local_model_even_in_icloud_mode() {
        let (candidates, _) = icloud_world();
        let (rows, _) = build_rows(&candidates, &[]);
        assert!(rows.iter().all(|r| r.scope == "local"));
        assert_eq!(rows[0].section, "withTodos");
    }

    #[test]
    fn the_list_is_capped_and_says_so() {
        let candidates: Vec<_> = (0..TODO_MAX_PROJECTS + 5)
            .map(|i| cand(&format!("/w/p{i}"), 1))
            .collect();
        let (rows, truncated) = build_rows(&candidates, &[]);
        assert_eq!(rows.len(), TODO_MAX_PROJECTS);
        assert!(truncated);
    }

    #[test]
    fn rows_never_carry_a_path() {
        let (candidates, sync) = icloud_world();
        for (rows, _) in [build_rows(&candidates, &sync), build_rows(&candidates, &[])] {
            let json = serde_json::to_string(&rows).unwrap();
            assert!(!json.contains("/Users"), "{json}");
        }
    }
}
