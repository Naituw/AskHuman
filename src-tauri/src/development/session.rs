//! Persistent, worktree-owned client profiles for independent native Agent tests.

use super::{build, routing, state};
use crate::agents::AgentKind;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub const PROFILE_ENV: &str = "ASKHUMAN_DEV_TEST_PROFILE";
pub const SOURCE_CHANNEL: &str = "dev-test";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Profile {
    pub version: u32,
    pub id: String,
    pub kind: AgentKind,
    pub worktree: PathBuf,
    pub directory: PathBuf,
    pub neutral_workspace: PathBuf,
    pub executable: String,
    pub build: build::BuildInfo,
    pub prepared: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Support {
    pub kind: AgentKind,
    pub supported: bool,
    pub terminal_available: bool,
    pub launch_reason: Option<String>,
    pub executable: Option<String>,
    pub reason: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Binding {
    profile: String,
    kind: AgentKind,
    session_id: String,
    transcript: Option<PathBuf>,
}

fn profile_path(root: &Path, id: &str) -> Result<PathBuf, String> {
    if uuid::Uuid::parse_str(id).is_err() {
        return Err("invalid development profile ID".into());
    }
    Ok(crate::dev_instance::instance_home(root)
        .join("state/test-profiles")
        .join(format!("{id}.json")))
}

pub fn read(root: &Path, id: &str) -> Result<Profile, String> {
    let bytes = std::fs::read(profile_path(root, id)?).map_err(|e| e.to_string())?;
    if bytes.len() > 64 * 1024 {
        return Err("development profile is too large".into());
    }
    let profile: Profile = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    let expected = root
        .join(crate::dev_instance::DEV_DIR)
        .join("clients")
        .join(profile.kind.as_str());
    let private_root = root
        .join(crate::dev_instance::DEV_DIR)
        .canonicalize()
        .map_err(|e| e.to_string())?;
    let actual_directory = profile
        .directory
        .canonicalize()
        .map_err(|e| e.to_string())?;
    if profile.version != 1
        || profile.id != id
        || !state::same_path(root, &profile.worktree)
        || !state::same_path(&expected, &profile.directory)
        || !actual_directory.starts_with(private_root)
    {
        return Err("development profile identity mismatch".into());
    }
    let owner: serde_json::Value = serde_json::from_slice(
        &std::fs::read(profile.neutral_workspace.join("askhuman-owner.json"))
            .map_err(|e| format!("isolated workspace ownership is missing: {e}"))?,
    )
    .map_err(|e| e.to_string())?;
    if owner["profile"].as_str() != Some(id)
        || owner["worktree"]
            .as_str()
            .is_none_or(|p| !state::same_path(Path::new(p), root))
        || !profile.neutral_workspace.join(".git").is_dir()
    {
        return Err("isolated test workspace ownership mismatch".into());
    }
    Ok(profile)
}

fn write(profile: &Profile) -> Result<(), String> {
    state::write_json(&profile_path(&profile.worktree, &profile.id)?, profile)
}

pub fn active() -> Result<Option<Profile>, String> {
    let Some(id) = std::env::var(PROFILE_ENV).ok().filter(|v| !v.is_empty()) else {
        return Ok(None);
    };
    let target =
        routing::inherited()?.ok_or("development test profile requires a resolved target")?;
    if target.kind != routing::TargetKind::Dev {
        return Err("development test profile cannot use the system instance".into());
    }
    let root = target
        .worktree
        .ok_or("development profile has no worktree")?;
    read(&root, &id).map(Some)
}

pub fn directory_for(kind: AgentKind) -> Option<PathBuf> {
    match active() {
        Ok(Some(profile)) if profile.kind == kind => Some(profile.directory),
        Ok(Some(profile)) => Some(profile.directory.join("other-clients").join(kind.as_str())),
        Ok(None) => None,
        // Never let a damaged reserved profile flag turn an isolated write into a global write.
        Err(_) => Some(
            crate::paths::config_dir()
                .join("invalid-test-profile")
                .join(kind.as_str()),
        ),
    }
}

pub fn support(kind: AgentKind) -> Support {
    let installed = crate::integrations::agent_launch::readiness(kind).executable;
    let reason = match kind {
        AgentKind::Cursor => Some("this Cursor adapter cannot isolate all global MCP and Hook sources; ordinary Dev verification remains available".to_string()),
        AgentKind::Antigravity => Some("Antigravity has no verified isolated native-session launcher; ordinary Dev verification remains available on supported platforms".to_string()),
        _ if installed.is_none() => Some("Agent executable is not installed".into()),
        _ => None,
    };
    let terminal_available = crate::integrations::agent_launch::development_terminal_available();
    Support {
        terminal_available,
        launch_reason: (!terminal_available).then(|| {
            "no supported terminal is available; use dev session --stdio for a foreground test"
                .into()
        }),
        kind,
        supported: reason.is_none(),
        executable: installed,
        reason,
    }
}

fn client_environment(profile: &Profile) -> BTreeMap<String, String> {
    let key = match profile.kind {
        AgentKind::Claude => "CLAUDE_CONFIG_DIR",
        AgentKind::Codex => "CODEX_HOME",
        AgentKind::Grok => "GROK_HOME",
        AgentKind::Pi => "PI_CODING_AGENT_DIR",
        AgentKind::Cursor => "CURSOR_CONFIG_DIR",
        AgentKind::Antigravity => "ASKHUMAN_UNSUPPORTED_PROFILE",
    };
    BTreeMap::from([
        (key.into(), profile.directory.to_string_lossy().to_string()),
        (PROFILE_ENV.into(), profile.id.clone()),
    ])
}

pub fn configure_process(
    command: &mut std::process::Command,
    profile: &Profile,
) -> Result<(), String> {
    super::clear_client_context(command);
    routing::configure(command, &routing::dev_target(&profile.worktree)?)?;
    super::verification::clear_native_context(command);
    command.env_remove(super::verification::VERIFICATION_ENV);
    command.envs(client_environment(profile));
    Ok(())
}

pub fn prepare(root: &Path, kind: AgentKind) -> Result<Profile, String> {
    let support = support(kind);
    if !support.supported {
        return Err(support.reason.unwrap_or_default());
    }
    let build = build::verify(root)?;
    let root = build.worktree.clone();
    let directory = root
        .join(crate::dev_instance::DEV_DIR)
        .join("clients")
        .join(kind.as_str());
    let private_root = root
        .join(crate::dev_instance::DEV_DIR)
        .canonicalize()
        .map_err(|e| e.to_string())?;
    std::fs::create_dir_all(directory.parent().unwrap()).map_err(|e| e.to_string())?;
    if !directory
        .parent()
        .unwrap()
        .canonicalize()
        .map_err(|e| e.to_string())?
        .starts_with(&private_root)
    {
        return Err("development client directory escapes the worktree".into());
    }
    std::fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
    if !directory
        .canonicalize()
        .map_err(|e| e.to_string())?
        .starts_with(&private_root)
    {
        return Err("development client profile escapes the worktree".into());
    }
    let _lock = crate::file_lock::FileLock::exclusive(&directory.join("profile.lock"))
        .map_err(|e| e.to_string())?;
    let index = directory.join("askhuman-profile.json");
    let previous = match std::fs::read(&index) {
        Ok(bytes) => Some(
            serde_json::from_slice::<Profile>(&bytes)
                .map_err(|e| format!("invalid existing development profile: {e}"))?,
        ),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(error.to_string()),
    };
    let (id, workspace) = if let Some(previous) = previous {
        let valid = read(&root, &previous.id)?;
        (valid.id, valid.neutral_workspace)
    } else {
        let id = uuid::Uuid::new_v4().to_string();
        let workspace = tempfile::Builder::new()
            .prefix("askhuman-dev-session-")
            .tempdir()
            .map_err(|e| e.to_string())?
            .keep();
        state::write_json(
            &workspace.join("askhuman-owner.json"),
            &serde_json::json!({ "profile": id, "worktree": root }),
        )?;
        let git = std::process::Command::new("git")
            .args(["init", "--quiet"])
            .arg(&workspace)
            .stdin(std::process::Stdio::null())
            .output()
            .map_err(|e| format!("cannot initialize isolated test workspace: {e}"))?;
        if !git.status.success() {
            return Err(
                "cannot initialize an isolated repository for native client configuration".into(),
            );
        }
        (id, workspace)
    };
    let profile = Profile {
        version: 1,
        id,
        kind,
        worktree: root,
        directory,
        neutral_workspace: workspace,
        executable: support
            .executable
            .ok_or("Agent executable is unavailable")?,
        build,
        prepared: false,
    };
    write(&profile)?;
    let target = routing::dev_target(&profile.worktree)?;
    let mut command = std::process::Command::new(&target.binary);
    configure_process(&mut command, &profile)?;
    let output = command
        .args(["dev", "session-configure", &profile.id])
        .output()
        .map_err(|e| e.to_string())?;
    if !output.status.success() {
        let diagnostic = profile.directory.join("askhuman-prepare.stderr.txt");
        crate::integrations::hook_edit::atomic_write_private(&diagnostic, &output.stderr)
            .map_err(|e| e.to_string())?;
        return Err(format!(
            "cannot prepare isolated Agent integration; private diagnostics: {}",
            diagnostic.display()
        ));
    }
    let mut profile = profile;
    profile.prepared = true;
    write(&profile)?;
    state::write_json(&index, &profile)?;
    Ok(profile)
}

pub fn configure_active(id: &str) -> Result<(), String> {
    use crate::integrations::{
        agent_lifecycle, agent_mode, agent_permission, agent_rules::AgentTarget, agent_stop,
    };
    let profile = active()?.ok_or("no isolated development profile")?;
    if profile.id != id {
        return Err("development profile does not match the configuration request".into());
    }
    let target = match profile.kind {
        AgentKind::Claude => AgentTarget::ClaudeCode,
        AgentKind::Codex => AgentTarget::Codex,
        AgentKind::Grok => AgentTarget::Grok,
        AgentKind::Pi => AgentTarget::Pi,
        _ => return Err("client has no isolated integration adapter".into()),
    };
    let mode = if profile.kind == AgentKind::Pi {
        agent_mode::Mode::Cli
    } else {
        agent_mode::Mode::Mcp
    };
    let mut env = client_environment(&profile);
    let resolved = routing::dev_target(&profile.worktree)?;
    env.insert(
        routing::TARGET_ENV.into(),
        serde_json::to_string(&resolved).map_err(|e| e.to_string())?,
    );
    env.insert(
        crate::dev_instance::ASKHUMAN_HOME_ENV.into(),
        resolved.home.to_string_lossy().to_string(),
    );
    env.insert("ASKHUMAN_NO_KEYCHAIN".into(), "1".into());
    if profile.kind == AgentKind::Codex {
        agent_mode::set_with_env(target, mode, &env).map_err(|e| e.to_string())?;
    } else {
        agent_mode::set(target, mode).map_err(|e| e.to_string())?;
        if profile.kind == AgentKind::Claude {
            let path = crate::paths::claude_json();
            let mut value: serde_json::Value =
                serde_json::from_slice(&std::fs::read(&path).map_err(|e| e.to_string())?)
                    .map_err(|e| e.to_string())?;
            value["mcpServers"]["askhuman"]["env"] =
                serde_json::to_value(&env).map_err(|e| e.to_string())?;
            state::write_json(&path, &value)?;
        } else if profile.kind == AgentKind::Grok {
            let path = crate::paths::grok_config_toml();
            let text = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
            let mut doc = text
                .parse::<toml_edit::DocumentMut>()
                .map_err(|e| e.to_string())?;
            let mut values = toml_edit::Table::new();
            for (key, value) in &env {
                values.insert(key, toml_edit::value(value));
            }
            doc["mcp_servers"]["askhuman"]["env"] = toml_edit::Item::Table(values);
            std::fs::write(path, doc.to_string()).map_err(|e| e.to_string())?;
        }
    }
    agent_lifecycle::set_enabled(profile.kind, true).map_err(|e| e.to_string())?;
    if agent_stop::supported(profile.kind) {
        agent_stop::set_enabled(profile.kind, true).map_err(|e| e.to_string())?;
    }
    if matches!(profile.kind, AgentKind::Claude | AgentKind::Codex) {
        agent_permission::set_enabled(target, true).map_err(|e| e.to_string())?;
    }
    Ok(())
}

pub fn restore_for_launch(profile_id: &str) -> Result<Profile, String> {
    let target = routing::inherited()?.ok_or("development launch has no resolved target")?;
    let root = target
        .worktree
        .ok_or("development launch has no worktree")?;
    let profile = read(&root, profile_id)?;
    if !profile.prepared
        || build::verify(&root)?.binary_fingerprint != profile.build.binary_fingerprint
    {
        return Err("development test profile requires preparation for the current build".into());
    }
    // This helper has not created a runtime or threads; the environment scopes only this launch.
    for (key, value) in client_environment(&profile) {
        std::env::set_var(key, value);
    }
    Ok(profile)
}

pub fn configure_agent_command(
    command: &mut std::process::Command,
    profile: &Profile,
) -> Result<(), String> {
    configure_process(command, profile)?;
    match profile.kind {
        AgentKind::Claude => {
            command
                .args([
                    "--setting-sources",
                    "user",
                    "--strict-mcp-config",
                    "--mcp-config",
                ])
                .arg(profile.directory.join(".claude.json"));
        }
        AgentKind::Codex | AgentKind::Grok => {
            // A neutral cwd prevents parent-directory project configs from restoring daily MCPs.
            command.current_dir(&profile.neutral_workspace);
            command.arg("--add-dir").arg(&profile.worktree);
        }
        AgentKind::Pi => {
            command
                .arg("--no-extensions")
                .arg("--extension")
                .arg(profile.directory.join("extensions/askhuman/index.ts"));
        }
        _ => return Err("isolated Agent command is unsupported".into()),
    }
    Ok(())
}

pub fn launch(root: &Path, kind: AgentKind, task: &str) -> Result<serde_json::Value, String> {
    launch_with_options(root, kind, task, false)
}

fn launch_with_options(
    root: &Path,
    kind: AgentKind,
    task: &str,
    stdio: bool,
) -> Result<serde_json::Value, String> {
    if !stdio && !crate::integrations::agent_launch::development_terminal_available() {
        return Err(
            "no supported terminal is available; use dev session --stdio for a foreground test"
                .into(),
        );
    }
    let profile = prepare(root, kind)?;
    let token = uuid::Uuid::new_v4().to_string();
    let path = crate::dev_instance::instance_home(&profile.worktree)
        .join("state/test-profile-tasks")
        .join(format!("{token}.json"));
    state::write_json(
        &path,
        &serde_json::json!({ "profile": profile.id, "task": task }),
    )?;
    let target = routing::dev_target(&profile.worktree)?;
    let mut command = std::process::Command::new(&target.binary);
    configure_process(&mut command, &profile)?;
    command.args(["dev", "session-launch", &profile.id, &token]);
    if stdio {
        command.arg("--stdio");
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            return Err(format!(
                "cannot enter development test session: {}",
                command.exec()
            ));
        }
        #[cfg(not(unix))]
        {
            let status = command.status().map_err(|e| e.to_string())?;
            std::process::exit(status.code().unwrap_or(1));
        }
    }
    let output = command.output().map_err(|e| e.to_string());
    let _ = std::fs::remove_file(&path);
    let output = output?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }
    serde_json::from_slice(&output.stdout)
        .map_err(|e| format!("invalid development launch receipt: {e}"))
}

