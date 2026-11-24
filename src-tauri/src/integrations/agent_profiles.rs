//! Extra config directory ("multi-profile") management for all 6 integrated agents
//! (spec `docs/specs/t3-code-and-multi-profile.md`, L2).
//!
//! Primary integration directories (`~/.gemini`, `~/.codex`, `~/.claude`, `~/.cursor`, `~/.grok`,
//! `~/.pi/agent`) remain the canonical owner of each agent's integration mode (`None / Cli / Mcp`)
//! and capability preferences. Users may additionally enroll extra config directories — either
//! discovered from T3 Code's isolated Antigravity homes (`~/.t3/userdata/providers/antigravity/*`)
//! or added manually for any agent (`CODEX_HOME`, `CLAUDE_CONFIG_DIR`, `GROK_HOME`,
//! `PI_CODING_AGENT_DIR`, `CURSOR_CONFIG_DIR`, `GEMINI_HOME`).
//!
//! Invariants:
//! - AskHuman never writes into a discovered directory until the user explicitly opts in (D3).
//! - Missing extra profile roots are never recreated (`mkdir -p` is refused; reported as
//!   `rootMissing`, P2).
//! - Primary is reconciled first; each enabled extra profile is reconciled independently so a
//!   missing or conflicting extra profile never blocks primary or other healthy profiles (P4).
//! - Switching to `Mode::None` uninstalls managed artifacts from primary and enabled extra
//!   profiles while retaining the saved profile list (P3).
//! - Dev test profiles (`development::session::directory_for`) stay completely isolated and never
//!   read or mutate extra profiles.

use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::agent_mode::{self, Mode};
use super::agent_rules::{self, AgentTarget, Variant};
use super::{
    agent_ask_question, agent_lifecycle, agent_permission, agent_stop, antigravity_plugin,
    codex_protocol, hook_edit, mutation_lock,
};
use crate::agents::AgentKind;
use crate::paths;

