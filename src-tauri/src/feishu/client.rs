//! 飞书单聊机器人 OpenAPI 客户端（reqwest）。
//!
//! 鉴权统一用 `tenant_access_token`（Bearer）。发消息 `receive_id_type=open_id`。
//! 互动卡片直接以 JSON 下发（`msg_type=interactive`，content 即卡片 JSON）。

use super::token;
use super::FeishuError;
use crate::config::FeishuChannelConfig;
use serde_json::{json, Value};
use std::time::Duration;

/// 渠道健康登记（R7）：API 调用失败登记、成功清除。放在统一出口，覆盖发送/编辑/上传全部路径。
fn track<T>(r: Result<T, FeishuError>) -> Result<T, FeishuError> {
    match &r {
        Ok(_) => crate::channels::health::clear("feishu"),
        Err(e) => crate::channels::health::report("feishu", e.to_string()),
    }
    r
}

#[derive(Clone)]
pub struct FeishuClient {
    app_id: String,
    app_secret: String,
    base_url: String,
    open_id: String,
    http: reqwest::Client,
}

impl FeishuClient {
    /// 构造客户端：校验 AppId/AppSecret（open_id 允许为空，自动识别流程不需要）。
    pub fn new(config: &FeishuChannelConfig) -> Result<Self, FeishuError> {
        let app_id = config.app_id.trim().to_string();
        let app_secret = config.app_secret.trim().to_string();
        let base_url = {
            let b = config.base_url.trim().trim_end_matches('/');
            if b.is_empty() {
                "https://open.feishu.cn".to_string()
            } else {
                b.to_string()
            }
        };
        if app_id.is_empty() {
            return Err(FeishuError::EmptyConfig("AppId".into()));
        }
        if app_secret.is_empty() {
            return Err(FeishuError::EmptyConfig("AppSecret".into()));
        }
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|e| FeishuError::Network(e.to_string()))?;
        Ok(Self {
            app_id,
            app_secret,
            base_url,
            open_id: config.open_id.trim().to_string(),
            http,
        })
    }

    pub fn http(&self) -> &reqwest::Client {
        &self.http
    }

    pub fn open_id(&self) -> &str {
        &self.open_id
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    pub fn app_id(&self) -> &str {
        &self.app_id
    }

    pub fn app_secret(&self) -> &str {
        &self.app_secret
    }

    async fn token(&self) -> Result<String, FeishuError> {
        token::get_token(&self.http, &self.base_url, &self.app_id, &self.app_secret).await
    }

    /// 仅校验凭据（换取一次 token）。供「测试连接」用。
    pub async fn verify(&self) -> Result<(), FeishuError> {
        self.token().await.map(|_| ())
    }

    /// 通用 JSON 调用：Bearer 鉴权 + 业务码 code==0 判定成功。结果顺带登记渠道健康表（R7）。
    async fn call(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Value,
    ) -> Result<Value, FeishuError> {
        track(self.call_inner(method, path, body).await)
    }

    async fn call_inner(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Value,
    ) -> Result<Value, FeishuError> {
        let token = self.token().await?;
        let resp = self
            .http
            .request(method, format!("{}{}", self.base_url, path))
            .bearer_auth(token)
            .json(&body)
            .send()
            .await
            .map_err(|e| FeishuError::Network(e.to_string()))?;
        let v: Value = resp.json().await.unwrap_or(Value::Null);
        if v.get("code").and_then(|c| c.as_i64()) == Some(0) {
            Ok(v)
        } else {
            let msg = v
                .get("msg")
                .and_then(|m| m.as_str())
                .unwrap_or("request failed")
                .to_string();
            Err(FeishuError::Api(msg))
        }
    }

    // ===== 单聊主动发送（im/v1/messages, receive_id_type=open_id）=====

    /// 发送一条消息，返回 message_id（卡片后续 PATCH 收尾用）。`content` 会被序列化为 JSON 字符串。
    async fn send_message(&self, msg_type: &str, content: &Value) -> Result<String, FeishuError> {
        let body = json!({
            "receive_id": self.open_id,
            "msg_type": msg_type,
            "content": content.to_string(),
        });
        let v = self
            .call(
                reqwest::Method::POST,
                "/open-apis/im/v1/messages?receive_id_type=open_id",
                body,
            )
            .await?;
        Ok(v.get("data")
            .and_then(|d| d.get("message_id"))
            .and_then(|m| m.as_str())
            .unwrap_or("")
            .to_string())
    }

    pub async fn send_text(&self, text: &str) -> Result<String, FeishuError> {
        self.send_message("text", &json!({ "text": text })).await
    }

    /// 发送互动卡片（卡片 JSON 直接作为 content）。返回 message_id。
    pub async fn send_card(&self, card: &Value) -> Result<String, FeishuError> {
        self.send_message("interactive", card).await
    }

    pub async fn send_image(&self, image_key: &str) -> Result<String, FeishuError> {
        self.send_message("image", &json!({ "image_key": image_key }))
            .await
    }

    pub async fn send_file(&self, file_key: &str) -> Result<String, FeishuError> {
        self.send_message("file", &json!({ "file_key": file_key }))
            .await
    }

    /// PATCH 更新已发送的卡片消息（收尾灰显 / 抢答收尾）。`card` 为完整卡片 JSON。
    pub async fn patch_card(&self, message_id: &str, card: &Value) -> Result<(), FeishuError> {
        let body = json!({ "content": card.to_string() });
        self.call(
            reqwest::Method::PATCH,
            &format!("/open-apis/im/v1/messages/{}", message_id),
            body,
        )
        .await?;
        Ok(())
    }

    // ===== 媒体上传（multipart）=====

    /// 上传图片，返回 image_key。
    pub async fn upload_image(&self, path: &str) -> Result<String, FeishuError> {
        let token = self.token().await?;
        let bytes = std::fs::read(path)
            .map_err(|e| FeishuError::Network(format!("failed to read file: {}", e)))?;
        let name = file_name_of(path);
        let part = reqwest::multipart::Part::bytes(bytes).file_name(name);
        let form = reqwest::multipart::Form::new()
            .text("image_type", "message")
            .part("image", part);
        let v = self.upload(&token, "/open-apis/im/v1/images", form).await?;
        v.get("data")
            .and_then(|d| d.get("image_key"))
            .and_then(|m| m.as_str())
            .map(|s| s.to_string())
            .ok_or(FeishuError::BadResponse)
    }

    /// 上传文件，返回 file_key。
    /// `file_type`：飞书要求枚举（opus/mp4/pdf/doc/xls/ppt/stream）；docx/xlsx/pptx 用
    /// doc/xls/ppt 以利预览，其余 `stream`。
    pub async fn upload_file(&self, path: &str, file_name: &str) -> Result<String, FeishuError> {
        let token = self.token().await?;
        let bytes = std::fs::read(path)
            .map_err(|e| FeishuError::Network(format!("failed to read file: {}", e)))?;
        let ext = std::path::Path::new(file_name)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        let file_type = match ext.as_str() {
            "pdf" => "pdf",
            "doc" | "docx" => "doc",
            "xls" | "xlsx" => "xls",
            "ppt" | "pptx" => "ppt",
            "mp4" => "mp4",
            "opus" => "opus",
            _ => "stream",
        };
        let part = reqwest::multipart::Part::bytes(bytes).file_name(file_name.to_string());
        let form = reqwest::multipart::Form::new()
            .text("file_type", file_type)
            .text("file_name", file_name.to_string())
            .part("file", part);
        let v = self.upload(&token, "/open-apis/im/v1/files", form).await?;
        v.get("data")
            .and_then(|d| d.get("file_key"))
            .and_then(|m| m.as_str())
            .map(|s| s.to_string())
            .ok_or(FeishuError::BadResponse)
    }

    async fn upload(
        &self,
        token: &str,
        path: &str,
        form: reqwest::multipart::Form,
    ) -> Result<Value, FeishuError> {
        track(self.upload_inner(token, path, form).await)
    }

    async fn upload_inner(
        &self,
        token: &str,
        path: &str,
        form: reqwest::multipart::Form,
    ) -> Result<Value, FeishuError> {
        let resp = self
            .http
            .post(format!("{}{}", self.base_url, path))
            .bearer_auth(token)
            .multipart(form)
            .send()
            .await
            .map_err(|e| FeishuError::Network(e.to_string()))?;
        let v: Value = resp.json().await.map_err(|_| FeishuError::BadResponse)?;
        if v.get("code").and_then(|c| c.as_i64()) == Some(0) {
            Ok(v)
        } else {
            let msg = v
                .get("msg")
                .and_then(|m| m.as_str())
                .unwrap_or("upload failed")
                .to_string();
            Err(FeishuError::Api(msg))
        }
    }

    // ===== 接收消息资源下载 =====

    /// 下载消息里的图片/文件资源到临时文件，返回本地路径。
    /// `kind` 为 `image` / `file`；`key` 为 image_key / file_key；`ext` 为期望扩展名。
    pub async fn download_resource_to(
        &self,
        message_id: &str,
        key: &str,
        kind: &str,
        ext: &str,
    ) -> Result<String, FeishuError> {
        let token = self.token().await?;
        let url = format!(
            "{}/open-apis/im/v1/messages/{}/resources/{}?type={}",
            self.base_url, message_id, key, kind
        );
        let resp = self
            .http
            .get(&url)
            .bearer_auth(token)
            .send()
            .await
            .map_err(|e| FeishuError::Network(e.to_string()))?;
        if !resp.status().is_success() {
            return Err(FeishuError::Api(format!(
                "resource download failed: HTTP {}",
                resp.status()
            )));
        }
        let bytes = resp
            .bytes()
            .await
            .map_err(|e| FeishuError::Network(e.to_string()))?;

        let dir = std::env::temp_dir().join("askhuman-feishu");
        std::fs::create_dir_all(&dir)
            .map_err(|e| FeishuError::Network(format!("failed to create temp dir: {}", e)))?;
        let ext = ext.trim_start_matches('.');
        let name = if ext.is_empty() {
            uuid::Uuid::new_v4().to_string()
        } else {
            format!("{}.{}", uuid::Uuid::new_v4(), ext)
        };
        let dest = dir.join(name);
        std::fs::write(&dest, &bytes)
            .map_err(|e| FeishuError::Network(format!("failed to write temp file: {}", e)))?;
        Ok(dest.to_string_lossy().to_string())
    }
}

fn file_name_of(path: &str) -> String {
    std::path::Path::new(path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("file")
        .to_string()
}
