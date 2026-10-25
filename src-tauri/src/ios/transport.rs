//! MQTT transport of the iOS gateway (plan §4.2).
//!
//! `BrokerTransport` is the narrow surface the gateway needs. Two implementations:
//! - [`RumqttcTransport`]: the real client (TLS first, WSS fallback, jittered backoff).
//! - [`LoopbackBroker`] / [`LoopbackTransport`]: an in-process broker with topic matching,
//!   retained messages, last wills and fault injection for tests.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};

use super::wire::KEEPALIVE_SECS;

/// What the transport reports to the gateway.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransportEvent {
    /// Connection (re)established. The session is clean, so subscriptions must be re-issued.
    Connected {
        via: String,
    },
    Disconnected {
        reason: String,
        auth_failed: bool,
    },
    Message {
        topic: String,
        payload: Vec<u8>,
        retain: bool,
    },
}

/// Last will registered with the broker at connect time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Will {
    pub topic: String,
    pub payload: Vec<u8>,
    pub retain: bool,
}

#[async_trait]
pub trait BrokerTransport: Send + Sync {
    /// QoS 1 publish.
    async fn publish(&self, topic: &str, payload: Vec<u8>, retain: bool) -> Result<(), String>;
    /// QoS 1 subscribe.
    async fn subscribe(&self, topic: &str) -> Result<(), String>;
    async fn unsubscribe(&self, topic: &str) -> Result<(), String>;
    /// Flushes pending publishes, disconnects cleanly (no last will) and stops reconnecting.
    async fn disconnect(&self);
    fn is_connected(&self) -> bool;
}

/// MQTT topic filter matching (`+` single level, `#` rest).
pub fn topic_matches(filter: &str, topic: &str) -> bool {
    let mut f = filter.split('/');
    let mut t = topic.split('/');
    loop {
        match (f.next(), t.next()) {
            (Some("#"), _) => return true,
            (Some("+"), Some(_)) => {}
            (Some(a), Some(b)) if a == b => {}
            (None, None) => return true,
            _ => return false,
        }
    }
}

// ---------------------------------------------------------------- loopback broker

struct LoopbackClient {
    tx: UnboundedSender<TransportEvent>,
    subs: Vec<String>,
    will: Option<Will>,
    /// Topic prefix the client may use (emulates the broker ACL); `None` = unrestricted.
    acl_prefix: Option<String>,
    connected: bool,
}

#[derive(Default)]
struct BrokerInner {
    clients: BTreeMap<String, LoopbackClient>,
    retained: BTreeMap<String, Vec<u8>>,
    /// Publishes to topics containing one of these fragments are silently dropped (fault injection).
    drop_topics: Vec<String>,
    /// Every publish the broker accepted, in order, for assertions: `(client id, topic)`.
    log: Vec<(String, String)>,
}

/// In-process MQTT broker for tests.
#[derive(Clone, Default)]
pub struct LoopbackBroker {
    inner: Arc<Mutex<BrokerInner>>,
}

impl LoopbackBroker {
    pub fn new() -> Self {
        Self::default()
    }

    /// Connects a client (a previous connection with the same id is replaced, like a real broker).
    pub fn connect(
        &self,
        client_id: &str,
        will: Option<Will>,
        acl_prefix: Option<&str>,
    ) -> (Arc<LoopbackTransport>, UnboundedReceiver<TransportEvent>) {
        let (tx, rx) = unbounded_channel();
        let mut inner = self.inner.lock().unwrap();
        if inner.clients.contains_key(client_id) {
            Self::drop_locked(&mut inner, client_id, "taken over");
        }
        let _ = tx.send(TransportEvent::Connected {
            via: "loopback".into(),
        });
        inner.clients.insert(
            client_id.to_string(),
            LoopbackClient {
                tx: tx.clone(),
                subs: vec![],
                will: will.clone(),
                acl_prefix: acl_prefix.map(str::to_string),
                connected: true,
            },
        );
        drop(inner);
        (
            Arc::new(LoopbackTransport {
                broker: self.clone(),
                client_id: client_id.to_string(),
                closed: AtomicBool::new(false),
                tx,
                will,
                acl_prefix: acl_prefix.map(str::to_string),
            }),
            rx,
        )
    }

