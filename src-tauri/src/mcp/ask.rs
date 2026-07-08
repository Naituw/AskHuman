//! `ask` 工具：把 MCP 入参翻译成 `AskHuman --output json …` argv，spawn 子进程复用既有 ask 流程，
//! 再把子进程的 JSON 结果整理成 MCP `structuredContent` + `TextContent`，并将人类回复中的图片读回为
//! `ImageContent` 一并返回。
//!
//! 关键点：
//! - 子进程用 `Command::output()` 运行 —— stdin 被置空、stdout/stderr 被捕获，因此**不会**污染本
//!   server 的 STDIO MCP 协议流。
//! - 子进程的 JSON 含脚本专用的 `selected_indices`；反序列化进 [`AskResult`]（无该字段）即自动丢弃，
//!   再重新序列化为 `structuredContent`，对 MCP 客户端不暴露该字段。

use base64::Engine;
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, Content, Implementation, ServerCapabilities, ServerInfo};
use rmcp::{tool, tool_handler, tool_router, ErrorData as McpError, ServerHandler};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Path, PathBuf};

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

// 单个问题。
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct AskQuestion {
    /// The question text.
    pub question: String,
    /// Optional predefined options the human may pick from.
    #[serde(default)]
    pub options: Option<Vec<AskOption>>,
}

// 单个选项。
#[derive(Debug, Clone, Deserialize, JsonSchema)]
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

/// MCP server：仅暴露 `ask` 一个工具。
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
`questions` (each with optional `options`) for structured choices. The reply is returned as \
structured content; any images the human attaches are returned as image content.",
        output_schema = rmcp::handler::server::tool::schema_for_type::<AskResult>()
    )]
    async fn ask(
        &self,
        Parameters(params): Parameters<AskParams>,
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

        // `output()` 置空 stdin、捕获 stdout/stderr，确保子进程不碰 MCP 的 STDIO 协议流。
        // `ASKHUMAN_FROM_MCP=1`：告知子进程这是 MCP 发起，daemon 据此「只刷新、不新建」会话（防幽灵）。
        let output = tokio::task::spawn_blocking(move || {
            std::process::Command::new(exe)
                .args(&argv)
                .env("ASKHUMAN_FROM_MCP", "1")
                .output()
        })
        .await
        .map_err(|e| McpError::internal_error(format!("ask task failed to join: {e}"), None))?
        .map_err(|e| McpError::internal_error(format!("failed to spawn AskHuman: {e}"), None))?;

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
                return Ok(CallToolResult::error(vec![Content::text(msg)]));
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
                        .push(Content::image(b64, mime.to_string()));
                }
                // 读不到就跳过；路径仍在 structuredContent.answers[].files 中可供模型参考。
                Err(_) => continue,
            }
        }

        Ok(tool_result)
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for AskServer {
    fn get_info(&self) -> ServerInfo {
        let mut info = ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_instructions(
            "AskHuman bridges the agent and a human operator. Call the `ask` tool whenever you \
need the human to decide, clarify, review, or approve something; it blocks until they reply.",
        );
        // `from_build_env()` 的名字/版本来自 rmcp crate（"rmcp"/"1.7.0"），改成本应用的品牌名与版本。
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

    fn params(json: Value) -> AskParams {
        serde_json::from_value(json).unwrap()
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
