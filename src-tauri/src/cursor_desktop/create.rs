//! Creating a task in Cursor Desktop's Agents window (docs/specs/cursor-desktop-task-creation.md).
//!
//! The daemon decides everything that can be decided without Cursor (the operation's shape, the
//! Max and model plan, the receipt that makes a retry safe) and hands the rest to the Agents
//! window's own creation code through the message bridge. That code checks the chat it made
//! against this plan before a single token is sent, so the daemon never has to trust its own
//! assumptions about Cursor's internals; it only has to read the answer.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::agents::cursor_max::catalog::{variant_for, Catalog, Found};
use crate::agents::cursor_max::{self, Assessment, Inputs, Need, Outcome, Plan, Request, Target};
use crate::config::{CursorMaxConfig, CursorMaxPolicy};
use crate::i18n::Lang;
use crate::integrations::agent_launch::MaxRequired;

/// The longest task text the bridge carries (its request cap is 256 KiB).
const MAX_TEXT_BYTES: usize = 100_000;
const MAX_TITLE_CHARS: usize = 200;
/// How long Cursor may take to create the chat, as the renderer is told (it refuses to submit
/// after this) and, five seconds later, as the daemon stops waiting for an answer.
const RENDERER_WINDOW: Duration = Duration::from_secs(40);
const DAEMON_WINDOW: Duration = Duration::from_secs(45);
/// The pause between two asks while the renderer still answers `pending`.
const POLL_PAUSE: Duration = Duration::from_millis(250);

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum Operation {
    Create {
        cwd: String,
        text: String,
        /// `agent-default` or `yolo`.
        permission: String,
        /// Also the id of the chat Cursor creates, and the key of the receipt.
        id: String,
        /// The model picked for this task, as `--model` takes it; absent leaves it to Cursor.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        model: Option<String>,
        /// The user allowed Cursor's Max mode for this task.
        #[serde(default)]
        allow_max: bool,
        /// Seed for the chat's name until Cursor names it.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        title: Option<String>,
        /// Create the chat and check it, then discard it without sending anything.
        #[serde(default)]
        dry_run: bool,
    },
}

/// A model as Cursor's composer configuration names it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DesktopModel {
    pub model_name: String,
    pub parameters: Vec<Parameter>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Parameter {
    pub id: String,
    pub value: String,
}

/// What the Max settings and the model catalog make of one task.
#[derive(Clone, Debug)]
pub struct DesktopPlan {
    /// The explicit model for the chat; `None` keeps the model Cursor starts new chats with.
    pub model: Option<DesktopModel>,
    /// `Some(false)`: Max is forced off and the chat is checked for it; `Some(true)`: the user
    /// allowed Max for a model that cannot run without it; `None`: Max is left to Cursor.
    pub max_mode: Option<bool>,
    pub assessment: Assessment,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanError {
    /// The model cannot run without Max and this task does not allow it.
    MaxRequired(MaxRequired),
    /// The picked model is not in Cursor's own catalog, or there is no catalog to map it with.
    UnknownModel(String),
}

impl std::fmt::Display for PlanError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PlanError::MaxRequired(required) => required.fmt(f),
            PlanError::UnknownModel(id) => {
                write!(
                    f,
                    "Cursor does not list the model {id} for its Agents window"
                )
            }
        }
    }
}

fn map_model(catalog: Option<&Catalog>, id: &str) -> Result<DesktopModel, PlanError> {
    let unknown = || PlanError::UnknownModel(id.to_string());
    let (model, variant) = match catalog.ok_or_else(unknown)?.find_flat(id) {
        Found::Variant(model, variant) => (model, Some(variant)),
        // A model named without a variant starts as the IDE would start it with Max off.
        Found::Model(model) => (model, variant_for(model, &[])),
        Found::Missing => return Err(unknown()),
    };
    Ok(DesktopModel {
        model_name: model.name.clone(),
        parameters: variant
            .map(|variant| {
                variant
                    .params
                    .iter()
                    .map(|(id, value)| Parameter {
                        id: id.clone(),
                        value: value.clone(),
                    })
                    .collect()
            })
            .unwrap_or_default(),
    })
}

/// The pure plan (docs/specs/cursor-max-control.md applied to the Agents window).
pub fn plan_with(
    settings: &CursorMaxConfig,
    inputs: &Inputs,
    model: Option<&str>,
    allow_max: bool,
) -> Result<DesktopPlan, PlanError> {
    let request = Request {
        target: Target::Desktop,
        model,
        allow_max,
    };
    let assessment = cursor_max::assess_with(settings, &request, inputs);
    let in_force =
        assessment.plan == Plan::Legacy && settings.policy == CursorMaxPolicy::EnforceOff;
    let max_mode = match assessment.outcome {
        Outcome::Blocked => {
            return Err(PlanError::MaxRequired(MaxRequired {
                model: assessment.model.clone(),
                label: assessment.label.clone(),
            }))
        }
        Outcome::ForceOff { .. } => Some(false),
        // Max was allowed for a model that cannot run without it: say so as Cursor's own picker
        // would. Anything else stays Cursor's decision.
        Outcome::Untouched if in_force && allow_max && assessment.need == Need::Required => {
            Some(true)
        }
        Outcome::Untouched => None,
    };
    let model = model
        .map(|id| map_model(inputs.catalog.as_ref(), id))
        .transpose()?;
    Ok(DesktopPlan {
        model,
        max_mode,
        assessment,
    })
}

