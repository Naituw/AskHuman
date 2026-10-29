//! Production wiring of the sync core (plan §4.4, §5 P4): where its files live, what the read hooks
//! do, turning sync on and off, the status the UI shows.
//!
//! Sync is **opt-in per Mac**: until [`enable`] has run there is no `enabled` marker in the sidecar
//! directory and every entry point here returns after a single `stat`. `todos.json` is never touched
//! by anything in this module except through the core's own local phase.

use super::cloudkeys;
use super::helper_client::{self, HelperClient, Reach};
use super::identity::FsResolver;
use super::model::{PauseReason, St};
use super::store;
use super::tick::{self, Env, Failpoints, Opts, TickError, TickReport};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// whats-next pulls only when the last complete pull is older than this (spec T11).
pub const STALE_MS: u64 = 20_000;
/// …and waits at most this long for it.
pub const PULL_BUDGET: Duration = Duration::from_millis(1_500);
/// How long a read hook waits for a running tick before giving up on the local phase.
const LOCAL_WAIT: Duration = Duration::from_millis(100);
/// The helper is (re)started at most this often by the hooks.
const HELPER_LAUNCH_GAP_MS: u64 = 60_000;

pub const DEFAULT_POLL_MINUTES: u32 = 10;

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

// ---- files --------------------------------------------------------------------

/// `~/.askhuman/state/todo-sync` (each Dev Instance has its own state dir, and never enables sync).
pub fn dir() -> PathBuf {
    crate::paths::state_dir().join("todo-sync")
}

fn marker_path(dir: &Path) -> PathBuf {
    dir.join("enabled")
}

fn last_tick_path(dir: &Path) -> PathBuf {
    dir.join("last-tick.json")
}

fn helper_config_path(dir: &Path) -> PathBuf {
    dir.join("helper.json")
}

fn helper_launch_path(dir: &Path) -> PathBuf {
    dir.join("helper-launch")
}

/// Sync is turned on for this Mac. One `stat`.
pub fn is_enabled() -> bool {
    marker_path(&dir()).exists()
}

/// Whether this build and instance can sync at all (the helper is a macOS app; Dev Instances never
/// touch the production container).
pub fn supported() -> bool {
    cfg!(target_os = "macos") && !crate::dev_instance::is_dev_instance()
}

fn device_name_from(config: &crate::config::AppConfig) -> String {
    config.todo_sync.device_name.trim().to_string()
}

pub fn production_env() -> Env {
    let config = crate::config::AppConfig::load_without_secrets();
    Env {
        todos_path: crate::todos::todos_file(),
        todos_lock: crate::todos::todos_lock(),
        dir: dir(),
        history_limit: config.general.todo_history_limit as usize,
        device_name: device_name_from(&config),
    }
}

/// The name this Mac goes by in Finder and System Settings; the default device name.
pub fn computer_name() -> String {
    // (`daemon/runtime/ios.rs` has the same lookup for its own purpose; it is private there.)
    #[cfg(target_os = "macos")]
    for key in ["ComputerName", "LocalHostName"] {
        if let Ok(out) = std::process::Command::new("scutil")
            .args(["--get", key])
            .output()
        {
            let name = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if out.status.success() && !name.is_empty() {
                return name;
            }
        }
    }
    for var in ["COMPUTERNAME", "HOSTNAME", "HOST"] {
        if let Ok(name) = std::env::var(var) {
            if !name.trim().is_empty() {
                return name.trim().to_string();
            }
        }
    }
    "Mac".to_string()
}

/// Local directories the core should keep linked besides the ones in `todos.json`: every recent
/// workspace (so a new worktree is materialized before anyone looks at it) and the caller's project.
fn known_keys(current: Option<&str>) -> Vec<String> {
    let mut keys: Vec<String> = Vec::new();
    for workspace in crate::agents::workspaces::list() {
        if workspace.hidden {
            continue;
        }
        let key = crate::project::detect_from(Path::new(&workspace.path));
        if !key.is_empty() {
            keys.push(key);
        }
    }
    if let Some(project) = current.filter(|p| !p.is_empty()) {
        keys.push(project.to_string());
    }
    keys.sort();
    keys.dedup();
    keys
}

// ---- the record of the last tick ------------------------------------------------

/// What the UI and `todo-sync status` say about the most recent tick on this Mac.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LastTick {
    pub at_ms: u64,
    pub reason: String,
    /// `ok`, `fresh`, `offline`, `timedOut`, `unavailable`, `paused`, `busy`, `disabled` or `error`.
    pub outcome: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(default)]
    pub fetched: usize,
    #[serde(default)]
    pub merged: usize,
    #[serde(default)]
    pub pushed: usize,
}

