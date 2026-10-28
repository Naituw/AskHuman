//! New Agent tasks from the iOS app (docs/plans/ios-channel.md §4.7): what the phone may choose
//! from, and starting the Agent in a new terminal window through the same launcher IM `/new` uses.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Duration;

use sha2::{Digest, Sha256};

use super::ios_agents::agent_key;
use super::task_launch::{
    resolve_task_input, start_task, task_gate_now, StartError, Started, TaskBlock, TaskSpec,
};
use super::*;
use crate::agents::model_catalog::cache::{CatalogCache, CATALOG_TTL, FAILURE_RETRY};
use crate::agents::model_catalog::source::{self, CatalogSource};
use crate::agents::model_catalog::{resolve_choice, AgentCatalog, ModelChoice};
use crate::codex_desktop::LaunchStatus;
use crate::config::AgentTaskPermission;
use crate::integrations::agent_launch::{self, AgentReadiness, LaunchPermission, LaunchRecord};
use crate::ios::gateway::{TaskChoices, TaskLaunch, TodoChoices};
use crate::ios::wire::{
    AgentOption, DefaultModel, ModelOption, TaskOptions, TodoOption, WorkspaceOption,
};

/// Longest todo text sent to the phone; the full text stays on the Mac and is resolved by id.
const TODO_PREVIEW_CHARS: usize = 600;
/// Longest reason text passed through to the phone.
const REASON_MAX_CHARS: usize = 160;
/// How long `task.options` waits for an Agent's model list the first time it is asked for. A
/// list that is later than this fills the cache for the next ask; the phone shows "Default" only
/// until then.
const OPTIONS_CATALOG_WAIT: Duration = Duration::from_secs(4);
/// How long `task.create` waits for a fresh model list when the cached one does not contain what
/// the phone picked.
const CREATE_CATALOG_WAIT: Duration = Duration::from_secs(6);
/// The most bytes of model lists in one `task.options`, all Agents together. Real lists are far
/// smaller; this keeps a pathological one from pushing the message past the wire's size limit.
const MODELS_BYTES_BUDGET: usize = 240 * 1024;

fn block_reason(block: TaskBlock) -> &'static str {
    match block {
        TaskBlock::Disabled => "disabled",
        TaskBlock::NotReady => "notReady",
        TaskBlock::NoTerminal => "noTerminal",
    }
}

fn permission_mode(config: &AppConfig) -> &'static str {
    match config.agent_tasks.permission_prompt {
        AgentTaskPermission::Ask => "ask",
        AgentTaskPermission::AgentDefault => "agentDefault",
        AgentTaskPermission::Yolo => "yolo",
    }
}

