//! Todos on the phone, Mac side (docs/specs/ios-todos.md §6): gathers the project list, reads one
//! project's todos and applies the phone's writes through the same `todos.rs` functions the
//! window and the CLI use.
//!
//! Everything works on a [`TodoEnv`] (where `todos.json` lives, what the sync side says) so the
//! tests run against a temp directory; the daemon hands in the real one. The project list itself
//! is assembled by the pure `ios::todos_view`.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::UNIX_EPOCH;

use serde_json::Value;

use super::*;
use crate::agents::workspaces::Workspace;
use crate::commands::{rank_todo_projects, RankedTodoProject};
use crate::ios::gateway::TodoApplied;
use crate::ios::todos_view::{self, project_id, Candidate, SyncedProject};
use crate::ios::wire::{self, TODO_MAX_HISTORY, TODO_MAX_PENDING};
use crate::todo_sync::cloudkeys;
use crate::todo_sync::service::{self, ProjectDetails};
use crate::todos::{self, AddError, TodoFile};

/// What the sync side tells the todo screen. Every answer is local metadata: nothing here waits
/// for the network.
pub(super) trait SyncView: Send + Sync {
    /// Sync is on for this Mac.
    fn enabled(&self) -> bool;
    fn status(&self) -> service::Status;
    fn projects(&self) -> Vec<service::SyncProject>;
    /// The local phase of a tick for `key`: a project that has only just appeared gets its todos
    /// materialized before they are read.
    fn before_read(&self, key: &str);
    fn details(&self, key: &str) -> ProjectDetails;
}

struct RealSync {
    env: crate::todo_sync::tick::Env,
}

impl SyncView for RealSync {
    fn enabled(&self) -> bool {
        service::is_enabled()
    }

    fn status(&self) -> service::Status {
        service::status_with(&self.env, service::supported())
    }

    fn projects(&self) -> Vec<service::SyncProject> {
        service::projects_with(&self.env)
    }

    fn before_read(&self, key: &str) {
        service::before_read_with(&self.env, key);
    }

    fn details(&self, key: &str) -> ProjectDetails {
        service::project_details_with(&self.env, key)
    }
}

pub(super) struct TodoEnv {
    path: PathBuf,
    lock: PathBuf,
    history_limit: usize,
    sync: Box<dyn SyncView>,
}

impl TodoEnv {
    pub(super) fn production() -> Self {
        let env = service::production_env();
        Self {
            path: env.todos_path.clone(),
            lock: env.todos_lock.clone(),
            history_limit: env.history_limit,
            sync: Box::new(RealSync { env }),
        }
    }

    fn file(&self) -> TodoFile {
        todos::read_checked_at(&self.path).unwrap_or_default()
    }
}

/// A signature of the todo store that changes whenever something writes to it (the file is
/// replaced atomically, so its modification time and size move with every write). Masked to 52
/// bits so any JSON reader keeps it exact.
pub(super) fn rev_of(path: &Path) -> u64 {
    let Ok(meta) = std::fs::metadata(path) else {
        return 0;
    };
    let nanos = meta
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |since| since.as_nanos() as u64);
    (nanos ^ meta.len().rotate_left(40)) & ((1 << 52) - 1)
}

/// What a project id is looked up in: everything on this Mac that can be a project.
struct World {
    file: TodoFile,
    ranked: Vec<RankedTodoProject>,
    sync: Vec<service::SyncProject>,
    enabled: bool,
}

impl World {
    fn gather(env: &TodoEnv, workspaces: Vec<Workspace>, agents: Option<&Value>) -> Self {
        let file = env.file();
        let ranked = rank_todo_projects(&file.projects, workspaces, agents);
        let enabled = env.sync.enabled();
        let sync = if enabled {
            env.sync.projects()
        } else {
            Vec::new()
        };
        Self {
            file,
            ranked,
            sync,
            enabled,
        }
    }

    /// The key a project id names, searched in the lists and in the sync links (any directory of
    /// a synced project resolves, so an id from before the list was re-ranked still works).
    fn resolve(&self, id: &str) -> Option<String> {
        let ranked = self.ranked.iter().map(|project| project.key.as_str());
        let stored = self
            .file
            .projects
            .keys()
            .chain(self.file.history.keys())
            .map(String::as_str);
        let synced = self
            .sync
            .iter()
            .flat_map(|project| project.keys.iter().chain(project.cloud_key.iter()))
            .map(String::as_str);
        ranked
            .chain(stored)
            .chain(synced)
            .find(|key| project_id(key) == id)
            .map(str::to_string)
    }
}

fn project_name(key: &str) -> String {
    match cloudkeys::identity_of(key) {
        Some(identity) => todos_view::basename(identity),
        None => crate::project::display_name(key),
    }
}

