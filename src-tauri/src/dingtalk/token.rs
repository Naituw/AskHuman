//! access_token 获取与进程内缓存。
//!
//! `POST https://api.dingtalk.com/v1.0/oauth2/accessToken {appKey,appSecret}` → `accessToken`(7200s)。
//! 同一 token 既用于新接口（header `x-acs-dingtalk-access-token`），也用于旧 oapi 接口（query `access_token`）。

use super::DingTalkError;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

struct Cached {
    token: String,
    expire_at: Instant,
}

static CACHE: OnceLock<Mutex<HashMap<String, Cached>>> = OnceLock::new();

fn cache() -> &'static Mutex<HashMap<String, Cached>> {
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// 取 access_token：命中未过期缓存直接返回，否则换取并缓存（过期前留 60s 余量）。
pub async fn get_token(
    http: &reqwest::Client,
    client_id: &str,
    client_secret: &str,
) -> Result<String, DingTalkError> {
    if let Ok(guard) = cache().lock() {
        if let Some(c) = guard.get(client_id) {
            if c.expire_at > Instant::now() {
                return Ok(c.token.clone());
            }
        }
    }

    let resp = http
        .post(format!("{}/v1.0/oauth2/accessToken", super::api_base()))
        .json(&serde_json::json!({ "appKey": client_id, "appSecret": client_secret }))
        .send()
        .await
        .map_err(DingTalkError::from_http)?;
    let status = resp.status();
    let body: Value = resp.json().await.map_err(|_| DingTalkError::BadResponse)?;
    if !status.is_success() {
        let msg = body
            .get("message")
            .and_then(|v| v.as_str())
            .unwrap_or("failed to obtain access_token (check ClientId/ClientSecret)")
            .to_string();
        return Err(DingTalkError::Api(msg));
    }
    let token = body
        .get("accessToken")
        .and_then(|v| v.as_str())
        .ok_or(DingTalkError::BadResponse)?
        .to_string();
    let expire_in = body
        .get("expireIn")
        .and_then(|v| v.as_u64())
        .unwrap_or(7200);
    let expire_at = Instant::now() + Duration::from_secs(expire_in.saturating_sub(60));

    if let Ok(mut guard) = cache().lock() {
        guard.insert(
            client_id.to_string(),
            Cached {
                token: token.clone(),
                expire_at,
            },
        );
    }
    Ok(token)
}
