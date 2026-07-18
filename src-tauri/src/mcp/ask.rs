//! `ask` 工具：把 MCP 入参翻译成 `AskHuman --output json …` argv，spawn 子进程复用既有 ask 流程，
//! 再把子进程的 JSON 结果整理成 MCP `structuredContent` + `TextContent`，并将人类回复中的图片读回为
//! `ImageContent` 一并返回。
//!
//! 关键点：
//! - 子进程用 Tokio `Command` 运行 —— stdin 被置空、stdout/stderr 被捕获，因此**不会**污染本
//!   server 的 STDIO MCP 协议流；`kill_on_drop(true)` + 对 rmcp `CancellationToken` 的 `select!`
//!   保证 MCP 调用被客户端取消时子进程随之终止，进而让 daemon 从 CLI socket EOF 取消在途请求。
//! - 子进程的 JSON 含脚本专用的 `selected_indices`；反序列化进 [`AskResult`]（无该字段）即自动丢弃，
//!   再重新序列化为 `structuredContent`，对 MCP 客户端不暴露该字段。
//!
//! ## 取消语义（为何必须 await token，而不能只靠 drop future）
//!
//! rmcp 收到 `notifications/cancelled` 时**只** `cancel()` 该 request 的 `CancellationToken`，
//! **不会** abort / drop 已 spawn 的 tool handler。因此仅设 `kill_on_drop` 而不 `select!` token
//! 时，`ask` 会继续 await 子进程，弹窗与 IM 卡成为孤儿。token 取消路径必须显式 `kill` 子进程。

use base64::Engine;
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, ContentBlock, Implementation, ServerCapabilities, ServerInfo};
use rmcp::{tool, tool_handler, tool_router, ErrorData as McpError, ServerHandler};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::{Output, Stdio};
use tokio_util::sync::CancellationToken;
use tokio_util::task::AbortOnDropHandle;

// `ask` 工具的入参（MCP 入参 schema 由 schemars 从本结构派生）。结构体级注释用 `//` 以免泄漏进对外
// schema 的 description；字段级 `///` 才是给 agent 读的描述，须为英文。
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AskParams {
    /// Shared context/description shown above all questions, rendered as Markdown
    /// (GitHub-flavored: headings, lists, code blocks, tables, links, etc.). When no
    /// `questions` are given, this text itself becomes the single question.
    #[serde(default)]
    pub message: Option<String>,
    /// One or more questions to ask the human.
    #[serde(default)]
    pub questions: Option<Vec<AskQuestion>>,
    /// Optional file paths (images or documents) to attach to what the human sees.
    #[serde(default)]
    pub files: Option<Vec<String>>,
}

// Input for `todo_add` (project-scoped queue; same store as CLI `todo add`).
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TodoAddParams {
    /// Concise task text to append to the current project's todo queue.
    /// Prefer a single executable sentence, ideally under 100 characters.
    pub text: String,
    /// When true, mark the todo for auto-run on the next `whats_next` (⚡).
    #[serde(default)]
    pub auto: Option<bool>,
}

// Output for `todo_add`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TodoAddResult {
    /// 1-based index in the project's pending list after the add.
    pub index: usize,
    /// Stable id of the new entry (for debugging / future tools).
    pub id: String,
    /// Trimmed task text that was stored.
    pub text: String,
    /// Absolute project key (git root path) the todo was attached to.
    pub project: String,
    /// Whether the entry is marked auto-run.
    pub auto: bool,
}

// Input for `whats_next` (spec todo-whats-next D2).
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct WhatsNextParams {
    /// Optional completion report shown to the human above the fixed "What should we do
    /// next?" question, rendered as Markdown.
    #[serde(default)]
    pub message: Option<String>,
    /// Optional concrete next-task suggestions. NEVER include an end, stop, or no-more-work option;
    /// AskHuman adds the ending choice. Omit this field when you have no task to suggest. Options
    /// appear before project todos, and `recommended` renders the same emphasis as in `ask`.
    #[serde(default)]
    pub options: Option<Vec<AskOption>>,
    /// Optional file paths (e.g. a report or summary document) to attach to what the human sees.
    #[serde(default)]
    pub files: Option<Vec<String>>,
}

