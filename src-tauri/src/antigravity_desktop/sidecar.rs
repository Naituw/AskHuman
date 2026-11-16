//! The sidecars AskHuman manages for the Antigravity App: what each one is, the files that make it
//! up, and AskHuman's registry of them (docs/specs/antigravity-deep-integration.md §12.3–§12.4).
//!
//! A sidecar is a directory `~/.gemini/config/sidecars/<id>/` holding `sidecar.json`, switched on
//! by `sidecars.<id>.enabled` in `config.json` (see [`host_config`]). The App starts its process
//! with the permissions the manifest grants and a project of its own. AskHuman keeps one
//! registry (`~/.askhuman/antigravity-sidecars.json`): it is the only source of the rules a
//! sidecar carries, `sidecar.json` is generated from it, and the SHA-256 it records is what tells
//! AskHuman's own file from one somebody edited. A file that does not match, a directory that is
//! not in the registry and one that another AskHuman instance marked are conflicts: nothing is
//! overwritten or removed, and the caller says why.
//!
//! Every function takes a [`Layout`], so tests run in a temporary home.

use super::host_config;
use crate::integrations::antigravity_plugin::PLUGIN_NAME;
use crate::integrations::hook_edit::{atomic_write, atomic_write_private};
use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// How many sidecars AskHuman keeps at most; each one is a project in the App's list.
pub(super) const LIMIT: usize = 20;
const MARKER: &str = ".askhuman-managed";
const MANIFEST: &str = "sidecar.json";
const REGISTRY_VERSION: u32 = 1;
const MAX_RULES: usize = 100;
const MAX_RULE_CHARS: usize = 500;
const MAX_FILE_BYTES: u64 = 1 << 20;
/// The actions a grant can name (`action(target)`), as Antigravity documents them.
const ACTIONS: [&str; 7] = [
    "command",
    "read_file",
    "write_file",
    "read_url",
    "execute_url",
    "download_url",
    "mcp",
];
/// What a YOLO sidecar may do without asking: everything the sandbox otherwise gates.
const YOLO_GRANTS: [&str; 5] = [
    "command(*)",
    "read_file(*)",
    "write_file(*)",
    "read_url(*)",
    "mcp(*)",
];

/// One process at a time edits the registry and the sidecar directories.
static LOCK: Mutex<()> = Mutex::new(());

// MARK: - Layout

/// Where the files of the sidecars live.
#[derive(Clone, Debug)]
pub(super) struct Layout {
    /// `~/.gemini/config/sidecars`.
    pub sidecars: PathBuf,
    /// `~/.gemini/config/config.json`.
    pub config_json: PathBuf,
    /// AskHuman's registry of the sidecars it made.
    pub registry: PathBuf,
    /// Names this AskHuman instance in the marker of every directory it made.
    pub instance: String,
}

impl Layout {
    pub fn current() -> Self {
        let config = crate::paths::antigravity_config_dir_in(&crate::paths::integration_home());
        Layout {
            sidecars: config.join("sidecars"),
            config_json: config.join("config.json"),
            registry: crate::paths::config_dir().join("antigravity-sidecars.json"),
            instance: crate::paths::config_dir().to_string_lossy().into_owned(),
        }
    }
}

// MARK: - What a sidecar is

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Kind {
    /// A project folder with a whitelist of grants.
    Workspace,
    /// A project folder with every grant: nothing is asked.
    Yolo,
    /// The one sidecar without a project of the user's: it only carries messages to conversations.
    Control,
}

impl Kind {
    fn prefix(self) -> &'static str {
        match self {
            Kind::Workspace => "askhuman-ws-",
            Kind::Yolo => "askhuman-yolo-",
            Kind::Control => "askhuman-ctl",
        }
    }
}

/// What a sidecar should be. The registry stores it; the files are generated from it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Spec {
    pub kind: Kind,
    /// The canonical project folder (for [`Kind::Control`], AskHuman's private empty folder).
    pub directory: PathBuf,
    /// Grants the user added for this folder, on top of the ones every sidecar of its kind has.
    pub rules: Vec<String>,
}

impl Spec {
    /// `askhuman-ws-<h8>` / `askhuman-yolo-<h8>`: `<h8>` is the first 8 hex digits of the SHA-256
    /// of the folder's path. The control sidecar has one fixed id.
    pub fn id(&self) -> String {
        match self.kind {
            Kind::Control => Kind::Control.prefix().to_string(),
            kind => {
                let hash = Sha256::digest(self.directory.to_string_lossy().as_bytes());
                format!("{}{}", kind.prefix(), &format!("{hash:x}")[..8])
            }
        }
    }
}

/// Whether `id` could be one of ours; checked before an id from outside names a path.
pub(super) fn valid_id(id: &str) -> bool {
    id.len() <= 64
        && id.starts_with(host_config::PREFIX)
        && id.len() > host_config::PREFIX.len()
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
}

/// The folders under which AskHuman delivers task attachments, which a whitelisted sidecar may
/// read: the request-scoped copies live below the temporary directory. The path can be written
/// three ways (as given, with the temporary directory's symlinks resolved, and fully resolved to
/// the spelling on disk, which can differ in case on a case-insensitive volume); the agent may
/// compare any of them, so each is granted.
pub(super) fn attachment_roots() -> Vec<PathBuf> {
    let temp = std::env::temp_dir();
    let root = temp.join("askhuman");
    let mut roots = vec![root.clone()];
    let mut add = |path: PathBuf| {
        if !roots.contains(&path) {
            roots.push(path);
        }
    };
    if let Ok(real_temp) = std::fs::canonicalize(&temp) {
        add(real_temp.join("askhuman"));
    }
    if let Ok(real) = std::fs::canonicalize(&root) {
        add(real);
    }
    roots
}

/// `file:///…` for an absolute path, percent-encoding everything outside the unreserved set.
pub(super) fn file_uri(path: &Path) -> Result<String> {
    if !path.is_absolute() {
        bail!("{} is not an absolute path", path.display());
    }
    let text = path
        .to_str()
        .ok_or_else(|| anyhow!("{} is not valid UTF-8", path.display()))?;
    let mut uri = String::from("file://");
    for byte in text.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' => {
                uri.push(byte as char)
            }
            other => uri.push_str(&format!("%{other:02X}")),
        }
    }
    Ok(uri)
}

fn grant_safe(path: &Path) -> bool {
    path.is_absolute()
        && !path
            .to_string_lossy()
            .chars()
            .any(|c| c.is_control() || c == '(' || c == ')')
}

/// Whether `rule` is a grant the user may add to a sidecar of `kind`: `action(target)` with a
/// known action. A whitelist may not carry a global wildcard (`mcp(*)`, `command(*)`, …): that is
/// what a YOLO sidecar is for.
pub(super) fn validate_rule(rule: &str, kind: Kind) -> Result<()> {
    if rule.chars().count() > MAX_RULE_CHARS || rule != rule.trim() {
        bail!("The rule {rule:?} is not a valid grant");
    }
    let Some((action, rest)) = rule.split_once('(') else {
        bail!("The rule {rule:?} must look like action(target)");
    };
    let Some(target) = rest.strip_suffix(')') else {
        bail!("The rule {rule:?} must look like action(target)");
    };
    if !ACTIONS.contains(&action) {
        bail!("The rule {rule:?} names an unknown action {action:?}");
    }
    if target.is_empty() || target.chars().any(char::is_control) {
        bail!("The rule {rule:?} has no usable target");
    }
    if target == "*" && kind != Kind::Yolo {
        bail!("The rule {rule:?} allows everything; use a YOLO task for that");
    }
    Ok(())
}

