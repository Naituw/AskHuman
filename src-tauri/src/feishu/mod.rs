//! 飞书（Feishu / Lark）OpenAPI / 长连接客户端层。
//!
//! 形态：企业自建应用 + 机器人 + 长连接(WebSocket)模式 + 单聊。
//! - 鉴权：`token`（tenant_access_token 缓存）。
//! - 发送：`client`（单聊文本/图片/文件、互动卡片 JSON、媒体上传、消息资源下载、卡片更新）。
//! - 卡片：`card`（卡片 JSON 2.0 组装 + card.action.trigger 回调解析）。
//! - 接收：`ws`（长连接：protobuf 帧 pbbp2；事件 im.message.receive_v1 + 卡片回调 card.action.trigger）。
//!
//! 与钉钉差异：长连接帧是 protobuf（非 JSON），且订阅 topic 由开发者后台配置（建连不声明 topic）。

pub mod card;
pub mod client;
pub mod router;
pub mod token;
pub mod ws;

use std::fmt;

#[derive(Debug)]
pub enum FeishuError {
    /// 配置缺失（附字段名提示）。
    EmptyConfig(String),
    /// 飞书接口返回业务错误；保留数值 code 供鉴权恢复等逻辑可靠判断。
    Api { code: Option<i64>, message: String },
    /// 网络错误。
    Network(String),
    /// 响应无法解析。
    BadResponse,
}

// 源语言(英文) Display：日志/技术细节统一英文；GUI 边界用 `localized()` 取本地化文案。
impl fmt::Display for FeishuError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FeishuError::EmptyConfig(field) => write!(f, "{} must not be empty", field),
            FeishuError::Api { message, .. } => write!(f, "Feishu API error: {}", message),
            FeishuError::Network(msg) => write!(f, "network error: {}", msg),
            FeishuError::BadResponse => write!(f, "failed to parse Feishu response"),
        }
    }
}

impl std::error::Error for FeishuError {}

impl FeishuError {
    pub(crate) fn api(code: Option<i64>, message: impl Into<String>) -> Self {
        Self::Api {
            code,
            message: message.into(),
        }
    }

    /// These codes identify an invalid tenant token for APIs that AskHuman always calls with a
    /// tenant token. Keep this code-based: Feishu documents `msg` as unstable display text.
    pub(crate) fn is_invalid_tenant_token(&self) -> bool {
        matches!(
            self,
            Self::Api {
                code: Some(99991663 | 99991665),
                ..
            }
        )
    }

    /// GUI 可见的本地化文案：校验类按界面语言翻译；技术细节(API/网络/解析)保留英文。
    pub fn localized(&self, lang: crate::i18n::Lang) -> String {
        match self {
            FeishuError::EmptyConfig(field) => {
                crate::i18n::tr(lang, "err.fsEmptyConfig").replace("{field}", field)
            }
            _ => self.to_string(),
        }
    }
}