// 单个问题。
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[schemars(inline)]
pub struct AskQuestion {
    /// The question text.
    pub question: String,
    /// Optional predefined options the human may pick from.
    #[serde(default)]
    pub options: Option<Vec<AskOption>>,
}

// 单个选项。
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[schemars(inline)]
pub struct AskOption {
    /// The option label.
    pub text: String,
    /// Mark this option as recommended (rendered with emphasis).
    #[serde(default)]
    pub recommended: bool,
}

// `ask` 工具的出参（同时用于声明 output schema 与承载 `structuredContent`）。
//
// 字段名刻意与 `cli::output::render_json` 的 snake_case 输出保持一致，从而能直接反序列化子进程的
// JSON；对外刻意精简：
//   - **不含** `selected_indices`（脚本专用，反序列化时被 serde 自动忽略）；
//   - **不含** `channel`（MCP 客户端无需；子进程 JSON 里的 `channel` 作为未知字段被忽略）；
//   - `action` 仅在**取消**时出现（正常作答省略，见 `ask()` 里的归一化）。
// 结构体级注释用 `//`，避免泄漏进对外 schema。
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AskResult {
    /// Only present (value "cancel") when the human dismissed the request without answering;
    /// omitted on a normal answer (in which case `answers` carries the reply).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action: Option<String>,
    /// Guidance present only when the human cancelled: they dismissed the request without
    /// answering, so you MUST ask again and keep asking until they give an explicit reply.
    /// Never treat a cancel as approval or as permission to proceed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    /// One entry per answered question (questions left blank are omitted).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub answers: Vec<AskAnswer>,
}

// 单题作答。
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AskAnswer {
    /// Zero-based index of the question this answer refers to.
    pub question_index: usize,
    /// Labels of the options the human selected.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub selected_options: Vec<String>,
    /// Free text the human typed, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_input: Option<String>,
    /// Absolute paths of files the human attached (images and/or documents).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub files: Vec<String>,
}

/// MCP server：暴露 `ask`、`whats_next`、`todo_add`。
#[derive(Clone)]
pub struct AskServer {
    tool_router: ToolRouter<Self>,
}

#[tool_router(router = tool_router)]
impl AskServer {
    pub fn new() -> Self {
        Self {
            tool_router: Self::tool_router(),
        }
    }