const STORE_VERSION: u32 = 1;
const MAX_DISCOVERY_ENTRIES: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProfileSource {
    T3Discovered,
    Manual,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileEntry {
    pub id: String,
    pub agent: String,
    pub root: String,
    pub label: String,
    pub source: ProfileSource,
    pub enabled: bool,
    #[serde(default)]
    pub created_at: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DismissedEntry {
    pub agent: String,
    pub root: String,
    #[serde(default)]
    pub dismissed_at: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Store {
    #[serde(default = "default_version")]
    version: u32,
    #[serde(default)]
    profiles: Vec<ProfileEntry>,
    #[serde(default)]
    dismissed: Vec<DismissedEntry>,
}

fn default_version() -> u32 {
    STORE_VERSION
}

/// Un-enrolled isolated directory discovered on disk (currently T3 Code Antigravity homes).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveredProfile {
    pub agent: String,
    pub root: String,
    #[serde(default)]
    pub display_root: String,
    pub label: String,
    pub source: ProfileSource,
}

/// Live status of one configured extra profile for Settings UI, CLI, and `doctor`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtraProfileStatus {
    pub id: String,
    pub agent: String,
    pub root: String,
    #[serde(default)]
    pub display_root: String,
    pub label: String,
    pub source: ProfileSource,
    pub enabled: bool,
    /// `"synced" | "outdated" | "notInstalled" | "conflict" | "rootMissing" | "disabled"`
    pub state: String,
    /// Observed on-disk mode in this profile (`"none" | "cli" | "mcp"`).
    pub mode: String,
    pub needs_update: bool,
    pub error: Option<String>,
    pub host_plugin_disabled: bool,
    pub mcp_grant: Option<bool>,
}

#[derive(Debug, Clone)]
pub struct ProfileFailure {
    pub id: String,
    pub label: String,
    pub root: String,
    pub error: String,
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn dev_isolated(kind: AgentKind) -> bool {
    crate::development::session::directory_for(kind).is_some()
}

pub(crate) fn kind_for_target(target: AgentTarget) -> AgentKind {
    agent_mode::kind_for_target(target)
}

/// Primary configuration root of `kind` under the user's integration home.
pub fn primary_root(kind: AgentKind) -> PathBuf {
    let home = paths::integration_home();
    let raw = match kind {
        AgentKind::Cursor => home.join(".cursor"),
        AgentKind::Claude => home.join(".claude"),
        AgentKind::Codex => home.join(".codex"),
        AgentKind::Grok => home.join(".grok"),
        AgentKind::Pi => home.join(".pi").join("agent"),
        AgentKind::Antigravity => home.join(".gemini"),
    };
    agent_rules::physical_root(raw)
}

/// Deterministic, filename-safe ID for `(kind, canonical_root)`.
pub fn profile_id(kind: AgentKind, canonical_root: &Path) -> String {
    let mut hasher = Sha256::new();
    hasher.update(kind.as_str().as_bytes());
    hasher.update(b":");
    hasher.update(canonical_root.to_string_lossy().as_bytes());
    let hex = format!("{:x}", hasher.finalize());
    format!("{}-{}", kind.as_str(), &hex[..16])
}

fn strip_antigravity_config_suffix(kind: AgentKind, path: PathBuf) -> PathBuf {
    // P1: If the user pasted `<gemini_root>/config` for Antigravity, normalize to `<gemini_root>`.
    if kind == AgentKind::Antigravity && path.file_name().and_then(|n| n.to_str()) == Some("config")
    {
        if let Some(parent) = path.parent() {
            let looks_like_gemini_root = parent.join("antigravity-acp").is_dir()
                || parent.join("antigravity").is_dir()
                || parent.join("antigravity-cli").is_dir()
                || parent
                    .parent()
                    .and_then(|gp| gp.file_name())
                    .and_then(|n| n.to_str())
                    == Some("antigravity");
            if looks_like_gemini_root {
                return parent.to_path_buf();
            }
        }
    }
    path
}

fn normalize_lookup_path(kind: AgentKind, raw: &str) -> PathBuf {
    let expanded = expand_tilde(raw.trim());
    let physical = agent_rules::physical_root(expanded);
    strip_antigravity_config_suffix(kind, physical)
}

/// Normalize and validate an extra profile root before enrollment.
pub fn normalize_root_for_add(kind: AgentKind, raw: &Path) -> Result<PathBuf> {
    if kind == AgentKind::Antigravity && !cfg!(unix) {
        bail!("Antigravity integration is only supported on Unix platforms");
    }
    let raw_str = raw.to_string_lossy();
    if raw_str.trim().is_empty() || raw_str.contains('\0') {
        bail!("profile directory path cannot be empty");
    }
    let expanded = expand_tilde(raw_str.trim());
    if !expanded.is_absolute() {
        bail!(
            "profile directory must be an absolute path: {}",
            expanded.display()
        );
    }
    if expanded
        .components()
        .any(|c| matches!(c, Component::ParentDir))
    {
        bail!(
            "profile directory must not contain '..': {}",
            expanded.display()
        );
    }
    if !expanded.is_dir() {
        bail!("profile directory does not exist: {}", expanded.display());
    }

    let normalized = strip_antigravity_config_suffix(kind, agent_rules::physical_root(expanded));

    let primary = primary_root(kind);
    if crate::path_identity::equivalent(&normalized.to_string_lossy(), &primary.to_string_lossy()) {
        bail!(
            "{} is already the primary directory for {}",
            normalized.display(),
            kind.label()
        );
    }

    Ok(normalized)
}

fn expand_tilde(raw: &str) -> PathBuf {
    if raw == "~" {
        return paths::integration_home();
    }
    if let Some(rest) = raw.strip_prefix("~/").or_else(|| raw.strip_prefix("~\\")) {
        return paths::integration_home().join(rest);
    }
    PathBuf::from(raw)
}

fn default_label(kind: AgentKind, root: &Path, source: ProfileSource) -> String {
    let tail = root
        .file_name()
        .and_then(|n| n.to_str())
        .filter(|s| !s.is_empty())
        .unwrap_or(kind.as_str());
    match source {
        ProfileSource::T3Discovered => {
            let short: String = tail.chars().take(8).collect();
            format!("T3 Code ({short})")
        }
        ProfileSource::Manual => tail.to_string(),
    }
}

fn load_store_from(path: &Path) -> Result<Store> {
    match std::fs::read(path) {
        Ok(bytes) => {
            let store: Store = serde_json::from_slice(&bytes)
                .with_context(|| format!("failed to parse {}", path.display()))?;
            Ok(store)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Store {
            version: STORE_VERSION,
            profiles: Vec::new(),
            dismissed: Vec::new(),
        }),
        Err(error) => Err(error).with_context(|| format!("failed to read {}", path.display())),
    }
}

fn load_store_lenient() -> Store {
    load_store_from(&paths::agent_profiles_file()).unwrap_or_else(|_| Store {
        version: STORE_VERSION,
        profiles: Vec::new(),
        dismissed: Vec::new(),
    })
}

fn save_store_to(path: &Path, store: &Store) -> Result<()> {
    let mut bytes = serde_json::to_vec_pretty(store)?;
    bytes.push(b'\n');
    hook_edit::atomic_write(path, &bytes)
}

fn same_root(a: &str, b: &str) -> bool {
    crate::path_identity::equivalent(a, b)
}

#[cfg(unix)]
fn is_user_owned_real_dir(path: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    let Ok(meta) = std::fs::symlink_metadata(path) else {
        return false;
    };
    if meta.file_type().is_symlink() || !meta.is_dir() {
        return false;
    }
    // SAFETY: `geteuid` has no preconditions and cannot fail.
    meta.uid() == unsafe { libc::geteuid() }
}

#[cfg(not(unix))]
fn is_user_owned_real_dir(_path: &Path) -> bool {
    false
}

/// Read-only scan for un-enrolled T3 Code Antigravity provider directories.
pub fn discover_for_agent(kind: AgentKind) -> Vec<DiscoveredProfile> {
    if kind != AgentKind::Antigravity || !cfg!(unix) || dev_isolated(kind) {
        return Vec::new();
    }
    let store = load_store_lenient();
    discover_t3_antigravity_in(&paths::integration_home(), &store)
}

const MAX_T3_SETTINGS_BYTES: u64 = 256 * 1024;

fn sha256_hex(input: &str) -> String {
    format!("{:x}", Sha256::digest(input.as_bytes()))
}

fn collapse_home_in(home: &Path, path: &Path) -> String {
    if let Ok(rest) = path.strip_prefix(home) {
        return if rest.as_os_str().is_empty() {
            "~".to_string()
        } else {
            format!("~/{}", rest.display())
        };
    }
    let physical_home = agent_rules::physical_root(home.to_path_buf());
    if physical_home != home {
        if let Ok(rest) = path.strip_prefix(&physical_home) {
            return if rest.as_os_str().is_empty() {
                "~".to_string()
            } else {
                format!("~/{}", rest.display())
            };
        }
    }
    path.display().to_string()
}

fn display_root_for(root: &str) -> String {
    collapse_home_in(&paths::integration_home(), Path::new(root))
}

/// Best-effort read-only reverse lookup of T3 Code Antigravity provider instance hashes.
///
/// T3 Code derives `~/.t3/userdata/providers/antigravity/<sha256(instanceId)>` per configured
/// provider instance (not per session). Hashing known `providerInstances` keys from
/// `~/.t3/userdata/settings.json` (plus the built-in `"antigravity"` instance) lets us display
/// `T3 Code (antigravity)` instead of an opaque hex prefix.
fn t3_instance_names_by_hash(home: &Path) -> BTreeMap<String, String> {
    let mut map = BTreeMap::new();
    map.insert(sha256_hex("antigravity"), "antigravity".to_string());

    let settings_path = home.join(".t3").join("userdata").join("settings.json");
    let Ok(meta) = std::fs::metadata(&settings_path) else {
        return map;
    };
    if !meta.is_file() || meta.len() > MAX_T3_SETTINGS_BYTES {
        return map;
    }
    let Ok(bytes) = std::fs::read(&settings_path) else {
        return map;
    };
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
        return map;
    };
    if let Some(instances) = value.get("providerInstances").and_then(|v| v.as_object()) {
        for (instance_id, cfg) in instances {
            let trimmed = instance_id.trim();
            if trimmed.is_empty() {
                continue;
            }
            let driver = cfg.get("driver").and_then(|v| v.as_str()).unwrap_or("");
            if driver.is_empty() || driver == "antigravity" {
                map.insert(sha256_hex(trimmed), trimmed.to_string());
            }
        }
    }
    map
}

fn discover_t3_antigravity_in(home: &Path, store: &Store) -> Vec<DiscoveredProfile> {
    if !cfg!(unix) {
        return Vec::new();
    }
    let providers = paths::t3_antigravity_providers_dir_in(home);
    let Ok(entries) = std::fs::read_dir(&providers) else {
        return Vec::new();
    };
    let known_names = t3_instance_names_by_hash(home);
    let mut discovered = Vec::new();
    for entry in entries.flatten().take(MAX_DISCOVERY_ENTRIES) {
        let candidate = entry.path();
        if !is_user_owned_real_dir(&candidate) {
            continue;
        }
        if !candidate.join("config").is_dir() && !candidate.join("antigravity-acp").is_dir() {
            continue;
        }
        let physical = agent_rules::physical_root(candidate);
        let root_str = physical.to_string_lossy().to_string();
        let already_known =
            store.profiles.iter().any(|p| {
                p.agent == AgentKind::Antigravity.as_str() && same_root(&p.root, &root_str)
            }) || store.dismissed.iter().any(|d| {
                d.agent == AgentKind::Antigravity.as_str() && same_root(&d.root, &root_str)
            });
        if already_known {
            continue;
        }
        let label = physical
            .file_name()
            .and_then(|n| n.to_str())
            .and_then(|dir_name| known_names.get(dir_name))
            .map(|instance_id| format!("T3 Code ({instance_id})"))
            .unwrap_or_else(|| {
                default_label(
                    AgentKind::Antigravity,
                    &physical,
                    ProfileSource::T3Discovered,
                )
            });
        discovered.push(DiscoveredProfile {
            agent: AgentKind::Antigravity.as_str().to_string(),
            display_root: collapse_home_in(home, &physical),
            label,
            root: root_str,
            source: ProfileSource::T3Discovered,
        });
    }
    discovered.sort_by(|a, b| a.root.cmp(&b.root));
    discovered
}

/// Enabled extra profile roots for `kind` (empty when inside a Dev test profile).
pub fn enabled_roots(kind: AgentKind) -> Vec<PathBuf> {
    if dev_isolated(kind) {
        return Vec::new();
    }
    load_store_lenient()
        .profiles
        .into_iter()
        .filter(|p| p.enabled && p.agent == kind.as_str())
        .map(|p| PathBuf::from(p.root))
        .filter(|p| p.is_dir())
        .collect()
}

/// Return status of all configured extra profiles for `target` relative to `primary_mode`.
pub fn statuses_for_agent(target: AgentTarget, primary_mode: Mode) -> Vec<ExtraProfileStatus> {
    let kind = kind_for_target(target);
    if dev_isolated(kind) {
        return Vec::new();
    }
    load_store_lenient()
        .profiles
        .into_iter()
        .filter(|p| p.agent == kind.as_str())
        .map(|entry| inspect_profile(&entry, target, primary_mode))
        .collect()
}

fn inspect_profile(
    entry: &ProfileEntry,
    target: AgentTarget,
    primary_mode: Mode,
) -> ExtraProfileStatus {
    let kind = kind_for_target(target);
    let root = Path::new(&entry.root);

    let display_root = display_root_for(&entry.root);

    if !entry.enabled {
        return ExtraProfileStatus {
            id: entry.id.clone(),
            agent: entry.agent.clone(),
            root: entry.root.clone(),
            display_root,
            label: entry.label.clone(),
            source: entry.source,
            enabled: false,
            state: "disabled".to_string(),
            mode: "none".to_string(),
            needs_update: false,
            error: None,
            host_plugin_disabled: false,
            mcp_grant: None,
        };
    }

    if !root.is_dir() {
        return ExtraProfileStatus {
            id: entry.id.clone(),
            agent: entry.agent.clone(),
            root: entry.root.clone(),
            display_root,
            label: entry.label.clone(),
            source: entry.source,
            enabled: true,
            state: "rootMissing".to_string(),
            mode: "none".to_string(),
            needs_update: false,
            error: Some(format!("directory does not exist: {}", root.display())),
            host_plugin_disabled: false,
            mcp_grant: None,
        };
    }

    if target == AgentTarget::Antigravity {
        return inspect_antigravity_profile(entry, root, primary_mode);
    }

    paths::with_agent_root(kind, root, || {
        let disk_mode = agent_mode::current(target);
        let updates = agent_mode::artifact_updates(target);
        let stop = agent_stop::status(kind);
        let ask_question = agent_ask_question::status(kind);
        let permission = agent_permission::status(target);
        let lifecycle = agent_lifecycle::status_for_mode(kind, primary_mode);

        let any_outdated = updates.rule
            || updates.hook
            || updates.mcp
            || (stop.supported && stop.outdated)
            || (ask_question.supported && ask_question.outdated)
            || (permission.supported && permission.needs_update)
            || lifecycle.needs_update;

        let (state, needs_update) = if disk_mode != primary_mode {
            if disk_mode == Mode::None && primary_mode != Mode::None {
                ("notInstalled", true)
            } else {
                ("outdated", true)
            }
        } else if any_outdated {
            ("outdated", true)
        } else {
            ("synced", false)
        };

        ExtraProfileStatus {
            id: entry.id.clone(),
            agent: entry.agent.clone(),
            root: entry.root.clone(),
            display_root: display_root.clone(),
            label: entry.label.clone(),
            source: entry.source,
            enabled: true,
            state: state.to_string(),
            mode: disk_mode.as_str().to_string(),
            needs_update,
            error: None,
            host_plugin_disabled: false,
            mcp_grant: None,
        }
    })
}

fn inspect_antigravity_profile(
    entry: &ProfileEntry,
    root: &Path,
    primary_mode: Mode,
) -> ExtraProfileStatus {
    let display_root = display_root_for(&entry.root);
    let snapshot = match antigravity_plugin::snapshot_for_root(root, &entry.id) {
        Ok(snapshot) => snapshot,
        Err(error) => {
            return ExtraProfileStatus {
                id: entry.id.clone(),
                agent: entry.agent.clone(),
                root: entry.root.clone(),
                display_root,
                label: entry.label.clone(),
                source: entry.source,
                enabled: true,
                state: "conflict".to_string(),
                mode: "none".to_string(),
                needs_update: false,
                error: Some(error.to_string()),
                host_plugin_disabled: false,
                mcp_grant: None,
            };
        }
    };

    let disk_mode = match snapshot.variant {
        Some(Variant::Cli) => Mode::Cli,
        Some(Variant::Mcp) => Mode::Mcp,
        None => Mode::None,
    };
    let host = (primary_mode != Mode::None && snapshot.present())
        .then(|| antigravity_plugin::host_view_for_root(root));

    let desired_lifecycle = agent_lifecycle::tracking_wanted(AgentKind::Antigravity);
    let desired_stop = agent_stop::active_in_mode(AgentKind::Antigravity, primary_mode);

    let (state, needs_update, error) = match snapshot.state {
        antigravity_plugin::State::Conflict => (
            "conflict",
            false,
            snapshot
                .reason
                .or_else(|| Some("plugin directory conflict".to_string())),
        ),
        antigravity_plugin::State::Missing => {
            if primary_mode == Mode::None {
                ("synced", false, None)
            } else {
                ("notInstalled", true, None)
            }
        }
        antigravity_plugin::State::OwnedCurrent => {
            if primary_mode == Mode::None {
                ("outdated", true, None)
            } else if disk_mode == primary_mode
                && snapshot.lifecycle == desired_lifecycle
                && snapshot.stop == desired_stop
            {
                ("synced", false, None)
            } else {
                ("outdated", true, None)
            }
        }
        antigravity_plugin::State::OwnedOutdated | antigravity_plugin::State::NeedsRepair => {
            ("outdated", true, None)
        }
    };

    ExtraProfileStatus {
        id: entry.id.clone(),
        agent: entry.agent.clone(),
        root: entry.root.clone(),
        display_root,
        label: entry.label.clone(),
        source: entry.source,
        enabled: true,
        state: state.to_string(),
        mode: disk_mode.as_str().to_string(),
        needs_update,
        error,
        host_plugin_disabled: host.is_some_and(|h| h.plugin_disabled),
        mcp_grant: host
            .filter(|_| primary_mode == Mode::Mcp)
            .map(|h| h.mcp_allowed),
    }
}

/// Reconcile a single extra profile to `mode` (and current shared capability preferences).
/// Caller holds `IntegrationMutationLock`. Refuses to create a missing root directory (P2).
pub(crate) fn reconcile_one_unlocked(
    entry: &ProfileEntry,
    target: AgentTarget,
    mode: Mode,
    env: &BTreeMap<String, String>,
) -> Result<()> {
    let kind = kind_for_target(target);
    let root = Path::new(&entry.root);
    if !root.is_dir() {
        bail!(
            "profile directory does not exist (refusing to recreate): {}",
            root.display()
        );
    }

    if target == AgentTarget::Antigravity {
        return match mode {
            Mode::None => antigravity_plugin::uninstall_for_root(root, &entry.id),
            Mode::Cli | Mode::Mcp => antigravity_plugin::install_for_root(
                root,
                &entry.id,
                antigravity_plugin::Request {
                    variant: if mode == Mode::Mcp {
                        Variant::Mcp
                    } else {
                        Variant::Cli
                    },
                    lifecycle: agent_lifecycle::tracking_wanted(AgentKind::Antigravity),
                    stop: agent_stop::active_in_mode(AgentKind::Antigravity, mode),
                },
            ),
        };
    }

    if target == AgentTarget::Codex && mode != Mode::None {
        codex_protocol::ensure_supported()?;
    }

    paths::with_agent_root(kind, root, || {
        agent_mode::set_single_unlocked_with_env(target, mode, env)
    })
}

/// Reconcile every enabled extra profile for `target` independently (P4: partial-failure isolation).
/// Caller holds `IntegrationMutationLock`.
pub(crate) fn reconcile_enabled_unlocked(
    target: AgentTarget,
    mode: Mode,
    env: &BTreeMap<String, String>,
) -> Result<()> {
    let kind = kind_for_target(target);
    if dev_isolated(kind) {
        return Ok(());
    }
    let store = load_store_lenient();
    let mut failures = Vec::new();
    for entry in store
        .profiles
        .iter()
        .filter(|p| p.enabled && p.agent == kind.as_str())
    {
        if let Err(error) = reconcile_one_unlocked(entry, target, mode, env) {
            failures.push(ProfileFailure {
                id: entry.id.clone(),
                label: entry.label.clone(),
                root: entry.root.clone(),
                error: format!("{error:#}"),
            });
        }
    }
    format_failures(&failures)
}

/// Best-effort reconciliation used only by startup migrations (`agent_stop::migrate_outdated`,
/// `agent_ask_question::migrate_outdated`). Never errors or touches missing/conflicting roots.
pub(crate) fn reconcile_outdated_best_effort_unlocked(target: AgentTarget) {
    let kind = kind_for_target(target);
    if dev_isolated(kind) {
        return;
    }
    let primary_mode = primary_mode_for(target).unwrap_or(Mode::None);
    let store = load_store_lenient();
    for entry in store
        .profiles
        .iter()
        .filter(|p| p.enabled && p.agent == kind.as_str())
    {
        let status = inspect_profile(entry, target, primary_mode);
        if status.needs_update && Path::new(&entry.root).is_dir() {
            let _ = reconcile_one_unlocked(entry, target, primary_mode, &BTreeMap::new());
        }
    }
}

fn format_failures(failures: &[ProfileFailure]) -> Result<()> {
    if failures.is_empty() {
        return Ok(());
    }
    let lines: Vec<String> = failures
        .iter()
        .map(|f| format!("[{}] {} ({}): {}", f.id, f.label, f.root, f.error))
        .collect();
    Err(anyhow!(
        "primary succeeded, but {} extra profile(s) failed:\n  {}",
        failures.len(),
        lines.join("\n  ")
    ))
}

fn primary_mode_for(target: AgentTarget) -> Result<Mode> {
    if target == AgentTarget::Antigravity {
        Ok(agent_mode::antigravity_status()?.0)
    } else {
        Ok(agent_mode::current(target))
    }
}

fn find_entry_index(store: &Store, kind: AgentKind, id_or_path: &str) -> Option<usize> {
    let needle = id_or_path.trim();
    if let Some(idx) = store
        .profiles
        .iter()
        .position(|p| p.agent == kind.as_str() && p.id == needle)
    {
        return Some(idx);
    }
    let candidate = normalize_lookup_path(kind, needle);
    let candidate_str = candidate.to_string_lossy().to_string();
    store
        .profiles
        .iter()
        .position(|p| p.agent == kind.as_str() && same_root(&p.root, &candidate_str))
}

/// Add a new extra profile directory. If `enable` is true and the agent is currently integrated
/// (`Mode::Cli | Mode::Mcp`), reconciles the profile immediately.
pub fn add(
    target: AgentTarget,
    raw_root: &Path,
    label: Option<&str>,
    source: ProfileSource,
    enable: bool,
) -> Result<ExtraProfileStatus> {
    let kind = kind_for_target(target);
    let _lock = mutation_lock::IntegrationMutationLock::acquire()?;
    let root = normalize_root_for_add(kind, raw_root)?;
    let root_str = root.to_string_lossy().to_string();

    let store_path = paths::agent_profiles_file();
    let mut store = load_store_from(&store_path)?;

    // If this root already exists, update its label/enabled state only when explicitly called via
    // enable/add of a previously disabled entry; otherwise reject duplicates clearly.
    if store
        .profiles
        .iter()
        .any(|p| p.agent == kind.as_str() && same_root(&p.root, &root_str))
    {
        bail!(
            "profile directory is already added for {}: {}",
            kind.label(),
            root.display()
        );
    }

    let id = profile_id(kind, &root);
    let label = label
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| default_label(kind, &root, source));

    let entry = ProfileEntry {
        id,
        agent: kind.as_str().to_string(),
        root: root_str.clone(),
        label,
        source,
        enabled: enable,
        created_at: now_secs(),
    };

    let primary_mode = primary_mode_for(target)?;
    if enable && primary_mode != Mode::None {
        reconcile_one_unlocked(&entry, target, primary_mode, &BTreeMap::new())?;
    }

    // Re-enrolling a previously dismissed root clears the dismissal.
    store
        .dismissed
        .retain(|d| !(d.agent == kind.as_str() && same_root(&d.root, &root_str)));
    store.profiles.push(entry.clone());
    save_store_to(&store_path, &store)?;

    Ok(inspect_profile(&entry, target, primary_mode))
}

