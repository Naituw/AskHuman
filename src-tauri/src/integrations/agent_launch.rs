//! Secure Terminal launch bridge for tasks created from IM.
//!
//! IM data is stored in a private one-time record. AppleScript and the login shell only receive
//! the absolute AskHuman executable plus an opaque UUID token.

use crate::agents::model_catalog::ModelChoice;
use crate::agents::AgentKind;
use crate::config::AgentTaskPermission;
use crate::integrations::agent_rules::{self, AgentTarget};
use crate::integrations::mcp_config;
use crate::integrations::{agent_lifecycle, agent_mode};
use crate::paths;
use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
#[cfg(test)]
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
#[cfg(test)]
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
#[cfg(test)]
use std::thread::ThreadId;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const RECORD_TTL_SECS: u64 = 5 * 60;
const MAX_TASK_CHARS: usize = 3000;
/// How long the user's interactive login shell may take to answer one `command -v`. It sources the
/// whole rc (nvm, conda, completions, endpoint-security scans), which on a cold or busy machine
/// routinely takes more than two seconds; a short limit turned those slow starts into a false
/// "not installed". The phone waits 15 s for the new-task options and the version probe below runs
/// after this one, so the two together stay well inside that.
const RESOLVE_TIMEOUT: Duration = Duration::from_secs(6);
/// Limit for `<agent> --version` / `--help`, which go through the login shell as well.
const HELP_TIMEOUT: Duration = Duration::from_secs(3);
/// How long an answer is reused: an executable that was found, or a shell that ran to completion
/// and found nothing.
const PROBE_CACHE_TTL: Duration = Duration::from_secs(60);
/// How long an answer is reused when the lookup itself did not complete (timeout, shell that would
/// not start). The next caller asks again soon instead of living with a guess for a minute.
const INCOMPLETE_PROBE_TTL: Duration = Duration::from_secs(5);
/// Every line the lookup script prints starts with this; whatever else the rc writes to stdout is
/// dropped as it is read.
#[cfg(unix)]
const MARKER_PREFIX: &str = "__ASKHUMAN_";
#[cfg(unix)]
const BIN_MARKER: &str = "__ASKHUMAN_BIN__";
#[cfg(unix)]
const PATH_MARKER: &str = "__ASKHUMAN_PATH__";
/// Longest stdout line read in one piece; a longer one is split, which only ever matters for noise.
#[cfg(unix)]
const MAX_LINE_BYTES: u64 = 64 * 1024;
/// Marker lines kept from one run.
#[cfg(unix)]
const MAX_MARKER_LINES: usize = 16;
pub const LAUNCH_ID_ENV: &str = "ASKHUMAN_AGENT_TASK_LAUNCH_ID";
pub const PI_MINIMUM_VERSION: &str = "0.82.0";
const PI_MINIMUM: (u64, u64, u64) = (0, 82, 0);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum LaunchPermission {
    AgentDefault,
    Yolo,
}

/// The one-time launch protocol is shared by fresh tasks and native session forks. Missing fields
/// in records written by older AskHuman versions deserialize as `New`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(
    tag = "type",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase"
)]
pub enum LaunchMode {
    #[default]
    New,
    Fork {
        source_session_id: String,
    },
}

impl TryFrom<AgentTaskPermission> for LaunchPermission {
    type Error = anyhow::Error;

