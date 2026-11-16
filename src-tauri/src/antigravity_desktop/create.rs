//! Creating a task in the Antigravity App (docs/specs/antigravity-deep-integration.md §12).
//!
//! The App cannot be told to open a conversation in a folder of our choosing, but a sidecar it
//! runs for that folder can: the sidecar's `agentapi new-conversation` makes one, and the actions
//! granted to the sidecar (a whitelist the user can see and edit) are the only ones the
//! conversation's agent may take without asking. The daemon plans everything that can be planned
//! without the App (what the operation is, the receipt that makes a retry safe, the sidecar the
//! folder needs), brings the App and the sidecar up when they are not, and only then has the
//! sidecar run the one call. What it hears back decides what the receipt says, and so whether
//! trying again can start a second conversation.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use super::launch::{self, UnavailableReason};
use super::link::{self, Command, LinkInfo, Outcome, RunError};
use super::sidecar::{self, Change};
use crate::i18n::Lang;

/// How long starting the App, the App's reading of a new sidecar and the sidecar's check-in may
/// take, all together.
const ONLINE_BUDGET: Duration = Duration::from_secs(30);
/// The longest title made from the task text.
const TITLE_CHARS: usize = 60;
/// How long the receipt of a finished operation is kept.
const RECEIPT_KEEP: Duration = Duration::from_secs(30 * 24 * 3600);

/// One task to create. Also the JSON `__antigravity-desktop` reads.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Request {
    /// The project folder the conversation belongs to.
    pub cwd: String,
    pub text: String,
    /// `agent-default` or `yolo`.
    pub permission: String,
    /// The operation's id, and the name of its receipt: a retry with the same id finds what the
    /// first try got to.
    pub id: String,
    /// The model picked for this task, as `agentapi --model` takes it; absent leaves it to the App.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// The conversation's title; absent makes one from the first line of the task.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Bring the route up and check it, then stop before anything is sent to a model.
    #[serde(default)]
    pub dry_run: bool,
}

// MARK: - What can go wrong

/// Why a task was not created, or may not have been. Every variant but [`Problem::Unclear`] means
/// no conversation exists.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Problem {
    /// The request itself is wrong.
    Invalid(String),
    /// The operation id was used for different content.
    IdReused,
    Unavailable(UnavailableReason),
    /// AskHuman already manages as many sidecars as it may.
    TooManySidecars,
    /// A sidecar folder of that name exists that AskHuman did not make.
    Conflict(String),
    /// AskHuman's or Antigravity's configuration files could not be read or written.
    Files(String),
    AppNotStarted(String),
    /// The sidecar did not check in in time. `started`: AskHuman started the App for this task.
    NotOnline {
        started: bool,
    },
    /// The sidecar cannot find `agentapi` in its environment.
    NoAgentapi,
    /// The command was not sent, so trying again cannot start two conversations.
    NotSent(String),
    /// `agentapi` ran and refused.
    Rejected(String),
    /// The command may have run: look in the App before trying again.
    Unclear {
        directory: String,
        detail: Option<String>,
    },
}

impl Problem {
    /// A short stable name for logs.
    pub(super) fn code(&self) -> &'static str {
        match self {
            Problem::Invalid(_) => "invalid",
            Problem::IdReused => "id_reused",
            Problem::Unavailable(_) => "unavailable",
            Problem::TooManySidecars => "too_many_sidecars",
            Problem::Conflict(_) => "sidecar_conflict",
            Problem::Files(_) => "files",
            Problem::AppNotStarted(_) => "app_not_started",
            Problem::NotOnline { .. } => "sidecar_offline",
            Problem::NoAgentapi => "no_agentapi",
            Problem::NotSent(_) => "not_sent",
            Problem::Rejected(_) => "rejected",
            Problem::Unclear { .. } => "unclear",
        }
    }

    pub(super) fn text(&self, lang: Lang) -> String {
        let seconds = ONLINE_BUDGET.as_secs();
        match (self, lang) {
            (Problem::Invalid(detail), Lang::Zh) => format!("任务请求无效：{detail}"),
            (Problem::Invalid(detail), Lang::En) => format!("The task request is invalid: {detail}"),
            (Problem::IdReused, Lang::Zh) => "该操作 ID 已用于不同内容。".into(),
            (Problem::IdReused, Lang::En) => {
                "This operation ID was already used for different content.".into()
            }
            (Problem::Unavailable(reason), Lang::Zh) => {
                format!("无法用 Antigravity App 创建任务：{}。", reason.text(lang))
            }
            (Problem::Unavailable(reason), Lang::En) => {
                format!("Antigravity's App cannot create the task: {}.", reason.text(lang))
            }
            (Problem::TooManySidecars, Lang::Zh) => format!(
                "AskHuman 管理的 Antigravity sidecar 已达上限（{} 个，每个项目目录一个）。请在设置里移除不用的，再重试。",
                sidecar::LIMIT
            ),
            (Problem::TooManySidecars, Lang::En) => format!(
                "AskHuman already manages as many Antigravity sidecars as it may ({}, one per project folder). Remove some in Settings and try again.",
                sidecar::LIMIT
            ),
            (Problem::Conflict(reason), Lang::Zh) => format!(
                "Antigravity 的 sidecar 目录里已有一个不是 AskHuman 创建的同名项目，AskHuman 不会改动它（{reason}）。"
            ),
            (Problem::Conflict(reason), Lang::En) => format!(
                "Antigravity's sidecar folder already has an entry of that name that AskHuman did not make, so AskHuman leaves it alone ({reason})."
            ),
            (Problem::Files(detail), Lang::Zh) => {
                format!("无法读写 Antigravity 的 sidecar 配置：{detail}")
            }
            (Problem::Files(detail), Lang::En) => {
                format!("Could not read or write Antigravity's sidecar configuration: {detail}")
            }
            (Problem::AppNotStarted(detail), Lang::Zh) => {
                format!("无法启动 Antigravity App：{detail}")
            }
            (Problem::AppNotStarted(detail), Lang::En) => {
                format!("Could not start the Antigravity App: {detail}")
            }
            (Problem::NotOnline { started: true }, Lang::Zh) => format!(
                "已启动 Antigravity App，但 {seconds} 秒内它没有启动这个目录的 sidecar，没有创建任务。请打开 App 确认已登录、AskHuman 插件没有停用，稍后重试。"
            ),
            (Problem::NotOnline { started: true }, Lang::En) => format!(
                "The Antigravity App was started, but it did not start this folder's sidecar within {seconds} seconds; no task was created. Open the App, check that you are signed in and the AskHuman plugin is on, and retry."
            ),
            (Problem::NotOnline { started: false }, Lang::Zh) => format!(
                "Antigravity App 在 {seconds} 秒内没有启动这个目录的 sidecar，没有创建任务。App 读取新配置需要几秒钟；请确认 App 在运行、AskHuman 插件没有停用，稍后重试。"
            ),
            (Problem::NotOnline { started: false }, Lang::En) => format!(
                "The Antigravity App did not start this folder's sidecar within {seconds} seconds; no task was created. The App needs a few seconds to read a new sidecar; check that it is running and the AskHuman plugin is on, and retry."
            ),
            (Problem::NoAgentapi, Lang::Zh) => {
                "sidecar 在它的环境里找不到 agentapi（需要较新的 Antigravity App），没有创建任务。".into()
            }
            (Problem::NoAgentapi, Lang::En) => {
                "The sidecar cannot find agentapi in its environment (a newer Antigravity App is needed); no task was created.".into()
            }
            (Problem::NotSent(detail), Lang::Zh) => {
                format!("没有创建任务：{detail}。可以直接重试。")
            }
            (Problem::NotSent(detail), Lang::En) => {
                format!("No task was created: {detail}. You can retry.")
            }
            (Problem::Rejected(detail), Lang::Zh) => {
                format!("Antigravity 拒绝创建任务（没有向模型发送任何内容）：{detail}")
            }
            (Problem::Rejected(detail), Lang::En) => {
                format!("Antigravity refused to create the task (nothing was sent to a model): {detail}")
            }
            (Problem::Unclear { directory, detail }, Lang::Zh) => format!(
                "无法确认任务是否已在 Antigravity 里创建。请先在 App 的会话列表里查看项目 {directory}，确认后再决定是否重新创建。{}",
                detail_suffix(detail.as_deref(), lang)
            ),
            (Problem::Unclear { directory, detail }, Lang::En) => format!(
                "Could not confirm whether the task was created in Antigravity. Look for it in the App's conversation list under {directory} before creating it again.{}",
                detail_suffix(detail.as_deref(), lang)
            ),
        }
    }
}

