//! In-process MQTT broker for tests: topic matching, retained messages, last wills, ACL
//! prefixes and fault injection. The production transport is [`super::RumqttcTransport`].

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};

use super::{BrokerTransport, TransportEvent, Will};

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
            Self::route_locked(inner, &will.topic, will.payload, will.retain);
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

    pub fn is_connected(&self, client_id: &str) -> bool {
        self.inner
            .lock()
            .unwrap()
            .clients
            .get(client_id)
            .is_some_and(|c| c.connected)
    }

    fn route_locked(inner: &mut BrokerInner, topic: &str, payload: Vec<u8>, retain: bool) {
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
        let bytes = payload.len();
        let mut inner = self.broker.inner.lock().unwrap();
        let Some(client) = inner.clients.get(&self.client_id) else {
            return Err("not connected".into());
        };
        if !Self::allowed(client, topic) {
            return Err(format!("not authorized to publish {topic}"));
        }
        let _ = self.tx.send(TransportEvent::Published { bytes, announce });
        if inner.drop_topics.iter().any(|f| topic.contains(f.as_str())) {
            return Ok(());
        }
        LoopbackBroker::route_locked(&mut inner, topic, payload, retain);
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