    /// Simulates a network failure: the will is published and the client is told it was dropped.
    pub fn drop_connection(&self, client_id: &str) {
        let mut inner = self.inner.lock().unwrap();
        Self::drop_locked(&mut inner, client_id, "connection lost");
    }

    fn drop_locked(inner: &mut BrokerInner, client_id: &str, reason: &str) {
        let Some(client) = inner.clients.remove(client_id) else {
            return;
        };
        let _ = client.tx.send(TransportEvent::Disconnected {
            reason: reason.into(),
            auth_failed: false,
        });
        if let Some(will) = client.will {
            Self::route_locked(inner, client_id, &will.topic, will.payload, will.retain);
        }
    }

    /// Drops publishes whose topic contains `fragment` until [`Self::clear_faults`].
    pub fn drop_publishes_to(&self, fragment: &str) {
        self.inner
            .lock()
            .unwrap()
            .drop_topics
            .push(fragment.to_string());
    }

    pub fn clear_faults(&self) {
        self.inner.lock().unwrap().drop_topics.clear();
    }

    pub fn retained(&self, topic: &str) -> Option<Vec<u8>> {
        self.inner.lock().unwrap().retained.get(topic).cloned()
    }

    /// Topics published so far by `client_id`.
    pub fn published_by(&self, client_id: &str) -> Vec<String> {
        self.inner
            .lock()
            .unwrap()
            .log
            .iter()
            .filter(|(c, _)| c == client_id)
            .map(|(_, t)| t.clone())
            .collect()
    }

    pub fn is_connected(&self, client_id: &str) -> bool {
        self.inner
            .lock()
            .unwrap()
            .clients
            .get(client_id)
            .is_some_and(|c| c.connected)
    }

    fn route_locked(
        inner: &mut BrokerInner,
        from: &str,
        topic: &str,
        payload: Vec<u8>,
        retain: bool,
    ) {
        inner.log.push((from.to_string(), topic.to_string()));
        if retain {
            if payload.is_empty() {
                inner.retained.remove(topic);
            } else {
                inner.retained.insert(topic.to_string(), payload.clone());
            }
        }
        // Like MQTT, a client sees its own publishes when it subscribed to the topic (the
        // gateway subscribes to `s/+` and therefore sees its own presence).
        for client in inner.clients.values() {
            if client.subs.iter().any(|f| topic_matches(f, topic)) {
                let _ = client.tx.send(TransportEvent::Message {
                    topic: topic.to_string(),
                    payload: payload.clone(),
                    retain: false,
                });
            }
        }
    }
}

pub struct LoopbackTransport {
    broker: LoopbackBroker,
    client_id: String,
    closed: AtomicBool,
    tx: UnboundedSender<TransportEvent>,
    will: Option<Will>,
    acl_prefix: Option<String>,
}

impl LoopbackTransport {
    /// Test aid: the network came back. Registers the client again (without subscriptions, like a
    /// clean MQTT session) and reports a fresh connection.
    pub fn reconnect(&self) {
        let mut inner = self.broker.inner.lock().unwrap();
        if inner.clients.contains_key(&self.client_id) {
            return;
        }
        let _ = self.tx.send(TransportEvent::Connected {
            via: "loopback".into(),
        });
        inner.clients.insert(
            self.client_id.clone(),
            LoopbackClient {
                tx: self.tx.clone(),
                subs: vec![],
                will: self.will.clone(),
                acl_prefix: self.acl_prefix.clone(),
                connected: true,
            },
        );
    }

    fn allowed(client: &LoopbackClient, topic: &str) -> bool {
        client
            .acl_prefix
            .as_ref()
            .is_none_or(|prefix| topic.starts_with(prefix.as_str()))
    }
}

