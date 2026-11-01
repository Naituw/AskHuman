//! Project-only MCP preparation; never calls the user-level integration installers.

use super::{state, workspace};
use crate::agents::AgentKind;
use jsonc_parser::cst::{CstInputValue, CstRootNode};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io::Write;
use std::path::{Path, PathBuf};

const SERVER: &str = "askhuman-dev";
const MAX_CONFIG: u64 = 16 * 1024 * 1024;

pub fn ensure_legacy_idle(root: &Path) -> Result<(), String> {
    if !root
        .join(crate::dev_instance::DEV_DIR)
        .join(crate::dev_instance::ENABLED_MARKER)
        .is_file()
    {
        return Ok(());
    }
    if let Some(bytes) = read_file(
        root,
        &crate::dev_instance::instance_home(root).join("agents.json"),
    )? {
        let value: Value =
            serde_json::from_slice(&bytes).map_err(|_| "cannot inspect existing Dev sessions")?;
        if let Some(active) = value["active"].as_array() {
            for record in active {
                let pid = record["pid"]
                    .as_u64()
                    .and_then(|pid| u32::try_from(pid).ok());
                if pid.is_none_or(crate::agents::detect::pid_alive) {
                    return Err(format!("close the existing Dev Agent session before updating this worktree (pid {})", pid.unwrap_or(0)));
                }
            }
        }
    }
    #[cfg(unix)]
    {
        let output = std::process::Command::new("ps")
            .args(["-axo", "pid=,comm="])
            .output()
            .map_err(|error| error.to_string())?;
        if !output.status.success() {
            return Err("cannot inspect existing Dev MCP connections".into());
        }
        let binary = crate::dev_instance::instance_bin(root)
            .to_string_lossy()
            .to_string();
        for line in String::from_utf8_lossy(&output.stdout).lines() {
            let Some((pid, command)) = line.trim().split_once(char::is_whitespace) else {
                continue;
            };
            if !command.to_ascii_lowercase().contains("askhuman") {
                continue;
            }
            let args = std::process::Command::new("ps")
                .args(["-p", pid, "-o", "args="])
                .output()
                .map_err(|error| error.to_string())?;
            let args = String::from_utf8_lossy(&args.stdout);
            if args.contains(&binary) && args.split_whitespace().any(|argument| argument == "mcp") {
                return Err(format!("restart the existing Dev MCP connection before updating this worktree (pid {pid})"));
            }
        }
    }
    #[cfg(windows)]
    {
        let binary = crate::dev_instance::instance_bin(root)
            .to_string_lossy()
            .replace('\'', "''");
        let script = format!("Get-CimInstance Win32_Process -Filter \"Name = 'AskHuman.exe'\" | Where-Object {{ $_.ExecutablePath -eq '{binary}' -and $_.CommandLine -match '(?:^|\\s)mcp(?:\\s|$)' }} | Select-Object -ExpandProperty ProcessId");
        let output = std::process::Command::new("powershell.exe")
            .args(["-NoProfile", "-NonInteractive", "-Command", &script])
            .output()
            .map_err(|error| error.to_string())?;
        if !output.status.success() {
            return Err("cannot inspect existing Dev MCP connections".into());
        }
        if let Some(pid) = String::from_utf8_lossy(&output.stdout)
            .split_whitespace()
            .next()
        {
            return Err(format!(
                "restart the existing Dev MCP connection before updating this worktree (pid {pid})"
            ));
        }
    }
    Ok(())
}

pub fn retire_legacy_marker(root: &Path) -> Result<(), String> {
    let marker = root
        .join(crate::dev_instance::DEV_DIR)
        .join(crate::dev_instance::ENABLED_MARKER);
    if let Some(before) = read_file(root, &marker)? {
        let backup = root
            .join(crate::dev_instance::DEV_DIR)
            .join("legacy-enabled.before");
        if !backup.exists() {
            publish(root, &backup, None, &before)?;
        }
        std::fs::remove_file(marker).map_err(|error| error.to_string())?;
    }
    Ok(())
}