/// [`plan_with`] from the current settings and Cursor's local files.
pub fn plan(model: Option<&str>, allow_max: bool) -> Result<DesktopPlan, PlanError> {
    let settings = crate::config::AppConfig::load_without_secrets()
        .agent_tasks
        .cursor_max;
    plan_with(&settings, &cursor_max::load_inputs(), model, allow_max)
}

// ---- receipts ----

pub(crate) fn receipts_dir() -> PathBuf {
    crate::paths::state_dir().join("cursor-desktop-actions")
}

pub(crate) fn receipt_path(id: &str) -> PathBuf {
    receipts_dir().join(format!("{id}.json"))
}

/// What the receipt of one operation remembers. `Prepared` is written before the bridge is
/// contacted; from `Creating` on, Cursor may have created the chat, so only `Rejected` (Cursor
/// refused before sending anything) and `Prepared` allow the same operation to run again.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Stage {
    Prepared,
    Creating,
    Accepted,
    DryRun,
    Rejected,
    Unknown,
}

impl Stage {
    fn name(self) -> &'static str {
        match self {
            Stage::Prepared => "prepared",
            Stage::Creating => "creating",
            Stage::Accepted => "accepted",
            Stage::DryRun => "dryRun",
            Stage::Rejected => "rejected",
            Stage::Unknown => "unknown",
        }
    }

    fn parse(name: &str) -> Self {
        match name {
            "prepared" => Stage::Prepared,
            "accepted" => Stage::Accepted,
            "dryRun" => Stage::DryRun,
            "rejected" => Stage::Rejected,
            // `creating`, `unknown` and anything unreadable: Cursor may have received the task.
            _ => Stage::Unknown,
        }
    }
}

struct Receipt {
    path: PathBuf,
    value: Value,
}

impl Receipt {
    fn load(path: PathBuf, input: Value) -> Result<Self, String> {
        let value = match std::fs::read(&path) {
            Ok(bytes) => serde_json::from_slice::<Value>(&bytes)
                .map_err(|_| "The saved record of this operation cannot be read".to_string())?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                json!({"input": input, "status": "prepared"})
            }
            Err(error) => return Err(format!("Could not read the operation record: {error}")),
        };
        if value["input"] != input {
            return Err("An operation ID cannot be reused for different content".into());
        }
        Ok(Self { path, value })
    }

    fn stage(&self) -> Stage {
        Stage::parse(self.value["status"].as_str().unwrap_or_default())
    }

    fn set(&mut self, stage: Stage) -> Result<(), String> {
        self.value["status"] = json!(stage.name());
        self.save()
    }

    fn save(&self) -> Result<(), String> {
        let dir = self.path.parent().ok_or("Invalid receipt path")?;
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
        }
        let mut file = tempfile::NamedTempFile::new_in(dir).map_err(|e| e.to_string())?;
        serde_json::to_writer(&mut file, &self.value).map_err(|e| e.to_string())?;
        std::io::Write::flush(&mut file).map_err(|e| e.to_string())?;
        file.persist(&self.path).map_err(|e| e.error.to_string())?;
        Ok(())
    }
}

// ---- the bridge side ----

/// What the Agents window said about one creation.
#[derive(Debug)]
pub(super) enum Reply {
    /// Cursor created the chat, passed every check and started it.
    Started(Value),
    /// A dry run: the chat passed every check and was discarded without sending anything.
    DryRun(Value),
    /// Nothing was sent to a model.
    Rejected {
        reason: String,
        detail: Option<String>,
    },
    /// Cursor may have received the task; nothing more can be said.
    Unknown(String),
}

/// Why one ask never reached the Agents window's code.
#[derive(Debug)]
pub(super) enum SendError {
    /// The request was not written to Cursor's endpoint at all.
    NotSent(String),
    /// It may have been; the answer is missing or unreadable.
    Unclear(String),
}

fn text_of(value: &Value) -> Option<String> {
    value
        .as_str()
        .filter(|text| !text.is_empty())
        .map(|text| text.chars().take(200).collect())
}

/// Asks until the Agents window gives a final answer or the daemon's window is over. The
/// renderer answers `pending` while the chat is still being made; asking again joins the same
/// creation, it never starts a second one.
pub(super) async fn poll<F, Fut>(
    mut send: F,
    request: Value,
    deadline: Instant,
    pause: Duration,
) -> Reply
where
    F: FnMut(Value) -> Fut,
    Fut: Future<Output = Result<Value, SendError>>,
{
    let mut heard = false;
    let mut last = String::from("no answer from Cursor");
    loop {
        match send(request.clone()).await {
            Ok(reply) => {
                heard = true;
                match reply["status"].as_str() {
                    Some("pending") => {}
                    Some("started") => return Reply::Started(reply),
                    Some("dryRun") => return Reply::DryRun(reply),
                    Some("rejected") => {
                        return Reply::Rejected {
                            reason: text_of(&reply["reason"])
                                .unwrap_or_else(|| "nativeUnavailable".into()),
                            detail: text_of(&reply["detail"]),
                        }
                    }
                    Some("unknown") => {
                        return Reply::Unknown(
                            text_of(&reply["detail"]).unwrap_or_else(|| "controlUnknown".into()),
                        )
                    }
                    _ => return Reply::Unknown("unexpected reply".into()),
                }
            }
            Err(SendError::NotSent(detail)) if !heard => {
                return Reply::Rejected {
                    reason: "nativeUnavailable".into(),
                    detail: Some(detail),
                }
            }
            Err(SendError::NotSent(detail) | SendError::Unclear(detail)) => last = detail,
        }
        if Instant::now() >= deadline {
            return Reply::Unknown(if heard {
                "Cursor kept creating the chat past the time limit".into()
            } else {
                last
            });
        }
        tokio::time::sleep(pause).await;
    }
}

