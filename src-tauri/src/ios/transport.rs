//! MQTT transport of the iOS gateway (plan §4.2).
//!
//! `BrokerTransport` is the narrow surface the gateway needs. Two implementations:
//! - [`RumqttcTransport`]: the real client (TLS first, WSS fallback, jittered backoff).
//! - `LoopbackBroker` / `LoopbackTransport` (test builds only, `transport/loopback.rs`): an
//!   in-process broker with topic matching, retained messages, last wills and fault injection.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};

use super::wire::KEEPALIVE_SECS;

/// What the transport reports to the gateway.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransportEvent {
    Published {
        bytes: usize,
        announce: bool,
    },
    TrafficUncertain,
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

// clippy 1.99 reports `double_must_use` on the methods `async_trait` generates; nothing in this
// trait carries `#[must_use]` itself.
#[allow(clippy::double_must_use)]
#[async_trait]
pub trait BrokerTransport: Send + Sync {
    /// QoS 1 publish.
    async fn publish(&self, topic: &str, payload: Vec<u8>, retain: bool) -> Result<(), String>;
    /// Tags publications that only replicate accounting, to prevent feedback loops.
    async fn publish_with_traffic(
        &self,
        topic: &str,
        payload: Vec<u8>,
        retain: bool,
        _announce: bool,
    ) -> Result<(), String> {
        self.publish(topic, payload, retain).await
    }
    /// QoS 1 subscribe.
    async fn subscribe(&self, topic: &str) -> Result<(), String>;
    async fn unsubscribe(&self, topic: &str) -> Result<(), String>;
    /// Flushes pending publishes, disconnects cleanly (no last will) and stops reconnecting.
    async fn disconnect(&self);
    // Nothing polls the connection state (the gateway follows `TransportEvent`s); tests do.
    #[allow(dead_code)]
    fn is_connected(&self) -> bool;
}

#[cfg(test)]
mod loopback;
#[cfg(test)]
pub use loopback::{topic_matches, LoopbackBroker, LoopbackTransport};

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

#[derive(Clone, Copy)]
struct Publication {
    bytes: usize,
    announce: bool,
}

#[derive(Clone)]
struct ConnectionClient {
    client: rumqttc::AsyncClient,
    publications: Arc<Mutex<VecDeque<Publication>>>,
}

struct Shared {
    client: Mutex<Option<ConnectionClient>>,
    enqueue: tokio::sync::Mutex<()>,
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
            enqueue: tokio::sync::Mutex::new(()),
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

    fn client(&self) -> Result<ConnectionClient, String> {
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
        self.publish_with_traffic(topic, payload, retain, true)
            .await
    }

    async fn publish_with_traffic(
        &self,
        topic: &str,
        payload: Vec<u8>,
        retain: bool,
        announce: bool,
    ) -> Result<(), String> {
        let _enqueue = self.shared.enqueue.lock().await;
        let connection = self.client()?;
        connection
            .publications
            .lock()
            .unwrap()
            .push_back(Publication {
                bytes: payload.len(),
                announce,
            });
        let result = connection
            .client
            .publish(topic, rumqttc::QoS::AtLeastOnce, retain, payload)
            .await
            .map_err(|error| error.to_string());
        if result.is_err() {
            connection.publications.lock().unwrap().pop_back();
        }
        result
    }

    async fn subscribe(&self, topic: &str) -> Result<(), String> {
        self.client()?
            .client
            .subscribe(topic, rumqttc::QoS::AtLeastOnce)
            .await
            .map_err(|e| e.to_string())
    }