fn detail_suffix(detail: Option<&str>, lang: Lang) -> String {
    match (detail.filter(|detail| !detail.is_empty()), lang) {
        (Some(detail), Lang::Zh) => format!("（{detail}）"),
        (Some(detail), Lang::En) => format!(" ({detail})"),
        (None, _) => String::new(),
    }
}

/// What failed in AskHuman's own sidecar files, as the user can act on it.
pub(super) fn problem_of(error: &anyhow::Error) -> Problem {
    if error.downcast_ref::<sidecar::LimitReached>().is_some() {
        Problem::TooManySidecars
    } else if let Some(conflict) = error.downcast_ref::<sidecar::Conflict>() {
        Problem::Conflict(conflict.0.clone())
    } else {
        Problem::Files(format!("{error:#}"))
    }
}

// MARK: - The receipt

/// What the receipt of one operation remembers. `Prepared` is written before anything is
/// changed; from `Sent` on the conversation may exist, so only `Prepared` and `Failed` (nothing
/// was created) allow the same operation to run again.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Stage {
    Prepared,
    Sent,
    Created,
    DryRun,
    Failed,
    Unknown,
}

impl Stage {
    fn name(self) -> &'static str {
        match self {
            Stage::Prepared => "prepared",
            Stage::Sent => "sent",
            Stage::Created => "created",
            Stage::DryRun => "dryRun",
            Stage::Failed => "failed",
            Stage::Unknown => "unknown",
        }
    }

    fn parse(name: &str) -> Self {
        match name {
            "prepared" => Stage::Prepared,
            "created" => Stage::Created,
            "dryRun" => Stage::DryRun,
            "failed" => Stage::Failed,
            // `sent`, `unknown` and anything unreadable: the conversation may exist.
            _ => Stage::Unknown,
        }
    }
}

struct Receipt {
    path: PathBuf,
    value: Value,
}

impl Receipt {
    /// The receipt at `path`, or the blank one of a new operation. It keeps a fingerprint of the
    /// request, not the task text, which stays out of the file.
    fn load(path: PathBuf, fingerprint: &str) -> Result<Self, Problem> {
        let value = match std::fs::read(&path) {
            Ok(bytes) => serde_json::from_slice::<Value>(&bytes).map_err(|_| {
                Problem::Files("The saved record of this operation cannot be read".into())
            })?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                json!({"fingerprint": fingerprint, "status": Stage::Prepared.name()})
            }
            Err(error) => {
                return Err(Problem::Files(format!(
                    "Could not read the record of this operation: {error}"
                )))
            }
        };
        if value["fingerprint"] != json!(fingerprint) {
            return Err(Problem::IdReused);
        }
        Ok(Self { path, value })
    }

    fn stage(&self) -> Stage {
        Stage::parse(self.value["status"].as_str().unwrap_or_default())
    }

    fn set(&mut self, stage: Stage) -> Result<(), Problem> {
        self.value["status"] = json!(stage.name());
        self.value["updatedAt"] = json!(super::now_ms());
        self.save()
    }

    fn save(&self) -> Result<(), Problem> {
        let files = |error: String| {
            Problem::Files(format!(
                "Could not save the record of this operation: {error}"
            ))
        };
        let dir = self
            .path
            .parent()
            .ok_or_else(|| files("Invalid receipt path".into()))?;
        std::fs::create_dir_all(dir).map_err(|error| files(error.to_string()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
        }
        let mut file =
            tempfile::NamedTempFile::new_in(dir).map_err(|error| files(error.to_string()))?;
        serde_json::to_writer(&mut file, &self.value).map_err(|error| files(error.to_string()))?;
        std::io::Write::flush(&mut file).map_err(|error| files(error.to_string()))?;
        file.persist(&self.path)
            .map_err(|error| files(error.error.to_string()))?;
        Ok(())
    }
}