#[derive(Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct Ledger {
    entries: Vec<OwnedEntry>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct OwnedEntry {
    path: PathBuf,
    known: Vec<Value>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    pub worktree: PathBuf,
    pub changed: bool,
    pub configured_client: Option<AgentKind>,
    pub configuration_path: Option<PathBuf>,
    pub requires_reload: bool,
    pub connection_observed: bool,
}

fn config_path(kind: AgentKind) -> Option<(&'static str, bool)> {
    match kind {
        AgentKind::Claude => Some((".mcp.json", false)),
        AgentKind::Codex => Some((".codex/config.toml", true)),
        AgentKind::Cursor => Some((".cursor/mcp.json", false)),
        AgentKind::Grok => Some((".grok/config.toml", true)),
        AgentKind::Antigravity => Some((".agents/mcp_config.json", false)),
        AgentKind::Pi => None,
    }
}

fn read_file(root: &Path, path: &Path) -> Result<Option<Vec<u8>>, String> {
    if !path.starts_with(root) {
        return Err("project configuration escapes its worktree".into());
    }
    for ancestor in path.ancestors().take_while(|ancestor| *ancestor != root) {
        if std::fs::symlink_metadata(ancestor)
            .is_ok_and(|metadata| metadata.file_type().is_symlink())
        {
            return Err(format!(
                "linked project configuration requires review: {}",
                ancestor.display()
            ));
        }
    }
    match std::fs::metadata(path) {
        Ok(metadata) if !metadata.is_file() || metadata.len() > MAX_CONFIG => Err(format!(
            "project configuration is not a bounded file: {}",
            path.display()
        )),
        Ok(_) => std::fs::read(path)
            .map(Some)
            .map_err(|error| error.to_string()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.to_string()),
    }
}

fn publish(root: &Path, path: &Path, before: Option<&[u8]>, after: &[u8]) -> Result<(), String> {
    if read_file(root, path)?.as_deref() != before {
        return Err(format!(
            "project configuration changed concurrently: {}",
            path.display()
        ));
    }
    let parent = path.parent().ok_or("project configuration has no parent")?;
    std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    let mut temporary =
        tempfile::NamedTempFile::new_in(parent).map_err(|error| error.to_string())?;
    temporary
        .write_all(after)
        .map_err(|error| error.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let permissions = std::fs::metadata(path)
            .map(|metadata| metadata.permissions())
            .unwrap_or_else(|_| std::fs::Permissions::from_mode(0o600));
        temporary
            .as_file()
            .set_permissions(permissions)
            .map_err(|error| error.to_string())?;
    }
    temporary
        .as_file()
        .sync_all()
        .map_err(|error| error.to_string())?;
    temporary.persist(path).map_err(|error| error.to_string())?;
    Ok(())
}

