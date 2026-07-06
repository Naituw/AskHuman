//! Slack 客户端层。
//!
//! 形态：Slack App + Socket Mode + 机器人 + 单聊（DM）。
//! - 发送：`client`（Web API：chat.postMessage / chat.update / conversations.open /
//!   files.getUploadURLExternal + completeUploadExternal / 文件下载 / auth.test）。
//! - 接收：`ws`（Socket Mode：JSON 帧；events_api 消息 + interactive 交互；收帧即 ack）。
//! - 卡片：`blockkit`（Block Kit 消息内表单：复选框 + 多行输入框 + 提交按钮，提交读 state.values）。
//! - Markdown：`markdown`（标准 Markdown → Slack mrkdwn）。
//! - 路由：`router`（进程内独占一条 Socket Mode 连接，按 message_ts / user_id 分发到各会话）。
//!
//! 与飞书差异：帧是 JSON（非 protobuf），且 ack（回 envelope_id）与卡片更新（chat.update）解耦，
//! 故 Router 收帧即 ack，无需飞书那样的「延迟回包」oneshot。

pub mod blockkit;
pub mod client;
pub mod markdown;
pub mod router;
pub mod select;
pub mod watch;
pub mod ws;

use std::fmt;

/// Slack Web API base (`https://slack.com/api`). Overridable via `ASKHUMAN_SLACK_API_BASE` for
/// tests/CI (the perf harness points it at a local mock IM); unset → the real endpoint, so
/// production behaviour is unchanged. Covers Web API calls and Socket Mode `apps.connections.open`.
pub fn api_base() -> String {
    std::env::var("ASKHUMAN_SLACK_API_BASE")
        .ok()
        .map(|v| v.trim().trim_end_matches('/').to_string())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| "https://slack.com/api".to_string())
}

#[derive(Debug)]
pub enum SlackError {
    /// 配置缺失（附字段名提示）。
    EmptyConfig(String),
    /// Slack 接口返回业务错误（`ok=false` 时的 `error`）。
    Api(String),
    /// 网络错误。
    Network(String),
    /// 响应无法解析。
    BadResponse,
}

// 源语言(英文) Display：日志/技术细节统一英文；GUI 边界用 `localized()` 取本地化文案。
impl fmt::Display for SlackError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SlackError::EmptyConfig(field) => write!(f, "{} must not be empty", field),
            SlackError::Api(msg) => write!(f, "Slack API error: {}", msg),
            SlackError::Network(msg) => write!(f, "network error: {}", msg),
            SlackError::BadResponse => write!(f, "failed to parse Slack response"),
        }
    }
}

impl std::error::Error for SlackError {}

impl SlackError {
    /// GUI 可见的本地化文案：校验类按界面语言翻译；技术细节(API/网络/解析)保留英文。
    pub fn localized(&self, lang: crate::i18n::Lang) -> String {
        match self {
            SlackError::EmptyConfig(field) => {
                crate::i18n::tr(lang, "err.slEmptyConfig").replace("{field}", field)
            }
            _ => self.to_string(),
        }
    }
}
