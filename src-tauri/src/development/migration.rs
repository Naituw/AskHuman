//! Explicit, journaled migration of owned integration artifacts. Backups never cross IPC.

use super::{routing, state};
use crate::integrations::{agent_mode, agent_rules::AgentTarget};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

pub const JOURNAL_ENV: &str = "ASKHUMAN_DEV_MIGRATION_ID";
const MAX_FILE: u64 = 16 * 1024 * 1024;

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Action {
    pub agent: String,
    pub mode: String,
    pub status: String,
    pub error: Option<String>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Artifact {
    pub path: PathBuf,
    pub before: Option<String>,
    pub known: Vec<Option<String>>,
    pub permissions: Option<u32>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Journal {
    pub version: u32,
    pub id: String,
    pub phase: String,
    pub previous_mode: state::RoutingMode,
    pub system_binary: PathBuf,
    pub actions: Vec<Action>,
    pub artifacts: Vec<Artifact>,
    pub conflicts: Vec<String>,
    pub error: Option<String>,
}

fn directory(id: &str) -> Result<PathBuf, String> {
    if uuid::Uuid::parse_str(id)
        .ok()
        .map(|v| v.to_string())
        .as_deref()
        != Some(id)
    {
        return Err("invalid migration ID".into());
    }
    Ok(state::directory().join("migrations").join(id))
}
fn save(journal: &Journal) -> Result<(), String> {
    state::write_json(&directory(&journal.id)?.join("journal.json"), journal)
}
pub fn read(id: &str) -> Result<Journal, String> {
    let bytes = read_file(&directory(id)?.join("journal.json"))?.ok_or("migration not found")?;
    let journal: Journal = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    if journal.version != 1 || journal.id != id {
        return Err("unsupported migration journal".into());
    }
    // A persisted path is never authority for an arbitrary file write.
    let allowed = artifact_paths();
    if journal.artifacts.iter().any(|a| !allowed.contains(&a.path)) {
        return Err("migration contains an unknown integration artifact".into());
    }
    Ok(journal)
}
fn require_system() -> Result<(), String> {
    let target = routing::inherited()?.ok_or("open migration through the explicit system entry")?;
    if target.kind != routing::TargetKind::System
        || !state::same_path(&target.home, &state::system_home())
        || std::env::var_os(super::session::PROFILE_ENV).is_some()
    {
        return Err("only the system instance can migrate everyday integrations".into());
    }
    Ok(())
}
fn artifact_paths() -> Vec<PathBuf> {
    use crate::paths as p;
    vec![
        p::claude_md(),
        p::claude_json(),
        p::claude_settings_json(),
        p::claude_hook_script(),
        p::codex_agents_md(),
        p::codex_config_toml(),
        p::codex_hooks_json(),
        p::cursor_rule_file(),
        p::cursor_mcp_json(),
        p::cursor_hooks_json(),
        p::cursor_hook_script(),
        p::legacy_cursor_hook_script(),
        p::grok_skill_md(),
        p::grok_config_toml(),
        p::grok_hooks_json(),
        p::pi_agents_md(),
        p::pi_extension_file(),
        p::antigravity_rule_file(),
        p::antigravity_mcp_config_json(),
        p::integration_state_file(),
        p::lifecycle_preferences_file(),
        p::permission_preferences_file(),
        p::stop_preferences_file(),
        p::ask_question_preferences_file(),
    ]
}
fn agent_paths(agent: &str) -> Vec<PathBuf> {
    use crate::paths as p;
    match agent {
        "claude" => vec![
            p::claude_md(),
            p::claude_json(),
            p::claude_settings_json(),
            p::claude_hook_script(),
        ],
        "codex" => vec![
            p::codex_agents_md(),
            p::codex_config_toml(),
            p::codex_hooks_json(),
        ],
        "cursor" => vec![
            p::cursor_rule_file(),
            p::cursor_mcp_json(),
            p::cursor_hooks_json(),
            p::cursor_hook_script(),
            p::legacy_cursor_hook_script(),
        ],
        "grok" => vec![
            p::grok_skill_md(),
            p::grok_config_toml(),
            p::grok_hooks_json(),
        ],
        "pi" => vec![p::pi_agents_md(), p::pi_extension_file()],
        "antigravity" => vec![p::antigravity_rule_file(), p::antigravity_mcp_config_json()],
        _ => vec![],
    }
}
fn read_file(path: &Path) -> Result<Option<Vec<u8>>, String> {
    let mut ancestor = Some(path);
    while let Some(current) = ancestor {
        match std::fs::symlink_metadata(current) {
            Ok(meta) if meta.file_type().is_symlink() => {
                return Err(format!(
                    "linked integration path requires manual review: {}",
                    current.display()
                ))
            }
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.to_string()),
        }
        ancestor = current.parent();
    }
    match std::fs::metadata(path) {
        Ok(meta) if !meta.is_file() || meta.len() > MAX_FILE => Err(format!(
            "invalid or oversized integration artifact: {}",
            path.display()
        )),
        Ok(_) => std::fs::read(path).map(Some).map_err(|e| e.to_string()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}
fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn current_hash(path: &Path) -> Result<Option<String>, String> {
    Ok(read_file(path)?.as_deref().map(hash))
}
fn backup_path(id: &str, index: usize) -> Result<PathBuf, String> {
    Ok(directory(id)?.join(format!("before-{index}")))
}
fn publish(path: &Path, bytes: &[u8], mode: Option<u32>) -> Result<(), String> {
    use std::io::Write;
    let parent = path.parent().ok_or("artifact has no parent")?;
    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let mut tmp = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
    tmp.write_all(bytes).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    if let Some(mode) = mode {
        use std::os::unix::fs::PermissionsExt;
        tmp.as_file()
            .set_permissions(std::fs::Permissions::from_mode(mode))
            .map_err(|e| e.to_string())?;
    }
    #[cfg(not(unix))]
    let _ = mode;
    tmp.as_file().sync_all().map_err(|e| e.to_string())?;
    tmp.persist(path).map_err(|e| e.to_string())?;
    Ok(())
}
fn permissions(path: &Path) -> Option<u32> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(path)
            .ok()
            .map(|m| m.permissions().mode() & 0o777)
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        None
    }
}

fn check_mcp(path: &Path, toml: bool) -> Result<(), String> {
    let Some(bytes) = read_file(path)? else {
        return Ok(());
    };
    let text = std::str::from_utf8(&bytes).map_err(|e| e.to_string())?;
    let entry: Option<serde_json::Value> = if toml {
        let doc = text.parse::<toml_edit::DocumentMut>().map_err(|_| {
            format!(
                "cannot parse MCP configuration; repair {} before migration",
                path.display()
            )
        })?;
        doc.get("mcp_servers")
            .and_then(|s| s.get("askhuman"))
            .map(|entry| {
                let command = entry.get("command").and_then(|v| v.as_str());
                let args = entry.get("args").and_then(|v| v.as_array()).map(|a| {
                    a.iter()
                        .map(|v| v.as_str().map(String::from))
                        .collect::<Vec<_>>()
                });
                let env = entry.get("env").and_then(|v| v.as_table_like()).map(|e| {
                    e.iter()
                        .map(|(k, v)| (k.to_string(), v.as_str().unwrap_or_default().to_string()))
                        .collect::<std::collections::BTreeMap<_, _>>()
                });
                serde_json::json!({"command":command,"args":args,"env":env})
            })
    } else {
        let doc: serde_json::Value =
            jsonc_parser::parse_to_serde_value::<serde_json::Value>(text, &Default::default())
                .map_err(|_| {
                    format!(
                        "cannot parse MCP configuration; repair {} before migration",
                        path.display()
                    )
                })?;
        let entry = doc.get("mcpServers").and_then(|s| s.get("askhuman"));
        if entry.and_then(|e| e.as_object()).is_some_and(|e| {
            e.keys().any(|key| {
                !matches!(
                    key.as_str(),
                    "command" | "args" | "env" | "timeout" | "timeoutSeconds"
                )
            })
        }) {
            return Err(format!(
                "custom MCP fields require manual review: {}",
                path.display()
            ));
        }
        entry.cloned()
    };
    let Some(entry) = entry else { return Ok(()) };
    let command = entry
        .get("command")
        .and_then(|v| v.as_str())
        .ok_or_else(|| format!("custom AskHuman MCP transport: {}", path.display()))?;
    let args = entry.get("args").and_then(|v| v.as_array());
    let owned_args = args.is_some_and(|args| {
        args == &vec![serde_json::json!("mcp")]
            || args
                == &vec![
                    serde_json::json!("--instance"),
                    serde_json::json!("system"),
                    serde_json::json!("mcp"),
                ]
    });
    if !owned_args {
        return Err(format!(
            "custom AskHuman MCP command requires manual review: {}",
            path.display()
        ));
    }
    let registered = state::read()?;
    let command_path = Path::new(command);
    let mut recognized = registered
        .system_binary
        .iter()
        .any(|p| state::same_path(p, command_path));
    recognized |= registered
        .worktrees
        .iter()
        .any(|p| state::same_path(&crate::dev_instance::instance_bin(p), command_path));
    if !command_path.is_absolute() || !recognized {
        return Err(format!(
            "unregistered AskHuman MCP executable requires manual review: {}",
            path.display()
        ));
    }
    if entry
        .get("env")
        .and_then(|v| v.as_object())
        .is_some_and(|env| {
            env.keys()
                .any(|k| k.starts_with("ASKHUMAN_") && k != "ASKHUMAN_NO_KEYCHAIN")
        })
    {
        return Err(format!(
            "custom AskHuman environment requires manual review: {}",
            path.display()
        ));
    }
    Ok(())
}

pub fn plan() -> Result<Journal, String> {
    require_system()?;
    let _lock = crate::file_lock::FileLock::exclusive(&state::directory().join("migration.lock"))
        .map_err(|e| e.to_string())?;
    if latest()?.is_some_and(|j| matches!(j.phase.as_str(), "partial" | "rollingBack")) {
        return Err(
            "finish or restore the incomplete migration before creating another plan".into(),
        );
    }
    let system = super::relay::system_target()?;
    let mut journal = Journal {
        version: 1,
        id: uuid::Uuid::new_v4().to_string(),
        phase: "planned".into(),
        previous_mode: state::read()?.mode,
        system_binary: system.binary,
        actions: vec![],
        artifacts: vec![],
        conflicts: vec![],
        error: None,
    };
    for agent in ["claude", "codex", "cursor", "grok", "pi", "antigravity"] {
        let target = AgentTarget::parse(agent).unwrap();
        if !crate::integrations::agent_rules::supported(target) {
            continue;
        }
        if let Err(e) = crate::integrations::agent_rules::validate_status(target) {
            journal.conflicts.push(format!("{agent}: {e}"));
            continue;
        }
        let mode = agent_mode::current(target);
        if mode != agent_mode::Mode::None {
            journal.actions.push(Action {
                agent: agent.into(),
                mode: mode.as_str().into(),
                status: "pending".into(),
                error: None,
            });
        }
    }
    use crate::paths as p;
    for (path, toml) in [
        (p::claude_json(), false),
        (p::codex_config_toml(), true),
        (p::cursor_mcp_json(), false),
        (p::grok_config_toml(), true),
        (p::antigravity_mcp_config_json(), false),
    ] {
        if let Err(e) = check_mcp(&path, toml) {
            journal.conflicts.push(e);
        }
    }
    let mut selected = journal
        .actions
        .iter()
        .flat_map(|action| agent_paths(&action.agent))
        .collect::<Vec<_>>();
    if !journal.actions.is_empty() {
        selected.extend([
            p::integration_state_file(),
            p::lifecycle_preferences_file(),
            p::permission_preferences_file(),
            p::stop_preferences_file(),
            p::ask_question_preferences_file(),
        ]);
    }
    for path in selected {
        match read_file(&path) {
            Ok(bytes) => {
                let before = bytes.as_deref().map(hash);
                let index = journal.artifacts.len();
                journal.artifacts.push(Artifact {
                    permissions: permissions(&path),
                    path,
                    before: before.clone(),
                    known: vec![before],
                });
                if let Some(bytes) = bytes {
                    publish(&backup_path(&journal.id, index)?, &bytes, Some(0o600))?;
                }
            }
            Err(e) => journal.conflicts.push(e),
        }
    }
    journal.conflicts.extend(quiescence_conflicts()?);
    save(&journal)?;
    Ok(journal)
}

fn quiescence_conflicts() -> Result<Vec<String>, String> {
    let mut conflicts = vec![];
    for root in state::read()?.worktrees {
        let home = crate::dev_instance::instance_home(&root);
        if let Some(bytes) = read_file(&home.join("agents.json"))? {
            let value: serde_json::Value = serde_json::from_slice(&bytes)
                .map_err(|e| format!("cannot inspect Dev sessions at {}: {e}", root.display()))?;
            if let Some(active) = value["active"].as_array() {
                for record in active {
                    let pid = record["pid"].as_u64().and_then(|p| u32::try_from(p).ok());
                    if pid.is_some_and(crate::agents::detect::pid_alive) || pid.is_none() {
                        conflicts.push(format!(
                            "close the active Dev Agent session before migration: {} (pid {})",
                            root.display(),
                            pid.unwrap_or(0)
                        ));
                    }
                }
            }
        }
    }
    // Existing Dev MCP connections must drain before their user-level bindings change.
    #[cfg(unix)]
    {
        let output = std::process::Command::new("ps")
            .args(["-axo", "pid=,comm="])
            .output()
            .map_err(|e| e.to_string())?;
        if !output.status.success() {
            return Err("cannot inspect existing MCP processes".into());
        }
        for line in String::from_utf8_lossy(&output.stdout).lines() {
            let line = line.trim();
            let Some((pid, command)) = line.split_once(char::is_whitespace) else {
                continue;
            };
            if command.contains(crate::dev_instance::DEV_DIR)
                && command.to_ascii_lowercase().contains("askhuman")
            {
                if let Ok(pid) = pid.parse::<u32>() {
                    let args = std::process::Command::new("ps")
                        .args(["-p", &pid.to_string(), "-o", "args="])
                        .output()
                        .map_err(|e| e.to_string())?;
                    if String::from_utf8_lossy(&args.stdout)
                        .split_whitespace()
                        .any(|v| v == "mcp")
                    {
                        conflicts.push(format!(
                            "restart the existing Dev MCP connection before migration (pid {pid})"
                        ));
                    }
                }
            }
        }
    }
    #[cfg(windows)]
    {
        let output = std::process::Command::new("powershell.exe").args(["-NoProfile","-NonInteractive","-Command", "Get-CimInstance Win32_Process -Filter \"Name = 'AskHuman.exe'\" | Where-Object { $_.ExecutablePath -like '*\\.askhuman-dev\\*' -and $_.CommandLine -match '(?:^|\\s)mcp(?:\\s|$)' } | Select-Object -ExpandProperty ProcessId"]).output().map_err(|e|e.to_string())?;
        if !output.status.success() {
            return Err("cannot inspect existing Dev MCP processes".into());
        }
        for pid in String::from_utf8_lossy(&output.stdout).split_whitespace() {
            conflicts.push(format!(
                "restart the existing Dev MCP connection before migration (pid {pid})"
            ));
        }
    }
    Ok(conflicts)
}

/// Called before every managed artifact publication. Persist the expected bytes first so a crash
/// cannot make rollback claim an unrecorded file. Unknown concurrent content is never overwritten.
pub fn before_write(path: &Path, bytes: Option<&[u8]>) -> anyhow::Result<()> {
    let Some(id) = std::env::var_os(JOURNAL_ENV) else {
        return Ok(());
    };
    require_system().map_err(anyhow::Error::msg)?;
    let id = id
        .to_str()
        .ok_or_else(|| anyhow::anyhow!("invalid migration context"))?;
    let mut journal = read(id).map_err(anyhow::Error::msg)?;
    let artifact = journal
        .artifacts
        .iter_mut()
        .find(|a| a.path == path)
        .ok_or_else(|| anyhow::anyhow!("unplanned migration artifact: {}", path.display()))?;
    let current = current_hash(path).map_err(anyhow::Error::msg)?;
    authorize(artifact, current, bytes.map(hash)).map_err(anyhow::Error::msg)?;
    save(&journal).map_err(anyhow::Error::msg)
}
fn authorize(
    artifact: &mut Artifact,
    current: Option<String>,
    next: Option<String>,
) -> Result<(), String> {
    if !artifact.known.contains(&current) {
        return Err(format!(
            "integration changed outside migration; preserving {}",
            artifact.path.display()
        ));
    }
    if !artifact.known.contains(&next) {
        artifact.known.push(next);
    }
    Ok(())
}
pub fn remove_file(path: &Path) -> std::io::Result<()> {
    before_write(path, None).map_err(std::io::Error::other)?;
    std::fs::remove_file(path)
}
fn unchanged(journal: &Journal) -> Result<(), String> {
    for artifact in &journal.artifacts {
        if !artifact.known.contains(&current_hash(&artifact.path)?) {
            return Err(format!(
                "integration changed outside migration; preserving {}",
                artifact.path.display()
            ));
        }
    }
    Ok(())
}

pub fn apply(id: &str) -> Result<Journal, String> {
    require_system()?;
    let _lock = crate::file_lock::FileLock::exclusive(&state::directory().join("migration.lock"))
        .map_err(|e| e.to_string())?;
    if latest()?.is_none_or(|j| j.id != id) {
        return Err("only the latest reviewed migration may be applied".into());
    }
    let mut journal = read(id)?;
    if journal.phase == "complete" {
        return Ok(journal);
    }
    if !matches!(journal.phase.as_str(), "planned" | "partial") {
        return Err("this migration cannot be applied".into());
    }
    if !journal.conflicts.is_empty() {
        return Err("resolve the plan's conflicts and create a fresh migration plan".into());
    }
    if !state::same_path(
        &journal.system_binary,
        &super::relay::system_target()?.binary,
    ) {
        return Err("system entry changed; create a fresh migration plan".into());
    }
    let active = quiescence_conflicts()?;
    if !active.is_empty() {
        return Err(active.join("\n"));
    }
    unchanged(&journal)?;
    journal.phase = "partial".into();
    journal.error = None;
    save(&journal)?;
    // Internal clients are pinned already; the transition only governs newly started callers.
    state::update(|s| {
        s.mode = state::RoutingMode::Explicit;
        Ok(())
    })?;
    for index in 0..journal.actions.len() {
        if journal.actions[index].status == "complete" {
            continue;
        }
        let action = journal.actions[index].clone();
        let target = super::relay::system_target()?;
        let mut command = std::process::Command::new(&target.binary);
        routing::configure(&mut command, &target)?;
        super::clear_client_context(&mut command);
        let output = command
            .env(JOURNAL_ENV, id)
            .env(routing::SYSTEM_BINDINGS_ENV, "1")
            .args(["dev", "migrate-worker", &action.agent, &action.mode])
            .output()
            .map_err(|e| e.to_string())?;
        // Workers append write intents. Never replace those with the parent's stale journal.
        journal = read(id)?;
        if output.status.success() {
            journal.actions[index].status = "complete".into();
            journal.actions[index].error = None;
        } else {
            let diagnostic = directory(id)?.join(format!("{}.stderr.txt", action.agent));
            publish(&diagnostic, &output.stderr, Some(0o600))?;
            let error = format!(
                "{} integration update failed (exit {:?}); private diagnostics: {}",
                action.agent,
                output.status.code(),
                diagnostic.display()
            );
            journal.actions[index].status = "failed".into();
            journal.actions[index].error = Some(error.clone());
            journal.error = Some(error);
            save(&journal)?;
            return Ok(journal);
        }
        save(&journal)?;
    }
    // Probe a new formal process from every registered worktree before claiming completion.
    for root in state::read()?.worktrees {
        let target = super::relay::system_target()?;
        let mut command = std::process::Command::new(&target.binary);
        routing::configure(&mut command, &target)?;
        super::clear_client_context(&mut command);
        let output = command
            .current_dir(&root)
            .args(["dev", "target"])
            .output()
            .map_err(|e| e.to_string())?;
        let resolved: Option<routing::Target> = serde_json::from_slice(&output.stdout)
            .map_err(|_| "system routing verification returned an invalid target")?;
        if !output.status.success()
            || !resolved.is_some_and(|t| {
                t.kind == routing::TargetKind::System
                    && state::same_path(&t.home, &state::system_home())
            })
        {
            return Err("system routing verification failed; migration remains partial".into());
        }
    }
    journal.phase = "complete".into();
    save(&journal)?;
    Ok(journal)
}

pub fn rollback(id: &str) -> Result<Journal, String> {
    require_system()?;
    let _lock = crate::file_lock::FileLock::exclusive(&state::directory().join("migration.lock"))
        .map_err(|e| e.to_string())?;
    let _integrations = crate::integrations::mutation_lock::IntegrationMutationLock::acquire()
        .map_err(|e| e.to_string())?;
    if latest()?.is_none_or(|j| j.id != id) {
        return Err("restore the latest migration first".into());
    }
    let mut journal = read(id)?;
    if journal.phase == "rolledBack" {
        return Ok(journal);
    }
    let active = quiescence_conflicts()?;
    if !active.is_empty() {
        return Err(active.join("\n"));
    }
    unchanged(&journal)?;
    journal.phase = "rollingBack".into();
    save(&journal)?;
    for (index, artifact) in journal.artifacts.iter().enumerate() {
        let bytes = read_file(&backup_path(id, index)?)?;
        if bytes.as_deref().map(hash) != artifact.before {
            return Err("migration backup is missing or changed".into());
        }
        // Recheck just before each atomic restoration; a conflict leaves other files recoverable.
        if !artifact.known.contains(&current_hash(&artifact.path)?) {
            return Err(format!(
                "preserving an external change at {}",
                artifact.path.display()
            ));
        }
        match bytes {
            Some(bytes) => publish(&artifact.path, &bytes, artifact.permissions)?,
            None => match std::fs::remove_file(&artifact.path) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(e.to_string()),
            },
        }
    }
    state::update(|s| {
        s.mode = journal.previous_mode;
        Ok(())
    })?;
    journal.phase = "rolledBack".into();
    journal.error = None;
    save(&journal)?;
    Ok(journal)
}