#[async_trait]
impl BrokerTransport for LoopbackTransport {
    async fn publish(&self, topic: &str, payload: Vec<u8>, retain: bool) -> Result<(), String> {
        let mut inner = self.broker.inner.lock().unwrap();
        let Some(client) = inner.clients.get(&self.client_id) else {
            return Err("not connected".into());
        };
        if !Self::allowed(client, topic) {
            return Err(format!("not authorized to publish {topic}"));
        }
        if inner.drop_topics.iter().any(|f| topic.contains(f.as_str())) {
            return Ok(());
        }
        LoopbackBroker::route_locked(&mut inner, &self.client_id, topic, payload, retain);
        Ok(())
    }

    async fn subscribe(&self, topic: &str) -> Result<(), String> {
        let mut inner = self.broker.inner.lock().unwrap();
        let retained: Vec<(String, Vec<u8>)> = inner
            .retained
            .iter()
            .filter(|(t, _)| topic_matches(topic, t))
            .map(|(t, p)| (t.clone(), p.clone()))
            .collect();
        let Some(client) = inner.clients.get_mut(&self.client_id) else {
            return Err("not connected".into());
        };
        if !Self::allowed(client, topic.trim_end_matches(['+', '#'])) {
            return Err(format!("not authorized to subscribe {topic}"));
        }
        if !client.subs.iter().any(|s| s == topic) {
            client.subs.push(topic.to_string());
        }
        for (t, payload) in retained {
            let _ = client.tx.send(TransportEvent::Message {
                topic: t,
                payload,
                retain: true,
            });
        }
        Ok(())
    }

    async fn unsubscribe(&self, topic: &str) -> Result<(), String> {
        let mut inner = self.broker.inner.lock().unwrap();
        if let Some(client) = inner.clients.get_mut(&self.client_id) {
            client.subs.retain(|s| s != topic);
        }
        Ok(())
    }

    async fn disconnect(&self) {
        self.closed.store(true, Ordering::SeqCst);
        let mut inner = self.broker.inner.lock().unwrap();
        // A clean disconnect discards the will.
        if let Some(client) = inner.clients.get_mut(&self.client_id) {
            client.will = None;
        }
        LoopbackBroker::drop_locked(&mut inner, &self.client_id, "disconnected");
    }

    fn is_connected(&self) -> bool {
        !self.closed.load(Ordering::SeqCst) && self.broker.is_connected(&self.client_id)
    }
}

// ---------------------------------------------------------------- rumqttc transport

/// Connection parameters of the real transport.
#[derive(Debug, Clone)]
pub struct ConnectParams {
    pub host: String,
    pub tls_port: u16,
    pub wss_port: u16,
    pub username: String,
    pub password: String,
    pub client_id: String,
    pub will: Will,
    /// `tls` or `wss`: start with the transport that worked last time.
    pub prefer: Option<String>,
    /// Plain TCP to `host:tls_port` (loopback test brokers only).
    pub insecure_local: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Via {
    Tls,
    Wss,
    Plain,
}

impl Via {
    fn name(self) -> &'static str {
        match self {
            Via::Tls => "tls",
            Via::Wss => "wss",
            Via::Plain => "tcp",
        }
    }

    fn other(self) -> Self {
        match self {
            Via::Tls => Via::Wss,
            Via::Wss => Via::Tls,
            Via::Plain => Via::Plain,
        }
    }
}

struct Shared {
    client: Mutex<Option<rumqttc::AsyncClient>>,
    connected: AtomicBool,
}

pub struct RumqttcTransport {
    shared: Arc<Shared>,
    stop: UnboundedSender<tokio::sync::oneshot::Sender<()>>,
}

fn tls_client_config() -> Arc<rustls::ClientConfig> {
    let mut roots = rustls::RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let config = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .expect("ring supports the default protocol versions")
        .with_root_certificates(roots)
        .with_no_client_auth();
    Arc::new(config)
}