/// The rules in order without repeats, each checked.
pub(super) fn validate_rules(rules: &[String], kind: Kind) -> Result<Vec<String>> {
    if rules.len() > MAX_RULES {
        bail!("A sidecar takes at most {MAX_RULES} rules");
    }
    let mut checked: Vec<String> = Vec::new();
    for rule in rules {
        validate_rule(rule, kind)?;
        if !checked.contains(rule) {
            checked.push(rule.clone());
        }
    }
    Ok(checked)
}

fn grants(spec: &Spec, roots: &[PathBuf]) -> Vec<String> {
    match spec.kind {
        Kind::Yolo => YOLO_GRANTS.iter().map(|grant| grant.to_string()).collect(),
        Kind::Control => Vec::new(),
        Kind::Workspace => {
            let mut grants = vec![format!("mcp({PLUGIN_NAME}_mcp/*)")];
            for root in roots.iter().filter(|root| grant_safe(root)) {
                grants.push(format!("read_file({})", root.display()));
            }
            for rule in &spec.rules {
                if !grants.contains(rule) {
                    grants.push(rule.clone());
                }
            }
            grants
        }
    }
}

fn folder_name(directory: &Path) -> String {
    let name = directory
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| directory.to_string_lossy().into_owned());
    name.chars().take(60).collect()
}

/// `sidecar.json` of `spec`: the format Antigravity's own documentation gives. The keys come out
/// in a fixed order, so the same spec always produces the same bytes.
pub(super) fn render_manifest(executable: &str, spec: &Spec, roots: &[PathBuf]) -> Result<Vec<u8>> {
    if !Path::new(executable).is_absolute() || executable.contains('\0') {
        bail!("The AskHuman executable path {executable:?} cannot be written into a sidecar");
    }
    let path = spec.directory.to_string_lossy();
    let (display_name, description) = match spec.kind {
        Kind::Workspace => (
            format!("AskHuman · {}", folder_name(&spec.directory)),
            format!(
                "Starts tasks from your phone or IM in {path}. Created and managed by AskHuman; remove it in AskHuman's settings. 由 AskHuman 创建并管理，用于从手机或 IM 在 {path} 新建任务；可在 AskHuman 设置中移除。"
            ),
        ),
        Kind::Yolo => (
            format!("AskHuman · {} · YOLO", folder_name(&spec.directory)),
            format!(
                "WARNING: broad access. Tasks started from your phone or IM in {path} run commands, read and write files and use the network without asking, and so do conversations you open in this project yourself. Created and managed by AskHuman; remove it in AskHuman's settings. 宽授权：在 {path} 中从手机或 IM 新建的任务（以及你自己在此项目中开的会话）会不再询问地执行命令、读写文件、访问网络。由 AskHuman 创建并管理，可在 AskHuman 设置中移除。"
            ),
        ),
        Kind::Control => (
            "AskHuman · control".to_string(),
            "Carries messages from AskHuman to your conversations. It has no permissions and no folder of yours. Created and managed by AskHuman; remove it in AskHuman's settings. 由 AskHuman 创建并管理，只负责把消息送到你的会话，没有任何权限；可在 AskHuman 设置中移除。".to_string(),
        ),
    };
    let value = json!({
        "command": executable,
        "args": ["__antigravity-sidecar", "--id", spec.id()],
        "restart_policy": "always",
        "display_name": display_name,
        "description": description,
        "agent_permissions": {
            "workspace_uris": [file_uri(&spec.directory)?],
            "access_grants": grants(spec, roots),
            "allows_user_interactions": false,
        },
    });
    let mut bytes = serde_json::to_vec_pretty(&value)?;
    bytes.push(b'\n');
    Ok(bytes)
}

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

// MARK: - Registry

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Entry {
    pub id: String,
    pub kind: Kind,
    pub directory: String,
    #[serde(default)]
    pub rules: Vec<String>,
    /// Milliseconds since the epoch.
    pub created_at: u64,
    #[serde(default)]
    pub last_used_at: u64,
    /// SHA-256 of the `sidecar.json` AskHuman wrote last.
    pub manifest_sha256: String,
    /// The one before it, while an update is in flight: a crash between recording the new hash and
    /// writing the file must not make AskHuman's own file look foreign.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous_manifest_sha256: Option<String>,
}

impl Entry {
    fn spec(&self) -> Spec {
        Spec {
            kind: self.kind,
            directory: PathBuf::from(&self.directory),
            rules: self.rules.clone(),
        }
    }

    fn accepts(&self, hash: &str) -> bool {
        self.manifest_sha256 == hash || self.previous_manifest_sha256.as_deref() == Some(hash)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Registry {
    pub version: u32,
    pub sidecars: Vec<Entry>,
}

impl Default for Registry {
    fn default() -> Self {
        Registry {
            version: REGISTRY_VERSION,
            sidecars: Vec::new(),
        }
    }
}

impl Registry {
    fn get(&self, id: &str) -> Option<&Entry> {
        self.sidecars.iter().find(|entry| entry.id == id)
    }

    fn upsert(&mut self, entry: Entry) {
        match self.sidecars.iter_mut().find(|item| item.id == entry.id) {
            Some(item) => *item = entry,
            None => self.sidecars.push(entry),
        }
    }
}

fn load_registry(path: &Path) -> Result<Registry> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Registry::default())
        }
        Err(error) => {
            return Err(error).with_context(|| format!("Could not read {}", path.display()))
        }
    };
    let registry: Registry = serde_json::from_slice(&bytes).map_err(|error| {
        anyhow!(
            "AskHuman's list of Antigravity sidecars ({}) cannot be read ({error}); fix or delete it, then try again",
            path.display()
        )
    })?;
    if registry.version > REGISTRY_VERSION {
        bail!(
            "AskHuman's list of Antigravity sidecars ({}) was written by a newer AskHuman; update AskHuman first",
            path.display()
        );
    }
    Ok(registry)
}

fn save_registry(path: &Path, registry: &Registry) -> Result<()> {
    let mut bytes = serde_json::to_vec_pretty(registry)?;
    bytes.push(b'\n');
    atomic_write_private(path, &bytes)
        .with_context(|| format!("Could not write {}", path.display()))
}

// MARK: - Errors callers tell apart

/// The registry already holds [`LIMIT`] sidecars.
#[derive(Debug)]
pub(super) struct LimitReached;

impl std::fmt::Display for LimitReached {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "AskHuman already manages {LIMIT} Antigravity sidecars; remove some in Settings first"
        )
    }
}

impl std::error::Error for LimitReached {}

/// Something is where a sidecar of ours would be, and AskHuman cannot show it is its own.
#[derive(Debug)]
pub(super) struct Conflict(pub String);

impl std::fmt::Display for Conflict {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for Conflict {}

/// The id names no sidecar AskHuman made.
#[derive(Debug)]
pub(super) struct NotManaged(pub String);

impl std::fmt::Display for NotManaged {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} is not a sidecar AskHuman made", self.0)
    }
}

impl std::error::Error for NotManaged {}

/// The rules a sidecar was asked to have are not valid grants.
#[derive(Debug)]
pub(super) struct BadRules(pub String);