/// Removes the receipts nobody can still retry from. Best effort.
fn prune(dir: &Path, keep: Duration) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let named_like_one = path.extension().is_some_and(|ext| ext == "json")
            && path
                .file_stem()
                .and_then(|stem| stem.to_str())
                .is_some_and(|stem| uuid::Uuid::parse_str(stem).is_ok());
        if !named_like_one {
            continue;
        }
        let old = entry
            .metadata()
            .and_then(|meta| meta.modified())
            .ok()
            .and_then(|modified| modified.elapsed().ok())
            .is_some_and(|age| age > keep);
        if old {
            let _ = std::fs::remove_file(&path);
        }
    }
}

// MARK: - Checking the request

struct Valid {
    /// The canonical project folder.
    directory: PathBuf,
    title: Option<String>,
    fingerprint: String,
}

/// The first line of the task, tidied, as the conversation's title.
fn derive_title(text: &str) -> Option<String> {
    let line = text.lines().map(str::trim).find(|line| !line.is_empty())?;
    // A tab or another control character separates words as a space does; none stays.
    let tidy: String = line
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if tidy.is_empty() {
        return None;
    }
    if tidy.chars().count() <= TITLE_CHARS {
        return Some(tidy);
    }
    let cut: String = tidy.chars().take(TITLE_CHARS - 1).collect();
    Some(format!("{}…", cut.trim_end()))
}

fn validate(request: &Request) -> Result<Valid, Problem> {
    let invalid = |detail: &str| Problem::Invalid(detail.to_string());
    // One spelling of the id, so one operation is one file even on a case-insensitive disk.
    let id = uuid::Uuid::parse_str(&request.id).map_err(|_| invalid("Invalid operation ID"))?;
    if id.hyphenated().to_string() != request.id {
        return Err(invalid("Invalid operation ID"));
    }
    match request.permission.as_str() {
        "agent-default" => {}
        // Not started yet by either route (docs/specs/antigravity-deep-integration.md §12.4).
        "yolo" => return Err(Problem::Unavailable(UnavailableReason::YoloUnsupported)),
        _ => return Err(invalid("Unknown permission mode")),
    }
    let given = Path::new(&request.cwd);
    if !given.is_absolute() {
        return Err(invalid("The project folder must be an absolute path"));
    }
    let directory =
        std::fs::canonicalize(given).map_err(|_| invalid("The project folder does not exist"))?;
    if !directory.is_dir() {
        return Err(invalid("The project folder is not a folder"));
    }
    let title = request
        .title
        .as_deref()
        .map(str::trim)
        .filter(|title| !title.is_empty())
        .map(str::to_string)
        .or_else(|| derive_title(&request.text));
    // What the sidecar would be asked to run; it refuses what cannot be an argument.
    Command::NewConversation {
        prompt: request.text.clone(),
        title: title.clone(),
        model: request.model.clone(),
    }
    .argv()
    .map_err(Problem::Invalid)?;
    let fingerprint = format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(request).unwrap_or_default())
    );
    Ok(Valid {
        directory,
        title,
        fingerprint,
    })
}

// MARK: - The operation

/// What a finished operation leaves for the caller.
#[derive(Debug)]
pub(super) struct Created {
    /// `{"status": "started" | "dryRun", "sessionId", "sidecarId", …}`. Nothing of the task's
    /// own text is in it, so the receipt can hold it.
    pub answer: Value,
}

/// What the operation needs from the world, so tests can stand in for the App.
pub(super) trait Backend {
    /// Why the App route cannot be used now, if so.
    async fn unavailable(&self) -> Option<UnavailableReason>;
    /// Makes the folder's sidecar exist and switches it on; returns its id.
    async fn ensure_sidecar(&self, directory: &Path) -> Result<(String, Change), Problem>;
    /// Whether the App is running; `None` when that cannot be told.
    async fn app_running(&self) -> Option<bool>;
    /// Starts the App without bringing it to the front.
    async fn start_app(&self) -> Result<(), String>;
    /// Waits until the sidecar has checked in, for at most `budget`.
    async fn wait_online(&self, id: &str, budget: Duration) -> bool;
    /// What the connected sidecar says about itself.
    async fn link(&self, id: &str) -> Option<LinkInfo>;
    async fn run(&self, id: &str, command: Command) -> Result<Outcome, RunError>;
}

/// The words of a command's end, short enough for a receipt: why it did not run, else what it
/// printed on stderr, else on stdout.
fn said(outcome: &Outcome) -> String {
    let words = match &outcome.error {
        Some(error) => error.as_str(),
        None if !outcome.stderr.trim().is_empty() => outcome.stderr.trim(),
        None => outcome.stdout.trim(),
    };
    words.chars().take(200).collect()
}

fn unclear(request: &Request, detail: Option<String>) -> Problem {
    Problem::Unclear {
        directory: request.cwd.clone(),
        detail,
    }
}