pub fn launch_active(id: &str, token: &str, stdio: bool) -> Result<serde_json::Value, String> {
    uuid::Uuid::parse_str(token).map_err(|_| "invalid development launch token")?;
    let profile = active()?.ok_or("development test profile is missing")?;
    if profile.id != id || !profile.prepared {
        return Err("development profile is not ready".into());
    }
    let path = crate::dev_instance::instance_home(&profile.worktree)
        .join("state/test-profile-tasks")
        .join(format!("{token}.json"));
    let bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
    if bytes.len() > 32 * 1024 {
        return Err("development task exceeds its size limit".into());
    }
    let value: serde_json::Value = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    if value["profile"].as_str() != Some(id) {
        return Err("development task names another profile".into());
    }
    std::fs::remove_file(&path).map_err(|e| e.to_string())?;
    let task = value["task"]
        .as_str()
        .filter(|task| !task.trim().is_empty())
        .ok_or("development task is empty")?;
    let prompt = format!("This is an independent AskHuman Dev test session. Target source worktree: {}. Use this isolated profile's AskHuman integration for questions and confirmations. Test task:\n\n{task}", profile.worktree.display());
    let record = crate::integrations::agent_launch::create_development_record(
        &profile,
        crate::integrations::agent_launch::LaunchPermission::AgentDefault,
        &prompt,
        crate::integrations::agent_launch::LaunchMode::New,
    )
    .map_err(|e| e.to_string())?;
    if stdio {
        crate::integrations::agent_launch::run_helper(std::slice::from_ref(&record.id))
            .map_err(|e| e.to_string())?;
    } else {
        crate::integrations::agent_launch::open_development_terminal(&record)
            .map_err(|e| e.to_string())?;
    }
    Ok(
        serde_json::json!({ "profileId": profile.id, "launchId": record.id, "worktree": profile.worktree }),
    )
}