/// Enable or disable an existing extra profile (or enable a discovered T3 profile by path).
/// Disabling uninstalls AskHuman-managed artifacts from that profile; on conflict, refuses unless
/// `detach == true` (which removes only AskHuman's ledger without touching disk files).
pub fn set_enabled(
    target: AgentTarget,
    id_or_path: &str,
    enabled: bool,
    detach: bool,
) -> Result<ExtraProfileStatus> {
    let kind = kind_for_target(target);
    let _lock = mutation_lock::IntegrationMutationLock::acquire()?;
    let store_path = paths::agent_profiles_file();
    let mut store = load_store_from(&store_path)?;
    let primary_mode = primary_mode_for(target)?;

    let Some(idx) = find_entry_index(&store, kind, id_or_path) else {
        // Convenience: `agents profile enable antigravity <discovered_path>` enrolls + enables it.
        if enabled {
            let candidate = normalize_lookup_path(kind, id_or_path);
            let discovered = discover_t3_antigravity_in(&paths::integration_home(), &store);
            if let Some(found) = discovered.into_iter().find(|d| {
                d.agent == kind.as_str() && same_root(&d.root, &candidate.to_string_lossy())
            }) {
                drop(_lock);
                return add(
                    target,
                    Path::new(&found.root),
                    Some(&found.label),
                    ProfileSource::T3Discovered,
                    true,
                );
            }
        }
        bail!(
            "extra profile not found for {}: {}",
            kind.label(),
            id_or_path
        );
    };

    let entry = store.profiles[idx].clone();
    if enabled {
        if !Path::new(&entry.root).is_dir() {
            bail!("profile directory does not exist: {}", entry.root);
        }
        if primary_mode != Mode::None {
            reconcile_one_unlocked(&entry, target, primary_mode, &BTreeMap::new())?;
        }
        store.profiles[idx].enabled = true;
    } else {
        cleanup_profile_unlocked(&entry, target, detach)?;
        store.profiles[idx].enabled = false;
    }

    let updated = store.profiles[idx].clone();
    save_store_to(&store_path, &store)?;
    Ok(inspect_profile(&updated, target, primary_mode))
}