impl std::fmt::Display for BadRules {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for BadRules {}

// MARK: - The directory of a sidecar

#[derive(Debug, Serialize, Deserialize)]
struct Marker {
    id: String,
    instance: String,
}

#[derive(Debug, PartialEq, Eq)]
enum Dir {
    Absent,
    /// AskHuman's own: marked, recorded, nothing foreign inside. `manifest` is the hash of
    /// `sidecar.json` as it is, `None` when it was never written.
    Ours {
        manifest: Option<String>,
    },
    Foreign(String),
}

fn read_small(path: &Path) -> std::io::Result<Vec<u8>> {
    use std::io::Read;
    let mut bytes = Vec::new();
    std::fs::File::open(path)?
        .take(MAX_FILE_BYTES + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_FILE_BYTES {
        return Err(std::io::Error::other("the file is too large"));
    }
    Ok(bytes)
}

fn inspect(layout: &Layout, id: &str, entry: Option<&Entry>) -> Result<Dir> {
    let dir = layout.sidecars.join(id);
    let meta = match std::fs::symlink_metadata(&dir) {
        Ok(meta) => meta,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Dir::Absent),
        Err(error) => {
            return Err(error).with_context(|| format!("Could not read {}", dir.display()))
        }
    };
    if !meta.is_dir() {
        return Ok(Dir::Foreign(format!(
            "{} is not a plain directory",
            dir.display()
        )));
    }
    let Some(entry) = entry else {
        return Ok(Dir::Foreign(format!(
            "{} exists but AskHuman has no record of making it",
            dir.display()
        )));
    };
    let names: Vec<String> = std::fs::read_dir(&dir)
        .with_context(|| format!("Could not read {}", dir.display()))?
        .filter_map(|child| child.ok())
        .map(|child| child.file_name().to_string_lossy().into_owned())
        .collect();
    // A directory AskHuman made and was interrupted in has nothing in it yet.
    if names.is_empty() {
        return Ok(Dir::Ours { manifest: None });
    }
    if let Some(other) = names
        .iter()
        .find(|name| name.as_str() != MANIFEST && name.as_str() != MARKER)
    {
        return Ok(Dir::Foreign(format!(
            "{} holds a file AskHuman did not put there ({other})",
            dir.display()
        )));
    }
    let marker = read_small(&dir.join(MARKER))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Marker>(&bytes).ok());
    match marker {
        Some(marker) if marker.id == id && marker.instance == layout.instance => {}
        Some(_) => {
            return Ok(Dir::Foreign(format!(
                "{} belongs to another AskHuman installation",
                dir.display()
            )))
        }
        None => {
            return Ok(Dir::Foreign(format!(
                "{} is not marked as AskHuman's",
                dir.display()
            )))
        }
    }
    match read_small(&dir.join(MANIFEST)) {
        Ok(bytes) => {
            let hash = digest(&bytes);
            if entry.accepts(&hash) {
                Ok(Dir::Ours {
                    manifest: Some(hash),
                })
            } else {
                Ok(Dir::Foreign(format!(
                    "{} was changed after AskHuman wrote it",
                    dir.join(MANIFEST).display()
                )))
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            Ok(Dir::Ours { manifest: None })
        }
        Err(error) => {
            Err(error).with_context(|| format!("Could not read {}", dir.join(MANIFEST).display()))
        }
    }
}

fn write_files(layout: &Layout, id: &str, manifest: &[u8]) -> Result<()> {
    let dir = layout.sidecars.join(id);
    std::fs::create_dir_all(&dir).with_context(|| format!("Could not create {}", dir.display()))?;
    let marker = serde_json::to_vec_pretty(&Marker {
        id: id.to_string(),
        instance: layout.instance.clone(),
    })?;
    atomic_write(&dir.join(MARKER), &marker)?;
    atomic_write(&dir.join(MANIFEST), manifest)?;
    Ok(())
}

// MARK: - Making, changing and removing

/// What [`ensure`] did to the files.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Change {
    Created,
    Updated,
    Unchanged,
}

/// Makes the sidecar of `spec` exist as AskHuman would write it and switches it on in
/// `config.json`. The registry is written before the files, so an interruption leaves a sidecar
/// that the next call finishes; an existing sidecar is changed only when it is provably ours.
pub(super) fn ensure(
    layout: &Layout,
    executable: &str,
    spec: &Spec,
    roots: &[PathBuf],
    now_ms: u64,
) -> Result<Change> {
    let spec = Spec {
        rules: validate_rules(&spec.rules, spec.kind)
            .map_err(|error| BadRules(error.to_string()))?,
        ..spec.clone()
    };
    let id = spec.id();
    let manifest = render_manifest(executable, &spec, roots)?;
    let hash = digest(&manifest);
    let _guard = LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let mut registry = load_registry(&layout.registry)?;
    let existing = registry.get(&id).cloned();
    if existing.is_none() && registry.sidecars.len() >= LIMIT {
        return Err(LimitReached.into());
    }
    let state = inspect(layout, &id, existing.as_ref())?;
    let created = state == Dir::Absent;
    let entry = |previous: Option<String>| Entry {
        id: id.clone(),
        kind: spec.kind,
        directory: spec.directory.to_string_lossy().into_owned(),
        rules: spec.rules.clone(),
        created_at: existing.as_ref().map_or(now_ms, |old| old.created_at),
        last_used_at: now_ms,
        manifest_sha256: hash.clone(),
        previous_manifest_sha256: previous,
    };
    let change = match state {
        Dir::Foreign(reason) => return Err(Conflict(reason).into()),
        Dir::Absent | Dir::Ours { manifest: None } => {
            registry.upsert(entry(None));
            save_registry(&layout.registry, &registry)?;
            write_files(layout, &id, &manifest)?;
            if created {
                Change::Created
            } else {
                Change::Updated
            }
        }
        Dir::Ours {
            manifest: Some(current),
        } if current == hash => {
            if existing.as_ref().map(|old| (&old.rules, old.last_used_at))
                != Some((&spec.rules, now_ms))
            {
                registry.upsert(entry(None));
                save_registry(&layout.registry, &registry)?;
            }
            Change::Unchanged
        }
        Dir::Ours {
            manifest: Some(current),
        } => {
            // The new hash goes in first, with the old one beside it until the file is replaced.
            registry.upsert(entry(Some(current)));
            save_registry(&layout.registry, &registry)?;
            write_files(layout, &id, &manifest)?;
            registry.upsert(entry(None));
            save_registry(&layout.registry, &registry)?;
            Change::Updated
        }
    };
    host_config::enable(&layout.config_json, &id)?;
    Ok(change)
}

/// Makes sure the whitelisted sidecar of `directory` exists, keeping the rules it already has, and
/// returns its id with what was done to its files. Used when a task is created: the user's own
/// rules for the folder must survive it.
pub(super) fn ensure_workspace(
    layout: &Layout,
    executable: &str,
    directory: &Path,
    roots: &[PathBuf],
    now_ms: u64,
) -> Result<(String, Change)> {
    let mut spec = Spec {
        kind: Kind::Workspace,
        directory: directory.to_path_buf(),
        rules: Vec::new(),
    };
    let id = spec.id();
    if let Some(existing) = entry(layout, &id)? {
        spec.rules = existing.rules;
    }
    let change = ensure(layout, executable, &spec, roots, now_ms)?;
    Ok((id, change))
}

/// The folder the control sidecar is attached to: an empty one of AskHuman's own, so the App's
/// project list does not gain a folder of the user's for it.
pub(super) fn control_directory() -> PathBuf {
    crate::paths::config_dir().join("antigravity-control")
}

