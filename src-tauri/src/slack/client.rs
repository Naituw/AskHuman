//! Slack 单聊机器人 Web API 客户端（reqwest）。
//!
//! 鉴权：Web API 统一用 Bot Token（`xoxb-…`，Bearer）；Socket Mode 建连用 App Token（见 `ws`）。
//! 发消息用 `chat.postMessage`（Block Kit）；DM 频道由 `conversations.open` 解析。
//! 文件上传走新版三步流程（`files.getUploadURLExternal` + `files.completeUploadExternal`）。

use super::SlackError;
use crate::config::SlackChannelConfig;
use serde_json::{json, Value};
use std::time::Duration;

/// 等待文件分享进时间线的最长时长（实测约数秒，留足余量）。
const FILE_SHARE_TIMEOUT: Duration = Duration::from_secs(15);
/// 轮询 `files.info` 的间隔。
const FILE_SHARE_POLL_INTERVAL: Duration = Duration::from_millis(400);

/// 渠道健康登记（R7）：API 调用失败登记、成功清除。放在统一出口，覆盖发送/编辑/上传全部路径。
fn track<T>(r: Result<T, SlackError>) -> Result<T, SlackError> {
    match &r {
        Ok(_) => crate::channels::health::clear("slack"),
        Err(e) => crate::channels::health::report("slack", e.to_string()),
    }
    r
}

#[derive(Clone)]
pub struct SlackClient {
    bot_token: String,
    app_token: String,
    user_id: String,
    http: reqwest::Client,
}