    /// Ask the human a question (or several) and block until they reply.
    #[tool(
        name = "ask",
        description = "Ask the human operator one or more questions and wait (possibly for a long \
time) until they reply. Use this whenever you need a decision, clarification, review, approval, or \
any input that only the human can provide. Provide `message` for free-form questions, or \
`questions` for structured choices. Each `questions` item requires `question`; each nested \
`options` item requires `text` and may set `recommended` to true. Example: \
`{\"questions\":[{\"question\":\"Continue?\",\"options\":[{\"text\":\"Yes\",\"recommended\":true}]}]}`. \
The reply is returned as \
structured content; any images the human attaches are returned as image content.",
        output_schema = rmcp::handler::server::tool::schema_for_type::<AskResult>()
    )]
    async fn ask(
        &self,
        Parameters(params): Parameters<AskParams>,
        // rmcp extracts the per-request token; cancelled when the client sends
        // `notifications/cancelled` (timeout / user stop / host abort). Not human dismiss.
        cancel: CancellationToken,
    ) -> Result<CallToolResult, McpError> {
        let has_questions = params
            .questions
            .as_ref()
            .map(|q| !q.is_empty())
            .unwrap_or(false);
        let has_message = params
            .message
            .as_ref()
            .map(|m| !m.trim().is_empty())
            .unwrap_or(false);
        if !has_questions && !has_message {
            return Err(McpError::invalid_params(
                "ask requires `message` or at least one entry in `questions`",
                None,
            ));
        }

        let argv = build_argv(&params);
        let exe = std::env::current_exe().map_err(|e| {
            McpError::internal_error(format!("cannot locate AskHuman executable: {e}"), None)
        })?;

        // stdin 置空、stdout/stderr 捕获，确保子进程不碰 MCP 的 STDIO 协议流。
        // `ASKHUMAN_FROM_MCP=1`：告知子进程这是 MCP 发起，daemon 据此「只刷新、不新建」会话（防幽灵）。
        let mut command = tokio::process::Command::new(exe);
        command.args(&argv).env("ASKHUMAN_FROM_MCP", "1");
        let output = match capture_output(command, cancel).await {
            Ok(o) => o,
            Err(CaptureError::Cancelled) => {
                // Caller abort — not a human cancel. Do not invent answers or action:"cancel".
                return Ok(CallToolResult::error(vec![ContentBlock::text(
                    "AskHuman request was cancelled by the MCP client (not by the human).",
                )]));
            }
            Err(CaptureError::Io(e)) => {
                return Err(McpError::internal_error(
                    format!("failed to spawn AskHuman: {e}"),
                    None,
                ));
            }
        };

        let stdout = String::from_utf8_lossy(&output.stdout);
        let code = output.status.code().unwrap_or(3);

        // 子进程对 answer/cancel 都会输出合法 JSON；解析失败一般意味着系统级错误（如连不上 daemon），
        // 以 is_error 结果回报，把 stderr 透传给模型，不让其误以为人类作答。
        let value: Value = match serde_json::from_str(stdout.trim()) {
            Ok(v) => v,
            Err(_) => {
                let stderr = String::from_utf8_lossy(&output.stderr);
                let msg = if stderr.trim().is_empty() {
                    format!("AskHuman produced no result (exit code {code})")
                } else {
                    format!("AskHuman failed (exit code {code}): {}", stderr.trim())
                };
                return Ok(CallToolResult::error(vec![ContentBlock::text(msg)]));
            }
        };

        // 反序列化进 AskResult 会自动丢弃脚本专用的 `selected_indices` 与 `channel`（未知字段），
        // 再序列化即为对外的 structuredContent。
        let mut result: AskResult = serde_json::from_value(value).map_err(|e| {
            McpError::internal_error(format!("unexpected AskHuman output: {e}"), None)
        })?;
        // 正常作答不暴露 `action`（由 `answers` 表达）；仅取消时保留 `action:"cancel"` 作为信号。
        if result.action.as_deref() == Some("answer") {
            result.action = None;
        }
        let structured = serde_json::to_value(&result).map_err(|e| {
            McpError::internal_error(format!("failed to serialize ask result: {e}"), None)
        })?;

        // `structured()` 会把 structuredContent 同步序列化为 content[0] 的 JSON 文本，
        // 兼容尚不读 structuredContent 的客户端。
        let mut tool_result = CallToolResult::structured(structured);
        // 把人类附带的图片直接读回为 ImageContent（非图片文件仅以路径出现在 structuredContent 中）。
        for (path, mime) in image_files(&result) {
            match std::fs::read(&path) {
                Ok(bytes) => {
                    let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
                    tool_result
                        .content
                        .push(ContentBlock::image(b64, mime.to_string()));
                }
                // 读不到就跳过；路径仍在 structuredContent.answers[].files 中可供模型参考。
                Err(_) => continue,
            }
        }

        Ok(tool_result)
    }

    /// End-of-task handoff for requesting a separate next task.
    #[tool(
        name = "whats_next",
        description = "End-of-task handoff: ask the human for a separate next task. You MUST call \
this only after the current task is fully complete and before ending; use `ask` for any question, \
decision, or next step within the current task. Optionally pass `message` with a completion report \
and `files` with report documents. The human replies with the next task (start it immediately), or \
approves ending the turn — only then may you end it."
    )]
    async fn whats_next(
        &self,
        Parameters(params): Parameters<WhatsNextParams>,
        cancel: CancellationToken,
    ) -> Result<CallToolResult, McpError> {
        let argv = build_whats_next_argv(&params);
        let exe = std::env::current_exe().map_err(|e| {
            McpError::internal_error(format!("cannot locate AskHuman executable: {e}"), None)
        })?;
        let mut command = tokio::process::Command::new(exe);
        command.args(&argv).env("ASKHUMAN_FROM_MCP", "1");
        let output = match capture_output(command, cancel).await {
            Ok(o) => o,
            Err(CaptureError::Cancelled) => {
                return Ok(CallToolResult::error(vec![ContentBlock::text(
                    "AskHuman request was cancelled by the MCP client (not by the human).",
                )]));
            }
            Err(CaptureError::Io(e)) => {
                return Err(McpError::internal_error(
                    format!("failed to spawn AskHuman: {e}"),
                    None,
                ));
            }
        };
        // 结果就是一段纯文本（spec D3）：任务内容 / 固定结束句 / `[status]` 取消引导。
        let stdout = String::from_utf8_lossy(&output.stdout);
        let text = stdout.trim();
        if !output.status.success() || text.is_empty() {
            let code = output.status.code().unwrap_or(3);
            let stderr = String::from_utf8_lossy(&output.stderr);
            let msg = if stderr.trim().is_empty() {
                format!("AskHuman produced no result (exit code {code})")
            } else {
                format!("AskHuman failed (exit code {code}): {}", stderr.trim())
            };
            return Ok(CallToolResult::error(vec![ContentBlock::text(msg)]));
        }
        Ok(CallToolResult::success(vec![ContentBlock::text(
            text.to_string(),
        )]))
    }

    /// Append a project todo (same queue as CLI `AskHuman todo add`).
    /// Writes from the MCP server process directly into `todos.json`.
    #[tool(
        name = "todo_add",
        description = "Add a project todo for the human to pick up later (whats-next chips / todos \
window / IM). Use only when the human asked to record a deferred task or accepted a concrete \
suggestion for later — never for your own work plan. Attaches to the project of the MCP server's \
cwd (git root). Returns the 1-based index and stored text on success.",
        output_schema = rmcp::handler::server::tool::schema_for_type::<TodoAddResult>()
    )]
    async fn todo_add(
        &self,
        Parameters(params): Parameters<TodoAddParams>,
    ) -> Result<CallToolResult, McpError> {
        let text = params.text.trim();
        if text.is_empty() {
            return Err(McpError::invalid_params(
                "todo_add requires non-empty `text`",
                None,
            ));
        }
        let project = crate::project::detect();
        if project.is_empty() {
            return Err(McpError::internal_error(
                "cannot determine project (cwd unavailable)",
                None,
            ));
        }
        let auto = params.auto.unwrap_or(false);
        let agent = crate::agents::detect::detect_invoking_agent();
        let added = match agent {
            Some(agent) => crate::todos::add_from_agent(&project, text, auto, agent),
            None if auto => crate::todos::add_auto(&project, text),
            None => crate::todos::add(&project, text),
        };
        let entry = match added {
            Ok(entry) => entry,
            Err(crate::todos::AddError::EmptyInput) => {
                return Err(McpError::invalid_params(
                    "todo_add requires non-empty `text`",
                    None,
                ));
            }
            Err(crate::todos::AddError::Persist) => {
                return Ok(CallToolResult::error(vec![ContentBlock::text(
                    "Failed to save todo: could not write ~/.askhuman/state/todos.json \
(check permissions).",
                )]));
            }
        };
        let Some(index) = crate::todos::index_of(&project, &entry.id) else {
            return Ok(CallToolResult::error(vec![ContentBlock::text(
                "Failed to save todo: entry missing after write (not persisted).",
            )]));
        };
        let result = TodoAddResult {
            index,
            id: entry.id,
            text: entry.text,
            project,
            auto: entry.auto,
        };
        let structured = serde_json::to_value(&result).map_err(|e| {
            McpError::internal_error(format!("failed to serialize todo_add result: {e}"), None)
        })?;
        // `structured()` mirrors structuredContent into content[0] as JSON text.
        Ok(CallToolResult::structured(structured))
    }
}