fn outcome_of(report: &TickReport) -> (&'static str, Option<String>) {
    if report.disabled {
        ("disabled", None)
    } else if report.busy {
        ("busy", None)
    } else if let Some(p) = report.paused {
        ("paused", Some(format!("{p:?}")))
    } else if let Some(why) = &report.unavailable {
        ("unavailable", Some(why.clone()))
    } else if report.offline {
        ("offline", None)
    } else if report.timed_out {
        ("timedOut", None)
    } else if report.fresh {
        ("fresh", None)
    } else {
        ("ok", None)
    }
}

fn write_json_atomic(path: &Path, value: &serde_json::Value) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension(format!("tmp-{}", uuid::Uuid::new_v4()));
    std::fs::write(
        &tmp,
        serde_json::to_vec_pretty(value).map_err(std::io::Error::other)?,
    )?;
    std::fs::rename(&tmp, path).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })
}

fn record_tick(dir: &Path, reason: &str, result: &Result<TickReport, TickError>) {
    // Ticks that did nothing because sync is off, or because another tick is busy, say nothing new.
    let last = match result {
        Ok(r) if r.disabled || r.busy => return,
        Ok(r) => {
            let (outcome, message) = outcome_of(r);
            LastTick {
                at_ms: now_ms(),
                reason: reason.to_string(),
                outcome: outcome.to_string(),
                message,
                fetched: r.fetched,
                merged: r.merged,
                pushed: r.pushed,
            }
        }
        Err(e) => LastTick {
            at_ms: now_ms(),
            reason: reason.to_string(),
            outcome: "error".to_string(),
            message: Some(e.to_string()),
            fetched: 0,
            merged: 0,
            pushed: 0,
        },
    };
    if let Ok(value) = serde_json::to_value(&last) {
        let _ = write_json_atomic(&last_tick_path(dir), &value);
    }
}

fn read_last_tick(dir: &Path) -> Option<LastTick> {
    serde_json::from_slice(&std::fs::read(last_tick_path(dir)).ok()?).ok()
}

// ---- side effects (replaceable in tests) -----------------------------------------

/// What the hooks do besides reading and writing files: starting processes.
pub trait Effects {
    /// Continue a pull that ran out of time, in a detached process.
    fn spawn_tick(&self, reason: &str);
    /// Start the helper app if it is installed (the hooks rate-limit this).
    fn launch_helper(&self);
}

pub struct RealEffects;

impl Effects for RealEffects {
    fn spawn_tick(&self, reason: &str) {
        spawn_tick_detached(reason);
    }

    fn launch_helper(&self) {
        launch_helper_app();
    }
}

fn spawn_tick_detached(reason: &str) {
    use std::process::{Command, Stdio};
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    let mut cmd = Command::new(exe);
    cmd.args(["todo-sync", "tick", "--reason", reason])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(unix)]
    unsafe {
        use std::os::unix::process::CommandExt;
        cmd.pre_exec(|| {
            libc::setsid();
            Ok(())
        });
    }
    crate::daemon::spawn::configure_background(&mut cmd);
    let _ = crate::daemon::spawn::spawn_and_reap(&mut cmd);
}

/// Where the helper app is installed (`scripts/todo-sync-install-helper.sh` puts it in
/// `~/Applications`); `ASKHUMAN_TODO_SYNC_APP` overrides for development.
pub fn helper_app_path() -> Option<PathBuf> {
    if let Some(custom) = std::env::var_os("ASKHUMAN_TODO_SYNC_APP").filter(|v| !v.is_empty()) {
        let path = PathBuf::from(custom);
        return path.exists().then_some(path);
    }
    let home = dirs_home()?;
    [
        home.join("Applications/AskHumanTodoSync.app"),
        PathBuf::from("/Applications/AskHumanTodoSync.app"),
    ]
    .into_iter()
    .find(|p| p.exists())
}

fn dirs_home() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .filter(|h| !h.is_empty())
        .map(PathBuf::from)
}

fn launch_helper_app() {
    #[cfg(target_os = "macos")]
    if let Some(app) = helper_app_path() {
        // `open -g`: start it without bringing anything to the front.
        let _ = std::process::Command::new("open")
            .arg("-g")
            .arg(app)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
    }
}

/// Start the helper unless that was tried a moment ago (every whats-next may notice it is missing).
fn launch_helper_throttled(dir: &Path, effects: &dyn Effects) {
    let stamp = helper_launch_path(dir);
    let recent = std::fs::metadata(&stamp)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.elapsed().ok())
        .is_some_and(|age| age < Duration::from_millis(HELPER_LAUNCH_GAP_MS));
    if recent {
        return;
    }
    let _ = std::fs::write(&stamp, b"");
    effects.launch_helper();
}