impl SlackClient {
    /// 构造客户端：校验 Bot/App Token（user_id 允许为空，自动识别流程不需要）。
    pub fn new(config: &SlackChannelConfig) -> Result<Self, SlackError> {
        let bot_token = config.bot_token.trim().to_string();
        let app_token = config.app_token.trim().to_string();
        if bot_token.is_empty() {
            return Err(SlackError::EmptyConfig("Bot Token".into()));
        }
        if app_token.is_empty() {
            return Err(SlackError::EmptyConfig("App Token".into()));
        }
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(60))
            .build()
            .map_err(|e| SlackError::Network(e.to_string()))?;
        Ok(Self {
            bot_token,
            app_token,
            user_id: config.user_id.trim().to_string(),
            http,
        })
    }

    pub fn http(&self) -> &reqwest::Client {
        &self.http
    }

    pub fn bot_token(&self) -> &str {
        &self.bot_token
    }

    pub fn app_token(&self) -> &str {
        &self.app_token
    }

    pub fn user_id(&self) -> &str {
        &self.user_id
    }

    /// 通用 JSON 调用：Bot Token Bearer + `ok==true` 判定成功，失败取 `error`。
    /// 结果顺带登记渠道健康表（R7）。
    async fn call(&self, method: &str, body: Value) -> Result<Value, SlackError> {
        track(self.call_inner(method, body).await)
    }

    async fn call_inner(&self, method: &str, body: Value) -> Result<Value, SlackError> {
        let resp = self
            .http
            .post(format!("{}/{}", super::api_base(), method))
            .bearer_auth(&self.bot_token)
            .json(&body)
            .send()
            .await
            .map_err(|e| SlackError::Network(e.to_string()))?;
        let v: Value = resp.json().await.unwrap_or(Value::Null);
        if v.get("ok").and_then(|o| o.as_bool()) == Some(true) {
            Ok(v)
        } else {
            let msg = v
                .get("error")
                .and_then(|m| m.as_str())
                .unwrap_or("request failed")
                .to_string();
            Err(SlackError::Api(msg))
        }
    }

    /// 校验 Bot Token（`auth.test`）。供「测试连接」用。
    pub async fn auth_test(&self) -> Result<(), SlackError> {
        self.call("auth.test", json!({})).await.map(|_| ())
    }

    /// 解析与配置 userId 的 DM 频道 id（`conversations.open`）。
    pub async fn open_dm(&self) -> Result<String, SlackError> {
        if self.user_id.is_empty() {
            return Err(SlackError::EmptyConfig("User ID".into()));
        }
        let v = self
            .call("conversations.open", json!({ "users": self.user_id }))
            .await?;
        v.get("channel")
            .and_then(|c| c.get("id"))
            .and_then(|id| id.as_str())
            .map(|s| s.to_string())
            .ok_or(SlackError::BadResponse)
    }

    /// 发送一条消息，返回消息 `ts`（卡片后续 update 收尾用）。
    /// `blocks` 为 Block Kit 数组（None 则纯文本）；`text` 为通知回退文本。
    pub async fn post_message(
        &self,
        channel: &str,
        blocks: Option<&Value>,
        text: &str,
    ) -> Result<String, SlackError> {
        let mut body = json!({ "channel": channel, "text": text });
        if let Some(b) = blocks {
            body["blocks"] = b.clone();
        }
        let v = self.call("chat.postMessage", body).await?;
        Ok(v.get("ts")
            .and_then(|t| t.as_str())
            .unwrap_or("")
            .to_string())
    }

    /// 纯文本消息（mrkdwn）。
    pub async fn post_text(&self, channel: &str, text: &str) -> Result<String, SlackError> {
        self.post_message(channel, None, text).await
    }

    pub async fn post_thread_text(
        &self,
        channel: &str,
        thread_ts: &str,
        text: &str,
    ) -> Result<String, SlackError> {
        let body = json!({ "channel": channel, "thread_ts": thread_ts, "text": text });
        let value = self.call("chat.postMessage", body).await?;
        Ok(value
            .get("ts")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string())
    }

    /// 更新已发送的消息（收尾置静态终态 / 抢答收尾）。
    pub async fn update_message(
        &self,
        channel: &str,
        ts: &str,
        blocks: Option<&Value>,
        text: &str,
    ) -> Result<(), SlackError> {
        let mut body = json!({ "channel": channel, "ts": ts, "text": text });
        if let Some(b) = blocks {
            body["blocks"] = b.clone();
        }
        self.call("chat.update", body).await.map(|_| ())
    }

    // ===== 文件上传（AI→人，新版三步流程）=====

    /// 上传一个文件并分享进 DM 频道。图片/文件统一走此流程。
    pub async fn upload_file(
        &self,
        channel: &str,
        path: &str,
        name: &str,
    ) -> Result<(), SlackError> {
        track(self.upload_file_inner(channel, path, name).await)
    }

    async fn upload_file_inner(
        &self,
        channel: &str,
        path: &str,
        name: &str,
    ) -> Result<(), SlackError> {
        let bytes = std::fs::read(path)
            .map_err(|e| SlackError::Network(format!("failed to read file: {}", e)))?;
        let length = bytes.len();

        // 1. 取上传地址 + file_id（form 编码）。
        let v = self
            .http
            .post(format!("{}/files.getUploadURLExternal", super::api_base()))
            .bearer_auth(&self.bot_token)
            .form(&[("filename", name), ("length", &length.to_string())])
            .send()
            .await
            .map_err(|e| SlackError::Network(e.to_string()))?
            .json::<Value>()
            .await
            .map_err(|_| SlackError::BadResponse)?;
        if v.get("ok").and_then(|o| o.as_bool()) != Some(true) {
            let msg = v
                .get("error")
                .and_then(|m| m.as_str())
                .unwrap_or("getUploadURLExternal failed")
                .to_string();
            return Err(SlackError::Api(msg));
        }
        let upload_url = v
            .get("upload_url")
            .and_then(|u| u.as_str())
            .ok_or(SlackError::BadResponse)?
            .to_string();
        let file_id = v
            .get("file_id")
            .and_then(|u| u.as_str())
            .ok_or(SlackError::BadResponse)?
            .to_string();

        // 2. PUT/POST 字节到 upload_url（multipart `file` 字段）。
        let part = reqwest::multipart::Part::bytes(bytes).file_name(name.to_string());
        let form = reqwest::multipart::Form::new().part("file", part);
        let status = self
            .http
            .post(&upload_url)
            .multipart(form)
            .send()
            .await
            .map_err(|e| SlackError::Network(e.to_string()))?
            .status();
        if !status.is_success() {
            return Err(SlackError::Api(format!(
                "file upload failed: HTTP {}",
                status
            )));
        }

        // 3. 完成上传并分享进 DM 频道。
        self.call(
            "files.completeUploadExternal",
            json!({
                "files": [ { "id": &file_id, "title": name } ],
                "channel_id": channel,
            }),
        )
        .await?;

        // 4. 等待文件真正分享进时间线再返回：completeUploadExternal 返回 ok 后，文件分享进频道是
        //    **异步**的（图片还要生成缩略图，实测约数秒），分享消息的 ts 按真正分享时刻计。若不等待，
        //    紧随其后发送的提问卡片会排在文件之前，破坏「message → 附件 → question」顺序。
        //    轮询 files.info 直到 shares 出现目标频道；超时则放弃等待（best-effort，不卡死）。
        self.wait_until_shared(&file_id, channel).await;
        Ok(())
    }

    /// 轮询 `files.info` 直到文件 `shares` 中出现目标频道（即已进入时间线）。
    /// 超时（`FILE_SHARE_TIMEOUT`）则返回，不视为错误（best-effort 顺序保障）。
    async fn wait_until_shared(&self, file_id: &str, channel: &str) {
        let deadline = std::time::Instant::now() + FILE_SHARE_TIMEOUT;
        loop {
            if let Ok(v) = self.call("files.info", json!({ "file": file_id })).await {
                if file_shared_in(&v, channel) {
                    return;
                }
            }
            if std::time::Instant::now() >= deadline {
                return;
            }
            tokio::time::sleep(FILE_SHARE_POLL_INTERVAL).await;
        }
    }

    // ===== 文件下载（人→AI）=====

    /// 下载用户在 DM 里发的图片/文件（`url_private_download`），返回本地临时文件路径。
    /// `ext` 为期望扩展名（取自 filetype/mimetype）。
    pub async fn download_file_to(&self, url: &str, ext: &str) -> Result<String, SlackError> {
        let resp = self
            .http
            .get(url)
            .bearer_auth(&self.bot_token)
            .send()
            .await
            .map_err(|e| SlackError::Network(e.to_string()))?;
        if !resp.status().is_success() {
            return Err(SlackError::Api(format!(
                "file download failed: HTTP {}",
                resp.status()
            )));
        }
        let bytes = resp
            .bytes()
            .await
            .map_err(|e| SlackError::Network(e.to_string()))?;

        let dir = std::env::temp_dir().join("askhuman-slack");
        std::fs::create_dir_all(&dir)
            .map_err(|e| SlackError::Network(format!("failed to create temp dir: {}", e)))?;
        let ext = ext.trim_start_matches('.');
        let name = if ext.is_empty() {
            uuid::Uuid::new_v4().to_string()
        } else {
            format!("{}.{}", uuid::Uuid::new_v4(), ext)
        };
        let dest = dir.join(name);
        std::fs::write(&dest, &bytes)
            .map_err(|e| SlackError::Network(format!("failed to write temp file: {}", e)))?;
        Ok(dest.to_string_lossy().to_string())
    }
}

/// 判断 `files.info` 响应里该文件是否已分享进指定频道（`shares.public`/`shares.private` 任一含 `channel`）。
fn file_shared_in(info: &Value, channel: &str) -> bool {
    let Some(shares) = info.get("file").and_then(|f| f.get("shares")) else {
        return false;
    };
    ["public", "private"]
        .iter()
        .filter_map(|k| shares.get(k))
        .any(|scope| scope.get(channel).is_some())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_detected_in_private_and_public() {
        let private = json!({ "file": { "shares": { "private": { "D1": [ { "ts": "1.2" } ] } } } });
        assert!(file_shared_in(&private, "D1"));
        let public = json!({ "file": { "shares": { "public": { "C9": [ { "ts": "1.2" } ] } } } });
        assert!(file_shared_in(&public, "C9"));
    }

    #[test]
    fn not_shared_when_empty_or_other_channel() {
        let empty = json!({ "file": { "shares": {} } });
        assert!(!file_shared_in(&empty, "D1"));
        let other = json!({ "file": { "shares": { "private": { "D2": [] } } } });
        assert!(!file_shared_in(&other, "D1"));
        let missing = json!({ "file": {} });
        assert!(!file_shared_in(&missing, "D1"));
    }
}
