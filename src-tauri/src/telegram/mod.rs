//! Telegram Bot API 客户端（手写 reqwest）。

pub mod markdown;

use serde_json::{json, Value};
use std::fmt;
use std::time::Duration;

#[derive(Debug)]
pub enum TelegramError {
    EmptyToken,
    EmptyChatId,
    InvalidChatId,
    Api(String),
    Network(String),
    BadResponse,
}

impl fmt::Display for TelegramError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TelegramError::EmptyToken => write!(f, "Bot Token 不能为空"),
            TelegramError::EmptyChatId => write!(f, "Chat ID 不能为空"),
            TelegramError::InvalidChatId => write!(f, "Chat ID 格式无效，请输入有效的数字 ID"),
            TelegramError::Api(msg) => write!(f, "Telegram API 错误: {}", msg),
            TelegramError::Network(msg) => write!(f, "网络错误: {}", msg),
            TelegramError::BadResponse => write!(f, "无法解析 Telegram 响应"),
        }
    }
}

impl std::error::Error for TelegramError {}

pub struct TelegramClient {
    token: String,
    chat_id: i64,
    api_base_url: String,
    http: reqwest::Client,
}

impl TelegramClient {
    pub fn new(
        token: String,
        chat_id_string: String,
        api_base_url: String,
    ) -> Result<Self, TelegramError> {
        let token = token.trim().to_string();
        let chat = chat_id_string.trim().to_string();
        if token.is_empty() {
            return Err(TelegramError::EmptyToken);
        }
        if chat.is_empty() {
            return Err(TelegramError::EmptyChatId);
        }
        if chat.starts_with('@') {
            return Err(TelegramError::InvalidChatId);
        }
        let chat_id: i64 = chat.parse().map_err(|_| TelegramError::InvalidChatId)?;
        let base = api_base_url.trim();
        let api_base_url = if base.is_empty() {
            "https://api.telegram.org".to_string()
        } else {
            base.to_string()
        };
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|e| TelegramError::Network(e.to_string()))?;
        Ok(Self {
            token,
            chat_id,
            api_base_url,
            http,
        })
    }

    pub fn chat_id(&self) -> i64 {
        self.chat_id
    }

    /// 调用某方法，返回 `result` 字段（成功）或错误（`ok=false`/网络/解析）。
    async fn call(&self, method: &str, params: Value) -> Result<Value, TelegramError> {
        let url = format!("{}/bot{}/{}", self.api_base_url, self.token, method);
        let resp = self
            .http
            .post(&url)
            .json(&params)
            .send()
            .await
            .map_err(|e| TelegramError::Network(e.to_string()))?;
        let v: Value = resp.json().await.map_err(|_| TelegramError::BadResponse)?;
        if v.get("ok").and_then(|o| o.as_bool()) == Some(true) {
            Ok(v.get("result").cloned().unwrap_or(Value::Null))
        } else {
            let desc = v
                .get("description")
                .and_then(|d| d.as_str())
                .unwrap_or("请求失败")
                .to_string();
            Err(TelegramError::Api(desc))
        }
    }

    /// 发送消息，返回 message_id。
    pub async fn send_message(
        &self,
        text: &str,
        parse_mode: Option<&str>,
        reply_markup: Option<Value>,
    ) -> Result<i64, TelegramError> {
        let mut params = serde_json::Map::new();
        params.insert("chat_id".into(), json!(self.chat_id));
        params.insert("text".into(), json!(text));
        if let Some(pm) = parse_mode {
            params.insert("parse_mode".into(), json!(pm));
        }
        if let Some(rm) = reply_markup {
            params.insert("reply_markup".into(), rm);
        }
        let result = self.call("sendMessage", Value::Object(params)).await?;
        Ok(result
            .get("message_id")
            .and_then(|m| m.as_i64())
            .unwrap_or(0))
    }

    /// 上传文件（multipart）。`method` 为 sendDocument/sendPhoto，`field` 为 document/photo。
    async fn send_file(
        &self,
        method: &str,
        field: &str,
        path: &str,
        filename: &str,
    ) -> Result<i64, TelegramError> {
        let bytes = std::fs::read(path)
            .map_err(|e| TelegramError::Network(format!("读取文件失败: {}", e)))?;
        let part = reqwest::multipart::Part::bytes(bytes).file_name(filename.to_string());
        let form = reqwest::multipart::Form::new()
            .text("chat_id", self.chat_id.to_string())
            .part(field.to_string(), part);
        let url = format!("{}/bot{}/{}", self.api_base_url, self.token, method);
        let resp = self
            .http
            .post(&url)
            .multipart(form)
            .send()
            .await
            .map_err(|e| TelegramError::Network(e.to_string()))?;
        let v: Value = resp.json().await.map_err(|_| TelegramError::BadResponse)?;
        if v.get("ok").and_then(|o| o.as_bool()) == Some(true) {
            Ok(v.get("result")
                .and_then(|r| r.get("message_id"))
                .and_then(|m| m.as_i64())
                .unwrap_or(0))
        } else {
            let desc = v
                .get("description")
                .and_then(|d| d.as_str())
                .unwrap_or("请求失败")
                .to_string();
            Err(TelegramError::Api(desc))
        }
    }

    /// 以文档形式发送文件。
    pub async fn send_document(&self, path: &str, filename: &str) -> Result<i64, TelegramError> {
        self.send_file("sendDocument", "document", path, filename).await
    }

    /// 以图片形式发送文件（可内联预览）。
    pub async fn send_photo(&self, path: &str, filename: &str) -> Result<i64, TelegramError> {
        self.send_file("sendPhoto", "photo", path, filename).await
    }

    pub async fn get_updates(&self, offset: i64) -> Result<Vec<Value>, TelegramError> {
        let result = self
            .call("getUpdates", json!({ "offset": offset, "timeout": 0 }))
            .await?;
        Ok(result.as_array().cloned().unwrap_or_default())
    }

    pub async fn answer_callback_query(&self, id: &str) {
        let _ = self
            .call("answerCallbackQuery", json!({ "callback_query_id": id }))
            .await;
    }

    pub async fn edit_message_reply_markup(&self, message_id: i64, markup: Value) {
        let _ = self
            .call(
                "editMessageReplyMarkup",
                json!({ "chat_id": self.chat_id, "message_id": message_id, "reply_markup": markup }),
            )
            .await;
    }

    /// 发送测试消息验证配置。
    pub async fn test_connection(&self) -> Result<String, TelegramError> {
        let text = "🤖 HumanInLoop 测试消息\n\n这是一条测试消息，表示 Telegram Bot 配置成功！";
        self.send_message(text, None, None).await?;
        Ok("测试消息发送成功！Telegram Bot 配置正确。".to_string())
    }
}
