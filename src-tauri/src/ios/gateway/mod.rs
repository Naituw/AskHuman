//! The iOS gateway: one long-lived task per daemon that owns the MQTT connection, the pairings
//! and everything the phones can see or ask for (plan §4.3).
//!
//! The task is an actor: transport events, commands from the daemon and a 250 ms tick are
//! handled one at a time, so state needs no locks. The daemon talks to it through the cloneable
//! [`IosGateway`] handle and answers its questions through the [`Host`] trait.

mod assets;
mod pairing;
pub mod requests;
mod support;
#[cfg(test)]
mod testkit;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_assets;
#[cfg(test)]
mod tests_requests;

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};
use tokio::sync::oneshot;

use super::crypto;
use super::envelope::{self, Plain, Scope};
use super::push::{PushClient, PushOutcome};
use super::store::{PhoneState, Store, Traffic};
use super::transport::{BrokerTransport, TransportEvent};
use super::wire::{self, t, Caps, Msg, Presence, SlotInfo, CLOCK_SKEW_MS, KID_PAIR, PROTO};
use crate::config::IosChannelConfig;

use assets::Uploads;
pub use pairing::PairInfo;
use requests::{DeliverRequest, Resolution};
use support::{Dedup, Outbox};

// ---------------------------------------------------------------- host

/// What the gateway needs to know about the daemon around it.
pub trait Host: Send + Sync + 'static {
    /// Unix milliseconds. Tests substitute a virtual clock.
    fn now_ms(&self) -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0)
    }
    /// Display name of this Mac on the phone.
    fn mac_name(&self) -> String;
    fn caps(&self) -> Caps;
    /// Whether iOS is the active channel slot and whether the slot follows `/here` automatically.
    fn slot(&self) -> SlotInfo;
}

