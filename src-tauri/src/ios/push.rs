//! Client of the push relay (plan §4.8). The Mac never holds an APNs key: it posts the
//! end-to-end encrypted envelope plus the phone's opaque handle to the relay, which builds the
//! alert from a fixed allowlist and forwards it to APNs.

use std::time::{Duration, Instant};

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

/// What the relay reported for `GET /v1/health`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelayHealth {
    /// Time from sending the request to having the answer (connection and TLS included).
    pub elapsed_ms: u64,
    /// Whether the relay holds an APNs key; without one it cannot send any push.
    pub apns: bool,
}

/// The relay half of the settings page's "test connection": the health document proves that the
/// address reaches an AskHuman relay (not just any web server) and says whether it can talk to
/// APNs. Unlike pushes this waits long enough for a cold function start.
pub async fn check_health(relay_url: &str) -> Result<RelayHealth, String> {
    let url = health_endpoint(relay_url)?;
    let http = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|e| e.to_string())?;
    let started = Instant::now();
    let response = http.get(url).send().await.map_err(describe_error)?;
    let status = response.status();
    let body = response.text().await.map_err(describe_error)?;
    let elapsed_ms = started.elapsed().as_millis() as u64;
    if !status.is_success() {
        return Err(format!(
            "the address answered HTTP {}, which an AskHuman relay would not: check the address",
            status.as_u16()
        ));
    }
    let not_a_relay = || "the address did not answer like an AskHuman relay".to_string();
    let doc: serde_json::Value = serde_json::from_str(&body).map_err(|_| not_a_relay())?;
    if doc["ok"] != true || doc["v"] != 1 {
        return Err(not_a_relay());
    }
    Ok(RelayHealth {
        elapsed_ms,
        apns: doc["apns"] == true,
    })
}

/// `<relay>/v1/health`. The address must be HTTPS (plain HTTP only on loopback, for tests): the
/// phone's opaque push handle travels in the clear otherwise.
fn health_endpoint(relay_url: &str) -> Result<String, String> {
    let base = relay_url.trim().trim_end_matches('/');
    let url = reqwest::Url::parse(base)
        .map_err(|_| "the relay address is not a valid URL".to_string())?;
    let loopback_http = url.scheme() == "http"
        && url
            .host_str()
            .is_some_and(super::transport::is_loopback_host);
    if url.scheme() != "https" && !loopback_http {
        return Err("the relay address must start with https://".to_string());
    }
    Ok(format!("{base}/v1/health"))
}

/// A request error with its causes (reqwest's own text stops at "error sending request") and
/// without the URL, which carries the account's function id.
fn describe_error(error: reqwest::Error) -> String {
    if error.is_timeout() {
        return "the relay did not answer in time".to_string();
    }
    let error = error.without_url();
    let mut text = error.to_string();
    let mut source = std::error::Error::source(&error);
    while let Some(cause) = source {
        text.push_str(": ");
        text.push_str(&cause.to_string());
        source = cause.source();
    }
    text
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

    /// Serves one canned HTTP answer on loopback and reports the request line it got.
    async fn serve_once(
        status: &'static str,
        body: &'static str,
    ) -> (String, tokio::task::JoinHandle<String>) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let task = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buffer = vec![0_u8; 4096];
            let read = socket.read(&mut buffer).await.unwrap();
            let request = String::from_utf8_lossy(&buffer[..read]).to_string();
            let answer = format!(
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            socket.write_all(answer.as_bytes()).await.unwrap();
            socket.shutdown().await.unwrap();
            request.lines().next().unwrap_or_default().to_string()
        });
        (base, task)
    }

    #[tokio::test]
    async fn health_check_reports_the_apns_state() {
        let (base, request) = serve_once("200 OK", r#"{"ok":true,"v":1,"apns":true}"#).await;
        let health = check_health(&format!("{base}/")).await.unwrap();
        assert!(health.apns);
        assert_eq!(request.await.unwrap(), "GET /v1/health HTTP/1.1");

        let (base, request) = serve_once("200 OK", r#"{"ok":true,"v":1,"apns":false}"#).await;
        let health = check_health(&format!("{base}/release")).await.unwrap();
        assert!(!health.apns);
        assert_eq!(request.await.unwrap(), "GET /release/v1/health HTTP/1.1");
    }

    #[tokio::test]
    async fn health_check_rejects_answers_that_are_not_a_relay() {
        let (base, _) = serve_once("404 Not Found", r#"{"error":"not_found"}"#).await;
        let error = check_health(&base).await.unwrap_err();
        assert!(error.contains("HTTP 404"), "{error}");

        let (base, _) = serve_once("200 OK", "<html>welcome</html>").await;
        let error = check_health(&base).await.unwrap_err();
        assert!(
            error.contains("did not answer like an AskHuman relay"),
            "{error}"
        );

        // A relay that speaks another protocol version is not one this Mac can use.
        let (base, _) = serve_once("200 OK", r#"{"ok":true,"v":2,"apns":true}"#).await;
        assert!(check_health(&base).await.is_err());
    }

    #[tokio::test]
    async fn health_check_needs_https_except_on_loopback() {
        for address in [
            "relay.example.com",
            "http://relay.example.com",
            "ftp://relay.example.com",
        ] {
            let error = check_health(address).await.unwrap_err();
            assert!(
                error.contains("valid URL") || error.contains("https://"),
                "{address}: {error}"
            );
        }
    }

    #[tokio::test]
    async fn health_check_reports_an_unreachable_relay() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = format!("http://{}", listener.local_addr().unwrap());
        drop(listener);
        let error = check_health(&address).await.unwrap_err();
        assert!(
            !error.contains("127.0.0.1"),
            "the address leaked into: {error}"
        );
        assert!(!error.is_empty());
    }
}
