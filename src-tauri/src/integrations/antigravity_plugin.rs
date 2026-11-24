//! The AskHuman plugin for Antigravity.
//!
//! The App and `agy` read the same per-user plugin directory, so one managed plugin carries
//! everything AskHuman installs for both: the interaction rules, the MCP server and the lifecycle
//! hooks. The user can switch it off as a whole inside Antigravity.
//!
//! The directory is AskHuman-owned content under strict ownership. AskHuman's own ledger (in its
//! configuration directory, so every instance keeps its own) records the path, the owner and the
//! SHA-256 of every published file. A file that no longer matches the ledger was changed by
//! someone else: that is a conflict, and nothing is overwritten or deleted.
//!
//! Publication never exposes a half-written plugin. A new plugin is built in a private staging
//! directory and moved into place with one `rename`. An update replaces only the files whose
//! content changed, in a fixed order, and records its intent in the ledger first, so an
//! interrupted update is still recognised as AskHuman's own and is finished by the next run.
//! Rewriting these files does not restart a running MCP server or interrupt a pending `ask`,
//! but a changed `mcp_config.json` only reaches a server that starts afterwards.
//!
//! There is no atomic compare-and-swap against other processes of the same user: the checks make
//! overwriting someone else's edit unlikely, not impossible.

use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

use super::agent_rules::Variant;
use crate::agents::antigravity_hook::HookEvent;
use crate::agents::AgentKind;

/// The plugin's name; Antigravity derives the MCP server's name from it (`<plugin>_<server>`).
pub const PLUGIN_NAME: &str = "askhuman";

const SCHEMA: u32 = 1;
/// Bump when the set of files or their meaning changes.
const GENERATION: &str = "1";
const MAX_FILE_BYTES: u64 = 1 << 20;
/// Seconds. A hook may block for a human (an interjection waits for the composer), so the limit
/// is the whole day; the handlers of this version never block.
const HOOK_TIMEOUT_SECONDS: u64 = 86_400;
const STAGING_PREFIX: &str = ".askhuman-plugin-staging-";
const STAGING_MAX_AGE: Duration = Duration::from_secs(600);

const RULES: &str = "rules/AGENTS.md";
const MCP: &str = "mcp_config.json";
const HOOKS: &str = "hooks.json";
const PLUGIN: &str = "plugin.json";
const MARKER: &str = ".askhuman-managed";
/// The order in which an update replaces files; the marker goes last.
const ORDER: [&str; 5] = [RULES, MCP, HOOKS, PLUGIN, MARKER];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Flavor {
    Cli,
    Mcp,
}

impl From<Variant> for Flavor {
    fn from(variant: Variant) -> Self {
        match variant {
            Variant::Cli => Flavor::Cli,
            Variant::Mcp => Flavor::Mcp,
        }
    }
}

impl From<Flavor> for Variant {
    fn from(flavor: Flavor) -> Self {
        match flavor {
            Flavor::Cli => Variant::Cli,
            Flavor::Mcp => Variant::Mcp,
        }
    }
}

/// What the published plugin consists of.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Manifest {
    variant: Flavor,
    lifecycle: bool,
    stop: bool,
}

/// What a caller asks the plugin to be.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Request {
    pub variant: Variant,
    /// Install the lifecycle hooks (session, turn and activity reporting).
    pub lifecycle: bool,
    /// Install the Stop confirmation in the shared Stop hook.
    pub stop: bool,
}