pub fn run_cli(args: &[String]) -> Result<serde_json::Value, String> {
    let mut root = None;
    let mut kind = None;
    let mut task = "Verify the AskHuman integration in this isolated test profile. Check a normal question, permission and Stop flow without changing everyday configuration.".to_string();
    let mut prepare_only = false;
    let mut stdio = false;
    let mut index = 0;
    while let Some(arg) = args.get(index) {
        match arg.as_str() {
            "--worktree" | "--task" => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or_else(|| format!("{arg} requires a value"))?;
                if arg == "--worktree" {
                    root = Some(PathBuf::from(value));
                } else {
                    task = value.clone();
                }
            }
            "--prepare-only" => prepare_only = true,
            "--stdio" => stdio = true,
            value if kind.is_none() => {
                kind = Some(AgentKind::parse(value).ok_or("unknown test Agent")?)
            }
            _ => return Err(format!("unknown development session option: {arg}")),
        }
        index += 1;
    }
    let root = root
        .or_else(|| {
            std::env::current_dir()
                .ok()
                .and_then(|cwd| crate::dev_instance::find_dev_root(&cwd))
        })
        .ok_or("specify an enabled development worktree with --worktree")?;
    let kind = kind.ok_or("dev session requires an Agent: claude, codex, grok, or pi")?;
    if prepare_only {
        return serde_json::to_value(prepare(&root, kind)?).map_err(|e| e.to_string());
    }
    launch_with_options(&root, kind, &task, stdio)
}

