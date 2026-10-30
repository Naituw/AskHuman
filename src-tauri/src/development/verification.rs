//! One-shot Dev asks with durable, build-specific results and no native session authority.

use super::{build, routing, state};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::process::Command;

pub const VERIFICATION_ENV: &str = "ASKHUMAN_DEV_VERIFICATION_ID";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Delivery {
    #[default]
    Local,
    Remote,
    Channels,
}

impl Delivery {
    pub fn parse(raw: &str) -> Result<Self, String> {
        match raw {
            "local" => Ok(Self::Local),
            "remote" => Ok(Self::Remote),
            "channels" => Ok(Self::Channels),
            _ => Err("delivery must be local, remote, or channels".into()),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Diagnostics {
    pub home: PathBuf,
    pub endpoint: PathBuf,
    pub reason: String,
}
impl Diagnostics {
    fn from_target(target: &routing::Target) -> Self {
        Self {
            home: target.home.clone(),
            endpoint: crate::ipc::transport::endpoint_path_for(&target.home, "daemon"),
            reason: target.reason.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VerificationInfo {
    pub id: String,
    pub worktree: PathBuf,
    pub build: build::BuildInfo,
    pub delivery: Delivery,
    #[serde(default)]
    pub relay: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub test_profile: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diagnostics: Option<Diagnostics>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Record {
    pub version: u32,
    pub info: VerificationInfo,
    pub request_fingerprint: String,
    pub status: String,
    pub executor_pid: u32,
    pub stdout: Option<String>,
    pub exit_code: Option<i32>,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Preferences {
    pub delivery: Delivery,
}

pub fn preferences(root: &Path) -> Result<Preferences, String> {
    let path = root
        .join(crate::dev_instance::DEV_DIR)
        .join("verification.json");
    match std::fs::read(&path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map_err(|e| format!("invalid Dev verification preferences: {e}")),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Preferences::default()),
        Err(e) => Err(e.to_string()),
    }
}

pub fn save_preferences(root: &Path, preferences: &Preferences) -> Result<(), String> {
    let target = routing::dev_target(root)?;
    state::write_json(
        &target
            .worktree
            .unwrap()
            .join(crate::dev_instance::DEV_DIR)
            .join("verification.json"),
        preferences,
    )
}

pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn record_path(root: &Path, id: &str) -> Result<PathBuf, String> {
    let uuid = uuid::Uuid::parse_str(id).map_err(|_| "invalid Dev verification ID")?;
    if uuid.to_string() != id {
        return Err("Dev verification ID must be a canonical UUID".into());
    }
    Ok(crate::dev_instance::instance_home(root)
        .join("state/verifications")
        .join(format!("{id}.json")))
}

pub fn read_record(root: &Path, id: &str) -> Result<Record, String> {
    use std::io::Read;
    let path = record_path(root, id)?;
    let mut bytes = Vec::new();
    std::fs::File::open(&path)
        .map_err(|e| format!("cannot read Dev verification: {e}"))?
        .take(16 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > 16 * 1024 * 1024 {
        return Err("Dev verification result exceeds 16 MiB".into());
    }
    let record: Record = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    if record.version != 1 || record.info.id != id || !state::same_path(root, &record.info.worktree)
    {
        return Err("Dev verification record identity mismatch".into());
    }
    Ok(record)
}

fn write_record(record: &Record) -> Result<(), String> {
    state::write_json(
        &record_path(&record.info.worktree, &record.info.id)?,
        record,
    )
}

pub fn complete(info: &VerificationInfo, stdout: &str, exit_code: i32) -> Result<(), String> {
    let path = record_path(&info.worktree, &info.id)?;
    let _lock = crate::file_lock::FileLock::exclusive(&path.with_extension("lock"))
        .map_err(|e| e.to_string())?;
    let mut record = read_record(&info.worktree, &info.id)?;
    if record.info.build.binary_fingerprint != info.build.binary_fingerprint {
        return Err("cannot store an answer for a different Dev build".into());
    }
    if record.status != "pending" {
        return Ok(());
    }
    if stdout.len() > 16 * 1024 * 1024 {
        return Err("Dev verification result exceeds 16 MiB".into());
    }
    record.status = if exit_code == 0 {
        "completed"
    } else {
        "failed"
    }
    .into();
    record.stdout = Some(stdout.to_string());
    record.exit_code = Some(exit_code);
    record.updated_at_ms = now_ms();
    write_record(&record)
}

pub fn interrupted(root: &Path, id: &str) -> Result<(), String> {
    let path = record_path(root, id)?;
    let _lock = crate::file_lock::FileLock::exclusive(&path.with_extension("lock"))
        .map_err(|e| e.to_string())?;
    let mut record = read_record(root, id)?;
    if record.status == "pending" {
        record.status = "interrupted".into();
        record.updated_at_ms = now_ms();
        write_record(&record)?;
    }
    Ok(())
}

pub fn decorate_task(task: &mut crate::ipc::TaskRequest) -> Result<(), String> {
    let Some(id) = std::env::var(VERIFICATION_ENV)
        .ok()
        .filter(|v| !v.is_empty())
    else {
        if let Some(profile) = super::session::active()? {
            let actual = build::verify(&profile.worktree)?;
            if actual.binary_fingerprint != profile.build.binary_fingerprint {
                return Err("prepare the development test profile for the current build".into());
            }
            task.message.development = Some(Box::new(VerificationInfo {
                id: uuid::Uuid::new_v4().to_string(),
                worktree: profile.worktree.clone(),
                build: profile.build,
                delivery: Delivery::Channels,
                relay: false,
                diagnostics: Some(Diagnostics::from_target(&routing::dev_target(
                    &profile.worktree,
                )?)),
                test_profile: Some(profile.id),
            }));
            task.project = profile.worktree.to_string_lossy().to_string();
            task.source = format!(
                "Dev · {}",
                profile
                    .worktree
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
            );
        }
        return Ok(());
    };
    let target = routing::inherited()?.ok_or("Dev verification requires a resolved target")?;
    let root = target
        .worktree
        .as_ref()
        .ok_or("Dev verification requires a worktree")?;
    let record = read_record(root, &id)?;
    if record.status != "pending" {
        return Err("Dev verification is no longer pending".into());
    }
    let actual = build::verify(root)?;
    if actual.binary_fingerprint != record.info.build.binary_fingerprint {
        return Err(
            "Dev was rebuilt after this verification started; begin a new verification".into(),
        );
    }
    task.message.development = Some(Box::new(record.info.clone()));
    task.agent_kind = None;
    task.agent_session_id = None;
    task.agent_pid = None;
    task.caller_pid = 0;
    task.mcp_instance_id = Some(format!("dev-verification:{}", record.info.id));
    task.from_mcp = true;
    task.project = root.to_string_lossy().to_string();
    task.source = format!(
        "Dev · {}",
        root.file_name().unwrap_or_default().to_string_lossy()
    );
    Ok(())
}

pub fn clear_native_context(command: &mut Command) {
    for kind in [
        crate::agents::AgentKind::Claude,
        crate::agents::AgentKind::Codex,
        crate::agents::AgentKind::Cursor,
        crate::agents::AgentKind::Grok,
        crate::agents::AgentKind::Pi,
    ] {
        command.env_remove(crate::agents::detect::session_id_env_var(kind));
    }
    for name in [
        crate::cli::MCP_AGENT_KIND_ENV,
        crate::cli::MCP_AGENT_SESSION_ID_ENV,
        crate::cli::MCP_INSTANCE_ID_ENV,
        "ASKHUMAN_AGENT_TASK_LAUNCH_ID",
    ] {
        command.env_remove(name);
    }
}

pub fn validate_task(task: &mut crate::ipc::TaskRequest) -> Result<(), String> {
    let Some(info) = task.message.development.as_ref() else {
        return Ok(());
    };
    if info.relay {
        return super::relay::validate(task);
    }
    let target = routing::inherited()?.ok_or("Dev verification daemon has no resolved target")?;
    if target.kind != routing::TargetKind::Dev
        || !state::same_path(
            &target.home,
            &crate::dev_instance::instance_home(&info.worktree),
        )
    {
        return Err("verification was sent to the wrong instance".into());
    }
    if let Some(profile_id) = &info.test_profile {
        let profile = super::session::read(&info.worktree, profile_id)?;
        if !profile.prepared
            || profile.build.binary_fingerprint != info.build.binary_fingerprint
            || build::verify(&info.worktree)?.binary_fingerprint != info.build.binary_fingerprint
        {
            return Err("development test profile does not match this build".into());
        }
        if let Some(session_id) = task.agent_session_id.as_deref() {
            let binding = super::session::for_native(profile.kind, session_id)
                .ok_or("native session is not bound to a development profile")?;
            if binding.id != profile.id || task.agent_kind.as_deref() != Some(profile.kind.as_str())
            {
                return Err(
                    "native session belongs to a different development test profile".into(),
                );
            }
        } else {
            task.agent_kind = None;
            task.agent_pid = None;
            task.caller_pid = 0;
            task.from_mcp = true;
        }
        task.project = info.worktree.to_string_lossy().to_string();
        return Ok(());
    }
    let record = read_record(&info.worktree, &info.id)?;
    if record.status != "pending"
        || serde_json::to_value(&record.info).map_err(|e| e.to_string())?
            != serde_json::to_value(info).map_err(|e| e.to_string())?
    {
        return Err("verification metadata does not match its pending record".into());
    }
    if build::verify(&info.worktree)?.binary_fingerprint != info.build.binary_fingerprint {
        return Err("verification no longer matches the installed Dev build".into());
    }
    task.agent_kind = None;
    task.agent_session_id = None;
    task.agent_pid = None;
    task.caller_pid = 0;
    task.native_request_id = None;
    task.whats_next = false;
    task.project = info.worktree.to_string_lossy().to_string();
    task.caller_cwd = Some(info.worktree.to_string_lossy().to_string());
    task.mcp_instance_id = Some(format!("dev-verification:{}", info.id));
    task.from_mcp = true;
    task.source = format!(
        "Dev · {}",
        info.worktree
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
    );
    Ok(())
}

pub fn run_cli(args: &[String]) -> Result<(), String> {
    let mut root = None;
    let mut delivery = None;
    let mut id = None;
    let mut resume = None;
    let mut index = 0;
    while let Some(arg) = args.get(index) {
        if arg == "--" {
            index += 1;
            break;
        }
        match arg.as_str() {
            "--worktree" | "--delivery" | "--verification-id" | "--resume" => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or_else(|| format!("{arg} requires a value"))?;
                match arg.as_str() {
                    "--worktree" => root = Some(PathBuf::from(value)),
                    "--delivery" => delivery = Some(Delivery::parse(value)?),
                    "--verification-id" => id = Some(value.clone()),
                    "--resume" => resume = Some(value.clone()),
                    _ => unreachable!(),
                }
            }
            _ => break,
        }
        index += 1;
    }
    let root = root
        .or_else(|| {
            std::env::current_dir()
                .ok()
                .and_then(|cwd| crate::dev_instance::find_dev_root(&cwd))
        })
        .ok_or("no enabled Dev worktree; specify --worktree")?;
    let target = routing::dev_target(&root)?;
    let root = target.worktree.as_ref().unwrap();
    if let Some(id) = resume {
        let record = read_record(root, &id)?;
        let current = build::verify(root)?;
        if current.binary_fingerprint != record.info.build.binary_fingerprint
            || current.source_fingerprint != record.info.build.source_fingerprint
        {
            return Err("verification belongs to a different build; start a new verification for the installed source".into());
        }
        if let (Some(stdout), Some(code)) = (record.stdout, record.exit_code) {
            if !stdout.is_empty() {
                println!("{stdout}");
            }
            std::process::exit(code);
        }
        return Err(format!(
            "Dev verification {id} has no terminal answer ({}); no new question was sent",
            record.status
        ));
    }
    let ask = &args[index..];
    if ask.is_empty() {
        return Err("dev ask requires a question".into());
    }
    if ask.iter().any(|arg| arg == "--whats-next") {
        return Err("Dev verification cannot dequeue everyday tasks; use an independent test session to test whats_next".into());
    }
    let build = build::verify(root)?;
    let id = id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let path = record_path(root, &id)?;
    let fingerprint = format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(ask).map_err(|e| e.to_string())?)
    );
    let _lock = crate::file_lock::FileLock::exclusive(&path.with_extension("lock"))
        .map_err(|e| e.to_string())?;
    if path.exists() {
        let record = read_record(root, &id)?;
        if record.info.build.binary_fingerprint != build.binary_fingerprint
            || record.request_fingerprint != fingerprint
        {
            return Err(
                "Dev verification ID was already used for different inputs or another build".into(),
            );
        }
        if let (Some(stdout), Some(code)) = (record.stdout, record.exit_code) {
            if !stdout.is_empty() {
                println!("{stdout}");
            }
            std::process::exit(code);
        }
        return Err(format!("Dev verification {id} already exists ({}); resume it instead of sending another question", record.status));
    }
    prune_records(root)?;
    let now = now_ms();
    let record = Record {
        version: 1,
        info: VerificationInfo {
            id: id.clone(),
            worktree: root.clone(),
            build,
            delivery: delivery.unwrap_or(preferences(root)?.delivery),
            relay: false,
            test_profile: None,
            diagnostics: Some(Diagnostics::from_target(&target)),
        },
        request_fingerprint: fingerprint,
        status: "pending".into(),
        executor_pid: std::process::id(),
        stdout: None,
        exit_code: None,
        created_at_ms: now,
        updated_at_ms: now,
    };
    if record.info.delivery == Delivery::Remote {
        super::relay::authorize(&record.info)?;
    }
    write_record(&record)?;
    drop(_lock);
    eprintln!(
        "Dev verification {id} · {} · built {}",
        root.display(),
        record.info.build.created_at_ms
    );
    let mut command = Command::new(&target.binary);
    super::clear_client_context(&mut command);
    crate::executable::clear_launch_context(&mut command);
    routing::configure(&mut command, &target)?;
    clear_native_context(&mut command);
    // A leading ask flag prevents a question named `config` from becoming a management command.
    command
        .args(["--output", "text"])
        .args(ask)
        .current_dir(root)
        .env(VERIFICATION_ENV, &id);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        let error = command.exec();
        let _ = interrupted(root, &id);
        Err(format!("cannot start Dev verification: {error}"))
    }
    #[cfg(not(unix))]
    {
        let status = command.status().map_err(|e| e.to_string())?;
        std::process::exit(status.code().unwrap_or(3));
    }
}

pub fn retains_attachments(info: &VerificationInfo) -> bool {
    !info.relay && info.test_profile.is_none()
}
fn attachment_directory(
    info: &VerificationInfo,
    question: usize,
    kind: &str,
) -> Result<PathBuf, String> {
    record_path(&info.worktree, &info.id)?;
    let directory = crate::dev_instance::instance_home(&info.worktree)
        .join("state/verifications")
        .join(&info.id)
        .join(format!("q{question}"))
        .join(kind);
    std::fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700))
            .map_err(|e| e.to_string())?;
    }
    Ok(directory)
}
pub fn retain_reply_files(
    info: &VerificationInfo,
    result: &mut crate::models::ChannelResult,
) -> Result<(), String> {
    if !retains_attachments(info) || !matches!(result.action, crate::models::ChannelAction::Send) {
        return Ok(());
    }
    let mut bytes = 0u64;
    let mut count = 0usize;
    for (question, answer) in result.answers.iter_mut().enumerate() {
        for image in &answer.images {
            bytes = bytes.saturating_add((image.data.len() as u64).saturating_mul(3) / 4);
            count += 1;
        }
        for (index, file) in answer.files.iter_mut().enumerate() {
            let source = PathBuf::from(&*file);
            let metadata = source
                .metadata()
                .map_err(|e| format!("cannot retain verification attachment: {e}"))?;
            bytes = bytes.saturating_add(metadata.len());
            count += 1;
            if !source.is_absolute()
                || !metadata.is_file()
                || metadata.len() > 100 * 1024 * 1024
                || bytes > 200 * 1024 * 1024
                || count > 128
            {
                return Err("verification attachments exceed the request budget".into());
            }
            let name = source
                .file_name()
                .ok_or("verification attachment has no filename")?;
            let target = attachment_directory(info, question, "files")?
                .join(format!("{index}-{}", name.to_string_lossy()));
            if !state::same_path(&source, &target) {
                std::fs::copy(&source, &target).map_err(|e| e.to_string())?;
            }
            *file = target.to_string_lossy().to_string();
        }
    }
    if bytes > 200 * 1024 * 1024 || count > 128 {
        return Err("verification attachments exceed the request budget".into());
    }
    Ok(())
}
pub fn save_reply_images(
    info: &VerificationInfo,
    images: &[crate::models::ImageAttachment],
    question: usize,
    lang: crate::i18n::Lang,
) -> anyhow::Result<Vec<String>> {
    if images.is_empty() {
        return Ok(vec![]);
    }
    let directory = attachment_directory(info, question, "images").map_err(anyhow::Error::msg)?;
    crate::cli::image_writer::save_into_dir(images, &directory, lang)
}

fn prune_records(root: &Path) -> Result<(), String> {
    let dir = crate::dev_instance::instance_home(root).join("state/verifications");
    let entries = match std::fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e.to_string()),
    };
    let mut terminal = vec![];
    let mut pending = 0usize;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_none_or(|e| e != "json") {
            continue;
        }
        let Some(id) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        let Ok(record) = read_record(root, id) else {
            continue;
        };
        if record.status == "pending" && crate::agents::detect::pid_alive(record.executor_pid) {
            pending += 1;
            continue;
        }
        terminal.push((record.updated_at_ms, id.to_string()));
    }
    if pending >= 256 {
        return Err("too many pending Dev verifications".into());
    }
    terminal.sort_by_key(|r| r.0);
    let excess = terminal.len().saturating_sub(999);
    for (index, (time, id)) in terminal.into_iter().enumerate() {
        if index >= excess && now_ms().saturating_sub(time) < 30 * 24 * 60 * 60 * 1000 {
            continue;
        }
        let path = record_path(root, &id)?;
        let Some(_lock) = crate::file_lock::FileLock::try_exclusive(&path.with_extension("lock"))
            .map_err(|e| e.to_string())?
        else {
            continue;
        };
        let _ = std::fs::remove_file(path);
        let files = dir.join(id);
        if files.is_dir() && !files.is_symlink() {
            let _ = std::fs::remove_dir_all(files);
        }
    }
    Ok(())
}