/// Uninstall (or detach when `detach == true`) and delete an extra profile entry.
pub fn remove(target: AgentTarget, id_or_path: &str, detach: bool) -> Result<()> {
    let kind = kind_for_target(target);
    let _lock = mutation_lock::IntegrationMutationLock::acquire()?;
    let store_path = paths::agent_profiles_file();
    let mut store = load_store_from(&store_path)?;
    let idx = find_entry_index(&store, kind, id_or_path).ok_or_else(|| {
        anyhow!(
            "extra profile not found for {}: {}",
            kind.label(),
            id_or_path
        )
    })?;
    let entry = store.profiles[idx].clone();

    cleanup_profile_unlocked(&entry, target, detach)?;
    store.profiles.remove(idx);
    save_store_to(&store_path, &store)?;
    Ok(())
}

fn cleanup_profile_unlocked(entry: &ProfileEntry, target: AgentTarget, detach: bool) -> Result<()> {
    let root = Path::new(&entry.root);
    if !root.is_dir() {
        if target == AgentTarget::Antigravity {
            let _ = antigravity_plugin::detach_ledger_for_root(&entry.id);
        }
        return Ok(());
    }
    if detach {
        if target == AgentTarget::Antigravity {
            antigravity_plugin::detach_ledger_for_root(&entry.id)?;
        }
        return Ok(());
    }
    reconcile_one_unlocked(entry, target, Mode::None, &BTreeMap::new())
}