/// Makes the control sidecar exist, with its private folder, and returns its id with what was
/// done to its files. It has no grants and no project of the user's; it only carries messages.
pub(super) fn ensure_control(
    layout: &Layout,
    executable: &str,
    directory: &Path,
    now_ms: u64,
) -> Result<(String, Change)> {
    std::fs::create_dir_all(directory)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(directory, std::fs::Permissions::from_mode(0o700));
    }
    let spec = Spec {
        kind: Kind::Control,
        directory: std::fs::canonicalize(directory)?,
        rules: Vec::new(),
    };
    let id = spec.id();
    let change = ensure(layout, executable, &spec, &[], now_ms)?;
    Ok((id, change))
}

/// Replaces the rules of the sidecar `id`. A YOLO or control sidecar has nothing to add to.
pub(super) fn set_rules(
    layout: &Layout,
    executable: &str,
    id: &str,
    rules: &[String],
    roots: &[PathBuf],
    now_ms: u64,
) -> Result<Change> {
    if !valid_id(id) {
        return Err(NotManaged(id.to_string()).into());
    }
    let entry = {
        let _guard = LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        load_registry(&layout.registry)?.get(id).cloned()
    }
    .ok_or_else(|| NotManaged(id.to_string()))?;
    if entry.kind != Kind::Workspace {
        bail!("Only a whitelisted sidecar takes rules of its own");
    }
    let spec = Spec {
        rules: rules.to_vec(),
        ..entry.spec()
    };
    ensure(layout, executable, &spec, roots, now_ms)
}

/// What [`remove`] took away.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Removed {
    /// The sidecar's directory was deleted (it may already have been gone).
    pub directory: bool,
}

/// Removes the sidecar `id`: its directory first (the App stops the process), then its entry in
/// `config.json`, then the registry entry, so an interruption is finished by repeating the call.
/// What the App made for it (its project, its data and the conversations) stays.
pub(super) fn remove(layout: &Layout, id: &str) -> Result<Removed> {
    if !valid_id(id) {
        return Err(NotManaged(id.to_string()).into());
    }
    let _guard = LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let mut registry = load_registry(&layout.registry)?;
    let entry = registry
        .get(id)
        .cloned()
        .ok_or_else(|| NotManaged(id.to_string()))?;
    let directory = match inspect(layout, id, Some(&entry))? {
        Dir::Foreign(reason) => return Err(Conflict(reason).into()),
        Dir::Absent => false,
        Dir::Ours { .. } => {
            let dir = layout.sidecars.join(id);
            std::fs::remove_dir_all(&dir)
                .with_context(|| format!("Could not remove {}", dir.display()))?;
            true
        }
    };
    host_config::remove(&layout.config_json, id)?;
    registry.sidecars.retain(|item| item.id != id);
    save_registry(&layout.registry, &registry)?;
    Ok(Removed { directory })
}

/// Removes every sidecar AskHuman made; one that cannot be removed is reported and the others
/// still go.
pub(super) fn remove_all(layout: &Layout) -> Result<Vec<(String, Result<Removed>)>> {
    let ids: Vec<String> = {
        let _guard = LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        load_registry(&layout.registry)?
            .sidecars
            .into_iter()
            .map(|entry| entry.id)
            .collect()
    };
    Ok(ids
        .into_iter()
        .map(|id| {
            let result = remove(layout, &id);
            (id, result)
        })
        .collect())
}

// MARK: - Listing

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Files {
    /// The directory is there and is AskHuman's.
    Present,
    /// The directory is gone: the sidecar is not running until it is made again.
    Missing,
    /// Something in the directory is not AskHuman's: it is left alone.
    Conflict,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Listing {
    pub id: String,
    pub kind: Kind,
    pub directory: String,
    /// The project folder still exists.
    pub directory_exists: bool,
    pub rules: Vec<String>,
    pub created_at: u64,
    pub last_used_at: u64,
    pub files: Files,
    /// Why [`Files::Conflict`].
    #[serde(skip_serializing_if = "Option::is_none")]
    pub conflict: Option<String>,
    /// `config.json` switches it on.
    pub enabled: bool,
    pub project_id: Option<String>,
    /// AskHuman's registry knows it. A marked directory it does not know is shown but never
    /// changed.
    pub tracked: bool,
}

/// What AskHuman manages, as Settings and the command line show it.
pub(super) fn list(layout: &Layout) -> Result<Vec<Listing>> {
    let _guard = LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let registry = load_registry(&layout.registry)?;
    let config = std::fs::read_to_string(&layout.config_json).unwrap_or_default();
    let mut listings = Vec::new();
    for entry in &registry.sidecars {
        let (files, conflict) = match inspect(layout, &entry.id, Some(entry))? {
            Dir::Absent => (Files::Missing, None),
            Dir::Ours { .. } => (Files::Present, None),
            Dir::Foreign(reason) => (Files::Conflict, Some(reason)),
        };
        let host = host_config::read_entry(&config, &entry.id).unwrap_or_default();
        listings.push(Listing {
            id: entry.id.clone(),
            kind: entry.kind,
            directory: entry.directory.clone(),
            directory_exists: Path::new(&entry.directory).is_dir(),
            rules: entry.rules.clone(),
            created_at: entry.created_at,
            last_used_at: entry.last_used_at,
            files,
            conflict,
            enabled: host.enabled,
            project_id: host.project_id,
            tracked: true,
        });
    }
    // Directories AskHuman marked but its registry lost: shown, never touched.
    if let Ok(children) = std::fs::read_dir(&layout.sidecars) {
        let mut untracked: Vec<String> = children
            .filter_map(|child| child.ok())
            .map(|child| child.file_name().to_string_lossy().into_owned())
            .filter(|name| valid_id(name) && registry.get(name).is_none())
            .collect();
        untracked.sort();
        for id in untracked {
            let dir = layout.sidecars.join(&id);
            let marked = read_small(&dir.join(MARKER))
                .ok()
                .and_then(|bytes| serde_json::from_slice::<Marker>(&bytes).ok())
                .is_some_and(|marker| marker.id == id && marker.instance == layout.instance);
            if !marked {
                continue;
            }
            let manifest = read_small(&dir.join(MANIFEST))
                .ok()
                .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok());
            let directory = manifest
                .as_ref()
                .and_then(|value| value["agent_permissions"]["workspace_uris"][0].as_str())
                .unwrap_or_default()
                .trim_start_matches("file://")
                .to_string();
            let host = host_config::read_entry(&config, &id).unwrap_or_default();
            listings.push(Listing {
                kind: kind_from_id(&id),
                directory_exists: Path::new(&directory).is_dir(),
                directory,
                id,
                rules: Vec::new(),
                created_at: 0,
                last_used_at: 0,
                files: Files::Present,
                conflict: None,
                enabled: host.enabled,
                project_id: host.project_id,
                tracked: false,
            });
        }
    }
    Ok(listings)
}

fn kind_from_id(id: &str) -> Kind {
    if id.starts_with(Kind::Yolo.prefix()) {
        Kind::Yolo
    } else if id == Kind::Control.prefix() {
        Kind::Control
    } else {
        Kind::Workspace
    }
}