/// Errors from [`capture_output`].
#[derive(Debug)]
enum CaptureError {
    /// MCP client cancelled the in-flight `tools/call` (`notifications/cancelled`).
    Cancelled,
    Io(std::io::Error),
}

/// 捕获子进程输出，并把 **rmcp request CancellationToken** 与 **future drop** 都传播为子进程终止。
///
/// - Token 取消（`notifications/cancelled`）：abort 持有 `Child` 的 task → `kill_on_drop` 杀进程 →
///   CLI socket EOF → daemon `wait_cli_eof` 取消 popup / IM。rmcp **不会** drop handler，故必须
///   在此 `select!` token（不能假设 future 被 drop）。
/// - Future drop（测试 abort / 极端宿主杀任务）：同一 `kill_on_drop(true)` 兜底。
///
/// 子进程放进独立 task，是因为 `wait_with_output` 会 move `Child`，无法在 `select!` 另一臂再 `kill`。
async fn capture_output(
    mut command: tokio::process::Command,
    cancel: CancellationToken,
) -> Result<Output, CaptureError> {
    command
        .kill_on_drop(true)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let child = command.spawn().map_err(CaptureError::Io)?;
    // AbortOnDropHandle: if this future is dropped without resolving (handler abort), the
    // child task is aborted → Child drop → kill_on_drop. Plain JoinHandle would detach.
    // Wrapped in Option so the cancel arm can take/drop it without fighting select! moves.
    let mut output_task = Some(AbortOnDropHandle::new(tokio::spawn(async move {
        child.wait_with_output().await
    })));

    tokio::select! {
        // Prefer cancel: if the client already abandoned the call, tear down even if the child
        // is about to exit with an orphaned answer nobody will read.
        biased;
        _ = cancel.cancelled() => {
            // Drop aborts the wait task → Child::drop → kill_on_drop → CLI socket EOF.
            drop(output_task.take());
            Err(CaptureError::Cancelled)
        }
        result = output_task.as_mut().unwrap() => {
            // Task finished (or aborted externally); disarm so Drop does not abort again.
            let _ = output_task.take();
            match result {
                Ok(Ok(output)) => Ok(output),
                Ok(Err(e)) => Err(CaptureError::Io(e)),
                Err(e) if e.is_cancelled() => Err(CaptureError::Cancelled),
                Err(e) => Err(CaptureError::Io(std::io::Error::other(e.to_string()))),
            }
        }
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for AskServer {
    fn get_info(&self) -> ServerInfo {
        let mut info = ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_instructions(
            "AskHuman bridges the agent and a human operator. Call the `ask` tool whenever you \
need the human to decide, clarify, review, or approve something; it blocks until they reply. \
Call the `whats_next` tool after completing the current task and before ending your turn to ask \
the human what to do next. Call the `todo_add` tool when the human asks to record a deferred \
project todo.",
        );
        // `from_build_env()` 的名字/版本来自 rmcp crate 自身，改成本应用的品牌名与版本。
        let mut implementation = Implementation::from_build_env();
        implementation.name = "AskHuman".to_string();
        implementation.version = env!("CARGO_PKG_VERSION").to_string();
        info.server_info = implementation;
        info
    }
}

/// 把 [`AskParams`] 翻译成 `AskHuman` 的 argv（不含程序名），末尾固定追加 `--output json`。
///
/// 纯函数，便于单测。注意：`message` 必须作为**首个**位置参数（CLI 只接受一个位置参数，且需在所有
/// `-q` 之前）。
fn build_argv(params: &AskParams) -> Vec<String> {
    let mut argv: Vec<String> = Vec::new();

    if let Some(message) = params.message.as_ref() {
        if !message.trim().is_empty() {
            argv.push(message.clone());
        }
    }

    if let Some(questions) = params.questions.as_ref() {
        for q in questions {
            argv.push("-q".to_string());
            argv.push(q.question.clone());
            if let Some(options) = q.options.as_ref() {
                for opt in options {
                    argv.push(if opt.recommended { "-o!" } else { "-o" }.to_string());
                    argv.push(opt.text.clone());
                }
            }
        }
    }

    if let Some(files) = params.files.as_ref() {
        for f in files {
            argv.push("-f".to_string());
            argv.push(f.clone());
        }
    }

    argv.push("--output".to_string());
    argv.push("json".to_string());
    argv
}

/// 把 [`WhatsNextParams`] 翻译成 `AskHuman --whats-next` 的 argv（纯函数，便于单测）。
/// 输出保持文本模式（结果本身就是一段纯文本，spec D3），不追加 `--output json`。
fn build_whats_next_argv(params: &WhatsNextParams) -> Vec<String> {
    let mut argv: Vec<String> = Vec::new();
    if let Some(message) = params.message.as_ref() {
        if !message.trim().is_empty() {
            argv.push(message.clone());
        }
    }
    argv.push("--whats-next".to_string());
    if let Some(options) = params.options.as_ref() {
        for option in options {
            argv.push(if option.recommended { "-o!" } else { "-o" }.to_string());
            argv.push(option.text.clone());
        }
    }
    if let Some(files) = params.files.as_ref() {
        for f in files {
            argv.push("-f".to_string());
            argv.push(f.clone());
        }
    }
    argv
}

/// 从结果中挑出「可作为 MCP 图片直接返回」的文件，返回 (路径, MIME) 列表。
fn image_files(result: &AskResult) -> Vec<(PathBuf, &'static str)> {
    let mut out = Vec::new();
    for ans in &result.answers {
        for f in &ans.files {
            let path = PathBuf::from(f);
            if let Some(mime) = image_mime(&path) {
                out.push((path, mime));
            }
        }
    }
    out
}

/// 按扩展名判断图片 MIME；非图片返回 `None`。
fn image_mime(path: &Path) -> Option<&'static str> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    Some(match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "bmp" => "image/bmp",
        "svg" => "image/svg+xml",
        "ico" => "image/x-icon",
        "tif" | "tiff" => "image/tiff",
        "heic" => "image/heic",
        "heif" => "image/heif",
        "avif" => "image/avif",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[cfg(unix)]
    use std::time::Duration;

    fn contains_ref(value: &Value) -> bool {
        match value {
            Value::Object(object) => {
                object.contains_key("$ref") || object.values().any(contains_ref)
            }
            Value::Array(items) => items.iter().any(contains_ref),
            _ => false,
        }
    }

    fn params(json: Value) -> AskParams {
        serde_json::from_value(json).unwrap()
    }

    #[cfg(unix)]
    async fn wait_for_pid_file(pid_file: &std::path::Path) -> i32 {
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if let Ok(text) = std::fs::read_to_string(pid_file) {
                    if let Ok(pid) = text.trim().parse::<i32>() {
                        break pid;
                    }
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("child should publish its pid")
    }

    #[cfg(unix)]
    async fn wait_until_dead(pid: i32) {
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                // kill(pid, 0) == -1 with ESRCH means the child no longer exists.
                let alive = unsafe { libc::kill(pid, 0) } == 0;
                if !alive {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("child process must exit")
    }

    /// rmcp 真实取消路径：cancel token（对应 `notifications/cancelled`）→ 杀子进程。
    #[cfg(unix)]
    #[tokio::test]
    async fn cancelled_token_kills_child() {
        let dir = tempfile::tempdir().unwrap();
        let pid_file = dir.path().join("pid");
        let script = format!("echo $$ > '{}'; exec sleep 60", pid_file.display());
        let mut command = tokio::process::Command::new("sh");
        command.args(["-c", &script]);

        let cancel = CancellationToken::new();
        let task = tokio::spawn(capture_output(command, cancel.clone()));
        let pid = wait_for_pid_file(&pid_file).await;

        cancel.cancel();
        let result = task.await.expect("capture_output task");
        assert!(
            matches!(result, Err(CaptureError::Cancelled)),
            "token cancel must yield CaptureError::Cancelled, got {result:?}"
        );

        wait_until_dead(pid).await;
    }

    /// 兜底：abort/drop future 时 kill_on_drop 仍杀子进程（宿主强杀任务等）。
    #[cfg(unix)]
    #[tokio::test]
    async fn cancelled_output_future_kills_child() {
        let dir = tempfile::tempdir().unwrap();
        let pid_file = dir.path().join("pid");
        let script = format!("echo $$ > '{}'; exec sleep 60", pid_file.display());
        let mut command = tokio::process::Command::new("sh");
        command.args(["-c", &script]);

        let task = tokio::spawn(capture_output(command, CancellationToken::new()));
        let pid = wait_for_pid_file(&pid_file).await;

        task.abort();
        let _ = task.await;

        wait_until_dead(pid).await;
    }

    #[test]
    fn argv_message_only_becomes_question() {
        let p = params(json!({ "message": "Continue?" }));
        assert_eq!(build_argv(&p), vec!["Continue?", "--output", "json"]);
    }

    #[test]
    fn argv_full_with_recommended_and_files() {
        let p = params(json!({
            "message": "Pick an env",
            "questions": [{
                "question": "Which environment?",
                "options": [
                    { "text": "production", "recommended": true },
                    { "text": "staging" }
                ]
            }],
            "files": ["/tmp/a.png"]
        }));
        assert_eq!(
            build_argv(&p),
            vec![
                "Pick an env",
                "-q",
                "Which environment?",
                "-o!",
                "production",
                "-o",
                "staging",
                "-f",
                "/tmp/a.png",
                "--output",
                "json",
            ]
        );
    }

    #[test]
    fn whats_next_argv_message_stays_message() {
        // Message 是完成报告，不追加 --output json（结果为纯文本，spec D3）。
        let p: WhatsNextParams =
            serde_json::from_value(json!({ "message": "All tests pass." })).unwrap();
        assert_eq!(
            build_whats_next_argv(&p),
            vec!["All tests pass.", "--whats-next"]
        );
    }

    #[test]
    fn whats_next_argv_includes_suggested_options() {
        let p: WhatsNextParams = serde_json::from_value(json!({
            "message": "All tests pass.",
            "options": [
                { "text": "Write docs" },
                { "text": "Add tests", "recommended": true }
            ]
        }))
        .unwrap();
        assert_eq!(
            build_whats_next_argv(&p),
            vec![
                "All tests pass.",
                "--whats-next",
                "-o",
                "Write docs",
                "-o!",
                "Add tests",
            ]
        );
    }

    #[test]
    fn whats_next_argv_bare_and_with_files() {
        let p: WhatsNextParams = serde_json::from_value(json!({})).unwrap();
        assert_eq!(build_whats_next_argv(&p), vec!["--whats-next"]);
        let p: WhatsNextParams =
            serde_json::from_value(json!({ "files": ["/tmp/report.md"] })).unwrap();
        assert_eq!(
            build_whats_next_argv(&p),
            vec!["--whats-next", "-f", "/tmp/report.md"]
        );
    }

    #[test]
    fn whats_next_tool_is_registered() {
        let server = AskServer::new();
        assert!(server.tool_router.get("whats_next").is_some());
    }

    #[test]
    fn todo_add_tool_is_registered() {
        let server = AskServer::new();
        assert!(server.tool_router.get("todo_add").is_some());
        let tool = server.tool_router.get("todo_add").unwrap();
        let schema = Value::Object((*tool.input_schema).clone());
        assert_eq!(
            schema.pointer("/properties/text/type"),
            Some(&json!("string"))
        );
        // `Option<bool>` → JSON Schema `["boolean","null"]` (or boolean depending on rmcp/schemars).
        let auto_type = schema.pointer("/properties/auto/type").cloned();
        assert!(
            auto_type == Some(json!("boolean")) || auto_type == Some(json!(["boolean", "null"])),
            "unexpected auto type: {auto_type:?}"
        );
        assert!(tool
            .description
            .as_deref()
            .unwrap_or("")
            .contains("never for your own work plan"));
    }

    #[test]
    fn whats_next_schema_exposes_inline_suggested_options() {
        let server = AskServer::new();
        let tool = server.tool_router.get("whats_next").unwrap();
        let description = tool.description.as_deref().unwrap();
        assert!(description.contains("End-of-task handoff"));
        assert!(description.contains("use `ask` for any question"));
        let schema = Value::Object((*tool.input_schema).clone());
        let options_description = schema
            .pointer("/properties/options/description")
            .and_then(Value::as_str)
            .unwrap();
        assert!(options_description.contains("NEVER include an end, stop, or no-more-work option"));
        assert!(options_description.contains("AskHuman adds the ending choice"));
        assert_eq!(
            schema.pointer("/properties/options/items/type"),
            Some(&json!("object"))
        );
        assert_eq!(
            schema.pointer("/properties/options/items/properties/text/type"),
            Some(&json!("string"))
        );
        assert_eq!(
            schema.pointer("/properties/options/items/properties/recommended/type"),
            Some(&json!("boolean"))
        );
    }

    #[test]
    fn ask_tool_schema_inlines_question_and_option_fields() {
        let server = AskServer::new();
        let tool = server.tool_router.get("ask").unwrap();
        let schema = Value::Object((*tool.input_schema).clone());

        assert!(
            !contains_ref(&schema),
            "ask input schema must not expose $ref"
        );
        assert!(schema.get("$defs").is_none());
        assert_eq!(
            schema.pointer("/properties/questions/items/type"),
            Some(&json!("object"))
        );
        assert_eq!(
            schema.pointer("/properties/questions/items/properties/question/type"),
            Some(&json!("string"))
        );
        assert_eq!(
            schema.pointer("/properties/questions/items/required"),
            Some(&json!(["question"]))
        );
        assert_eq!(
            schema.pointer("/properties/questions/items/properties/options/items/type"),
            Some(&json!("object"))
        );
        assert_eq!(
            schema.pointer(
                "/properties/questions/items/properties/options/items/properties/text/type"
            ),
            Some(&json!("string"))
        );
        assert_eq!(
            schema.pointer("/properties/questions/items/properties/options/items/required"),
            Some(&json!(["text"]))
        );
    }

    /// 模拟 `ask()` 对子进程 JSON 的归一化：反序列化 + 正常作答清空 `action`。
    fn normalize(child: Value) -> Value {
        let mut result: AskResult = serde_json::from_value(child).unwrap();
        if result.action.as_deref() == Some("answer") {
            result.action = None;
        }
        serde_json::to_value(&result).unwrap()
    }

    #[test]
    fn result_answer_drops_channel_action_and_selected_indices() {
        // 模拟 render_json 的输出形态（含脚本专用 selected_indices + channel）。
        let out = normalize(json!({
            "action": "answer",
            "channel": "popup",
            "answers": [{
                "question_index": 0,
                "selected_options": ["production"],
                "selected_indices": [1],
                "user_input": "go",
                "files": ["/tmp/a.png"]
            }]
        }));
        assert_eq!(out["answers"][0]["question_index"], 0);
        assert_eq!(out["answers"][0]["selected_options"][0], "production");
        assert_eq!(out["answers"][0]["user_input"], "go");
        // 对外精简：正常作答不带 action，且从不带 channel；selected_indices 永远剔除。
        assert!(out.get("action").is_none());
        assert!(out.get("channel").is_none());
        assert!(out["answers"][0].get("selected_indices").is_none());
    }

    #[test]
    fn result_cancel_keeps_action_drops_channel() {
        let out = normalize(json!({ "action": "cancel", "channel": "popup" }));
        assert_eq!(out["action"], "cancel");
        assert!(out.get("channel").is_none());
        assert!(out.get("answers").is_none());
    }

    #[test]
    fn result_cancel_passes_through_status() {
        // 子进程 render_json 取消时带 status 引导；薄壳应原样透传到 structuredContent。
        let out = normalize(json!({
            "action": "cancel",
            "channel": "popup",
            "status": "The human cancelled. You must ask again."
        }));
        assert_eq!(out["status"], "The human cancelled. You must ask again.");
    }

    #[test]
    fn result_answer_omits_status() {
        let out = normalize(json!({ "action": "answer", "channel": "popup" }));
        assert!(out.get("status").is_none());
    }

    #[test]
    fn image_files_filters_by_extension() {
        let result = AskResult {
            action: None,
            status: None,
            answers: vec![AskAnswer {
                question_index: 0,
                selected_options: vec![],
                user_input: None,
                files: vec![
                    "/tmp/a.PNG".into(),
                    "/tmp/notes.md".into(),
                    "/tmp/b.jpeg".into(),
                ],
            }],
        };
        let imgs = image_files(&result);
        assert_eq!(imgs.len(), 2);
        assert_eq!(imgs[0].1, "image/png");
        assert_eq!(imgs[1].1, "image/jpeg");
    }
}