fn exclude_generated_file(root: &Path, relative: &str) -> Result<(), String> {
    // The private helper tests need no Git process; the public CLI requires a real worktree.
    if !root.join(".git").exists() {
        return Ok(());
    }
    let mut git = std::process::Command::new("git");
    git.arg("-C")
        .arg(root)
        .args(["rev-parse", "--git-common-dir"]);
    for key in [
        "GIT_DIR",
        "GIT_WORK_TREE",
        "GIT_COMMON_DIR",
        "GIT_INDEX_FILE",
    ] {
        git.env_remove(key);
    }
    let output = git.output().map_err(|error| error.to_string())?;
    if !output.status.success() {
        return Err("cannot locate repository-local excludes".into());
    }
    let directory = PathBuf::from(
        String::from_utf8(output.stdout)
            .map_err(|_| "invalid Git directory")?
            .trim(),
    );
    let directory = if directory.is_absolute() {
        directory
    } else {
        root.join(directory)
    };
    let directory = directory
        .canonicalize()
        .map_err(|error| error.to_string())?;
    let _lock = crate::file_lock::FileLock::exclusive(&directory.join("askhuman-excludes.lock"))
        .map_err(|error| error.to_string())?;
    let parent = directory.join("info");
    std::fs::create_dir_all(&parent).map_err(|error| error.to_string())?;
    if !parent
        .canonicalize()
        .map_err(|error| error.to_string())?
        .starts_with(&directory)
    {
        return Err("repository-local excludes directory escapes its Git directory".into());
    }
    let file = parent.join("exclude");
    if std::fs::symlink_metadata(&file).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
        return Err("linked repository-local excludes require manual review".into());
    }
    if std::fs::metadata(&file).is_ok_and(|metadata| metadata.len() > 1024 * 1024) {
        return Err("repository-local excludes exceed 1 MiB".into());
    }
    let before = match std::fs::read_to_string(&file) {
        Ok(value) => value,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(error.to_string()),
    };
    let pattern = format!("/{relative}");
    if !before.lines().any(|line| line == pattern) {
        let mut append = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(file)
            .map_err(|error| error.to_string())?;
        let text = format!("\n# AskHuman generated project MCP\n{pattern}\n");
        append
            .write_all(text.as_bytes())
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn expected_entry(node: &str, args: &[String], kind: AgentKind, toml: bool) -> Value {
    let mut value = serde_json::json!({"command":node,"args":args});
    if toml {
        value["startup_timeout_sec"] = 30.into();
        value["tool_timeout_sec"] = 86400.into();
        if kind == AgentKind::Grok {
            value["tool_timeouts"] = serde_json::json!({"dev_ask":86400});
        }
    } else if kind == AgentKind::Claude {
        value["timeout"] = 86_400_000.into();
    } else if kind == AgentKind::Antigravity {
        value["timeoutSeconds"] = 86400.into();
    }
    value
}

fn cst_value(value: &Value) -> Result<CstInputValue, String> {
    match value {
        Value::String(value) => Ok(CstInputValue::String(value.clone())),
        Value::Array(values) => values
            .iter()
            .map(cst_value)
            .collect::<Result<Vec<_>, _>>()
            .map(CstInputValue::Array),
        Value::Object(values) => values
            .iter()
            .map(|(key, value)| Ok((key.clone(), cst_value(value)?)))
            .collect::<Result<Vec<_>, String>>()
            .map(CstInputValue::Object),
        Value::Number(value) => {
            let integer = value.as_i64().ok_or("invalid project timeout")?;
            Ok(jsonc_parser::json!(integer))
        }
        _ => Err("unsupported project MCP value".into()),
    }
}

fn edit_json(text: &str, desired: Option<&Value>) -> Result<(Option<Value>, String), String> {
    let source = if text.trim().is_empty() { "{}" } else { text };
    let value: Value = jsonc_parser::parse_to_serde_value(source, &Default::default())
        .map_err(|_| "cannot parse project MCP configuration; original preserved")?;
    if !value.is_object()
        || value
            .get("mcpServers")
            .is_some_and(|value| !value.is_object())
    {
        return Err("project MCP configuration and mcpServers must be objects".into());
    }
    let before = value
        .get("mcpServers")
        .and_then(|servers| servers.get(SERVER))
        .cloned();
    let root = CstRootNode::parse(source, &Default::default())
        .map_err(|_| "cannot parse project MCP configuration")?;
    let object = root
        .object_value_or_create()
        .ok_or("project MCP root is not an object")?;
    if let Some(desired) = desired {
        let servers = object
            .object_value_or_create("mcpServers")
            .ok_or("project MCP servers are not an object")?;
        if let Some(property) = servers.get(SERVER) {
            property.set_value(cst_value(desired)?);
        } else {
            servers.ensure_multiline();
            servers.append(SERVER, cst_value(desired)?);
        }
    } else if let Some(servers) = object.object_value("mcpServers") {
        if let Some(property) = servers.get(SERVER) {
            property.remove();
        }
    }
    Ok((before, root.to_string()))
}

fn toml_json(item: &toml_edit::Item) -> Result<Value, String> {
    if let Some(value) = item.as_str() {
        return Ok(Value::String(value.into()));
    }
    if let Some(value) = item.as_integer() {
        return Ok(value.into());
    }
    if let Some(array) = item.as_array() {
        return array
            .iter()
            .map(|value| toml_json(&toml_edit::Item::Value(value.clone())))
            .collect::<Result<Vec<_>, _>>()
            .map(Value::Array);
    }
    if let Some(table) = item.as_table_like() {
        return table
            .iter()
            .map(|(key, value)| Ok((key.into(), toml_json(value)?)))
            .collect::<Result<serde_json::Map<_, _>, String>>()
            .map(Value::Object);
    }
    Err("custom project MCP value requires manual review".into())
}

fn edit_toml(text: &str, desired: Option<&Value>) -> Result<(Option<Value>, String), String> {
    let mut doc = text
        .parse::<toml_edit::DocumentMut>()
        .map_err(|_| "cannot parse project TOML; original preserved")?;
    let before = doc
        .get("mcp_servers")
        .and_then(|servers| servers.get(SERVER))
        .map(toml_json)
        .transpose()?;
    if desired.is_some() && doc.get("mcp_servers").is_none() {
        doc.insert(
            "mcp_servers",
            toml_edit::Item::Table(toml_edit::Table::new()),
        );
    }
    if let Some(servers) = doc.get_mut("mcp_servers") {
        let servers = servers
            .as_table_like_mut()
            .ok_or("project mcp_servers is not a table")?;
        if let Some(desired) = desired {
            let mut table = toml_edit::Table::new();
            for (key, value) in desired.as_object().ok_or("invalid project MCP entry")? {
                let item = match value {
                    Value::String(value) => toml_edit::value(value),
                    Value::Number(value) => {
                        toml_edit::value(value.as_i64().ok_or("invalid timeout")?)
                    }
                    Value::Array(values) => {
                        let mut array = toml_edit::Array::new();
                        for value in values {
                            array.push(value.as_str().ok_or("invalid MCP argument")?);
                        }
                        toml_edit::value(array)
                    }
                    Value::Object(values) => {
                        let mut inline = toml_edit::InlineTable::new();
                        for (key, value) in values {
                            inline
                                .insert(key, value.as_i64().ok_or("invalid tool timeout")?.into());
                        }
                        toml_edit::value(inline)
                    }
                    _ => return Err("unsupported project MCP entry".into()),
                };
                table.insert(key, item);
            }
            servers.insert(SERVER, toml_edit::Item::Table(table));
        } else {
            servers.remove(SERVER);
        }
    }
    Ok((before, doc.to_string()))
}

fn node_executable() -> Result<PathBuf, String> {
    use std::process::Stdio;
    let mut child = std::process::Command::new("node")
        .args(["-p", "JSON.stringify({path:process.execPath,major:Number(process.versions.node.split('.')[0])})"])
        .stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn()
        .map_err(|error| format!("Node.js 20+ is required for project MCP: {error}"))?;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if std::time::Instant::now() < deadline => {
                std::thread::sleep(std::time::Duration::from_millis(10))
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("Node.js capability probe timed out".into());
            }
        }
    }
    let output = child
        .wait_with_output()
        .map_err(|error| error.to_string())?;
    let value: Value = serde_json::from_slice(&output.stdout)
        .map_err(|_| "Node.js returned an invalid capability response")?;
    let path = PathBuf::from(
        value["path"]
            .as_str()
            .ok_or("Node.js did not return its executable path")?,
    );
    if !output.status.success() || value["major"].as_u64().unwrap_or(0) < 20 || !path.is_absolute()
    {
        return Err("Node.js 20+ is required for project MCP".into());
    }
    Ok(path)
}