/// Sync a single enabled extra profile to the agent's current primary mode & preferences.
pub fn sync_one(target: AgentTarget, id_or_path: &str) -> Result<ExtraProfileStatus> {
    let kind = kind_for_target(target);
    let _lock = mutation_lock::IntegrationMutationLock::acquire()?;
    let store = load_store_from(&paths::agent_profiles_file())?;
    let idx = find_entry_index(&store, kind, id_or_path).ok_or_else(|| {
        anyhow!(
            "extra profile not found for {}: {}",
            kind.label(),
            id_or_path
        )
    })?;
    let entry = &store.profiles[idx];
    if !entry.enabled {
        bail!(
            "profile {} is disabled; enable it before syncing",
            entry.label
        );
    }
    let primary_mode = primary_mode_for(target)?;
    reconcile_one_unlocked(entry, target, primary_mode, &BTreeMap::new())?;
    Ok(inspect_profile(entry, target, primary_mode))
}

/// Sync all enabled extra profiles for `target` to the agent's current primary mode & preferences.
pub fn sync_all(target: AgentTarget) -> Result<Vec<ExtraProfileStatus>> {
    let _lock = mutation_lock::IntegrationMutationLock::acquire()?;
    let primary_mode = primary_mode_for(target)?;
    reconcile_enabled_unlocked(target, primary_mode, &BTreeMap::new())?;
    Ok(statuses_for_agent(target, primary_mode))
}

