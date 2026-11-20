//! Text for a failed outbound HTTP request, shared by the IM channels.
//!
//! reqwest's own text for a transport failure has two defects for logs and UI:
//!
//! - it ends at the outermost layer ("error sending request for url (…)"); the layer that says
//!   *why* (refused, DNS, TLS, proxy, timeout) exists only in the `source()` chain;
//! - it carries the full URL, and several channels put a credential there: Telegram's bot token
//!   is a path segment, DingTalk's media upload takes `access_token` as a query parameter, and
//!   Slack's upload address is itself a one-time credential.
//!
//! [`describe`] keeps the origin (scheme, host, port), which is enough to tell which server could
//! not be reached, drops everything after it, and appends each cause as `: cause`.

/// Text for a failed request. See the module docs for what is kept and what is dropped.
pub(crate) fn describe(error: reqwest::Error) -> String {
    use std::error::Error as _;
    // An origin is scheme, host and port only: no userinfo, path, query or fragment. A URL
    // without a host has no tuple origin and keeps nothing.
    let origin = error
        .url()
        .map(|url| url.origin())
        .filter(|origin| origin.is_tuple())
        .and_then(|origin| reqwest::Url::parse(&origin.ascii_serialization()).ok());
    let error = match origin {
        Some(url) => error.with_url(url),
        None => error.without_url(),
    };
    let mut text = error.to_string();
    let mut cause = error.source();
    while let Some(layer) = cause {
        text.push_str(": ");
        text.push_str(&layer.to_string());
        cause = layer.source();
    }
    text
}

/// A client plus a loopback base URL nothing listens on, for failure-path tests. The client
/// ignores proxy environment variables so the refusal always comes from the loopback itself.
#[cfg(test)]
pub(crate) fn refused_connection() -> (reqwest::Client, String) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let http = reqwest::Client::builder().no_proxy().build().unwrap();
    (http, format!("http://127.0.0.1:{port}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn failure_of(http: &reqwest::Client, url: &str) -> reqwest::Error {
        http.get(url).send().await.unwrap_err()
    }

    #[tokio::test]
    async fn only_the_origin_of_the_url_survives() {
        let (http, base_url) = refused_connection();
        // A credential in each place the channels put one: a path segment (Telegram's bot
        // token) and a query parameter (DingTalk's `access_token`), plus a fragment. Userinfo is
        // not listed: reqwest moves it into an Authorization header before it can fail.
        let url = format!(
            "{base_url}/bot123456:PATH-SECRET/sendMessage?access_token=QUERY-SECRET#FRAGMENT-SECRET"
        );
        let text = describe(failure_of(&http, &url).await);
        assert!(text.contains(&base_url), "the host is missing from: {text}");
        for secret in [
            "PATH-SECRET",
            "QUERY-SECRET",
            "FRAGMENT-SECRET",
            "sendMessage",
        ] {
            assert!(!text.contains(secret), "{secret} leaked into: {text}");
        }
        assert!(
            text.contains("os error"),
            "the transport cause is missing from: {text}"
        );
    }

    #[tokio::test]
    async fn a_timeout_stays_distinguishable_from_a_refusal() {
        // The listener completes the TCP handshake but never answers, like a firewall that
        // swallows the traffic. It must stay alive until the request has timed out.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/secret/path", listener.local_addr().unwrap());
        let http = reqwest::Client::builder()
            .no_proxy()
            .timeout(std::time::Duration::from_millis(100))
            .build()
            .unwrap();
        let source = failure_of(&http, &url).await;
        assert!(source.is_timeout());
        let text = describe(source);
        assert!(
            text.contains("timed out"),
            "the timeout is missing from: {text}"
        );
        assert!(!text.contains("os error"), "{text}");
        assert!(!text.contains("secret/path"), "{text}");
        drop(listener);
    }

    #[tokio::test]
    async fn a_request_that_never_had_a_url_still_describes_itself() {
        // An unparsable address fails while the request is built, so there is no URL to keep.
        let http = reqwest::Client::builder().no_proxy().build().unwrap();
        let text = describe(failure_of(&http, "http://[::1").await);
        assert!(text.starts_with("builder error"), "{text}");
    }
}