pub fn bridge_fingerprint(root: &Path) -> Result<String, String> {
    use sha2::{Digest, Sha256};
    let mut hash = Sha256::new();
    for name in [
        "scripts/dev.mjs",
        "scripts/development/runtime.mjs",
        "scripts/development/mcp.mjs",
        "scripts/development/caller.mjs",
    ] {
        hash.update(name.as_bytes());
        hash.update(std::fs::read(root.join(name)).map_err(|e| e.to_string())?);
    }
    Ok(format!("{:x}", hash.finalize()))
}

fn observed_connection(root: &Path) -> bool {
    let Ok(fingerprint) = bridge_fingerprint(root) else {
        return false;
    };
    let Ok(entries) = std::fs::read_dir(root.join(".askhuman-dev/mcp-connections")) else {
        return false;
    };
    entries.flatten().take(256).any(|entry| {
        let Ok(Some(bytes)) = read_file(root, &entry.path()) else {
            return false;
        };
        let Ok(value) = serde_json::from_slice::<Value>(&bytes) else {
            return false;
        };
        value["fingerprint"] == fingerprint
            && value["worktree"].as_str().is_some_and(|worktree| {
                let worktree = Path::new(worktree);
                worktree.is_absolute() && state::same_path(worktree, root)
            })
            && value["pid"]
                .as_u64()
                .and_then(|pid| u32::try_from(pid).ok())
                .is_some_and(crate::agents::detect::pid_alive)
    })
}