/// The helper is not there at all (as opposed to there but unhappy).
fn helper_missing(report: &TickReport) -> bool {
    report
        .unavailable
        .as_deref()
        .is_some_and(|why| why.contains("not running") || why.contains("cannot reach"))
}

// ---- read hooks ---------------------------------------------------------------------

/// Before a read of `project`'s todos (CLI `todo`, Stop card, popup, IM `/todo`): the local phase
/// only, never the network. A brand-new worktree gets its todos materialized before it is looked at.
pub fn before_read(project: &str) {
    if !is_enabled() {
        return;
    }
    before_read_with(&production_env(), project);
}

pub fn before_read_with(env: &Env, project: &str) {
    if !marker_path(&env.dir).exists() {
        return;
    }
    let extra = [project.to_string()];
    let result = tick::local_only(
        env,
        &FsResolver,
        &extra,
        now_ms(),
        &Failpoints::off(),
        LOCAL_WAIT,
    );
    // A local phase that worked is not a sync tick and says nothing; one that failed does.
    if result.is_err() {
        record_tick(&env.dir, "read", &result);
    }
}

/// Before whats-next builds its card (spec §6.3): the local phase, then — if the last complete pull is
/// older than [`STALE_MS`] — one pull within [`PULL_BUDGET`]. It never fails and never blocks longer;
/// a pull that did not finish in time continues in a detached tick.
pub fn before_whats_next(project: &str) {
    if !is_enabled() {
        return;
    }
    before_whats_next_with(&production_env(), project, &RealEffects);
}

pub fn before_whats_next_with(env: &Env, project: &str, effects: &dyn Effects) {
    if !marker_path(&env.dir).exists() {
        return;
    }
    let deadline = Instant::now() + PULL_BUDGET;
    let mut client =
        HelperClient::new(helper_client::socket_path(&env.dir)).with_deadline(Some(deadline));
    let opts = Opts {
        deadline: Some(deadline),
        fresh_within_ms: Some(STALE_MS),
    };
    let extra = [project.to_string()];
    let result = tick::full(
        env,
        &FsResolver,
        &mut client,
        &extra,
        now_ms(),
        &Failpoints::off(),
        &opts,
    );
    record_tick(&env.dir, "whats-next", &result);
    if let Ok(report) = &result {
        if report.timed_out && !report.busy {
            effects.spawn_tick("late");
        }
        if helper_missing(report) {
            launch_helper_throttled(&env.dir, effects);
        }
    }
}

/// The todos window opened or got focus: pull if stale, without a deadline (the caller is a
/// background thread; nothing waits for it).
pub fn refresh_if_stale() {
    if !is_enabled() {
        return;
    }
    let env = production_env();
    let mut client = HelperClient::new(helper_client::socket_path(&env.dir));
    let opts = Opts {
        deadline: None,
        fresh_within_ms: Some(STALE_MS),
    };
    let result = tick::full(
        &env,
        &FsResolver,
        &mut client,
        &known_keys(None),
        now_ms(),
        &Failpoints::off(),
        &opts,
    );
    record_tick(&env.dir, "window", &result);
    if let Ok(report) = &result {
        if helper_missing(report) {
            launch_helper_throttled(&env.dir, &RealEffects);
        }
    }
}

// ---- ticks ----------------------------------------------------------------------------

/// A complete tick with no time limit: what the helper's triggers (`AskHuman todo-sync tick`) and
/// "sync now" run.
pub fn run_tick(reason: &str) -> Result<TickReport, TickError> {
    let env = production_env();
    run_tick_with(&env, reason, &RealEffects)
}

pub fn run_tick_with(
    env: &Env,
    reason: &str,
    effects: &dyn Effects,
) -> Result<TickReport, TickError> {
    if !marker_path(&env.dir).exists() {
        return Ok(TickReport {
            disabled: true,
            ..TickReport::default()
        });
    }
    // The opt-in marker without its state file (deleted or lost): as far as the user can tell sync is
    // on, so build the state again instead of idling. A corrupt state file gets the same treatment
    // inside the tick; the cloud still has the todos and entities are adopted by id.
    let healed;
    let env = if store::exists(&env.dir) {
        env
    } else {
        let mut named = env.clone();
        if named.device_name.is_empty() {
            named.device_name = computer_name();
        }
        tick::enable(&named, now_ms())?;
        healed = named;
        &healed
    };
    let mut client = HelperClient::new(helper_client::socket_path(&env.dir));
    let result = tick::full(
        env,
        &FsResolver,
        &mut client,
        &known_keys(None),
        now_ms(),
        &Failpoints::off(),
        &Opts::default(),
    );
    record_tick(&env.dir, reason, &result);
    if let Ok(report) = &result {
        // A helper that is not running cannot have triggered this tick: somebody else did (the user
        // pressing "sync now", `enable`). Try to bring it up for next time.
        if helper_missing(report) {
            launch_helper_throttled(&env.dir, effects);
        }
    }
    result
}