impl From<Request> for Manifest {
    fn from(request: Request) -> Self {
        Manifest {
            variant: request.variant.into(),
            lifecycle: request.lifecycle,
            stop: request.stop,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Pending {
    manifest: Manifest,
    files: BTreeMap<String, String>,
}

/// AskHuman's record of the plugin it published.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Ledger {
    schema: u32,
    generation: String,
    path: String,
    uid: Option<u32>,
    manifest: Manifest,
    /// Relative path → SHA-256 of what was published.
    files: BTreeMap<String, String>,
    /// The state an update in flight is moving to; absent when nothing is in flight.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pending: Option<Pending>,
}

impl Ledger {
    /// The manifest the plugin is heading for: an interrupted update is finished, not undone.
    fn effective_manifest(&self) -> Manifest {
        self.pending
            .as_ref()
            .map_or(self.manifest, |pending| pending.manifest)
    }

    /// Every file that may legitimately be on disk: what was published and what an update in
    /// flight is moving to.
    fn known_files(&self) -> BTreeSet<String> {
        self.files
            .keys()
            .chain(self.pending.iter().flat_map(|pending| pending.files.keys()))
            .cloned()
            .collect()
    }

    /// Files that must be on disk at every moment: those that belong to the published state and
    /// to the state an update in flight is moving to. A file only one of them has may not exist
    /// yet, or may already be gone.
    fn required_files(&self) -> BTreeSet<String> {
        match &self.pending {
            None => self.files.keys().cloned().collect(),
            Some(pending) => self
                .files
                .keys()
                .filter(|rel| pending.files.contains_key(*rel))
                .cloned()
                .collect(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    /// Nothing at the plugin location and nothing recorded.
    Missing,
    OwnedCurrent,
    /// AskHuman's own plugin whose content differs from what this build would publish.
    OwnedOutdated,
    /// AskHuman recorded a plugin whose directory or files are gone.
    NeedsRepair,
    /// Something is there that AskHuman cannot prove to be its own.
    Conflict,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    pub state: State,
    pub variant: Option<Variant>,
    pub lifecycle: bool,
    /// The Stop hook asks the human before a turn ends.
    pub stop: bool,
    /// Why the state is a conflict.
    pub reason: Option<String>,
    /// Where the plugin lives (or would live).
    pub location: PathBuf,
}

impl Snapshot {
    fn without_plugin(layout: &Layout, state: State, reason: Option<String>) -> Self {
        Snapshot {
            state,
            variant: None,
            lifecycle: false,
            stop: false,
            reason,
            location: layout.dir.clone(),
        }
    }

    /// Whether AskHuman has (or had, for [`State::NeedsRepair`]) a plugin of its own.
    pub fn present(&self) -> bool {
        matches!(
            self.state,
            State::OwnedCurrent | State::OwnedOutdated | State::NeedsRepair
        )
    }

    /// Fails with an actionable message when something at the plugin location is not provably
    /// AskHuman's, so that no status view reports an integration it cannot manage.
    pub fn manageable(&self) -> Result<()> {
        match (&self.state, &self.reason) {
            (State::Conflict, reason) => Err(conflict_message(
                &self.location,
                reason.as_deref().unwrap_or("it cannot be verified"),
            )),
            _ => Ok(()),
        }
    }
}

// MARK: - Public API

/// A read-only look at the plugin.
pub fn snapshot() -> Result<Snapshot> {
    require_platform()?;
    let executable = executable().ok();
    snapshot_at(
        &Layout::current(),
        executable.as_deref(),
        &super::agent_rules::antigravity_rule_bytes,
    )
}

/// Publish the plugin as requested. Only the files whose content changed are rewritten.
pub fn install(request: Request) -> Result<()> {
    let executable = executable()?;
    install_at(
        &Layout::current(),
        request.into(),
        &executable,
        &super::agent_rules::antigravity_rule_bytes,
    )
}

/// Remove the plugin AskHuman published. Anything else in the directory stays where it is.
pub fn uninstall() -> Result<()> {
    uninstall_at(&Layout::current())
}

/// A read-only look at the plugin in an extra Antigravity root (`GEMINI_HOME`).
pub(crate) fn snapshot_for_root(gemini_root: &Path, profile_id: &str) -> Result<Snapshot> {
    require_platform()?;
    let executable = executable().ok();
    snapshot_at(
        &Layout::for_antigravity_root(gemini_root, &crate::paths::config_dir(), profile_id),
        executable.as_deref(),
        &super::agent_rules::antigravity_rule_bytes,
    )
}

/// Publish the plugin into an extra Antigravity root (`GEMINI_HOME`).
pub(crate) fn install_for_root(
    gemini_root: &Path,
    profile_id: &str,
    request: Request,
) -> Result<()> {
    let executable = executable()?;
    install_at(
        &Layout::for_antigravity_root(gemini_root, &crate::paths::config_dir(), profile_id),
        request.into(),
        &executable,
        &super::agent_rules::antigravity_rule_bytes,
    )
}

/// Remove the managed plugin from an extra Antigravity root (`GEMINI_HOME`).
pub(crate) fn uninstall_for_root(gemini_root: &Path, profile_id: &str) -> Result<()> {
    uninstall_at(&Layout::for_antigravity_root(
        gemini_root,
        &crate::paths::config_dir(),
        profile_id,
    ))
}

/// Remove only AskHuman's ownership ledger for an extra profile without touching disk files
/// (used when detaching a conflicting profile explicitly).
pub(crate) fn detach_ledger_for_root(profile_id: &str) -> Result<()> {
    let askhuman = super::agent_rules::physical_root(crate::paths::config_dir());
    let ledger = crate::paths::antigravity_extra_plugin_ledger_in(&askhuman, profile_id);
    match std::fs::remove_file(&ledger) {
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
            Err(error).with_context(|| format!("cannot remove {}", ledger.display()))
        }
        _ => Ok(()),
    }
}

/// The host view of an extra Antigravity root (`<gemini_root>/config/config.json`).
pub(crate) fn host_view_for_root(gemini_root: &Path) -> HostView {
    let root = super::agent_rules::physical_root(gemini_root.to_path_buf());
    host_view_at(&root.join("config").join("config.json"))
}

/// Turn the lifecycle hooks on or off. Without a plugin there is nothing to carry them, so this
/// is then a no-op: the plugin belongs to an integration mode, never to this switch.
pub fn set_lifecycle(enabled: bool) -> Result<()> {
    let layout = Layout::current();
    let manifest = match observe(&layout)? {
        Observed::Missing => return Ok(()),
        Observed::Conflict(reason) => return Err(conflict_error(&layout, &reason)),
        Observed::Owned(owned) => owned.ledger.effective_manifest(),
    };
    let executable = executable()?;
    install_at(
        &layout,
        Manifest {
            lifecycle: enabled,
            ..manifest
        },
        &executable,
        &super::agent_rules::antigravity_rule_bytes,
    )
}

/// Turn the Stop confirmation on or off. Like [`set_lifecycle`] it only changes an existing
/// plugin; the hooks file is rewritten only when its content changes.
pub fn set_stop(enabled: bool) -> Result<()> {
    let layout = Layout::current();
    let manifest = match observe(&layout)? {
        Observed::Missing => return Ok(()),
        Observed::Conflict(reason) => return Err(conflict_error(&layout, &reason)),
        Observed::Owned(owned) => owned.ledger.effective_manifest(),
    };
    let executable = executable()?;
    install_at(
        &layout,
        Manifest {
            stop: enabled,
            ..manifest
        },
        &executable,
        &super::agent_rules::antigravity_rule_bytes,
    )
}

/// The files a plugin of this shape consists of, below the user's home, for previews. Derived from
/// the same rendering that publishes them, so a preview cannot drift from what is written.
pub fn planned_files(request: Request) -> Vec<PathBuf> {
    let dir = crate::paths::antigravity_plugin_dir_in(&crate::paths::integration_home());
    render(request.into(), "/AskHuman", b"")
        .map(|files| files.keys().map(|rel| dir.join(rel)).collect())
        .unwrap_or_default()
}

/// What Antigravity's own configuration says about AskHuman.
///
/// `config.json` holds the user's private settings. AskHuman only reads it, never writes it, and
/// nothing depends on the answer: it feeds status hints.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct HostView {
    /// The user switched the plugin off inside Antigravity (`plugins.askhuman.enabled == false`),
    /// so its hooks and MCP server do not run.
    pub plugin_disabled: bool,
    /// The user's "always allow" list covers AskHuman's MCP tool. The format is private, so a
    /// grant AskHuman does not recognise only means the hint is shown.
    pub mcp_allowed: bool,
}

const HOST_CONFIG_MAX_BYTES: u64 = 4 << 20;

/// The host view of the current user. A missing, unreadable or unparseable file reads as the
/// default: not disabled, no grant seen.
pub fn host_view() -> HostView {
    host_view_at(
        &crate::paths::antigravity_config_dir_in(&crate::paths::integration_home())
            .join("config.json"),
    )
}

fn host_view_at(path: &Path) -> HostView {
    use std::io::Read;
    let Ok(file) = std::fs::File::open(path) else {
        return HostView::default();
    };
    let mut bytes = Vec::new();
    if file
        .take(HOST_CONFIG_MAX_BYTES + 1)
        .read_to_end(&mut bytes)
        .is_err()
        || bytes.len() as u64 > HOST_CONFIG_MAX_BYTES
    {
        return HostView::default();
    }
    let Ok(config) = serde_json::from_slice::<Value>(&bytes) else {
        return HostView::default();
    };
    let server = format!("{PLUGIN_NAME}_mcp");
    let allowed = |grant: &str| {
        grant == "mcp(*)"
            || grant == format!("mcp({server}/*)")
            || grant == format!("mcp({server}/ask)")
    };
    HostView {
        plugin_disabled: config["plugins"][PLUGIN_NAME]["enabled"] == Value::Bool(false),
        mcp_allowed: config["userSettings"]["globalPermissionGrants"]["allow"]
            .as_array()
            .is_some_and(|grants| grants.iter().filter_map(Value::as_str).any(allowed)),
    }
}

fn executable() -> Result<String> {
    Ok(crate::development::routing::program_path()
        .context("failed to resolve the AskHuman executable")?
        .to_string_lossy()
        .into_owned())
}

// MARK: - Content

type Files = BTreeMap<&'static str, Vec<u8>>;

fn render(manifest: Manifest, executable: &str, rules: &[u8]) -> Result<Files> {
    command_safe(executable)?;
    let mut files = Files::new();
    files.insert(
        PLUGIN,
        json_bytes(&json!({
            "name": PLUGIN_NAME,
            "description": "AskHuman: lets Antigravity ask you questions, report its progress and take your replies. Generated by AskHuman; change it in AskHuman's settings."
        })),
    );
    files.insert(RULES, rules.to_vec());
    if manifest.variant == Flavor::Mcp {
        files.insert(
            MCP,
            json_bytes(&json!({
                "mcpServers": {
                    "mcp": {
                        "command": executable,
                        "args": ["mcp"],
                        "timeoutSeconds": super::mcp_config::ANTIGRAVITY_TOOL_TIMEOUT_SEC,
                    }
                }
            })),
        );
    }
    if let Some(hooks) = hooks_json(manifest, executable) {
        files.insert(HOOKS, hooks);
    }
    files.insert(MARKER, marker_bytes(manifest));
    Ok(files)
}

/// The hooks the plugin registers, written in their final form: a handler that does not block yet
/// still gets the long timeout, so enabling interjection later never rewrites the file.
fn hooks_json(manifest: Manifest, executable: &str) -> Option<Vec<u8>> {
    let hook = |command: String, timeout: Option<u64>| {
        let mut hook = json!({ "type": "command", "command": command });
        if let Some(timeout) = timeout {
            hook["timeout"] = json!(timeout);
        }
        json!([hook])
    };
    let mut events = serde_json::Map::new();
    if manifest.lifecycle {
        let lifecycle = |event: HookEvent| {
            format!(
                "\"{executable}\" {} {} {}",
                super::agent_lifecycle::MARKER,
                AgentKind::Antigravity.as_str(),
                event.as_str()
            )
        };
        events.insert(
            "SessionStart".into(),
            hook(lifecycle(HookEvent::SessionStart), None),
        );
        events.insert(
            "PreInvocation".into(),
            hook(
                lifecycle(HookEvent::PreInvocation),
                Some(HOOK_TIMEOUT_SECONDS),
            ),
        );
    }
    if manifest.lifecycle || manifest.stop {
        events.insert(
            "Stop".into(),
            hook(
                super::agent_stop::hook_command_for(
                    executable,
                    AgentKind::Antigravity,
                    manifest.lifecycle,
                    manifest.stop,
                ),
                Some(HOOK_TIMEOUT_SECONDS),
            ),
        );
    }
    (!events.is_empty()).then(|| json_bytes(&json!({ PLUGIN_NAME: Value::Object(events) })))
}

fn marker_bytes(manifest: Manifest) -> Vec<u8> {
    json_bytes(&json!({
        "managedBy": "AskHuman",
        "schema": SCHEMA,
        "variant": manifest.variant,
        "lifecycle": manifest.lifecycle,
        "stop": manifest.stop,
        "note": "This directory is generated by AskHuman. Change it in AskHuman's settings; a manual edit blocks AskHuman's updates until it is undone."
    }))
}

fn json_bytes(value: &Value) -> Vec<u8> {
    let mut bytes = serde_json::to_vec_pretty(value).expect("a JSON value serializes");
    bytes.push(b'\n');
    bytes
}

/// Hook commands run through `sh -c` inside double quotes.
fn command_safe(executable: &str) -> Result<()> {
    if executable.is_empty()
        || !Path::new(executable).is_absolute()
        || executable
            .chars()
            .any(|c| matches!(c, '"' | '`' | '$' | '\\' | '\n' | '\r' | '\0'))
    {
        bail!(
            "the AskHuman executable path {executable:?} cannot be written into an Antigravity hook command"
        );
    }
    Ok(())
}

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn digests(files: &Files) -> BTreeMap<String, String> {
    files
        .iter()
        .map(|(rel, bytes)| ((*rel).to_string(), digest(bytes)))
        .collect()
}

// MARK: - Layout and filesystem inspection

#[derive(Debug, Clone)]
struct Layout {
    /// Antigravity's per-user configuration directory.
    config: PathBuf,
    /// The plugin directory below it.
    dir: PathBuf,
    /// AskHuman's ownership ledger for the plugin.
    ledger: PathBuf,
}

impl Layout {
    fn current() -> Self {
        // The two trust roots belong to the user's environment and may sit behind links; only
        // what lies below them is held to the strict rules.
        let home = super::agent_rules::physical_root(crate::paths::integration_home());
        let askhuman = super::agent_rules::physical_root(crate::paths::config_dir());
        Self::new(&home, &askhuman)
    }

    fn new(home: &Path, askhuman_config: &Path) -> Self {
        Layout {
            config: crate::paths::antigravity_config_dir_in(home),
            dir: crate::paths::antigravity_plugin_dir_in(home),
            ledger: crate::paths::antigravity_plugin_ledger_in(askhuman_config),
        }
    }

    fn for_antigravity_root(gemini_root: &Path, askhuman_config: &Path, profile_id: &str) -> Self {
        let root = super::agent_rules::physical_root(gemini_root.to_path_buf());
        let askhuman = super::agent_rules::physical_root(askhuman_config.to_path_buf());
        let config = root.join("config");
        let dir = config.join("plugins").join(PLUGIN_NAME);
        let ledger = crate::paths::antigravity_extra_plugin_ledger_in(&askhuman, profile_id);
        Layout {
            config,
            dir,
            ledger,
        }
    }

    /// The directories between the home directory and the plugin directory, outermost first.
    fn ancestors(&self) -> Vec<PathBuf> {
        let plugins = self
            .dir
            .parent()
            .expect("the plugin directory has a parent");
        let gemini = self
            .config
            .parent()
            .expect("the configuration directory has a parent");
        vec![
            gemini.to_path_buf(),
            self.config.clone(),
            plugins.to_path_buf(),
        ]
    }
}

enum Entry {
    Missing,
    Dir(std::fs::Metadata),
    File(std::fs::Metadata),
    Other(&'static str),
}

/// What lives at a path, without following a link at the last component.
fn entry(path: &Path) -> Result<Entry> {
    match std::fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_symlink() => Ok(Entry::Other("a symbolic link")),
        Ok(meta) if meta.is_dir() => Ok(Entry::Dir(meta)),
        Ok(meta) if meta.is_file() => Ok(Entry::File(meta)),
        Ok(_) => Ok(Entry::Other("neither a regular file nor a directory")),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Entry::Missing),
        Err(error) => Err(error).with_context(|| format!("cannot inspect {}", path.display())),
    }
}

#[cfg(unix)]
fn effective_uid() -> Option<u32> {
    // SAFETY: `geteuid` has no preconditions and cannot fail.
    Some(unsafe { libc::geteuid() })
}

#[cfg(not(unix))]
fn effective_uid() -> Option<u32> {
    None
}

#[cfg(unix)]
fn owned_by_us(meta: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;
    Some(meta.uid()) == effective_uid()
}

#[cfg(not(unix))]
fn owned_by_us(_: &std::fs::Metadata) -> bool {
    true
}

#[cfg(unix)]
fn single_link(meta: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;
    meta.nlink() == 1
}

#[cfg(not(unix))]
fn single_link(_: &std::fs::Metadata) -> bool {
    true
}

enum Read {
    Missing,
    Conflict(String),
    Bytes(Vec<u8>),
}

/// Why a directory cannot hold managed content, if it cannot.
fn directory_problem(path: &Path, found: Entry) -> Option<String> {
    match found {
        Entry::Dir(meta) if owned_by_us(&meta) => None,
        Entry::Dir(_) => Some(format!("{} belongs to another user", path.display())),
        Entry::File(_) => Some(format!("{} is a file, not a directory", path.display())),
        Entry::Other(what) => Some(format!("{} is {what}", path.display())),
        Entry::Missing => Some(format!("{} disappeared", path.display())),
    }
}

fn read_managed(layout: &Layout, rel: &str) -> Result<Read> {
    let path = layout.dir.join(rel);
    if let Some(parent) = Path::new(rel)
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        let parent_path = layout.dir.join(parent);
        match entry(&parent_path)? {
            Entry::Missing => return Ok(Read::Missing),
            found => {
                if let Some(problem) = directory_problem(&parent_path, found) {
                    return Ok(Read::Conflict(problem));
                }
            }
        }
    }
    Ok(match entry(&path)? {
        Entry::Missing => Read::Missing,
        Entry::File(meta) if !owned_by_us(&meta) => {
            Read::Conflict(format!("{} belongs to another user", path.display()))
        }
        Entry::File(meta) if !single_link(&meta) => {
            Read::Conflict(format!("{} has more than one hard link", path.display()))
        }
        Entry::File(meta) if meta.len() > MAX_FILE_BYTES => {
            Read::Conflict(format!("{} is unexpectedly large", path.display()))
        }
        Entry::File(_) => Read::Bytes(
            std::fs::read(&path).with_context(|| format!("cannot read {}", path.display()))?,
        ),
        Entry::Dir(_) => Read::Conflict(format!("{} is a directory", path.display())),
        Entry::Other(what) => Read::Conflict(format!("{} is {what}", path.display())),
    })
}

// MARK: - Observation

enum Observed {
    Missing,
    Conflict(String),
    Owned(Box<OwnedObservation>),
}

struct OwnedObservation {
    ledger: Ledger,
    /// SHA-256 of every recorded file that is on disk.
    present: BTreeMap<String, String>,
    /// Recorded files that are not on disk.
    missing: Vec<String>,
    dir_exists: bool,
}

fn read_ledger(path: &Path) -> std::result::Result<Option<Ledger>, String> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(format!(
                "AskHuman's ownership record {} cannot be read: {error}",
                path.display()
            ))
        }
    };
    let ledger: Ledger = serde_json::from_slice(&bytes).map_err(|error| {
        format!(
            "AskHuman's ownership record {} is unreadable: {error}",
            path.display()
        )
    })?;
    if ledger.schema != SCHEMA {
        return Err(format!(
            "AskHuman's ownership record {} was written by a different version",
            path.display()
        ));
    }
    Ok(Some(ledger))
}