pub fn prepare(
    root: &Path,
    system_binary: &Path,
    kind: Option<AgentKind>,
) -> Result<Report, String> {
    let node = kind
        .and_then(config_path)
        .map(|_| node_executable())
        .transpose()?;
    prepare_with_node(root, system_binary, kind, node.as_deref())
}

fn prepare_with_node(
    root: &Path,
    system_binary: &Path,
    kind: Option<AgentKind>,
    node: Option<&Path>,
) -> Result<Report, String> {
    let root = root.canonicalize().map_err(|error| error.to_string())?;
    let _lock = crate::file_lock::FileLock::exclusive(
        &root
            .join(crate::dev_instance::DEV_DIR)
            .join("project-config.lock"),
    )
    .map_err(|error| error.to_string())?;
    let manifest_path = workspace::path(&root);
    let mut manifest = if manifest_path.exists() {
        workspace::read(&root)?
    } else {
        workspace::Workspace {
            version: workspace::VERSION,
            worktree: root.clone(),
            system_binary: system_binary.to_path_buf(),
            system_home: state::system_home(),
        }
    };
    let formal_changed = manifest.system_binary != system_binary;
    manifest.system_binary = system_binary.to_path_buf();
    workspace::validate(&root, &manifest)?;
    let mut changed = false;
    let configuration_path = if let Some((kind, (relative, toml))) =
        kind.and_then(|kind| config_path(kind).map(|config| (kind, config)))
    {
        let target = root.join(relative);
        let before = read_file(&root, &target)?;
        let text = std::str::from_utf8(before.as_deref().unwrap_or_default())
            .map_err(|_| "project MCP config must be UTF-8")?;
        let node = node
            .ok_or("Node.js executable is missing")?
            .to_string_lossy();
        let node = node.as_ref();
        let bridge = root.join("scripts/dev.mjs");
        if !bridge.is_file() {
            return Err("this worktree has no project development bridge".into());
        }
        let args = vec![
            bridge.to_string_lossy().to_string(),
            "mcp".into(),
            "--worktree".into(),
            root.to_string_lossy().to_string(),
        ];
        let desired = expected_entry(node, &args, kind, toml);
        let (current, rendered) = if toml {
            edit_toml(text, Some(&desired))?
        } else {
            edit_json(text, Some(&desired))?
        };
        let ledger_path = root
            .join(crate::dev_instance::DEV_DIR)
            .join("project-integrations.json");
        let mut ledger: Ledger = read_file(&root, &ledger_path)?
            .map(|bytes| {
                serde_json::from_slice(&bytes)
                    .map_err(|_| "invalid project ownership ledger".to_string())
            })
            .transpose()?
            .unwrap_or_default();
        let owned = ledger
            .entries
            .iter()
            .position(|entry| entry.path == Path::new(relative));
        if current.as_ref() != Some(&desired) {
            if let Some(current) = &current {
                if !owned.is_some_and(|index| ledger.entries[index].known.contains(current)) {
                    return Err(format!(
                        "custom {SERVER} entry preserved; review {}",
                        target.display()
                    ));
                }
            }
            let index = owned.unwrap_or_else(|| {
                ledger.entries.push(OwnedEntry {
                    path: PathBuf::from(relative),
                    known: vec![],
                });
                ledger.entries.len() - 1
            });
            if !ledger.entries[index].known.contains(&desired) {
                ledger.entries[index].known.push(desired.clone());
            }
            state::write_json(&ledger_path, &ledger)?;
            if before.is_none() {
                exclude_generated_file(&root, relative)?;
            }
            publish(&root, &target, before.as_deref(), rendered.as_bytes())?;
            ledger.entries[index].known = vec![desired];
            state::write_json(&ledger_path, &ledger)?;
            changed = true;
        }
        Some(target)
    } else {
        None
    };
    if !manifest_path.exists() || formal_changed {
        state::write_json(&manifest_path, &manifest)?;
        workspace::read(&root)?;
        changed = true;
    }
    let connection_observed = !changed && observed_connection(&root);
    Ok(Report {
        worktree: root,
        changed,
        configured_client: kind,
        requires_reload: configuration_path.is_some() && !connection_observed,
        connection_observed,
        configuration_path,
    })
}