// ---- enabling and disabling -----------------------------------------------------------

#[derive(Debug)]
pub enum ControlError {
    /// Dev Instances and non-macOS builds.
    Unsupported,
    Tick(TickError),
    Io(std::io::Error),
}

impl std::fmt::Display for ControlError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ControlError::Unsupported => {
                write!(
                    f,
                    "iCloud todo sync is only available on macOS, outside Dev Instances"
                )
            }
            ControlError::Tick(e) => write!(f, "{e}"),
            ControlError::Io(e) => write!(f, "{e}"),
        }
    }
}

impl From<TickError> for ControlError {
    fn from(e: TickError) -> Self {
        ControlError::Tick(e)
    }
}

impl From<std::io::Error> for ControlError {
    fn from(e: std::io::Error) -> Self {
        ControlError::Io(e)
    }
}

fn poll_minutes() -> u32 {
    crate::config::AppConfig::load_without_secrets()
        .todo_sync
        .poll_minutes
}

/// The hand-over file the helper reads (plan §2): which `AskHuman` to run, where the state lives.
pub fn write_helper_config(env: &Env, enabled: bool, poll_minutes: u32) -> std::io::Result<()> {
    let exe = std::env::current_exe()?;
    write_json_atomic(
        &helper_config_path(&env.dir),
        &json!({
            "protocol": helper_client::PROTOCOL,
            "askhuman": exe,
            "stateDir": env.dir,
            "todosPath": env.todos_path,
            "pollMinutes": poll_minutes.max(1),
            "enabled": enabled,
        }),
    )
}

/// Turn sync on for this Mac: create the sidecar state (an existing one is kept — "stop sync" keeps
/// it, so turning it on again only reconciles), tell the helper, and start the first sync in the
/// background. Failure of any later step leaves the Mac "local only" with sync merely not running.
pub fn enable() -> Result<(), ControlError> {
    if !supported() {
        return Err(ControlError::Unsupported);
    }
    let mut env = production_env();
    if env.device_name.is_empty() {
        env.device_name = computer_name();
    }
    enable_with(&env, poll_minutes(), &RealEffects)
}

pub fn enable_with(
    env: &Env,
    poll_minutes: u32,
    effects: &dyn Effects,
) -> Result<(), ControlError> {
    tick::enable(env, now_ms())?;
    std::fs::write(marker_path(&env.dir), b"")?;
    write_helper_config(env, true, poll_minutes)?;
    effects.launch_helper();
    // The first sync (account check, zone, full pull, merge, upload) runs in the background.
    effects.spawn_tick("enable");
    Ok(())
}

/// "Stop sync, keep this Mac's copy": the state stays (turning it on again reconciles
/// incrementally), nothing in the cloud or in `todos.json` changes, the helper idles.
pub fn disable() -> Result<(), ControlError> {
    let env = production_env();
    disable_with(&env, poll_minutes())
}

pub fn disable_with(env: &Env, poll_minutes: u32) -> Result<(), ControlError> {
    match std::fs::remove_file(marker_path(&env.dir)) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e.into()),
    }
    // Best effort: with the marker gone every hook and tick is already inert.
    let _ = write_helper_config(env, false, poll_minutes);
    Ok(())
}

/// "Clear iCloud data": delete the zone. This Mac pauses until the user chooses what to do next.
pub fn clear_cloud() -> Result<(), ControlError> {
    let env = production_env();
    let mut client = HelperClient::new(helper_client::socket_path(&env.dir));
    tick::clear_cloud(&env, &mut client, now_ms())?;
    Ok(())
}

/// After a pause the user decided to continue with this Mac's copy.
pub fn restart_from_local() -> Result<(), ControlError> {
    tick::restart_from_local(&production_env())?;
    Ok(())
}

/// "Try again" after making room in iCloud.
pub fn resume() -> Result<bool, ControlError> {
    Ok(tick::resume(&production_env())?)
}