// ---------------------------------------------------------------- views for IPC and settings

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PhoneView {
    pub phone_id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alias_name: Option<String>,
    /// `pendingConfirm` or `active`.
    pub state: String,
    pub online: bool,
    pub foreground: bool,
    pub created_at: i64,
    pub last_seen_at: i64,
    /// Push registration status (`ok`, `gone`, `error`) or `None` without a handle.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub push: Option<String>,
    /// `keychain` or `file` (plaintext fallback).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key_backend: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PairingView {
    pub ticket_id: String,
    pub code: String,
    pub expires_at_ms: i64,
    /// `waiting`, `pending` (phone knocked, needs a decision) or `accepted`.
    pub stage: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phone_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sas: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GatewayState {
    pub connected: bool,
    /// Transport in use: `tls`, `wss` or `tcp`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub via: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    pub mac_id: String,
    pub mac_name: String,
    pub phones: Vec<PhoneView>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pairing: Option<PairingView>,
    pub traffic: Traffic,
    pub pending_requests: usize,
}

// ---------------------------------------------------------------- handle

/// Flags the daemon reads without asking the actor.
#[derive(Default)]
pub struct SharedStatus {
    connected: AtomicBool,
    has_active_phone: AtomicBool,
    live_watch: std::sync::atomic::AtomicUsize,
}

pub(crate) enum Cmd {
    PairStart(oneshot::Sender<Result<PairInfo, String>>),
    PairCancel,
    PairDecision {
        ticket_id: String,
        allow: bool,
    },
    Revoke {
        phone_id: String,
        reply: oneshot::Sender<bool>,
    },
    Rename {
        phone_id: String,
        name: String,
    },
    State(oneshot::Sender<GatewayState>),
    Deliver(Box<DeliverRequest>),
    Resolve {
        request_id: String,
        resolution: Resolution,
        done: Option<oneshot::Sender<()>>,
    },
    PushResult {
        phone_id: String,
        outcome: PushOutcome,
    },
    Shutdown(oneshot::Sender<()>),
    /// Shutdown of a gateway that is being replaced (see [`IosGateway::retire`]).
    Retire(oneshot::Sender<()>),
}

/// Everything the gateway is built from.
pub struct GatewayDeps {
    pub host: Arc<dyn Host>,
    pub cfg: IosChannelConfig,
    pub store: Store,
    pub transport: Arc<dyn BrokerTransport>,
    pub events: UnboundedReceiver<TransportEvent>,
    pub pusher: Arc<dyn PushClient>,
}

/// Cloneable handle to the gateway task.
#[derive(Clone)]
pub struct IosGateway {
    cmd: UnboundedSender<Cmd>,
    status: Arc<SharedStatus>,
}

impl IosGateway {
    /// Starts the gateway actor. The transport and its event stream are created by the caller
    /// (the real client in the daemon, the loopback broker in tests).
    pub fn start(deps: GatewayDeps) -> Self {
        let (cmd_tx, cmd_rx) = unbounded_channel();
        let status = Arc::new(SharedStatus::default());
        let GatewayDeps {
            host,
            cfg,
            store,
            transport,
            events,
            pusher,
        } = deps;
        let actor = Actor::new(
            host,
            cfg,
            store,
            transport,
            pusher,
            status.clone(),
            cmd_tx.clone(),
        );
        tokio::spawn(actor.run(cmd_rx, events));
        Self {
            cmd: cmd_tx,
            status,
        }
    }

    /// Connected to the broker with at least one active pairing: iOS can carry requests.
    pub fn is_ready(&self) -> bool {
        self.status.connected.load(Ordering::Relaxed)
            && self.status.has_active_phone.load(Ordering::Relaxed)
    }

    pub fn is_connected(&self) -> bool {
        self.status.connected.load(Ordering::Relaxed)
    }

    /// At least one phone is paired: requests have somewhere to go (even if the broker is down,
    /// the push path may still work).
    pub fn has_active_phone(&self) -> bool {
        self.status.has_active_phone.load(Ordering::Relaxed)
    }

    /// Whether any phone is watching an Agent live (keeps an idle daemon alive).
    pub fn live_watch_count(&self) -> usize {
        self.status.live_watch.load(Ordering::Relaxed)
    }

    pub async fn state(&self) -> Option<GatewayState> {
        let (tx, rx) = oneshot::channel();
        self.cmd.send(Cmd::State(tx)).ok()?;
        rx.await.ok()
    }

    pub async fn pair_start(&self) -> Result<PairInfo, String> {
        let (tx, rx) = oneshot::channel();
        self.cmd
            .send(Cmd::PairStart(tx))
            .map_err(|_| "gateway stopped".to_string())?;
        rx.await.map_err(|_| "gateway stopped".to_string())?
    }

    pub fn pair_cancel(&self) {
        let _ = self.cmd.send(Cmd::PairCancel);
    }

    pub fn pair_decision(&self, ticket_id: &str, allow: bool) {
        let _ = self.cmd.send(Cmd::PairDecision {
            ticket_id: ticket_id.to_string(),
            allow,
        });
    }

    pub async fn revoke(&self, phone_id: &str) -> bool {
        let (tx, rx) = oneshot::channel();
        if self
            .cmd
            .send(Cmd::Revoke {
                phone_id: phone_id.to_string(),
                reply: tx,
            })
            .is_err()
        {
            return false;
        }
        rx.await.unwrap_or(false)
    }

    pub fn rename(&self, phone_id: &str, name: &str) {
        let _ = self.cmd.send(Cmd::Rename {
            phone_id: phone_id.to_string(),
            name: name.to_string(),
        });
    }

    /// Hands a request to the gateway for delivery to the paired phones.
    pub fn deliver(&self, request: DeliverRequest) {
        let _ = self.cmd.send(Cmd::Deliver(Box::new(request)));
    }

    /// The request reached a terminal state somewhere (answered, cancelled, expired).
    pub fn resolve(&self, request_id: &str, resolution: Resolution) {
        let _ = self.cmd.send(Cmd::Resolve {
            request_id: request_id.to_string(),
            resolution,
            done: None,
        });
    }

    /// Like [`Self::resolve`], returning once the phones have been told.
    pub async fn resolve_and_wait(&self, request_id: &str, resolution: Resolution) {
        let (tx, rx) = oneshot::channel();
        if self
            .cmd
            .send(Cmd::Resolve {
                request_id: request_id.to_string(),
                resolution,
                done: Some(tx),
            })
            .is_ok()
        {
            let _ = tokio::time::timeout(Duration::from_secs(3), rx).await;
        }
    }

    /// Like [`Self::shutdown`] for a gateway that is being replaced: the requests it still
    /// holds are reported as lost surfaces, so the daemon can hand them to the successor.
    pub async fn retire(&self) {
        let (tx, rx) = oneshot::channel();
        if self.cmd.send(Cmd::Retire(tx)).is_ok() {
            let _ = tokio::time::timeout(Duration::from_secs(6), rx).await;
        }
    }

    /// Publishes offline presence, flushes and disconnects.
    pub async fn shutdown(&self) {
        let (tx, rx) = oneshot::channel();
        if self.cmd.send(Cmd::Shutdown(tx)).is_ok() {
            let _ = tokio::time::timeout(Duration::from_secs(6), rx).await;
        }
    }
}

// ---------------------------------------------------------------- actor

/// Runtime view of a stored phone.
#[derive(Default)]
pub(crate) struct PhoneRt {
    pub online: bool,
    pub foreground: bool,
    pub presence_ts: i64,
    /// Time of the last accepted `hello` of the current online session.
    pub hello_at: Option<i64>,
    pub device_name: String,
    pub dedup: Dedup,
}

#[derive(Default)]
pub(crate) struct Revs {
    pub inbox: u64,
    pub agents: u64,
    pub follow: u64,
    pub slot: u64,
}

pub(crate) struct Actor {
    pub host: Arc<dyn Host>,
    pub cfg: IosChannelConfig,
    pub user: String,
    pub mac_id: String,
    pub epoch: String,
    pub store: Store,
    pub transport: Arc<dyn BrokerTransport>,
    pub outbox: Outbox,
    pub status: Arc<SharedStatus>,
    pub pusher: Arc<dyn PushClient>,
    pub cmd_tx: UnboundedSender<Cmd>,

    pub connected: bool,
    pub via: Option<String>,
    pub last_error: Option<String>,
    /// Snapshots go to online phones shortly after (re)connecting, once presence has arrived.
    pub sync_at: Option<i64>,

    pub rt: HashMap<String, PhoneRt>,
    pub pairing: Option<pairing::Pairing>,
    pub revs: Revs,
    pub requests: requests::Requests,
    pub uploads: Uploads,
    /// Last slot state announced to the phones (`None` until the first tick).
    pub last_slot: Option<SlotInfo>,

    pub dirty: bool,
    pub last_flush_ms: i64,
    pub last_prune_ms: i64,
}

fn month_of(ms: i64) -> String {
    chrono::DateTime::<chrono::Utc>::from_timestamp_millis(ms)
        .map(|dt| dt.format("%Y-%m").to_string())
        .unwrap_or_default()
}

/// Display names come from the network: keep them short and printable.
pub(crate) fn sanitize_name(raw: &str) -> String {
    let cleaned: String = raw
        .chars()
        .filter(|c| !c.is_control())
        .take(64)
        .collect::<String>()
        .trim()
        .to_string();
    if cleaned.is_empty() {
        "iPhone".to_string()
    } else {
        cleaned
    }
}

impl Actor {
    fn new(
        host: Arc<dyn Host>,
        cfg: IosChannelConfig,
        store: Store,
        transport: Arc<dyn BrokerTransport>,
        pusher: Arc<dyn PushClient>,
        status: Arc<SharedStatus>,
        cmd_tx: UnboundedSender<Cmd>,
    ) -> Self {
        let now = host.now_ms();
        let mac_id = store.state.mac_id.clone();
        let outbox = Outbox::spawn(transport.clone());
        let mut actor = Self {
            user: cfg.username.clone(),
            mac_id,
            epoch: hex16(),
            store,
            transport,
            outbox,
            status,
            pusher,
            cmd_tx,
            host,
            cfg,
            connected: false,
            via: None,
            last_error: None,
            sync_at: None,
            rt: HashMap::new(),
            pairing: None,
            revs: Revs::default(),
            requests: requests::Requests::default(),
            uploads: Uploads::default(),
            last_slot: None,
            dirty: false,
            last_flush_ms: now,
            last_prune_ms: now,
        };
        let ids: Vec<String> = actor
            .store
            .state
            .phones
            .iter()
            .map(|p| p.phone_id.clone())
            .collect();
        for id in ids {
            actor.rt.insert(id, PhoneRt::default());
        }
        // A pairing that never completed (daemon stopped mid-handshake) is discarded on start.
        let stale: Vec<String> = actor
            .store
            .state
            .phones
            .iter()
            .filter(|p| p.state == PhoneState::PendingConfirm)
            .map(|p| p.phone_id.clone())
            .collect();
        for id in stale {
            actor.store.remove_phone(&id);
            actor.rt.remove(&id);
        }
        actor.store.journal_prune(now);
        actor.sweep_upload_files();
        actor.refresh_status();
        actor
    }

    pub(crate) fn now(&self) -> i64 {
        self.host.now_ms()
    }

    pub(crate) fn refresh_status(&self) {
        self.status
            .connected
            .store(self.connected, Ordering::Relaxed);
        self.status
            .has_active_phone
            .store(self.store.state.has_active_phone(), Ordering::Relaxed);
    }

    pub(crate) fn mac_name(&self) -> String {
        let configured = self.cfg.mac_name.trim();
        if configured.is_empty() {
            self.host.mac_name()
        } else {
            configured.to_string()
        }
    }

    async fn run(
        mut self,
        mut cmds: UnboundedReceiver<Cmd>,
        mut events: UnboundedReceiver<TransportEvent>,
    ) {
        let mut ticker = tokio::time::interval(Duration::from_millis(250));
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                Some(event) = events.recv() => self.on_transport(event).await,
                cmd = cmds.recv() => match cmd {
                    Some(Cmd::Shutdown(done)) => {
                        self.shutdown().await;
                        let _ = done.send(());
                        break;
                    }
                    Some(Cmd::Retire(done)) => {
                        self.drop_requests("iOS gateway restarted");
                        self.shutdown().await;
                        let _ = done.send(());
                        break;
                    }
                    Some(cmd) => self.on_cmd(cmd).await,
                    None => {
                        self.shutdown().await;
                        break;
                    }
                },
                _ = ticker.tick() => self.tick().await,
            }
        }
    }

    async fn on_cmd(&mut self, cmd: Cmd) {
        match cmd {
            Cmd::PairStart(reply) => {
                let result = self.pair_start().await;
                let _ = reply.send(result);
            }
            Cmd::PairCancel => self.pair_cancel().await,
            Cmd::PairDecision { ticket_id, allow } => self.pair_decision(&ticket_id, allow).await,
            Cmd::Revoke { phone_id, reply } => {
                let _ = reply.send(self.revoke(&phone_id).await);
            }
            Cmd::Rename { phone_id, name } => {
                let name = sanitize_name(&name);
                if let Some(phone) = self.store.state.phone_mut(&phone_id) {
                    phone.alias_name = Some(name);
                    self.store.save_state();
                }
            }
            Cmd::State(reply) => {
                let _ = reply.send(self.state_view());
            }
            Cmd::Deliver(request) => self.deliver(*request).await,
            Cmd::Resolve {
                request_id,
                resolution,
                done,
            } => {
                self.resolve(&request_id, resolution).await;
                if let Some(done) = done {
                    let _ = done.send(());
                }
            }
            Cmd::PushResult { phone_id, outcome } => self.on_push_result(&phone_id, outcome),
            Cmd::Shutdown(_) | Cmd::Retire(_) => unreachable!("handled in run"),
        }
    }

    pub(crate) fn state_view(&self) -> GatewayState {
        let phones = self
            .store
            .state
            .phones
            .iter()
            .map(|p| {
                let rt = self.rt.get(&p.phone_id);
                PhoneView {
                    phone_id: p.phone_id.clone(),
                    name: p.name.clone(),
                    alias_name: p.alias_name.clone(),
                    state: match p.state {
                        PhoneState::PendingConfirm => "pendingConfirm",
                        PhoneState::Active => "active",
                    }
                    .into(),
                    online: rt.is_some_and(|r| r.online),
                    foreground: rt.is_some_and(|r| r.foreground),
                    created_at: p.created_at,
                    last_seen_at: p.last_seen_at,
                    push: p.push.as_ref().map(|reg| reg.status.clone()),
                    key_backend: self.store.key_backend(&p.phone_id).map(|b| {
                        match b {
                            super::store::KeyBackend::Keychain => "keychain",
                            super::store::KeyBackend::File => "file",
                        }
                        .to_string()
                    }),
                }
            })
            .collect();
        GatewayState {
            connected: self.connected,
            via: self.via.clone(),
            last_error: self.last_error.clone(),
            mac_id: self.mac_id.clone(),
            mac_name: self.mac_name(),
            phones,
            pairing: self.pairing_view(),
            traffic: self.store.state.traffic.clone(),
            pending_requests: self.requests.pending_count(),
        }
    }

    // ------------------------------------------------------------ transport events

    async fn on_transport(&mut self, event: TransportEvent) {
        match event {
            TransportEvent::Connected { via } => self.on_connected(via).await,
            TransportEvent::Disconnected {
                reason,
                auth_failed,
            } => self.on_disconnected(reason, auth_failed),
            TransportEvent::Message {
                topic,
                payload,
                retain: _,
            } => self.on_message(&topic, &payload).await,
        }
    }

    async fn on_connected(&mut self, via: String) {
        self.connected = true;
        self.last_error = None;
        if (via == "tls" || via == "wss")
            && self.store.state.last_transport.as_deref() != Some(via.as_str())
        {
            self.store.state.last_transport = Some(via.clone());
            self.store.save_state();
        }
        self.via = Some(via);
        // Clean session: every subscription has to be issued again.
        let mut topics = vec![
            wire::topic_up(&self.user, &self.mac_id),
            wire::topic_presence_filter(&self.user),
        ];
        if let Some(pairing) = &self.pairing {
            topics.push(wire::topic_pair(&self.user, &pairing.ticket_id));
        }
        for topic in topics {
            let _ = self.transport.subscribe(&topic).await;
        }
        self.publish_presence(true);
        self.sync_at = Some(self.now() + 1_000);
        self.refresh_status();
    }

    fn on_disconnected(&mut self, reason: String, auth_failed: bool) {
        self.connected = false;
        self.via = None;
        self.last_error = Some(if auth_failed {
            "Broker rejected the credentials (wrong username or password, or the plan's quota is used up)".to_string()
        } else {
            reason
        });
        // Presence is only trustworthy while connected; the retained messages come back after
        // the next subscribe.
        for rt in self.rt.values_mut() {
            rt.online = false;
            rt.foreground = false;
            rt.hello_at = None;
        }
        self.sync_at = None;
        self.refresh_status();
    }

    pub(crate) fn publish_presence(&self, online: bool) {
        let presence = Presence {
            o: online as u8,
            t: self.now() / 1000,
            r: "mac".into(),
            fg: 0,
            e: online.then(|| self.epoch.clone()),
        };
        let payload = serde_json::to_vec(&presence).expect("presence serializes");
        self.outbox.send(
            wire::topic_presence(&self.user, &self.mac_id),
            payload,
            true,
        );
    }

    // ------------------------------------------------------------ inbound

    async fn on_message(&mut self, topic: &str, payload: &[u8]) {
        if let Some(device) = wire::presence_device(&self.user, topic) {
            self.on_presence(&device, payload);
            return;
        }
        if topic == wire::topic_up(&self.user, &self.mac_id) {
            self.on_up(topic, payload).await;
            return;
        }
        if let Some(ticket_id) = topic.strip_prefix(&format!("ah/{}/p/", self.user)) {
            self.on_pair_topic(topic, ticket_id, payload).await;
        }
    }

    fn on_presence(&mut self, device: &str, payload: &[u8]) {
        if device == self.mac_id {
            return;
        }
        let Ok(presence) = serde_json::from_slice::<Presence>(payload) else {
            return;
        };
        if presence.r != "phone" {
            return;
        }
        let Some(rt) = self.rt.get_mut(device) else {
            return;
        };
        rt.online = presence.o == 1;
        rt.foreground = presence.o == 1 && presence.fg == 1;
        rt.presence_ts = presence.t;
        if !rt.online {
            rt.hello_at = None;
        }
    }

    async fn on_up(&mut self, topic: &str, payload: &[u8]) {
        let Ok(env) = envelope::parse(payload) else {
            return;
        };
        if env.f == self.mac_id {
            return;
        }
        let Some(phone) = self.store.state.phone(&env.f) else {
            return;
        };
        let state = phone.state;
        let Some(keys) = self.store.keys(&env.f).cloned() else {
            return;
        };
        if env.k != keys.kid {
            return;
        }
        let Ok(plain) = envelope::open_plain(&keys.p2m, Scope::Topic(topic), &env) else {
            return;
        };
        let now = self.now();
        self.store
            .state
            .count_traffic(&month_of(now), true, payload.len());
        let phone_id = env.f.clone();
        if (plain.ts - now).abs() > CLOCK_SKEW_MS {
            self.send_error(
                &phone_id,
                &plain.i,
                "clock_skew",
                Some("check the clocks of the Mac and the phone"),
            );
            return;
        }
        let fresh = self
            .rt
            .entry(phone_id.clone())
            .or_default()
            .dedup
            .insert(&plain.i, now);
        if !fresh {
            return;
        }
        // A phone that is still completing its pairing may only say `pair.done`.
        if state == PhoneState::PendingConfirm && plain.t != t::PAIR_DONE {
            return;
        }
        self.touch_phone(&phone_id, now);
        self.dispatch(&phone_id, plain).await;
    }

    fn touch_phone(&mut self, phone_id: &str, now: i64) {
        if let Some(phone) = self.store.state.phone_mut(phone_id) {
            phone.last_seen_at = now;
            self.dirty = true;
        }
    }

    async fn dispatch(&mut self, phone_id: &str, plain: Plain) {
        macro_rules! body {
            ($ty:ty) => {
                match serde_json::from_value::<$ty>(plain.b.clone()) {
                    Ok(body) => body,
                    Err(_) => {
                        self.send_error(phone_id, &plain.i, "bad_request", None);
                        return;
                    }
                }
            };
        }
        match plain.t.as_str() {
            t::PAIR_DONE => {
                let done = body!(wire::PairDone);
                self.on_pair_done(phone_id, done).await;
            }
            t::HELLO => {
                let hello = body!(wire::Hello);
                self.on_hello(phone_id, hello).await;
            }
            t::RESYNC => {
                let resync = body!(wire::Resync);
                self.send_stream_snapshot(phone_id, &resync.stream);
            }
            t::PING => {
                let ping = body!(wire::Ping);
                self.send(phone_id, &wire::Pong { t: ping.t });
            }
            t::UNPAIR => {
                self.remove_phone(phone_id);
            }
            t::PUSH_REGISTER => {
                let reg = body!(wire::PushRegister);
                self.on_push_register(phone_id, reg);
            }
            t::REQUEST_GET => {
                let get = body!(wire::RequestGet);
                self.on_request_get(phone_id, &get.request_id);
            }
            t::REQUEST_ACK => {
                let ack = body!(wire::RequestAck);
                self.on_request_ack(phone_id, &ack.request_id);
            }
            t::REQUEST_SEEN => {
                let seen = body!(wire::RequestSeen);
                self.on_request_seen(phone_id, &seen.request_id);
            }
            t::ANSWER_SUBMIT => {
                let submit = body!(wire::AnswerSubmit);
                self.on_answer_submit(phone_id, submit).await;
            }
            t::CONFIRM_DECIDE => {
                let decide = body!(wire::ConfirmDecide);
                self.on_confirm_decide(phone_id, decide).await;
            }
            t::ASSET_GET => {
                let get = body!(wire::AssetGet);
                self.on_asset_get(phone_id, get);
            }
            t::UPLOAD_BEGIN => {
                let begin = body!(wire::UploadBegin);
                self.on_upload_begin(phone_id, begin);
            }
            t::UPLOAD_CHUNK => {
                let chunk = body!(wire::UploadChunk);
                self.on_upload_chunk(phone_id, chunk);
            }
            t::UPLOAD_CANCEL => {
                let cancel = body!(wire::UploadCancel);
                self.on_upload_cancel(phone_id, cancel);
            }
            _ => self.send_error(phone_id, &plain.i, "unknown_type", None),
        }
    }

    // ------------------------------------------------------------ outbound

    /// Seals `body` for one phone and queues it. Messages larger than the broker allows are dropped.
    pub(crate) fn send<M: Msg>(&mut self, phone_id: &str, body: &M) -> bool {
        self.send_value(phone_id, M::TYPE, body.to_value())
    }

    pub(crate) fn send_value(&mut self, phone_id: &str, msg_type: &str, body: Value) -> bool {
        let Some(keys) = self.store.keys(phone_id).cloned() else {
            return false;
        };
        let now = self.now();
        let plain = Plain::new(msg_type, body, now);
        let topic = wire::topic_down(&self.user, phone_id);
        let bytes = envelope::seal_plain(
            &keys.m2p,
            Scope::Topic(&topic),
            &self.mac_id,
            keys.kid,
            &plain,
        );
        if bytes.len() > wire::MAX_ENVELOPE_BYTES {
            return false;
        }
        self.store
            .state
            .count_traffic(&month_of(now), false, bytes.len());
        self.outbox.send(topic, bytes, false);
        true
    }

    pub(crate) fn send_error(
        &mut self,
        phone_id: &str,
        reference: &str,
        code: &str,
        message: Option<&str>,
    ) {
        self.send(
            phone_id,
            &wire::ErrorBody {
                reference: reference.to_string(),
                code: code.to_string(),
                message: message.map(str::to_string),
            },
        );
    }

    /// Phones that receive deltas: online, foreground and past their hello.
    pub(crate) fn live_phones(&self) -> Vec<String> {
        self.store
            .state
            .active_phones()
            .filter(|p| {
                self.rt
                    .get(&p.phone_id)
                    .is_some_and(|rt| rt.online && rt.foreground && rt.hello_at.is_some())
            })
            .map(|p| p.phone_id.clone())
            .collect()
    }

    // ------------------------------------------------------------ hello and snapshots

    async fn on_hello(&mut self, phone_id: &str, hello: wire::Hello) {
        if hello.proto != PROTO {
            self.send_error(phone_id, &hello.nonce, "unsupported_proto", None);
            return;
        }
        let now = self.now();
        if let Some(rt) = self.rt.get_mut(phone_id) {
            rt.hello_at = Some(now);
            rt.device_name = sanitize_name(&hello.device_name);
        }
        self.send_hello_sync(phone_id, &hello.nonce);
    }

    fn send_hello_sync(&mut self, phone_id: &str, nonce: &str) {
        let ack = wire::HelloAck {
            nonce: nonce.to_string(),
            epoch: self.epoch.clone(),
            mac_name: self.mac_name(),
            caps: self.host.caps(),
            slot: self.host.slot(),
        };
        self.send(phone_id, &ack);
        for stream in ["inbox", "agents", "follow", "slot"] {
            self.send_stream_snapshot(phone_id, stream);
        }
    }

    pub(crate) fn send_stream_snapshot(&mut self, phone_id: &str, stream: &str) {
        match stream {
            "inbox" => {
                let snapshot = self.inbox_snapshot();
                self.send(phone_id, &snapshot);
            }
            "agents" => {
                let snapshot = wire::AgentsSnapshot {
                    epoch: self.epoch.clone(),
                    rev: self.revs.agents,
                    agents: Vec::new(),
                };
                self.send(phone_id, &snapshot);
            }
            "follow" => {
                let keys = self
                    .store
                    .state
                    .phone(phone_id)
                    .map(|p| p.follow_set.clone())
                    .unwrap_or_default();
                self.send(
                    phone_id,
                    &wire::FollowState {
                        rev: self.revs.follow,
                        keys,
                        error: None,
                    },
                );
            }
            "slot" => {
                let slot = self.host.slot();
                self.send(
                    phone_id,
                    &wire::SlotState {
                        rev: self.revs.slot,
                        active: slot.active,
                        auto_activation: slot.auto_activation,
                    },
                );
            }
            _ => {}
        }
    }

    fn on_push_register(&mut self, phone_id: &str, reg: wire::PushRegister) {
        if reg.handle.is_empty() || reg.handle.len() > 2048 {
            return;
        }
        if let Some(phone) = self.store.state.phone_mut(phone_id) {
            phone.push = Some(super::store::PushReg {
                handle: reg.handle,
                env: reg.env,
                expires_at: reg.expires_at,
                status: "ok".into(),
            });
            self.store.save_state();
        }
        self.send(
            phone_id,
            &wire::PushState {
                status: "ok".into(),
            },
        );
    }

    // ------------------------------------------------------------ phones

    /// Drops a pairing for good (the phone asked for it, or the user revoked it).
    pub(crate) fn remove_phone(&mut self, phone_id: &str) -> bool {
        self.requests.forget_phone(phone_id);
        self.drop_phone_uploads(phone_id);
        self.rt.remove(phone_id);
        let removed = self.store.remove_phone(phone_id);
        self.refresh_status();
        removed
    }

    async fn revoke(&mut self, phone_id: &str) -> bool {
        if self.store.state.phone(phone_id).is_none() {
            return false;
        }
        // Tell the phone first (best effort), while the keys still exist.
        self.send(phone_id, &wire::Unpaired {});
        self.outbox.flush().await;
        self.remove_phone(phone_id)
    }

    // ------------------------------------------------------------ timers

    async fn tick(&mut self) {
        let now = self.now();
        self.tick_pairing(now).await;
        self.tick_requests(now).await;
        self.prune_uploads(now);
        self.tick_slot();
        if let Some(at) = self.sync_at {
            if now >= at {
                self.sync_at = None;
                let online: Vec<String> = self
                    .store
                    .state
                    .active_phones()
                    .filter(|p| {
                        self.rt
                            .get(&p.phone_id)
                            .is_some_and(|rt| rt.online && rt.foreground)
                    })
                    .map(|p| p.phone_id.clone())
                    .collect();
                for phone_id in online {
                    for stream in ["inbox", "agents", "follow", "slot"] {
                        self.send_stream_snapshot(&phone_id, stream);
                    }
                }
            }
        }
        if self.dirty && now - self.last_flush_ms >= 30_000 {
            self.dirty = false;
            self.last_flush_ms = now;
            self.store.save_state();
        }
        if now - self.last_prune_ms >= 3_600_000 {
            self.last_prune_ms = now;
            self.store.journal_prune(now);
            self.sweep_upload_files();
        }
    }

    /// Tells the live phones when the slot (is iOS the active channel?) changes.
    fn tick_slot(&mut self) {
        let slot = self.host.slot();
        let previous = self.last_slot.replace(slot.clone());
        if previous.is_none_or(|previous| previous == slot) {
            return;
        }
        self.revs.slot += 1;
        for phone_id in self.live_phones() {
            self.send(
                &phone_id,
                &wire::SlotState {
                    rev: self.revs.slot,
                    active: slot.active,
                    auto_activation: slot.auto_activation,
                },
            );
        }
    }

    async fn shutdown(&mut self) {
        self.publish_presence(false);
        if self.dirty {
            self.store.save_state();
        }
        self.outbox.flush().await;
        self.transport.disconnect().await;
        self.connected = false;
        self.refresh_status();
    }
}

fn hex16() -> String {
    crypto::random_bytes::<8>()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

#[allow(dead_code)]
const _: u32 = KID_PAIR;