/// Stable id of a workspace on the wire: the phone never sees (or sends) a path.
fn workspace_id(path: &str) -> String {
    Sha256::digest(path.as_bytes())
        .iter()
        .take(8)
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn clip(text: &str, max: usize) -> String {
    let text = text.trim();
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut out: String = text.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

/// `~/code/app` for a workspace inside the home directory.
fn parent_label(path: &str) -> String {
    let parent = Path::new(path)
        .parent()
        .map(|parent| parent.to_string_lossy().to_string())
        .unwrap_or_default();
    let home = crate::paths::home().to_string_lossy().to_string();
    match parent.strip_prefix(&home) {
        Some(rest) => format!("~{rest}"),
        None => parent,
    }
}

/// What the probes of the installed Agents found.
struct AgentProbes {
    readiness: Vec<AgentReadiness>,
    codex: LaunchStatus,
    /// The Codex executable and data directory a task started now would run on.
    codex_runtime: Option<(PathBuf, PathBuf)>,
}

fn probe_agents() -> AgentProbes {
    let readiness = agent_launch::all_readiness();
    let codex = crate::codex_desktop::launch_status(false);
    let terminal_codex = readiness
        .iter()
        .find(|item| item.kind == AgentKind::Codex)
        .and_then(|item| item.executable.clone());
    let codex_runtime = codex.target.and_then(|target| {
        crate::codex_desktop::catalog_runtime(target, terminal_codex.as_deref())
    });
    AgentProbes {
        readiness,
        codex,
        codex_runtime,
    }
}

/// Whether a task can be started on this Agent right now. Codex is judged by its launch route
/// (Desktop App or terminal); the others by their own readiness.
fn is_ready(item: &AgentReadiness, codex: &LaunchStatus) -> bool {
    if item.kind == AgentKind::Codex {
        codex.target.is_some()
    } else {
        item.ready
    }
}

/// Where the model list of `kind` is read from: the executable the launch would use. Codex is
/// asked through the runtime of its launch route.
fn catalog_source(
    kind: AgentKind,
    executable: Option<&str>,
    codex_runtime: Option<&(PathBuf, PathBuf)>,
) -> Option<CatalogSource> {
    if kind == AgentKind::Codex {
        let (executable, home) = codex_runtime?;
        return Some(CatalogSource {
            kind,
            executable: executable.to_string_lossy().into_owned(),
            codex_home: Some(home.clone()),
        });
    }
    Some(CatalogSource {
        kind,
        executable: executable?.to_string(),
        codex_home: None,
    })
}

/// The model lists to ask for: one per Agent that can be started.
fn catalog_sources(probes: &AgentProbes) -> Vec<CatalogSource> {
    probes
        .readiness
        .iter()
        .filter(|item| is_ready(item, &probes.codex))
        .filter_map(|item| {
            catalog_source(
                item.kind,
                item.executable.as_deref(),
                probes.codex_runtime.as_ref(),
            )
        })
        .collect()
}

fn catalog_cache() -> &'static Arc<CatalogCache> {
    static CACHE: OnceLock<Arc<CatalogCache>> = OnceLock::new();
    CACHE.get_or_init(|| CatalogCache::new(CATALOG_TTL, FAILURE_RETRY))
}

/// The cached model lists of `sources`, each Agent asked at the same time. An Agent whose list is
/// not there in time (or never will be) is missing from the result.
async fn fetch_catalogs(sources: Vec<CatalogSource>) -> HashMap<AgentKind, Arc<AgentCatalog>> {
    let cache = catalog_cache();
    let fetched = futures_util::future::join_all(sources.into_iter().map(|source| async move {
        let kind = source.kind;
        let key = source.cache_key();
        let loader = source.clone();
        let catalog = cache
            .get(&key, OPTIONS_CATALOG_WAIT, move || source::load(&loader))
            .await
            // The list may be half an hour old; what "Default" means is read from the
            // configuration as it is now.
            .map(|catalog| Arc::new(source::refresh_default(&source, &catalog)));
        (kind, catalog)
    }))
    .await;
    fetched
        .into_iter()
        .filter_map(|(kind, catalog)| Some((kind, catalog?)))
        .collect()
}

/// What the phone is shown of an Agent's catalog: the models (until `budget` bytes are used up)
/// and what "Default" stands for. Nothing known gives nothing, and the phone offers "Default" only.
fn model_options(
    catalog: Option<&AgentCatalog>,
    budget: &mut usize,
) -> (Option<Vec<ModelOption>>, Option<DefaultModel>) {
    let Some(catalog) = catalog else {
        return (None, None);
    };
    let mut models = Vec::new();
    for entry in &catalog.models {
        let option = ModelOption {
            id: entry.id.clone(),
            label: entry.label.clone(),
            note: entry.note.clone(),
            efforts: (!entry.efforts.is_empty()).then(|| entry.efforts.clone()),
            default_effort: entry.default_effort.clone(),
        };
        let size = serde_json::to_vec(&option).map_or(usize::MAX, |bytes| bytes.len());
        if size > *budget {
            break;
        }
        *budget -= size;
        models.push(option);
    }
    let default = &catalog.default;
    let default_model = (default.label.is_some() || !default.efforts.is_empty()).then(|| {
        DefaultModel {
            label: default.label.clone(),
            efforts: (!default.efforts.is_empty()).then(|| default.efforts.clone()),
            // A default effort that is not one of the offered ones would mark nothing.
            default_effort: default
                .default_effort
                .clone()
                .filter(|effort| default.efforts.contains(effort)),
        }
    });
    ((!models.is_empty()).then_some(models), default_model)
}

/// The Agents of `task.options`: readiness, why not, and the models of the ones that can start.
fn agent_options(
    probes: &AgentProbes,
    mode: &str,
    lang: Lang,
    catalogs: &HashMap<AgentKind, Arc<AgentCatalog>>,
) -> Vec<AgentOption> {
    let mut budget = MODELS_BYTES_BUDGET;
    probes
        .readiness
        .iter()
        .map(|item| {
            let is_codex = item.kind == AgentKind::Codex;
            let ready = is_ready(item, &probes.codex);
            let subtitle = if is_codex {
                probes.codex.description(lang)
            } else {
                item.integration_mode.clone()
            };
            let reason = (!ready).then(|| {
                let reason = if is_codex {
                    probes.codex.description(lang)
                } else {
                    item.diagnostics.first().cloned().unwrap_or_default()
                };
                clip(&reason, REASON_MAX_CHARS)
            });
            let (models, default_model) = if ready {
                let catalog = catalogs.get(&item.kind).map(|catalog| &**catalog);
                model_options(catalog, &mut budget)
            } else {
                (None, None)
            };
            AgentOption {
                kind: item.kind.as_str().to_string(),
                label: item.label.clone(),
                ready,
                subtitle,
                // Pi has no permission step; otherwise the Mac decides whether the user is asked.
                permission_selectable: item.kind != AgentKind::Pi && mode == "ask",
                reason: reason.filter(|reason| !reason.is_empty()),
                models,
                default_model,
            }
        })
        .collect()
}

/// What a phone may pick from. Mirrors the gates of IM `/new`, but reports them as codes.
pub(super) async fn choices(state: &Arc<ServerState>) -> TaskChoices {
    let config = state.config_snapshot();
    let mode = permission_mode(&config);
    if let Err(block) = task_gate_now(&config) {
        return TaskChoices::blocked(mode, block_reason(block));
    }
    // The workspace scan and the Agent probes (login-shell lookups that can take seconds on a slow
    // shell) are independent; run side by side so the phone's 15 s wait is spent once, not twice.
    let (workspaces, probes) = tokio::join!(
        tokio::task::spawn_blocking(crate::agents::workspaces::refresh),
        tokio::task::spawn_blocking(probe_agents),
    );
    let workspaces = workspaces.unwrap_or_default();
    let probes = probes.unwrap_or_else(|_| AgentProbes {
        readiness: Vec::new(),
        codex: crate::codex_desktop::launch_status(false),
        codex_runtime: None,
    });
    if workspaces.is_empty() {
        return TaskChoices::blocked(mode, "noWorkspace");
    }
    // Only Agents that can be started get a model list; the lists come from the cache and are
    // asked for side by side, so the wait is spent once whatever the number of Agents.
    let catalogs = fetch_catalogs(catalog_sources(&probes)).await;
    let agents = agent_options(&probes, mode, Lang::current(), &catalogs);
    if !agents.iter().any(|agent| agent.ready) {
        return TaskChoices::blocked(mode, "noAgent");
    }
    let mut ids = HashMap::new();
    let workspaces = workspaces
        .into_iter()
        .map(|workspace| {
            let id = workspace_id(&workspace.path);
            ids.insert(id.clone(), workspace.path.clone());
            WorkspaceOption {
                id,
                subtitle: parent_label(&workspace.path),
                label: workspace.label,
                pinned: workspace.pinned,
                last_used_at: workspace.last_used_at as i64 * 1000,
            }
        })
        .collect();
    TaskChoices {
        options: TaskOptions {
            enabled: true,
            blocked: None,
            permission_mode: mode.to_string(),
            workspaces,
            agents,
        },
        workspaces: ids,
    }
}

/// The TODOs of the project a workspace belongs to, as IM `/new` offers them.
pub(super) fn todos(workspace: &str) -> TodoChoices {
    let project = crate::project::detect_from(Path::new(workspace));
    crate::todo_sync::service::before_read(&project);
    let mut todos = crate::todos::list(&project);
    let overflow = todos.len().saturating_sub(crate::todos::MAX_OPTION_TODOS) as u64;
    todos.truncate(crate::todos::MAX_OPTION_TODOS);
    TodoChoices {
        todos: todos
            .into_iter()
            .map(|todo| TodoOption {
                id: todo.id,
                text: clip(&todo.text, TODO_PREVIEW_CHARS),
                auto: todo.auto,
                attachments: todo.attachments.len() as u64,
            })
            .collect(),
        overflow,
    }
}

/// The permission a launch will use, or `None` when the phone asked for something the Mac's
/// setting no longer allows (the phone's options are stale).
fn effective_permission(
    mode: AgentTaskPermission,
    kind: AgentKind,
    requested: &str,
) -> Option<LaunchPermission> {
    if kind == AgentKind::Pi {
        return Some(LaunchPermission::AgentDefault);
    }
    let requested = match requested {
        "default" => LaunchPermission::AgentDefault,
        "yolo" => LaunchPermission::Yolo,
        _ => return None,
    };
    let fixed = match mode {
        AgentTaskPermission::Ask => return Some(requested),
        AgentTaskPermission::AgentDefault => LaunchPermission::AgentDefault,
        AgentTaskPermission::Yolo => LaunchPermission::Yolo,
    };
    (fixed == requested).then_some(fixed)
}

/// Remembers a terminal launch so the Agent's first turn can be matched back to the phone.
fn register_pending(
    state: &Arc<ServerState>,
    record: &LaunchRecord,
    phone_id: &str,
    client_msg_id: &str,
) {
    state
        .pending_launches
        .lock()
        .unwrap()
        .push(PendingLaunchWatch {
            id: record.id.clone(),
            channel: None,
            ios: Some((phone_id.to_string(), client_msg_id.to_string())),
            kind: record.kind,
            cwd: record.cwd.clone(),
            task_sha256: record.task_sha256.clone(),
            source_session_id: None,
            created_at: now_secs(),
        });
}

/// Matches what the phone picked against the Agent's own model list and returns what the launch
/// should carry. A default pick never looks at a list. `Err` is `modelUnavailable` or
/// `effortUnavailable`: nothing is started with a model the Agent does not offer.
async fn resolve_pick(
    kind: AgentKind,
    source: Option<CatalogSource>,
    wanted: &ModelChoice,
) -> Result<ModelChoice, String> {
    if wanted.is_default() {
        return Ok(ModelChoice::default());
    }
    let Some(source) = source else {
        return Err(unavailable(wanted));
    };
    let key = source.cache_key();
    let current = source.clone();
    resolve_pick_with(
        catalog_cache(),
        kind,
        &key,
        CREATE_CATALOG_WAIT,
        move || source::load(&source),
        move |catalog| source::refresh_default(&current, catalog),
        wanted,
    )
    .await
}

/// The reason for a pick that cannot be checked against any list.
fn unavailable(wanted: &ModelChoice) -> String {
    let reason = if wanted.model.is_some() {
        crate::agents::model_catalog::ChoiceError::ModelUnavailable
    } else {
        crate::agents::model_catalog::ChoiceError::EffortUnavailable
    };
    reason.reason().to_string()
}

/// [`resolve_pick`] against `cache`, loading with `load`. A cached list that already holds the
/// pick is used as it is; one that does not is renewed once (waiting up to `wait`), and if that
/// fails or is late the older list stands. `current` brings a cached list up to date with the
/// Agent's configuration, and every check is made against that.
async fn resolve_pick_with(
    cache: &Arc<CatalogCache>,
    kind: AgentKind,
    key: &str,
    wait: Duration,
    load: impl FnOnce() -> Result<AgentCatalog, String> + Send + 'static,
    current: impl Fn(&AgentCatalog) -> AgentCatalog,
    wanted: &ModelChoice,
) -> Result<ModelChoice, String> {
    let catalog = cache
        .get_enough(key, wait, load, |catalog| {
            resolve_choice(kind, &current(catalog), wanted).is_ok()
        })
        .await
        .unwrap_or_default();
    resolve_choice(kind, &current(&catalog), wanted).map_err(|error| error.reason().to_string())
}

/// Starts a validated `task.create`. `Err` is a short reason for the phone.
pub(super) async fn create(state: &Arc<ServerState>, launch: TaskLaunch) -> Result<(), String> {
    let config = state.config_snapshot();
    task_gate_now(&config).map_err(|block| block_reason(block).to_string())?;
    let kind = AgentKind::parse(&launch.agent_kind).ok_or("invalid")?;
    if !Path::new(&launch.workspace).is_dir() {
        return Err("workspaceGone".into());
    }
    // A model or effort the phone picked is matched against the Agent's own list below; for that
    // the list's source is read from the same probe that says whether the Agent can start.
    let wanted = ModelChoice::new(launch.model.clone(), launch.effort.clone());
    let want_source = !wanted.is_default();
    let (ready, source) = tokio::task::spawn_blocking(move || {
        let readiness = agent_launch::readiness(kind);
        if kind == AgentKind::Codex {
            let status = crate::codex_desktop::launch_status(false);
            let runtime = status.target.filter(|_| want_source).and_then(|target| {
                crate::codex_desktop::catalog_runtime(target, readiness.executable.as_deref())
            });
            (
                status.target.is_some(),
                catalog_source(kind, None, runtime.as_ref()),
            )
        } else {
            let source = catalog_source(kind, readiness.executable.as_deref(), None);
            (readiness.ready, source)
        }
    })
    .await
    .unwrap_or((false, None));
    if !ready {
        return Err("agentNotReady".into());
    }
    let permission = effective_permission(
        config.agent_tasks.permission_prompt,
        kind,
        &launch.permission,
    )
    .ok_or("permissionChanged")?;
    let project = crate::project::detect_from(Path::new(&launch.workspace));
    let todo = match launch.todo_id.as_deref() {
        Some(id) => Some(
            crate::todos::list(&project)
                .into_iter()
                .find(|todo| todo.id == id)
                .ok_or("todoGone")?,
        ),
        None => None,
    };
    let task = resolve_task_input(todo.as_ref(), Some(&launch.text)).map_err(|_| "invalid")?;
    // The last check: it may wait for the Agent's own model list. The launch carries what that
    // list resolved the pick to, not what the phone wrote.
    let choice = resolve_pick(kind, source, &wanted).await?;

    // Asking for work from the phone makes it the place to answer what the Agent asks next, the
    // same as `/new` does for the IM it was typed in.
    let _ = set_active_channel(state, "ios").await;

    let phone_id = launch.phone_id.clone();
    let client_msg_id = launch.client_msg_id.clone();
    let spec = TaskSpec {
        channel: "ios".to_string(),
        source_target: launch.phone_id,
        workspace: launch.workspace,
        kind,
        permission,
        target: None,
        task,
        todo: todo.map(|todo| (project, todo)),
        // Resolved against the Agent's model list above; a default pick stays default.
        choice,
    };
    let result = start_task(state, spec, |record| {
        register_pending(state, record, &phone_id, &client_msg_id)
    })
    .await;
    state.watch.notify.notify_one();
    match result {
        Ok(Started::Terminal) => Ok(()),
        Ok(Started::Desktop(answer)) => {
            // The Codex App names the session right away: there is nothing to wait for.
            if let (Some(sid), Some(gateway)) = (answer["sessionId"].as_str(), ios_gateway(state)) {
                gateway.task_matched(&phone_id, &client_msg_id, &agent_key(AgentKind::Codex, sid));
            }
            Ok(())
        }
        Err(StartError::Message(message)) => Err(clip(&message, REASON_MAX_CHARS)),
        Err(StartError::Terminal(error)) => {
            log(&format!("ios: task launch failed: {error:#}"));
            Err("terminalFailed".into())
        }
    }
}

#[cfg(test)]
mod model_tests;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_ids_are_stable_and_distinct() {
        assert_eq!(workspace_id("/a/b"), workspace_id("/a/b"));
        assert_ne!(workspace_id("/a/b"), workspace_id("/a/c"));
        assert_eq!(workspace_id("/a/b").len(), 16);
    }

    #[test]
    fn the_mac_setting_decides_what_the_phone_may_ask_for() {
        use AgentTaskPermission::*;
        let allowed = |mode, kind, requested: &str| effective_permission(mode, kind, requested);
        // Asking: the phone chooses.
        assert_eq!(
            allowed(Ask, AgentKind::Claude, "yolo"),
            Some(LaunchPermission::Yolo)
        );
        assert_eq!(
            allowed(Ask, AgentKind::Claude, "default"),
            Some(LaunchPermission::AgentDefault)
        );
        // A fixed setting is not overridden, and a stale request is refused instead of changed.
        assert_eq!(
            allowed(AgentDefault, AgentKind::Claude, "default"),
            Some(LaunchPermission::AgentDefault)
        );
        assert_eq!(allowed(AgentDefault, AgentKind::Claude, "yolo"), None);
        assert_eq!(
            allowed(Yolo, AgentKind::Claude, "yolo"),
            Some(LaunchPermission::Yolo)
        );
        assert_eq!(allowed(Yolo, AgentKind::Claude, "default"), None);
        // Pi has no permission step at all.
        assert_eq!(
            allowed(Yolo, AgentKind::Pi, "yolo"),
            Some(LaunchPermission::AgentDefault)
        );
        assert_eq!(allowed(Ask, AgentKind::Claude, "root"), None);
    }

    #[test]
    fn blocks_have_stable_codes() {
        assert_eq!(block_reason(TaskBlock::Disabled), "disabled");
        assert_eq!(block_reason(TaskBlock::NotReady), "notReady");
        assert_eq!(block_reason(TaskBlock::NoTerminal), "noTerminal");
    }

    #[test]
    fn long_text_is_clipped_with_an_ellipsis() {
        assert_eq!(clip("  short  ", 10), "short");
        let clipped = clip(&"x".repeat(50), 10);
        assert_eq!(clipped.chars().count(), 10);
        assert!(clipped.ends_with('…'));
    }
}
