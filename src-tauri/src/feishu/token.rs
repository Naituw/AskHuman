//! tenant_access_token 获取与进程内缓存。
//!
//! `POST {base}/open-apis/auth/v3/tenant_access_token/internal {app_id, app_secret}`
//! → `tenant_access_token`（有效期 `expire` 秒，约 7200）。所有 OpenAPI 调用 header 携带
//! `Authorization: Bearer <tenant_access_token>`。

use super::FeishuError;
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

/// 取 tenant_access_token：命中未过期缓存直接返回，否则换取并缓存（过期前留 60s 余量）。
/// 缓存键用 `app_id`（不同应用各自缓存）。
pub async fn get_token(
    http: &reqwest::Client,
    base_url: &str,
    app_id: &str,
    app_secret: &str,
) -> Result<String, FeishuError> {
    if let Ok(guard) = cache().lock() {
        if let Some(c) = guard.get(app_id) {
            if c.expire_at > Instant::now() {
                return Ok(c.token.clone());
            }
        }
    }

    let url = format!("{}/open-apis/auth/v3/tenant_access_token/internal", base_url);
    let resp = http
        .post(&url)
        .json(&serde_json::json!({ "app_id": app_id, "app_secret": app_secret }))
        .send()
        .await
        .map_err(|e| FeishuError::Network(e.to_string()))?;
    let body: Value = resp.json().await.map_err(|_| FeishuError::BadResponse)?;
    // 飞书业务码：code==0 成功；否则取 msg。
    if body.get("code").and_then(|c| c.as_i64()) != Some(0) {
        let msg = body
            .get("msg")
            .and_then(|m| m.as_str())
            .unwrap_or("failed to obtain tenant_access_token (check AppId/AppSecret)")
            .to_string();
        return Err(FeishuError::Api(msg));
    }
    let token = body
        .get("tenant_access_token")
        .and_then(|v| v.as_str())
        .ok_or(FeishuError::BadResponse)?
        .to_string();
    let expire = body.get("expire").and_then(|v| v.as_u64()).unwrap_or(7200);
    let expire_at = Instant::now() + Duration::from_secs(expire.saturating_sub(60));

    if let Ok(mut guard) = cache().lock() {
        guard.insert(
            app_id.to_string(),
            Cached {
                token: token.clone(),
                expire_at,
            },
        );
    }
    Ok(token)
}
