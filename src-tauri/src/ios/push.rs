//! Client of the push relay (plan §4.8). The Mac never holds an APNs key: it posts the
//! end-to-end encrypted envelope plus the phone's opaque handle to the relay, which builds the
//! alert from a fixed allowlist and forwards it to APNs.

use std::time::Duration;

use async_trait::async_trait;

use super::wire::RelayPush;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PushOutcome {
    /// The relay handed the push to APNs.
    Accepted,
    /// The handle or device token is no longer valid; the phone has to register again.
    Gone,
    /// Temporary failure (network, relay, APNs).
    Failed(String),
}

#[async_trait]
pub trait PushClient: Send + Sync {
    async fn send(&self, relay_url: &str, push: &RelayPush) -> PushOutcome;
}

/// Real client: HTTPS with a short timeout and one retry on server errors.
pub struct HttpPushClient {
    http: reqwest::Client,
}

impl HttpPushClient {
    pub fn new() -> Self {
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(3))
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap_or_default();
        Self { http }
    }

    fn endpoint(relay_url: &str) -> String {
        format!("{}/v1/push", relay_url.trim().trim_end_matches('/'))
    }

    async fn attempt(&self, url: &str, push: &RelayPush) -> Result<PushOutcome, String> {
        let response = self
            .http
            .post(url)
            .json(push)
            .send()
            .await
            .map_err(|e| e.to_string())?;
        let status = response.status().as_u16();
        Ok(match status {
            200..=299 => PushOutcome::Accepted,
            // 410: APNs says the token is dead; 401: the handle is expired, revoked or forged.
            410 | 401 => PushOutcome::Gone,
            500..=599 => return Err(format!("relay returned {status}")),
            other => PushOutcome::Failed(format!("relay returned {other}")),
        })
    }
}

impl Default for HttpPushClient {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl PushClient for HttpPushClient {
    async fn send(&self, relay_url: &str, push: &RelayPush) -> PushOutcome {
        if relay_url.trim().is_empty() {
            return PushOutcome::Failed("no relay configured".into());
        }
        let url = Self::endpoint(relay_url);
        match self.attempt(&url, push).await {
            Ok(outcome) => outcome,
            Err(_) => {
                tokio::time::sleep(Duration::from_millis(500)).await;
                match self.attempt(&url, push).await {
                    Ok(outcome) => outcome,
                    Err(error) => PushOutcome::Failed(error),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoint_normalizes_trailing_slashes() {
        assert_eq!(
            HttpPushClient::endpoint("https://relay.example.com/"),
            "https://relay.example.com/v1/push"
        );
        assert_eq!(
            HttpPushClient::endpoint(" https://relay.example.com/release/ "),
            "https://relay.example.com/release/v1/push"
        );
    }
}