pub fn worker(agent: &str, mode: &str) -> Result<(), String> {
    require_system()?;
    let id = std::env::var(JOURNAL_ENV).map_err(|_| "missing migration journal")?;
    let journal = read(&id)?;
    if journal.phase != "partial"
        || !journal
            .actions
            .iter()
            .any(|a| a.agent == agent && a.mode == mode)
    {
        return Err("unplanned migration action".into());
    }
    agent_mode::set(
        AgentTarget::parse(agent).ok_or("unknown Agent")?,
        agent_mode::Mode::parse(mode).ok_or("unknown mode")?,
    )
    .map_err(|e| e.to_string())
}

pub fn latest() -> Result<Option<Journal>, String> {
    let root = state::directory().join("migrations");
    let entries = match std::fs::read_dir(root) {
        Ok(v) => v,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.to_string()),
    };
    let mut journals = entries
        .filter_map(Result::ok)
        .filter_map(|e| {
            let id = e.file_name().to_string_lossy().to_string();
            let modified = e
                .path()
                .join("journal.json")
                .metadata()
                .ok()?
                .modified()
                .ok()?;
            Some((modified, id))
        })
        .collect::<Vec<_>>();
    journals.sort_by_key(|v| v.0);
    journals.last().map(|(_, id)| read(id)).transpose()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn write_intents_accept_partial_publication_but_reject_user_edits() {
        let mut a = Artifact {
            path: "config".into(),
            before: Some(hash(b"old")),
            known: vec![Some(hash(b"old"))],
            permissions: None,
        };
        authorize(&mut a, Some(hash(b"old")), Some(hash(b"step one"))).unwrap();
        authorize(&mut a, Some(hash(b"step one")), Some(hash(b"new"))).unwrap();
        assert!(authorize(&mut a, Some(hash(b"custom")), None).is_err());
        assert!(a.known.contains(&a.before));
    }
    #[test]
    fn linked_artifacts_are_never_claimed() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().canonicalize().unwrap().join("config");
        std::fs::write(&p, b"custom").unwrap();
        assert_eq!(read_file(&p).unwrap().unwrap(), b"custom");
        #[cfg(unix)]
        {
            let link = dir.path().canonicalize().unwrap().join("linked");
            std::os::unix::fs::symlink(&p, &link).unwrap();
            assert!(read_file(&link).is_err());
        }
    }
    #[test]
    fn rollback_backups_are_private_and_preserve_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().canonicalize().unwrap().join("before");
        publish(&p, b"token = 'private'\n", Some(0o600)).unwrap();
        assert_eq!(read_file(&p).unwrap().unwrap(), b"token = 'private'\n");
        #[cfg(unix)]
        assert_eq!(permissions(&p), Some(0o600));
    }
    #[test]
    fn migration_preserves_custom_json_server_controls_instead_of_reinstalling_them() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().canonicalize().unwrap().join("mcp.json");
        let bytes=br#"{"mcpServers":{"askhuman":{"command":"/tools/AskHuman","args":["mcp"],"disabled":true}}}"#;
        std::fs::write(&path, bytes).unwrap();
        assert!(check_mcp(&path, false)
            .unwrap_err()
            .contains("custom MCP fields"));
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
    }
}