// ---- status ----------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceRow {
    pub name: String,
    pub last_sync_ms: u64,
    pub this_mac: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HelperStatus {
    /// `running`, `notRunning`, `incompatible` or `unsupported`.
    pub state: String,
    pub version: Option<String>,
    /// CloudKit environment of the running build (`production` / `development`).
    pub environment: Option<String>,
    /// Why the running helper cannot reach CloudKit, if it cannot.
    pub problem: Option<String>,
    pub installed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub supported: bool,
    pub enabled: bool,
    /// A tick is running right now (its lock is held).
    pub syncing: bool,
    pub device_name: String,
    /// `accountChanged`, `zoneMissing`, `notSignedIn` or `quota`.
    pub paused: Option<String>,
    pub last_pull_ok_ms: u64,
    pub last_push_ok_ms: u64,
    /// Entities with local changes the cloud has not acknowledged.
    pub unsynced: usize,
    pub todos: usize,
    pub history: usize,
    pub devices: Vec<DeviceRow>,
    pub helper: HelperStatus,
    pub last_tick: Option<LastTick>,
    /// `state.json` belongs to a newer build.
    pub newer_state: bool,
}

fn pause_name(reason: PauseReason) -> &'static str {
    match reason {
        PauseReason::AccountChanged => "accountChanged",
        PauseReason::ZoneMissing => "zoneMissing",
        PauseReason::NotSignedIn => "notSignedIn",
        PauseReason::Quota => "quota",
    }
}

pub fn status() -> Status {
    status_with(&production_env(), supported())
}

pub fn status_with(env: &Env, supported: bool) -> Status {
    let enabled = supported && marker_path(&env.dir).exists();
    let mut out = Status {
        supported,
        enabled,
        syncing: false,
        device_name: env.device_name.clone(),
        paused: None,
        last_pull_ok_ms: 0,
        last_push_ok_ms: 0,
        unsynced: 0,
        todos: 0,
        history: 0,
        devices: Vec::new(),
        helper: HelperStatus {
            state: if supported {
                "notRunning"
            } else {
                "unsupported"
            }
            .to_string(),
            version: None,
            environment: None,
            problem: None,
            installed: supported && helper_app_path().is_some(),
        },
        last_tick: read_last_tick(&env.dir),
        newer_state: false,
    };
    if enabled {
        // Probing the lock is the only way to see another process's tick; the probe holds it for
        // no longer than it takes to drop the guard.
        out.syncing = matches!(store::lock_timeout(&env.dir, Duration::ZERO), Ok(None));
    }
    match tick::peek(env) {
        Ok(Some(state)) => {
            if out.device_name.is_empty() {
                out.device_name = state.device_name.clone();
            }
            out.paused = state
                .paused
                .as_ref()
                .map(|p| pause_name(p.reason).to_string());
            out.last_pull_ok_ms = state.last_pull_ok_ms;
            out.last_push_ok_ms = state.last_push_ok_ms;
            out.unsynced = state.entities.values().filter(|r| r.dirty).count();
            for rec in state.entities.values() {
                match rec.e.st.v {
                    St::Pending => out.todos += 1,
                    St::Done => out.history += 1,
                    St::Deleted => {}
                }
            }
            let mut devices: Vec<DeviceRow> = state
                .devices
                .iter()
                .map(|(id, d)| DeviceRow {
                    name: d.name.clone(),
                    last_sync_ms: d.last_sync_ms,
                    this_mac: *id == state.device_id,
                })
                .collect();
            devices.sort_by(|a, b| b.this_mac.cmp(&a.this_mac).then(a.name.cmp(&b.name)));
            out.devices = devices;
        }
        Ok(None) => {}
        Err(TickError::NewerSchema(_)) => out.newer_state = true,
        Err(_) => {}
    }
    if supported {
        match helper_client::probe(&helper_client::socket_path(&env.dir)) {
            Reach::NotRunning => {}
            Reach::Incompatible { .. } => out.helper.state = "incompatible".to_string(),
            Reach::Running(info) => {
                out.helper.state = "running".to_string();
                out.helper.version = Some(info.helper_version);
                out.helper.environment = Some(info.environment);
                out.helper.problem = info.problem;
            }
        }
    }
    out
}

/// How many finished todos of `project` the cloud holds for every Mac: what "clear history" erases
/// when sync is on (T16). `None` while the project does not sync (sync off, or a local-only project).
pub fn cloud_history_count(project: &str) -> Option<usize> {
    if !is_enabled() {
        return None;
    }
    cloud_history_count_with(&production_env(), project)
}

pub fn cloud_history_count_with(env: &Env, project: &str) -> Option<usize> {
    let state = tick::peek(env).ok()??;
    let identity = state.links.get(project)?.p.as_ref()?;
    Some(
        state
            .entities
            .values()
            .filter(|r| r.e.p == *identity && r.e.st.v == St::Done)
            .count(),
    )
}