/// The sidecar `id` as AskHuman recorded it.
pub(super) fn entry(layout: &Layout, id: &str) -> Result<Option<Entry>> {
    let _guard = LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    Ok(load_registry(&layout.registry)?.get(id).cloned())
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXE: &str = "/Applications/AskHuman.app/Contents/MacOS/AskHuman";

    struct Home {
        _dir: tempfile::TempDir,
        layout: Layout,
        root: PathBuf,
    }

    fn home() -> Home {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        let config = root.join(".gemini/config");
        std::fs::create_dir_all(config.join("sidecars")).unwrap();
        std::fs::create_dir_all(config.join("projects")).unwrap();
        std::fs::write(
            config.join("config.json"),
            "{\n  \"userSettings\": {\n    \"globalPermissionGrants\": {\n      \"allow\": [\n        \"command(ls)\"\n      ]\n    }\n  }\n}\n",
        )
        .unwrap();
        let layout = Layout {
            sidecars: config.join("sidecars"),
            config_json: config.join("config.json"),
            registry: root.join(".askhuman/antigravity-sidecars.json"),
            instance: root.join(".askhuman").to_string_lossy().into_owned(),
        };
        Home {
            _dir: dir,
            layout,
            root,
        }
    }

    fn spec(kind: Kind, directory: &str) -> Spec {
        Spec {
            kind,
            directory: PathBuf::from(directory),
            rules: Vec::new(),
        }
    }

    fn roots() -> Vec<PathBuf> {
        vec![PathBuf::from("/var/folders/ab/T/askhuman")]
    }

    fn manifest_of(home: &Home, id: &str) -> Value {
        let text = std::fs::read_to_string(home.layout.sidecars.join(id).join(MANIFEST)).unwrap();
        serde_json::from_str(&text).unwrap()
    }

    fn config_of(home: &Home) -> Value {
        serde_json::from_str(&std::fs::read_to_string(&home.layout.config_json).unwrap()).unwrap()
    }

    /// What the App does to the file: it rewrites it with fields of its own.
    fn app_sets(home: &Home, id: &str, field: &str, value: Value) {
        let mut config = config_of(home);
        config["sidecars"][id][field] = value;
        std::fs::write(
            &home.layout.config_json,
            serde_json::to_string_pretty(&config).unwrap(),
        )
        .unwrap();
    }

    #[test]
    fn ids_come_from_the_kind_and_the_folder() {
        let ws = spec(Kind::Workspace, "/Users/me/project");
        let yolo = spec(Kind::Yolo, "/Users/me/project");
        let id = ws.id();
        assert!(id.starts_with("askhuman-ws-"), "{id}");
        assert_eq!(id.len(), "askhuman-ws-".len() + 8);
        assert!(id["askhuman-ws-".len()..]
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit()));
        // The same folder gives the same id; another folder, another; YOLO has its own family.
        assert_eq!(id, spec(Kind::Workspace, "/Users/me/project").id());
        assert_ne!(id, spec(Kind::Workspace, "/Users/me/other").id());
        assert!(yolo.id().starts_with("askhuman-yolo-"));
        assert_eq!(
            yolo.id()["askhuman-yolo-".len()..],
            id["askhuman-ws-".len()..]
        );
        assert_eq!(spec(Kind::Control, "/anything").id(), "askhuman-ctl");
        // Every id is one the rest of the module accepts as naming a directory of its own.
        for spec in [&ws, &yolo, &spec_control()] {
            assert!(valid_id(&spec.id()), "{}", spec.id());
        }
    }

    fn spec_control() -> Spec {
        spec(Kind::Control, "/private/control")
    }

    #[test]
    fn an_id_from_outside_cannot_name_another_path() {
        for id in [
            "",
            "askhuman-",
            "swarm/swarm_board",
            "askhuman-../../x",
            "askhuman-ws-1a2b3c4d/..",
            "other",
            &format!("askhuman-{}", "a".repeat(80)),
        ] {
            assert!(!valid_id(id), "{id:?}");
        }
        assert!(valid_id("askhuman-ws-1a2b3c4d"));
    }

    #[test]
    fn a_path_becomes_a_file_uri_with_everything_unusual_escaped() {
        assert_eq!(
            file_uri(Path::new("/Users/me/project")).unwrap(),
            "file:///Users/me/project"
        );
        assert_eq!(
            file_uri(Path::new("/Users/me/My Project/ä(1)#")).unwrap(),
            "file:///Users/me/My%20Project/%C3%A4%281%29%23"
        );
        assert!(file_uri(Path::new("relative/path")).is_err());
    }

    #[test]
    fn the_manifest_has_the_documented_shape_and_the_whitelist_grants() {
        let mut ws = spec(Kind::Workspace, "/Users/me/project");
        ws.rules = vec!["command(git)".into(), "read_url(example.com)".into()];
        let value: Value =
            serde_json::from_slice(&render_manifest(EXE, &ws, &roots()).unwrap()).unwrap();
        assert_eq!(value["command"], EXE);
        assert_eq!(
            value["args"],
            json!(["__antigravity-sidecar", "--id", ws.id()])
        );
        assert_eq!(value["restart_policy"], "always");
        assert_eq!(value["display_name"], "AskHuman · project");
        assert!(value["description"]
            .as_str()
            .unwrap()
            .contains("/Users/me/project"));
        let permissions = &value["agent_permissions"];
        assert_eq!(
            permissions["workspace_uris"],
            json!(["file:///Users/me/project"])
        );
        assert_eq!(permissions["allows_user_interactions"], false);
        // AskHuman's own MCP tool, the attachment folder read-only, then the user's rules.
        assert_eq!(
            permissions["access_grants"],
            json!([
                "mcp(askhuman_mcp/*)",
                "read_file(/var/folders/ab/T/askhuman)",
                "command(git)",
                "read_url(example.com)"
            ])
        );
        // The same spec always renders the same bytes.
        assert_eq!(
            render_manifest(EXE, &ws, &roots()).unwrap(),
            render_manifest(EXE, &ws, &roots()).unwrap()
        );
    }

    #[test]
    fn a_yolo_sidecar_is_named_as_such_and_grants_everything() {
        let yolo = spec(Kind::Yolo, "/Users/me/project");
        let value: Value =
            serde_json::from_slice(&render_manifest(EXE, &yolo, &roots()).unwrap()).unwrap();
        assert_eq!(value["display_name"], "AskHuman · project · YOLO");
        assert!(value["description"].as_str().unwrap().contains("WARNING"));
        assert_eq!(
            value["agent_permissions"]["access_grants"],
            json!([
                "command(*)",
                "read_file(*)",
                "write_file(*)",
                "read_url(*)",
                "mcp(*)"
            ])
        );
    }

    #[test]
    fn the_control_sidecar_has_no_grants_and_no_folder_of_the_users() {
        let value: Value =
            serde_json::from_slice(&render_manifest(EXE, &spec_control(), &roots()).unwrap())
                .unwrap();
        assert_eq!(value["agent_permissions"]["access_grants"], json!([]));
        assert_eq!(
            value["agent_permissions"]["workspace_uris"],
            json!(["file:///private/control"])
        );
    }

    #[test]
    fn a_relative_executable_or_folder_is_refused_and_unsafe_attachment_roots_are_left_out() {
        let ws = spec(Kind::Workspace, "/Users/me/project");
        assert!(render_manifest("AskHuman", &ws, &roots()).is_err());
        assert!(render_manifest(EXE, &spec(Kind::Workspace, "relative"), &roots()).is_err());
        let value: Value = serde_json::from_slice(
            &render_manifest(
                EXE,
                &ws,
                &[
                    PathBuf::from("/tmp/ask(human)"),
                    PathBuf::from("relative"),
                    PathBuf::from("/tmp/askhuman"),
                ],
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(
            value["agent_permissions"]["access_grants"],
            json!(["mcp(askhuman_mcp/*)", "read_file(/tmp/askhuman)"])
        );
    }

    #[test]
    fn rules_are_grants_and_a_whitelist_never_allows_everything() {
        for rule in [
            "command(git)",
            "read_file(/Users/me/data)",
            "write_file(/Users/me/out/*)",
            "mcp(askhuman_mcp/ask)",
            "mcp(other/*)",
            "read_url(example.com)",
        ] {
            assert!(validate_rule(rule, Kind::Workspace).is_ok(), "{rule}");
        }
        for rule in [
            "",
            "command",
            "command()",
            "shell(ls)",
            "command(git",
            " command(git)",
            "command(git) ",
            "command(*)",
            "mcp(*)",
            "read_file(*)",
            "command(a\nb)",
            &format!("command({})", "x".repeat(600)),
        ] {
            assert!(validate_rule(rule, Kind::Workspace).is_err(), "{rule:?}");
        }
        // Allowing everything is what a YOLO sidecar is for.
        assert!(validate_rule("command(*)", Kind::Yolo).is_ok());
        let rules = validate_rules(
            &[
                "command(git)".into(),
                "command(git)".into(),
                "command(ls)".into(),
            ],
            Kind::Workspace,
        )
        .unwrap();
        assert_eq!(rules, vec!["command(git)", "command(ls)"]);
        let many: Vec<String> = (0..=MAX_RULES).map(|n| format!("command(c{n})")).collect();
        assert!(validate_rules(&many, Kind::Workspace).is_err());
    }

    #[test]
    fn making_a_sidecar_writes_the_files_the_registry_entry_and_switches_it_on() {
        let home = home();
        let ws = spec(Kind::Workspace, "/Users/me/project");
        let id = ws.id();
        assert_eq!(
            ensure(&home.layout, EXE, &ws, &roots(), 1_000).unwrap(),
            Change::Created
        );
        let dir = home.layout.sidecars.join(&id);
        let mut names: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        assert_eq!(names, vec![".askhuman-managed", "sidecar.json"]);
        let marker: Value =
            serde_json::from_slice(&std::fs::read(dir.join(MARKER)).unwrap()).unwrap();
        assert_eq!(marker["id"], id.as_str());
        assert_eq!(marker["instance"], home.layout.instance.as_str());
        // The registry holds what the file was made from and its hash.
        let entry = entry(&home.layout, &id).unwrap().unwrap();
        assert_eq!(entry.directory, "/Users/me/project");
        assert_eq!(entry.created_at, 1_000);
        assert_eq!(
            entry.manifest_sha256,
            digest(&std::fs::read(dir.join(MANIFEST)).unwrap())
        );
        assert_eq!(entry.previous_manifest_sha256, None);
        // The App is told to start it, and what the user had in the file is still there.
        let config = config_of(&home);
        assert_eq!(config["sidecars"][&id]["enabled"], true);
        assert_eq!(
            config["userSettings"]["globalPermissionGrants"]["allow"][0],
            "command(ls)"
        );
    }

    #[test]
    fn making_it_again_changes_no_file_and_only_notes_the_time() {
        let home = home();
        let ws = spec(Kind::Workspace, "/Users/me/project");
        ensure(&home.layout, EXE, &ws, &roots(), 1_000).unwrap();
        let id = ws.id();
        let before = std::fs::read(home.layout.sidecars.join(&id).join(MANIFEST)).unwrap();
        assert_eq!(
            ensure(&home.layout, EXE, &ws, &roots(), 2_000).unwrap(),
            Change::Unchanged
        );
        assert_eq!(
            std::fs::read(home.layout.sidecars.join(&id).join(MANIFEST)).unwrap(),
            before
        );
        let entry = entry(&home.layout, &id).unwrap().unwrap();
        assert_eq!((entry.created_at, entry.last_used_at), (1_000, 2_000));
    }

    #[test]
    fn a_switched_off_sidecar_is_switched_on_again_for_a_new_task() {
        let home = home();
        let ws = spec(Kind::Workspace, "/Users/me/project");
        ensure(&home.layout, EXE, &ws, &roots(), 1).unwrap();
        app_sets(&home, &ws.id(), "enabled", json!(false));
        assert_eq!(config_of(&home)["sidecars"][ws.id()]["enabled"], false);
        assert_eq!(
            ensure(&home.layout, EXE, &ws, &roots(), 2).unwrap(),
            Change::Unchanged
        );
        assert_eq!(config_of(&home)["sidecars"][ws.id()]["enabled"], true);
    }

    #[test]
    fn new_rules_reach_the_file_through_the_registry() {
        let home = home();
        let ws = spec(Kind::Workspace, "/Users/me/project");
        let id = ws.id();
        ensure(&home.layout, EXE, &ws, &roots(), 1).unwrap();
        let old_hash = entry(&home.layout, &id).unwrap().unwrap().manifest_sha256;
        let change = set_rules(
            &home.layout,
            EXE,
            &id,
            &["command(git)".to_string()],
            &roots(),
            2,
        )
        .unwrap();
        assert_eq!(change, Change::Updated);
        let grants = &manifest_of(&home, &id)["agent_permissions"]["access_grants"];
        assert!(grants.as_array().unwrap().contains(&json!("command(git)")));
        let entry = entry(&home.layout, &id).unwrap().unwrap();
        assert_eq!(entry.rules, vec!["command(git)"]);
        assert_ne!(entry.manifest_sha256, old_hash);
        assert_eq!(entry.previous_manifest_sha256, None);
        // An update only the registry knows about is still recognised as ours afterwards.
        assert_eq!(
            ensure(
                &home.layout,
                EXE,
                &Spec {
                    rules: vec!["command(git)".into()],
                    ..ws.clone()
                },
                &roots(),
                3
            )
            .unwrap(),
            Change::Unchanged
        );
        // A rule that allows everything is refused and the file is as it was.
        assert!(set_rules(&home.layout, EXE, &id, &["mcp(*)".to_string()], &roots(), 4).is_err());
        assert_eq!(
            entry_hash(&home, &id),
            digest(&std::fs::read(home.layout.sidecars.join(&id).join(MANIFEST)).unwrap())
        );
        // Only a whitelist takes rules, and only a sidecar of ours.
        let yolo = spec(Kind::Yolo, "/Users/me/project");
        ensure(&home.layout, EXE, &yolo, &roots(), 5).unwrap();
        assert!(set_rules(&home.layout, EXE, &yolo.id(), &[], &roots(), 6).is_err());
        let error =
            set_rules(&home.layout, EXE, "askhuman-ws-00000000", &[], &roots(), 7).unwrap_err();
        assert!(error.downcast_ref::<NotManaged>().is_some());
    }

    fn entry_hash(home: &Home, id: &str) -> String {
        entry(&home.layout, id).unwrap().unwrap().manifest_sha256
    }

    #[test]
    fn creating_a_task_keeps_the_rules_the_user_gave_the_folder() {
        let home = home();
        let directory = Path::new("/Users/me/project");
        let (id, change) =
            ensure_workspace(&home.layout, EXE, directory, &roots(), 1).expect("first call");
        assert_eq!(change, Change::Created);
        assert_eq!(id, spec(Kind::Workspace, "/Users/me/project").id());
        // The user adds a rule in Settings; the next task must not take it away.
        set_rules(
            &home.layout,
            EXE,
            &id,
            &["command(git)".to_string()],
            &roots(),
            2,
        )
        .unwrap();
        let (again, change) =
            ensure_workspace(&home.layout, EXE, directory, &roots(), 3).expect("second call");
        assert_eq!(again, id);
        assert_eq!(change, Change::Unchanged);
        assert_eq!(
            entry(&home.layout, &id).unwrap().unwrap().rules,
            vec!["command(git)"]
        );
        let grants = &manifest_of(&home, &id)["agent_permissions"]["access_grants"];
        assert!(grants.as_array().unwrap().contains(&json!("command(git)")));
    }

    #[test]
    fn a_file_somebody_edited_is_a_conflict_and_is_left_alone() {
        let home = home();
        let ws = spec(Kind::Workspace, "/Users/me/project");
        let id = ws.id();
        ensure(&home.layout, EXE, &ws, &roots(), 1).unwrap();
        let path = home.layout.sidecars.join(&id).join(MANIFEST);
        let edited = std::fs::read_to_string(&path)
            .unwrap()
            .replace("AskHuman · project", "My own name");
        std::fs::write(&path, &edited).unwrap();
        let rules = Spec {
            rules: vec!["command(git)".into()],
            ..ws.clone()
        };
        let error = ensure(&home.layout, EXE, &rules, &roots(), 2).unwrap_err();
        assert!(error.downcast_ref::<Conflict>().is_some(), "{error}");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), edited);
        let error = remove(&home.layout, &id).unwrap_err();
        assert!(error.downcast_ref::<Conflict>().is_some(), "{error}");
        assert!(path.exists());
        assert!(entry(&home.layout, &id).unwrap().is_some());
        // It shows up as a conflict, with the reason.
        let listing = list(&home.layout).unwrap();
        assert_eq!(listing[0].files, Files::Conflict);
        assert!(listing[0].conflict.as_deref().unwrap().contains("changed"));
    }

    #[test]
    fn a_directory_of_that_name_that_is_not_provably_ours_is_never_claimed() {
        let home = home();
        let ws = spec(Kind::Workspace, "/Users/me/project");
        let id = ws.id();
        let dir = home.layout.sidecars.join(&id);
        // Somebody else's directory, with no record of it in the registry.
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(MANIFEST), "{}").unwrap();
        let error = ensure(&home.layout, EXE, &ws, &roots(), 1).unwrap_err();
        assert!(error.downcast_ref::<Conflict>().is_some(), "{error}");
        assert_eq!(std::fs::read_to_string(dir.join(MANIFEST)).unwrap(), "{}");
        assert!(config_of(&home)["sidecars"].get(&id).is_none());
        // A directory another installation marked is not ours either, even with a registry entry.
        std::fs::remove_dir_all(&dir).unwrap();
        ensure(&home.layout, EXE, &ws, &roots(), 2).unwrap();
        std::fs::write(
            dir.join(MARKER),
            serde_json::to_vec(&Marker {
                id: id.clone(),
                instance: "/somewhere/else/.askhuman".into(),
            })
            .unwrap(),
        )
        .unwrap();
        let error = ensure(&home.layout, EXE, &ws, &roots(), 3).unwrap_err();
        assert!(error.downcast_ref::<Conflict>().is_some(), "{error}");
        assert!(error.to_string().contains("another AskHuman"), "{error}");
        // A file we never wrote makes it foreign too.
        std::fs::write(
            dir.join(MARKER),
            serde_json::to_vec(&Marker {
                id: id.clone(),
                instance: home.layout.instance.clone(),
            })
            .unwrap(),
        )
        .unwrap();
        std::fs::write(dir.join("notes.txt"), "mine").unwrap();
        assert!(ensure(&home.layout, EXE, &ws, &roots(), 4).is_err());
        assert!(remove(&home.layout, &id).is_err());
        assert!(dir.join("notes.txt").exists());
    }

    #[test]
    fn the_twenty_first_sidecar_is_refused_but_an_existing_one_can_still_change() {
        let home = home();
        for n in 0..LIMIT {
            ensure(
                &home.layout,
                EXE,
                &spec(Kind::Workspace, &format!("/Users/me/p{n}")),
                &roots(),
                n as u64,
            )
            .unwrap();
        }
        let error = ensure(
            &home.layout,
            EXE,
            &spec(Kind::Workspace, "/Users/me/one-too-many"),
            &roots(),
            99,
        )
        .unwrap_err();
        assert!(error.downcast_ref::<LimitReached>().is_some(), "{error}");
        assert!(!home
            .layout
            .sidecars
            .join(spec(Kind::Workspace, "/Users/me/one-too-many").id())
            .exists());
        // The same folder as YOLO is another sidecar and counts as one more.
        assert!(ensure(
            &home.layout,
            EXE,
            &spec(Kind::Yolo, "/Users/me/p0"),
            &roots(),
            99
        )
        .is_err());
        // An existing one is not a new one.
        let again = Spec {
            rules: vec!["command(git)".into()],
            ..spec(Kind::Workspace, "/Users/me/p3")
        };
        assert_eq!(
            ensure(&home.layout, EXE, &again, &roots(), 100).unwrap(),
            Change::Updated
        );
    }

    #[test]
    fn removing_takes_the_directory_then_the_config_entry_then_the_record_and_leaves_the_apps_data()
    {
        let home = home();
        let ws = spec(Kind::Workspace, "/Users/me/project");
        let id = ws.id();
        ensure(&home.layout, EXE, &ws, &roots(), 1).unwrap();
        // What the App creates for a sidecar stays: AskHuman never removes it.
        let project = home.root.join(format!(".gemini/config/projects/{id}.json"));
        std::fs::write(&project, "{}").unwrap();
        let removed = remove(&home.layout, &id).unwrap();
        assert!(removed.directory);
        assert!(!home.layout.sidecars.join(&id).exists());
        assert!(config_of(&home)["sidecars"].get(&id).is_none());
        assert_eq!(entry(&home.layout, &id).unwrap(), None);
        assert!(project.exists());
        assert_eq!(
            config_of(&home)["userSettings"]["globalPermissionGrants"]["allow"][0],
            "command(ls)"
        );
        // Removing what is gone says so.
        let error = remove(&home.layout, &id).unwrap_err();
        assert!(error.downcast_ref::<NotManaged>().is_some());
        assert!(remove(&home.layout, "../../etc").is_err());
    }

    #[test]
    fn a_removal_that_was_interrupted_is_finished_by_repeating_it() {
        let home = home();
        let ws = spec(Kind::Workspace, "/Users/me/project");
        let id = ws.id();
        ensure(&home.layout, EXE, &ws, &roots(), 1).unwrap();
        // The directory is gone (the first step ran), the config entry and the record are not.
        std::fs::remove_dir_all(home.layout.sidecars.join(&id)).unwrap();
        let removed = remove(&home.layout, &id).unwrap();
        assert!(!removed.directory);
        assert!(config_of(&home)["sidecars"].get(&id).is_none());
        assert_eq!(entry(&home.layout, &id).unwrap(), None);
    }

    #[test]
    fn a_making_that_was_interrupted_is_finished_by_the_next_call() {
        let home = home();
        let ws = spec(Kind::Workspace, "/Users/me/project");
        let id = ws.id();
        ensure(&home.layout, EXE, &ws, &roots(), 1).unwrap();
        let dir = home.layout.sidecars.join(&id);
        // Only the registry entry survived: no directory.
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(
            ensure(&home.layout, EXE, &ws, &roots(), 2).unwrap(),
            Change::Created
        );
        // Only an empty directory was made.
        std::fs::remove_dir_all(&dir).unwrap();
        std::fs::create_dir(&dir).unwrap();
        assert_eq!(
            ensure(&home.layout, EXE, &ws, &roots(), 3).unwrap(),
            Change::Updated
        );
        // The marker is there but the manifest never was.
        std::fs::remove_file(dir.join(MANIFEST)).unwrap();
        assert_eq!(
            ensure(&home.layout, EXE, &ws, &roots(), 4).unwrap(),
            Change::Updated
        );
        assert!(dir.join(MANIFEST).exists());
        // The new hash was recorded and the file not yet replaced: the old file is still ours.
        let wider = Spec {
            rules: vec!["command(ls)".to_string()],
            ..ws.clone()
        };
        let new_hash = digest(&render_manifest(EXE, &wider, &roots()).unwrap());
        let mut registry = load_registry(&home.layout.registry).unwrap();
        let old = registry.get(&id).unwrap().manifest_sha256.clone();
        assert_ne!(old, new_hash);
        let recorded = registry.sidecars.first_mut().unwrap();
        recorded.rules = wider.rules.clone();
        recorded.manifest_sha256 = new_hash.clone();
        recorded.previous_manifest_sha256 = Some(old);
        save_registry(&home.layout.registry, &registry).unwrap();
        assert_eq!(
            ensure(&home.layout, EXE, &wider, &roots(), 5).unwrap(),
            Change::Updated
        );
        assert_eq!(entry_hash(&home, &id), new_hash);
        assert_eq!(
            digest(&std::fs::read(dir.join(MANIFEST)).unwrap()),
            new_hash
        );
    }

    #[test]
    fn the_list_shows_the_state_of_each_sidecar_and_the_directories_it_cannot_vouch_for() {
        let home = home();
        let kept = spec(Kind::Workspace, "/Users/me/kept");
        let gone = spec(Kind::Yolo, "/Users/me/gone");
        ensure(&home.layout, EXE, &kept, &roots(), 10).unwrap();
        ensure(&home.layout, EXE, &gone, &roots(), 20).unwrap();
        // The App added its project id to our entry.
        app_sets(&home, &kept.id(), "projectId", json!("p-1"));
        std::fs::remove_dir_all(home.layout.sidecars.join(gone.id())).unwrap();
        // A directory AskHuman marked whose registry entry was lost: shown, read-only.
        let lost = spec(Kind::Workspace, "/Users/me/lost");
        ensure(&home.layout, EXE, &lost, &roots(), 30).unwrap();
        let mut registry = load_registry(&home.layout.registry).unwrap();
        registry.sidecars.retain(|entry| entry.id != lost.id());
        save_registry(&home.layout.registry, &registry).unwrap();
        // And a directory that is not ours at all is not listed.
        std::fs::create_dir_all(home.layout.sidecars.join("askhuman-ws-deadbeef")).unwrap();
        std::fs::create_dir_all(home.layout.sidecars.join("swarm")).unwrap();
        let listing = list(&home.layout).unwrap();
        assert_eq!(listing.len(), 3);
        let find = |id: &str| listing.iter().find(|item| item.id == id).unwrap();
        let kept_item = find(&kept.id());
        assert_eq!(
            (kept_item.files, kept_item.enabled, kept_item.tracked),
            (Files::Present, true, true)
        );
        assert_eq!(kept_item.project_id.as_deref(), Some("p-1"));
        assert_eq!(kept_item.last_used_at, 10);
        assert!(!kept_item.directory_exists);
        let gone_item = find(&gone.id());
        assert_eq!(
            (gone_item.files, gone_item.kind),
            (Files::Missing, Kind::Yolo)
        );
        let lost_item = find(&lost.id());
        assert!(!lost_item.tracked);
        assert_eq!(lost_item.directory, "/Users/me/lost");
        assert_eq!(lost_item.files, Files::Present);
        // The lost one is never changed by anything: it is a conflict to AskHuman.
        let error = remove(&home.layout, &lost.id()).unwrap_err();
        assert!(error.downcast_ref::<NotManaged>().is_some());
        assert!(home.layout.sidecars.join(lost.id()).exists());
    }

    #[test]
    fn removing_all_carries_on_past_one_that_cannot_be_removed() {
        let home = home();
        let a = spec(Kind::Workspace, "/Users/me/a");
        let b = spec(Kind::Workspace, "/Users/me/b");
        let c = spec(Kind::Yolo, "/Users/me/c");
        for spec in [&a, &b, &c] {
            ensure(&home.layout, EXE, spec, &roots(), 1).unwrap();
        }
        std::fs::write(home.layout.sidecars.join(b.id()).join("notes.txt"), "mine").unwrap();
        let results = remove_all(&home.layout).unwrap();
        assert_eq!(results.len(), 3);
        let failed: Vec<_> = results
            .iter()
            .filter(|(_, result)| result.is_err())
            .map(|(id, _)| id.clone())
            .collect();
        assert_eq!(failed, vec![b.id()]);
        assert!(!home.layout.sidecars.join(a.id()).exists());
        assert!(!home.layout.sidecars.join(c.id()).exists());
        assert!(home.layout.sidecars.join(b.id()).join("notes.txt").exists());
        let left = load_registry(&home.layout.registry).unwrap();
        assert_eq!(left.sidecars.len(), 1);
        assert_eq!(left.sidecars[0].id, b.id());
        let config = config_of(&home);
        assert!(config["sidecars"].get(a.id()).is_none());
        assert!(config["sidecars"].get(b.id()).is_some());
    }

    #[test]
    fn a_registry_that_cannot_be_read_blocks_changes_and_is_not_overwritten() {
        let home = home();
        std::fs::create_dir_all(home.layout.registry.parent().unwrap()).unwrap();
        std::fs::write(&home.layout.registry, "{ not json").unwrap();
        let ws = spec(Kind::Workspace, "/Users/me/project");
        let error = ensure(&home.layout, EXE, &ws, &roots(), 1).unwrap_err();
        assert!(error.to_string().contains("cannot be read"), "{error}");
        assert!(list(&home.layout).is_err());
        assert!(remove(&home.layout, &ws.id()).is_err());
        assert_eq!(
            std::fs::read_to_string(&home.layout.registry).unwrap(),
            "{ not json"
        );
        assert!(!home.layout.sidecars.join(ws.id()).exists());
        // One written by a newer AskHuman is not rewritten by this one either.
        std::fs::write(&home.layout.registry, r#"{"version": 99, "sidecars": []}"#).unwrap();
        let error = ensure(&home.layout, EXE, &ws, &roots(), 1).unwrap_err();
        assert!(error.to_string().contains("newer"), "{error}");
    }

    #[test]
    fn the_registry_is_private_and_reads_back_what_was_saved() {
        let home = home();
        let ws = spec(Kind::Workspace, "/Users/me/project");
        ensure(&home.layout, EXE, &ws, &roots(), 1).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&home.layout.registry)
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
        let registry = load_registry(&home.layout.registry).unwrap();
        assert_eq!(registry.version, REGISTRY_VERSION);
        assert_eq!(registry.sidecars.len(), 1);
        let value: Value =
            serde_json::from_slice(&std::fs::read(&home.layout.registry).unwrap()).unwrap();
        assert_eq!(value["sidecars"][0]["kind"], "workspace");
        assert_eq!(value["sidecars"][0]["createdAt"], 1);
    }

    #[test]
    fn the_attachment_folders_are_inside_the_temporary_directory() {
        let roots = attachment_roots();
        assert!(!roots.is_empty());
        assert_eq!(roots[0], std::env::temp_dir().join("askhuman"));
        for root in &roots {
            assert!(root.is_absolute());
            // The on-disk spelling may differ in case on a case-insensitive volume.
            let name = root.file_name().unwrap().to_string_lossy().to_lowercase();
            assert_eq!(name, "askhuman", "{roots:?}");
        }
        let unique: std::collections::HashSet<_> = roots.iter().collect();
        assert_eq!(unique.len(), roots.len());
    }
}