pub fn remove(root: &Path) -> Result<(), String> {
    let _lock = crate::file_lock::FileLock::exclusive(
        &root
            .join(crate::dev_instance::DEV_DIR)
            .join("project-config.lock"),
    )
    .map_err(|error| error.to_string())?;
    let ledger_path = root
        .join(crate::dev_instance::DEV_DIR)
        .join("project-integrations.json");
    let Some(bytes) = read_file(root, &ledger_path)? else {
        return Ok(());
    };
    let ledger: Ledger =
        serde_json::from_slice(&bytes).map_err(|_| "invalid project ownership ledger")?;
    for entry in ledger.entries {
        let format = [
            AgentKind::Claude,
            AgentKind::Codex,
            AgentKind::Cursor,
            AgentKind::Grok,
            AgentKind::Antigravity,
        ]
        .into_iter()
        .filter_map(config_path)
        .find(|(path, _)| Path::new(path) == entry.path)
        .ok_or("unknown project ownership path")?;
        let target = root.join(&entry.path);
        let Some(before) = read_file(root, &target)? else {
            continue;
        };
        let text = std::str::from_utf8(&before).map_err(|_| "project MCP config must be UTF-8")?;
        let (current, rendered) = if format.1 {
            edit_toml(text, None)?
        } else {
            edit_json(text, None)?
        };
        if current.is_none() {
            continue;
        }
        if current.is_some_and(|value| !entry.known.contains(&value)) {
            return Err(format!(
                "externally edited project MCP entry preserved: {}",
                target.display()
            ));
        }
        publish(root, &target, Some(&before), rendered.as_bytes())?;
    }
    std::fs::remove_file(ledger_path).map_err(|error| error.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> tempfile::TempDir {
        let temp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(temp.path().join("scripts")).unwrap();
        std::fs::write(temp.path().join("scripts/dev.mjs"), "// fixture\n").unwrap();
        temp
    }

    #[test]
    fn project_json_preserves_the_daily_server_and_comments() {
        let source = "{\n// preserve this comment\n\"mcpServers\":{\"askhuman\":{\"command\":\"daily\"},\"other\":{\"url\":\"https://example.test\"}}}\n";
        let desired = serde_json::json!({"command":"node","args":["bridge","mcp"]});
        let (before, after) = edit_json(source, Some(&desired)).unwrap();
        assert!(before.is_none());
        assert!(after.contains("// preserve this comment"));
        let value: Value = jsonc_parser::parse_to_serde_value(&after, &Default::default()).unwrap();
        assert_eq!(value["mcpServers"]["askhuman"]["command"], "daily");
        assert_eq!(value["mcpServers"]["other"]["url"], "https://example.test");
        let (_, removed) = edit_json(&after, None).unwrap();
        let value: Value =
            jsonc_parser::parse_to_serde_value(&removed, &Default::default()).unwrap();
        assert!(value["mcpServers"].get(SERVER).is_none());
        assert_eq!(value["mcpServers"]["askhuman"]["command"], "daily");
    }

    #[test]
    fn project_preparation_is_idempotent_and_removal_preserves_external_servers() {
        let temp = fixture();
        let root = temp.path().canonicalize().unwrap();
        let system = root.parent().unwrap().join("formal/AskHuman");
        let node = root.parent().unwrap().join("node");
        let first = prepare_with_node(&root, &system, Some(AgentKind::Codex), Some(&node)).unwrap();
        assert!(first.changed && first.requires_reload);
        let config = root.join(".codex/config.toml");
        let before = std::fs::read(&config).unwrap();
        let manifest_before = std::fs::read(workspace::path(&root)).unwrap();
        let second =
            prepare_with_node(&root, &system, Some(AgentKind::Codex), Some(&node)).unwrap();
        assert!(!second.changed);
        assert_eq!(std::fs::read(&config).unwrap(), before);
        assert_eq!(
            std::fs::read(workspace::path(&root)).unwrap(),
            manifest_before
        );
        let mut text = std::fs::read_to_string(&config).unwrap();
        text.push_str("\n# another integration\n[mcp_servers.other]\ncommand = 'other'\n");
        std::fs::write(&config, text).unwrap();
        remove(&root).unwrap();
        let doc = std::fs::read_to_string(&config)
            .unwrap()
            .parse::<toml_edit::DocumentMut>()
            .unwrap();
        assert!(doc["mcp_servers"].get(SERVER).is_none());
        assert_eq!(
            doc["mcp_servers"]["other"]["command"].as_str(),
            Some("other")
        );
    }

    #[test]
    fn custom_project_entries_are_never_claimed_or_replaced() {
        let temp = fixture();
        let root = temp.path().canonicalize().unwrap();
        let config = root.join(".mcp.json");
        let original = br#"{"mcpServers":{"askhuman-dev":{"command":"custom","args":[]}}}"#;
        std::fs::write(&config, original).unwrap();
        assert!(prepare_with_node(
            &root,
            &root.parent().unwrap().join("formal/AskHuman"),
            Some(AgentKind::Claude),
            Some(&root.join("node"))
        )
        .is_err());
        assert_eq!(std::fs::read(&config).unwrap(), original);
        assert!(!workspace::path(&root).exists());
    }

    #[test]
    fn an_external_edit_to_the_owned_entry_blocks_removal() {
        let temp = fixture();
        let root = temp.path().canonicalize().unwrap();
        prepare_with_node(
            &root,
            &root.parent().unwrap().join("formal/AskHuman"),
            Some(AgentKind::Cursor),
            Some(&root.join("node")),
        )
        .unwrap();
        let config = root.join(".cursor/mcp.json");
        let original = std::fs::read_to_string(&config).unwrap();
        let changed = original.replace("\"command\"", "\"customCommand\"");
        std::fs::write(&config, &changed).unwrap();
        assert!(remove(&root).is_err());
        assert_eq!(std::fs::read_to_string(&config).unwrap(), changed);
    }
}