fn binding_path(kind: AgentKind, session_id: &str) -> PathBuf {
    use sha2::{Digest, Sha256};
    let name = format!(
        "{:x}",
        Sha256::digest(format!("{}\0{session_id}", kind.as_str()).as_bytes())
    );
    crate::paths::state_dir()
        .join("test-session-bindings")
        .join(format!("{name}.json"))
}

pub fn bind_native(kind: AgentKind, session_id: &str, transcript: Option<&str>) {
    let Ok(Some(profile)) = active() else {
        return;
    };
    if profile.kind != kind || session_id.is_empty() {
        return;
    }
    let transcript = transcript.map(PathBuf::from).filter(|path| {
        path.canonicalize()
            .ok()
            .is_some_and(|path| path.starts_with(&profile.directory))
    });
    let binding = Binding {
        profile: profile.id,
        kind,
        session_id: session_id.to_string(),
        transcript,
    };
    let _ = state::write_json(&binding_path(kind, session_id), &binding);
}

pub fn for_native(kind: AgentKind, session_id: &str) -> Option<Profile> {
    let target = routing::inherited().ok().flatten()?;
    if target.kind != routing::TargetKind::Dev {
        return None;
    }
    let bytes = std::fs::read(binding_path(kind, session_id)).ok()?;
    if bytes.len() > 64 * 1024 {
        return None;
    }
    let binding: Binding = serde_json::from_slice(&bytes).ok()?;
    if binding.kind != kind || binding.session_id != session_id {
        return None;
    }
    read(target.worktree.as_ref()?, &binding.profile).ok()
}

