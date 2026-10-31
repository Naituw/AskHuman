//! Credential-free delegation to the everyday instance's delivery channels.

use super::{routing, state, verification};
use crate::channels::{Channel, ConversationOrigin, Interruption, ResultSink};
use crate::models::{AskRequest, ChannelAction, ChannelResult, OutputFormat, QuestionAnswer};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

const VALID_FOR_MS: u64 = 24 * 60 * 60 * 1000;

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Grant {
    version: u32,
    info: verification::VerificationInfo,
    expires_at_ms: u64,
    request_fingerprint: Option<String>,
}

fn grant_path(id: &str) -> Result<PathBuf, String> {
    let uuid = uuid::Uuid::parse_str(id).map_err(|_| "invalid verification grant ID")?;
    if uuid.to_string() != id {
        return Err("non-canonical verification grant ID".into());
    }
    Ok(state::directory()
        .join("relay-grants")
        .join(format!("{id}.json")))
}

pub fn system_target() -> Result<routing::Target, String> {
    let root = routing::inherited()?
        .and_then(|target| target.worktree)
        .or_else(|| {
            std::env::current_dir()
                .ok()
                .and_then(|cwd| super::workspace::find_root(&cwd))
        })
        .ok_or("a formal relay requires an explicit development worktree")?;
    system_target_for(&root)
}

pub fn system_target_for(root: &Path) -> Result<routing::Target, String> {
    let workspace = super::workspace::read(root)?;
    let binary = workspace.system_binary;
    routing::check_capability(&binary)?;
    Ok(routing::Target {
        version: 1,
        kind: routing::TargetKind::System,
        binary,
        home: state::system_home(),
        worktree: None,
        reason: "explicit development delivery relay".into(),
        no_keychain: false,
    })
}

pub fn authorize(info: &verification::VerificationInfo) -> Result<(), String> {
    preflight(&info.worktree)?;
    let path = grant_path(&info.id)?;
    let directory = path.parent().unwrap();
    let _lock = crate::file_lock::FileLock::exclusive(&directory.join("grants.lock"))
        .map_err(|e| e.to_string())?;
    let mut count = 0;
    for entry in std::fs::read_dir(directory)
        .map_err(|e| e.to_string())?
        .flatten()
    {
        if entry.path().extension().is_none_or(|s| s != "json") {
            continue;
        }
        let expired = read_grant(&entry.path()).is_ok_and(|grant| {
            grant.expires_at_ms <= verification::now_ms()
                || verification::read_record(&grant.info.worktree, &grant.info.id)
                    .is_ok_and(|record| record.status != "pending")
        });
        if expired {
            let _ = std::fs::remove_file(entry.path());
        } else {
            count += 1;
        }
    }
    if count >= 256 {
        return Err("too many active development relay grants".into());
    }
    state::write_json(
        &path,
        &Grant {
            version: 1,
            info: info.clone(),
            expires_at_ms: verification::now_ms().saturating_add(VALID_FOR_MS),
            request_fingerprint: None,
        },
    )
}