/// A Windows driver cannot exec its child. If it disappears, drop the socket future so the
/// daemon cancels this exact request. Unix exec keeps the record's executor PID unchanged.
pub async fn wait_for_executor_exit(info: &VerificationInfo) {
    if info.relay || info.test_profile.is_some() {
        return std::future::pending().await;
    }
    let Ok(record) = read_record(&info.worktree, &info.id) else {
        return;
    };
    if record.executor_pid == std::process::id() {
        return std::future::pending().await;
    }
    let expected = crate::agents::detect::inspect_process(record.executor_pid);
    loop {
        if !crate::agents::detect::pid_alive(record.executor_pid) {
            return;
        }
        if let (Some(expected), Some(current)) = (
            &expected,
            crate::agents::detect::inspect_process(record.executor_pid),
        ) {
            if expected.creation_time != current.creation_time
                || expected.executable != current.executable
            {
                return;
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verification_ids_cannot_escape_the_instance() {
        assert!(record_path(Path::new("/work"), "../../system").is_err());
        assert!(record_path(Path::new("/work"), &uuid::Uuid::new_v4().to_string()).is_ok());
    }

    #[test]
    fn interrupted_verification_never_accepts_a_late_answer() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let info = VerificationInfo {
            id: uuid::Uuid::new_v4().to_string(),
            worktree: root.clone(),
            delivery: Delivery::Local,
            relay: false,
            test_profile: None,
            diagnostics: None,
            build: build::BuildInfo {
                version: 1,
                worktree: root.clone(),
                binary: root.join("AskHuman"),
                source_fingerprint: "source".into(),
                binary_fingerprint: "binary".into(),
                git_commit: None,
                created_at_ms: 1,
            },
        };
        write_record(&Record {
            version: 1,
            info: info.clone(),
            request_fingerprint: "request".into(),
            status: "pending".into(),
            executor_pid: 1,
            stdout: None,
            exit_code: None,
            created_at_ms: 1,
            updated_at_ms: 1,
        })
        .unwrap();
        interrupted(&root, &info.id).unwrap();
        complete(&info, "late answer", 0).unwrap();
        let record = read_record(&root, &info.id).unwrap();
        assert_eq!(record.status, "interrupted");
        assert!(record.stdout.is_none());
    }
    #[test]
    fn retained_attachments_survive_original_file_changes() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let info = VerificationInfo {
            id: uuid::Uuid::new_v4().to_string(),
            worktree: root.clone(),
            delivery: Delivery::Local,
            relay: false,
            test_profile: None,
            diagnostics: None,
            build: build::BuildInfo {
                version: 1,
                worktree: root.clone(),
                binary: root.join("AskHuman"),
                source_fingerprint: "source".into(),
                binary_fingerprint: "build".into(),
                git_commit: None,
                created_at_ms: 0,
            },
        };
        let file = root.join("human.txt");
        std::fs::write(&file, b"original attachment").unwrap();
        let mut result = crate::models::ChannelResult {
            action: crate::models::ChannelAction::Send,
            source_channel_id: "test".into(),
            answers: vec![crate::models::QuestionAnswer {
                files: vec![file.to_string_lossy().to_string()],
                ..Default::default()
            }],
        };
        retain_reply_files(&info, &mut result).unwrap();
        std::fs::write(&file, b"changed later").unwrap();
        assert_eq!(
            std::fs::read(&result.answers[0].files[0]).unwrap(),
            b"original attachment"
        );
        let image = crate::models::ImageAttachment {
            data: "aW1hZ2U=".into(),
            media_type: "image/png".into(),
            filename: Some("human.png".into()),
        };
        let images = save_reply_images(&info, &[image], 0, crate::i18n::Lang::En).unwrap();
        assert!(Path::new(&images[0]).starts_with(crate::dev_instance::instance_home(&root)));
        assert_eq!(std::fs::read(&images[0]).unwrap(), b"image");
    }
}