    fn try_from(value: AgentTaskPermission) -> Result<Self> {
        match value {
            AgentTaskPermission::AgentDefault => Ok(Self::AgentDefault),
            AgentTaskPermission::Yolo => Ok(Self::Yolo),
            AgentTaskPermission::Ask => Err(anyhow!("permission choice is still required")),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LaunchSource {
    pub channel: String,
    pub target: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LaunchRecord {
    pub id: String,
    pub created_at: u64,
    pub expires_at: u64,
    pub source: LaunchSource,
    pub task: String,
    pub task_sha256: String,
    #[serde(default)]
    pub files: Vec<String>,
    #[serde(default)]
    pub warnings: Vec<String>,
    #[serde(default)]
    pub payload_sha256: String,
    pub cwd: String,
    pub kind: AgentKind,
    pub permission: LaunchPermission,
    pub executable: String,
    pub askhuman_executable: String,
    #[serde(default)]
    pub launch_mode: LaunchMode,
    /// The model picked for this task. `None` (and absent in records from older versions) leaves
    /// the choice to the Agent. Covered by `payload_sha256` whenever it is set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// The reasoning effort picked for this task; same rules as `model`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
}

impl LaunchRecord {
    /// The model and effort this record carries.
    pub fn choice(&self) -> ModelChoice {
        ModelChoice::new(self.model.clone(), self.effort.clone())
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ForkReadiness {
    pub kind: AgentKind,
    pub ready: bool,
    pub diagnostics: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentReadiness {
    pub kind: AgentKind,
    pub label: String,
    pub command: String,
    pub executable: Option<String>,
    pub version: Option<String>,
    pub binary_ready: bool,
    pub lifecycle_ready: bool,
    pub integration_ready: bool,
    pub integration_mode: String,
    pub ready: bool,
    /// Whether this family can be started by New Task / Fork. Readiness still reports every
    /// family so Settings and doctor can explain unsupported integrations.
    pub launch_supported: bool,
    pub diagnostics: Vec<String>,
    /// Codex only: the oldest version that can receive the interaction protocol through its
    /// SessionStart hook (spec `codex-delegated-run-silence` D11). Informational: it never affects
    /// `binary_ready` / `ready`.
    pub protocol_minimum_version: Option<String>,
    /// `false` only when this is a Codex whose detected version is below the protocol floor.
    pub protocol_supported: bool,
}

/// What the login shell answered when asked where an Agent command lives.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Lookup {
    /// An absolute, canonical path of an executable file.
    Found(String),
    /// The shell ran to completion without handing back a usable executable. `detail` says what it
    /// answered instead when that is known (a shell alias or function, a path that is not an
    /// executable file, ...); `None` is a plain "no such command".
    NotFound { detail: Option<String> },
    /// The lookup did not complete: the shell timed out, would not start, or died. This says
    /// nothing about whether the Agent is installed.
    Failed(String),
}

impl Lookup {
    fn executable(&self) -> Option<&str> {
        match self {
            Lookup::Found(path) => Some(path),
            _ => None,
        }
    }

    fn not_found() -> Self {
        Lookup::NotFound { detail: None }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct BinaryProbe {
    lookup: Lookup,
    pi_version: Option<(u64, u64, u64)>,
    codex_version: Option<(u64, u64, u64)>,
}

struct ProbeCache {
    values: HashMap<AgentKind, (Instant, BinaryProbe)>,
    /// The latest probe per Agent that produced a usable executable. It outlives `values` so a
    /// later lookup that does not complete can fall back to it (see `recover`).
    last_good: HashMap<AgentKind, BinaryProbe>,
    inflight: HashMap<AgentKind, Arc<Mutex<()>>>,
}

fn probe_cache() -> &'static Mutex<ProbeCache> {
    static CACHE: OnceLock<Mutex<ProbeCache>> = OnceLock::new();
    CACHE.get_or_init(|| {
        Mutex::new(ProbeCache {
            values: HashMap::new(),
            last_good: HashMap::new(),
            inflight: HashMap::new(),
        })
    })
}

#[cfg(test)]
struct BinaryProbeTestHarness {
    lock: Mutex<()>,
    threads: Mutex<HashSet<ThreadId>>,
    r#override: Mutex<Option<BinaryProbe>>,
    hits: AtomicUsize,
}

#[cfg(test)]
fn binary_probe_test_harness() -> &'static BinaryProbeTestHarness {
    static HARNESS: OnceLock<BinaryProbeTestHarness> = OnceLock::new();
    HARNESS.get_or_init(|| BinaryProbeTestHarness {
        lock: Mutex::new(()),
        threads: Mutex::new(HashSet::new()),
        r#override: Mutex::new(None),
        hits: AtomicUsize::new(0),
    })
}

pub fn readiness(kind: AgentKind) -> AgentReadiness {
    readiness_with(kind, false)
}

/// Antigravity has no supported New Task or Fork path; every other family does.
pub fn launch_supported(kind: AgentKind) -> bool {
    !matches!(kind, AgentKind::Antigravity)
}

pub fn readiness_fresh(kind: AgentKind) -> AgentReadiness {
    readiness_with(kind, true)
}

fn readiness_with(kind: AgentKind, force: bool) -> AgentReadiness {
    let command = command_name(kind).to_string();
    let probe = binary_probe(kind, force);
    let executable = probe.lookup.executable().map(str::to_string);
    let lifecycle = agent_lifecycle::status(kind);
    let target = target(kind);
    let mode = agent_mode::current(target);
    let integration_ready = !integration_unavailable(target, mode);
    let pi_version = (kind == AgentKind::Pi)
        .then_some(probe.pi_version)
        .flatten();
    let codex_version = (kind == AgentKind::Codex)
        .then_some(probe.codex_version)
        .flatten();
    let version_ready = kind != AgentKind::Pi || pi_version.is_some_and(pi_version_supported);
    let binary_ready = executable.is_some() && version_ready;
    let lifecycle_ready =
        lifecycle.supported && lifecycle.enabled && lifecycle.installed && !lifecycle.outdated;
    let mut diagnostics = Vec::new();
    if !binary_ready {
        diagnostics.extend(binary_diagnostic(kind, &probe.lookup, pi_version));
    }
    if !lifecycle_ready {
        diagnostics.push(format!(
            "{} lifecycle tracking is missing or outdated",
            kind.label()
        ));
    }
    if !integration_ready {
        diagnostics.push(format!(
            "{} AskHuman integration is disabled or unavailable",
            kind.label()
        ));
    }
    AgentReadiness {
        kind,
        label: kind.label().to_string(),
        command,
        executable,
        version: pi_version
            .or(codex_version)
            .map(|(major, minor, patch)| format!("{major}.{minor}.{patch}")),
        binary_ready,
        lifecycle_ready,
        integration_ready,
        integration_mode: mode.as_str().to_string(),
        ready: binary_ready && lifecycle_ready && integration_ready,
        launch_supported: launch_supported(kind),
        diagnostics,
        protocol_minimum_version: (kind == AgentKind::Codex)
            .then(|| super::codex_protocol::MINIMUM_VERSION.to_string()),
        protocol_supported: kind != AgentKind::Codex
            || super::codex_protocol::version_supported(codex_version),
    }
}

/// Why the Agent's CLI itself is not usable, or `None` when the binary side is fine. A lookup that
/// did not complete says so rather than claiming the CLI is missing: the phone shows this line, and
/// "not found" sends people off to reinstall something that is there.
fn binary_diagnostic(
    kind: AgentKind,
    lookup: &Lookup,
    pi_version: Option<(u64, u64, u64)>,
) -> Option<String> {
    let label = kind.label();
    match lookup {
        Lookup::Failed(reason) => Some(format!("{label} CLI lookup did not complete: {reason}")),
        Lookup::NotFound { detail: None } => {
            Some(format!("{label} CLI was not found in the login shell"))
        }
        Lookup::NotFound {
            detail: Some(detail),
        } => Some(format!(
            "{label} CLI was not found in the login shell ({detail})"
        )),
        Lookup::Found(_) if kind == AgentKind::Pi => match pi_version {
            Some(version) if pi_version_supported(version) => None,
            Some((major, minor, patch)) => Some(format!(
                "Pi {major}.{minor}.{patch} is unsupported; AskHuman requires Pi >= {PI_MINIMUM_VERSION}"
            )),
            None => Some(format!(
                "Pi version could not be detected; AskHuman requires Pi >= {PI_MINIMUM_VERSION}"
            )),
        },
        Lookup::Found(_) => None,
    }
}

/// Installed Codex version from the cached binary probe (a login-shell lookup on first use).
/// `None` when Codex is not installed or its version cannot be read; callers treat that as
/// "supported" so a probing failure never blocks the integration.
pub fn codex_version() -> Option<(u64, u64, u64)> {
    binary_probe(AgentKind::Codex, false).codex_version
}

pub fn grok_version() -> Option<(u64, u64, u64)> {
    type VersionCache = Option<(String, Instant, Option<(u64, u64, u64)>)>;
    static CACHE: OnceLock<Mutex<VersionCache>> = OnceLock::new();
    let probe = binary_probe(AgentKind::Grok, false);
    let executable = probe.lookup.executable()?;
    let mut cache = CACHE.get_or_init(|| Mutex::new(None)).lock().unwrap();
    if let Some((path, at, version)) = cache.as_ref() {
        if path == executable && at.elapsed() < PROBE_CACHE_TTL {
            return *version;
        }
    }
    let version = detect_version(executable);
    *cache = Some((executable.to_owned(), Instant::now(), version));
    version
}

fn binary_probe(kind: AgentKind, force: bool) -> BinaryProbe {
    if !force {
        if let Some(hit) = cached_probe(kind) {
            return hit;
        }
    }
    let gate = {
        let mut cache = probe_cache().lock().unwrap();
        cache
            .inflight
            .entry(kind)
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone()
    };
    let _guard = gate.lock().unwrap();
    if !force {
        if let Some(hit) = cached_probe(kind) {
            return hit;
        }
    }
    let value = recover(kind, binary_probe_uncached(kind));
    probe_cache()
        .lock()
        .unwrap()
        .values
        .insert(kind, (Instant::now(), value.clone()));
    value
}

fn cached_probe(kind: AgentKind) -> Option<BinaryProbe> {
    let cache = probe_cache().lock().unwrap();
    let (at, value) = cache.values.get(&kind)?;
    (at.elapsed() < probe_ttl(kind, value)).then(|| value.clone())
}

/// How long `probe` may be reused. An answer that does not settle anything (the lookup did not
/// complete, or Pi was found but its version could not be read) is asked again soon.
fn probe_ttl(kind: AgentKind, probe: &BinaryProbe) -> Duration {
    let unsettled = matches!(probe.lookup, Lookup::Failed(_))
        || (kind == AgentKind::Pi
            && probe.lookup.executable().is_some()
            && probe.pi_version.is_none());
    if unsettled {
        INCOMPLETE_PROBE_TTL
    } else {
        PROBE_CACHE_TTL
    }
}

/// Keeps one slow or broken moment from turning an installed Agent into a missing one.
///
/// - A lookup that did not complete falls back to the executable found earlier, as long as that
///   file is still there: the CLI is evidently installed, the shell was just late.
/// - A version probe that did not complete falls back to the version read earlier from the very
///   same executable.
/// - A shell that ran to completion and found nothing is the answer: the earlier path is forgotten.
fn recover(kind: AgentKind, fresh: BinaryProbe) -> BinaryProbe {
    let mut cache = probe_cache().lock().unwrap();
    match &fresh.lookup {
        Lookup::Failed(_) => {
            let earlier = cache.last_good.get(&kind).filter(|previous| {
                previous
                    .lookup
                    .executable()
                    .is_some_and(|path| is_executable(Path::new(path)))
            });
            match earlier {
                Some(previous) => previous.clone(),
                None => fresh,
            }
        }
        Lookup::NotFound { .. } => {
            cache.last_good.remove(&kind);
            fresh
        }
        Lookup::Found(_) => {
            let mut probe = fresh;
            if let Some(previous) = cache.last_good.get(&kind) {
                if previous.lookup == probe.lookup {
                    probe.pi_version = probe.pi_version.or(previous.pi_version);
                    probe.codex_version = probe.codex_version.or(previous.codex_version);
                }
            }
            if kind != AgentKind::Pi || probe.pi_version.is_some() {
                cache.last_good.insert(kind, probe.clone());
            }
            probe
        }
    }
}

fn binary_probe_uncached(kind: AgentKind) -> BinaryProbe {
    #[cfg(test)]
    {
        let harness = binary_probe_test_harness();
        let participating = harness
            .threads
            .lock()
            .unwrap()
            .contains(&std::thread::current().id());
        if participating {
            harness.hits.fetch_add(1, Ordering::SeqCst);
            if let Some(value) = harness.r#override.lock().unwrap().clone() {
                return value;
            }
        }
    }
    let lookup = lookup_executable(command_name(kind));
    let pi_version = (kind == AgentKind::Pi)
        .then(|| lookup.executable().and_then(detect_version))
        .flatten();
    let codex_version = (kind == AgentKind::Codex)
        .then(|| lookup.executable().and_then(detect_version))
        .flatten();
    BinaryProbe {
        lookup,
        pi_version,
        codex_version,
    }
}

#[cfg(test)]
fn reset_binary_probe_test_state() {
    let harness = binary_probe_test_harness();
    harness.threads.lock().unwrap().clear();
    *harness.r#override.lock().unwrap() = None;
    harness.hits.store(0, Ordering::SeqCst);
    let mut cache = probe_cache().lock().unwrap();
    cache.values.clear();
    cache.last_good.clear();
    cache.inflight.clear();
}

#[cfg(test)]
pub(crate) fn with_binary_probe_test_state<R>(f: impl FnOnce() -> R) -> R {
    let harness = binary_probe_test_harness();
    let _guard = harness.lock.lock().unwrap();
    reset_binary_probe_test_state();
    let result = f();
    reset_binary_probe_test_state();
    result
}

#[cfg(test)]
pub(crate) fn binary_probe_test_count() -> usize {
    binary_probe_test_harness().hits.load(Ordering::SeqCst)
}

#[cfg(test)]
fn set_binary_probe_test_override(value: BinaryProbe) {
    let harness = binary_probe_test_harness();
    *harness.r#override.lock().unwrap() = Some(value);
}

#[cfg(test)]
pub(crate) fn participate_in_binary_probe_test() {
    binary_probe_test_harness()
        .threads
        .lock()
        .unwrap()
        .insert(std::thread::current().id());
}

/// Task readiness requires the active AskHuman transport to exist and be current. Prompt text and
/// Subagent Guard drift stay visible in integration settings but do not block `/new`.
fn integration_unavailable(target: AgentTarget, mode: agent_mode::Mode) -> bool {
    integration_unavailable_from(
        mode,
        agent_rules::is_installed(target),
        agent_mode::timeout_hook_supported(target),
        agent_mode::timeout_hook_is_installed(target),
        agent_mode::timeout_hook_needs_update(target),
        mcp_config::is_installed(target),
        mcp_config::needs_update(target),
    )
}

fn integration_unavailable_from(
    mode: agent_mode::Mode,
    rule_installed: bool,
    timeout_supported: bool,
    timeout_installed: bool,
    timeout_outdated: bool,
    mcp_installed: bool,
    mcp_outdated: bool,
) -> bool {
    match mode {
        agent_mode::Mode::None => true,
        agent_mode::Mode::Cli => {
            !rule_installed || (timeout_supported && (!timeout_installed || timeout_outdated))
        }
        agent_mode::Mode::Mcp => !rule_installed || !mcp_installed || mcp_outdated,
    }
}

pub fn all_readiness() -> Vec<AgentReadiness> {
    collect_readiness(None, false)
}

pub fn all_readiness_fresh() -> Vec<AgentReadiness> {
    collect_readiness(None, true)
}

pub fn collect_readiness(kind: Option<AgentKind>, force: bool) -> Vec<AgentReadiness> {
    let kinds: Vec<AgentKind> = kind
        .map(|kind| vec![kind])
        .unwrap_or_else(|| AgentKind::ALL.to_vec());
    readiness_of(kinds, force)
}

/// The families New Task / Fork can start. [`all_readiness`] also reports the ones that cannot so
/// Settings and doctor can explain them, but pickers (phone, IM `/new`) must neither offer them
/// nor list their diagnostics: the user cannot fix an unsupported family.
fn launchable_kinds() -> Vec<AgentKind> {
    AgentKind::ALL
        .into_iter()
        .filter(|kind| launch_supported(*kind))
        .collect()
}

/// Readiness of [`launchable_kinds`] only; see there for why pickers use this instead of
/// [`all_readiness`].
pub fn launchable_readiness() -> Vec<AgentReadiness> {
    readiness_of(launchable_kinds(), false)
}

fn readiness_of(kinds: Vec<AgentKind>, force: bool) -> Vec<AgentReadiness> {
    std::thread::scope(|scope| {
        let handles: Vec<_> = kinds
            .into_iter()
            .map(|kind| scope.spawn(move || readiness_with(kind, force)))
            .collect();
        handles
            .into_iter()
            .filter_map(|handle| handle.join().ok())
            .collect()
    })
}

/// Native fork capability is probed against the exact executable that will be stored in the
/// launch record. Cursor Agent CLI has no supported fork surface in V1.
pub fn fork_readiness(kind: AgentKind) -> ForkReadiness {
    const CACHE_TTL: Duration = Duration::from_secs(60);
    static CACHE: OnceLock<
        Mutex<std::collections::HashMap<AgentKind, (std::time::Instant, ForkReadiness)>>,
    > = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(std::collections::HashMap::new()));
    if let Some((at, value)) = cache.lock().unwrap().get(&kind) {
        if at.elapsed() < CACHE_TTL {
            return value.clone();
        }
    }
    let value = fork_readiness_uncached(kind);
    cache
        .lock()
        .unwrap()
        .insert(kind, (std::time::Instant::now(), value.clone()));
    value
}

fn fork_readiness_uncached(kind: AgentKind) -> ForkReadiness {
    if kind == AgentKind::Antigravity {
        return ForkReadiness {
            kind,
            ready: false,
            diagnostics: vec!["Antigravity launch/fork is unsupported".into()],
        };
    }
    if kind == AgentKind::Cursor {
        return ForkReadiness {
            kind,
            ready: false,
            diagnostics: vec!["Cursor Agent CLI does not support native session fork".into()],
        };
    }
    let base = readiness(kind);
    if !base.ready {
        return ForkReadiness {
            kind,
            ready: false,
            diagnostics: base.diagnostics,
        };
    }
    let Some(executable) = base.executable else {
        return ForkReadiness {
            kind,
            ready: false,
            diagnostics: vec!["Agent executable is unavailable".into()],
        };
    };
    let probe = match kind {
        AgentKind::Claude | AgentKind::Grok => {
            probe_help(&executable, &["--help"]).is_some_and(|text| text.contains("--fork-session"))
        }
        AgentKind::Codex => probe_help(&executable, &["fork", "--help"]).is_some_and(|text| {
            text.contains("SESSION_ID") && text.to_ascii_lowercase().contains("fork")
        }),
        AgentKind::Cursor | AgentKind::Antigravity => false,
        AgentKind::Pi => {
            probe_help(&executable, &["--help"]).is_some_and(|text| text.contains("--fork"))
        }
    };
    ForkReadiness {
        kind,
        ready: probe,
        diagnostics: (!probe)
            .then(|| format!("{} CLI does not expose native session fork", kind.label()))
            .into_iter()
            .collect(),
    }
}

pub fn all_fork_readiness() -> Vec<ForkReadiness> {
    std::thread::scope(|scope| {
        let handles: Vec<_> = AgentKind::ALL
            .into_iter()
            .map(|kind| scope.spawn(move || fork_readiness(kind)))
            .collect();
        handles
            .into_iter()
            .filter_map(|handle| handle.join().ok())
            .collect()
    })
}

#[cfg(target_os = "macos")]
pub fn terminal_available() -> bool {
    [
        "/System/Applications/Utilities/Terminal.app",
        "/Applications/Utilities/Terminal.app",
    ]
    .into_iter()
    .any(|path| Path::new(path).exists())
}

#[cfg(target_os = "windows")]
pub fn terminal_available() -> bool {
    resolve_windows_terminal().is_some()
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub fn terminal_available() -> bool {
    false
}

/// Open a harmless platform terminal self-check without resolving or starting an Agent binary.
#[cfg(target_os = "macos")]
pub fn test_terminal() -> Result<()> {
    let script = r#"tell application "Terminal"
activate
do script "printf '\\nAskHuman Terminal test succeeded.\\n'"
end tell"#;
    let status = Command::new("/usr/bin/osascript")
        .args(["-e", script])
        .status()
        .context("failed to ask Terminal.app to open a test window")?;
    status
        .success()
        .then_some(())
        .ok_or_else(|| anyhow!("Terminal.app rejected the test"))
}

/// Open a harmless Windows Terminal tab using only fixed arguments.
#[cfg(target_os = "windows")]
pub fn test_terminal() -> Result<()> {
    use std::os::windows::process::CommandExt;
    use windows_sys::Win32::System::Threading::{CREATE_NEW_PROCESS_GROUP, CREATE_NO_WINDOW};

    let terminal = resolve_windows_terminal()
        .ok_or_else(|| anyhow!("Windows Terminal (wt.exe) is unavailable"))?;
    let status = Command::new(terminal)
        .args([
            "-w",
            "new",
            "new-tab",
            "--title",
            "AskHuman Test",
            "cmd.exe",
            "/d",
            "/k",
            "echo AskHuman Terminal test succeeded.",
        ])
        .creation_flags(CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .context("failed to ask Windows Terminal to open a test window")?;
    status
        .success()
        .then_some(())
        .ok_or_else(|| anyhow!("Windows Terminal rejected the test"))
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub fn test_terminal() -> Result<()> {
    Err(anyhow!(
        "Agent task terminal launch is unsupported on this platform"
    ))
}

pub fn cleanup_expired_records() {
    let dir = paths::agent_launch_dir();
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let now = epoch_secs();
    for entry in entries.flatten() {
        let path = entry.path();
        let keep = path.extension().and_then(|value| value.to_str()) == Some("json")
            && fs::read(&path)
                .ok()
                .and_then(|bytes| serde_json::from_slice::<LaunchRecord>(&bytes).ok())
                .is_some_and(|record| record.expires_at >= now);
        if !keep {
            let _ = fs::remove_file(path);
        }
    }
}

pub fn create_record(
    source: LaunchSource,
    cwd: &Path,
    kind: AgentKind,
    permission: LaunchPermission,
    task: &str,
) -> Result<LaunchRecord> {
    create_record_with_files(source, cwd, kind, permission, task, &[], &[])
}

pub fn create_record_with_files(
    source: LaunchSource,
    cwd: &Path,
    kind: AgentKind,
    permission: LaunchPermission,
    task: &str,
    files: &[String],
    warnings: &[String],
) -> Result<LaunchRecord> {
    create_record_with_choice(
        source,
        cwd,
        kind,
        permission,
        task,
        files,
        warnings,
        &ModelChoice::default(),
    )
}

/// [`create_record_with_files`] for a task that also picks the model and/or reasoning effort. A
/// default `choice` is the same as not passing one: the Agent is started with no such flag.
#[allow(clippy::too_many_arguments)]
pub fn create_record_with_choice(
    source: LaunchSource,
    cwd: &Path,
    kind: AgentKind,
    permission: LaunchPermission,
    task: &str,
    files: &[String],
    warnings: &[String],
    choice: &ModelChoice,
) -> Result<LaunchRecord> {
    create_record_internal(
        source,
        cwd,
        kind,
        permission,
        task,
        files,
        warnings,
        LaunchMode::New,
        choice,
    )
}

pub fn create_fork_record(
    source: LaunchSource,
    cwd: &Path,
    kind: AgentKind,
    permission: LaunchPermission,
    source_session_id: &str,
    task: &str,
) -> Result<LaunchRecord> {
    validate_source_session_id(source_session_id)?;
    if crate::agents::transcript_full::transcript_mtime(kind, source_session_id).is_none() {
        return Err(anyhow!("source session transcript is unavailable"));
    }
    create_record_internal(
        source,
        cwd,
        kind,
        permission,
        task,
        &[],
        &[],
        LaunchMode::Fork {
            source_session_id: source_session_id.to_string(),
        },
        // A fork keeps the model of the session it continues.
        &ModelChoice::default(),
    )
}

#[allow(clippy::too_many_arguments)]
fn create_record_internal(
    source: LaunchSource,
    cwd: &Path,
    kind: AgentKind,
    permission: LaunchPermission,
    task: &str,
    files: &[String],
    warnings: &[String],
    launch_mode: LaunchMode,
    choice: &ModelChoice,
) -> Result<LaunchRecord> {
    if kind == AgentKind::Antigravity {
        return Err(anyhow!("Antigravity launch/fork is unsupported"));
    }
    if kind == AgentKind::Pi && permission != LaunchPermission::AgentDefault {
        return Err(anyhow!(
            "Pi does not expose a built-in permission mode; use AgentDefault"
        ));
    }
    // Checked before any readiness probe or file write, like the task boundaries below.
    choice.check_for(kind).map_err(|error| anyhow!(error))?;
    if matches!(launch_mode, LaunchMode::Fork { .. }) && !choice.is_default() {
        return Err(anyhow!("a forked session keeps its own model"));
    }
    let task = task.trim();
    if task.is_empty() {
        return Err(anyhow!("task must not be empty"));
    }
    if task.chars().count() > MAX_TASK_CHARS {
        return Err(anyhow!("task exceeds {MAX_TASK_CHARS} characters"));
    }
    if files.len() > crate::todo_attachments::MAX_ATTACHMENTS_PER_TODO {
        return Err(anyhow!("too many task attachments"));
    }
    if files
        .iter()
        .chain(warnings.iter())
        .any(|value| value.contains(['\0', '\r']))
    {
        return Err(anyhow!(
            "attachment payload contains unsupported control characters"
        ));
    }
    let cwd = fs::canonicalize(cwd).context("failed to resolve workspace")?;
    if !cwd.is_dir() {
        return Err(anyhow!("workspace is not a directory"));
    }
    let status = readiness(kind);
    if !status.ready {
        return Err(anyhow!(status.diagnostics.join("; ")));
    }
    let executable = status
        .executable
        .ok_or_else(|| anyhow!("Agent executable unavailable"))?;
    if matches!(launch_mode, LaunchMode::Fork { .. }) {
        let fork = fork_readiness(kind);
        if !fork.ready {
            return Err(anyhow!(fork.diagnostics.join("; ")));
        }
    }
    let askhuman_executable = std::env::current_exe()
        .context("failed to resolve AskHuman executable")?
        .to_string_lossy()
        .to_string();
    let created_at = epoch_secs();
    let record = LaunchRecord {
        id: uuid::Uuid::new_v4().to_string(),
        created_at,
        expires_at: created_at + RECORD_TTL_SECS,
        source,
        task: task.to_string(),
        task_sha256: sha256(task.as_bytes()),
        files: files.to_vec(),
        warnings: warnings.to_vec(),
        payload_sha256: payload_sha256(task, files, warnings, &launch_mode, choice),
        cwd: cwd.to_string_lossy().to_string(),
        kind,
        permission,
        executable,
        askhuman_executable,
        launch_mode,
        model: choice.model.clone(),
        effort: choice.effort.clone(),
    };
    write_private_record(&record)?;
    Ok(record)
}

/// Open a new Terminal.app window for an existing launch record. This never starts an Agent in the
/// current process; the one-time helper in the new terminal claims the record first.
#[cfg(target_os = "macos")]
pub fn open_terminal(record: &LaunchRecord) -> Result<()> {
    let command = terminal_helper_command(&record.askhuman_executable, &record.id);
    // `do script <command>` can inject before a newly created login shell has finished enabling
    // job control. A long-running TUI may then be treated as a background job and receive SIGTTOU
    // on its first terminal write. Create the tab first and wait for its startup command to become
    // idle before sending the one-time helper command.
    let script = r#"on run argv
tell application "Terminal"
  set launchTab to do script ""
  repeat while busy of launchTab
    delay 0.05
  end repeat
  delay 0.1
  do script (item 1 of argv) in launchTab
end tell
end run"#;
    let status = Command::new("/usr/bin/osascript")
        .args(["-e", script, &command])
        .status()
        .context("failed to ask Terminal.app to open a window")?;
    if !status.success() {
        return Err(anyhow!("Terminal.app rejected the launch request"));
    }
    Ok(())
}

/// Open a uniquely named Windows Terminal window and pass only the trusted AskHuman executable
/// plus the opaque launch token. A fixed, application-suppressed tab title gives the focus adapter
/// an exact target without exposing the task or relying on a mutable tab index.
#[cfg(target_os = "windows")]
pub fn open_terminal(record: &LaunchRecord) -> Result<()> {
    use std::os::windows::process::CommandExt;
    use windows_sys::Win32::System::Threading::{CREATE_NEW_PROCESS_GROUP, CREATE_NO_WINDOW};

    let terminal = resolve_windows_terminal()
        .ok_or_else(|| anyhow!("Windows Terminal (wt.exe) is required to launch Agent tasks"))?;
    let (window_name, tab_title) =
        super::terminal_focus::windows_terminal_identity(&record.id).map_err(anyhow::Error::msg)?;
    let mut command = Command::new(terminal);
    command
        .args(["-w", &window_name, "new-tab", "--title", &tab_title])
        .arg("--suppressApplicationTitle")
        .arg("--startingDirectory")
        .arg(&record.cwd)
        .arg(&record.askhuman_executable)
        .arg("__agent-launch")
        .arg(&record.id)
        .creation_flags(CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let status = command
        .status()
        .context("failed to ask Windows Terminal to open a window")?;
    if !status.success() {
        return Err(anyhow!("Windows Terminal rejected the launch request"));
    }
    Ok(())
}

fn terminal_helper_command(askhuman_executable: &str, launch_id: &str) -> String {
    // Terminal.app can accept the second `do script` during the short handoff from the login
    // shell to its line editor. In that race it may discard the first injected character. Keep
    // sacrificial shell whitespace ahead of the quoted executable so either outcome is valid.
    format!(
        "  {} __agent-launch {}",
        shell_quote(askhuman_executable),
        shell_quote(launch_id)
    )
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub fn open_terminal(_record: &LaunchRecord) -> Result<()> {
    Err(anyhow!("IM Agent launch is unsupported on this platform"))
}

/// Hidden helper entry point. Returns only on validation failure; success replaces this process
/// with the selected Agent so it inherits the terminal's real TTY.
pub fn run_helper(args: &[String]) -> Result<()> {
    let token = args
        .first()
        .ok_or_else(|| anyhow!("missing launch token"))?;
    let record = claim_record(token)?;
    validate_claim(&record, token)?;
    #[cfg(windows)]
    {
        let action = if super::terminal_focus::register_windows_terminal_window(&record.id).is_ok()
        {
            "window_registered"
        } else {
            "window_registration_failed"
        };
        crate::daemon::lifecycle::log_runtime_event("windows_terminal", action, None);
    }
    std::env::set_current_dir(&record.cwd).context("failed to enter workspace")?;
    let mut command = assemble_command(&record);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        Err(command.exec()).context("failed to start Agent")
    }
    #[cfg(windows)]
    {
        let status = command.status().context("failed to start Agent")?;
        if status.success() {
            Ok(())
        } else {
            Err(anyhow!("Agent exited with {status}"))
        }
    }
    #[cfg(not(any(unix, windows)))]
    Err(anyhow!("Agent launch is unsupported on this platform"))
}

fn write_private_record(record: &LaunchRecord) -> Result<()> {
    let dir = paths::agent_launch_dir();
    fs::create_dir_all(&dir)?;
    harden(&dir, 0o700);
    let path = record_path(&record.id);
    let tmp = dir.join(format!(".{}.tmp", record.id));
    fs::write(&tmp, serde_json::to_vec(record)?)?;
    harden(&tmp, 0o600);
    fs::rename(tmp, path)?;
    Ok(())
}

fn claim_record(token: &str) -> Result<LaunchRecord> {
    validate_token(token)?;
    let source = record_path(token);
    let claimed = paths::agent_launch_dir().join(format!("{token}.claimed"));
    fs::rename(&source, &claimed).context("launch record is missing or already claimed")?;
    let bytes = fs::read(&claimed)?;
    let _ = fs::remove_file(&claimed);
    serde_json::from_slice(&bytes).context("invalid launch record")
}

fn validate_claim(record: &LaunchRecord, token: &str) -> Result<()> {
    if record.id != token || epoch_secs() > record.expires_at {
        return Err(anyhow!("launch record expired or mismatched"));
    }
    if sha256(record.task.as_bytes()) != record.task_sha256 {
        return Err(anyhow!("launch record task hash mismatch"));
    }
    let choice = record.choice();
    if !record.payload_sha256.is_empty() {
        let expected = payload_sha256(
            &record.task,
            &record.files,
            &record.warnings,
            &record.launch_mode,
            &choice,
        );
        let legacy = legacy_payload_sha256(&record.task, &record.files, &record.warnings);
        // The pre-launch-mode hash only ever described a plain new task; it must not vouch for a
        // record that also names a model.
        if record.payload_sha256 != expected
            && !(record.launch_mode == LaunchMode::New
                && choice.is_default()
                && record.payload_sha256 == legacy)
        {
            return Err(anyhow!("launch record payload hash mismatch"));
        }
    }
    // The values below reach an argv; refuse anything creation would not have written.
    choice
        .check_for(record.kind)
        .map_err(|_| anyhow!("launch record carries an invalid model choice"))?;
    if record.launch_mode != LaunchMode::New && !choice.is_default() {
        return Err(anyhow!("launch record carries an invalid model choice"));
    }
    if let LaunchMode::Fork { source_session_id } = &record.launch_mode {
        validate_source_session_id(source_session_id)?;
        if crate::agents::transcript_full::transcript_mtime(record.kind, source_session_id)
            .is_none()
        {
            return Err(anyhow!("source session transcript is unavailable"));
        }
        let fork = fork_readiness(record.kind);
        if !fork.ready {
            return Err(anyhow!(fork.diagnostics.join("; ")));
        }
    }
    let cwd = fs::canonicalize(&record.cwd).context("workspace is no longer available")?;
    if !crate::path_identity::equivalent(&cwd.to_string_lossy(), &record.cwd) {
        return Err(anyhow!("workspace path changed after launch was requested"));
    }
    let executable =
        fs::canonicalize(&record.executable).context("Agent executable is unavailable")?;
    if !crate::path_identity::equivalent(&executable.to_string_lossy(), &record.executable)
        || !is_executable(&executable)
    {
        return Err(anyhow!(
            "Agent executable changed after launch was requested"
        ));
    }
    let current = std::env::current_exe()?;
    if !crate::path_identity::equivalent(&current.to_string_lossy(), &record.askhuman_executable) {
        return Err(anyhow!(
            "AskHuman executable changed after launch was requested"
        ));
    }
    Ok(())
}

fn payload_sha256(
    task: &str,
    files: &[String],
    warnings: &[String],
    launch_mode: &LaunchMode,
    choice: &ModelChoice,
) -> String {
    // A record without a model or effort hashes exactly as it did before they existed, so one
    // written by an older build still claims and nothing changes for tasks that pick nothing.
    let payload = if choice.is_default() {
        serde_json::to_vec(&(task, files, warnings, launch_mode))
    } else {
        serde_json::to_vec(&(
            task,
            files,
            warnings,
            launch_mode,
            &choice.model,
            &choice.effort,
        ))
    };
    sha256(&payload.unwrap_or_default())
}

fn legacy_payload_sha256(task: &str, files: &[String], warnings: &[String]) -> String {
    let payload = serde_json::to_vec(&(task, files, warnings)).unwrap_or_default();
    sha256(&payload)
}

fn validate_source_session_id(session_id: &str) -> Result<()> {
    if session_id.is_empty()
        || session_id.len() > 256
        || session_id
            .chars()
            .any(|ch| ch.is_whitespace() || ch.is_control())
    {
        return Err(anyhow!("invalid source session id"));
    }
    Ok(())
}

/// The command the helper replaces itself with. The record is already validated, so nothing here
/// can fail; nothing is started.
fn assemble_command(record: &LaunchRecord) -> Command {
    let mut command = Command::new(&record.executable);
    command.env(LAUNCH_ID_ENV, &record.id);
    command.args(agent_args(
        record.kind,
        record.permission,
        &record.launch_mode,
        &record.choice(),
        &task_with_attachments(&record.task, &record.files, &record.warnings),
    ));
    command
}

/// The flags that select a model and reasoning effort for a new task (none for a default
/// choice). The values were checked by `ModelChoice::check_for`; they are passed as separate
/// argv elements, never through a shell.
fn model_flags(kind: AgentKind, choice: &ModelChoice) -> Vec<String> {
    let mut flags = Vec::new();
    if let Some(model) = &choice.model {
        flags.extend(["--model".to_string(), model.clone()]);
    }
    if let Some(effort) = &choice.effort {
        match kind {
            AgentKind::Claude => flags.extend(["--effort".to_string(), effort.clone()]),
            AgentKind::Codex => flags.extend([
                "-c".to_string(),
                format!("model_reasoning_effort=\"{effort}\""),
            ]),
            AgentKind::Grok => flags.extend(["--reasoning-effort".to_string(), effort.clone()]),
            AgentKind::Pi => flags.extend(["--thinking".to_string(), effort.clone()]),
            // Folded into the model id; `check_for` rejects the combination before this point.
            AgentKind::Cursor => {}
            // Antigravity launches are rejected by readiness before command assembly.
            AgentKind::Antigravity => {}
        }
    }
    flags
}

fn agent_args(
    kind: AgentKind,
    permission: LaunchPermission,
    mode: &LaunchMode,
    choice: &ModelChoice,
    prompt: &str,
) -> Vec<String> {
    let mut args = Vec::new();
    match mode {
        LaunchMode::New => {
            if permission == LaunchPermission::Yolo {
                args.push(yolo_flag(kind).into());
            }
            args.extend(model_flags(kind, choice));
            if kind == AgentKind::Codex {
                args.push("--".into());
            }
            args.push(prompt.into());
        }
        LaunchMode::Fork { source_session_id } => match kind {
            AgentKind::Antigravity => return Vec::new(),
            AgentKind::Claude | AgentKind::Grok => {
                if permission == LaunchPermission::Yolo {
                    args.push(yolo_flag(kind).into());
                }
                args.extend([
                    "--resume".into(),
                    source_session_id.clone(),
                    "--fork-session".into(),
                    "--".into(),
                    prompt.into(),
                ]);
            }
            AgentKind::Codex => {
                args.push("fork".into());
                if permission == LaunchPermission::Yolo {
                    args.push(yolo_flag(kind).into());
                }
                args.extend(["--".into(), source_session_id.clone(), prompt.into()]);
            }
            AgentKind::Cursor => {
                // Creation rejects this mode through `fork_readiness`; keep helper fail-closed.
            }
            AgentKind::Pi => {
                args.extend(["--fork".into(), source_session_id.clone(), prompt.into()]);
            }
        },
    }
    args
}

fn probe_help(executable: &str, args: &[&str]) -> Option<String> {
    // GUI hosts often have a sparse PATH. Homebrew/npm Agent entrypoints may be scripts with an
    // `#!/usr/bin/env node` shebang, so probing the canonical script directly can fail even though
    // the login shell can launch it. Only the previously resolved executable and fixed help args
    // enter this shell; source session ids and user prompts never use this path.
    #[cfg(unix)]
    let mut child = {
        let command = std::iter::once(executable)
            .chain(args.iter().copied())
            .map(shell_quote)
            .collect::<Vec<_>>()
            .join(" ");
        Command::new(login_shell())
            .args(["-lc", &command])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .ok()?
    };
    #[cfg(windows)]
    let mut child = Command::new(executable)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .ok()?;
    #[cfg(not(any(unix, windows)))]
    return None;
    let started = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let output = child.wait_with_output().ok()?;
                if !status.success() {
                    return None;
                }
                let mut bytes = output.stdout;
                bytes.extend(output.stderr);
                return String::from_utf8(bytes).ok();
            }
            Ok(None) if started.elapsed() < HELP_TIMEOUT => {
                std::thread::sleep(Duration::from_millis(20));
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
}

/// The most output of a listing command that is kept.
const LISTING_MAX_BYTES: usize = 1024 * 1024;

/// A command that runs `executable args...` the way [`probe_help`] runs an Agent: in the user's
/// login shell on Unix (GUI hosts have a sparse PATH, and Agent entrypoints may be scripts that
/// need the PATH the login shell builds), directly on Windows. `exec` makes the Agent the process
/// that is waited for and killed, not a shell around it. The caller sets up the pipes.
///
/// Only a previously resolved executable and fixed arguments may be passed in: both are quoted,
/// but they are meant to be known words, never user input.
pub(crate) fn login_shell_command(executable: &str, args: &[&str]) -> Command {
    #[cfg(unix)]
    {
        let script = ["exec".to_string(), shell_quote(executable)]
            .into_iter()
            .chain(args.iter().map(|arg| shell_quote(arg)))
            .collect::<Vec<_>>()
            .join(" ");
        let mut command = Command::new(login_shell());
        command.args(["-lc", &script]);
        command
    }
    #[cfg(not(unix))]
    {
        let mut command = Command::new(executable);
        command.args(args);
        command
    }
}

/// What `executable args...` prints (stdout, then stderr), for the model catalogs.
///
/// Run like [`probe_help`] runs an Agent, in the user's login shell, but for output that can be
/// long: both pipes are drained while the command runs (a long list must not fill a pipe and
/// stall it), at most [`LISTING_MAX_BYTES`] are kept, and the exit status is ignored, because a
/// CLI that is not signed in may still print a usable list while the parsers accept only
/// well-formed lines. `None`: the command could not be started, was not done within `timeout`
/// (it is killed) or printed nothing.
///
/// Only the previously resolved executable and fixed listing arguments reach the shell. `exec`
/// makes the Agent the process that is waited for and killed, not a shell around it.
pub fn run_listing(executable: &str, args: &[&str], timeout: Duration) -> Option<String> {
    use std::io::Read;
    use std::sync::mpsc;

    /// What one pipe has delivered so far, and a signal for "the pipe is closed".
    struct Drained {
        kept: Arc<Mutex<Vec<u8>>>,
        closed: mpsc::Receiver<()>,
    }

    impl Drained {
        fn start(mut pipe: impl Read + Send + 'static) -> Self {
            let kept = Arc::new(Mutex::new(Vec::new()));
            let (closed_tx, closed) = mpsc::channel();
            let sink = kept.clone();
            std::thread::spawn(move || {
                let mut chunk = [0u8; 8192];
                loop {
                    match pipe.read(&mut chunk) {
                        Ok(0) | Err(_) => break,
                        Ok(read) => {
                            // Keep reading past the cap: a full pipe would stall the command.
                            let mut kept = sink.lock().unwrap();
                            let room = LISTING_MAX_BYTES.saturating_sub(kept.len());
                            kept.extend_from_slice(&chunk[..read.min(room)]);
                        }
                    }
                }
                let _ = closed_tx.send(());
            });
            Self { kept, closed }
        }

        /// Whatever arrived, after giving the pipe `grace` to close.
        fn collect(self, grace: Duration) -> Vec<u8> {
            let _ = self.closed.recv_timeout(grace);
            std::mem::take(&mut *self.kept.lock().unwrap())
        }
    }

    let mut child = login_shell_command(executable, args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .ok()?;
    let stdout = Drained::start(child.stdout.take()?);
    let stderr = Drained::start(child.stderr.take()?);
    let started = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if started.elapsed() < timeout => {
                std::thread::sleep(Duration::from_millis(20));
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
    // A background process the command left behind may hold a pipe open: wait a moment for the
    // pipes to close, then take what has arrived.
    let grace = Duration::from_millis(500);
    let mut bytes = stdout.collect(grace);
    bytes.extend(stderr.collect(grace));
    let text = String::from_utf8_lossy(&bytes).into_owned();
    (!text.trim().is_empty()).then_some(text)
}

pub fn task_with_attachments(task: &str, files: &[String], warnings: &[String]) -> String {
    let mut output = task.to_string();
    let available: Vec<&String> = files
        .iter()
        .filter(|path| Path::new(path.as_str()).is_file())
        .collect();
    let mut runtime_warnings = warnings.to_vec();
    for path in files {
        if !Path::new(path).is_file() {
            runtime_warnings.push(format!("Attachment became unavailable: {path}"));
        }
    }
    if !available.is_empty() {
        output.push_str("\n\nAttachments (local file paths):\n");
        for path in available {
            let quoted = serde_json::to_string(path).unwrap_or_else(|_| format!("\"{path}\""));
            output.push_str("- ");
            output.push_str(&quoted);
            output.push('\n');
        }
        output.push_str("Open and use these files as inputs for this task.");
    }
    if let Some(block) = crate::todo_attachments::warning_block(&runtime_warnings) {
        output.push_str("\n\n");
        output.push_str(&block);
    }
    output
}

/// Where the user's own shell says the Agent command lives. Unix asks the interactive login shell
/// (so the rc runs exactly as in a Terminal window); Windows resolves it from the system PATH.
fn lookup_executable(name: &str) -> Lookup {
    #[cfg(windows)]
    {
        match resolve_windows_executable(name) {
            Some(path) => Lookup::Found(path),
            None => Lookup::not_found(),
        }
    }
    #[cfg(unix)]
    {
        lookup_in_login_shell(&login_shell(), name, RESOLVE_TIMEOUT)
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = name;
        Lookup::not_found()
    }
}

/// `$SHELL` when it is an absolute path, else zsh (macOS's default).
#[cfg(unix)]
fn login_shell() -> String {
    std::env::var("SHELL")
        .ok()
        .filter(|value| Path::new(value).is_absolute())
        .unwrap_or_else(|| "/bin/zsh".to_string())
}

/// What one run of the login shell produced.
#[cfg(unix)]
struct ShellRun {
    /// `None`: the shell was still running at the deadline and was killed.
    status: Option<std::process::ExitStatus>,
    /// The lines of stdout that start with [`MARKER_PREFIX`], in order.
    markers: Vec<String>,
    elapsed: Duration,
}

/// Runs `shell -lic script` with a deadline and keeps the marker lines it prints.
#[cfg(unix)]
fn run_login_shell(shell: &str, script: &str, timeout: Duration) -> std::io::Result<ShellRun> {
    use std::io::{BufRead, BufReader, Read};
    use std::sync::mpsc;

    let started = Instant::now();
    let mut child = Command::new(shell)
        .args(["-lic", script])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    let pipe = child.stdout.take().expect("stdout is piped");
    let (lines, received) = mpsc::channel::<String>();
    // Drained while the shell runs so a chatty rc cannot fill the pipe and stall it. Never joined:
    // a background process the rc started (an agent, a daemon) may keep the pipe open long after
    // the shell itself is gone.
    std::thread::spawn(move || {
        let mut reader = BufReader::new(pipe);
        let mut line = Vec::new();
        let mut sent = 0;
        loop {
            line.clear();
            match (&mut reader)
                .take(MAX_LINE_BYTES)
                .read_until(b'\n', &mut line)
            {
                Ok(0) | Err(_) => break,
                Ok(_) => {}
            }
            let text = String::from_utf8_lossy(&line);
            let text = text.trim();
            if text.starts_with(MARKER_PREFIX) && sent < MAX_MARKER_LINES {
                sent += 1;
                if lines.send(text.to_string()).is_err() {
                    break;
                }
            }
        }
    });
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) if started.elapsed() < timeout => {
                std::thread::sleep(Duration::from_millis(20));
            }
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(error);
            }
        }
    };
    // Whatever the shell printed is already in the pipe. The short grace lets the reader hand it
    // over without waiting for a pipe that some other process still holds open (or keeps writing to).
    let mut markers = Vec::new();
    let drain_deadline = Instant::now() + Duration::from_millis(300);
    while let Some(left) = drain_deadline.checked_duration_since(Instant::now()) {
        let Ok(line) = received.recv_timeout(left.min(Duration::from_millis(100))) else {
            break;
        };
        markers.push(line);
    }
    Ok(ShellRun {
        status,
        markers,
        elapsed: started.elapsed(),
    })
}

/// Asks `shell` (an interactive login shell) where the command `name` lives.
///
/// The script also prints the PATH the shell came up with, for diagnosis only: when a Terminal
/// window finds an Agent and this process does not, that PATH is where the difference shows.
#[cfg(unix)]
fn lookup_in_login_shell(shell: &str, name: &str, timeout: Duration) -> Lookup {
    let (lookup, login_path, elapsed) = match run_login_shell(shell, &lookup_script(name), timeout)
    {
        Ok(run) => (
            interpret_lookup(shell, timeout, &run),
            marked_line(&run.markers, PATH_MARKER).unwrap_or_default(),
            Some(run.elapsed),
        ),
        Err(error) => (
            Lookup::Failed(format!("could not start the login shell {shell}: {error}")),
            String::new(),
            None,
        ),
    };
    if lookup.executable().is_none() {
        note_lookup_miss(name, shell, &lookup, &login_path, elapsed);
    }
    lookup
}

/// The command line the login shell runs: print the PATH it came up with, then ask where `name`
/// lives. `name` is one of the fixed command names of [`command_name`], never user input.
///
/// A command that is not there ends the shell with status 1 whichever shell it is. Left to
/// itself, `command -v` fails with 1 in bash and zsh but with 127 in dash, and [`interpret_lookup`]
/// reads 127 as a broken shell rather than as a missing command.
#[cfg(unix)]
fn lookup_script(name: &str) -> String {
    format!(
        "printf '\\n{PATH_MARKER}%s\\n' \"$PATH\"; p=$(command -v {name}) || exit 1; printf '\\n{BIN_MARKER}%s\\n' \"$p\""
    )
}

/// The text after `marker` on the first marker line that starts with it.
#[cfg(unix)]
fn marked_line(markers: &[String], marker: &str) -> Option<String> {
    markers
        .iter()
        .find_map(|line| line.strip_prefix(marker).map(str::to_string))
}

/// Turns one finished (or killed) login-shell run into a [`Lookup`].
#[cfg(unix)]
fn interpret_lookup(shell: &str, timeout: Duration, run: &ShellRun) -> Lookup {
    let shell_name = Path::new(shell).file_name().map_or_else(
        || shell.to_string(),
        |name| name.to_string_lossy().to_string(),
    );
    // A marker means `command -v` ran and succeeded, whatever the rc did to the exit status after.
    if let Some(printed) = marked_line(&run.markers, BIN_MARKER) {
        return classify_printed(&printed);
    }
    let Some(status) = run.status else {
        return Lookup::Failed(format!(
            "the login shell ({shell_name}) did not answer within {}",
            describe_wait(timeout)
        ));
    };
    match status.code() {
        // `command -v` found nothing.
        Some(1) => Lookup::not_found(),
        // The shell ended cleanly without ever running the lookup: the rc left (`exit`, `exec`).
        Some(0) => Lookup::NotFound {
            detail: Some(format!(
                "the login shell ({shell_name}) ended before running the lookup"
            )),
        },
        Some(code) => Lookup::Failed(format!(
            "the login shell ({shell_name}) exited with status {code}"
        )),
        None => Lookup::Failed(format!(
            "the login shell ({shell_name}) was killed by a signal"
        )),
    }
}

/// What `command -v` printed: a path to an executable file, or something the Agent cannot be
/// launched through (an alias, a function, a path that does not lead to an executable).
#[cfg(unix)]
fn classify_printed(printed: &str) -> Lookup {
    let printed = printed.trim();
    if printed.is_empty() {
        return Lookup::not_found();
    }
    let path = Path::new(printed);
    if !path.is_absolute() {
        // Aliases print their definition, functions and builtins print their bare name.
        let what = if printed.starts_with("alias ") {
            "a shell alias"
        } else {
            "a shell function or builtin"
        };
        return Lookup::NotFound {
            detail: Some(format!(
                "the login shell resolves it to {what}, not to an executable file"
            )),
        };
    }
    match fs::canonicalize(path) {
        Ok(real) if is_executable(&real) => Lookup::Found(real.to_string_lossy().to_string()),
        Ok(_) => Lookup::NotFound {
            detail: Some("the path it resolves to is not an executable file".to_string()),
        },
        Err(error) => Lookup::NotFound {
            detail: Some(format!("the path it resolves to cannot be opened: {error}")),
        },
    }
}

/// `6 s`, `0.3 s`.
#[cfg(unix)]
fn describe_wait(timeout: Duration) -> String {
    if timeout.subsec_millis() == 0 {
        format!("{} s", timeout.as_secs())
    } else {
        format!("{:.1} s", timeout.as_secs_f32())
    }
}

/// Whether a lookup that found nothing leaves a line in the process log. The daemon switches this
/// on: it is the process whose environment differs from a Terminal window's, and the phone only
/// shows the one-line reason.
static LOG_LOOKUP_MISSES: AtomicBool = AtomicBool::new(false);

pub fn log_lookup_misses(enabled: bool) {
    LOG_LOOKUP_MISSES.store(enabled, Ordering::Relaxed);
}

/// One line per lookup that did not find the Agent: what the shell said, how long it took, and
/// the PATH the login shell ended up with next to the PATH this process inherited. Lookups are
/// cached, so this stays at a few lines per minute at most.
#[cfg(unix)]
fn note_lookup_miss(
    name: &str,
    shell: &str,
    lookup: &Lookup,
    login_path: &str,
    elapsed: Option<Duration>,
) {
    if !LOG_LOOKUP_MISSES.load(Ordering::Relaxed) {
        return;
    }
    let outcome = match lookup {
        Lookup::Found(_) => return,
        Lookup::NotFound { detail: None } => "not found".to_string(),
        Lookup::NotFound {
            detail: Some(detail),
        } => format!("not found ({detail})"),
        Lookup::Failed(reason) => format!("lookup did not complete ({reason})"),
    };
    let took = elapsed.map_or_else(
        || "-".to_string(),
        |elapsed| elapsed.as_millis().to_string(),
    );
    let inherited = std::env::var("PATH").unwrap_or_default();
    eprintln!(
        "[agent-lookup] {name}: {outcome}; shell={shell}; took={took}ms; login-shell PATH={login_path}; inherited PATH={inherited}"
    );
}

#[cfg(windows)]
pub(crate) fn resolve_windows_terminal() -> Option<String> {
    // Microsoft Store execution aliases are zero-byte reparse points. Rust canonicalization can
    // reject them even though CreateProcess resolves them correctly, so prefer the fixed per-user
    // WindowsApps alias instead of treating it like a regular Agent executable.
    let alias = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)?
        .join("Microsoft")
        .join("WindowsApps")
        .join("wt.exe");
    if fs::symlink_metadata(&alias).is_ok() {
        return Some(alias.to_string_lossy().to_string());
    }
    resolve_windows_executable("wt.exe")
}

#[cfg(windows)]
fn resolve_windows_executable(name: &str) -> Option<String> {
    let output = Command::new("where.exe")
        .arg(name)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(PathBuf::from)
        // npm installs both an extensionless POSIX shim and a `.cmd` launcher. `where.exe`
        // returns the POSIX shim first, but CreateProcess rejects it with ERROR_BAD_EXE_FORMAT.
        .filter(|path| is_windows_launchable(path))
        .find_map(|path| fs::canonicalize(path).ok())
        .filter(|path| is_windows_launchable(path))
        .map(|path| path.to_string_lossy().to_string())
}

fn target(kind: AgentKind) -> AgentTarget {
    match kind {
        AgentKind::Claude => AgentTarget::ClaudeCode,
        AgentKind::Codex => AgentTarget::Codex,
        AgentKind::Cursor => AgentTarget::Cursor,
        AgentKind::Grok => AgentTarget::Grok,
        AgentKind::Pi => AgentTarget::Pi,
        AgentKind::Antigravity => AgentTarget::Antigravity,
    }
}

fn command_name(kind: AgentKind) -> &'static str {
    match kind {
        AgentKind::Claude => "claude",
        AgentKind::Codex => "codex",
        AgentKind::Cursor => "cursor-agent",
        AgentKind::Grok => "grok",
        AgentKind::Pi => "pi",
        AgentKind::Antigravity => "agy",
    }
}

fn yolo_flag(kind: AgentKind) -> &'static str {
    match kind {
        AgentKind::Claude => "--dangerously-skip-permissions",
        AgentKind::Codex => "--dangerously-bypass-approvals-and-sandbox",
        AgentKind::Cursor => "--yolo",
        AgentKind::Grok => "--always-approve",
        AgentKind::Pi | AgentKind::Antigravity => "",
    }
}

/// `<executable> --version` parsed as `major.minor[.patch]` (Pi prints `pi 0.82.0`, Codex
/// `codex-cli 0.163.0`).
fn detect_version(executable: &str) -> Option<(u64, u64, u64)> {
    let output = probe_help(executable, &["--version"])?;
    parse_pi_version(&output)
}

fn parse_pi_version(output: &str) -> Option<(u64, u64, u64)> {
    output
        .split(|ch: char| !(ch.is_ascii_digit() || ch == '.'))
        .filter(|part| !part.is_empty())
        .find_map(|part| {
            let mut numbers = part.split('.').map(str::parse::<u64>);
            let major = numbers.next()?.ok()?;
            let minor = numbers.next()?.ok()?;
            let patch = numbers.next().transpose().ok()?.unwrap_or(0);
            Some((major, minor, patch))
        })
}

fn pi_version_supported(version: (u64, u64, u64)) -> bool {
    version >= PI_MINIMUM
}

fn record_path(token: &str) -> PathBuf {
    paths::agent_launch_dir().join(format!("{token}.json"))
}

fn validate_token(token: &str) -> Result<()> {
    uuid::Uuid::parse_str(token).context("invalid launch token")?;
    Ok(())
}

fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn epoch_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.is_file() && fs::metadata(path).is_ok_and(|m| m.permissions().mode() & 0o111 != 0)
}

fn is_windows_launchable(path: &Path) -> bool {
    path.is_file()
        && path
            .extension()
            .and_then(|value| value.to_str())
            .is_some_and(|extension| {
                matches!(
                    extension.to_ascii_lowercase().as_str(),
                    "exe" | "com" | "cmd" | "bat"
                )
            })
}

#[cfg(windows)]
fn is_executable(path: &Path) -> bool {
    is_windows_launchable(path)
}

#[cfg(not(any(unix, windows)))]
fn is_executable(path: &Path) -> bool {
    path.is_file()
}

#[cfg(unix)]
fn harden(path: &Path, mode: u32) {
    use std::os::unix::fs::PermissionsExt;
    let _ = fs::set_permissions(path, fs::Permissions::from_mode(mode));
}

#[cfg(not(unix))]
fn harden(_path: &Path, _mode: u32) {}

#[cfg(all(test, unix))]
mod listing_tests;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codex_new_tasks_keep_option_like_and_multiline_text_as_one_argument() {
        for prompt in [
            "ordinary task",
            "-x",
            "--help",
            "--bad-option\nquotes ' \" $HOME $(touch /tmp/never) `echo test`",
        ] {
            assert_eq!(
                agent_args(
                    AgentKind::Codex,
                    LaunchPermission::AgentDefault,
                    &LaunchMode::New,
                    &ModelChoice::default(),
                    prompt
                ),
                vec!["--", prompt]
            );
            assert_eq!(
                agent_args(
                    AgentKind::Codex,
                    LaunchPermission::Yolo,
                    &LaunchMode::New,
                    &ModelChoice::default(),
                    prompt
                ),
                vec!["--dangerously-bypass-approvals-and-sandbox", "--", prompt]
            );
            assert_eq!(
                agent_args(
                    AgentKind::Claude,
                    LaunchPermission::AgentDefault,
                    &LaunchMode::New,
                    &ModelChoice::default(),
                    prompt
                ),
                vec![prompt]
            );
        }
    }

    fn choice(model: Option<&str>, effort: Option<&str>) -> ModelChoice {
        ModelChoice::new(model.map(str::to_string), effort.map(str::to_string))
    }

    #[test]
    fn a_default_choice_leaves_every_agent_argv_as_it_was() {
        let prompt = "task";
        let default = ModelChoice::default();
        let argv =
            |kind, permission| agent_args(kind, permission, &LaunchMode::New, &default, prompt);
        for kind in AgentKind::ALL {
            let plain = if kind == AgentKind::Codex {
                vec!["--", prompt]
            } else {
                vec![prompt]
            };
            assert_eq!(
                argv(kind, LaunchPermission::AgentDefault),
                plain,
                "{kind:?}"
            );
            if kind != AgentKind::Pi {
                let mut yolo = vec![yolo_flag(kind)];
                yolo.extend(plain);
                assert_eq!(argv(kind, LaunchPermission::Yolo), yolo, "{kind:?}");
            }
        }
    }

    #[test]
    fn model_and_effort_become_fixed_flags_ahead_of_the_prompt() {
        let prompt = "--model\nnot a flag $(touch /tmp/never)";
        let new = |kind, permission, choice: &ModelChoice| {
            agent_args(kind, permission, &LaunchMode::New, choice, prompt)
        };
        let both = choice(Some("model-a"), Some("high"));

        assert_eq!(
            new(AgentKind::Claude, LaunchPermission::AgentDefault, &both),
            vec!["--model", "model-a", "--effort", "high", prompt]
        );
        assert_eq!(
            new(AgentKind::Codex, LaunchPermission::AgentDefault, &both),
            vec![
                "--model",
                "model-a",
                "-c",
                "model_reasoning_effort=\"high\"",
                "--",
                prompt
            ]
        );
        assert_eq!(
            new(AgentKind::Grok, LaunchPermission::AgentDefault, &both),
            vec!["--model", "model-a", "--reasoning-effort", "high", prompt]
        );
        assert_eq!(
            new(AgentKind::Pi, LaunchPermission::AgentDefault, &both),
            vec!["--model", "model-a", "--thinking", "high", prompt]
        );
        // Cursor has no effort switch: only the model id is passed.
        assert_eq!(
            new(
                AgentKind::Cursor,
                LaunchPermission::AgentDefault,
                &choice(Some("model-a"), None)
            ),
            vec!["--model", "model-a", prompt]
        );

        // Either half on its own, and after the permission flag.
        assert_eq!(
            new(
                AgentKind::Claude,
                LaunchPermission::AgentDefault,
                &choice(Some("opus"), None)
            ),
            vec!["--model", "opus", prompt]
        );
        assert_eq!(
            new(
                AgentKind::Claude,
                LaunchPermission::AgentDefault,
                &choice(None, Some("max"))
            ),
            vec!["--effort", "max", prompt]
        );
        assert_eq!(
            new(AgentKind::Codex, LaunchPermission::Yolo, &both),
            vec![
                "--dangerously-bypass-approvals-and-sandbox",
                "--model",
                "model-a",
                "-c",
                "model_reasoning_effort=\"high\"",
                "--",
                prompt
            ]
        );
        assert_eq!(
            new(AgentKind::Grok, LaunchPermission::Yolo, &both),
            vec![
                "--always-approve",
                "--model",
                "model-a",
                "--reasoning-effort",
                "high",
                prompt
            ]
        );
    }

    #[test]
    fn a_fork_never_takes_a_model_flag() {
        let fork = LaunchMode::Fork {
            source_session_id: "source-123".into(),
        };
        // Creation refuses the combination; the argv builder ignores a choice in this mode too.
        assert_eq!(
            agent_args(
                AgentKind::Claude,
                LaunchPermission::AgentDefault,
                &fork,
                &choice(Some("model-a"), Some("high")),
                "go"
            ),
            agent_args(
                AgentKind::Claude,
                LaunchPermission::AgentDefault,
                &fork,
                &ModelChoice::default(),
                "go"
            )
        );
    }

    #[test]
    fn launch_support_excludes_only_antigravity() {
        for kind in AgentKind::ALL {
            assert_eq!(
                launch_supported(kind),
                kind != AgentKind::Antigravity,
                "{kind:?}"
            );
        }
        // Phone and IM pickers probe, offer and explain only the families that can launch.
        let launchable = launchable_kinds();
        assert!(!launchable.contains(&AgentKind::Antigravity));
        assert_eq!(launchable.len(), AgentKind::ALL.len() - 1);
    }

    #[test]
    fn yolo_flags_are_fixed() {
        assert_eq!(
            yolo_flag(AgentKind::Claude),
            "--dangerously-skip-permissions"
        );
        assert_eq!(
            yolo_flag(AgentKind::Codex),
            "--dangerously-bypass-approvals-and-sandbox"
        );
        assert_eq!(yolo_flag(AgentKind::Cursor), "--yolo");
        assert_eq!(yolo_flag(AgentKind::Grok), "--always-approve");
        assert_eq!(yolo_flag(AgentKind::Pi), "");
    }

    #[test]
    fn fork_arguments_are_fixed_and_keep_prompt_as_one_argv() {
        let prompt = "-$(touch /tmp/never)\nsecond line";
        let fork = LaunchMode::Fork {
            source_session_id: "source-123".into(),
        };
        let default = ModelChoice::default();
        assert_eq!(
            agent_args(
                AgentKind::Claude,
                LaunchPermission::AgentDefault,
                &fork,
                &default,
                prompt
            ),
            vec!["--resume", "source-123", "--fork-session", "--", prompt]
        );
        assert_eq!(
            agent_args(
                AgentKind::Codex,
                LaunchPermission::Yolo,
                &fork,
                &default,
                prompt
            ),
            vec![
                "fork",
                "--dangerously-bypass-approvals-and-sandbox",
                "--",
                "source-123",
                prompt,
            ]
        );
        assert_eq!(
            agent_args(
                AgentKind::Grok,
                LaunchPermission::Yolo,
                &fork,
                &default,
                prompt
            ),
            vec![
                "--always-approve",
                "--resume",
                "source-123",
                "--fork-session",
                "--",
                prompt,
            ]
        );
        assert!(agent_args(
            AgentKind::Cursor,
            LaunchPermission::AgentDefault,
            &fork,
            &default,
            prompt
        )
        .is_empty());
        assert_eq!(
            agent_args(
                AgentKind::Pi,
                LaunchPermission::AgentDefault,
                &fork,
                &default,
                prompt
            ),
            vec!["--fork", "source-123", prompt]
        );
    }

    #[test]
    fn pi_version_gate_accepts_082_and_newer() {
        assert_eq!(parse_pi_version("pi 0.82.0"), Some((0, 82, 0)));
        assert_eq!(parse_pi_version("v1.3.4\n"), Some((1, 3, 4)));
        assert_eq!(parse_pi_version("pi version 0.81"), Some((0, 81, 0)));
        assert!(pi_version_supported((0, 82, 0)));
        assert!(pi_version_supported((0, 83, 0)));
        assert!(!pi_version_supported((0, 81, 99)));
        assert_eq!(parse_pi_version("unknown"), None);
    }

    #[test]
    fn binary_probe_cache_reuses_until_force() {
        with_binary_probe_test_state(|| {
            participate_in_binary_probe_test();
            set_binary_probe_test_override(BinaryProbe {
                lookup: Lookup::Found("/tmp/pi-test".into()),
                pi_version: Some((0, 82, 0)),
                codex_version: None,
            });
            let first = binary_probe(AgentKind::Pi, false);
            let second = binary_probe(AgentKind::Pi, false);
            assert_eq!(first, second);
            assert_eq!(binary_probe_test_count(), 1);
            let _forced = binary_probe(AgentKind::Pi, true);
            assert_eq!(binary_probe_test_count(), 2);
        });
    }

    #[test]
    fn binary_probe_single_flight_shares_one_uncached_call() {
        with_binary_probe_test_state(|| {
            set_binary_probe_test_override(BinaryProbe {
                lookup: Lookup::Found("/tmp/grok-test".into()),
                pi_version: None,
                codex_version: None,
            });
            std::thread::scope(|scope| {
                let first = scope.spawn(|| {
                    participate_in_binary_probe_test();
                    binary_probe(AgentKind::Grok, false)
                });
                let second = scope.spawn(|| {
                    participate_in_binary_probe_test();
                    binary_probe(AgentKind::Grok, false)
                });
                first.join().unwrap();
                second.join().unwrap();
            });
            assert_eq!(binary_probe_test_count(), 1);
        });
    }

    fn probe(lookup: Lookup) -> BinaryProbe {
        BinaryProbe {
            lookup,
            pi_version: None,
            codex_version: None,
        }
    }

    #[test]
    fn only_unsettled_answers_are_asked_again_soon() {
        let found = probe(Lookup::Found("/x/claude".into()));
        assert_eq!(probe_ttl(AgentKind::Claude, &found), PROBE_CACHE_TTL);
        assert_eq!(
            probe_ttl(AgentKind::Claude, &probe(Lookup::not_found())),
            PROBE_CACHE_TTL
        );
        assert_eq!(
            probe_ttl(AgentKind::Claude, &probe(Lookup::Failed("late".into()))),
            INCOMPLETE_PROBE_TTL
        );
        // Pi is only usable with a readable version, so "found, version unknown" is unsettled too.
        assert_eq!(probe_ttl(AgentKind::Pi, &found), INCOMPLETE_PROBE_TTL);
        let pi = BinaryProbe {
            pi_version: Some((0, 82, 0)),
            ..found
        };
        assert_eq!(probe_ttl(AgentKind::Pi, &pi), PROBE_CACHE_TTL);
    }

    #[test]
    fn diagnostics_say_what_the_lookup_actually_found_out() {
        let label = AgentKind::Claude.label();
        assert_eq!(
            binary_diagnostic(AgentKind::Claude, &Lookup::not_found(), None),
            Some(format!("{label} CLI was not found in the login shell"))
        );
        assert_eq!(
            binary_diagnostic(
                AgentKind::Claude,
                &Lookup::NotFound {
                    detail: Some("the login shell resolves it to a shell alias".into())
                },
                None
            ),
            Some(format!(
                "{label} CLI was not found in the login shell (the login shell resolves it to a shell alias)"
            ))
        );
        // A lookup that never finished must not claim the CLI is missing.
        assert_eq!(
            binary_diagnostic(
                AgentKind::Claude,
                &Lookup::Failed("the login shell (zsh) did not answer within 6 s".into()),
                None
            ),
            Some(format!(
                "{label} CLI lookup did not complete: the login shell (zsh) did not answer within 6 s"
            ))
        );
        let found = Lookup::Found("/x/claude".into());
        assert_eq!(binary_diagnostic(AgentKind::Claude, &found, None), None);
        let pi = Lookup::Found("/x/pi".into());
        assert_eq!(
            binary_diagnostic(AgentKind::Pi, &pi, Some((0, 82, 0))),
            None
        );
        assert!(binary_diagnostic(AgentKind::Pi, &pi, Some((0, 81, 0)))
            .is_some_and(|text| text.contains("0.81.0 is unsupported")));
        assert!(binary_diagnostic(AgentKind::Pi, &pi, None)
            .is_some_and(|text| text.contains("could not be detected")));
    }

    #[cfg(unix)]
    fn executable_file(dir: &Path, name: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join(name);
        fs::write(&path, "#!/bin/sh\n").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    #[test]
    #[cfg(unix)]
    fn an_incomplete_lookup_keeps_the_executable_found_earlier() {
        with_binary_probe_test_state(|| {
            participate_in_binary_probe_test();
            let dir = tempfile::tempdir().unwrap();
            let path = executable_file(dir.path(), "cursor-agent");
            let found = probe(Lookup::Found(path.to_string_lossy().to_string()));

            set_binary_probe_test_override(found.clone());
            assert_eq!(binary_probe(AgentKind::Cursor, true), found);

            // The shell is late next time: the CLI is evidently still installed.
            set_binary_probe_test_override(probe(Lookup::Failed("late".into())));
            assert_eq!(binary_probe(AgentKind::Cursor, true), found);

            // A shell that ran to completion and found nothing is the answer, and the earlier
            // path is forgotten with it.
            set_binary_probe_test_override(probe(Lookup::not_found()));
            assert_eq!(
                binary_probe(AgentKind::Cursor, true).lookup,
                Lookup::not_found()
            );
            set_binary_probe_test_override(probe(Lookup::Failed("late".into())));
            assert_eq!(
                binary_probe(AgentKind::Cursor, true).lookup,
                Lookup::Failed("late".into())
            );
        });
    }

    #[test]
    #[cfg(unix)]
    fn an_earlier_executable_that_is_gone_is_not_resurrected() {
        with_binary_probe_test_state(|| {
            participate_in_binary_probe_test();
            let dir = tempfile::tempdir().unwrap();
            let path = executable_file(dir.path(), "cursor-agent");

            set_binary_probe_test_override(probe(Lookup::Found(
                path.to_string_lossy().to_string(),
            )));
            binary_probe(AgentKind::Cursor, true);
            fs::remove_file(&path).unwrap();

            set_binary_probe_test_override(probe(Lookup::Failed("late".into())));
            assert_eq!(
                binary_probe(AgentKind::Cursor, true).lookup,
                Lookup::Failed("late".into())
            );
        });
    }

    #[test]
    fn a_version_probe_that_did_not_complete_keeps_the_version_read_earlier() {
        with_binary_probe_test_state(|| {
            participate_in_binary_probe_test();
            let with_version = BinaryProbe {
                pi_version: Some((0, 82, 0)),
                ..probe(Lookup::Found("/x/pi".into()))
            };
            set_binary_probe_test_override(with_version.clone());
            assert_eq!(binary_probe(AgentKind::Pi, true), with_version);

            // `pi --version` timed out this time, for the same executable.
            set_binary_probe_test_override(probe(Lookup::Found("/x/pi".into())));
            assert_eq!(binary_probe(AgentKind::Pi, true), with_version);

            // A different executable has not been read yet, so nothing is carried over.
            set_binary_probe_test_override(probe(Lookup::Found("/y/pi".into())));
            assert_eq!(binary_probe(AgentKind::Pi, true).pi_version, None);
        });
    }

    #[test]
    #[cfg(unix)]
    fn waits_are_described_in_seconds() {
        assert_eq!(describe_wait(Duration::from_secs(6)), "6 s");
        assert_eq!(describe_wait(Duration::from_millis(300)), "0.3 s");
        assert_eq!(describe_wait(Duration::from_millis(2500)), "2.5 s");
    }

    /// A stand-in for the login shell. The script only runs for `-lic`, so the warm-up exec below
    /// returns at once.
    #[cfg(unix)]
    fn fake_login_shell(dir: &Path, name: &str, body: &str) -> String {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join(name);
        fs::write(
            &path,
            format!("#!/bin/sh\n[ \"$1\" = \"-lic\" ] || exit 0\n{body}\n"),
        )
        .unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        // A fork in another test can briefly hold the write handle, which fails the first exec
        // with ETXTBSY. Get past that here rather than inside the code under test.
        for _ in 0..50 {
            let status = Command::new(&path)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
            match status {
                Err(error) if error.kind() == std::io::ErrorKind::ExecutableFileBusy => {
                    std::thread::sleep(Duration::from_millis(20));
                }
                _ => break,
            }
        }
        path.to_string_lossy().to_string()
    }

    #[test]
    #[cfg(unix)]
    fn the_lookup_script_prints_the_path_and_the_command_it_found() {
        let dir = tempfile::tempdir().unwrap();
        let pi = executable_file(dir.path(), "pi");
        let search = format!("{}:/usr/bin:/bin", dir.path().display());
        // Every one of these that is installed: dash fails a missing `command -v` with 127 where
        // bash says 1, and a Linux runner's /bin/sh is dash while a Mac's is bash. (zsh is left
        // out: it reads ~/.zshenv even for `-c`, which can change the PATH the test compares.)
        let shells: Vec<&str> = ["/bin/sh", "/bin/bash", "/bin/dash"]
            .into_iter()
            .filter(|shell| Path::new(shell).exists())
            .collect();
        assert!(!shells.is_empty());

        for shell in shells {
            let run = |name: &str| {
                Command::new(shell)
                    .args(["-c", &lookup_script(name)])
                    .env("PATH", &search)
                    .output()
                    .unwrap()
            };

            let found = run("pi");
            let printed = String::from_utf8_lossy(&found.stdout).to_string();
            assert!(found.status.success(), "{shell}: {printed}");
            assert!(
                printed.contains(&format!("{PATH_MARKER}{search}")),
                "{shell}: {printed}"
            );
            assert!(
                printed.contains(&format!("{BIN_MARKER}{}", pi.display())),
                "{shell}: {printed}"
            );

            // A command that does not exist ends with status 1 and no answer line, but the PATH
            // line is still there for diagnosis.
            let missing = run("askhuman-no-such-command");
            let printed = String::from_utf8_lossy(&missing.stdout).to_string();
            assert_eq!(missing.status.code(), Some(1), "{shell}");
            assert!(printed.contains(PATH_MARKER), "{shell}: {printed}");
            assert!(!printed.contains(BIN_MARKER), "{shell}: {printed}");
        }
    }

    #[test]
    #[cfg(unix)]
    fn login_shell_lookup_returns_the_canonical_executable() {
        let dir = tempfile::tempdir().unwrap();
        let real = executable_file(dir.path(), "claude-2.1.0");
        let link = dir.path().join("claude");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        let shell = fake_login_shell(
            dir.path(),
            "found-shell",
            &format!(
                "echo '{PATH_MARKER}/usr/bin'\necho '{BIN_MARKER}{}'",
                link.display()
            ),
        );
        assert_eq!(
            lookup_in_login_shell(&shell, "claude", Duration::from_secs(10)),
            Lookup::Found(
                fs::canonicalize(&real)
                    .unwrap()
                    .to_string_lossy()
                    .to_string()
            )
        );
    }

    #[test]
    #[cfg(unix)]
    fn login_shell_lookup_reports_a_command_that_is_not_there() {
        let dir = tempfile::tempdir().unwrap();
        let shell = fake_login_shell(
            dir.path(),
            "empty-shell",
            &format!("echo '{PATH_MARKER}/usr/bin'\nexit 1"),
        );
        assert_eq!(
            lookup_in_login_shell(&shell, "pi", Duration::from_secs(10)),
            Lookup::not_found()
        );
        // An rc that ends the shell before the lookup ran says nothing about the command.
        let left_early = fake_login_shell(dir.path(), "left-early-shell", "exit 0");
        let lookup = lookup_in_login_shell(&left_early, "pi", Duration::from_secs(10));
        assert!(
            matches!(&lookup, Lookup::NotFound { detail: Some(detail) } if detail.contains("ended before running the lookup")),
            "{lookup:?}"
        );
    }

    #[test]
    #[cfg(unix)]
    fn login_shell_lookup_explains_answers_that_are_not_executable_files() {
        let dir = tempfile::tempdir().unwrap();
        let plain = dir.path().join("plain-file");
        fs::write(&plain, "not a program").unwrap();
        let cases = [
            (
                "alias claude=/opt/claude/bin/claude".to_string(),
                "a shell alias",
            ),
            ("claude".to_string(), "a shell function or builtin"),
            (plain.display().to_string(), "is not an executable file"),
            (
                dir.path().join("missing").display().to_string(),
                "cannot be opened",
            ),
        ];
        for (index, (printed, expected)) in cases.into_iter().enumerate() {
            let shell = fake_login_shell(
                dir.path(),
                &format!("answer-shell-{index}"),
                &format!("echo '{BIN_MARKER}{printed}'"),
            );
            let lookup = lookup_in_login_shell(&shell, "claude", Duration::from_secs(10));
            assert!(
                matches!(&lookup, Lookup::NotFound { detail: Some(detail) } if detail.contains(expected)),
                "{printed}: {lookup:?}"
            );
        }
    }

    #[test]
    #[cfg(unix)]
    fn login_shell_lookup_gives_up_on_a_shell_that_does_not_answer() {
        let dir = tempfile::tempdir().unwrap();
        let shell = fake_login_shell(dir.path(), "slow-shell", "sleep 5");
        let started = Instant::now();
        let lookup = lookup_in_login_shell(&shell, "claude", Duration::from_millis(300));
        assert_eq!(
            lookup,
            Lookup::Failed("the login shell (slow-shell) did not answer within 0.3 s".to_string())
        );
        // The orphaned `sleep` keeps the pipe open for five seconds; the lookup must not wait.
        assert!(started.elapsed() < Duration::from_secs(3));
    }

    #[test]
    #[cfg(unix)]
    fn login_shell_lookup_reports_a_shell_that_cannot_run_or_start() {
        let dir = tempfile::tempdir().unwrap();
        let shell = fake_login_shell(dir.path(), "broken-shell", "exit 127");
        assert_eq!(
            lookup_in_login_shell(&shell, "claude", Duration::from_secs(10)),
            Lookup::Failed("the login shell (broken-shell) exited with status 127".to_string())
        );
        let lookup = lookup_in_login_shell(
            "/nonexistent/askhuman-shell",
            "claude",
            Duration::from_secs(10),
        );
        assert!(
            matches!(&lookup, Lookup::Failed(reason) if reason.starts_with("could not start the login shell /nonexistent/askhuman-shell")),
            "{lookup:?}"
        );
    }

    #[test]
    #[cfg(unix)]
    fn login_shell_lookup_survives_a_noisy_rc_and_lingering_children() {
        let dir = tempfile::tempdir().unwrap();
        let real = executable_file(dir.path(), "claude");
        // 4000 lines is well past a pipe buffer; the background `sleep` keeps the pipe open after
        // the shell has exited; the answer comes with an odd exit status.
        let shell = fake_login_shell(
            dir.path(),
            "noisy-shell",
            &format!(
                "(sleep 3 &)\ni=0\nwhile [ $i -lt 4000 ]; do echo 'rc noise rc noise rc noise rc noise rc noise rc noise'; i=$((i+1)); done\necho '{PATH_MARKER}/usr/bin'\necho '{BIN_MARKER}{}'\nexit 3",
                real.display()
            ),
        );
        let started = Instant::now();
        assert_eq!(
            lookup_in_login_shell(&shell, "claude", Duration::from_secs(10)),
            Lookup::Found(
                fs::canonicalize(&real)
                    .unwrap()
                    .to_string_lossy()
                    .to_string()
            )
        );
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn legacy_launch_record_defaults_to_new_mode() {
        let value = serde_json::json!({
            "id": "4f37c6d8-7397-458c-8203-65a165395dae",
            "createdAt": 1,
            "expiresAt": 2,
            "source": { "channel": "popup", "target": "" },
            "task": "continue",
            "taskSha256": "hash",
            "cwd": "/tmp",
            "kind": "claude",
            "permission": "agent-default",
            "executable": "/usr/bin/claude",
            "askhumanExecutable": "/usr/bin/AskHuman"
        });
        let record: LaunchRecord = serde_json::from_value(value).unwrap();
        assert_eq!(record.launch_mode, LaunchMode::New);
    }

    #[test]
    fn shell_quote_handles_apostrophes() {
        assert_eq!(shell_quote("/tmp/it's"), "'/tmp/it'\\''s'");
    }

    #[test]
    fn terminal_helper_command_has_sacrificial_whitespace() {
        assert_eq!(
            terminal_helper_command("/tmp/Ask Human", "launch-id"),
            "  '/tmp/Ask Human' __agent-launch 'launch-id'"
        );
    }

    #[test]
    fn windows_launchable_paths_reject_posix_npm_shims() {
        let temp = tempfile::tempdir().unwrap();
        let posix_shim = temp.path().join("codex");
        let command_shim = temp.path().join("codex.CMD");
        let executable = temp.path().join("codex.exe");
        let powershell = temp.path().join("codex.ps1");
        for path in [&posix_shim, &command_shim, &executable, &powershell] {
            fs::write(path, b"placeholder").unwrap();
        }
        assert!(!is_windows_launchable(&posix_shim));
        assert!(is_windows_launchable(&command_shim));
        assert!(is_windows_launchable(&executable));
        assert!(!is_windows_launchable(&powershell));
    }

    /// Task boundary validation happens before any filesystem/readiness side effect
    /// (spec gui-agent-task-launch §2.5 shares this path with IM /new).
    #[test]
    fn create_record_validates_task_boundaries_first() {
        let source = || LaunchSource {
            channel: "test".to_string(),
            target: String::new(),
        };
        let cwd = Path::new("/nonexistent-askhuman-test-dir");
        let err = create_record(
            source(),
            cwd,
            AgentKind::Claude,
            LaunchPermission::AgentDefault,
            "   ",
        )
        .unwrap_err();
        assert!(err.to_string().contains("must not be empty"));

        let over = "a".repeat(MAX_TASK_CHARS + 1);
        let err = create_record(
            source(),
            cwd,
            AgentKind::Claude,
            LaunchPermission::AgentDefault,
            &over,
        )
        .unwrap_err();
        assert!(err.to_string().contains("exceeds"));

        // Exactly at the limit passes length validation: with a nonexistent cwd the
        // next check (workspace resolution) fails instead, with no record written.
        let exact = "a".repeat(MAX_TASK_CHARS);
        let err = create_record(
            source(),
            cwd,
            AgentKind::Claude,
            LaunchPermission::AgentDefault,
            &exact,
        )
        .unwrap_err();
        assert!(err.to_string().contains("workspace"));
    }

    #[test]
    fn pi_rejects_permission_override_before_launch_side_effects() {
        let error = create_record(
            LaunchSource {
                channel: "test".into(),
                target: String::new(),
            },
            Path::new("/nonexistent-askhuman-test-dir"),
            AgentKind::Pi,
            LaunchPermission::Yolo,
            "task",
        )
        .unwrap_err();
        assert!(error.to_string().contains("built-in permission mode"));
    }

    #[test]
    fn a_bad_model_choice_is_rejected_before_any_side_effect() {
        let attempt = |kind, choice: &ModelChoice| {
            create_record_with_choice(
                LaunchSource {
                    channel: "test".into(),
                    target: String::new(),
                },
                Path::new("/nonexistent-askhuman-test-dir"),
                kind,
                LaunchPermission::AgentDefault,
                "task",
                &[],
                &[],
                choice,
            )
            .unwrap_err()
            .to_string()
        };
        assert!(
            attempt(AgentKind::Claude, &choice(Some("--yolo"), None)).contains("invalid model id")
        );
        assert!(attempt(AgentKind::Codex, &choice(Some("a b"), None)).contains("invalid model id"));
        assert!(attempt(AgentKind::Claude, &choice(None, Some("HIGH")))
            .contains("invalid reasoning effort"));
        assert!(attempt(AgentKind::Cursor, &choice(None, Some("high")))
            .contains("no separate reasoning effort"));
        // A well-formed choice gets as far as the next check: the workspace does not exist.
        assert!(
            attempt(AgentKind::Claude, &choice(Some("opus"), Some("high"))).contains("workspace")
        );
        assert!(attempt(AgentKind::Cursor, &choice(Some("auto"), None)).contains("workspace"));
    }

    #[test]
    fn the_payload_hash_covers_a_choice_only_when_there_is_one() {
        let files = vec!["/tmp/a b.md".to_string()];
        let warnings = vec!["warning".to_string()];
        let hash = |mode: &LaunchMode, choice: &ModelChoice| {
            payload_sha256("task", &files, &warnings, mode, choice)
        };

        // No choice: byte-for-byte what it was before model and effort existed.
        let before =
            sha256(&serde_json::to_vec(&("task", &files, &warnings, &LaunchMode::New)).unwrap());
        assert_eq!(hash(&LaunchMode::New, &ModelChoice::default()), before);

        let model = hash(&LaunchMode::New, &choice(Some("a"), None));
        let effort = hash(&LaunchMode::New, &choice(None, Some("a")));
        let both = hash(&LaunchMode::New, &choice(Some("a"), Some("high")));
        let other = hash(&LaunchMode::New, &choice(Some("b"), Some("high")));
        let all = [&before, &model, &effort, &both, &other];
        for (i, a) in all.iter().enumerate() {
            for b in &all[i + 1..] {
                assert_ne!(a, b);
            }
        }
    }

    /// A claimable record for `kind` in `dir` whose executable is a file nobody ever runs.
    #[cfg(unix)]
    fn stub_record(dir: &Path, kind: AgentKind, choice: &ModelChoice) -> LaunchRecord {
        use std::os::unix::fs::PermissionsExt;
        let executable = dir.join("never-run-agent");
        fs::write(&executable, b"#!/bin/sh\nexit 1\n").unwrap();
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o755)).unwrap();
        let task = "write a haiku";
        let created_at = epoch_secs();
        LaunchRecord {
            id: uuid::Uuid::new_v4().to_string(),
            created_at,
            expires_at: created_at + RECORD_TTL_SECS,
            source: LaunchSource {
                channel: "test".into(),
                target: String::new(),
            },
            task: task.into(),
            task_sha256: sha256(task.as_bytes()),
            files: Vec::new(),
            warnings: Vec::new(),
            payload_sha256: payload_sha256(task, &[], &[], &LaunchMode::New, choice),
            cwd: fs::canonicalize(dir).unwrap().to_string_lossy().into(),
            kind,
            permission: LaunchPermission::AgentDefault,
            executable: fs::canonicalize(&executable)
                .unwrap()
                .to_string_lossy()
                .into(),
            askhuman_executable: std::env::current_exe().unwrap().to_string_lossy().into(),
            launch_mode: LaunchMode::New,
            model: choice.model.clone(),
            effort: choice.effort.clone(),
        }
    }

    #[cfg(unix)]
    #[test]
    fn the_helper_builds_the_command_for_the_recorded_choice() {
        let dir = tempfile::tempdir().unwrap();
        let record = stub_record(
            dir.path(),
            AgentKind::Codex,
            &choice(Some("model-a"), Some("high")),
        );
        validate_claim(&record, &record.id).unwrap();

        let command = assemble_command(&record);
        assert_eq!(command.get_program(), record.executable.as_str());
        let args: Vec<_> = command.get_args().collect();
        assert_eq!(
            args,
            [
                "--model",
                "model-a",
                "-c",
                "model_reasoning_effort=\"high\"",
                "--",
                "write a haiku"
            ]
        );
        assert!(command.get_envs().any(|(key, value)| {
            key == LAUNCH_ID_ENV && value == Some(std::ffi::OsStr::new(&record.id))
        }));

        // Without a choice the command is the plain one.
        let plain = stub_record(dir.path(), AgentKind::Claude, &ModelChoice::default());
        validate_claim(&plain, &plain.id).unwrap();
        assert_eq!(
            assemble_command(&plain).get_args().collect::<Vec<_>>(),
            ["write a haiku"]
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_record_that_no_longer_matches_its_hash_or_names_a_bad_choice_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let claim = |record: &LaunchRecord| validate_claim(record, &record.id);
        let chosen = choice(Some("model-a"), Some("high"));

        // Edited after the hash was taken.
        let mut edited = stub_record(dir.path(), AgentKind::Claude, &chosen);
        edited.model = Some("model-b".into());
        assert!(claim(&edited).unwrap_err().to_string().contains("hash"));
        let mut stripped = stub_record(dir.path(), AgentKind::Claude, &chosen);
        stripped.effort = None;
        assert!(claim(&stripped).unwrap_err().to_string().contains("hash"));

        // The pre-launch-mode hash vouches for a plain task only.
        let mut legacy = stub_record(dir.path(), AgentKind::Claude, &ModelChoice::default());
        legacy.payload_sha256 =
            legacy_payload_sha256(&legacy.task, &legacy.files, &legacy.warnings);
        claim(&legacy).unwrap();
        legacy.model = Some("model-a".into());
        assert!(claim(&legacy).unwrap_err().to_string().contains("hash"));

        // A hash that matches does not make a malformed choice acceptable.
        for (kind, bad) in [
            (AgentKind::Claude, choice(Some("--yolo"), None)),
            (AgentKind::Codex, choice(Some("a b"), None)),
            (AgentKind::Grok, choice(None, Some("-x"))),
            (AgentKind::Cursor, choice(None, Some("high"))),
        ] {
            let record = stub_record(dir.path(), kind, &bad);
            assert!(
                claim(&record)
                    .unwrap_err()
                    .to_string()
                    .contains("invalid model choice"),
                "{kind:?} {bad:?}"
            );
        }

        // A continued session keeps its own model, whatever the record says.
        let mut fork = stub_record(dir.path(), AgentKind::Claude, &chosen);
        fork.launch_mode = LaunchMode::Fork {
            source_session_id: "source-123".into(),
        };
        fork.payload_sha256 = payload_sha256(&fork.task, &[], &[], &fork.launch_mode, &chosen);
        assert!(claim(&fork)
            .unwrap_err()
            .to_string()
            .contains("invalid model choice"));
    }

    #[cfg(unix)]
    #[test]
    fn launch_records_with_and_without_a_choice_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let plain = stub_record(dir.path(), AgentKind::Claude, &ModelChoice::default());
        let mut value = serde_json::to_value(&plain).unwrap();
        // The fields are absent when unused, so a record is what an older build wrote.
        assert!(value.get("model").is_none() && value.get("effort").is_none());
        let back: LaunchRecord = serde_json::from_value(value.clone()).unwrap();
        assert!(back.choice().is_default());

        let chosen = stub_record(
            dir.path(),
            AgentKind::Claude,
            &choice(Some("model-a"), Some("high")),
        );
        value = serde_json::to_value(&chosen).unwrap();
        assert_eq!(value["model"], "model-a");
        assert_eq!(value["effort"], "high");
        let back: LaunchRecord = serde_json::from_value(value).unwrap();
        assert_eq!(back.choice(), choice(Some("model-a"), Some("high")));
    }

    #[test]
    fn task_hash_is_stable() {
        assert_eq!(
            sha256(b"hello"),
            "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
        );
    }

    #[test]
    fn attachment_prompt_quotes_paths_and_reports_files_that_disappear() {
        let temp = tempfile::tempdir().unwrap();
        let existing = temp.path().join("a file.md");
        fs::write(&existing, b"x").unwrap();
        let missing = temp.path().join("gone.md");
        let prompt = task_with_attachments(
            "review",
            &[
                existing.to_string_lossy().into_owned(),
                missing.to_string_lossy().into_owned(),
            ],
            &["Earlier warning".into()],
        );
        assert!(prompt.contains("Attachments (local file paths):"));
        assert!(prompt.contains(&serde_json::to_string(&existing).unwrap()));
        assert!(prompt.contains("Earlier warning"));
        assert!(prompt.contains("Attachment became unavailable"));
    }

    #[test]
    fn readiness_ignores_prompt_and_guard_freshness_but_requires_transport() {
        assert!(!integration_unavailable_from(
            agent_mode::Mode::Cli,
            true,
            true,
            true,
            false,
            false,
            false,
        ));
        assert!(!integration_unavailable_from(
            agent_mode::Mode::Mcp,
            true,
            false,
            false,
            false,
            true,
            false,
        ));
        assert!(integration_unavailable_from(
            agent_mode::Mode::Cli,
            true,
            true,
            false,
            false,
            false,
            false,
        ));
        assert!(integration_unavailable_from(
            agent_mode::Mode::Mcp,
            true,
            false,
            false,
            false,
            true,
            true,
        ));
    }

    /// Local installation contract probe. Ignored in CI because the Codex binary/integration is
    /// optional; run explicitly when diagnosing a reported `forkReady=false`.
    #[test]
    #[ignore]
    fn real_codex_fork_help_when_available() {
        let executable = lookup_executable("codex")
            .executable()
            .expect("Codex is not installed")
            .to_string();
        let help =
            probe_help(&executable, &["fork", "--help"]).expect("Codex fork help probe failed");
        assert!(help.contains("SESSION_ID"), "{help}");
    }
}