/// Runs one creation. `receipts` is where the operation's record lives.
pub(super) async fn execute_with<B: Backend>(
    backend: &B,
    receipts: &Path,
    request: Request,
) -> Result<Created, Problem> {
    let valid = validate(&request)?;
    prune(receipts, RECEIPT_KEEP);
    let mut receipt = Receipt::load(
        receipts.join(format!("{}.json", request.id)),
        &valid.fingerprint,
    )?;
    match receipt.stage() {
        Stage::Created | Stage::DryRun => {
            return Ok(Created {
                answer: receipt.value["answer"].clone(),
            })
        }
        Stage::Sent | Stage::Unknown => return Err(unclear(&request, None)),
        Stage::Prepared | Stage::Failed => {}
    }
    if let Some(reason) = backend.unavailable().await {
        return Err(Problem::Unavailable(reason));
    }
    receipt.value["failure"] = Value::Null;
    receipt.set(Stage::Prepared)?;
    let started = Instant::now();
    let (sidecar_id, change) = backend.ensure_sidecar(&valid.directory).await?;
    let mut app_started = false;
    if backend.app_running().await == Some(false) {
        backend.start_app().await.map_err(Problem::AppNotStarted)?;
        app_started = true;
    }
    let remaining = ONLINE_BUDGET.saturating_sub(started.elapsed());
    if !backend.wait_online(&sidecar_id, remaining).await {
        return Err(Problem::NotOnline {
            started: app_started,
        });
    }
    let Some(link) = backend.link(&sidecar_id).await else {
        // It went away again between the check-in and now.
        return Err(Problem::NotOnline {
            started: app_started,
        });
    };
    if !link.agentapi {
        return Err(Problem::NoAgentapi);
    }
    let change_name = match change {
        Change::Created => "created",
        Change::Updated => "updated",
        Change::Unchanged => "unchanged",
    };
    if request.dry_run {
        let outcome = match backend.run(&sidecar_id, Command::Probe).await {
            Ok(outcome) => outcome,
            Err(error) => return Err(Problem::NotSent(error.to_string())),
        };
        let answer = json!({
            "status": "dryRun",
            "dryRun": true,
            "sidecarId": sidecar_id,
            "sidecar": change_name,
            "appStarted": app_started,
            "agentapi": {
                "ran": outcome.error.is_none() && outcome.status.is_some(),
                "exitStatus": outcome.status,
                "said": said(&outcome),
            },
        });
        receipt.value["answer"] = answer.clone();
        receipt.set(Stage::DryRun)?;
        return Ok(Created { answer });
    }
    receipt.value["sidecarId"] = json!(sidecar_id);
    receipt.set(Stage::Sent)?;
    let command = Command::NewConversation {
        prompt: request.text.clone(),
        title: valid.title.clone(),
        model: request.model.clone(),
    };
    match backend.run(&sidecar_id, command).await {
        Err(RunError::NotSent(detail)) => {
            receipt.value["failure"] = json!({"reason": "notSent", "detail": detail});
            receipt.set(Stage::Failed)?;
            Err(Problem::NotSent(detail))
        }
        Err(RunError::Unknown(detail)) => {
            receipt.value["failure"] = json!({"reason": "unknown", "detail": detail});
            receipt.set(Stage::Unknown)?;
            Err(unclear(&request, Some(detail)))
        }
        Ok(outcome) => match link::conversation_id(&outcome) {
            Ok(conversation) => {
                let answer = json!({
                    "status": "started",
                    "dryRun": false,
                    "sessionId": conversation,
                    "sidecarId": sidecar_id,
                    "sidecar": change_name,
                    "appStarted": app_started,
                });
                receipt.value["answer"] = answer.clone();
                receipt.set(Stage::Created)?;
                Ok(Created { answer })
            }
            Err(detail) => {
                // `agentapi` that exits with an error made no conversation. Anything else (it
                // was killed, or it succeeded and said something unreadable) may have.
                let refused =
                    outcome.error.is_none() && outcome.status.is_some_and(|status| status != 0);
                receipt.value["failure"] = json!({
                    "reason": if refused { "agentapi" } else { "unknown" },
                    "detail": detail,
                });
                if refused {
                    receipt.set(Stage::Failed)?;
                    Err(Problem::Rejected(detail))
                } else {
                    receipt.set(Stage::Unknown)?;
                    Err(unclear(&request, Some(detail)))
                }
            }
        },
    }
}

// MARK: - The real thing

struct Live;

impl Backend for Live {
    async fn unavailable(&self) -> Option<UnavailableReason> {
        let app = crate::config::AppConfig::load_without_secrets();
        let yolo = app.agent_tasks.permission_prompt == crate::config::AgentTaskPermission::Yolo;
        launch::desktop_reason(launch::static_facts(&app.antigravity_desktop, yolo))
    }

    async fn ensure_sidecar(&self, directory: &Path) -> Result<(String, Change), Problem> {
        let directory = directory.to_path_buf();
        tokio::task::spawn_blocking(move || {
            let executable = super::executable().map_err(|error| problem_of(&error))?;
            sidecar::ensure_workspace(
                &sidecar::Layout::current(),
                &executable,
                &directory,
                &sidecar::attachment_roots(),
                super::now_ms(),
            )
            .map_err(|error| problem_of(&error))
        })
        .await
        .map_err(|error| Problem::Files(error.to_string()))?
    }

    async fn app_running(&self) -> Option<bool> {
        tokio::task::spawn_blocking(super::runtime::app_running)
            .await
            .ok()
            .flatten()
    }

    async fn start_app(&self) -> Result<(), String> {
        tokio::task::spawn_blocking(super::runtime::start_app)
            .await
            .map_err(|error| error.to_string())?
            .map_err(|error| format!("{error:#}"))
    }

    async fn wait_online(&self, id: &str, budget: Duration) -> bool {
        super::manager().wait_online(id, budget).await
    }

    async fn link(&self, id: &str) -> Option<LinkInfo> {
        super::manager().online().remove(id)
    }

    async fn run(&self, id: &str, command: Command) -> Result<Outcome, RunError> {
        super::manager()
            .run(id, command, link::COMMAND_TIMEOUT)
            .await
    }
}

