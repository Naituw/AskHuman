//! 钉钉单聊机器人 OpenAPI 客户端（reqwest）。
//!
//! robotCode 统一取 `client_id`（企业内部应用机器人 robotCode = AppKey）。

use super::token;
use super::DingTalkError;
use crate::config::DingTalkChannelConfig;
use serde_json::{json, Value};
use std::time::Duration;

const API_BASE: &str = "https://api.dingtalk.com";
const OAPI_BASE: &str = "https://oapi.dingtalk.com";

pub struct DingTalkClient {
    client_id: String,
    client_secret: String,
    user_id: String,
    http: reqwest::Client,
}

impl DingTalkClient {
    pub fn new(config: &DingTalkChannelConfig) -> Result<Self, DingTalkError> {
        let client_id = config.client_id.trim().to_string();
        let client_secret = config.client_secret.trim().to_string();
        let user_id = config.user_id.trim().to_string();
        if client_id.is_empty() {
            return Err(DingTalkError::EmptyConfig("ClientId".into()));
        }
        if client_secret.is_empty() {
            return Err(DingTalkError::EmptyConfig("ClientSecret".into()));
        }
        if user_id.is_empty() {
            return Err(DingTalkError::EmptyConfig("UserId".into()));
        }
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|e| DingTalkError::Network(e.to_string()))?;
        Ok(Self {
            client_id,
            client_secret,
            user_id,
            http,
        })
    }

    /// 机器人编码 = AppKey(ClientId)。
    pub fn robot_code(&self) -> &str {
        &self.client_id
    }

    pub fn user_id(&self) -> &str {
        &self.user_id
    }

    pub fn http(&self) -> &reqwest::Client {
        &self.http
    }

    async fn token(&self) -> Result<String, DingTalkError> {
        token::get_token(&self.http, &self.client_id, &self.client_secret).await
    }

    /// 调用新版接口（`https://api.dingtalk.com{path}`，header 携带 access_token），返回响应体。
    /// 以 HTTP 2xx 判定成功；失败时取 body.message 作为错误信息。
    pub(crate) async fn call_new(&self, path: &str, body: Value) -> Result<Value, DingTalkError> {
        self.call_new_method(reqwest::Method::POST, path, body).await
    }

    /// 同 `call_new`，但可指定 HTTP method（如更新卡片用 PUT）。
    async fn call_new_method(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Value,
    ) -> Result<Value, DingTalkError> {
        let token = self.token().await?;
        let resp = self
            .http
            .request(method, format!("{}{}", API_BASE, path))
            .header("x-acs-dingtalk-access-token", token)
            .json(&body)
            .send()
            .await
            .map_err(|e| DingTalkError::Network(e.to_string()))?;
        let status = resp.status();
        let v: Value = resp.json().await.unwrap_or(Value::Null);
        if status.is_success() {
            Ok(v)
        } else {
            let msg = v
                .get("message")
                .and_then(|m| m.as_str())
                .unwrap_or("request failed")
                .to_string();
            Err(DingTalkError::Api(msg))
        }
    }

    // ===== 单聊主动发送（oToMessages/batchSend）=====

    async fn send_oto(&self, msg_key: &str, msg_param: Value) -> Result<(), DingTalkError> {
        let body = json!({
            "robotCode": self.robot_code(),
            "userIds": [self.user_id],
            "msgKey": msg_key,
            "msgParam": msg_param.to_string(),
        });
        self.call_new("/v1.0/robot/oToMessages/batchSend", body).await?;
        Ok(())
    }

    pub async fn send_oto_text(&self, content: &str) -> Result<(), DingTalkError> {
        self.send_oto("sampleText", json!({ "content": content })).await
    }

    pub async fn send_oto_markdown(&self, title: &str, text: &str) -> Result<(), DingTalkError> {
        self.send_oto("sampleMarkdown", json!({ "title": title, "text": text }))
            .await
    }

    pub async fn send_oto_image(&self, media_id: &str) -> Result<(), DingTalkError> {
        self.send_oto("sampleImageMsg", json!({ "photoURL": media_id }))
            .await
    }

    pub async fn send_oto_file(
        &self,
        media_id: &str,
        file_name: &str,
        file_type: &str,
    ) -> Result<(), DingTalkError> {
        self.send_oto(
            "sampleFile",
            json!({ "mediaId": media_id, "fileName": file_name, "fileType": file_type }),
        )
        .await
    }

    // ===== 媒体上传（旧 oapi，access_token 走 query）=====

    /// 上传媒体文件，返回 media_id。`kind` 为 `image` / `file`。
    pub async fn upload_media(&self, path: &str, kind: &str) -> Result<String, DingTalkError> {
        let token = self.token().await?;
        let bytes = std::fs::read(path)
            .map_err(|e| DingTalkError::Network(format!("failed to read file: {}", e)))?;
        let file_name = std::path::Path::new(path)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("file")
            .to_string();
        let part = reqwest::multipart::Part::bytes(bytes).file_name(file_name);
        let form = reqwest::multipart::Form::new().part("media", part);
        let url = format!("{}/media/upload?access_token={}&type={}", OAPI_BASE, token, kind);
        let resp = self
            .http
            .post(&url)
            .multipart(form)
            .send()
            .await
            .map_err(|e| DingTalkError::Network(e.to_string()))?;
        let v: Value = resp.json().await.map_err(|_| DingTalkError::BadResponse)?;
        if v.get("errcode").and_then(|c| c.as_i64()) == Some(0) {
            v.get("media_id")
                .and_then(|m| m.as_str())
                .map(|s| s.to_string())
                .ok_or(DingTalkError::BadResponse)
        } else {
            let msg = v
                .get("errmsg")
                .and_then(|m| m.as_str())
                .unwrap_or("media upload failed")
                .to_string();
            Err(DingTalkError::Api(msg))
        }
    }

    // ===== 互动卡片高级版（创建并投放 / 更新）=====

    /// 创建并投放互动卡片高级版到机器人单聊。回调走 Stream（`callbackType=STREAM`）。
    /// `card_param_map` 为公有数据；`private_param_map` 为当前用户私有数据（私有变量默认值，
    /// 缺省会导致模板「内容加载失败」）。值均为字符串；`out_track_id` 唯一标识本卡片实例。
    pub async fn create_and_deliver_card(
        &self,
        out_track_id: &str,
        card_template_id: &str,
        card_param_map: Value,
        private_param_map: Value,
    ) -> Result<(), DingTalkError> {
        let mut private = serde_json::Map::new();
        private.insert(
            self.user_id.clone(),
            json!({ "cardParamMap": private_param_map }),
        );
        let body = json!({
            "cardTemplateId": card_template_id,
            "outTrackId": out_track_id,
            "cardData": { "cardParamMap": card_param_map },
            "privateData": Value::Object(private),
            "openSpaceId": format!("dtv1.card//IM_ROBOT.{}", self.user_id),
            "imRobotOpenSpaceModel": { "supportForward": true },
            "imRobotOpenDeliverModel": { "spaceType": "IM_ROBOT", "robotCode": self.robot_code() },
            "callbackType": "STREAM",
            "userIdType": 1,
        });
        self.call_new("/v1.0/card/instances/createAndDeliver", body)
            .await?;
        Ok(())
    }

    /// 按 key 更新卡片数据：公有变量写 `cardData`，私有变量写当前用户的 `privateData`。
    /// 用于收尾/抢答时 best-effort 置 `submitted=true`（私有）并写 `submit_status`（公有）。
    /// 公私必须分开下发：把私有变量塞进公有 `cardData` 会被钉钉拒绝导致整份数据失效。
    /// 两个 map 的值均为字符串；任一为空对象时对应部分留空。
    pub async fn update_card_private(
        &self,
        out_track_id: &str,
        public_map: Value,
        private_map: Value,
    ) -> Result<(), DingTalkError> {
        let mut private = serde_json::Map::new();
        private.insert(self.user_id.clone(), json!({ "cardParamMap": private_map }));
        let body = json!({
            "outTrackId": out_track_id,
            "cardData": { "cardParamMap": public_map },
            "privateData": Value::Object(private),
            "cardUpdateOptions": {
                "updateCardDataByKey": true,
                "updatePrivateDataByKey": true,
            },
            "userIdType": 1,
        });
        self.call_new_method(reqwest::Method::PUT, "/v1.0/card/instances", body)
            .await?;
        Ok(())
    }

    // ===== 接收消息中的文件下载 =====

    /// 用 downloadCode 换临时下载链接并下载到本地临时文件，返回本地路径。
    /// `ext` 为期望扩展名（钉钉下载默认 `.file`，需按真实类型修正）。
    pub async fn download_message_file_to(
        &self,
        download_code: &str,
        ext: &str,
    ) -> Result<String, DingTalkError> {
        let body = json!({ "downloadCode": download_code, "robotCode": self.robot_code() });
        let v = self
            .call_new("/v1.0/robot/messageFiles/download", body)
            .await?;
        let url = v
            .get("downloadUrl")
            .and_then(|u| u.as_str())
            .ok_or(DingTalkError::BadResponse)?;
        let bytes = self
            .http
            .get(url)
            .send()
            .await
            .map_err(|e| DingTalkError::Network(e.to_string()))?
            .bytes()
            .await
            .map_err(|e| DingTalkError::Network(e.to_string()))?;

        let dir = std::env::temp_dir().join("askhuman-dingtalk");
        std::fs::create_dir_all(&dir)
            .map_err(|e| DingTalkError::Network(format!("failed to create temp dir: {}", e)))?;
        let ext = ext.trim_start_matches('.');
        let name = if ext.is_empty() {
            uuid::Uuid::new_v4().to_string()
        } else {
            format!("{}.{}", uuid::Uuid::new_v4(), ext)
        };
        let dest = dir.join(name);
        std::fs::write(&dest, &bytes)
            .map_err(|e| DingTalkError::Network(format!("failed to write temp file: {}", e)))?;
        Ok(dest.to_string_lossy().to_string())
    }
}
