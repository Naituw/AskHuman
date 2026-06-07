//! 进程间通信（IPC）：CLI / GUI Helper ↔ 常驻 Daemon。
//!
//! 传输：NDJSON（一行一个 JSON 消息）over Unix domain socket（mac/Linux）/ Windows named pipe。
//! 本文件定义协议消息类型；编解码见 `codec`，传输（socket 路径/连接/监听）见 `transport`。
//!
//! Phase 0 仅含握手与 daemon 控制（status/stop）；任务提交（submit/show/...）在后续 Phase 引入。

pub mod codec;
pub mod transport;

pub use codec::{read_msg, write_msg};

use crate::daemon::lifecycle::Fingerprint;
use crate::models::{AskRequest, ChannelAction, MessagePrompt, Question, QuestionAnswer};
use serde::{Deserialize, Serialize};

/// IPC 协议版本：不兼容变更时 +1，握手不一致即触发换新。
pub const PROTOCOL_VERSION: u32 = 1;

/// CLI/GUI 连接时的握手信息。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientHello {
    pub protocol_version: u32,
    pub client_version: String,
    pub binary_path: String,
    pub fingerprint: Fingerprint,
    pub pid: u32,
}

/// 握手结果状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum HelloStatus {
    /// 正常，可继续。
    Ok,
    /// Daemon 已过时（二进制指纹/协议变化），将自行退出；客户端应等其下线后用新二进制拉起。
    Restarting,
}

/// 对 `ClientHello` 的回应。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HelloAck {
    pub protocol_version: u32,
    pub daemon_version: String,
    pub status: HelloStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// `daemon status` 返回的运行信息。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StatusInfo {
    pub pid: u32,
    pub version: String,
    pub protocol_version: u32,
    pub uptime_secs: u64,
    pub socket: String,
    pub active_requests: usize,
    /// 当前常热的 IM 长连接（"dingtalk" / "feishu" / "telegram"），按已建连且存活计入。
    #[serde(default)]
    pub im_connections: Vec<String>,
}

/// CLI 提交的一次提问任务（A11：`-f` 已在 CLI 解析为绝对路径；硬性上送 source name 与解析好的 lang；
/// `request_id` 由 Daemon 分配，故此处不含 id）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskRequest {
    /// 共享 Message：描述文本与展示附件（绝对路径）。
    pub message: MessagePrompt,
    /// 问题列表（CLI 已归一化，恒 ≥1）。
    pub questions: Vec<Question>,
    /// 是否按 Markdown 渲染（全局）。
    pub is_markdown: bool,
    /// 调用方来源名（来自 `ASKHUMAN_ENV_SOURCE_NAME`，CLI 读取后上送）。
    pub source: String,
    /// CLI 解析好的界面语言（"en" / "zh"），使 `auto` 跟随调用方而非 Daemon。
    pub lang: String,
    /// 当前项目 key（CLI 计算：向上找 .git 根、回退 cwd），用于回复历史归类。
    /// 旧 CLI 不带此字段 → 默认空串（归入「未知项目」）。
    #[serde(default)]
    pub project: String,
}

/// 自动识别 userId/open_id 请求（设置进程 → Daemon，Q6）：用表单当前凭据，
/// 等用户私聊机器人发送识别码后返回其 id。Daemon 若已有同 `app_key` 的活动长连接则**观察现有连接**
/// （零冲突），否则自行临时开一条连接完成识别。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DetectRequest {
    /// 渠道类型："dingtalk" | "feishu"。
    pub kind: String,
    /// 钉钉 client_id / 飞书 app_id（也是「是否复用现有连接」的匹配键）。
    pub app_key: String,
    /// 钉钉 client_secret / 飞书 app_secret。
    pub app_secret: String,
    /// 飞书自定义 base_url（钉钉忽略，可传空）。
    pub base_url: String,
    /// 用户需私聊发送的识别码。
    pub code: String,
    /// 设置进程解析好的界面语言（"en" / "zh"），供 Daemon 本地化超时/断连等提示。
    pub lang: String,
}

/// Daemon → GUI Helper 的题目下发（show 是 submit 的子集 + Daemon 分配的 request_id + 上下文）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShowPayload {
    pub request_id: String,
    /// 完整请求（含 Daemon 分配的 id），供弹窗渲染。
    pub request: AskRequest,
    /// 调用方来源名（弹窗标题「Question from {source}」）。
    pub source: String,
    /// 界面语言（"en" / "zh"）。
    pub lang: String,
    /// 当前项目 key（供历史窗口默认过滤当前项目）。
    #[serde(default)]
    pub project: String,
}

/// 客户端（CLI / GUI Helper）→ Daemon 的消息。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ClientMsg {
    /// CLI / 控制连接握手。
    Hello(ClientHello),
    /// `daemon status`。
    Status,
    /// `daemon stop`。
    Stop,
    /// CLI 提交一次提问任务（握手后发送）。
    Submit(TaskRequest),
    /// GUI Helper 握手：出示 Daemon 下发的一次性 token。
    GuiHello { token: String },
    /// 设置进程请求「自动识别 userId/open_id」（Q6）。握手后发送，阻塞等单个结果。
    Detect(DetectRequest),
    /// GUI Helper 回传用户作答（`action` 区分发送/取消）。
    Answer {
        request_id: String,
        action: ChannelAction,
        #[serde(default)]
        answers: Vec<QuestionAnswer>,
    },
}

/// Daemon → 客户端（CLI / GUI Helper）的消息。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ServerMsg {
    HelloAck(HelloAck),
    Status(StatusInfo),
    Stopping,
    Error { message: String },
    /// 任务已受理，回带 Daemon 分配的 request_id（D→CLI）。
    Accepted { request_id: String },
    /// 流式警告 / 诊断 → CLI 的 stderr（D→CLI）。
    Warn { text: String },
    /// 终态：渲染好的结果文本 + 退出码（D→CLI）。CLI 原样打印 stdout 后按码退出。
    Final { stdout: String, exit_code: i32 },
    /// 自动识别成功，回带识别出的 userId/open_id（D→设置进程，Q6）。失败用 `Error`。
    Detected { id: String },
    /// 下发题目（D→GUI）。
    Show(ShowPayload),
    /// 被其它渠道抢答，通知 GUI 收尾关窗（D→GUI）。
    Cancel { request_id: String, winner: String },
    /// 配置实时变更，下发新的 `general` 配置给活动 GUI Helper 以即时切主题/语言（D→GUI，A12）。
    ConfigChanged { general: serde_json::Value },
}