fn sync_info(status: &service::Status) -> wire::TodoSyncInfo {
    wire::TodoSyncInfo {
        syncing: status.syncing,
        paused: status.paused.clone(),
        helper: status.helper.state.clone(),
        unsynced: status.unsynced as u64,
        last_pull_ms: status.last_pull_ok_ms,
        last_push_ms: status.last_push_ok_ms,
        devices: status
            .devices
            .iter()
            .map(|device| wire::TodoSyncDevice {
                name: device.name.clone(),
                last_sync_ms: device.last_sync_ms,
                this_mac: device.this_mac,
            })
            .collect(),
    }
}

fn projects_of(env: &TodoEnv, world: &World) -> wire::TodoProjects {
    let history_of = |key: &str| world.file.history.get(key).map_or(0, Vec::len);
    let mut candidates: Vec<Candidate> = world
        .ranked
        .iter()
        .map(|project| Candidate {
            key: project.key.clone(),
            name: project_name(&project.key),
            pending: project.count,
            history: history_of(&project.key),
            activity: project.activity_rank,
        })
        .collect();
    // A synced project that finished everything is not in the ranking, but its row still says
    // how many finished todos it holds.
    let ranked: HashSet<&str> = world.ranked.iter().map(|p| p.key.as_str()).collect();
    for project in &world.sync {
        for key in project.keys.iter().chain(project.cloud_key.iter()) {
            if !ranked.contains(key.as_str()) && history_of(key) > 0 {
                candidates.push(Candidate {
                    key: key.clone(),
                    name: project_name(key),
                    pending: 0,
                    history: history_of(key),
                    activity: 2,
                });
            }
        }
    }
    let synced: Vec<SyncedProject> = world
        .sync
        .iter()
        .map(|project| SyncedProject {
            identity: project.identity.clone(),
            keys: project.keys.clone(),
            cloud_key: project.cloud_key.clone(),
            macs: project.macs.iter().map(|mac| mac.name.clone()).collect(),
        })
        .collect();
    let (projects, truncated) = todos_view::build_rows(&candidates, &synced);
    wire::TodoProjects {
        rev: rev_of(&env.path),
        mode: if world.enabled { "icloud" } else { "local" }.to_string(),
        sync: world.enabled.then(|| sync_info(&env.sync.status())),
        projects,
        truncated,
    }
}

/// The todos of one project as the phone sees them (docs/specs/ios-todos.md §6.3).
fn list_of(env: &TodoEnv, key: &str, id: &str) -> wire::TodoList {
    // The local phase first: a project that only just appeared has its todos in place.
    env.sync.before_read(key);
    let file = env.file();
    let details = if env.sync.enabled() {
        env.sync.details(key)
    } else {
        ProjectDetails::default()
    };
    let pending_all = file
        .projects
        .get(key)
        .map(Vec::as_slice)
        .unwrap_or_default();
    let history_all = file.history.get(key).map(Vec::as_slice).unwrap_or_default();
    let pending = pending_all
        .iter()
        .take(TODO_MAX_PENDING)
        .map(|entry| wire::TodoItem {
            id: entry.id.clone(),
            text: entry.text.clone(),
            created_at_ms: entry.created_at_ms,
            agent: entry.agent_kind.clone(),
            auto: entry.auto,
            att: entry.attachments.len() as u64,
            by: details
                .created_on
                .get(&entry.id)
                .filter(|device| !device.is_empty())
                .cloned(),
            att_on: details
                .attachments
                .get(&entry.id)
                .map(|hint| wire::TodoAttHint {
                    n: u64::from(hint.count),
                    dev: hint.device.clone(),
                }),
        })
        .collect();
    // The file keeps the history oldest first; the phone gets the newest first.
    let history = history_all
        .iter()
        .rev()
        .take(TODO_MAX_HISTORY)
        .map(|entry| wire::TodoDone {
            id: entry.id.clone(),
            text: entry.text.clone(),
            created_at_ms: entry.created_at_ms,
            done_at_ms: entry.done_at_ms,
            agent: entry.agent_kind.clone(),
            att: entry.attachments.len() as u64,
            done_on: details
                .done_on
                .get(&entry.id)
                .filter(|origin| !origin.this_mac && !origin.device.is_empty())
                .map(|origin| origin.device.clone()),
        })
        .collect();
    wire::TodoList {
        id: id.to_string(),
        found: true,
        rev: rev_of(&env.path),
        name: project_name(key),
        can_auto: !cloudkeys::is_virtual(key),
        pending,
        pending_total: pending_all.len() as u64,
        history,
        history_total: history_all.len() as u64,
    }
}