// ---- projects (what the iCloud mode of the todos window shows) ---------------------------------

/// One synced project, merged across this Mac's directories: a repository cloned twice, or checked
/// out in several worktrees, is one project (spec todo-sync §5.2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncProject {
    /// Normalized git remote, e.g. `github.com/acme/app`.
    pub identity: String,
    /// Project keys of this Mac's directories that belong to it, sorted. Empty when this Mac only
    /// carries the iCloud replica.
    pub keys: Vec<String>,
    /// The key the project's todos live under while none of this Mac's directories has it
    /// (`icloud:<identity>`); `None` as soon as a directory exists.
    pub cloud_key: Option<String>,
    /// The Macs that have a directory for the project, this one first. A Mac that only carries the
    /// replica is not listed: the list answers "where can I work on it".
    pub macs: Vec<DeviceRow>,
}

/// The synced projects of this Mac, ordered by identity. Empty while sync is off. Reads the sidecar
/// only (no todo text, no network).
pub fn projects() -> Vec<SyncProject> {
    if !is_enabled() {
        return Vec::new();
    }
    projects_with(&production_env())
}

pub fn projects_with(env: &Env) -> Vec<SyncProject> {
    let Ok(Some(state)) = tick::peek(env) else {
        return Vec::new();
    };
    let mut by_identity: BTreeMap<String, SyncProject> = BTreeMap::new();
    for (key, link) in &state.links {
        let Some(identity) = link.p.as_deref() else {
            continue;
        };
        let project = by_identity
            .entry(identity.to_string())
            .or_insert_with(|| SyncProject {
                identity: identity.to_string(),
                keys: Vec::new(),
                cloud_key: None,
                macs: Vec::new(),
            });
        if cloudkeys::is_virtual(key) {
            project.cloud_key = Some(key.clone());
        } else {
            project.keys.push(key.clone());
        }
    }
    let this_name = if state.device_name.is_empty() {
        env.device_name.clone()
    } else {
        state.device_name.clone()
    };
    let this_last_sync = state.last_pull_ok_ms.max(state.last_push_ok_ms);
    for project in by_identity.values_mut() {
        project.keys.sort();
        if !project.keys.is_empty() {
            project.macs.push(DeviceRow {
                name: this_name.clone(),
                last_sync_ms: this_last_sync,
                this_mac: true,
            });
        }
        let mut others: Vec<DeviceRow> = state
            .devices
            .iter()
            .filter(|(id, d)| **id != state.device_id && d.projects.contains(&project.identity))
            .map(|(_, d)| DeviceRow {
                name: d.name.clone(),
                last_sync_ms: d.last_sync_ms,
                this_mac: false,
            })
            .collect();
        others.sort_by(|a, b| a.name.cmp(&b.name));
        project.macs.extend(others);
    }
    by_identity.into_values().collect()
}

/// Which Mac a todo came from or was finished on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Origin {
    /// The Mac's name; empty while its record has not arrived yet.
    pub device: String,
    pub this_mac: bool,
}

/// "N attachments, on that Mac": attachments never sync, only this hint does (spec T6).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AttachmentHint {
    pub count: u32,
    pub device: String,
}

/// What the window shows next to a project's todos in iCloud mode, keyed by todo id.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectDetails {
    /// Todos created on another Mac (todos created here are the default and not listed).
    pub created_on: BTreeMap<String, String>,
    /// Finished todos and where they were finished.
    pub done_on: BTreeMap<String, Origin>,
    /// Pending todos whose attachments live on another Mac.
    pub attachments: BTreeMap<String, AttachmentHint>,
}

/// Origins for the todos of `project` (any key of it, including the virtual one). Empty when sync is
/// off or the project is not synced.
pub fn project_details(project: &str) -> ProjectDetails {
    if !is_enabled() {
        return ProjectDetails::default();
    }
    project_details_with(&production_env(), project)
}

pub fn project_details_with(env: &Env, project: &str) -> ProjectDetails {
    let mut out = ProjectDetails::default();
    let Ok(Some(state)) = tick::peek(env) else {
        return out;
    };
    let Some(identity) = state.project_of(project) else {
        return out;
    };
    let name_of = |device: &str| -> String {
        state
            .devices
            .get(device)
            .map(|d| d.name.clone())
            .unwrap_or_default()
    };
    let me = state.device_id.as_str();
    for (id, rec) in &state.entities {
        if rec.e.p != identity {
            continue;
        }
        match rec.e.st.v {
            St::Pending => {
                if rec.e.c.dev != me {
                    out.created_on.insert(id.clone(), name_of(&rec.e.c.dev));
                }
                if let Some(att) = rec.e.att.as_ref().filter(|a| a.dev != me) {
                    out.attachments.insert(
                        id.clone(),
                        AttachmentHint {
                            count: att.n,
                            device: name_of(&att.dev),
                        },
                    );
                }
            }
            St::Done => {
                if let Some(done) = &rec.e.dn {
                    out.done_on.insert(
                        id.clone(),
                        Origin {
                            device: name_of(&done.dev),
                            this_mac: done.dev == me,
                        },
                    );
                }
            }
            St::Deleted => {}
        }
    }
    out
}