pub fn transcript_for(kind: AgentKind, session_id: &str) -> Option<PathBuf> {
    let profile = for_native(kind, session_id)?;
    let binding: Binding =
        serde_json::from_slice(&std::fs::read(binding_path(kind, session_id)).ok()?).ok()?;
    if let Some(path) = binding.transcript.filter(|path| path.is_file()) {
        return Some(path);
    }
    fn find(root: &Path, session: &str, depth: usize, budget: &mut usize) -> Option<PathBuf> {
        if depth == 0 || *budget == 0 {
            return None;
        }
        for entry in std::fs::read_dir(root).ok()?.flatten() {
            if *budget == 0 {
                break;
            }
            *budget -= 1;
            let file_type = entry.file_type().ok()?;
            let path = entry.path();
            if file_type.is_symlink() {
                continue;
            }
            if file_type.is_dir() {
                if let Some(found) = find(&path, session, depth - 1, budget) {
                    return Some(found);
                }
            } else {
                let name = entry.file_name().to_string_lossy().to_string();
                if name.ends_with(".jsonl") && name.contains(session) {
                    return Some(path);
                }
                if name == "chat_history.jsonl"
                    && path.parent()?.file_name()?.to_string_lossy() == session
                {
                    return Some(path);
                }
            }
        }
        None
    }
    find(&profile.directory, session_id, 8, &mut 10_000)
}