    async fn unsubscribe(&self, topic: &str) -> Result<(), String> {
        self.client()?
            .client
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

fn account_publication(
    event: rumqttc::Event,
    publications: &Mutex<VecDeque<Publication>>,
    inflight: &mut HashMap<u16, Publication>,
    events: &UnboundedSender<TransportEvent>,
) {
    match event {
        rumqttc::Event::Outgoing(rumqttc::Outgoing::Publish(id)) => {
            if let Some(publication) = publications.lock().unwrap().pop_front() {
                inflight.insert(id, publication);
            }
        }
        rumqttc::Event::Incoming(rumqttc::Packet::PubAck(ack)) => {
            if let Some(publication) = inflight.remove(&ack.pkid) {
                let _ = events.send(TransportEvent::Published {
                    bytes: publication.bytes,
                    announce: publication.announce,
                });
            }
        }
        _ => {}
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

/// The transport to start with. Plain TCP is for loopback test brokers only: a remote host always
/// gets TLS, whatever the config says.
fn first_via(params: &ConnectParams) -> Via {
    if params.insecure_local && is_loopback_host(&params.host) {
        Via::Plain
    } else if params.prefer.as_deref() == Some("wss") {
        Via::Wss
    } else {
        Via::Tls
    }
}

async fn supervise(
    params: ConnectParams,
    shared: Arc<Shared>,
    events: UnboundedSender<TransportEvent>,
    mut stop: UnboundedReceiver<tokio::sync::oneshot::Sender<()>>,
) {
    use rumqttc::{AsyncClient, Event, NetworkOptions, Packet};

    let mut via = first_via(&params);
    let mut attempt = 0u32;
    loop {
        let options = build_options(&params, via);
        let (client, mut eventloop) = AsyncClient::new(options, 256);
        let mut network = NetworkOptions::new();
        network.set_connection_timeout(5);
        network.set_tcp_nodelay(true);
        eventloop.set_network_options(network);
        let publications = Arc::new(Mutex::new(VecDeque::new()));
        let mut inflight = HashMap::new();
        *shared.client.lock().unwrap() = Some(ConnectionClient {
            client: client.clone(),
            publications: publications.clone(),
        });

        let mut connected_once = false;
        let (reason, auth_failed) = loop {
            tokio::select! {
                biased;
                done = stop.recv() => {
                    // Graceful stop: flush the queue, send DISCONNECT (the will is discarded).
                    let _ = client.disconnect().await;
                    let _ = tokio::time::timeout(Duration::from_secs(2), async {
                        while let Ok(event) = eventloop.poll().await {
                            account_publication(event, &publications, &mut inflight, &events);
                        }
                    })
                    .await;
                    if !inflight.is_empty() || !publications.lock().unwrap().is_empty() {
                        let _ = events.send(TransportEvent::TrafficUncertain);
                    }
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
                    Ok(event) => account_publication(event, &publications, &mut inflight, &events),
                    Err(error) => {
                        if !inflight.is_empty() || !publications.lock().unwrap().is_empty() {
                            let _ = events.send(TransportEvent::TrafficUncertain);
                        }
                        break classify(&error);
                    },
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

/// What the isolation self-check saw (docs/specs/ios-open-access.md §6.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Isolation {
    /// Nothing came back from a topic outside the account's own namespace: the broker refused the
    /// subscription or the publish, dropped the message, or closed the connection. The answer we
    /// want, and also the default when the broker was too slow to say.
    Isolated,
    /// A message published outside the account's own namespace came back: the account can read
    /// and write other accounts' topics.
    Leaks,
}

/// What a successful connectivity probe learned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProbeReport {
    /// Transport that worked: `tls`, `wss` or `tcp`.
    pub via: String,
    /// Time from the first connection attempt to the message coming back.
    pub elapsed_ms: u64,
    /// Whether the account is held to its own namespace.
    pub isolation: Isolation,
}

/// Why a probe failed, kept apart so the caller can word it in the user's language and the
/// settings page can add a hint that fits the broker service.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProbeError {
    /// The broker refused the login.
    Rejected { reason: String },
    /// Logged in, but the message sent to the account's own topic never came back: the access
    /// rules do not let the account use it.
    NoEcho { via: String },
    /// Never got as far as a login before the time ran out; `last_error` is the last thing the
    /// connection reported, when it reported one.
    TimedOut { last_error: String },
    /// Anything else (a request the transport refused, the supervisor stopping).
    Failed { reason: String },
}

/// How long the isolation check waits for an echo that should never come. A refusal is quick, so
/// this only bounds the slowest honest "no": a broker that is slower than this is taken for
/// isolated (a leak is only reported when the echo really arrived).
const ISOLATION_WAIT: Duration = Duration::from_millis(1500);

/// "Test connection" of the settings page: connects like the gateway would (TLS first, WSS as the
/// fallback) and sends one message to itself through the broker, which proves the credentials and
/// the access rules in one go; then checks that the account cannot reach outside its own
/// namespace. Uses a throwaway client id and writes nothing retained.
pub async fn probe(mut params: ConnectParams, limit: Duration) -> Result<ProbeReport, ProbeError> {
    let id = super::crypto::new_id();
    let topic = format!("ah/{}/t/{id}", params.username);
    // Another account's namespace: a random name nobody has, so nothing real is ever touched.
    let outside = format!("ah/{}/t/{id}", super::crypto::new_id());
    params.client_id = format!("probe-{id}");
    params.will = Will {
        topic: topic.clone(),
        payload: Vec::new(),
        retain: false,
    };
    params.prefer = None;
    let (transport, events) = RumqttcTransport::spawn(params);
    let outcome = probe_over(&*transport, events, &topic, &outside, limit, ISOLATION_WAIT).await;
    transport.disconnect().await;
    outcome
}

/// The probe over any transport (the tests use the in-process broker).
async fn probe_over(
    transport: &dyn BrokerTransport,
    mut events: UnboundedReceiver<TransportEvent>,
    topic: &str,
    outside: &str,
    limit: Duration,
    isolation_wait: Duration,
) -> Result<ProbeReport, ProbeError> {
    let started = std::time::Instant::now();
    let deadline = tokio::time::Instant::now() + limit;
    let mut via: Option<String> = None;
    let mut last_error = String::new();
    loop {
        let event = tokio::select! {
            event = events.recv() => event,
            _ = tokio::time::sleep_until(deadline) => return Err(match via {
                Some(via) => ProbeError::NoEcho { via },
                None => ProbeError::TimedOut { last_error },
            }),
        };
        match event {
            Some(TransportEvent::Connected { via: used }) => {
                via = Some(used);
                let sent = async {
                    transport.subscribe(topic).await?;
                    transport.publish(topic, b"probe".to_vec(), false).await
                }
                .await;
                if let Err(reason) = sent {
                    return Err(ProbeError::Failed { reason });
                }
            }
            Some(TransportEvent::Message { topic: got, .. }) if got == topic => {
                let elapsed_ms = started.elapsed().as_millis() as u64;
                return Ok(ProbeReport {
                    via: via.unwrap_or_default(),
                    elapsed_ms,
                    isolation: check_isolation(transport, &mut events, outside, isolation_wait)
                        .await,
                });
            }
            Some(
                TransportEvent::Message { .. }
                | TransportEvent::Published { .. }
                | TransportEvent::TrafficUncertain,
            ) => {}
            Some(TransportEvent::Disconnected {
                reason,
                auth_failed,
            }) => {
                if auth_failed {
                    return Err(ProbeError::Rejected { reason });
                }
                last_error = reason;
            }
            None => {
                return Err(ProbeError::Failed {
                    reason: "the connection supervisor stopped".to_string(),
                })
            }
        }
    }
}

/// Subscribes to a topic outside the account's namespace, publishes one non-retained QoS 1 message
/// to it and waits briefly for the echo. Brokers separate accounts in three ways, and all three
/// read as "isolated" here: refusing the subscription or the publish, accepting both and then
/// never delivering across accounts, or closing the connection.
async fn check_isolation(
    transport: &dyn BrokerTransport,
    events: &mut UnboundedReceiver<TransportEvent>,
    outside: &str,
    wait: Duration,
) -> Isolation {
    if transport.subscribe(outside).await.is_err()
        || transport
            .publish(outside, b"probe".to_vec(), false)
            .await
            .is_err()
    {
        return Isolation::Isolated;
    }
    let deadline = tokio::time::Instant::now() + wait;
    loop {
        tokio::select! {
            event = events.recv() => match event {
                Some(TransportEvent::Message { topic, .. }) if topic == outside => {
                    return Isolation::Leaks;
                }
                Some(TransportEvent::Disconnected { .. }) | None => return Isolation::Isolated,
                Some(_) => {}
            },
            _ = tokio::time::sleep_until(deadline) => return Isolation::Isolated,
        }
    }
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

    fn params(host: &str, insecure_local: bool, prefer: Option<&str>) -> ConnectParams {
        ConnectParams {
            host: host.into(),
            tls_port: 8883,
            wss_port: 8084,
            username: "u".into(),
            password: "p".into(),
            client_id: "c".into(),
            will: Will {
                topic: String::new(),
                payload: Vec::new(),
                retain: false,
            },
            prefer: prefer.map(str::to_string),
            insecure_local,
        }
    }

    #[test]
    fn plain_tcp_is_only_ever_used_towards_loopback() {
        assert_eq!(first_via(&params("127.0.0.1", true, None)), Via::Plain);
        assert_eq!(first_via(&params("localhost", true, None)), Via::Plain);
        // A remote host gets TLS even when the config asks for plain TCP.
        assert_eq!(
            first_via(&params("broker.example.com", true, None)),
            Via::Tls
        );
        assert_eq!(
            first_via(&params("broker.example.com", true, Some("wss"))),
            Via::Wss
        );
        assert_eq!(first_via(&params("127.0.0.1", false, None)), Via::Tls);
        assert_eq!(
            first_via(&params("127.0.0.1", false, Some("wss"))),
            Via::Wss
        );
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

    #[test]
    fn a_queued_or_written_publication_is_counted_only_after_puback() {
        let publications = Mutex::new(VecDeque::from([Publication {
            bytes: 42,
            announce: false,
        }]));
        let mut inflight = HashMap::new();
        let (tx, mut rx) = unbounded_channel();
        assert!(rx.try_recv().is_err());
        account_publication(
            rumqttc::Event::Outgoing(rumqttc::Outgoing::Publish(7)),
            &publications,
            &mut inflight,
            &tx,
        );
        assert!(rx.try_recv().is_err());
        account_publication(
            rumqttc::Event::Incoming(rumqttc::Packet::PubAck(rumqttc::PubAck::new(7))),
            &publications,
            &mut inflight,
            &tx,
        );
        assert_eq!(
            rx.try_recv().unwrap(),
            TransportEvent::Published {
                bytes: 42,
                announce: false
            }
        );
        account_publication(
            rumqttc::Event::Incoming(rumqttc::Packet::PubAck(rumqttc::PubAck::new(7))),
            &publications,
            &mut inflight,
            &tx,
        );
        assert!(rx.try_recv().is_err());
    }

    async fn next(rx: &mut UnboundedReceiver<TransportEvent>) -> TransportEvent {
        loop {
            let event = tokio::time::timeout(Duration::from_secs(1), rx.recv())
                .await
                .expect("event in time")
                .expect("channel open");
            if !matches!(
                event,
                TransportEvent::Published { .. } | TransportEvent::TrafficUncertain
            ) {
                return event;
            }
        }
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

    // ---- the probe behind "Test connection", and its isolation self-check

    const OWN: &str = "ah/alice/t/own";
    const OUTSIDE: &str = "ah/zed/t/other";
    /// Short enough for a test, long enough for the in-process broker to answer.
    const BRIEF: Duration = Duration::from_millis(150);
    const PLENTY: Duration = Duration::from_secs(5);

    async fn run_probe(
        transport: &dyn BrokerTransport,
        events: UnboundedReceiver<TransportEvent>,
        limit: Duration,
    ) -> Result<ProbeReport, ProbeError> {
        probe_over(transport, events, OWN, OUTSIDE, limit, BRIEF).await
    }

    /// A broker that closes the connection of an account publishing outside its namespace, which some
    /// brokers do instead of refusing quietly.
    struct ClosesOnForeignPublish {
        inner: Arc<LoopbackTransport>,
        broker: LoopbackBroker,
        client_id: String,
    }

    #[async_trait]
    impl BrokerTransport for ClosesOnForeignPublish {
        async fn publish(&self, topic: &str, payload: Vec<u8>, retain: bool) -> Result<(), String> {
            if !topic.starts_with("ah/alice/") {
                self.broker.drop_connection(&self.client_id);
                return Ok(());
            }
            self.inner.publish(topic, payload, retain).await
        }
        async fn subscribe(&self, topic: &str) -> Result<(), String> {
            self.inner.subscribe(topic).await
        }
        async fn unsubscribe(&self, topic: &str) -> Result<(), String> {
            self.inner.unsubscribe(topic).await
        }
        async fn disconnect(&self) {
            self.inner.disconnect().await
        }
        fn is_connected(&self) -> bool {
            self.inner.is_connected()
        }
    }

    #[tokio::test]
    async fn an_account_held_to_its_own_namespace_is_isolated() {
        // The strict kind: the broker refuses the subscription or the publish outside `ah/alice/`.
        let broker = LoopbackBroker::new();
        let (transport, events) = broker.connect("probe", None, Some("ah/alice/"));
        let report = run_probe(&*transport, events, PLENTY).await.unwrap();
        assert_eq!(report.via, "loopback");
        assert_eq!(report.isolation, Isolation::Isolated);
    }

    #[tokio::test]
    async fn an_account_that_reaches_other_namespaces_is_flagged() {
        // No access rules at all: the message to somebody else's namespace comes straight back.
        let broker = LoopbackBroker::new();
        let (transport, events) = broker.connect("probe", None, None);
        let report = run_probe(&*transport, events, PLENTY).await.unwrap();
        assert_eq!(report.isolation, Isolation::Leaks);
    }

    #[tokio::test]
    async fn accepting_and_then_never_delivering_across_accounts_counts_as_isolated() {
        // The delivery-layer kind (flespi, Mosquitto with a pattern ACL): subscribing and publishing
        // both succeed, the message just never arrives.
        let broker = LoopbackBroker::new();
        let (transport, events) = broker.connect("probe", None, None);
        broker.drop_publishes_to("ah/zed/");
        let report = run_probe(&*transport, events, PLENTY).await.unwrap();
        assert_eq!(report.isolation, Isolation::Isolated);
        // The account's own topic still worked, and only the outside one was held back.
        assert_eq!(report.via, "loopback");
    }

    #[tokio::test]
    async fn being_disconnected_for_publishing_outside_counts_as_isolated() {
        let broker = LoopbackBroker::new();
        let (inner, events) = broker.connect("probe", None, None);
        let transport = ClosesOnForeignPublish {
            inner,
            broker: broker.clone(),
            client_id: "probe".into(),
        };
        // The disconnect ends the wait at once: a generous wait must not be used up.
        let report = tokio::time::timeout(
            Duration::from_secs(2),
            probe_over(
                &transport,
                events,
                OWN,
                OUTSIDE,
                PLENTY,
                Duration::from_secs(30),
            ),
        )
        .await
        .expect("a closed connection ends the isolation check early")
        .unwrap();
        assert_eq!(report.isolation, Isolation::Isolated);
    }

    #[tokio::test]
    async fn the_isolation_check_leaves_nothing_behind() {
        // Neither of the probe's messages is retained, so there is nothing to clean up on the broker.
        let broker = LoopbackBroker::new();
        let (transport, events) = broker.connect("probe", None, None);
        run_probe(&*transport, events, PLENTY).await.unwrap();
        assert_eq!(broker.retained(OWN), None);
        assert_eq!(broker.retained(OUTSIDE), None);
    }

    #[tokio::test]
    async fn no_echo_on_the_accounts_own_topic_names_the_path_that_connected() {
        // Logged in, but the access rules do not let the account use its own namespace.
        let broker = LoopbackBroker::new();
        let (transport, events) = broker.connect("probe", None, None);
        broker.drop_publishes_to("ah/alice/");
        let error = run_probe(&*transport, events, BRIEF).await.unwrap_err();
        assert_eq!(
            error,
            ProbeError::NoEcho {
                via: "loopback".into()
            }
        );
    }

    #[tokio::test]
    async fn a_refused_login_and_a_connection_that_never_came_up_are_told_apart() {
        let broker = LoopbackBroker::new();
        let (transport, _unused) = broker.connect("probe", None, None);

        let (tx, rx) = unbounded_channel();
        tx.send(TransportEvent::Disconnected {
            reason: "bad credentials".into(),
            auth_failed: true,
        })
        .unwrap();
        assert_eq!(
            run_probe(&*transport, rx, PLENTY).await.unwrap_err(),
            ProbeError::Rejected {
                reason: "bad credentials".into()
            }
        );

        let (tx, rx) = unbounded_channel();
        tx.send(TransportEvent::Disconnected {
            reason: "connection reset".into(),
            auth_failed: false,
        })
        .unwrap();
        assert_eq!(
            run_probe(&*transport, rx, BRIEF).await.unwrap_err(),
            ProbeError::TimedOut {
                last_error: "connection reset".into()
            }
        );
        drop(tx);
    }
}