/// Loopback addresses only: the plain-TCP test option must never reach a real network.
pub fn is_loopback_host(host: &str) -> bool {
    host == "localhost"
        || host
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback())
}

fn build_options(params: &ConnectParams, via: Via) -> rumqttc::MqttOptions {
    use rumqttc::{LastWill, MqttOptions, QoS, Transport};
    let mut options = match via {
        Via::Wss => MqttOptions::new(
            &params.client_id,
            format!("wss://{}:{}/mqtt", params.host, params.wss_port),
            params.wss_port,
        ),
        _ => MqttOptions::new(&params.client_id, &params.host, params.tls_port),
    };
    options
        .set_keep_alive(Duration::from_secs(KEEPALIVE_SECS))
        .set_clean_session(true)
        .set_credentials(&params.username, &params.password)
        .set_max_packet_size(1_000_000, 1_000_000)
        .set_request_channel_capacity(256)
        .set_last_will(LastWill::new(
            &params.will.topic,
            params.will.payload.clone(),
            QoS::AtLeastOnce,
            params.will.retain,
        ));
    match via {
        Via::Tls => {
            options.set_transport(Transport::tls_with_config(
                rumqttc::TlsConfiguration::Rustls(tls_client_config()),
            ));
        }
        Via::Wss => {
            options.set_transport(Transport::wss_with_config(
                rumqttc::TlsConfiguration::Rustls(tls_client_config()),
            ));
        }
        Via::Plain => {
            options.set_transport(Transport::Tcp);
        }
    }
    options
}

/// 1, 2, 4, 8, 15, 30 seconds with ±20 % jitter; 60 seconds after an authentication failure.
pub fn backoff_delay(attempt: u32, auth_failed: bool) -> Duration {
    let base = if auth_failed {
        60.0
    } else {
        [1.0, 2.0, 4.0, 8.0, 15.0, 30.0][attempt.min(5) as usize]
    };
    let jitter = 0.8 + 0.4 * (crate::ios::crypto::random_bytes::<1>()[0] as f64 / 255.0);
    Duration::from_secs_f64(base * jitter)
}

impl RumqttcTransport {
    /// Starts the connection supervisor on the current tokio runtime.
    pub fn spawn(params: ConnectParams) -> (Arc<Self>, UnboundedReceiver<TransportEvent>) {
        let (events_tx, events_rx) = unbounded_channel();
        let (stop_tx, stop_rx) = unbounded_channel();
        let shared = Arc::new(Shared {
            client: Mutex::new(None),
            connected: AtomicBool::new(false),
        });
        tokio::spawn(supervise(params, shared.clone(), events_tx, stop_rx));
        (
            Arc::new(Self {
                shared,
                stop: stop_tx,
            }),
            events_rx,
        )
    }

    fn client(&self) -> Result<rumqttc::AsyncClient, String> {
        self.shared
            .client
            .lock()
            .unwrap()
            .clone()
            .ok_or_else(|| "not connected".to_string())
    }
}

#[async_trait]
impl BrokerTransport for RumqttcTransport {
    async fn publish(&self, topic: &str, payload: Vec<u8>, retain: bool) -> Result<(), String> {
        self.client()?
            .publish(topic, rumqttc::QoS::AtLeastOnce, retain, payload)
            .await
            .map_err(|e| e.to_string())
    }

    async fn subscribe(&self, topic: &str) -> Result<(), String> {
        self.client()?
            .subscribe(topic, rumqttc::QoS::AtLeastOnce)
            .await
            .map_err(|e| e.to_string())
    }

    async fn unsubscribe(&self, topic: &str) -> Result<(), String> {
        self.client()?
            .unsubscribe(topic)
            .await
            .map_err(|e| e.to_string())
    }

    async fn disconnect(&self) {
        let (done_tx, done_rx) = tokio::sync::oneshot::channel();
        if self.stop.send(done_tx).is_ok() {
            let _ = tokio::time::timeout(Duration::from_secs(4), done_rx).await;
        }
    }