fn write_ledger(path: &Path, ledger: &Ledger) -> Result<()> {
    let dir = path.parent().context("the ledger has a directory")?;
    std::fs::create_dir_all(dir).with_context(|| format!("cannot create {}", dir.display()))?;
    let temp = dir.join(format!(
        ".antigravity-plugin.json.tmp-{}",
        uuid::Uuid::new_v4()
    ));
    let mut bytes = serde_json::to_vec_pretty(ledger)?;
    bytes.push(b'\n');
    let published = write_private(&temp, &bytes).and_then(|()| {
        std::fs::rename(&temp, path).with_context(|| format!("cannot publish {}", path.display()))
    });
    if published.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    published
}

/// A ledger without its plugin on disk: AskHuman published it once and it has since vanished.
fn absent(layout: &Layout, ledger: Option<Ledger>) -> Observed {
    let Some(ledger) = ledger else {
        return Observed::Missing;
    };
    if ledger.path != layout.dir.to_string_lossy() {
        return Observed::Conflict(format!(
            "AskHuman's ownership record names a different plugin location ({})",
            ledger.path
        ));
    }
    Observed::Owned(Box::new(OwnedObservation {
        missing: ledger.known_files().into_iter().collect(),
        ledger,
        present: BTreeMap::new(),
        dir_exists: false,
    }))
}