/// Dismiss a discovered directory so it no longer surfaces as an enrollment hint (P5).
pub fn dismiss_discovered(target: AgentTarget, raw_root: &Path) -> Result<()> {
    let kind = kind_for_target(target);
    let _lock = mutation_lock::IntegrationMutationLock::acquire()?;
    let physical = normalize_lookup_path(kind, &raw_root.to_string_lossy());
    let root_str = physical.to_string_lossy().to_string();

    let store_path = paths::agent_profiles_file();
    let mut store = load_store_from(&store_path)?;
    if !store
        .dismissed
        .iter()
        .any(|d| d.agent == kind.as_str() && same_root(&d.root, &root_str))
    {
        store.dismissed.push(DismissedEntry {
            agent: kind.as_str().to_string(),
            root: root_str,
            dismissed_at: now_secs(),
        });
        save_store_to(&store_path, &store)?;
    }
    Ok(())
}

/// Reveal an extra profile's config/plugin folder in the OS file manager.
pub fn reveal(target: AgentTarget, id_or_path: &str) -> Result<()> {
    let kind = kind_for_target(target);
    let store = load_store_lenient();
    let idx = find_entry_index(&store, kind, id_or_path).ok_or_else(|| {
        anyhow!(
            "extra profile not found for {}: {}",
            kind.label(),
            id_or_path
        )
    })?;
    let root = PathBuf::from(&store.profiles[idx].root);
    let reveal_target = if target == AgentTarget::Antigravity {
        let plugin = root.join("config/plugins/askhuman");
        if plugin.exists() {
            plugin
        } else if root.join("config").exists() {
            root.join("config")
        } else {
            root
        }
    } else {
        root
    };
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("open")
            .arg("-R")
            .arg(&reveal_target)
            .spawn();
    }
    #[cfg(target_os = "windows")]
    {
        let _ = std::process::Command::new("explorer")
            .arg(&reveal_target)
            .spawn();
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let _ = std::process::Command::new("xdg-open")
            .arg(&reveal_target)
            .spawn();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_id_is_stable_and_filename_safe() {
        let id = profile_id(
            AgentKind::Antigravity,
            Path::new("/Users/test/.t3/userdata/providers/antigravity/ac0a3dfd"),
        );
        assert!(id.starts_with("antigravity-"));
        assert_eq!(id.len(), "antigravity-".len() + 16);
        assert!(id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'));
    }

    #[test]
    fn normalize_antigravity_config_subdir_to_gemini_root() {
        let dir = tempfile::tempdir().unwrap();
        let provider_root = dir.path().join("providers/antigravity/ac0a3dfd");
        let config_dir = provider_root.join("config");
        std::fs::create_dir_all(&config_dir).unwrap();
        std::fs::create_dir_all(provider_root.join("antigravity-acp")).unwrap();

        let normalized = normalize_root_for_add(AgentKind::Antigravity, &config_dir).unwrap();
        assert_eq!(
            normalized,
            agent_rules::physical_root(provider_root.clone())
        );

        let direct = normalize_root_for_add(AgentKind::Antigravity, &provider_root).unwrap();
        assert_eq!(direct, normalized);
    }

    #[test]
    fn rejects_relative_missing_and_primary_roots() {
        assert!(normalize_root_for_add(AgentKind::Codex, Path::new("relative/codex")).is_err());
        assert!(normalize_root_for_add(
            AgentKind::Codex,
            Path::new("/nonexistent/askhuman-test-profile-xyz")
        )
        .is_err());

        let primary = primary_root(AgentKind::Codex);
        if primary.is_dir() {
            assert!(normalize_root_for_add(AgentKind::Codex, &primary).is_err());
        }
    }

    #[cfg(unix)]
    #[test]
    fn discovers_t3_antigravity_dirs_and_respects_enrolled_and_dismissed() {
        let home = tempfile::tempdir().unwrap();
        let providers = paths::t3_antigravity_providers_dir_in(home.path());
        let p1 = providers.join("ac0a3dfd12345678");
        let p2 = providers.join("deadbeef87654321");
        let ignored_empty = providers.join("empty_dir");
        std::fs::create_dir_all(p1.join("config")).unwrap();
        std::fs::create_dir_all(p2.join("antigravity-acp")).unwrap();
        std::fs::create_dir_all(&ignored_empty).unwrap();

        let empty_store = Store::default();
        let found = discover_t3_antigravity_in(home.path(), &empty_store);
        assert_eq!(found.len(), 2);
        assert!(found.iter().any(|d| d.label == "T3 Code (ac0a3dfd)"));
        assert!(found.iter().any(|d| d.label == "T3 Code (deadbeef)"));

        let store_with_one_enrolled_and_one_dismissed = Store {
            version: 1,
            profiles: vec![ProfileEntry {
                id: "ag-1".into(),
                agent: "antigravity".into(),
                root: agent_rules::physical_root(p1).to_string_lossy().to_string(),
                label: "T3".into(),
                source: ProfileSource::T3Discovered,
                enabled: false,
                created_at: 1,
            }],
            dismissed: vec![DismissedEntry {
                agent: "antigravity".into(),
                root: agent_rules::physical_root(p2).to_string_lossy().to_string(),
                dismissed_at: 1,
            }],
        };
        let after =
            discover_t3_antigravity_in(home.path(), &store_with_one_enrolled_and_one_dismissed);
        assert!(after.is_empty());
    }

    #[test]
    fn with_agent_root_redirects_paths_and_restores_on_panic() {
        let dir = tempfile::tempdir().unwrap();
        let custom_claude = dir.path().join("claude-custom");
        std::fs::create_dir_all(&custom_claude).unwrap();
        let expected = agent_rules::physical_root(custom_claude.clone());

        paths::with_agent_root(AgentKind::Claude, &custom_claude, || {
            assert_eq!(paths::claude_dir(), expected);
            assert_eq!(paths::claude_json(), expected.join(".claude.json"));
        });
        assert_ne!(paths::claude_dir(), expected);

        let panicked = std::panic::catch_unwind(|| {
            paths::with_agent_root(AgentKind::Codex, &custom_claude, || {
                assert_eq!(paths::codex_dir(), expected);
                panic!("simulated failure");
            });
        });
        assert!(panicked.is_err());
        assert_ne!(paths::codex_dir(), expected);
    }

    #[test]
    fn reconcile_refuses_to_recreate_missing_extra_profile_root() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("vanished-t3-instance");
        let entry = ProfileEntry {
            id: "antigravity-missing".into(),
            agent: "antigravity".into(),
            root: missing.to_string_lossy().to_string(),
            label: "T3 Code (vanished)".into(),
            source: ProfileSource::T3Discovered,
            enabled: true,
            created_at: 1,
        };
        let err = reconcile_one_unlocked(
            &entry,
            AgentTarget::Antigravity,
            Mode::Mcp,
            &BTreeMap::new(),
        )
        .unwrap_err();
        assert!(err.to_string().contains("refusing to recreate"));
        assert!(!missing.exists());

        let status = inspect_profile(&entry, AgentTarget::Antigravity, Mode::Mcp);
        assert_eq!(status.state, "rootMissing");
        assert!(!status.needs_update);
        assert!(status.error.is_some());
    }

    #[cfg(unix)]
    #[test]
    fn normalize_lookup_path_resolves_symlinks_and_antigravity_config_suffix() {
        let dir = tempfile::tempdir().unwrap();
        let real_root = dir.path().join("providers/antigravity/ac0a3dfd");
        std::fs::create_dir_all(real_root.join("config")).unwrap();
        std::fs::create_dir_all(real_root.join("antigravity-acp")).unwrap();

        let alias_parent = dir.path().join("alias");
        std::os::unix::fs::symlink(dir.path().join("providers"), &alias_parent).unwrap();
        let via_symlink = alias_parent.join("antigravity/ac0a3dfd");
        let via_symlink_config = via_symlink.join("config");

        let expected = agent_rules::physical_root(real_root);
        assert_eq!(
            normalize_lookup_path(AgentKind::Antigravity, &via_symlink.to_string_lossy()),
            expected
        );
        assert_eq!(
            normalize_lookup_path(
                AgentKind::Antigravity,
                &via_symlink_config.to_string_lossy()
            ),
            expected
        );
    }

    #[cfg(unix)]
    #[test]
    fn resolves_t3_instance_names_from_hash_and_collapses_home_prefix() {
        let home = tempfile::tempdir().unwrap();
        let userdata = home.path().join(".t3/userdata");
        std::fs::create_dir_all(&userdata).unwrap();
        std::fs::write(
            userdata.join("settings.json"),
            r#"{"providerInstances":{"antigravity":{"driver":"antigravity"},"antigravity-work":{"driver":"antigravity"}}}"#,
        )
        .unwrap();

        let providers = paths::t3_antigravity_providers_dir_in(home.path());
        let default_hash = sha256_hex("antigravity");
        let work_hash = sha256_hex("antigravity-work");
        let unknown_hash = "deadbeef0123456789";
        std::fs::create_dir_all(providers.join(&default_hash).join("config")).unwrap();
        std::fs::create_dir_all(providers.join(&work_hash).join("antigravity-acp")).unwrap();
        std::fs::create_dir_all(providers.join(unknown_hash).join("config")).unwrap();

        let found = discover_t3_antigravity_in(home.path(), &Store::default());
        assert_eq!(found.len(), 3);
        let default_item = found
            .iter()
            .find(|d| d.root.ends_with(&default_hash))
            .unwrap();
        assert_eq!(default_item.label, "T3 Code (antigravity)");
        assert_eq!(
            default_item.display_root,
            format!("~/.t3/userdata/providers/antigravity/{default_hash}")
        );

        let work_item = found.iter().find(|d| d.root.ends_with(&work_hash)).unwrap();
        assert_eq!(work_item.label, "T3 Code (antigravity-work)");

        let unknown_item = found
            .iter()
            .find(|d| d.root.ends_with(unknown_hash))
            .unwrap();
        assert_eq!(unknown_item.label, "T3 Code (deadbeef)");
    }
}