    fn is_connected(&self) -> bool {
        self.shared.connected.load(Ordering::SeqCst)
    }
}

fn classify(error: &rumqttc::ConnectionError) -> (String, bool) {
    let auth = matches!(
        error,
        rumqttc::ConnectionError::ConnectionRefused(
            rumqttc::ConnectReturnCode::BadUserNamePassword
                | rumqttc::ConnectReturnCode::NotAuthorized
        )
    );
    (error.to_string(), auth)
}

async fn supervise(
    params: ConnectParams,
    shared: Arc<Shared>,
    events: UnboundedSender<TransportEvent>,
    mut stop: UnboundedReceiver<tokio::sync::oneshot::Sender<()>>,
) {
    use rumqttc::{AsyncClient, Event, NetworkOptions, Packet};

    let mut via = if params.insecure_local {
        Via::Plain
    } else if params.prefer.as_deref() == Some("wss") {
        Via::Wss
    } else {
        Via::Tls
    };
    let mut attempt = 0u32;
    loop {
        let options = build_options(&params, via);
        let (client, mut eventloop) = AsyncClient::new(options, 256);
        let mut network = NetworkOptions::new();
        network.set_connection_timeout(5);
        network.set_tcp_nodelay(true);
        eventloop.set_network_options(network);
        *shared.client.lock().unwrap() = Some(client.clone());

        let mut connected_once = false;
        let (reason, auth_failed) = loop {
            tokio::select! {
                biased;
                done = stop.recv() => {
                    // Graceful stop: flush the queue, send DISCONNECT (the will is discarded).
                    let _ = client.disconnect().await;
                    let _ = tokio::time::timeout(Duration::from_secs(2), async {
                        while eventloop.poll().await.is_ok() {}
                    })
                    .await;
                    shared.connected.store(false, Ordering::SeqCst);
                    *shared.client.lock().unwrap() = None;
                    if let Some(done) = done {
                        let _ = done.send(());
                    }
                    return;
                }
                event = eventloop.poll() => match event {
                    // A refused CONNACK surfaces as `ConnectionError::ConnectionRefused` below.
                    Ok(Event::Incoming(Packet::ConnAck(_))) => {
                        connected_once = true;
                        shared.connected.store(true, Ordering::SeqCst);
                        let _ = events.send(TransportEvent::Connected { via: via.name().into() });
                    }
                    Ok(Event::Incoming(Packet::Publish(p))) => {
                        let _ = events.send(TransportEvent::Message {
                            topic: p.topic,
                            payload: p.payload.to_vec(),
                            retain: p.retain,
                        });
                    }
                    Ok(_) => {}
                    Err(error) => break classify(&error),
                }
            }
        };
        shared.connected.store(false, Ordering::SeqCst);
        *shared.client.lock().unwrap() = None;
        if events
            .send(TransportEvent::Disconnected {
                reason,
                auth_failed,
            })
            .is_err()
        {
            return;
        }
        if connected_once {
            attempt = 0;
        } else {
            attempt = attempt.saturating_add(1);
            // Never connected over this path: try the other one next.
            via = via.other();
        }
        let delay = backoff_delay(attempt.saturating_sub(1), auth_failed);
        tokio::select! {
            biased;
            done = stop.recv() => {
                if let Some(done) = done {
                    let _ = done.send(());
                }
                return;
            }
            _ = tokio::time::sleep(delay) => {}
        }
    }
}

/// What a successful connectivity probe learned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProbeReport {
    /// Transport that worked: `tls`, `wss` or `tcp`.
    pub via: String,
    /// Time from the first connection attempt to the message coming back.
    pub elapsed_ms: u64,
}