fn observe(layout: &Layout) -> Result<Observed> {
    let ledger = match read_ledger(&layout.ledger) {
        Ok(ledger) => ledger,
        Err(reason) => return Ok(Observed::Conflict(reason)),
    };
    for ancestor in layout.ancestors() {
        match entry(&ancestor)? {
            Entry::Missing => return Ok(absent(layout, ledger)),
            found => {
                if let Some(problem) = directory_problem(&ancestor, found) {
                    return Ok(Observed::Conflict(problem));
                }
            }
        }
    }
    match entry(&layout.dir)? {
        Entry::Missing => return Ok(absent(layout, ledger)),
        found => {
            if let Some(problem) = directory_problem(&layout.dir, found) {
                return Ok(Observed::Conflict(problem));
            }
        }
    }
    let Some(ledger) = ledger else {
        return Ok(Observed::Conflict(format!(
            "{} exists, but AskHuman has no record of creating it",
            layout.dir.display()
        )));
    };
    if ledger.path != layout.dir.to_string_lossy() {
        return Ok(Observed::Conflict(format!(
            "AskHuman's ownership record names a different plugin location ({})",
            ledger.path
        )));
    }
    if ledger.uid != effective_uid() {
        return Ok(Observed::Conflict(
            "AskHuman's ownership record belongs to a different user".into(),
        ));
    }
    let required = ledger.required_files();
    let mut present = BTreeMap::new();
    let mut missing = Vec::new();
    for rel in ledger.known_files() {
        match read_managed(layout, &rel)? {
            Read::Missing => {
                if required.contains(&rel) {
                    missing.push(rel);
                }
            }
            Read::Conflict(reason) => return Ok(Observed::Conflict(reason)),
            Read::Bytes(bytes) => {
                let sha = digest(&bytes);
                let published = ledger.files.get(&rel) == Some(&sha);
                let intended = ledger
                    .pending
                    .as_ref()
                    .is_some_and(|pending| pending.files.get(&rel) == Some(&sha));
                if !published && !intended {
                    return Ok(Observed::Conflict(format!(
                        "{} was changed outside AskHuman",
                        layout.dir.join(&rel).display()
                    )));
                }
                present.insert(rel, sha);
            }
        }
    }
    Ok(Observed::Owned(Box::new(OwnedObservation {
        ledger,
        present,
        missing,
        dir_exists: true,
    })))
}

fn conflict_error(layout: &Layout, reason: &str) -> anyhow::Error {
    conflict_message(&layout.dir, reason)
}

fn conflict_message(location: &Path, reason: &str) -> anyhow::Error {
    anyhow!(
        "cannot manage the Antigravity plugin at {}: {reason}; AskHuman leaves it untouched. Restore the file, remove the directory, or use the AskHuman instance that owns it, then retry explicitly: AskHuman agents mode antigravity <none|cli|mcp>",
        location.display()
    )
}

fn snapshot_at(
    layout: &Layout,
    executable: Option<&str>,
    rules: &dyn Fn(Variant) -> Vec<u8>,
) -> Result<Snapshot> {
    Ok(match observe(layout)? {
        Observed::Missing => Snapshot::without_plugin(layout, State::Missing, None),
        Observed::Conflict(reason) => {
            Snapshot::without_plugin(layout, State::Conflict, Some(reason))
        }
        Observed::Owned(owned) => {
            let manifest = owned.ledger.effective_manifest();
            let state = if !owned.missing.is_empty() || !owned.dir_exists {
                State::NeedsRepair
            } else {
                let desired = executable.and_then(|executable| {
                    render(manifest, executable, &rules(manifest.variant.into())).ok()
                });
                match desired {
                    Some(desired)
                        if owned.ledger.pending.is_none()
                            && owned.ledger.manifest == manifest
                            && owned.present == digests(&desired)
                            && owned.ledger.files == owned.present =>
                    {
                        State::OwnedCurrent
                    }
                    _ => State::OwnedOutdated,
                }
            };
            Snapshot {
                state,
                variant: Some(manifest.variant.into()),
                lifecycle: manifest.lifecycle,
                stop: manifest.stop,
                reason: None,
                location: layout.dir.clone(),
            }
        }
    })
}

// MARK: - Writing

#[cfg(unix)]
fn create_private_file(path: &Path) -> std::io::Result<std::fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
}

#[cfg(not(unix))]
fn create_private_file(path: &Path) -> std::io::Result<std::fs::File> {
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
}

#[cfg(unix)]
fn create_private_dir(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    std::fs::DirBuilder::new().mode(0o700).create(path)
}

#[cfg(not(unix))]
fn create_private_dir(path: &Path) -> std::io::Result<()> {
    std::fs::create_dir(path)
}

fn write_private(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut file =
        create_private_file(path).with_context(|| format!("cannot create {}", path.display()))?;
    file.write_all(bytes)
        .and_then(|()| file.sync_all())
        .with_context(|| format!("cannot write {}", path.display()))
}

/// A directory of ours, created private if it is missing.
fn ensure_directory(path: &Path) -> Result<()> {
    match entry(path)? {
        Entry::Missing => {
            create_private_dir(path).with_context(|| format!("cannot create {}", path.display()))
        }
        found => match directory_problem(path, found) {
            None => Ok(()),
            Some(problem) => bail!("{problem}"),
        },
    }
}

fn ensure_chain(layout: &Layout) -> Result<()> {
    layout
        .ancestors()
        .iter()
        .try_for_each(|dir| ensure_directory(dir))
}

/// A private directory next to `plugins/` where files are written before they are moved into
/// place, so the host never sees a temporary file inside the plugin.
struct Staging {
    path: PathBuf,
}

impl Staging {
    fn create(layout: &Layout) -> Result<Self> {
        ensure_chain(layout)?;
        sweep_staging(layout);
        let path = layout
            .config
            .join(format!("{STAGING_PREFIX}{}", uuid::Uuid::new_v4()));
        create_private_dir(&path).with_context(|| format!("cannot create {}", path.display()))?;
        Ok(Staging { path })
    }