#[allow(async_fn_in_trait)]
pub(super) trait Backend {
    /// Why the Desktop route cannot be used right now, from what is known without attaching.
    async fn unavailable(&self) -> Option<super::launch::UnavailableReason>;
    /// Hands the request to the Agents window and waits for its final answer.
    async fn create(&self, request: Value, deadline: Instant) -> Reply;
}

fn validate(operation: &Operation) -> Result<(), String> {
    let Operation::Create {
        cwd,
        text,
        permission,
        id,
        model,
        title,
        ..
    } = operation;
    uuid::Uuid::parse_str(id).map_err(|_| "Invalid operation ID")?;
    if id != &id.to_ascii_lowercase() {
        return Err("Invalid operation ID".into());
    }
    if !Path::new(cwd).is_absolute() || !Path::new(cwd).is_dir() {
        return Err("The workspace must be an existing absolute directory".into());
    }
    if text.trim().is_empty() {
        return Err("Task must not be empty".into());
    }
    if text.contains('\0') {
        return Err("Task must not contain NUL characters".into());
    }
    if text.len() > MAX_TEXT_BYTES {
        return Err("Task is too long".into());
    }
    if !matches!(permission.as_str(), "agent-default" | "yolo") {
        return Err("Unknown permission mode".into());
    }
    if model
        .as_deref()
        .is_some_and(|model| model.is_empty() || model.len() > 200 || model.contains('\0'))
    {
        return Err("Invalid model".into());
    }
    if title
        .as_deref()
        .is_some_and(|title| title.chars().count() > MAX_TITLE_CHARS || title.contains('\0'))
    {
        return Err("Title is too long".into());
    }
    Ok(())
}

/// What a dry run puts in the chat's prompt. The pre-submit gate is what keeps a chat from ever
/// reaching a model; should Cursor one day ignore it, this is all that would be sent, never the
/// user's task.
const DRY_RUN_TEXT: &str = "AskHuman dry run: this message is not meant to be sent.";

/// The request the Agents window's code receives.
fn request_of(operation: &Operation, plan: &DesktopPlan, deadline_ms: u64) -> Value {
    let Operation::Create {
        cwd,
        text,
        permission,
        id,
        title,
        dry_run,
        ..
    } = operation;
    let canonical = std::fs::canonicalize(cwd)
        .ok()
        .and_then(|path| path.to_str().map(str::to_string));
    let mut aliases = vec![cwd.clone()];
    if let Some(canonical) = &canonical {
        aliases.push(canonical.clone());
    }
    json!({
        "id": id,
        "cwd": canonical.unwrap_or_else(|| cwd.clone()),
        "aliases": aliases,
        "text": if *dry_run { DRY_RUN_TEXT } else { text.as_str() },
        "yolo": permission == "yolo",
        "title": title,
        "dryRun": dry_run,
        "deadline": deadline_ms,
        "maxMode": plan.max_mode,
        "model": plan.model,
    })
}

/// What a finished creation leaves for the caller.
pub(super) struct Executed {
    pub answer: Value,
    /// Set when Max was forced off and the chat is worth watching afterwards.
    pub watch: Option<Watch>,
}

pub(super) struct Watch {
    pub session_id: String,
    pub project: String,
}

fn failed(reason: &str, detail: Option<&str>) -> String {
    reject_text(reason, detail, Lang::current())
}

/// Runs one creation. `receipts` is where the operation's record lives.
pub(super) async fn execute_with<B: Backend>(
    backend: &B,
    receipts: &Path,
    plan_for: impl FnOnce(Option<&str>, bool) -> Result<DesktopPlan, PlanError>,
    operation: Operation,
) -> Result<Executed, String> {
    validate(&operation)?;
    let Operation::Create {
        ref id,
        ref cwd,
        ref model,
        allow_max,
        dry_run,
        ..
    } = operation;
    let id = id.clone();
    let mut receipt = Receipt::load(
        receipts.join(format!("{id}.json")),
        serde_json::to_value(&operation).map_err(|e| e.to_string())?,
    )?;
    match receipt.stage() {
        Stage::Accepted | Stage::DryRun => {
            return Ok(Executed {
                answer: receipt.value["answer"].clone(),
                watch: None,
            })
        }
        Stage::Creating | Stage::Unknown => {
            return Err(unknown_text(&id, None, Lang::current()));
        }
        Stage::Prepared | Stage::Rejected => {}
    }
    let plan = plan_for(model.as_deref(), allow_max).map_err(|error| match error {
        PlanError::MaxRequired(required) => required.guidance(Lang::current()),
        other => other.to_string(),
    })?;
    if let Some(reason) = backend.unavailable().await {
        return Err(reason.text(Lang::current()).to_string());
    }
    receipt.set(Stage::Creating)?;
    let now = Instant::now();
    let deadline_ms = super::now_ms() + RENDERER_WINDOW.as_millis() as u64;
    let reply = backend
        .create(
            request_of(&operation, &plan, deadline_ms),
            now + DAEMON_WINDOW,
        )
        .await;
    match reply {
        Reply::Started(reply) | Reply::DryRun(reply) => {
            let dry = matches!(reply["status"].as_str(), Some("dryRun"));
            let answer = json!({
                "sessionId": id,
                "status": if dry { "dryRun" } else { "started" },
                "dryRun": dry,
                "observed": reply["observed"],
                "tidied": reply["tidied"],
                "maxMode": plan.max_mode,
            });
            receipt.value["answer"] = answer.clone();
            receipt.set(if dry { Stage::DryRun } else { Stage::Accepted })?;
            let watch = (!dry && !dry_run && plan.max_mode == Some(false)).then(|| Watch {
                session_id: id.clone(),
                project: crate::project::detect_from(Path::new(cwd)),
            });
            Ok(Executed { answer, watch })
        }
        Reply::Rejected { reason, detail } => {
            receipt.value["failure"] = json!({"reason": reason, "detail": detail});
            receipt.set(Stage::Rejected)?;
            Err(failed(&reason, detail.as_deref()))
        }
        Reply::Unknown(detail) => {
            receipt.value["failure"] = json!({"reason": "unknown", "detail": detail});
            receipt.set(Stage::Unknown)?;
            Err(unknown_text(&id, Some(&detail), Lang::current()))
        }
    }
}