fn list_by_id(env: &TodoEnv, world: &World, id: &str) -> wire::TodoList {
    match world.resolve(id) {
        Some(key) => list_of(env, &key, id),
        None => wire::TodoList::missing(id, rev_of(&env.path)),
    }
}

/// Applies one validated write (docs/specs/ios-todos.md §6.4) and reads the project back.
fn apply_op(env: &TodoEnv, world: &World, op: &wire::TodoOp) -> TodoApplied {
    let Some(key) = world.resolve(&op.id) else {
        return TodoApplied::refused("notFound", "unknownProject", None);
    };
    let (path, lock) = (env.path.as_path(), env.lock.as_path());
    let todo_id = op.todo_id.as_deref().unwrap_or_default();
    let text = op.text.as_deref().unwrap_or_default();
    let cloud_only = cloudkeys::is_virtual(&key);
    let mut created = None;
    let outcome: Result<(), (&'static str, &'static str)> = match op.op.as_str() {
        "add" => {
            // The ⚡ flag means nothing for a project without a directory on this Mac.
            let auto = op.auto == Some(true) && !cloud_only;
            let added = if auto {
                todos::add_auto_at(path, lock, &key, text)
            } else {
                todos::add_at(path, lock, &key, text)
            };
            match added {
                Ok(entry) => {
                    created = Some(entry.id);
                    Ok(())
                }
                Err(AddError::EmptyInput) => Err(("rejected", "empty")),
                Err(_) => Err(("failed", "persist")),
            }
        }
        "edit" => match todos::set_text_at(path, lock, &key, todo_id, text) {
            Some(_) => Ok(()),
            None => Err(("notFound", "notFound")),
        },
        "complete" => {
            let ids = [todo_id.to_string()];
            if todos::take_at(path, lock, &key, &ids, env.history_limit).is_empty() {
                Err(("notFound", "notFound"))
            } else {
                Ok(())
            }
        }
        "restore" => {
            if todos::restore_at(path, lock, &key, todo_id) {
                Ok(())
            } else {
                Err(("notFound", "notFound"))
            }
        }
        "remove" => match todos::remove_checked_at(path, lock, &key, todo_id) {
            Ok(true) => Ok(()),
            Ok(false) => Err(("notFound", "notFound")),
            Err(_) => Err(("failed", "persist")),
        },
        "reorder" => {
            let order = op.order.as_deref().unwrap_or_default();
            // `false` means the order did not change, which is fine.
            todos::reorder_at(path, lock, &key, order);
            Ok(())
        }
        "auto" if cloud_only => Err(("rejected", "cloudOnly")),
        "auto" => match todos::set_auto_at(path, lock, &key, todo_id, op.auto == Some(true)) {
            Some(_) => Ok(()),
            None => Err(("notFound", "notFound")),
        },
        _ => Err(("rejected", "unsupported")),
    };
    // Also after a refusal: the phone catches up with where things stand.
    let list = Some(list_of(env, &key, &op.id));
    match outcome {
        Ok(()) => TodoApplied::ok(created, list),
        Err((status, reason)) => TodoApplied::refused(status, reason, list),
    }
}

// ---------------------------------------------------------------- daemon entry points

async fn blocking<T: Send + 'static>(work: impl FnOnce() -> T + Send + 'static, fallback: T) -> T {
    tokio::task::spawn_blocking(work).await.unwrap_or(fallback)
}

pub(super) async fn projects(state: &Arc<ServerState>) -> wire::TodoProjects {
    let agents = state.agents.snapshot();
    blocking(
        move || {
            let env = TodoEnv::production();
            let world = World::gather(&env, crate::agents::workspaces::list(), Some(&agents));
            projects_of(&env, &world)
        },
        wire::TodoProjects::empty(),
    )
    .await
}

pub(super) async fn list(state: &Arc<ServerState>, id: &str) -> wire::TodoList {
    let agents = state.agents.snapshot();
    let id = id.to_string();
    let fallback = wire::TodoList::missing(&id, 0);
    blocking(
        move || {
            let env = TodoEnv::production();
            let world = World::gather(&env, crate::agents::workspaces::list(), Some(&agents));
            list_by_id(&env, &world, &id)
        },
        fallback,
    )
    .await
}

pub(super) async fn apply(state: &Arc<ServerState>, op: &wire::TodoOp) -> TodoApplied {
    let agents = state.agents.snapshot();
    let op = op.clone();
    blocking(
        move || {
            let env = TodoEnv::production();
            let world = World::gather(&env, crate::agents::workspaces::list(), Some(&agents));
            apply_op(&env, &world, &op)
        },
        TodoApplied::refused("failed", "persist", None),
    )
    .await
}

/// The signature the gateway polls while a phone watches.
pub(super) fn rev() -> u64 {
    rev_of(&todos::todos_file())
}

static REFRESHING: AtomicBool = AtomicBool::new(false);