    fn write(&self, rel: &str, bytes: &[u8]) -> Result<()> {
        let target = self.path.join(rel);
        if let Some(parent) = target.parent().filter(|parent| *parent != self.path) {
            ensure_directory(parent)?;
        }
        write_private(&target, bytes)
    }
}

impl Drop for Staging {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// Remove staging directories an interrupted run left behind. Only old directories that are
/// ours (a real directory, our owner, our prefix) are touched.
fn sweep_staging(layout: &Layout) {
    let Ok(entries) = std::fs::read_dir(&layout.config) else {
        return;
    };
    for item in entries.flatten() {
        let name = item.file_name();
        if !name.to_string_lossy().starts_with(STAGING_PREFIX) {
            continue;
        }
        let path = item.path();
        let Ok(Entry::Dir(meta)) = entry(&path) else {
            continue;
        };
        let old = meta
            .modified()
            .ok()
            .and_then(|modified| modified.elapsed().ok())
            .is_some_and(|age| age > STAGING_MAX_AGE);
        if owned_by_us(&meta) && old {
            let _ = std::fs::remove_dir_all(&path);
        }
    }
}

fn require_platform() -> Result<()> {
    if cfg!(unix) {
        Ok(())
    } else {
        bail!("the Antigravity integration is not supported on this platform")
    }
}

fn install_at(
    layout: &Layout,
    manifest: Manifest,
    executable: &str,
    rules: &dyn Fn(Variant) -> Vec<u8>,
) -> Result<()> {
    require_platform()?;
    let desired = render(manifest, executable, &rules(manifest.variant.into()))?;
    let wanted = digests(&desired);
    match observe(layout)? {
        Observed::Conflict(reason) => Err(conflict_error(layout, &reason)),
        Observed::Missing => publish_new(layout, manifest, &desired, &wanted),
        Observed::Owned(owned) if !owned.dir_exists => {
            publish_new(layout, manifest, &desired, &wanted)
        }
        Observed::Owned(owned) => update_existing(layout, *owned, manifest, &desired, &wanted),
    }
}

/// Build the whole plugin privately, then publish it with one `rename`.
fn publish_new(
    layout: &Layout,
    manifest: Manifest,
    desired: &Files,
    wanted: &BTreeMap<String, String>,
) -> Result<()> {
    let staging = Staging::create(layout)?;
    for (rel, bytes) in desired {
        staging.write(rel, bytes)?;
    }
    // The record goes first: a crash before the `rename` leaves a record without a plugin, which
    // the next run recognises as ours and publishes again.
    write_ledger(
        &layout.ledger,
        &Ledger {
            schema: SCHEMA,
            generation: GENERATION.into(),
            path: layout.dir.to_string_lossy().into_owned(),
            uid: effective_uid(),
            manifest,
            files: wanted.clone(),
            pending: None,
        },
    )?;
    std::fs::rename(&staging.path, &layout.dir).with_context(|| {
        format!(
            "cannot publish the plugin at {}; the ownership record is already written, retry explicitly: AskHuman agents mode antigravity <cli|mcp>",
            layout.dir.display()
        )
    })
}

/// Replace only what changed, in the fixed order, recording the intent first.
fn update_existing(
    layout: &Layout,
    owned: OwnedObservation,
    manifest: Manifest,
    desired: &Files,
    wanted: &BTreeMap<String, String>,
) -> Result<()> {
    let mut ledger = owned.ledger;
    let changed: Vec<&str> = ORDER
        .iter()
        .copied()
        .filter(|&rel| match (desired.get(rel), owned.present.get(rel)) {
            (Some(bytes), Some(sha)) => digest(bytes) != *sha,
            (None, None) => false,
            _ => true,
        })
        .collect();
    let settled =
        ledger.pending.is_none() && ledger.manifest == manifest && ledger.files == *wanted;
    if changed.is_empty() && settled {
        return Ok(());
    }
    if !changed.is_empty() {
        ledger.pending = Some(Pending {
            manifest,
            files: wanted.clone(),
        });
        write_ledger(&layout.ledger, &ledger)?;
        let staging = Staging::create(layout)?;
        for &rel in &changed {
            match desired.get(rel) {
                Some(bytes) => replace_file(layout, &staging, rel, bytes)?,
                None => {
                    let path = layout.dir.join(rel);
                    std::fs::remove_file(&path)
                        .with_context(|| format!("cannot remove {}", path.display()))?;
                }
            }
        }
    }
    ledger.manifest = manifest;
    ledger.files = wanted.clone();
    ledger.pending = None;
    write_ledger(&layout.ledger, &ledger)
}

fn replace_file(layout: &Layout, staging: &Staging, rel: &str, bytes: &[u8]) -> Result<()> {
    let target = layout.dir.join(rel);
    if let Some(parent) = target.parent().filter(|parent| *parent != layout.dir) {
        ensure_directory(parent)?;
    }
    let temp = staging.path.join(format!("{}.tmp", uuid::Uuid::new_v4()));
    write_private(&temp, bytes)?;
    std::fs::rename(&temp, &target).with_context(|| format!("cannot publish {}", target.display()))
}

fn uninstall_at(layout: &Layout) -> Result<()> {
    let owned = match observe(layout)? {
        Observed::Missing => return Ok(()),
        Observed::Conflict(reason) => return Err(conflict_error(layout, &reason)),
        Observed::Owned(owned) => owned,
    };
    for &rel in ORDER.iter().rev() {
        if owned.present.contains_key(rel) {
            let path = layout.dir.join(rel);
            std::fs::remove_file(&path)
                .with_context(|| format!("cannot remove {}", path.display()))?;
        }
    }
    if owned.dir_exists {
        // Directories only come off when empty: a file the user added keeps its directory.
        let _ = std::fs::remove_dir(layout.dir.join("rules"));
        let _ = std::fs::remove_dir(&layout.dir);
    }
    match std::fs::remove_file(&layout.ledger) {
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
            Err(error).with_context(|| format!("cannot remove {}", layout.ledger.display()))
        }
        _ => Ok(()),
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    const EXE: &str = "/opt/Ask Human/AskHuman";

    struct Fixture {
        _root: tempfile::TempDir,
        layout: Layout,
    }

    impl Fixture {
        fn new() -> Self {
            let root = tempfile::tempdir().unwrap();
            let root_path = root.path().canonicalize().unwrap();
            let home = root_path.join("home");
            let askhuman = root_path.join("askhuman");
            std::fs::create_dir(&home).unwrap();
            std::fs::create_dir(&askhuman).unwrap();
            Self {
                layout: Layout::new(&home, &askhuman),
                _root: root,
            }
        }

        fn install(&self, manifest: Manifest) -> Result<()> {
            install_at(&self.layout, manifest, EXE, &rules)
        }

        fn state(&self) -> State {
            snapshot_at(&self.layout, Some(EXE), &rules).unwrap().state
        }

        fn path(&self, rel: &str) -> PathBuf {
            self.layout.dir.join(rel)
        }

        fn read(&self, rel: &str) -> String {
            std::fs::read_to_string(self.path(rel)).unwrap()
        }

        fn json(&self, rel: &str) -> Value {
            serde_json::from_str(&self.read(rel)).unwrap()
        }

        fn ledger(&self) -> Ledger {
            read_ledger(&self.layout.ledger).unwrap().unwrap()
        }

        /// Every file and directory below the plugin directory with its content or mode.
        fn tree(&self) -> BTreeMap<String, String> {
            fn walk(base: &Path, dir: &Path, out: &mut BTreeMap<String, String>) {
                let Ok(entries) = std::fs::read_dir(dir) else {
                    return;
                };
                for item in entries.flatten() {
                    let path = item.path();
                    let rel = path
                        .strip_prefix(base)
                        .unwrap()
                        .to_string_lossy()
                        .into_owned();
                    if path.is_dir() {
                        out.insert(rel, "dir".into());
                        walk(base, &path, out);
                    } else {
                        out.insert(rel, std::fs::read_to_string(&path).unwrap());
                    }
                }
            }
            let mut out = BTreeMap::new();
            walk(&self.layout.dir, &self.layout.dir, &mut out);
            out
        }
    }

    fn rules(variant: Variant) -> Vec<u8> {
        match variant {
            Variant::Cli => b"cli rules\n".to_vec(),
            Variant::Mcp => b"mcp rules\n".to_vec(),
        }
    }

    fn manifest(variant: Variant, lifecycle: bool) -> Manifest {
        Manifest {
            variant: variant.into(),
            lifecycle,
            stop: false,
        }
    }

    #[test]
    fn a_new_plugin_has_the_documented_layout_and_private_permissions() {
        let fixture = Fixture::new();
        fixture.install(manifest(Variant::Mcp, true)).unwrap();

        assert_eq!(fixture.state(), State::OwnedCurrent);
        assert_eq!(
            fixture.tree().keys().cloned().collect::<Vec<_>>(),
            [
                ".askhuman-managed",
                "hooks.json",
                "mcp_config.json",
                "plugin.json",
                "rules",
                "rules/AGENTS.md"
            ]
        );
        assert_eq!(fixture.read("rules/AGENTS.md"), "mcp rules\n");
        assert_eq!(fixture.json("plugin.json")["name"], PLUGIN_NAME);
        for rel in ORDER {
            let mode = std::fs::metadata(fixture.path(rel))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600, "{rel}");
        }
        let mode = std::fs::metadata(&fixture.layout.dir)
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o700);