/// "Remove this project from iCloud": every todo and finished todo of the project disappears from
/// every Mac (the project's directories and git itself are untouched). `project` is any key of it,
/// including the virtual one. Returns how many entries were removed; later todos start the project
/// again, removing is not a ban.
pub fn remove_project(project: &str) -> Result<usize, ControlError> {
    if !supported() {
        return Err(ControlError::Unsupported);
    }
    if !is_enabled() {
        return Ok(0);
    }
    remove_project_with(&production_env(), project, &RealEffects)
}

pub fn remove_project_with(
    env: &Env,
    project: &str,
    effects: &dyn Effects,
) -> Result<usize, ControlError> {
    let Some(state) = tick::peek(env)? else {
        return Ok(0);
    };
    let Some(identity) = state.project_of(project).map(str::to_string) else {
        return Ok(0);
    };
    let removed = tick::remove_project(
        env,
        &FsResolver,
        &known_keys(Some(project)),
        &identity,
        now_ms(),
        &Failpoints::off(),
    )?;
    if removed > 0 {
        // Carry the deletions to the other Macs right away instead of at the next poll.
        effects.spawn_tick("remove");
    }
    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    #[derive(Default)]
    struct Spy {
        ticks: RefCell<Vec<String>>,
        launches: RefCell<usize>,
    }

    impl Effects for Spy {
        fn spawn_tick(&self, reason: &str) {
            self.ticks.borrow_mut().push(reason.to_string());
        }
        fn launch_helper(&self) {
            *self.launches.borrow_mut() += 1;
        }
    }

    /// A throwaway installation: `todos.json`, its lock and the sidecar directory.
    fn temp_env(tag: &str) -> Env {
        let root = PathBuf::from(format!(
            "/tmp/ahp-svc-{}-{}-{}",
            tag,
            std::process::id(),
            uuid::Uuid::new_v4().simple()
        ));
        std::fs::create_dir_all(&root).unwrap();
        Env {
            todos_path: root.join("todos.json"),
            todos_lock: root.join("todos.lock"),
            dir: root.join("todo-sync"),
            history_limit: 100,
            device_name: "Test Mac".to_string(),
        }
    }

    fn cleanup(env: &Env) {
        let _ = std::fs::remove_dir_all(env.dir.parent().unwrap());
    }

    #[test]
    fn nothing_is_created_while_sync_is_off() {
        let env = temp_env("off");
        let spy = Spy::default();
        before_read_with(&env, "/work/app");
        before_whats_next_with(&env, "/work/app", &spy);
        assert!(!env.dir.exists());
        assert!(spy.ticks.borrow().is_empty());
        assert_eq!(*spy.launches.borrow(), 0);
        let report = run_tick_with(&env, "poll", &spy).unwrap();
        assert!(report.disabled);
        assert!(!env.dir.exists());
        cleanup(&env);
    }

    #[test]
    fn enable_creates_state_marker_helper_config_and_starts_the_first_sync() {
        let env = temp_env("enable");
        let spy = Spy::default();
        enable_with(&env, 7, &spy).unwrap();
        assert!(store::exists(&env.dir));
        assert!(marker_path(&env.dir).exists());
        assert_eq!(*spy.launches.borrow(), 1);
        assert_eq!(spy.ticks.borrow().as_slice(), ["enable"]);

        let handover: serde_json::Value =
            serde_json::from_slice(&std::fs::read(helper_config_path(&env.dir)).unwrap()).unwrap();
        assert_eq!(handover["enabled"], true);
        assert_eq!(handover["pollMinutes"], 7);
        assert_eq!(handover["protocol"], helper_client::PROTOCOL);
        assert_eq!(handover["stateDir"], env.dir.to_string_lossy().as_ref());
        assert_eq!(
            handover["todosPath"],
            env.todos_path.to_string_lossy().as_ref()
        );
        assert!(handover["askhuman"].as_str().is_some_and(|p| !p.is_empty()));
        cleanup(&env);
    }

    #[test]
    fn disable_keeps_the_state_and_turning_it_on_again_reuses_it() {
        let env = temp_env("again");
        let spy = Spy::default();
        enable_with(&env, 10, &spy).unwrap();
        let first = tick::peek(&env).unwrap().unwrap().device_id;

        disable_with(&env, 10).unwrap();
        assert!(!marker_path(&env.dir).exists());
        assert!(store::exists(&env.dir), "stopping keeps the sidecar state");
        let handover: serde_json::Value =
            serde_json::from_slice(&std::fs::read(helper_config_path(&env.dir)).unwrap()).unwrap();
        assert_eq!(handover["enabled"], false);
        // Disabled again is a no-op, and while off the hooks and ticks are inert.
        disable_with(&env, 10).unwrap();
        assert!(run_tick_with(&env, "poll", &spy).unwrap().disabled);

        enable_with(&env, 10, &spy).unwrap();
        assert_eq!(tick::peek(&env).unwrap().unwrap().device_id, first);
        cleanup(&env);
    }

    #[test]
    fn a_missing_state_file_behind_the_marker_is_rebuilt_by_the_next_tick() {
        let env = temp_env("heal");
        let spy = Spy::default();
        enable_with(&env, 10, &spy).unwrap();
        let first = tick::peek(&env).unwrap().unwrap().device_id;
        std::fs::remove_file(store::state_file(&env.dir)).unwrap();
        assert!(status_with(&env, true).enabled, "the marker still says on");

        let report = run_tick_with(&env, "poll", &spy).unwrap();
        assert!(!report.disabled, "sync is on: the tick must not idle");
        let state = tick::peek(&env).unwrap().expect("the state is back");
        assert_ne!(state.device_id, first, "a rebuilt state is a new device");
        assert_eq!(state.device_name, "Test Mac");

        // Without the marker nothing is created, whatever else is missing.
        disable_with(&env, 10).unwrap();
        std::fs::remove_file(store::state_file(&env.dir)).unwrap();
        assert!(run_tick_with(&env, "poll", &spy).unwrap().disabled);
        assert!(!store::exists(&env.dir));
        cleanup(&env);
    }

    // Needs the Unix-socket helper client. The non-unix fallback client reports "only available on
    // macOS" rather than "not running", so no launch is ever attempted there.
    #[cfg(unix)]
    #[test]
    fn a_missing_helper_is_soft_and_gets_started_at_most_once_a_minute() {
        let env = temp_env("missing");
        let spy = Spy::default();
        enable_with(&env, 10, &spy).unwrap();
        *spy.launches.borrow_mut() = 0;
        spy.ticks.borrow_mut().clear();

        // No socket: the account step reports the helper as unavailable; nothing fails.
        before_whats_next_with(&env, "/work/app", &spy);
        before_whats_next_with(&env, "/work/app", &spy);
        assert_eq!(*spy.launches.borrow(), 1, "second attempt is throttled");
        assert!(
            spy.ticks.borrow().is_empty(),
            "no detached tick: nothing timed out"
        );
        let last = read_last_tick(&env.dir).unwrap();
        assert_eq!(last.outcome, "unavailable");
        assert_eq!(last.reason, "whats-next");
        cleanup(&env);
    }

    #[test]
    fn status_reflects_the_sidecar_without_a_helper() {
        let env = temp_env("status");
        let off = status_with(&env, true);
        assert!(!off.enabled);
        assert_eq!(off.helper.state, "notRunning");
        assert_eq!(off.device_name, "Test Mac");

        let spy = Spy::default();
        enable_with(&env, 10, &spy).unwrap();
        let on = status_with(&env, true);
        assert!(on.enabled);
        assert_eq!(on.paused, None);
        assert_eq!((on.todos, on.history, on.unsynced), (0, 0, 0));

        let unsupported = status_with(&env, false);
        assert!(!unsupported.enabled && !unsupported.supported);
        assert_eq!(unsupported.helper.state, "unsupported");
        cleanup(&env);
    }

    #[test]
    fn a_read_hook_before_the_first_pull_leaves_todos_json_alone() {
        let env = temp_env("firstpull");
        let spy = Spy::default();
        enable_with(&env, 10, &spy).unwrap();
        std::fs::write(
            &env.todos_path,
            r#"{"projects":{"/work/app":[{"id":"t1","text":"hello","createdAtMs":1}]}}"#,
        )
        .unwrap();
        let before = std::fs::read(&env.todos_path).unwrap();
        before_read_with(&env, "/work/app");
        assert_eq!(std::fs::read(&env.todos_path).unwrap(), before);
        // …and nothing was turned into an entity yet (they wait for the first pull).
        assert!(tick::peek(&env).unwrap().unwrap().entities.is_empty());
        cleanup(&env);
    }
}