// ---- words ----

fn unknown_text(id: &str, detail: Option<&str>, lang: Lang) -> String {
    let detail = detail.filter(|detail| !detail.is_empty());
    match lang {
        Lang::Zh => format!(
            "无法确认任务是否已在 Cursor 里发出。请先在 Cursor 的 Agents 窗口里查看会话 {id}，确认后再决定是否重新创建。{}",
            detail.map(|d| format!("（{d}）")).unwrap_or_default()
        ),
        Lang::En => format!(
            "Could not confirm whether the task reached Cursor. Check chat {id} in Cursor's Agents window before creating it again.{}",
            detail.map(|d| format!(" ({d})")).unwrap_or_default()
        ),
    }
}

/// Words a refusal from the Agents window. Every one of them means nothing was sent to a model.
pub(crate) fn reject_text(reason: &str, detail: Option<&str>, lang: Lang) -> String {
    let with = |text: &str| match (detail, lang) {
        (Some(detail), Lang::Zh) => format!("{text}（{detail}）"),
        (Some(detail), Lang::En) => format!("{text} ({detail})"),
        (None, _) => text.to_string(),
    };
    let mismatch_zh = |what: &str| {
        with(&format!(
            "Cursor 创建的会话与请求不一致（{what}），已放弃，没有向模型发送任何内容。"
        ))
    };
    let mismatch_en = |what: &str| {
        with(&format!(
            "The chat Cursor created does not match the request ({what}); it was dropped and nothing was sent to a model."
        ))
    };
    match (reason, lang) {
        ("noAgentsWindow" | "notAgentsWindow", Lang::Zh) => {
            "请先打开 Cursor 的 Agents 窗口再创建任务；也可以在设置里把 Cursor 新任务的运行位置改为 CLI。".into()
        }
        ("noAgentsWindow" | "notAgentsWindow", Lang::En) => {
            "Open Cursor's Agents window first, or set new Cursor tasks to run in the CLI in Settings.".into()
        }
        ("multipleAgentsWindows", Lang::Zh) => "检测到多个 Agents 窗口，无法确定目标，请只保留一个再试。".into(),
        ("multipleAgentsWindows", Lang::En) => {
            "More than one Agents window is open, so there is no single target. Close the others and retry.".into()
        }
        ("projectNotFound", Lang::Zh) => with(
            "Agents 窗口里没有这个项目。请先在 Cursor 的 Agents 窗口添加该项目，或把运行位置改为 CLI。",
        ),
        ("projectNotFound", Lang::En) => with(
            "The Agents window does not know this project. Add it in Cursor's Agents window first, or run the task in the CLI.",
        ),
        ("projectAmbiguous", Lang::Zh) => "Agents 窗口里有多个项目对应同一目录，无法确定目标。".into(),
        ("projectAmbiguous", Lang::En) => {
            "More than one project in the Agents window points at this directory.".into()
        }
        ("modelUnavailable", Lang::Zh) => with("无法确定 Cursor 当前的模型配置，没有创建任务。"),
        ("modelUnavailable", Lang::En) => with("Cursor's current model configuration could not be read; no task was created."),
        ("composerMismatch", Lang::Zh) => mismatch_zh("会话标识"),
        ("composerMismatch", Lang::En) => mismatch_en("chat identity"),
        ("locationMismatch", Lang::Zh) => mismatch_zh("不是本地项目"),
        ("locationMismatch", Lang::En) => mismatch_en("not a local project"),
        ("projectMismatch", Lang::Zh) => mismatch_zh("项目"),
        ("projectMismatch", Lang::En) => mismatch_en("project"),
        ("modeMismatch", Lang::Zh) => mismatch_zh("模式"),
        ("modeMismatch", Lang::En) => mismatch_en("mode"),
        ("modelMismatch", Lang::Zh) => mismatch_zh("模型"),
        ("modelMismatch", Lang::En) => mismatch_en("model"),
        ("maxModeMismatch", Lang::Zh) => mismatch_zh("Max 模式"),
        ("maxModeMismatch", Lang::En) => mismatch_en("Max mode"),
        ("approvalMismatch", Lang::Zh) => mismatch_zh("权限模式"),
        ("approvalMismatch", Lang::En) => mismatch_en("permission mode"),
        ("timeout", Lang::Zh) => "等待 Cursor 超时，没有发送任何内容。".into(),
        ("timeout", Lang::En) => "Cursor took too long; nothing was sent.".into(),
        ("conflict", Lang::Zh) => "该操作 ID 已用于不同内容。".into(),
        ("conflict", Lang::En) => "This operation ID was already used for different content.".into(),
        (_, Lang::Zh) => with("无法使用 Cursor 的 Agents 窗口创建任务，没有发送任何内容。"),
        (_, Lang::En) => with("Cursor's Agents window could not be used to create the task; nothing was sent."),
    }
}