        let marker = fixture.json(MARKER);
        assert_eq!(marker["managedBy"], "AskHuman");
        assert_eq!(marker["variant"], "mcp");
        assert_eq!(marker["lifecycle"], true);

        let ledger = fixture.ledger();
        assert_eq!(ledger.path, fixture.layout.dir.to_string_lossy());
        assert_eq!(ledger.files.len(), 5);
        assert!(ledger.pending.is_none());
        // No staging directory is left beside the plugins.
        let leftovers: Vec<_> = std::fs::read_dir(&fixture.layout.config)
            .unwrap()
            .flatten()
            .map(|item| item.file_name().to_string_lossy().into_owned())
            .filter(|name| name.starts_with(STAGING_PREFIX))
            .collect();
        assert!(leftovers.is_empty(), "{leftovers:?}");
    }

    #[test]
    fn the_mcp_server_and_hooks_are_written_in_their_final_form() {
        let fixture = Fixture::new();
        fixture.install(manifest(Variant::Mcp, true)).unwrap();

        let mcp = fixture.json(MCP);
        let server = &mcp["mcpServers"]["mcp"];
        assert_eq!(server["command"], EXE);
        assert_eq!(server["args"], json!(["mcp"]));
        assert_eq!(server["timeoutSeconds"], 86_400);

        let hooks = fixture.json(HOOKS);
        let events = &hooks[PLUGIN_NAME];
        assert_eq!(
            events["SessionStart"][0]["command"],
            format!("\"{EXE}\" __agent-hook antigravity session-start")
        );
        assert_eq!(
            events["PreInvocation"][0]["command"],
            format!("\"{EXE}\" __agent-hook antigravity pre-invocation")
        );
        // The shared Stop handler also carries lifecycle tracking.
        assert_eq!(
            events["Stop"][0]["command"],
            format!("\"{EXE}\" __stop-hook antigravity track")
        );
        assert_eq!(events["SessionStart"][0].get("timeout"), None);
        assert_eq!(events["PreInvocation"][0]["timeout"], 86_400);
        assert_eq!(events["Stop"][0]["timeout"], 86_400);
        assert_eq!(events.as_object().unwrap().len(), 3);
    }

    #[test]
    fn the_cli_variant_has_no_mcp_server_and_a_plugin_without_hooks_has_no_hook_file() {
        let fixture = Fixture::new();
        fixture.install(manifest(Variant::Cli, false)).unwrap();
        assert!(!fixture.path(MCP).exists());
        assert!(!fixture.path(HOOKS).exists());
        assert_eq!(fixture.read("rules/AGENTS.md"), "cli rules\n");
        assert_eq!(fixture.state(), State::OwnedCurrent);

        let confirm_only = render(
            Manifest {
                variant: Flavor::Cli,
                lifecycle: false,
                stop: true,
            },
            EXE,
            b"r",
        )
        .unwrap();
        let hooks: Value = serde_json::from_slice(&confirm_only[HOOKS]).unwrap();
        let events = hooks[PLUGIN_NAME].as_object().unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(
            events["Stop"][0]["command"],
            format!("\"{EXE}\" __stop-hook antigravity confirm")
        );
    }

    #[test]
    fn an_unchanged_request_rewrites_nothing() {
        use std::os::unix::fs::MetadataExt;
        let fixture = Fixture::new();
        fixture.install(manifest(Variant::Mcp, true)).unwrap();
        let identity = |fixture: &Fixture| {
            ORDER
                .iter()
                .map(|rel| {
                    let meta = std::fs::metadata(fixture.path(rel)).unwrap();
                    (meta.ino(), meta.mtime(), meta.mtime_nsec())
                })
                .chain(std::iter::once({
                    let meta = std::fs::metadata(&fixture.layout.ledger).unwrap();
                    (meta.ino(), meta.mtime(), meta.mtime_nsec())
                }))
                .collect::<Vec<_>>()
        };
        let before = identity(&fixture);
        std::thread::sleep(Duration::from_millis(20));
        fixture.install(manifest(Variant::Mcp, true)).unwrap();
        assert_eq!(identity(&fixture), before);
    }

    #[test]
    fn an_update_rewrites_only_the_files_that_changed() {
        use std::os::unix::fs::MetadataExt;
        let fixture = Fixture::new();
        fixture.install(manifest(Variant::Mcp, false)).unwrap();
        let inode = |rel: &str| std::fs::metadata(fixture.path(rel)).unwrap().ino();
        let (rules_before, mcp_before, plugin_before) = (inode(RULES), inode(MCP), inode(PLUGIN));

        fixture.install(manifest(Variant::Mcp, true)).unwrap();

        assert!(fixture.path(HOOKS).exists());
        assert_eq!(fixture.json(MARKER)["lifecycle"], true);
        assert_eq!(inode(RULES), rules_before);
        assert_eq!(inode(MCP), mcp_before);
        assert_eq!(inode(PLUGIN), plugin_before);
        assert_eq!(fixture.state(), State::OwnedCurrent);
        assert!(fixture.ledger().manifest.lifecycle);
    }

    #[test]
    fn switching_the_variant_swaps_the_rules_and_removes_the_mcp_server() {
        let fixture = Fixture::new();
        fixture.install(manifest(Variant::Mcp, true)).unwrap();
        fixture.install(manifest(Variant::Cli, true)).unwrap();
        assert!(!fixture.path(MCP).exists());
        assert_eq!(fixture.read("rules/AGENTS.md"), "cli rules\n");
        assert_eq!(fixture.state(), State::OwnedCurrent);
        assert!(!fixture.ledger().files.contains_key(MCP));

        fixture.install(manifest(Variant::Mcp, false)).unwrap();
        assert!(fixture.path(MCP).exists());
        assert!(!fixture.path(HOOKS).exists());
        assert_eq!(fixture.read("rules/AGENTS.md"), "mcp rules\n");
        assert_eq!(fixture.state(), State::OwnedCurrent);
    }

    #[test]
    fn a_changed_executable_or_rule_makes_the_plugin_outdated_until_it_is_updated() {
        let fixture = Fixture::new();
        fixture.install(manifest(Variant::Mcp, true)).unwrap();
        let moved = snapshot_at(&fixture.layout, Some("/elsewhere/AskHuman"), &rules).unwrap();
        assert_eq!(moved.state, State::OwnedOutdated);
        assert_eq!(moved.variant, Some(Variant::Mcp));
        assert!(moved.lifecycle);
        let unknown = snapshot_at(&fixture.layout, None, &rules).unwrap();
        assert_eq!(unknown.state, State::OwnedOutdated);

        let edited_rules = |_: Variant| b"new collaboration style\n".to_vec();
        let changed = snapshot_at(&fixture.layout, Some(EXE), &edited_rules).unwrap();
        assert_eq!(changed.state, State::OwnedOutdated);
        install_at(
            &fixture.layout,
            manifest(Variant::Mcp, true),
            EXE,
            &edited_rules,
        )
        .unwrap();
        assert_eq!(fixture.read("rules/AGENTS.md"), "new collaboration style\n");
        assert_eq!(
            snapshot_at(&fixture.layout, Some(EXE), &edited_rules)
                .unwrap()
                .state,
            State::OwnedCurrent
        );
    }

    #[test]
    fn a_file_edited_by_someone_else_is_a_conflict_that_nothing_touches() {
        let fixture = Fixture::new();
        fixture.install(manifest(Variant::Mcp, true)).unwrap();
        std::fs::write(fixture.path(RULES), "my own rules\n").unwrap();
        let before = fixture.tree();
        let ledger = std::fs::read(&fixture.layout.ledger).unwrap();

        let snapshot = snapshot_at(&fixture.layout, Some(EXE), &rules).unwrap();
        assert_eq!(snapshot.state, State::Conflict);
        assert!(snapshot
            .reason
            .unwrap()
            .contains("changed outside AskHuman"));
        let error = fixture
            .install(manifest(Variant::Cli, true))
            .unwrap_err()
            .to_string();
        assert!(error.contains("leaves it untouched"), "{error}");
        assert!(
            error.contains("AskHuman agents mode antigravity"),
            "{error}"
        );
        assert!(uninstall_at(&fixture.layout).is_err());
        assert_eq!(fixture.tree(), before);
        assert_eq!(std::fs::read(&fixture.layout.ledger).unwrap(), ledger);
    }

    #[test]
    fn a_directory_without_a_record_is_never_taken_over() {
        let fixture = Fixture::new();
        std::fs::create_dir_all(&fixture.layout.dir).unwrap();
        std::fs::write(fixture.path("plugin.json"), "{\"name\":\"askhuman\"}\n").unwrap();
        let before = fixture.tree();
        assert_eq!(fixture.state(), State::Conflict);
        assert!(fixture.install(manifest(Variant::Mcp, true)).is_err());
        assert!(uninstall_at(&fixture.layout).is_err());
        assert_eq!(fixture.tree(), before);
        assert!(!fixture.layout.ledger.exists());
    }

    #[test]
    fn another_users_record_or_location_is_not_ours() {
        let fixture = Fixture::new();
        fixture.install(manifest(Variant::Cli, false)).unwrap();
        let mut ledger = fixture.ledger();
        ledger.path = "/somewhere/else".into();
        write_ledger(&fixture.layout.ledger, &ledger).unwrap();
        assert_eq!(fixture.state(), State::Conflict);
        let mut ledger = fixture.ledger();
        ledger.path = fixture.layout.dir.to_string_lossy().into_owned();
        ledger.uid = Some(effective_uid().unwrap().wrapping_add(1));
        write_ledger(&fixture.layout.ledger, &ledger).unwrap();
        assert_eq!(fixture.state(), State::Conflict);

        std::fs::write(&fixture.layout.ledger, b"{ not json").unwrap();
        assert_eq!(fixture.state(), State::Conflict);
        assert!(fixture.install(manifest(Variant::Cli, false)).is_err());
    }

    #[test]
    fn links_are_refused_at_every_level() {
        use std::os::unix::fs::symlink;
        // The plugin directory itself.
        let fixture = Fixture::new();
        let elsewhere = fixture.layout.config.parent().unwrap().join("elsewhere");
        std::fs::create_dir_all(&elsewhere).unwrap();
        std::fs::create_dir_all(fixture.layout.dir.parent().unwrap()).unwrap();
        symlink(&elsewhere, &fixture.layout.dir).unwrap();
        assert_eq!(fixture.state(), State::Conflict);
        assert!(fixture.install(manifest(Variant::Cli, false)).is_err());
        assert!(std::fs::read_dir(&elsewhere).unwrap().next().is_none());

        // A directory above it.
        let fixture = Fixture::new();
        let real = fixture
            .layout
            .config
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("real");
        std::fs::create_dir_all(real.join("config")).unwrap();
        symlink(&real, fixture.layout.config.parent().unwrap()).unwrap();
        assert_eq!(fixture.state(), State::Conflict);
        assert!(fixture.install(manifest(Variant::Cli, false)).is_err());
        assert!(!real.join("config/plugins").exists());

        // The rules directory inside an owned plugin.
        let fixture = Fixture::new();
        fixture.install(manifest(Variant::Cli, false)).unwrap();
        let target = fixture.layout.config.join("redirect");
        std::fs::create_dir(&target).unwrap();
        std::fs::copy(fixture.path(RULES), target.join("AGENTS.md")).unwrap();
        std::fs::remove_dir_all(fixture.path("rules")).unwrap();
        symlink(&target, fixture.path("rules")).unwrap();
        assert_eq!(fixture.state(), State::Conflict);

        // A managed file replaced by a link, and one with another hard link.
        let fixture = Fixture::new();
        fixture.install(manifest(Variant::Cli, false)).unwrap();
        let stash = fixture.layout.config.join("stash");
        std::fs::rename(fixture.path(PLUGIN), &stash).unwrap();
        symlink(&stash, fixture.path(PLUGIN)).unwrap();
        assert_eq!(fixture.state(), State::Conflict);
        std::fs::remove_file(fixture.path(PLUGIN)).unwrap();
        std::fs::rename(&stash, fixture.path(PLUGIN)).unwrap();
        assert_eq!(fixture.state(), State::OwnedCurrent);
        std::fs::hard_link(fixture.path(PLUGIN), fixture.layout.config.join("alias")).unwrap();
        assert_eq!(fixture.state(), State::Conflict);
    }

    #[test]
    fn an_interrupted_update_is_recognised_and_finished_by_the_next_run() {
        let fixture = Fixture::new();
        fixture.install(manifest(Variant::Mcp, false)).unwrap();
        // Simulate a crash after the intent and the first replaced file of an update that
        // turns the lifecycle hooks on and changes the rules.
        let edited = |_: Variant| b"edited rules\n".to_vec();
        let target = manifest(Variant::Mcp, true);
        let desired = render(target, EXE, &edited(Variant::Mcp)).unwrap();
        let mut ledger = fixture.ledger();
        ledger.pending = Some(Pending {
            manifest: target,
            files: digests(&desired),
        });
        write_ledger(&fixture.layout.ledger, &ledger).unwrap();
        std::fs::write(fixture.path(RULES), &desired[RULES]).unwrap();

        let snapshot = snapshot_at(&fixture.layout, Some(EXE), &edited).unwrap();
        assert_eq!(snapshot.state, State::OwnedOutdated, "not a conflict");
        assert!(snapshot.lifecycle, "the intended state is what is reported");

        install_at(&fixture.layout, target, EXE, &edited).unwrap();
        assert_eq!(
            snapshot_at(&fixture.layout, Some(EXE), &edited)
                .unwrap()
                .state,
            State::OwnedCurrent
        );
        assert!(fixture.path(HOOKS).exists());
        let ledger = fixture.ledger();
        assert!(ledger.pending.is_none());
        assert_eq!(ledger.manifest, target);
    }

    #[test]
    fn a_crash_after_the_last_file_only_needs_the_final_ledger() {
        let fixture = Fixture::new();
        fixture.install(manifest(Variant::Mcp, false)).unwrap();
        let target = manifest(Variant::Mcp, true);
        let desired = render(target, EXE, &rules(Variant::Mcp)).unwrap();
        let mut ledger = fixture.ledger();
        ledger.pending = Some(Pending {
            manifest: target,
            files: digests(&desired),
        });
        write_ledger(&fixture.layout.ledger, &ledger).unwrap();
        for rel in ORDER {
            if let Some(bytes) = desired.get(rel) {
                std::fs::write(fixture.path(rel), bytes).unwrap();
            }
        }
        assert_eq!(fixture.state(), State::OwnedOutdated);
        fixture.install(target).unwrap();
        assert_eq!(fixture.state(), State::OwnedCurrent);
    }

    #[test]
    fn a_plugin_that_vanished_is_republished_from_the_record() {
        let fixture = Fixture::new();
        fixture.install(manifest(Variant::Mcp, true)).unwrap();
        std::fs::remove_dir_all(&fixture.layout.dir).unwrap();
        let snapshot = snapshot_at(&fixture.layout, Some(EXE), &rules).unwrap();
        assert_eq!(snapshot.state, State::NeedsRepair);
        assert_eq!(snapshot.variant, Some(Variant::Mcp));
        assert!(snapshot.lifecycle);
        assert!(snapshot.present());

        fixture.install(manifest(Variant::Mcp, true)).unwrap();
        assert_eq!(fixture.state(), State::OwnedCurrent);

        // One missing file is repaired in place.
        std::fs::remove_file(fixture.path(HOOKS)).unwrap();
        assert_eq!(fixture.state(), State::NeedsRepair);
        fixture.install(manifest(Variant::Mcp, true)).unwrap();
        assert_eq!(fixture.state(), State::OwnedCurrent);
        assert!(fixture.path(HOOKS).exists());
    }

    #[test]
    fn uninstalling_removes_what_was_published_and_keeps_what_the_user_added() {
        let fixture = Fixture::new();
        uninstall_at(&fixture.layout).unwrap();
        fixture.install(manifest(Variant::Mcp, true)).unwrap();
        uninstall_at(&fixture.layout).unwrap();
        assert!(!fixture.layout.dir.exists());
        assert!(!fixture.layout.ledger.exists());
        assert!(
            fixture.layout.dir.parent().unwrap().exists(),
            "plugins/ is Antigravity's"
        );
        assert_eq!(fixture.state(), State::Missing);
        uninstall_at(&fixture.layout).unwrap();

        fixture.install(manifest(Variant::Cli, false)).unwrap();
        std::fs::write(fixture.path("notes.txt"), "mine").unwrap();
        uninstall_at(&fixture.layout).unwrap();
        assert_eq!(
            fixture.tree().keys().cloned().collect::<Vec<_>>(),
            ["notes.txt"]
        );
        assert!(!fixture.layout.ledger.exists());
        // What is left is the user's directory now, not ours.
        assert_eq!(fixture.state(), State::Conflict);
    }

    #[test]
    fn lifecycle_is_a_switch_on_an_existing_plugin_only() {
        // `set_lifecycle` resolves the real layout; its logic is `install_at` with the flag
        // changed, which is what is checked here.
        let fixture = Fixture::new();
        fixture.install(manifest(Variant::Cli, false)).unwrap();
        let current = fixture.ledger().effective_manifest();
        fixture
            .install(Manifest {
                lifecycle: true,
                ..current
            })
            .unwrap();
        assert!(fixture.path(HOOKS).exists());
        assert_eq!(fixture.ledger().manifest.variant, Flavor::Cli);
        fixture
            .install(Manifest {
                lifecycle: false,
                ..fixture.ledger().manifest
            })
            .unwrap();
        assert!(!fixture.path(HOOKS).exists());
        assert!(!fixture.json(MARKER)["lifecycle"].as_bool().unwrap());
    }

    #[test]
    fn stale_staging_directories_are_swept_but_foreign_ones_are_not() {
        let fixture = Fixture::new();
        std::fs::create_dir_all(&fixture.layout.config).unwrap();
        let old = fixture.layout.config.join(format!("{STAGING_PREFIX}old"));
        let young = fixture.layout.config.join(format!("{STAGING_PREFIX}young"));
        let foreign = fixture.layout.config.join("rules");
        for dir in [&old, &young, &foreign] {
            std::fs::create_dir(dir).unwrap();
            std::fs::write(dir.join("x"), "x").unwrap();
        }
        let long_ago = std::time::SystemTime::now() - STAGING_MAX_AGE * 2;
        std::fs::File::open(&old)
            .unwrap()
            .set_modified(long_ago)
            .unwrap();

        sweep_staging(&fixture.layout);
        assert!(!old.exists());
        assert!(young.exists());
        assert!(foreign.exists());
    }

    #[test]
    fn an_executable_path_that_would_break_the_hook_command_is_refused() {
        let manifest = manifest(Variant::Mcp, true);
        for bad in [
            "relative/AskHuman",
            "",
            "/opt/\"quoted\"/AskHuman",
            "/opt/$HOME/AskHuman",
            "/opt/`x`/AskHuman",
            "/opt/a\\b/AskHuman",
            "/opt/line\nbreak/AskHuman",
        ] {
            assert!(render(manifest, bad, b"r").is_err(), "{bad:?}");
        }
        assert!(render(
            manifest,
            "/Applications/Ask Human.app/Contents/MacOS/AskHuman",
            b"r"
        )
        .is_ok());
    }

    fn host(config: &str) -> HostView {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        std::fs::write(&path, config).unwrap();
        host_view_at(&path)
    }

    #[test]
    fn the_host_config_is_read_for_the_switch_and_the_grant_and_nothing_else() {
        let neither = HostView::default();
        // Nothing to read: no file, unreadable content, the wrong shape, an unrelated plugin.
        assert_eq!(
            host_view_at(Path::new("/nonexistent/askhuman/config.json")),
            neither
        );
        for config in [
            "",
            "not json",
            "[]",
            "null",
            r#"{"plugins":[],"userSettings":3}"#,
            r#"{"plugins":{"swarm":{"enabled":false}}}"#,
            r#"{"plugins":{"askhuman":{"enabled":true}}}"#,
            r#"{"plugins":{"askhuman":{}}}"#,
            r#"{"plugins":{"askhuman":{"enabled":"false"}}}"#,
            r#"{"userSettings":{"globalPermissionGrants":{"allow":["command(AskHuman)","command(ls)"]}}}"#,
            r#"{"userSettings":{"globalPermissionGrants":{"allow":"mcp(*)"}}}"#,
            r#"{"userSettings":{"globalPermissionGrants":{"allow":["mcp(other_mcp/*)",7,null]}}}"#,
            r#"{"userSettings":{"globalPermissionGrants":{"allow":["mcp(askhuman_mcp/other)"]}}}"#,
        ] {
            assert_eq!(host(config), neither, "{config}");
        }

        // The switch is only what a literal `false` says.
        assert_eq!(
            host(r#"{"plugins":{"askhuman":{"enabled":false}}}"#),
            HostView {
                plugin_disabled: true,
                mcp_allowed: false
            }
        );
        // Any grant that covers the plugin's MCP tool counts, wherever it sits in the list.
        for grant in ["mcp(*)", "mcp(askhuman_mcp/*)", "mcp(askhuman_mcp/ask)"] {
            let config = format!(
                r#"{{"userSettings":{{"globalPermissionGrants":{{"allow":["command(ls)","{grant}"]}}}}}}"#
            );
            assert_eq!(
                host(&config),
                HostView {
                    plugin_disabled: false,
                    mcp_allowed: true
                },
                "{grant}"
            );
        }
    }

    #[test]
    fn an_oversized_host_config_is_not_trusted() {
        let padding = "x".repeat(HOST_CONFIG_MAX_BYTES as usize);
        let config =
            format!(r#"{{"plugins":{{"askhuman":{{"enabled":false}}}},"pad":"{padding}"}}"#);
        assert_eq!(host(&config), HostView::default());
    }
}