/// A phone looks at the todos: pull what other devices changed when the last pull is old (the
/// window does the same when it opens). One at a time, in the background.
pub(super) fn refresh() {
    if !service::is_enabled() || REFRESHING.swap(true, Ordering::SeqCst) {
        return;
    }
    tokio::task::spawn_blocking(|| {
        struct Done;
        impl Drop for Done {
            fn drop(&mut self) {
                REFRESHING.store(false, Ordering::SeqCst);
            }
        }
        let _done = Done;
        service::refresh_if_stale();
    });
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    use super::*;
    use crate::todo_sync::service::{DeviceRow, HelperStatus, Origin, SyncProject};

    #[derive(Default, Clone)]
    struct FakeSync {
        enabled: bool,
        projects: Vec<SyncProject>,
        details: HashMap<String, ProjectDetails>,
        reads: Arc<Mutex<Vec<String>>>,
    }

    impl SyncView for FakeSync {
        fn enabled(&self) -> bool {
            self.enabled
        }

        fn status(&self) -> service::Status {
            service::Status {
                supported: true,
                enabled: self.enabled,
                syncing: true,
                device_name: "Studio".into(),
                paused: Some("quota".into()),
                last_pull_ok_ms: 11,
                last_push_ok_ms: 22,
                unsynced: 3,
                todos: 0,
                history: 0,
                devices: vec![
                    DeviceRow {
                        name: "Studio".into(),
                        last_sync_ms: 5,
                        this_mac: true,
                    },
                    DeviceRow {
                        name: "MacBook".into(),
                        last_sync_ms: 4,
                        this_mac: false,
                    },
                ],
                helper: HelperStatus {
                    state: "running".into(),
                    version: None,
                    environment: None,
                    problem: None,
                    installed: true,
                },
                last_tick: None,
                newer_state: false,
            }
        }

        fn projects(&self) -> Vec<SyncProject> {
            self.projects.clone()
        }

        fn before_read(&self, key: &str) {
            self.reads.lock().unwrap().push(key.to_string());
        }

        fn details(&self, key: &str) -> ProjectDetails {
            self.details.get(key).cloned().unwrap_or_default()
        }
    }

    struct Setup {
        _dir: tempfile::TempDir,
        env: TodoEnv,
        reads: Arc<Mutex<Vec<String>>>,
    }

    fn setup(sync: FakeSync) -> Setup {
        let dir = tempfile::tempdir().unwrap();
        let reads = sync.reads.clone();
        let env = TodoEnv {
            path: dir.path().join("todos.json"),
            lock: dir.path().join("todos.lock"),
            history_limit: 100,
            sync: Box::new(sync),
        };
        Setup {
            _dir: dir,
            env,
            reads,
        }
    }

    impl Setup {
        fn add(&self, key: &str, text: &str) -> String {
            todos::add_at(&self.env.path, &self.env.lock, key, text)
                .unwrap()
                .id
        }

        fn finish(&self, key: &str, id: &str) {
            let taken = todos::take_at(
                &self.env.path,
                &self.env.lock,
                key,
                &[id.to_string()],
                self.env.history_limit,
            );
            assert_eq!(taken.len(), 1);
        }

        fn world(&self) -> World {
            World::gather(&self.env, Vec::new(), None)
        }

        fn world_with(&self, workspaces: Vec<Workspace>) -> World {
            World::gather(&self.env, workspaces, None)
        }

        fn projects(&self) -> wire::TodoProjects {
            projects_of(&self.env, &self.world())
        }

        fn list(&self, key: &str) -> wire::TodoList {
            list_by_id(&self.env, &self.world(), &project_id(key))
        }

        fn apply(&self, key: &str, op: wire::TodoOp) -> TodoApplied {
            apply_op(
                &self.env,
                &self.world(),
                &wire::TodoOp {
                    id: project_id(key),
                    ..op
                },
            )
        }
    }

    fn op(kind: &str) -> wire::TodoOp {
        wire::TodoOp {
            client_msg_id: "m".into(),
            id: String::new(),
            op: kind.into(),
            todo_id: None,
            text: None,
            auto: None,
            order: None,
        }
    }

    fn workspace(path: &str) -> Workspace {
        Workspace {
            path: path.into(),
            label: path.into(),
            last_used_at: 1,
            agents: Vec::new(),
            pinned: false,
            hidden: false,
        }
    }

    fn synced(identity: &str, keys: &[&str], cloud: Option<&str>) -> SyncProject {
        SyncProject {
            identity: identity.into(),
            keys: keys.iter().map(|k| k.to_string()).collect(),
            cloud_key: cloud.map(str::to_string),
            macs: if keys.is_empty() {
                Vec::new()
            } else {
                vec![DeviceRow {
                    name: "Studio".into(),
                    last_sync_ms: 5,
                    this_mac: true,
                }]
            },
        }
    }

    const CLOUD: &str = "icloud:github.com/acme/dotfiles";

    // ------------------------------------------------------------ project list

    #[test]
    fn local_mode_lists_projects_with_todos_then_recent_workspaces() {
        let s = setup(FakeSync::default());
        s.add("/w/app", "one");
        s.add("/w/app", "two");
        s.add("/w/blog", "three");
        let world = s.world_with(vec![workspace("/w/idle")]);
        let list = projects_of(&s.env, &world);
        assert_eq!(list.mode, "local");
        assert!(list.sync.is_none());
        assert!(!list.truncated);
        let summary: Vec<_> = list
            .projects
            .iter()
            .map(|p| {
                (
                    p.name.as_str(),
                    p.pending,
                    p.section.as_str(),
                    p.scope.as_str(),
                )
            })
            .collect();
        assert_eq!(
            summary,
            [
                ("app", 2, "withTodos", "local"),
                ("blog", 1, "withTodos", "local"),
                ("idle", 0, "recent", "local"),
            ]
        );
        assert_ne!(list.rev, 0, "the store has a signature once it exists");
    }

    #[test]
    fn a_project_with_only_history_and_no_workspace_is_not_listed_like_the_window() {
        let s = setup(FakeSync::default());
        let id = s.add("/w/app", "one");
        s.finish("/w/app", &id);
        assert!(s.projects().projects.is_empty());
        // …but a workspace brings it back with its history count.
        let world = s.world_with(vec![workspace("/w/app")]);
        let list = projects_of(&s.env, &world);
        assert_eq!(list.projects.len(), 1);
        assert_eq!((list.projects[0].pending, list.projects[0].history), (0, 1));
    }

    #[test]
    fn an_empty_store_has_no_projects_and_signature_zero() {
        let s = setup(FakeSync::default());
        let list = s.projects();
        assert!(list.projects.is_empty());
        assert_eq!(list.rev, 0);
    }

    #[test]
    fn icloud_mode_reports_the_sync_facts_and_merges_directories_of_a_repository() {
        let sync = FakeSync {
            enabled: true,
            projects: vec![
                synced("github.com/acme/app", &["/w/app", "/w/app-wt"], None),
                synced("github.com/acme/dotfiles", &[], Some(CLOUD)),
            ],
            ..FakeSync::default()
        };
        let s = setup(sync);
        s.add("/w/app", "one");
        s.add("/w/app-wt", "one");
        s.add("/w/notes", "local only");
        s.add(CLOUD, "from the cloud");
        let list = s.projects();
        assert_eq!(list.mode, "icloud");
        let info = list.sync.expect("sync facts");
        assert!(info.syncing);
        assert_eq!(info.paused.as_deref(), Some("quota"));
        assert_eq!(info.helper, "running");
        assert_eq!(
            (info.unsynced, info.last_pull_ms, info.last_push_ms),
            (3, 11, 22)
        );
        assert_eq!(
            info.devices
                .iter()
                .map(|d| (d.name.as_str(), d.this_mac))
                .collect::<Vec<_>>(),
            [("Studio", true), ("MacBook", false)]
        );
        let rows: Vec<_> = list
            .projects
            .iter()
            .map(|p| (p.name.as_str(), p.scope.as_str(), p.section.as_str()))
            .collect();
        assert_eq!(rows.len(), 3, "{rows:?}");
        assert!(rows.contains(&("app", "synced", "synced")));
        assert!(rows.contains(&("dotfiles", "cloudOnly", "synced")));
        assert!(rows.contains(&("notes", "localOnly", "localOnly")));
        let app = list.projects.iter().find(|p| p.name == "app").unwrap();
        assert_eq!(app.repo.as_deref(), Some("github.com/acme/app"));
        assert_eq!(app.macs, ["Studio"]);
    }

    #[test]
    fn a_finished_cloud_project_still_has_a_row_with_its_history_count() {
        let sync = FakeSync {
            enabled: true,
            projects: vec![synced("github.com/acme/dotfiles", &[], Some(CLOUD))],
            ..FakeSync::default()
        };
        let s = setup(sync);
        let id = s.add(CLOUD, "one");
        s.finish(CLOUD, &id);
        let list = s.projects();
        assert_eq!(list.projects.len(), 1);
        let row = &list.projects[0];
        assert_eq!(
            (row.section.as_str(), row.pending, row.history),
            ("empty", 0, 1)
        );
        assert_eq!(row.id, project_id(CLOUD));
    }

    #[test]
    fn sync_enabled_with_nothing_synced_falls_back_to_the_local_rows() {
        let s = setup(FakeSync {
            enabled: true,
            ..FakeSync::default()
        });
        s.add("/w/app", "one");
        let list = s.projects();
        assert_eq!(list.mode, "icloud");
        assert_eq!(list.projects[0].scope, "local");
    }

    // ------------------------------------------------------------ reading a project

    #[test]
    fn a_list_has_the_queue_in_order_and_the_history_newest_first() {
        let s = setup(FakeSync::default());
        let a = s.add("/w/app", "first");
        s.add("/w/app", "second");
        let c = s.add("/w/app", "third");
        s.finish("/w/app", &a);
        s.finish("/w/app", &c);
        let list = s.list("/w/app");
        assert!(list.found);
        assert_eq!(list.name, "app");
        assert!(list.can_auto);
        assert_eq!(
            list.pending
                .iter()
                .map(|t| t.text.as_str())
                .collect::<Vec<_>>(),
            ["second"]
        );
        assert_eq!(list.pending_total, 1);
        assert_eq!(
            list.history
                .iter()
                .map(|t| t.text.as_str())
                .collect::<Vec<_>>(),
            ["third", "first"]
        );
        assert_eq!(list.history_total, 2);
        assert_ne!(list.rev, 0);
    }

    #[test]
    fn reading_runs_the_local_phase_for_the_project_first() {
        let s = setup(FakeSync::default());
        s.add("/w/app", "one");
        s.list("/w/app");
        assert_eq!(*s.reads.lock().unwrap(), ["/w/app"]);
    }

    #[test]
    fn an_unknown_id_is_not_found() {
        let s = setup(FakeSync::default());
        s.add("/w/app", "one");
        let list = s.list("/w/other");
        assert!(!list.found);
        assert!(list.pending.is_empty());
        assert!(s.reads.lock().unwrap().is_empty(), "nothing to materialize");
    }

    #[test]
    fn any_directory_of_a_synced_project_resolves() {
        let s = setup(FakeSync {
            enabled: true,
            projects: vec![synced(
                "github.com/acme/app",
                &["/w/app", "/w/app-wt"],
                None,
            )],
            ..FakeSync::default()
        });
        s.add("/w/app", "one");
        assert!(s.list("/w/app-wt").found, "a replica without todos yet");
        assert!(s.list("/w/app").found);
    }

    #[test]
    fn origins_come_from_the_sync_details() {
        let mut details = ProjectDetails::default();
        let mut sync = FakeSync {
            enabled: true,
            projects: vec![synced("github.com/acme/app", &["/w/app"], None)],
            ..FakeSync::default()
        };
        let s0 = setup(FakeSync::default());
        let a = s0.add("/w/app", "made elsewhere");
        let b = s0.add("/w/app", "files elsewhere");
        let c = s0.add("/w/app", "done elsewhere");
        s0.finish("/w/app", &c);
        details.created_on.insert(a.clone(), "MacBook".into());
        details.attachments.insert(
            b.clone(),
            service::AttachmentHint {
                count: 2,
                device: "MacBook".into(),
            },
        );
        details.done_on.insert(
            c.clone(),
            Origin {
                device: "MacBook".into(),
                this_mac: false,
            },
        );
        sync.details.insert("/w/app".into(), details);
        // Same store, now read through a sync view that knows the origins.
        let env = TodoEnv {
            path: s0.env.path.clone(),
            lock: s0.env.lock.clone(),
            history_limit: 100,
            sync: Box::new(sync),
        };
        let world = World::gather(&env, Vec::new(), None);
        let list = list_by_id(&env, &world, &project_id("/w/app"));
        assert_eq!(list.pending[0].by.as_deref(), Some("MacBook"));
        assert_eq!(
            list.pending[1]
                .att_on
                .as_ref()
                .map(|h| (h.n, h.dev.as_str())),
            Some((2, "MacBook"))
        );
        assert_eq!(list.history[0].done_on.as_deref(), Some("MacBook"));
    }

    #[test]
    fn the_cloud_only_project_cannot_auto_run() {
        let s = setup(FakeSync {
            enabled: true,
            projects: vec![synced("github.com/acme/dotfiles", &[], Some(CLOUD))],
            ..FakeSync::default()
        });
        s.add(CLOUD, "one");
        let list = s.list(CLOUD);
        assert_eq!(list.name, "dotfiles");
        assert!(!list.can_auto);
    }

    #[test]
    fn lists_are_capped_and_keep_their_totals() {
        let s = setup(FakeSync::default());
        for i in 0..TODO_MAX_PENDING + 5 {
            s.add("/w/app", &format!("todo {i}"));
        }
        let list = s.list("/w/app");
        assert_eq!(list.pending.len(), TODO_MAX_PENDING);
        assert_eq!(list.pending_total, (TODO_MAX_PENDING + 5) as u64);
        assert_eq!(
            list.pending[0].text, "todo 0",
            "the head of the queue is kept"
        );

        let finished: Vec<String> = todos::list_at(&s.env.path, "/w/app")
            .iter()
            .take(TODO_MAX_HISTORY + 7)
            .map(|entry| entry.id.clone())
            .collect();
        todos::take_at(&s.env.path, &s.env.lock, "/w/app", &finished, 1000);
        let list = s.list("/w/app");
        assert_eq!(
            list.pending.len(),
            TODO_MAX_PENDING + 5 - (TODO_MAX_HISTORY + 7)
        );
        assert_eq!(list.history.len(), TODO_MAX_HISTORY);
        assert_eq!(list.history_total, (TODO_MAX_HISTORY + 7) as u64);
    }

    // ------------------------------------------------------------ writing

    #[test]
    fn add_appends_to_the_queue_and_returns_the_new_id() {
        let s = setup(FakeSync::default());
        s.add("/w/app", "one");
        let result = s.apply(
            "/w/app",
            wire::TodoOp {
                text: Some("two".into()),
                ..op("add")
            },
        );
        assert_eq!(result.status, "ok");
        let id = result.todo_id.expect("the new todo's id");
        let list = result.list.expect("the project afterwards");
        assert_eq!(
            list.pending
                .iter()
                .map(|t| t.text.as_str())
                .collect::<Vec<_>>(),
            ["one", "two"]
        );
        assert_eq!(list.pending[1].id, id);
        assert!(!list.pending[1].auto);
    }

    #[test]
    fn add_can_flag_a_todo_for_auto_run_except_in_a_cloud_only_project() {
        let s = setup(FakeSync {
            enabled: true,
            projects: vec![synced("github.com/acme/dotfiles", &[], Some(CLOUD))],
            ..FakeSync::default()
        });
        s.add("/w/app", "x");
        let flagged = s.apply(
            "/w/app",
            wire::TodoOp {
                text: Some("go".into()),
                auto: Some(true),
                ..op("add")
            },
        );
        assert!(flagged.list.unwrap().pending[1].auto);
        s.add(CLOUD, "x");
        let ignored = s.apply(
            CLOUD,
            wire::TodoOp {
                text: Some("go".into()),
                auto: Some(true),
                ..op("add")
            },
        );
        assert_eq!(ignored.status, "ok");
        assert!(
            !ignored.list.unwrap().pending[1].auto,
            "the flag means nothing here"
        );
    }

    #[test]
    fn add_to_a_project_that_has_no_todos_yet_works_when_a_workspace_lists_it() {
        let s = setup(FakeSync::default());
        let world = s.world_with(vec![workspace("/w/fresh")]);
        let result = apply_op(
            &s.env,
            &world,
            &wire::TodoOp {
                id: project_id("/w/fresh"),
                text: Some("first".into()),
                ..op("add")
            },
        );
        assert_eq!(result.status, "ok");
        assert_eq!(result.list.unwrap().pending.len(), 1);
    }

    #[test]
    fn edit_changes_the_text_and_keeps_the_rest() {
        let s = setup(FakeSync::default());
        let id = s.add("/w/app", "old");
        todos::set_auto_at(&s.env.path, &s.env.lock, "/w/app", &id, true).unwrap();
        let result = s.apply(
            "/w/app",
            wire::TodoOp {
                todo_id: Some(id.clone()),
                text: Some("new".into()),
                ..op("edit")
            },
        );
        assert_eq!(result.status, "ok");
        let item = &result.list.unwrap().pending[0];
        assert_eq!(
            (item.id.as_str(), item.text.as_str(), item.auto),
            (id.as_str(), "new", true)
        );
    }

    #[test]
    fn complete_moves_the_todo_into_the_history_and_restore_brings_it_back_to_the_end() {
        let s = setup(FakeSync::default());
        let a = s.add("/w/app", "a");
        s.add("/w/app", "b");
        let done = s.apply(
            "/w/app",
            wire::TodoOp {
                todo_id: Some(a.clone()),
                ..op("complete")
            },
        );
        assert_eq!(done.status, "ok");
        let list = done.list.unwrap();
        assert_eq!(list.pending.len(), 1);
        assert_eq!(list.history[0].id, a);
        let back = s.apply(
            "/w/app",
            wire::TodoOp {
                todo_id: Some(a.clone()),
                ..op("restore")
            },
        );
        assert_eq!(back.status, "ok");
        let list = back.list.unwrap();
        assert_eq!(
            list.pending
                .iter()
                .map(|t| t.text.as_str())
                .collect::<Vec<_>>(),
            ["b", "a"]
        );
        assert!(list.history.is_empty());
    }

    #[test]
    fn complete_honors_the_history_limit_setting() {
        let mut s = setup(FakeSync::default());
        s.env.history_limit = 0;
        let a = s.add("/w/app", "a");
        let done = s.apply(
            "/w/app",
            wire::TodoOp {
                todo_id: Some(a),
                ..op("complete")
            },
        );
        assert_eq!(done.status, "ok");
        let list = done.list.unwrap();
        assert!(list.pending.is_empty());
        assert!(
            list.history.is_empty(),
            "limit 0 records nothing, like the window"
        );
    }

    #[test]
    fn remove_drops_the_todo_without_a_history_entry() {
        let s = setup(FakeSync::default());
        let a = s.add("/w/app", "a");
        let result = s.apply(
            "/w/app",
            wire::TodoOp {
                todo_id: Some(a),
                ..op("remove")
            },
        );
        assert_eq!(result.status, "ok");
        let list = result.list.unwrap();
        assert!(list.pending.is_empty() && list.history.is_empty());
    }

    #[test]
    fn reorder_follows_the_given_order_and_keeps_the_ones_it_forgot() {
        let s = setup(FakeSync::default());
        let a = s.add("/w/app", "a");
        let b = s.add("/w/app", "b");
        let c = s.add("/w/app", "c");
        let result = s.apply(
            "/w/app",
            wire::TodoOp {
                order: Some(vec![c.clone(), "ghost".into(), a.clone()]),
                ..op("reorder")
            },
        );
        assert_eq!(result.status, "ok");
        let ids: Vec<_> = result
            .list
            .unwrap()
            .pending
            .iter()
            .map(|t| t.id.clone())
            .collect();
        assert_eq!(ids, [c, a, b]);
    }

    #[test]
    fn auto_sets_the_flag_and_is_refused_for_a_cloud_only_project() {
        let s = setup(FakeSync {
            enabled: true,
            projects: vec![synced("github.com/acme/dotfiles", &[], Some(CLOUD))],
            ..FakeSync::default()
        });
        let a = s.add("/w/app", "a");
        let on = s.apply(
            "/w/app",
            wire::TodoOp {
                todo_id: Some(a.clone()),
                auto: Some(true),
                ..op("auto")
            },
        );
        assert!(on.list.unwrap().pending[0].auto);
        let off = s.apply(
            "/w/app",
            wire::TodoOp {
                todo_id: Some(a),
                auto: Some(false),
                ..op("auto")
            },
        );
        assert!(!off.list.unwrap().pending[0].auto);

        let b = s.add(CLOUD, "b");
        let refused = s.apply(
            CLOUD,
            wire::TodoOp {
                todo_id: Some(b),
                auto: Some(true),
                ..op("auto")
            },
        );
        assert_eq!(
            (refused.status, refused.reason.as_deref()),
            ("rejected", Some("cloudOnly"))
        );
        assert!(!refused.list.unwrap().pending[0].auto);
    }

    #[test]
    fn a_write_to_something_that_is_gone_says_so_and_shows_where_things_stand() {
        let s = setup(FakeSync::default());
        s.add("/w/app", "a");
        for kind in ["edit", "complete", "restore", "remove", "auto"] {
            let result = s.apply(
                "/w/app",
                wire::TodoOp {
                    todo_id: Some("ghost".into()),
                    text: Some("x".into()),
                    auto: Some(true),
                    ..op(kind)
                },
            );
            assert_eq!(
                (result.status, result.reason.as_deref()),
                ("notFound", Some("notFound")),
                "{kind}"
            );
            assert_eq!(result.list.expect("the list").pending.len(), 1, "{kind}");
        }
        let unknown = apply_op(
            &s.env,
            &s.world(),
            &wire::TodoOp {
                id: "feedfacefeedface".into(),
                text: Some("x".into()),
                ..op("add")
            },
        );
        assert_eq!(
            (unknown.status, unknown.reason.as_deref()),
            ("notFound", Some("unknownProject"))
        );
        assert!(unknown.list.is_none());
    }

    #[test]
    fn an_unknown_operation_is_refused() {
        let s = setup(FakeSync::default());
        s.add("/w/app", "a");
        let result = s.apply("/w/app", op("explode"));
        assert_eq!(
            (result.status, result.reason.as_deref()),
            ("rejected", Some("unsupported"))
        );
    }

    // ------------------------------------------------------------ signature

    #[test]
    fn the_signature_moves_with_every_write_and_fits_52_bits() {
        let s = setup(FakeSync::default());
        let mut seen = HashSet::new();
        for i in 0..5 {
            s.add("/w/app", &format!("todo {i}"));
            let rev = rev_of(&s.env.path);
            assert!(rev < (1 << 52));
            assert!(seen.insert(rev), "write {i} did not change the signature");
        }
    }
}