/// The daemon's entry: creates the task, or says in words why not.
pub async fn execute(request: Request) -> Result<Value, String> {
    let dry_run = request.dry_run;
    match execute_with(&Live, &super::receipt::dir(), request).await {
        Ok(created) => {
            crate::daemon::lifecycle::log_runtime_event(
                "antigravity_desktop",
                if dry_run {
                    "create_dry_run"
                } else {
                    "create_started"
                },
                created.answer["sessionId"].as_str(),
            );
            Ok(created.answer)
        }
        Err(problem) => {
            crate::daemon::lifecycle::log_runtime_event(
                "antigravity_desktop",
                &format!("create_{}", problem.code()),
                None,
            );
            Err(problem.text(Lang::current()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::sync::Mutex;

    const ID: &str = "8d4c2c0a-1111-4222-8333-944455556666";
    const CONVERSATION: &str = "0b7f2c1e-aaaa-4bbb-8ccc-ddddeeeeffff";

    fn request(cwd: &Path) -> Request {
        Request {
            cwd: cwd.to_string_lossy().into_owned(),
            text: "Fix the failing test\nthen run the suite".into(),
            permission: "agent-default".into(),
            id: ID.into(),
            model: None,
            title: None,
            dry_run: false,
        }
    }

    fn created_outcome() -> Outcome {
        Outcome {
            status: Some(0),
            stdout: format!(
                "{{\"response\":{{\"newConversation\":{{\"conversationId\":\"{CONVERSATION}\"}}}}}}"
            ),
            ..Outcome::default()
        }
    }

    fn refused_outcome(said: &str) -> Outcome {
        Outcome {
            status: Some(1),
            stderr: said.into(),
            ..Outcome::default()
        }
    }

    struct Fake {
        unavailable: Option<UnavailableReason>,
        ensure: Mutex<VecDeque<Result<(String, Change), Problem>>>,
        app_running: Option<bool>,
        start_error: Option<String>,
        online: Mutex<VecDeque<bool>>,
        agentapi: bool,
        runs: Mutex<VecDeque<Result<Outcome, RunError>>>,
        calls: Mutex<Vec<String>>,
    }

    impl Fake {
        fn new() -> Self {
            Self {
                unavailable: None,
                ensure: Mutex::new(VecDeque::new()),
                app_running: Some(true),
                start_error: None,
                online: Mutex::new(VecDeque::new()),
                agentapi: true,
                runs: Mutex::new(VecDeque::new()),
                calls: Mutex::new(Vec::new()),
            }
        }

        fn runs(self, runs: Vec<Result<Outcome, RunError>>) -> Self {
            *self.runs.lock().unwrap() = runs.into();
            self
        }

        fn calls(&self) -> Vec<String> {
            self.calls.lock().unwrap().clone()
        }

        fn note(&self, call: &str) {
            self.calls.lock().unwrap().push(call.to_string());
        }
    }

    impl Backend for Fake {
        async fn unavailable(&self) -> Option<UnavailableReason> {
            self.note("unavailable");
            self.unavailable
        }

        async fn ensure_sidecar(&self, _: &Path) -> Result<(String, Change), Problem> {
            self.note("ensure");
            self.ensure
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or_else(|| Ok(("askhuman-ws-12345678".into(), Change::Unchanged)))
        }

        async fn app_running(&self) -> Option<bool> {
            self.note("app_running");
            self.app_running
        }

        async fn start_app(&self) -> Result<(), String> {
            self.note("start_app");
            self.start_error.clone().map_or(Ok(()), Err)
        }

        async fn wait_online(&self, _: &str, _: Duration) -> bool {
            self.note("wait_online");
            self.online.lock().unwrap().pop_front().unwrap_or(true)
        }

        async fn link(&self, _: &str) -> Option<LinkInfo> {
            self.note("link");
            Some(LinkInfo {
                project_id: None,
                version: "1.0.0".into(),
                agentapi: self.agentapi,
                pid: 1,
                connected_at: 0,
            })
        }

        async fn run(&self, _: &str, command: Command) -> Result<Outcome, RunError> {
            self.note(match command {
                Command::NewConversation { .. } => "run:new",
                Command::Probe => "run:probe",
            });
            self.runs
                .lock()
                .unwrap()
                .pop_front()
                .expect("a command nobody scripted")
        }
    }

    fn receipt_of(receipts: &Path) -> Value {
        serde_json::from_slice(&std::fs::read(receipts.join(format!("{ID}.json"))).unwrap())
            .unwrap()
    }

    async fn run(fake: &Fake, receipts: &Path, request: Request) -> Result<Created, Problem> {
        execute_with(fake, receipts, request).await
    }

    #[tokio::test]
    async fn a_created_task_is_remembered_and_a_retry_answers_from_the_receipt() {
        let folder = tempfile::tempdir().unwrap();
        let receipts = tempfile::tempdir().unwrap();
        let fake = Fake::new().runs(vec![Ok(created_outcome())]);
        let created = run(&fake, receipts.path(), request(folder.path()))
            .await
            .unwrap();
        assert_eq!(created.answer["status"], "started");
        assert_eq!(created.answer["sessionId"], CONVERSATION);
        assert_eq!(created.answer["sidecarId"], "askhuman-ws-12345678");
        assert!(created.answer.get("title").is_none());
        assert_eq!(receipt_of(receipts.path())["status"], "created");

        let calls_before = fake.calls().len();
        let again = run(&fake, receipts.path(), request(folder.path()))
            .await
            .unwrap();
        assert_eq!(again.answer, created.answer);
        assert_eq!(fake.calls().len(), calls_before, "a retry touches nothing");
    }

    #[tokio::test]
    async fn the_task_text_stays_out_of_the_receipt() {
        let folder = tempfile::tempdir().unwrap();
        let receipts = tempfile::tempdir().unwrap();
        let mut secret = request(folder.path());
        secret.text = "rotate the key hunter2-and-friends".into();
        let fake = Fake::new().runs(vec![Ok(created_outcome())]);
        run(&fake, receipts.path(), secret).await.unwrap();
        let bytes = std::fs::read(receipts.path().join(format!("{ID}.json"))).unwrap();
        assert!(!String::from_utf8_lossy(&bytes).contains("hunter2"));
    }

    #[tokio::test]
    async fn the_command_carries_the_task_the_title_and_the_model() {
        let folder = tempfile::tempdir().unwrap();
        let receipts = tempfile::tempdir().unwrap();
        struct Spy(Mutex<Option<Command>>);
        impl Backend for Spy {
            async fn unavailable(&self) -> Option<UnavailableReason> {
                None
            }
            async fn ensure_sidecar(&self, _: &Path) -> Result<(String, Change), Problem> {
                Ok(("askhuman-ws-12345678".into(), Change::Created))
            }
            async fn app_running(&self) -> Option<bool> {
                Some(true)
            }
            async fn start_app(&self) -> Result<(), String> {
                Ok(())
            }
            async fn wait_online(&self, _: &str, _: Duration) -> bool {
                true
            }
            async fn link(&self, _: &str) -> Option<LinkInfo> {
                Some(LinkInfo {
                    project_id: None,
                    version: String::new(),
                    agentapi: true,
                    pid: 1,
                    connected_at: 0,
                })
            }
            async fn run(&self, _: &str, command: Command) -> Result<Outcome, RunError> {
                *self.0.lock().unwrap() = Some(command);
                Ok(created_outcome())
            }
        }
        let spy = Spy(Mutex::new(None));
        let mut asked = request(folder.path());
        asked.model = Some("gemini-3.5-flash-low".into());
        asked.title = Some("  Custom title ".into());
        execute_with(&spy, receipts.path(), asked).await.unwrap();
        assert_eq!(
            spy.0.lock().unwrap().clone(),
            Some(Command::NewConversation {
                prompt: "Fix the failing test\nthen run the suite".into(),
                title: Some("Custom title".into()),
                model: Some("gemini-3.5-flash-low".into()),
            })
        );
    }

    #[tokio::test]
    async fn an_id_cannot_be_reused_for_different_content() {
        let folder = tempfile::tempdir().unwrap();
        let receipts = tempfile::tempdir().unwrap();
        let fake = Fake::new().runs(vec![Ok(created_outcome())]);
        run(&fake, receipts.path(), request(folder.path()))
            .await
            .unwrap();
        let mut other = request(folder.path());
        other.text = "something else".into();
        let error = run(&fake, receipts.path(), other).await.unwrap_err();
        assert_eq!(error, Problem::IdReused);
    }

    #[tokio::test]
    async fn an_unavailable_route_is_reported_before_anything_exists() {
        let folder = tempfile::tempdir().unwrap();
        let receipts = tempfile::tempdir().unwrap();
        let mut fake = Fake::new();
        fake.unavailable = Some(UnavailableReason::SidecarsNotAllowed);
        let error = run(&fake, receipts.path(), request(folder.path()))
            .await
            .unwrap_err();
        assert_eq!(
            error,
            Problem::Unavailable(UnavailableReason::SidecarsNotAllowed)
        );
        assert_eq!(fake.calls(), vec!["unavailable"]);
        assert!(!receipts.path().join(format!("{ID}.json")).exists());
    }

    #[tokio::test]
    async fn a_command_that_was_not_sent_can_be_tried_again() {
        let folder = tempfile::tempdir().unwrap();
        let receipts = tempfile::tempdir().unwrap();
        let fake = Fake::new().runs(vec![
            Err(RunError::NotSent("The sidecar is not connected".into())),
            Ok(created_outcome()),
        ]);
        let error = run(&fake, receipts.path(), request(folder.path()))
            .await
            .unwrap_err();
        assert!(matches!(error, Problem::NotSent(_)));
        assert_eq!(receipt_of(receipts.path())["status"], "failed");
        let created = run(&fake, receipts.path(), request(folder.path()))
            .await
            .unwrap();
        assert_eq!(created.answer["sessionId"], CONVERSATION);
        assert_eq!(receipt_of(receipts.path())["status"], "created");
        assert_eq!(
            receipt_of(receipts.path())["failure"],
            Value::Null,
            "a later success clears the old failure"
        );
    }

    #[tokio::test]
    async fn a_command_that_may_have_run_is_never_repeated() {
        let folder = tempfile::tempdir().unwrap();
        let receipts = tempfile::tempdir().unwrap();
        let fake = Fake::new().runs(vec![Err(RunError::Unknown(
            "The sidecar did not answer within 45 seconds".into(),
        ))]);
        let error = run(&fake, receipts.path(), request(folder.path()))
            .await
            .unwrap_err();
        assert!(matches!(error, Problem::Unclear { .. }));
        assert_eq!(receipt_of(receipts.path())["status"], "unknown");
        let before = fake.calls().len();
        let again = run(&fake, receipts.path(), request(folder.path()))
            .await
            .unwrap_err();
        assert!(matches!(again, Problem::Unclear { .. }));
        assert_eq!(fake.calls().len(), before);
    }

    #[tokio::test]
    async fn an_interrupted_operation_is_unclear_not_retried() {
        let folder = tempfile::tempdir().unwrap();
        let receipts = tempfile::tempdir().unwrap();
        // The daemon went away after writing `sent`.
        let valid = validate(&request(folder.path())).unwrap();
        let mut receipt = Receipt::load(
            receipts.path().join(format!("{ID}.json")),
            &valid.fingerprint,
        )
        .unwrap();
        receipt.set(Stage::Sent).unwrap();
        let fake = Fake::new();
        let error = run(&fake, receipts.path(), request(folder.path()))
            .await
            .unwrap_err();
        assert!(matches!(error, Problem::Unclear { .. }));
        assert!(fake.calls().is_empty());
    }

    #[tokio::test]
    async fn agentapi_refusing_means_nothing_was_created_and_allows_a_retry() {
        let folder = tempfile::tempdir().unwrap();
        let receipts = tempfile::tempdir().unwrap();
        let fake = Fake::new().runs(vec![
            Ok(refused_outcome("model gemini-9 is not available")),
            Ok(created_outcome()),
        ]);
        let error = run(&fake, receipts.path(), request(folder.path()))
            .await
            .unwrap_err();
        let Problem::Rejected(detail) = &error else {
            panic!("expected a refusal, got {error:?}");
        };
        assert!(detail.contains("not available"), "{detail}");
        assert_eq!(receipt_of(receipts.path())["status"], "failed");
        run(&fake, receipts.path(), request(folder.path()))
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn an_answer_nobody_can_read_may_hide_a_conversation() {
        let folder = tempfile::tempdir().unwrap();
        for outcome in [
            // It succeeded and said something else.
            Outcome {
                status: Some(0),
                stdout: "created!".into(),
                ..Outcome::default()
            },
            // It was killed.
            Outcome::failed_to_run("The command did not finish in 45 seconds"),
            // It ended without an exit status.
            Outcome::default(),
        ] {
            let receipts = tempfile::tempdir().unwrap();
            let fake = Fake::new().runs(vec![Ok(outcome)]);
            let error = run(&fake, receipts.path(), request(folder.path()))
                .await
                .unwrap_err();
            assert!(matches!(error, Problem::Unclear { .. }), "{error:?}");
            assert_eq!(receipt_of(receipts.path())["status"], "unknown");
        }
    }

    #[tokio::test]
    async fn a_dry_run_asks_agentapi_something_harmless_and_creates_nothing() {
        let folder = tempfile::tempdir().unwrap();
        let receipts = tempfile::tempdir().unwrap();
        let fake = Fake::new().runs(vec![Ok(refused_outcome("conversation not found"))]);
        let mut dry = request(folder.path());
        dry.dry_run = true;
        let created = run(&fake, receipts.path(), dry.clone()).await.unwrap();
        assert_eq!(created.answer["status"], "dryRun");
        assert_eq!(created.answer["agentapi"]["ran"], true);
        assert_eq!(created.answer["agentapi"]["exitStatus"], 1);
        assert!(fake.calls().contains(&"run:probe".to_string()));
        assert!(!fake.calls().contains(&"run:new".to_string()));
        assert_eq!(receipt_of(receipts.path())["status"], "dryRun");
        // The same dry run answers from its receipt.
        let before = fake.calls().len();
        let again = run(&fake, receipts.path(), dry).await.unwrap();
        assert_eq!(again.answer, created.answer);
        assert_eq!(fake.calls().len(), before);
    }

    #[tokio::test]
    async fn a_dry_run_that_could_not_ask_says_so_and_can_be_repeated() {
        let folder = tempfile::tempdir().unwrap();
        let receipts = tempfile::tempdir().unwrap();
        let fake = Fake::new().runs(vec![
            Err(RunError::Unknown("The sidecar disconnected".into())),
            Ok(Outcome {
                status: Some(0),
                ..Outcome::default()
            }),
        ]);
        let mut dry = request(folder.path());
        dry.dry_run = true;
        let error = run(&fake, receipts.path(), dry.clone()).await.unwrap_err();
        assert!(matches!(error, Problem::NotSent(_)), "{error:?}");
        assert_eq!(receipt_of(receipts.path())["status"], "prepared");
        assert_eq!(
            run(&fake, receipts.path(), dry).await.unwrap().answer["status"],
            "dryRun"
        );
    }

    #[tokio::test]
    async fn the_app_is_started_only_when_it_is_known_not_to_run() {
        for (running, starts) in [(Some(false), true), (Some(true), false), (None, false)] {
            let folder = tempfile::tempdir().unwrap();
            let receipts = tempfile::tempdir().unwrap();
            let mut fake = Fake::new().runs(vec![Ok(created_outcome())]);
            fake.app_running = running;
            let created = run(&fake, receipts.path(), request(folder.path()))
                .await
                .unwrap();
            assert_eq!(
                fake.calls().contains(&"start_app".to_string()),
                starts,
                "{running:?}"
            );
            assert_eq!(created.answer["appStarted"], starts);
        }
    }

    #[tokio::test]
    async fn an_app_that_will_not_start_or_a_sidecar_that_never_comes_is_reported() {
        let folder = tempfile::tempdir().unwrap();
        let receipts = tempfile::tempdir().unwrap();
        let mut fake = Fake::new();
        fake.app_running = Some(false);
        fake.start_error = Some("Could not run open".into());
        let error = run(&fake, receipts.path(), request(folder.path()))
            .await
            .unwrap_err();
        assert_eq!(error, Problem::AppNotStarted("Could not run open".into()));

        fake.start_error = None;
        *fake.online.lock().unwrap() = VecDeque::from([false]);
        let error = run(&fake, receipts.path(), request(folder.path()))
            .await
            .unwrap_err();
        assert_eq!(error, Problem::NotOnline { started: true });
        // Nothing was sent, so the same operation can go again once the sidecar is there.
        assert_eq!(receipt_of(receipts.path())["status"], "prepared");
        fake.app_running = Some(true);
        *fake.runs.lock().unwrap() = VecDeque::from([Ok(created_outcome())]);
        assert!(run(&fake, receipts.path(), request(folder.path()))
            .await
            .is_ok());
    }

    #[tokio::test]
    async fn the_wait_gets_what_is_left_of_the_budget() {
        let folder = tempfile::tempdir().unwrap();
        let receipts = tempfile::tempdir().unwrap();
        struct Budget(Mutex<Option<Duration>>);
        impl Backend for Budget {
            async fn unavailable(&self) -> Option<UnavailableReason> {
                None
            }
            async fn ensure_sidecar(&self, _: &Path) -> Result<(String, Change), Problem> {
                Ok(("askhuman-ws-12345678".into(), Change::Created))
            }
            async fn app_running(&self) -> Option<bool> {
                Some(true)
            }
            async fn start_app(&self) -> Result<(), String> {
                Ok(())
            }
            async fn wait_online(&self, _: &str, budget: Duration) -> bool {
                *self.0.lock().unwrap() = Some(budget);
                false
            }
            async fn link(&self, _: &str) -> Option<LinkInfo> {
                None
            }
            async fn run(&self, _: &str, _: Command) -> Result<Outcome, RunError> {
                unreachable!()
            }
        }
        let backend = Budget(Mutex::new(None));
        let error = execute_with(&backend, receipts.path(), request(folder.path()))
            .await
            .unwrap_err();
        assert_eq!(error, Problem::NotOnline { started: false });
        let waited = backend.0.lock().unwrap().unwrap();
        assert!(waited <= ONLINE_BUDGET && waited > ONLINE_BUDGET - Duration::from_secs(5));
    }

    #[tokio::test]
    async fn a_sidecar_that_cannot_find_agentapi_is_reported_before_sending() {
        let folder = tempfile::tempdir().unwrap();
        let receipts = tempfile::tempdir().unwrap();
        let mut fake = Fake::new();
        fake.agentapi = false;
        let error = run(&fake, receipts.path(), request(folder.path()))
            .await
            .unwrap_err();
        assert_eq!(error, Problem::NoAgentapi);
        assert!(!fake.calls().contains(&"run:new".to_string()));
    }

    #[tokio::test]
    async fn a_sidecar_that_cannot_be_made_is_reported_with_the_reason() {
        let folder = tempfile::tempdir().unwrap();
        let receipts = tempfile::tempdir().unwrap();
        for problem in [
            Problem::TooManySidecars,
            Problem::Conflict("edited by hand".into()),
        ] {
            let fake = Fake::new();
            fake.ensure.lock().unwrap().push_back(Err(problem.clone()));
            let error = run(&fake, receipts.path(), request(folder.path()))
                .await
                .unwrap_err();
            assert_eq!(error, problem);
            assert_eq!(receipt_of(receipts.path())["status"], "prepared");
        }
    }

    #[tokio::test]
    async fn a_bad_request_is_refused_before_anything_happens() {
        let folder = tempfile::tempdir().unwrap();
        let receipts = tempfile::tempdir().unwrap();
        let file = folder.path().join("a-file");
        std::fs::write(&file, "x").unwrap();
        let file_cwd = file.to_string_lossy().into_owned();
        type Edit = Box<dyn Fn(&mut Request)>;
        let cases: Vec<(&str, Edit)> = vec![
            (
                "relative folder",
                Box::new(|r: &mut Request| r.cwd = "relative/dir".into()),
            ),
            (
                "missing folder",
                Box::new(|r: &mut Request| r.cwd = "/definitely/not/here".into()),
            ),
            (
                "a file",
                Box::new(move |r: &mut Request| r.cwd = file_cwd.clone()),
            ),
            (
                "not a uuid",
                Box::new(|r: &mut Request| r.id = "nope".into()),
            ),
            (
                "upper-case id",
                Box::new(|r: &mut Request| r.id = ID.to_uppercase()),
            ),
            (
                "empty text",
                Box::new(|r: &mut Request| r.text = "  \n ".into()),
            ),
            (
                "NUL in text",
                Box::new(|r: &mut Request| r.text = "a\0b".into()),
            ),
            (
                "model that is not an id",
                Box::new(|r: &mut Request| r.model = Some("--evil".into())),
            ),
            (
                "unknown permission",
                Box::new(|r: &mut Request| r.permission = "root".into()),
            ),
            (
                "control characters in the title",
                Box::new(|r: &mut Request| r.title = Some("a\nb".into())),
            ),
        ];
        for (name, change) in cases {
            let fake = Fake::new();
            let mut bad = request(folder.path());
            change(&mut bad);
            let error = run(&fake, receipts.path(), bad).await.unwrap_err();
            assert!(matches!(error, Problem::Invalid(_)), "{name}: {error:?}");
            assert!(fake.calls().is_empty(), "{name}");
        }
        assert!(!receipts.path().join(format!("{ID}.json")).exists());
    }

    #[tokio::test]
    async fn yolo_is_not_started_yet() {
        let folder = tempfile::tempdir().unwrap();
        let receipts = tempfile::tempdir().unwrap();
        let fake = Fake::new();
        let mut yolo = request(folder.path());
        yolo.permission = "yolo".into();
        let error = run(&fake, receipts.path(), yolo).await.unwrap_err();
        assert_eq!(
            error,
            Problem::Unavailable(UnavailableReason::YoloUnsupported)
        );
        assert!(fake.calls().is_empty());
    }

    #[test]
    fn a_title_is_the_first_line_tidied_and_short() {
        assert_eq!(
            derive_title("  \n  Fix   the\ttest \nsecond line").as_deref(),
            Some("Fix the test")
        );
        assert_eq!(derive_title("   \n\n"), None);
        assert_eq!(derive_title("\u{7}\u{8}"), None);
        let long = "word ".repeat(40);
        let title = derive_title(&long).unwrap();
        assert!(title.chars().count() <= TITLE_CHARS, "{title}");
        assert!(title.ends_with('…'));
        // What is cut is whole characters, not bytes.
        let wide = "任".repeat(100);
        let title = derive_title(&wide).unwrap();
        assert_eq!(title.chars().count(), TITLE_CHARS);
    }

    #[test]
    fn receipts_nobody_can_retry_from_are_pruned() {
        let dir = tempfile::tempdir().unwrap();
        let old = dir.path().join(format!("{ID}.json"));
        let young = dir.path().join("11111111-2222-4333-8444-555566667777.json");
        let stranger = dir.path().join("notes.json");
        for path in [&old, &young, &stranger] {
            std::fs::write(path, "{}").unwrap();
        }
        let long_ago = std::time::SystemTime::now() - Duration::from_secs(40 * 24 * 3600);
        std::fs::File::options()
            .write(true)
            .open(&old)
            .unwrap()
            .set_modified(long_ago)
            .unwrap();
        std::fs::File::options()
            .write(true)
            .open(&stranger)
            .unwrap()
            .set_modified(long_ago)
            .unwrap();
        prune(dir.path(), RECEIPT_KEEP);
        assert!(!old.exists());
        assert!(young.exists());
        assert!(stranger.exists(), "only receipts are touched");
    }

    #[test]
    fn every_problem_has_words_in_both_languages() {
        let problems = [
            Problem::Invalid("x".into()),
            Problem::IdReused,
            Problem::Unavailable(UnavailableReason::SidecarsNotAllowed),
            Problem::TooManySidecars,
            Problem::Conflict("edited".into()),
            Problem::Files("denied".into()),
            Problem::AppNotStarted("no".into()),
            Problem::NotOnline { started: true },
            Problem::NotOnline { started: false },
            Problem::NoAgentapi,
            Problem::NotSent("offline".into()),
            Problem::Rejected("bad model".into()),
            Problem::Unclear {
                directory: "/work/app".into(),
                detail: Some("timed out".into()),
            },
            Problem::Unclear {
                directory: "/work/app".into(),
                detail: None,
            },
        ];
        for problem in problems {
            for lang in [Lang::Zh, Lang::En] {
                let text = problem.text(lang);
                assert!(!text.is_empty(), "{problem:?}");
                assert!(!problem.code().is_empty());
            }
        }
        let unclear = Problem::Unclear {
            directory: "/work/app".into(),
            detail: Some("timed out".into()),
        };
        assert!(unclear.text(Lang::En).contains("/work/app"));
        assert!(unclear.text(Lang::En).contains("timed out"));
        assert!(Problem::Unavailable(UnavailableReason::SidecarsNotAllowed)
            .text(Lang::En)
            .contains("sidecars"));
    }

    #[test]
    fn sidecar_errors_become_what_the_user_can_act_on() {
        assert_eq!(
            problem_of(&sidecar::LimitReached.into()),
            Problem::TooManySidecars
        );
        assert_eq!(
            problem_of(&sidecar::Conflict("edited".into()).into()),
            Problem::Conflict("edited".into())
        );
        assert!(matches!(
            problem_of(&anyhow::anyhow!("disk full")),
            Problem::Files(text) if text.contains("disk full")
        ));
    }

    #[test]
    fn the_request_has_a_stable_wire_form() {
        let request: Request = serde_json::from_value(json!({
            "cwd": "/work/app",
            "text": "do it",
            "permission": "agent-default",
            "id": ID,
            "dryRun": true,
        }))
        .unwrap();
        assert!(request.dry_run);
        assert_eq!(request.model, None);
        assert_eq!(request.title, None);
        let back = serde_json::to_value(&request).unwrap();
        assert_eq!(back["dryRun"], true);
        assert!(back.get("model").is_none());
    }
}