fn read_grant(path: &Path) -> Result<Grant, String> {
    use std::io::Read;
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .map_err(|e| e.to_string())?
        .take(64 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > 64 * 1024 {
        return Err("relay grant is too large".into());
    }
    serde_json::from_slice(&bytes).map_err(|e| format!("invalid relay grant: {e}"))
}

pub fn validate(task: &mut crate::ipc::TaskRequest) -> Result<(), String> {
    let info = task
        .message
        .development
        .as_ref()
        .ok_or("relay metadata is missing")?;
    if !info.relay || info.delivery != verification::Delivery::Remote {
        return Err("request is not a remote development relay".into());
    }
    let target = routing::inherited()?.ok_or("relay daemon has no resolved target")?;
    if target.kind != routing::TargetKind::System
        || !state::same_path(&target.home, &state::system_home())
    {
        return Err("development relay must use the everyday system instance".into());
    }
    let fingerprint = serde_json::to_vec(&(
        &task.message,
        &task.questions,
        task.select_only,
        task.single,
    ))
    .map_err(|e| e.to_string())?;
    validate_grant(info, &fingerprint)?;
    strip_todos(&mut task.questions);
    task.record_history = false;
    task.stop_confirmation = false;
    task.whats_next = false;
    task.agent_kind = None;
    task.agent_session_id = None;
    task.agent_pid = None;
    task.caller_pid = 0;
    task.native_request_id = Some(format!("dev-relay:{}", info.id));
    task.mcp_instance_id = Some(format!("dev-relay:{}", info.id));
    task.from_mcp = true;
    task.project = info.worktree.to_string_lossy().to_string();
    task.caller_cwd = Some(info.worktree.to_string_lossy().to_string());
    task.source = format!(
        "Dev · {}",
        info.worktree
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
    );
    Ok(())
}

fn strip_todos(questions: &mut [crate::models::Question]) {
    for question in questions {
        for option in &mut question.predefined_options {
            option.todo_id = None;
            option.todo_text = None;
            option.todo_attachments.clear();
        }
    }
}

fn validate_grant(info: &verification::VerificationInfo, request: &[u8]) -> Result<(), String> {
    if !info.relay || info.delivery != verification::Delivery::Remote {
        return Err("invalid relay metadata".into());
    }
    let target = routing::inherited()?.ok_or("relay target is missing")?;
    if target.kind != routing::TargetKind::System
        || !state::same_path(&target.home, &state::system_home())
    {
        return Err("relay requires the formal instance".into());
    }
    let path = grant_path(&info.id)?;
    let _lock = crate::file_lock::FileLock::exclusive(&path.with_extension("lock"))
        .map_err(|e| e.to_string())?;
    if verification::read_record(&info.worktree, &info.id)?.status != "pending" {
        return Err("development request is no longer pending".into());
    }
    let mut grant = read_grant(&path)?;
    claim(
        &mut grant,
        info,
        format!("{:x}", Sha256::digest(request)),
        verification::now_ms(),
    )?;
    state::write_json(&path, &grant)
}

pub fn validate_confirm(task: &mut crate::ipc::ConfirmTask) -> Result<(), String> {
    let info = task
        .development
        .as_ref()
        .ok_or("relay permission metadata missing")?;
    validate_grant(
        info,
        &serde_json::to_vec(&task.spec).map_err(|e| e.to_string())?,
    )?;
    task.memory = None;
    task.popup_edit = None;
    task.agent_session_id = format!("dev-relay:{}", info.id);
    task.caller_pid = 0;
    task.project = info.worktree.to_string_lossy().into_owned();
    task.caller_cwd = None;
    task.source = format!(
        "Dev · {}",
        info.worktree
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
    );
    Ok(())
}

/// Query the formal daemon without Hello, so an incompatible peer can never be replaced.
pub async fn check_peer() -> Result<(), String> {
    let status = crate::client::request_status_result().await.map_err(|e|e.to_string())?
        .ok_or("the formal daemon is not running; start the formal instance before remote verification")?;
    if status.protocol_version != crate::ipc::PROTOCOL_VERSION
        || status.development_verification_version != super::CAPABILITY_VERSION
        || status.fingerprint != Some(crate::daemon::lifecycle::current_fingerprint())
        || status.draining
    {
        return Err("the formal relay requires a compatible running formal daemon; no replacement was attempted".into());
    }
    if status.development_remote_outlets.is_empty() {
        return Err("the formal instance has no deliverable remote channel; configure one or explicitly select local delivery".into());
    }
    Ok(())
}

pub fn preflight(root: &Path) -> Result<(), String> {
    let target = system_target_for(root)?;
    let mut command = std::process::Command::new(&target.binary);
    routing::configure(&mut command, &target)?;
    super::clear_client_context(&mut command);
    verification::clear_native_context(&mut command);
    let mut child = command
        .arg("__dev-relay-preflight")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| e.to_string())?;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while child.try_wait().map_err(|e| e.to_string())?.is_none() {
        if std::time::Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err("formal relay preflight timed out".into());
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let output = child.wait_with_output().map_err(|e| e.to_string())?;
    if !output.status.success() {
        return Err(format!(
            "formal relay is unavailable: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(())
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", content = "task", rename_all = "camelCase")]
enum Payload {
    Ask(Box<crate::ipc::TaskRequest>),
    Confirm(Box<crate::ipc::ConfirmTask>),
}

fn claim(
    grant: &mut Grant,
    info: &verification::VerificationInfo,
    fingerprint: String,
    now: u64,
) -> Result<(), String> {
    let mut original = info.clone();
    original.relay = false;
    if grant.version != 1
        || grant.expires_at_ms <= now
        || serde_json::to_value(&grant.info).map_err(|e| e.to_string())?
            != serde_json::to_value(&original).map_err(|e| e.to_string())?
    {
        return Err(
            "development relay grant expired or does not match this request and build".into(),
        );
    }
    if grant
        .request_fingerprint
        .as_ref()
        .is_some_and(|previous| previous != &fingerprint)
    {
        return Err("relay grant was already used for a different question".into());
    }
    grant.request_fingerprint = Some(fingerprint);
    Ok(())
}

fn payload_path(root: &Path, id: &str) -> Result<PathBuf, String> {
    grant_path(id)?;
    Ok(crate::dev_instance::instance_home(root)
        .join("state/relay")
        .join(format!("{id}.json")))
}

pub fn run_cli(root: &Path, id: &str) -> Result<(), String> {
    use std::io::Read;
    let path = payload_path(root, id)?;
    let mut bytes = Vec::new();
    std::fs::File::open(&path)
        .map_err(|e| e.to_string())?
        .take(2 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > 2 * 1024 * 1024 {
        return Err("relay request is too large".into());
    }
    let payload: Payload = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    let info = match &payload {
        Payload::Ask(task) => task.message.development.as_deref(),
        Payload::Confirm(task) => task.development.as_deref(),
    }
    .ok_or("relay metadata missing")?;
    if info.id != id || !state::same_path(root, &info.worktree) {
        return Err("relay payload identity mismatch".into());
    }
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| e.to_string())?;
    runtime.block_on(check_peer())?;
    drop(runtime);
    match payload {
        Payload::Ask(mut task) => {
            validate(&mut task)?;
            crate::client::run_ask(*task)
        }
        Payload::Confirm(mut task) => {
            validate_confirm(&mut task)?;
            let result = crate::client::run_confirm(*task)
                .ok_or("formal permission relay ended without a human decision")?;
            println!(
                "{}",
                serde_json::to_string(&result).map_err(|e| e.to_string())?
            );
            Ok(())
        }
    }
}

pub struct RelayChannel {
    cancel: CancellationToken,
}

impl RelayChannel {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            cancel: CancellationToken::new(),
        })
    }
}

impl Channel for RelayChannel {
    fn id(&self) -> &str {
        "dev-relay"
    }

    fn start(&self, request: &AskRequest, _origin: &ConversationOrigin, sink: ResultSink) {
        let request = request.clone();
        let cancel = self.cancel.clone();
        tokio::spawn(async move {
            if let Err(error) = forward(request, cancel.clone(), sink.clone()).await {
                if !cancel.is_cancelled() {
                    sink.surface_lost("dev-relay", &error);
                }
            }
        });
    }

    fn interrupt(&self, _reason: &Interruption) {
        self.cancel.cancel();
    }
}

async fn forward(
    request: AskRequest,
    cancel: CancellationToken,
    sink: ResultSink,
) -> Result<(), String> {
    let mut info = request
        .message
        .development
        .clone()
        .ok_or("Dev relay metadata missing")?;
    info.relay = true;
    let system = system_target_for(&info.worktree)?;
    let mut message = request.message.clone();
    message.development = Some(info.clone());
    let label = if crate::i18n::Lang::current() == crate::i18n::Lang::Zh {
        "开发验收"
    } else {
        "Development verification"
    };
    message.text = format!(
        "{label} · {} · {}\n\n{}",
        info.worktree
            .file_name()
            .unwrap_or_default()
            .to_string_lossy(),
        crate::local_time::absolute((info.build.created_at_ms / 1000) as i64)
            .unwrap_or_else(|| info.build.created_at_ms.to_string()),
        message.text
    );
    let mut task = crate::ipc::TaskRequest {
        caller_cwd: Some(info.worktree.to_string_lossy().to_string()),
        native_request_id: None,
        message,
        questions: request.questions.clone(),
        is_markdown: request.is_markdown,
        source: "Dev".into(),
        lang: crate::i18n::Lang::current().code().into(),
        project: info.worktree.to_string_lossy().to_string(),
        select_only: request.select_only,
        single: request.single,
        output_format: OutputFormat::Json,
        record_history: false,
        stop_confirmation: false,
        agent_kind: None,
        agent_session_id: None,
        mcp_instance_id: None,
        agent_pid: None,
        caller_pid: 0,
        from_mcp: true,
        perf_id: String::new(),
        perf_autodismiss: false,
        whats_next: false,
    };
    let path = payload_path(&info.worktree, &info.id)?;
    strip_todos(&mut task.questions);
    state::write_json(&path, &Payload::Ask(Box::new(task)))?;
    let mut command = std::process::Command::new(&system.binary);
    routing::configure(&mut command, &system)?;
    verification::clear_native_context(&mut command);
    command.arg("__dev-relay").arg(&info.worktree).arg(&info.id);
    let output = tokio::time::timeout(
        std::time::Duration::from_millis(VALID_FOR_MS),
        crate::mcp::ask::capture_output(command.into(), cancel.clone()),
    )
    .await;
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(grant_path(&info.id)?);
    let output = output
        .map_err(|_| "development relay expired")?
        .map_err(|error| match error {
            crate::mcp::ask::CaptureError::Cancelled => "development relay cancelled".to_string(),
            crate::mcp::ask::CaptureError::Io(error) => error.to_string(),
        })?;
    if !output.status.success() {
        return Err(format!(
            "development delivery failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    if cancel.is_cancelled() {
        return Err("development relay cancelled".into());
    }
    let result = decode_result(&output.stdout, &request, &info)?;
    sink.submit(result);
    Ok(())
}

pub fn start_confirm(
    entry: Arc<crate::daemon::request::ConfirmEntry>,
    mut task: crate::ipc::ConfirmTask,
) {
    entry.start_delivery("dev-relay");
    tokio::spawn(async move {
        let result = async {
            let info = task.development.as_mut().ok_or("permission relay metadata missing")?;
            info.relay = true;
            let info = info.clone();
            let system = system_target_for(&info.worktree)?;
            task.memory = None;
            task.popup_edit = None;
            task.agent_session_id = format!("dev-relay:{}", info.id);
            task.caller_pid = 0;
            task.spec.title = format!("Dev · {} · {}", info.worktree.file_name().unwrap_or_default().to_string_lossy(), task.spec.title);
            let path = payload_path(&info.worktree, &info.id)?;
            state::write_json(&path, &Payload::Confirm(Box::new(task)))?;
            let mut command = std::process::Command::new(&system.binary);
            routing::configure(&mut command, &system)?;
            super::clear_client_context(&mut command);
            verification::clear_native_context(&mut command);
            command.arg("__dev-relay").arg(&info.worktree).arg(&info.id);
            let cancel = CancellationToken::new();
            let output = tokio::select! {
                output = crate::mcp::ask::capture_output(command.into(), cancel.clone()) => output,
                _ = async {
                    loop {
                        if entry.coordinator.is_terminal() { break; }
                        tokio::select! { _ = entry.cancel.notified() => break, _ = tokio::time::sleep(std::time::Duration::from_millis(100)) => {} }
                    }
                } => { cancel.cancel(); Err(crate::mcp::ask::CaptureError::Cancelled) }
            };
            let _ = std::fs::remove_file(path);
            let _ = std::fs::remove_file(grant_path(&info.id)?);
            let output = output.map_err(|_|"permission relay cancelled or disconnected")?;
            if !output.status.success() { return Err(format!("permission relay failed: {}", String::from_utf8_lossy(&output.stderr).trim())); }
            let result: crate::models::ConfirmResult = serde_json::from_slice(&output.stdout).map_err(|e|e.to_string())?;
            let index = entry.request.choices.iter().position(|choice|choice.id == result.action_id).ok_or("permission relay returned an unknown action")?;
            entry.coordinator.submit_wire(index, result.comment, "dev-relay")?;
            Ok::<(), String>(())
        }.await;
        if let Err(error) = result {
            eprintln!("AskHuman: {error}");
            entry.mark_failed("dev-relay", error);
            entry
                .coordinator
                .fallback(crate::models::ConfirmFallbackReason::NoAvailableChannel);
        }
    });
}

fn decode_result(
    bytes: &[u8],
    request: &AskRequest,
    info: &verification::VerificationInfo,
) -> Result<ChannelResult, String> {
    if bytes.len() > 16 * 1024 * 1024 {
        return Err("relay result is too large".into());
    }
    let value: serde_json::Value = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
    let action = match value["action"].as_str() {
        Some("answer") => ChannelAction::Send,
        Some("cancel") => ChannelAction::Cancel,
        _ => return Err("relay returned no human decision".into()),
    };
    let mut answers = vec![QuestionAnswer::default(); request.questions.len()];
    let mut attachment_bytes = 0u64;
    let mut attachment_count = 0usize;
    if let Some(values) = value["answers"].as_array() {
        let mut seen = std::collections::HashSet::new();
        for value in values {
            let index = value["question_index"]
                .as_u64()
                .ok_or("relay answer has no question index")? as usize;
            if index >= answers.len() || !seen.insert(index) {
                return Err("relay answer question index is invalid".into());
            }
            answers[index].selected_options = if value["selected_options"].is_null() {
                vec![]
            } else {
                serde_json::from_value(value["selected_options"].clone())
                    .map_err(|_| "relay returned invalid selected options")?
            };
            answers[index].user_input = value["user_input"].as_str().map(str::to_string);
            if let Some(files) = value["files"].as_array() {
                if files.len() > 32 {
                    return Err("relay returned too many attachments".into());
                }
                for (file_index, file) in files.iter().enumerate() {
                    let path =
                        PathBuf::from(file.as_str().ok_or("relay attachment is not a path")?);
                    let metadata = std::fs::metadata(&path).map_err(|e| e.to_string())?;
                    if !path.is_absolute()
                        || !metadata.is_file()
                        || metadata.len() > 100 * 1024 * 1024
                    {
                        return Err("relay attachment is not an allowed file".into());
                    }
                    attachment_count += 1;
                    attachment_bytes = attachment_bytes.saturating_add(metadata.len());
                    if attachment_count > 128 || attachment_bytes > 200 * 1024 * 1024 {
                        return Err("relay attachments exceed the request budget".into());
                    }
                    let directory = crate::dev_instance::instance_home(&info.worktree)
                        .join("state/verifications")
                        .join(&info.id)
                        .join(format!("q{index}"));
                    std::fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
                    let name = path
                        .file_name()
                        .ok_or("attachment has no filename")?
                        .to_string_lossy();
                    let target = directory.join(format!("{file_index}-{name}"));
                    std::fs::copy(&path, &target).map_err(|e| e.to_string())?;
                    let extension = target
                        .extension()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .to_ascii_lowercase();
                    let media_type = match extension.as_str() {
                        "png" => Some("image/png"),
                        "jpg" | "jpeg" => Some("image/jpeg"),
                        "webp" => Some("image/webp"),
                        "gif" => Some("image/gif"),
                        _ => None,
                    };
                    if let Some(media_type) = media_type {
                        use base64::Engine;
                        let bytes = std::fs::read(&target).map_err(|e| e.to_string())?;
                        answers[index].images.push(crate::models::ImageAttachment {
                            data: base64::engine::general_purpose::STANDARD.encode(bytes),
                            media_type: media_type.into(),
                            filename: Some(name.to_string()),
                        });
                    } else {
                        answers[index]
                            .files
                            .push(target.to_string_lossy().to_string());
                    }
                }
            }
        }
    }
    Ok(ChannelResult {
        action,
        answers,
        source_channel_id: "dev-relay".into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture(root: &Path) -> (AskRequest, verification::VerificationInfo) {
        let request = AskRequest::new(
            crate::models::MessagePrompt::default(),
            vec![
                crate::models::Question::new("first".into(), vec![]),
                crate::models::Question::new("second".into(), vec![]),
            ],
            false,
        );
        let info = verification::VerificationInfo {
            id: uuid::Uuid::new_v4().to_string(),
            worktree: root.into(),
            delivery: verification::Delivery::Remote,
            relay: false,
            test_profile: None,
            diagnostics: None,
            build: super::super::build::BuildInfo {
                version: 1,
                worktree: root.into(),
                binary: root.join("AskHuman"),
                source_fingerprint: "source".into(),
                binary_fingerprint: "binary".into(),
                git_commit: None,
                created_at_ms: 0,
            },
        };
        (request, info)
    }
    #[test]
    fn sparse_answers_keep_the_original_question_indices_and_image_attachments() {
        let temp = tempfile::tempdir().unwrap();
        let (request, info) = fixture(temp.path());
        let path = temp.path().join("image.png");
        std::fs::write(&path, b"image payload").unwrap();
        let value = serde_json::json!({"action":"answer","answers":[{"question_index":1,"selected_options":["B"],"files":[path]}]});
        let result = decode_result(&serde_json::to_vec(&value).unwrap(), &request, &info).unwrap();
        assert!(result.answers[0].is_empty());
        assert_eq!(result.answers[1].selected_options, vec!["B"]);
        assert_eq!(result.answers[1].images.len(), 1);
        assert!(result.answers[1].files.is_empty());
    }
    #[test]
    fn duplicate_or_out_of_range_answers_are_rejected() {
        let temp = tempfile::tempdir().unwrap();
        let (request, info) = fixture(temp.path());
        for indices in [vec![0, 0], vec![2]] {
            let value = serde_json::json!({"action":"answer","answers":indices.into_iter().map(|i|serde_json::json!({"question_index":i})).collect::<Vec<_>>()});
            assert!(decode_result(&serde_json::to_vec(&value).unwrap(), &request, &info).is_err());
        }
    }
    #[test]
    fn missing_decision_and_invalid_options_are_errors() {
        let temp = tempfile::tempdir().unwrap();
        let (request, info) = fixture(temp.path());
        for value in [
            serde_json::json!({"error":"offline"}),
            serde_json::json!({"action":"answer","answers":[{"question_index":0,"selected_options":123}]}),
        ] {
            assert!(decode_result(&serde_json::to_vec(&value).unwrap(), &request, &info).is_err());
        }
        assert!(matches!(
            decode_result(br#"{"action":"cancel"}"#, &request, &info)
                .unwrap()
                .action,
            ChannelAction::Cancel
        ));
    }
    #[test]
    fn relay_grants_bind_one_worktree_build_question_and_expiry() {
        let root = tempfile::tempdir().unwrap();
        let (_, info) = fixture(root.path());
        let mut grant = Grant {
            version: 1,
            info: info.clone(),
            expires_at_ms: 100,
            request_fingerprint: None,
        };
        let mut presented = info.clone();
        presented.relay = true;
        claim(&mut grant, &presented, "same question".into(), 50).unwrap();
        claim(&mut grant, &presented, "same question".into(), 51).unwrap();
        assert!(claim(&mut grant, &presented, "different question".into(), 51).is_err());
        assert!(claim(&mut grant, &presented, "same question".into(), 100).is_err());
        presented.build.binary_fingerprint = "different build".into();
        assert!(claim(&mut grant, &presented, "same question".into(), 51).is_err());
        presented = info;
        presented.worktree = root.path().join("other");
        assert!(claim(&mut grant, &presented, "same question".into(), 51).is_err());
    }
}