/// Purge only neutral workspaces whose private owner marker still belongs to this worktree.
/// Client profiles, credentials and verification history otherwise remain in .askhuman-dev.
pub fn cleanup_workspaces(root: &Path) -> Result<(), String> {
    let directory = crate::dev_instance::instance_home(root).join("state/test-profiles");
    let entries = match std::fs::read_dir(directory) {
        Ok(v) => v,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e.to_string()),
    };
    if let Ok(bytes) = std::fs::read(crate::dev_instance::instance_home(root).join("agents.json")) {
        let value: serde_json::Value = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
        if value["active"].as_array().is_some_and(|items| {
            items.iter().any(|r| {
                r["pid"]
                    .as_u64()
                    .and_then(|p| u32::try_from(p).ok())
                    .is_some_and(crate::agents::detect::pid_alive)
            })
        }) {
            return Err("close the active Dev test sessions before purging their profiles".into());
        }
    }
    let temp = std::env::temp_dir()
        .canonicalize()
        .map_err(|e| e.to_string())?;
    let mut workspaces = vec![];
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(id) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        if path.extension().is_none_or(|s| s != "json") {
            continue;
        }
        let raw: Profile =
            serde_json::from_slice(&std::fs::read(&path).map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())?;
        if !raw.neutral_workspace.exists() {
            continue;
        }
        let profile = read(root, id)?;
        let workspace = profile
            .neutral_workspace
            .canonicalize()
            .map_err(|e| e.to_string())?;
        if workspace.parent() != Some(temp.as_path())
            || !workspace
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .starts_with("askhuman-dev-session-")
        {
            return Err("development workspace no longer has a managed temporary location".into());
        }
        workspaces.push(workspace);
    }
    for workspace in workspaces {
        std::fs::remove_dir_all(workspace).map_err(|e| e.to_string())?;
    }
    Ok(())
}