// ---- the real thing ----

struct Live;

impl Backend for Live {
    async fn unavailable(&self) -> Option<super::launch::UnavailableReason> {
        use super::launch::{desktop_reason, static_facts};
        let facts = static_facts();
        // Not macOS, a Dev Instance, the integration off: no need to look at the processes.
        if let Some(reason) = desktop_reason(facts) {
            return Some(reason);
        }
        #[cfg(target_os = "macos")]
        return desktop_reason(super::launch::DesktopFacts {
            processes: super::runtime::process_count().await.ok(),
            ..facts
        });
        #[cfg(not(target_os = "macos"))]
        None
    }

    async fn create(&self, request: Value, deadline: Instant) -> Reply {
        #[cfg(target_os = "macos")]
        {
            super::runtime::create(request, deadline).await
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = (request, deadline);
            Reply::Rejected {
                reason: "unsupported".into(),
                detail: None,
            }
        }
    }
}

/// The daemon's entry: runs the operation, and watches the chat afterwards when Max was forced off.
pub async fn execute(operation: Operation) -> Result<Value, String> {
    let executed = execute_with(&Live, &receipts_dir(), plan, operation).await?;
    crate::daemon::lifecycle::log_runtime_event(
        "cursor_desktop",
        if executed.answer["status"] == "started" {
            "create_started"
        } else {
            "create_dry_run"
        },
        executed.answer["sessionId"].as_str(),
    );
    if let Some(watch) = executed.watch {
        tokio::spawn(cursor_max::recheck::after_desktop(
            cursor_max::recheck::DesktopLaunch {
                id: watch.session_id,
                project: watch.project,
            },
        ));
    }
    Ok(executed.answer)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::cursor_max::catalog::fixtures::catalog;
    use std::sync::Mutex;

    const ID: &str = "3f2b1c0e-5a4d-4e6f-8a7b-9c0d1e2f3a4b";

    fn settings() -> CursorMaxConfig {
        CursorMaxConfig {
            policy: CursorMaxPolicy::EnforceOff,
            plan: crate::config::CursorPlanSetting::Auto,
        }
    }

    /// A legacy-plan account: the synthetic catalog says `hasTokenBasedPricing: false`.
    fn inputs() -> Inputs {
        Inputs {
            catalog: Some(catalog(Some(false))),
            cli: None,
        }
    }

    fn operation(dir: &Path) -> Operation {
        Operation::Create {
            cwd: dir.to_string_lossy().into_owned(),
            text: "Build it".into(),
            permission: "agent-default".into(),
            id: ID.into(),
            model: None,
            allow_max: false,
            title: Some("Demo".into()),
            dry_run: false,
        }
    }

    struct Scripted {
        unavailable: Option<super::super::launch::UnavailableReason>,
        replies: Mutex<Vec<Reply>>,
        requests: Mutex<Vec<Value>>,
    }

    impl Scripted {
        fn new(replies: Vec<Reply>) -> Self {
            Self {
                unavailable: None,
                replies: Mutex::new(replies),
                requests: Mutex::default(),
            }
        }
    }

    impl Backend for Scripted {
        async fn unavailable(&self) -> Option<super::super::launch::UnavailableReason> {
            self.unavailable
        }
        async fn create(&self, request: Value, _deadline: Instant) -> Reply {
            self.requests.lock().unwrap().push(request);
            self.replies.lock().unwrap().remove(0)
        }
    }

    fn started() -> Reply {
        Reply::Started(json!({"status": "started", "observed": {"maxMode": false}}))
    }

    fn live_plan(model: Option<&str>, allow_max: bool) -> Result<DesktopPlan, PlanError> {
        plan_with(&settings(), &inputs(), model, allow_max)
    }

    // -- the plan --

    #[test]
    fn max_stays_off_for_a_model_that_does_not_need_it_and_the_variant_is_explicit() {
        let plan = plan_with(&settings(), &inputs(), Some("mixed-200k-high"), false).unwrap();
        assert_eq!(plan.max_mode, Some(false));
        assert_eq!(plan.assessment.need, Need::NotRequired);
        let model = plan.model.unwrap();
        assert_eq!(model.model_name, "mixed");
        assert_eq!(
            model.parameters,
            vec![
                Parameter {
                    id: "context".into(),
                    value: "200k".into()
                },
                Parameter {
                    id: "effort".into(),
                    value: "high".into()
                },
            ]
        );
    }

    #[test]
    fn without_a_model_the_chat_starts_with_cursors_own_default_and_max_off() {
        // The synthetic default is `mixed` with 200k/high, which does not need Max.
        let plan = plan_with(&settings(), &inputs(), None, false).unwrap();
        assert_eq!(plan.max_mode, Some(false));
        assert!(plan.model.is_none());
        assert_eq!(plan.assessment.need, Need::NotRequired);
    }

    #[test]
    fn a_model_that_needs_max_is_refused_unless_max_is_allowed() {
        for id in ["flag-only-max", "grok-only-max-high", "mixed-1m-high"] {
            match plan_with(&settings(), &inputs(), Some(id), false) {
                Err(PlanError::MaxRequired(required)) => {
                    assert_eq!(required.model.as_deref(), Some(id))
                }
                other => panic!("{id}: expected a refusal, got {other:?}"),
            }
            let allowed = plan_with(&settings(), &inputs(), Some(id), true).unwrap();
            assert_eq!(allowed.max_mode, Some(true), "{id}");
        }
    }

    #[test]
    fn allowing_max_never_turns_it_on_for_a_model_that_does_not_need_it() {
        let plan = plan_with(&settings(), &inputs(), Some("mixed-200k-high"), true).unwrap();
        assert_eq!(plan.max_mode, None);
    }

    #[test]
    fn the_inherit_policy_and_the_token_plan_leave_max_alone() {
        let mut inherit = settings();
        inherit.policy = CursorMaxPolicy::Inherit;
        let plan = plan_with(&inherit, &inputs(), Some("flag-only-max"), false).unwrap();
        assert_eq!(plan.max_mode, None);
        let mut token = settings();
        token.plan = crate::config::CursorPlanSetting::TokenBased;
        let plan = plan_with(&token, &inputs(), Some("flag-only-max"), true).unwrap();
        assert_eq!(plan.max_mode, None);
    }

    #[test]
    fn without_a_catalog_max_is_forced_off_but_an_explicit_model_cannot_be_mapped() {
        let empty = Inputs::default();
        let plan = plan_with(&settings(), &empty, None, false).unwrap();
        assert_eq!(plan.max_mode, Some(false));
        assert_eq!(plan.assessment.need, Need::Unknown);
        assert!(plan.model.is_none());
        assert!(matches!(
            plan_with(&settings(), &empty, Some("mixed-200k-high"), false),
            Err(PlanError::UnknownModel(_))
        ));
        assert!(matches!(
            plan_with(&settings(), &inputs(), Some("no-such-model"), false),
            Err(PlanError::UnknownModel(_))
        ));
    }

    #[test]
    fn a_model_named_without_a_variant_starts_as_the_ide_would_start_it() {
        let catalog = catalog(Some(false));
        let mapped = map_model(Some(&catalog), "mixed").unwrap();
        assert_eq!(mapped.model_name, "mixed");
        // The default non-Max variant of `mixed`.
        assert_eq!(mapped.parameters.len(), 2);
        let auto = map_model(Some(&catalog), "auto").unwrap();
        assert_eq!(auto.model_name, "default");
        assert!(auto.parameters.is_empty());
    }

    // -- the operation --

    #[test]
    fn the_operation_is_checked_before_anything_else() {
        let dir = tempfile::tempdir().unwrap();
        let ok = operation(dir.path());
        assert!(validate(&ok).is_ok());
        let bad = |change: &dyn Fn(&mut Operation)| {
            let mut op = operation(dir.path());
            change(&mut op);
            validate(&op).unwrap_err()
        };
        assert!(bad(&|op| {
            let Operation::Create { id, .. } = op;
            *id = "nope".into();
        })
        .contains("operation ID"));
        assert!(bad(&|op| {
            let Operation::Create { id, .. } = op;
            *id = ID.to_uppercase();
        })
        .contains("operation ID"));
        assert!(bad(&|op| {
            let Operation::Create { cwd, .. } = op;
            *cwd = "relative".into();
        })
        .contains("directory"));
        assert!(bad(&|op| {
            let Operation::Create { text, .. } = op;
            *text = "  ".into();
        })
        .contains("empty"));
        assert!(bad(&|op| {
            let Operation::Create { text, .. } = op;
            *text = "a\0b".into();
        })
        .contains("NUL"));
        assert!(bad(&|op| {
            let Operation::Create { permission, .. } = op;
            *permission = "root".into();
        })
        .contains("permission"));
        assert!(bad(&|op| {
            let Operation::Create { model, .. } = op;
            *model = Some(String::new());
        })
        .contains("model"));
    }

    #[test]
    fn the_request_names_the_project_by_every_spelling_and_carries_the_plan() {
        let dir = tempfile::tempdir().unwrap();
        let op = operation(dir.path());
        let plan = live_plan(Some("mixed-200k-high"), false).unwrap();
        let request = request_of(&op, &plan, 1234);
        assert_eq!(request["id"], ID);
        assert_eq!(request["yolo"], false);
        assert_eq!(request["dryRun"], false);
        assert_eq!(request["text"], "Build it");
        // A dry run never carries the user's task, whatever the gate does.
        let mut dry = operation(dir.path());
        let Operation::Create { dry_run, .. } = &mut dry;
        *dry_run = true;
        let dry_request = request_of(&dry, &plan, 1234);
        assert_eq!(dry_request["dryRun"], true);
        assert_eq!(dry_request["text"], DRY_RUN_TEXT);
        assert!(!dry_request.to_string().contains("Build it"));
        assert_eq!(request["deadline"], 1234);
        assert_eq!(request["maxMode"], false);
        assert_eq!(request["model"]["modelName"], "mixed");
        assert!(request["model"]["parameters"].is_array());
        let aliases = request["aliases"].as_array().unwrap();
        assert!(aliases.iter().any(|a| a == dir.path().to_str().unwrap()));
        // A task that leaves Max to Cursor says so with null, not false.
        let inherit = request_of(
            &op,
            &plan_with(
                &{
                    let mut s = settings();
                    s.policy = CursorMaxPolicy::Inherit;
                    s
                },
                &inputs(),
                None,
                false,
            )
            .unwrap(),
            1,
        );
        assert!(inherit["maxMode"].is_null());
        assert!(inherit["model"].is_null());
    }

    // -- polling --

    fn replies(
        script: Vec<Result<Value, SendError>>,
    ) -> impl FnMut(Value) -> std::future::Ready<Result<Value, SendError>> {
        let script = std::sync::Arc::new(Mutex::new(script));
        move |_| {
            let mut script = script.lock().unwrap();
            let next = if script.len() > 1 {
                script.remove(0)
            } else {
                clone_reply(&script[0])
            };
            std::future::ready(next)
        }
    }

    fn clone_reply(reply: &Result<Value, SendError>) -> Result<Value, SendError> {
        match reply {
            Ok(value) => Ok(value.clone()),
            Err(SendError::NotSent(text)) => Err(SendError::NotSent(text.clone())),
            Err(SendError::Unclear(text)) => Err(SendError::Unclear(text.clone())),
        }
    }

    fn soon() -> Instant {
        Instant::now() + Duration::from_millis(60)
    }

    #[tokio::test]
    async fn pending_answers_are_asked_again_until_the_final_one() {
        let reply = poll(
            replies(vec![
                Ok(json!({"status": "pending"})),
                Ok(json!({"status": "pending"})),
                Ok(json!({"status": "started", "observed": {}})),
            ]),
            json!({}),
            soon(),
            Duration::from_millis(1),
        )
        .await;
        assert!(matches!(reply, Reply::Started(_)));
    }

    #[tokio::test]
    async fn a_refusal_and_a_dry_run_are_final() {
        let refused = poll(
            replies(vec![Ok(json!({"status": "rejected", "reason": "projectNotFound", "detail": "3 known projects"}))]),
            json!({}),
            soon(),
            Duration::from_millis(1),
        )
        .await;
        match refused {
            Reply::Rejected { reason, detail } => {
                assert_eq!(reason, "projectNotFound");
                assert_eq!(detail.as_deref(), Some("3 known projects"));
            }
            other => panic!("{other:?}"),
        }
        let dry = poll(
            replies(vec![Ok(json!({"status": "dryRun"}))]),
            json!({}),
            soon(),
            Duration::from_millis(1),
        )
        .await;
        assert!(matches!(dry, Reply::DryRun(_)));
    }

    #[tokio::test]
    async fn a_request_that_never_left_is_a_refusal_but_a_lost_answer_is_unknown() {
        let never_sent = poll(
            replies(vec![Err(SendError::NotSent("endpoint invalid".into()))]),
            json!({}),
            soon(),
            Duration::from_millis(1),
        )
        .await;
        assert!(
            matches!(never_sent, Reply::Rejected { ref reason, .. } if reason == "nativeUnavailable")
        );
        let lost = poll(
            replies(vec![Err(SendError::Unclear("timed out".into()))]),
            json!({}),
            soon(),
            Duration::from_millis(1),
        )
        .await;
        assert!(matches!(lost, Reply::Unknown(ref detail) if detail == "timed out"));
        // A transient failure in the middle does not end the wait.
        let recovered = poll(
            replies(vec![
                Ok(json!({"status": "pending"})),
                Err(SendError::Unclear("timed out".into())),
                Ok(json!({"status": "started"})),
            ]),
            json!({}),
            soon(),
            Duration::from_millis(1),
        )
        .await;
        assert!(matches!(recovered, Reply::Started(_)));
    }

    #[tokio::test]
    async fn a_creation_that_is_still_pending_at_the_deadline_is_unknown() {
        let reply = poll(
            replies(vec![Ok(json!({"status": "pending"}))]),
            json!({}),
            soon(),
            Duration::from_millis(5),
        )
        .await;
        assert!(matches!(reply, Reply::Unknown(_)));
        let strange = poll(
            replies(vec![Ok(json!({"status": "surprise"}))]),
            json!({}),
            soon(),
            Duration::from_millis(5),
        )
        .await;
        assert!(matches!(strange, Reply::Unknown(_)));
    }

    // -- the whole operation --

    #[tokio::test]
    async fn a_started_chat_is_remembered_and_a_retry_answers_from_the_receipt() {
        let dir = tempfile::tempdir().unwrap();
        let receipts = tempfile::tempdir().unwrap();
        let backend = Scripted::new(vec![started()]);
        let executed = execute_with(&backend, receipts.path(), live_plan, operation(dir.path()))
            .await
            .unwrap();
        assert_eq!(executed.answer["sessionId"], ID);
        assert_eq!(executed.answer["status"], "started");
        let watch = executed
            .watch
            .expect("Max was forced off, so the chat is watched");
        assert_eq!(watch.session_id, ID);
        // The same operation again does not reach Cursor a second time.
        let again = execute_with(&backend, receipts.path(), live_plan, operation(dir.path()))
            .await
            .unwrap();
        assert_eq!(again.answer, executed.answer);
        assert!(again.watch.is_none());
        assert_eq!(backend.requests.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn an_operation_id_cannot_be_reused_for_other_content() {
        let dir = tempfile::tempdir().unwrap();
        let receipts = tempfile::tempdir().unwrap();
        let backend = Scripted::new(vec![started()]);
        execute_with(&backend, receipts.path(), live_plan, operation(dir.path()))
            .await
            .unwrap();
        let mut other = operation(dir.path());
        let Operation::Create { text, .. } = &mut other;
        *text = "Something else".into();
        let error = execute_with(&backend, receipts.path(), live_plan, other)
            .await
            .err()
            .unwrap();
        assert!(error.contains("cannot be reused"));
        assert_eq!(backend.requests.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn a_refusal_leaves_the_operation_free_to_run_again() {
        let dir = tempfile::tempdir().unwrap();
        let receipts = tempfile::tempdir().unwrap();
        let backend = Scripted::new(vec![
            Reply::Rejected {
                reason: "noAgentsWindow".into(),
                detail: None,
            },
            started(),
        ]);
        let error = execute_with(&backend, receipts.path(), live_plan, operation(dir.path()))
            .await
            .err()
            .unwrap();
        assert!(error.contains("Agents"));
        assert!(
            execute_with(&backend, receipts.path(), live_plan, operation(dir.path()))
                .await
                .is_ok()
        );
        assert_eq!(backend.requests.lock().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn an_unknown_outcome_is_never_retried() {
        let dir = tempfile::tempdir().unwrap();
        let receipts = tempfile::tempdir().unwrap();
        let backend = Scripted::new(vec![Reply::Unknown("lost".into()), started()]);
        let first = execute_with(&backend, receipts.path(), live_plan, operation(dir.path()))
            .await
            .err()
            .unwrap();
        assert!(first.contains(ID));
        let second = execute_with(&backend, receipts.path(), live_plan, operation(dir.path()))
            .await
            .err()
            .unwrap();
        assert!(second.contains(ID));
        assert_eq!(backend.requests.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn a_crash_between_sending_and_hearing_back_reads_as_unknown() {
        let dir = tempfile::tempdir().unwrap();
        let receipts = tempfile::tempdir().unwrap();
        let op = operation(dir.path());
        let mut receipt = Receipt::load(
            receipts.path().join(format!("{ID}.json")),
            serde_json::to_value(&op).unwrap(),
        )
        .unwrap();
        receipt.set(Stage::Creating).unwrap();
        let backend = Scripted::new(vec![started()]);
        let error = execute_with(&backend, receipts.path(), live_plan, op)
            .await
            .err()
            .unwrap();
        assert!(error.contains(ID));
        assert!(backend.requests.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_dry_run_is_remembered_but_never_watched() {
        let dir = tempfile::tempdir().unwrap();
        let receipts = tempfile::tempdir().unwrap();
        let backend = Scripted::new(vec![Reply::DryRun(
            json!({"status": "dryRun", "tidied": "archived"}),
        )]);
        let mut op = operation(dir.path());
        let Operation::Create { dry_run, .. } = &mut op;
        *dry_run = true;
        let executed = execute_with(&backend, receipts.path(), live_plan, op)
            .await
            .unwrap();
        assert_eq!(executed.answer["status"], "dryRun");
        assert_eq!(executed.answer["dryRun"], true);
        assert!(executed.watch.is_none());
        assert_eq!(backend.requests.lock().unwrap()[0]["dryRun"], true);
    }

    #[tokio::test]
    async fn a_task_that_needs_max_never_reaches_cursor_without_it() {
        let dir = tempfile::tempdir().unwrap();
        let receipts = tempfile::tempdir().unwrap();
        let backend = Scripted::new(vec![started()]);
        let mut op = operation(dir.path());
        let Operation::Create { model, .. } = &mut op;
        *model = Some("flag-only-max".into());
        let error = execute_with(&backend, receipts.path(), live_plan, op)
            .await
            .err()
            .unwrap();
        assert!(error.contains("Max"));
        assert!(backend.requests.lock().unwrap().is_empty());
        assert!(!receipts.path().join(format!("{ID}.json")).exists());
    }

    #[tokio::test]
    async fn an_unavailable_route_is_reported_before_a_receipt_exists() {
        let dir = tempfile::tempdir().unwrap();
        let receipts = tempfile::tempdir().unwrap();
        let mut backend = Scripted::new(vec![started()]);
        backend.unavailable = Some(super::super::launch::UnavailableReason::AppNotRunning);
        let error = execute_with(&backend, receipts.path(), live_plan, operation(dir.path()))
            .await
            .err()
            .unwrap();
        assert!(!error.is_empty());
        assert!(backend.requests.lock().unwrap().is_empty());
        assert!(!receipts.path().join(format!("{ID}.json")).exists());
    }

    #[test]
    fn every_refusal_says_that_nothing_was_sent() {
        for reason in [
            "noAgentsWindow",
            "multipleAgentsWindows",
            "projectNotFound",
            "projectAmbiguous",
            "modelUnavailable",
            "composerMismatch",
            "locationMismatch",
            "projectMismatch",
            "modeMismatch",
            "modelMismatch",
            "maxModeMismatch",
            "approvalMismatch",
            "timeout",
            "nativeUnavailable",
        ] {
            let zh = reject_text(reason, None, Lang::Zh);
            let en = reject_text(reason, None, Lang::En);
            assert!(!zh.is_empty() && !en.is_empty(), "{reason}");
        }
        assert!(reject_text("maxModeMismatch", Some("true"), Lang::En).contains("nothing was sent"));
        assert!(reject_text("maxModeMismatch", None, Lang::Zh).contains("没有向模型发送任何内容"));
    }
}