/// "Test connection" of the settings page: connects like the gateway would (TLS first, WSS as the
/// fallback) and sends one message to itself through the broker, which proves the credentials and
/// the access rules in one go. Uses a throwaway client id and writes nothing retained.
pub async fn probe(mut params: ConnectParams, limit: Duration) -> Result<ProbeReport, String> {
    let id = super::crypto::new_id();
    let topic = format!("ah/{}/t/{id}", params.username);
    params.client_id = format!("probe-{id}");
    params.will = Will {
        topic: topic.clone(),
        payload: Vec::new(),
        retain: false,
    };
    params.prefer = None;
    let (transport, mut events) = RumqttcTransport::spawn(params);
    let started = std::time::Instant::now();
    let deadline = tokio::time::Instant::now() + limit;
    let mut via: Option<String> = None;
    let mut last_error = String::new();
    let outcome = loop {
        let event = tokio::select! {
            event = events.recv() => event,
            _ = tokio::time::sleep_until(deadline) => break Err(match &via {
                Some(via) => format!(
                    "connected via {via}, but a test message did not come back: check the broker's access rules for topics under ah/<username>/"
                ),
                None if last_error.is_empty() => "timed out".to_string(),
                None => format!("timed out ({last_error})"),
            }),
        };
        match event {
            Some(TransportEvent::Connected { via: used }) => {
                via = Some(used);
                let sent = async {
                    transport.subscribe(&topic).await?;
                    transport.publish(&topic, b"probe".to_vec(), false).await
                }
                .await;
                if let Err(error) = sent {
                    break Err(error);
                }
            }
            Some(TransportEvent::Message { topic: got, .. }) if got == topic => {
                break Ok(ProbeReport {
                    via: via.clone().unwrap_or_default(),
                    elapsed_ms: started.elapsed().as_millis() as u64,
                });
            }
            Some(TransportEvent::Message { .. }) => {}
            Some(TransportEvent::Disconnected {
                reason,
                auth_failed,
            }) => {
                if auth_failed {
                    break Err(format!("the broker rejected the credentials ({reason})"));
                }
                last_error = reason;
            }
            None => break Err("the connection supervisor stopped".to_string()),
        }
    };
    transport.disconnect().await;
    outcome
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn topic_filters_match_like_mqtt() {
        assert!(topic_matches("ah/u/s/+", "ah/u/s/dev1"));
        assert!(!topic_matches("ah/u/s/+", "ah/u/s/dev1/x"));
        assert!(topic_matches("ah/u/#", "ah/u/d/p"));
        assert!(topic_matches("ah/u/#", "ah/u"));
        assert!(!topic_matches("ah/u/d/p", "ah/u/d/q"));
        assert!(!topic_matches("ah/u/d/p/#", "ah/u/d"));
        assert!(topic_matches("a/b", "a/b"));
        assert!(!topic_matches("a/b/c", "a/b"));
    }

    #[test]
    fn loopback_hosts_only() {
        assert!(is_loopback_host("localhost"));
        assert!(is_loopback_host("127.0.0.1"));
        assert!(is_loopback_host("::1"));
        assert!(!is_loopback_host("192.168.1.2"));
        assert!(!is_loopback_host("broker.example.com"));
    }

    #[test]
    fn backoff_follows_the_documented_ladder() {
        for (attempt, base) in [
            (0u32, 1.0f64),
            (1, 2.0),
            (2, 4.0),
            (3, 8.0),
            (4, 15.0),
            (5, 30.0),
            (9, 30.0),
        ] {
            let d = backoff_delay(attempt, false).as_secs_f64();
            assert!(d >= base * 0.79 && d <= base * 1.21, "{attempt}: {d}");
        }
        let d = backoff_delay(0, true).as_secs_f64();
        assert!((47.0..=73.0).contains(&d));
    }

    async fn next(rx: &mut UnboundedReceiver<TransportEvent>) -> TransportEvent {
        tokio::time::timeout(Duration::from_secs(1), rx.recv())
            .await
            .expect("event in time")
            .expect("channel open")
    }

    #[tokio::test]
    async fn loopback_delivers_to_subscribers_and_replays_retained() {
        let broker = LoopbackBroker::new();
        let (a, mut a_rx) = broker.connect("a", None, None);
        assert!(matches!(
            next(&mut a_rx).await,
            TransportEvent::Connected { .. }
        ));
        a.publish("t/state", b"on".to_vec(), true).await.unwrap();
        let (b, mut b_rx) = broker.connect("b", None, None);
        let _ = next(&mut b_rx).await;
        b.subscribe("t/+").await.unwrap();
        // Retained message replayed on subscribe.
        match next(&mut b_rx).await {
            TransportEvent::Message {
                topic,
                payload,
                retain,
            } => {
                assert_eq!(
                    (topic.as_str(), payload.as_slice(), retain),
                    ("t/state", &b"on"[..], true)
                );
            }
            other => panic!("{other:?}"),
        }
        a.publish("t/live", b"x".to_vec(), false).await.unwrap();
        assert!(matches!(
            next(&mut b_rx).await,
            TransportEvent::Message { .. }
        ));
        // Clearing retained with an empty payload.
        a.publish("t/state", vec![], true).await.unwrap();
        assert_eq!(broker.retained("t/state"), None);
    }

    #[tokio::test]
    async fn will_is_published_on_drop_but_not_on_clean_disconnect() {
        let broker = LoopbackBroker::new();
        let will = |id: &str| Will {
            topic: format!("s/{id}"),
            payload: b"off".to_vec(),
            retain: true,
        };
        let (_w, mut w_rx) = broker.connect("watcher", None, None);
        let _ = next(&mut w_rx).await;
        let (watcher, _) = broker.connect("watcher2", None, None);
        watcher.subscribe("s/+").await.unwrap();

        let (_dirty, _d_rx) = broker.connect("dirty", Some(will("dirty")), None);
        broker.drop_connection("dirty");
        assert_eq!(broker.retained("s/dirty"), Some(b"off".to_vec()));

        let (clean, _c_rx) = broker.connect("clean", Some(will("clean")), None);
        clean.disconnect().await;
        assert_eq!(broker.retained("s/clean"), None);
        assert!(!clean.is_connected());
    }

    #[tokio::test]
    async fn acl_prefix_blocks_foreign_topics() {
        let broker = LoopbackBroker::new();
        let (c, _rx) = broker.connect("c", None, Some("ah/user1/"));
        assert!(c.publish("ah/user1/d/p", vec![1], false).await.is_ok());
        assert!(c.publish("ah/user2/d/p", vec![1], false).await.is_err());
        assert!(c.subscribe("ah/user1/s/+").await.is_ok());
        assert!(c.subscribe("other/#").await.is_err());
    }

    #[tokio::test]
    async fn fault_injection_drops_matching_publishes() {
        let broker = LoopbackBroker::new();
        let (a, _a_rx) = broker.connect("a", None, None);
        let (b, mut b_rx) = broker.connect("b", None, None);
        let _ = next(&mut b_rx).await;
        b.subscribe("x/#").await.unwrap();
        broker.drop_publishes_to("/d/");
        a.publish("x/d/1", vec![1], false).await.unwrap();
        a.publish("x/u/1", vec![2], false).await.unwrap();
        match next(&mut b_rx).await {
            TransportEvent::Message { topic, .. } => assert_eq!(topic, "x/u/1"),
            other => panic!("{other:?}"),
        }
        broker.clear_faults();
        a.publish("x/d/2", vec![3], false).await.unwrap();
        match next(&mut b_rx).await {
            TransportEvent::Message { topic, .. } => assert_eq!(topic, "x/d/2"),
            other => panic!("{other:?}"),
        }
    }

    #[tokio::test]
    async fn taking_over_a_client_id_drops_the_old_connection() {
        let broker = LoopbackBroker::new();
        let (_first, mut first_rx) = broker.connect("same", None, None);
        let _ = next(&mut first_rx).await;
        let (_second, _second_rx) = broker.connect("same", None, None);
        assert!(matches!(
            next(&mut first_rx).await,
            TransportEvent::Disconnected { .. }
        ));
    }
}
