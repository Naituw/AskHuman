//! Test helpers for the gateway: a virtual-clock host, a recording push client and `TestPhone`,
//! a phone-side implementation of the protocol built on the same crypto as the Mac.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use serde::de::DeserializeOwned;
use tokio::sync::mpsc::UnboundedReceiver;

use super::requests::{AskSink, ConfirmSink};
use super::{
    FrameData, GatewayDeps, Host, InterjectResult, IosGateway, StagedFile, TaskChoices, TaskLaunch,
    TodoChoices,
};
use crate::config::IosChannelConfig;
use crate::ios::crypto::{self, KeyPair, PairContext};
use crate::ios::envelope::{self, Plain, Scope};
use crate::ios::push::{PushClient, PushOutcome};
use crate::ios::store::{PhoneKeys, Store};
use crate::ios::transport::{
    BrokerTransport, LoopbackBroker, LoopbackTransport, TransportEvent, Will,
};
use crate::ios::wire::{
    self, t, AgentRef, AgentView, AskView, CanInterject, Caps, ConfirmChoiceView,
    ConfirmDetailView, ConfirmView, Msg, OptionView, PairCode, Presence, ProjectRef, QuestionView,
    RelayPush, RequestView, SlotInfo, KID_TICKET,
};
use crate::models::{AskRequest, ChannelResult, MessagePrompt, OptionItem, Question};

pub const BASE_MS: i64 = 1_800_000_000_000;
pub const USER: &str = "u1";

// ---------------------------------------------------------------- host and push

/// Clock follows tokio's (pausable) time, so tests run instantly and deterministically. The
/// scripted fields stand in for the daemon's Agent registry, terminal and interject queue.
pub struct TestHost {
    start: tokio::time::Instant,
    pub slot: Mutex<SlotInfo>,
    pub agents: Mutex<Vec<AgentView>>,
    pub frames: Mutex<HashMap<String, FrameData>>,
    /// What `interject` answers; `None` means "queued, one pending".
    pub interject_result: Mutex<Option<InterjectResult>>,
    /// `(phone, agent, text, file count)` of every interjection that reached the host.
    pub interjects: Mutex<Vec<(String, String, String, usize)>>,
    pub interject_pending: Mutex<(u64, String)>,
    /// Files of the latest interjection.
    pub staged: Mutex<Vec<StagedFile>>,
    pub cleared: Mutex<Vec<String>>,
    pub choices: Mutex<TaskChoices>,
    pub todos: Mutex<TodoChoices>,
    pub task_result: Mutex<Result<(), String>>,
    /// Virtual time `task_create` takes.
    pub task_delay_ms: AtomicU64,
    pub launches: Mutex<Vec<TaskLaunch>>,
    pub slot_taken: AtomicUsize,
}

impl TestHost {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            start: tokio::time::Instant::now(),
            slot: Mutex::new(SlotInfo {
                active: true,
                auto_activation: true,
            }),
            agents: Mutex::new(Vec::new()),
            frames: Mutex::new(HashMap::new()),
            interject_result: Mutex::new(None),
            interjects: Mutex::new(Vec::new()),
            interject_pending: Mutex::new((0, String::new())),
            staged: Mutex::new(Vec::new()),
            cleared: Mutex::new(Vec::new()),
            choices: Mutex::new(sample_choices()),
            todos: Mutex::new(TodoChoices::default()),
            task_result: Mutex::new(Ok(())),
            task_delay_ms: AtomicU64::new(0),
            launches: Mutex::new(Vec::new()),
            slot_taken: AtomicUsize::new(0),
        })
    }

    /// Replaces the Agent list.
    pub fn set_agents(&self, agents: Vec<AgentView>) {
        *self.agents.lock().unwrap() = agents;
    }

    /// Changes one Agent in place (state, tool, …).
    pub fn update_agent(&self, key: &str, change: impl FnOnce(&mut AgentView)) {
        let mut agents = self.agents.lock().unwrap();
        if let Some(agent) = agents.iter_mut().find(|a| a.key == key) {
            change(agent);
        }
    }

    pub fn set_frame(&self, key: &str, sig: &str, text: &str, ended: bool) {
        self.frames.lock().unwrap().insert(
            key.to_string(),
            FrameData {
                frame: serde_json::json!({
                    "phase": if ended { "ended" } else { "working" },
                    "title": "Refactor",
                    "text": text,
                    "steps": [],
                    "todos": [],
                }),
                sig: sig.to_string(),
                ended,
            },
        );
    }
}

#[async_trait]
impl Host for TestHost {
    fn now_ms(&self) -> i64 {
        BASE_MS + self.start.elapsed().as_millis() as i64
    }

    fn mac_name(&self) -> String {
        "Test Mac".into()
    }

    fn caps(&self) -> Caps {
        Caps {
            new_task: true,
            interject: true,
            watch: true,
        }
    }

    fn slot(&self) -> SlotInfo {
        self.slot.lock().unwrap().clone()
    }

    fn agents(&self) -> Vec<AgentView> {
        self.agents.lock().unwrap().clone()
    }

    fn watch_frame(&self, agent_key: &str) -> Option<FrameData> {
        self.frames.lock().unwrap().get(agent_key).cloned()
    }

    async fn interject(
        &self,
        phone_id: &str,
        agent_key: &str,
        text: &str,
        files: Vec<StagedFile>,
    ) -> InterjectResult {
        self.interjects.lock().unwrap().push((
            phone_id.to_string(),
            agent_key.to_string(),
            text.to_string(),
            files.len(),
        ));
        *self.staged.lock().unwrap() = files;
        self.interject_result
            .lock()
            .unwrap()
            .clone()
            .unwrap_or(InterjectResult {
                status: "queued",
                reason: None,
                pending: 1,
            })
    }

    fn interject_clear(&self, agent_key: &str) -> bool {
        self.cleared.lock().unwrap().push(agent_key.to_string());
        *self.interject_pending.lock().unwrap() = (0, String::new());
        true
    }

    fn interject_state(&self, _agent_key: &str) -> (u64, String) {
        self.interject_pending.lock().unwrap().clone()
    }

    async fn task_choices(&self) -> TaskChoices {
        self.choices.lock().unwrap().clone()
    }

    async fn task_todos(&self, _workspace: &str) -> TodoChoices {
        self.todos.lock().unwrap().clone()
    }

    async fn task_create(&self, launch: TaskLaunch) -> Result<(), String> {
        let delay = self.task_delay_ms.load(Ordering::SeqCst);
        if delay > 0 {
            tokio::time::sleep(Duration::from_millis(delay)).await;
        }
        self.launches.lock().unwrap().push(launch);
        self.task_result.lock().unwrap().clone()
    }

    async fn take_slot(&self) {
        self.slot_taken.fetch_add(1, Ordering::SeqCst);
        self.slot.lock().unwrap().active = true;
    }
}

/// An Agent session as the daemon would describe it.
pub fn sample_agent(key: &str, state: &str) -> AgentView {
    let (kind, session) = key.split_once(':').unwrap_or(("claude", key));
    AgentView {
        key: key.to_string(),
        kind: kind.to_string(),
        kind_label: "Claude Code".into(),
        session_id: session.to_string(),
        title: format!("Task {session}"),
        project: ProjectRef {
            key: "/tmp/proj".into(),
            name: "proj".into(),
        },
        state: state.to_string(),
        started_at: BASE_MS - 60_000,
        last_activity_at: BASE_MS,
        active_elapsed_secs: 30,
        turn_steps: 0,
        current_tool: None,
        waiting_request_id: None,
        waiting_preview: None,
        pending_interject: 0,
        last_turn_failed: false,
        can_interject: CanInterject {
            ok: state == "working",
            reason: (state != "working").then(|| "notWorking".to_string()),
        },
        followed: false,
    }
}

/// Workspaces and Agents a phone can pick from.
pub fn sample_choices() -> TaskChoices {
    TaskChoices {
        options: wire::TaskOptions {
            enabled: true,
            blocked: None,
            permission_mode: "ask".into(),
            workspaces: vec![wire::WorkspaceOption {
                id: "ws1".into(),
                label: "proj".into(),
                subtitle: "~/dev".into(),
                pinned: true,
                last_used_at: BASE_MS,
            }],
            agents: vec![wire::AgentOption {
                kind: "claude".into(),
                label: "Claude Code".into(),
                ready: true,
                subtitle: "hooks".into(),
                permission_selectable: true,
                reason: None,
            }],
        },
        workspaces: HashMap::from([("ws1".to_string(), "/tmp/proj".to_string())]),
    }
}

#[derive(Default)]
pub struct FakePusher {
    pub sent: Mutex<Vec<RelayPush>>,
    pub outcome: Mutex<Option<PushOutcome>>,
}

impl FakePusher {
    pub fn count(&self) -> usize {
        self.sent.lock().unwrap().len()
    }

    pub fn last(&self) -> Option<RelayPush> {
        self.sent.lock().unwrap().last().cloned()
    }
}

#[async_trait]
impl PushClient for FakePusher {
    async fn send(&self, _relay_url: &str, push: &RelayPush) -> PushOutcome {
        self.sent.lock().unwrap().push(push.clone());
        self.outcome
            .lock()
            .unwrap()
            .clone()
            .unwrap_or(PushOutcome::Accepted)
    }
}

// ---------------------------------------------------------------- sinks

pub struct FakeAsk {
    pub results: Mutex<Vec<ChannelResult>>,
    pub win: AtomicBool,
    pub lost: Mutex<Vec<String>>,
    /// Channel that decided the request; like the real coordinator, unset until somebody did.
    pub decided: Mutex<Option<String>>,
}

impl FakeAsk {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            results: Mutex::new(Vec::new()),
            win: AtomicBool::new(true),
            lost: Mutex::new(Vec::new()),
            decided: Mutex::new(None),
        })
    }
}

impl AskSink for FakeAsk {
    fn try_submit(&self, result: ChannelResult) -> bool {
        self.results.lock().unwrap().push(result);
        let won = self.win.load(Ordering::SeqCst);
        *self.decided.lock().unwrap() = Some(if won { "ios" } else { "popup" }.into());
        won
    }

    fn winner(&self) -> Option<String> {
        self.decided.lock().unwrap().clone()
    }

    fn surface_lost(&self, reason: &str) {
        self.lost.lock().unwrap().push(reason.to_string());
    }
}

pub struct FakeConfirm {
    pub decisions: Mutex<Vec<(usize, Option<String>)>>,
    pub result: Mutex<Result<bool, String>>,
    pub lost: Mutex<Vec<String>>,
    pub decided: Mutex<Option<String>>,
}

impl FakeConfirm {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            decisions: Mutex::new(Vec::new()),
            result: Mutex::new(Ok(true)),
            lost: Mutex::new(Vec::new()),
            decided: Mutex::new(None),
        })
    }
}

impl ConfirmSink for FakeConfirm {
    fn decide(&self, wire_index: usize, comment: Option<String>) -> Result<bool, String> {
        self.decisions.lock().unwrap().push((wire_index, comment));
        let result = self.result.lock().unwrap().clone();
        match &result {
            Ok(true) => *self.decided.lock().unwrap() = Some("ios".into()),
            Ok(false) => *self.decided.lock().unwrap() = Some("feishu".into()),
            Err(_) => {}
        }
        result
    }

    fn winner(&self) -> Option<String> {
        self.decided.lock().unwrap().clone()
    }

    fn surface_lost(&self, reason: &str) {
        self.lost.lock().unwrap().push(reason.to_string());
    }
}

// ---------------------------------------------------------------- sample requests

pub fn sample_ask(id: &str) -> (Arc<AskRequest>, RequestView) {
    let ask = AskRequest {
        id: id.to_string(),
        is_markdown: true,
        message: MessagePrompt {
            text: "Pick a deployment strategy".into(),
            files: Vec::new(),
        },
        questions: vec![
            Question {
                message: "Which environment?".into(),
                predefined_options: vec![
                    OptionItem::new("staging", false),
                    OptionItem::new("production", true),
                ],
            },
            Question {
                message: "Anything else?".into(),
                predefined_options: vec![
                    OptionItem::new("no", false),
                    OptionItem::new("yes", false),
                ],
            },
        ],
        select_only: false,
        single: false,
        output_format: Default::default(),
        whats_next: false,
    };
    let view = RequestView {
        id: id.to_string(),
        kind: "ask".into(),
        created_at: BASE_MS,
        agent: AgentRef {
            kind: "codex".into(),
            label: "Codex".into(),
        },
        project: ProjectRef {
            key: "/tmp/proj".into(),
            name: "proj".into(),
        },
        source: "Codex".into(),
        content_hash: String::new(),
        truncated: false,
        ask: Some(AskView {
            is_markdown: true,
            message: wire::MessageView {
                text: ask.message.text.clone(),
                files: Vec::new(),
            },
            questions: ask
                .questions
                .iter()
                .map(|q| QuestionView {
                    text: q.message.clone(),
                    options: q
                        .predefined_options
                        .iter()
                        .map(|o| OptionView {
                            text: o.text.clone(),
                            recommended: o.recommended,
                            todo: false,
                        })
                        .collect(),
                })
                .collect(),
            select_only: false,
            single: false,
            whats_next: false,
        }),
        confirm: None,
    };
    (Arc::new(ask), view)
}

pub fn sample_confirm(id: &str, expires_at: i64) -> RequestView {
    RequestView {
        id: id.to_string(),
        kind: "confirm".into(),
        created_at: BASE_MS,
        agent: AgentRef {
            kind: "claude".into(),
            label: "Claude Code".into(),
        },
        project: ProjectRef {
            key: "/tmp/proj".into(),
            name: "proj".into(),
        },
        source: "Claude Code".into(),
        content_hash: String::new(),
        truncated: false,
        ask: None,
        confirm: Some(ConfirmView {
            title: "Permission request".into(),
            context: Vec::new(),
            detail: ConfirmDetailView {
                summary: "Run git status".into(),
                body_md: "```sh\ngit status\n```".into(),
            },
            choices: vec![
                ConfirmChoiceView {
                    wire_index: 0,
                    label: "Allow".into(),
                    description: String::new(),
                    role: "primary".into(),
                    recommended: false,
                },
                ConfirmChoiceView {
                    wire_index: 2,
                    label: "Deny".into(),
                    description: String::new(),
                    role: "destructive".into(),
                    recommended: false,
                },
            ],
            default_index: Some(0),
            input: None,
            dismiss_index: Some(2),
            submit_label: "Submit".into(),
            expires_at,
        }),
    }
}

// ---------------------------------------------------------------- fixture

pub fn test_config() -> IosChannelConfig {
    IosChannelConfig {
        enabled: true,
        host: "broker.test".into(),
        port: 8883,
        ws_port: 8084,
        username: USER.into(),
        password: "pw".into(),
        relay_url: "https://relay.test".into(),
        mac_name: String::new(),
        insecure_local: false,
    }
}

pub struct Fixture {
    pub broker: LoopbackBroker,
    /// The Mac's own broker connection (for simulating network loss and recovery).
    pub transport: Arc<LoopbackTransport>,
    pub gateway: IosGateway,
    pub host: Arc<TestHost>,
    pub pusher: Arc<FakePusher>,
    pub mac_id: String,
    pub dir: tempfile::TempDir,
}

impl Fixture {
    pub async fn new() -> Self {
        Self::build(
            tempfile::tempdir().unwrap(),
            LoopbackBroker::new(),
            TestHost::new(),
        )
        .await
    }

    pub async fn build(
        dir: tempfile::TempDir,
        broker: LoopbackBroker,
        host: Arc<TestHost>,
    ) -> Self {
        let store = Store::open_with(dir.path().to_path_buf(), false);
        let mac_id = store.state.mac_id.clone();
        let will = Will {
            topic: wire::topic_presence(USER, &mac_id),
            payload: serde_json::to_vec(&Presence {
                o: 0,
                t: 0,
                r: "mac".into(),
                fg: 0,
                e: None,
            })
            .unwrap(),
            retain: true,
        };
        let (transport, events) = broker.connect(
            &format!("mac-{mac_id}"),
            Some(will),
            Some(&format!("ah/{USER}/")),
        );
        let pusher = Arc::new(FakePusher::default());
        let gateway = IosGateway::start(GatewayDeps {
            host: host.clone(),
            cfg: test_config(),
            store,
            transport: transport.clone(),
            events,
            pusher: pusher.clone(),
        });
        let fixture = Self {
            broker,
            transport,
            gateway,
            host,
            pusher,
            mac_id,
            dir,
        };
        fixture.settle().await;
        fixture
    }

    /// Lets the actor process what is queued (virtual time keeps ticking while idle).
    pub async fn settle(&self) {
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    /// Stops the gateway and starts a new one on the same storage and broker.
    pub async fn restart(self) -> Self {
        self.gateway.shutdown().await;
        let Self {
            broker, host, dir, ..
        } = self;
        Self::build(dir, broker, host).await
    }

    pub async fn pair_phone(&self) -> TestPhone {
        let mut phone = TestPhone::connect(self).await;
        phone.pair(self).await;
        phone
    }

    /// Like [`Self::pair_phone`], but the phone never registered for push.
    pub async fn pair_phone_without_push(&self) -> TestPhone {
        let mut phone = TestPhone::connect(self).await;
        phone.with_push = false;
        phone.pair(self).await;
        phone
    }

    /// A paired phone that is online, in the foreground and has said hello.
    pub async fn live_phone(&self) -> TestPhone {
        let mut phone = self.pair_phone().await;
        phone.set_presence(true, true).await;
        phone.hello().await;
        phone
    }
}

// ---------------------------------------------------------------- phone

pub struct TestPhone {
    pub phone_id: String,
    transport: Arc<LoopbackTransport>,
    events: UnboundedReceiver<TransportEvent>,
    mac_id: String,
    pub keys: Option<PhoneKeys>,
    pub sas: String,
    /// Whether `pair.done` carries a push handle.
    pub with_push: bool,
    ticket_key: Option<[u8; 32]>,
    pair_topic: Option<String>,
    queue: VecDeque<Plain>,
    pair_queue: VecDeque<Plain>,
}

impl TestPhone {
    pub async fn connect(fx: &Fixture) -> Self {
        let phone_id = crypto::new_id();
        let will = Will {
            topic: wire::topic_presence(USER, &phone_id),
            payload: serde_json::to_vec(&Presence {
                o: 0,
                t: 0,
                r: "phone".into(),
                fg: 0,
                e: None,
            })
            .unwrap(),
            retain: true,
        };
        let (transport, events) = fx.broker.connect(
            &format!("phone-{phone_id}"),
            Some(will),
            Some(&format!("ah/{USER}/")),
        );
        transport
            .subscribe(&wire::topic_down(USER, &phone_id))
            .await
            .unwrap();
        Self {
            phone_id,
            transport,
            events,
            mac_id: fx.mac_id.clone(),
            keys: None,
            sas: String::new(),
            with_push: true,
            ticket_key: None,
            pair_topic: None,
            queue: VecDeque::new(),
            pair_queue: VecDeque::new(),
        }
    }

    pub async fn set_presence(&self, online: bool, foreground: bool) {
        let presence = Presence {
            o: online as u8,
            t: BASE_MS / 1000,
            r: "phone".into(),
            fg: foreground as u8,
            e: None,
        };
        self.transport
            .publish(
                &wire::topic_presence(USER, &self.phone_id),
                serde_json::to_vec(&presence).unwrap(),
                true,
            )
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // ------------------------------------------------------------ pairing

    /// Sends `pair.request` from a pairing code and returns once the Mac showed the check code.
    pub async fn knock(&mut self, code: &str) {
        let code = PairCode::decode(code).expect("valid pair code");
        let ticket_secret: [u8; 32] = crypto::b64u_decode(&code.ts).unwrap().try_into().unwrap();
        let mac_public: [u8; 32] = crypto::b64u_decode(&code.ek).unwrap().try_into().unwrap();
        let key_pair = KeyPair::generate();
        let shared = key_pair.diffie_hellman(&mac_public).expect("contributory");
        let keys = crypto::derive_pair_keys(
            &shared,
            &PairContext {
                mac_id: &code.m,
                phone_id: &self.phone_id,
                ticket_id: &code.t,
                ticket_secret: &ticket_secret,
                mac_public: &mac_public,
                phone_public: &key_pair.public,
            },
        );
        self.sas = crypto::sas_code(&keys.sas);
        self.keys = Some(PhoneKeys::from_pair_keys(&keys));
        let k_t = crypto::ticket_key(&ticket_secret, &code.t);
        let topic = wire::topic_pair(USER, &code.t);
        self.ticket_key = Some(k_t);
        self.pair_topic = Some(topic.clone());
        self.transport.subscribe(&topic).await.unwrap();
        self.send_on_pair_topic(
            &k_t,
            &topic,
            &wire::PairRequest {
                phone_id: self.phone_id.clone(),
                name: "Test iPhone".into(),
                ek: crypto::b64u(&key_pair.public),
                app_ver: "1.0".into(),
                proto: wire::PROTO,
            },
        )
        .await;
    }

    async fn send_on_pair_topic<M: Msg>(&self, k_t: &[u8; 32], topic: &str, body: &M) {
        let plain = Plain::new(M::TYPE, body.to_value(), BASE_MS);
        let bytes =
            envelope::seal_plain(k_t, Scope::Topic(topic), &self.phone_id, KID_TICKET, &plain);
        self.transport.publish(topic, bytes, false).await.unwrap();
    }

    /// Full pairing in which the user compares the check codes and allows.
    pub async fn pair(&mut self, fx: &Fixture) {
        let info = fx.gateway.pair_start().await.expect("pair start");
        self.knock(&info.code).await;
        let _ = self.recv_pair(t::PAIR_PENDING).await;
        let state = fx.gateway.state().await.unwrap();
        let view = state.pairing.expect("pairing in progress");
        assert_eq!(view.stage, "pending");
        assert_eq!(
            view.sas.as_deref(),
            Some(self.sas.as_str()),
            "check codes match"
        );
        fx.gateway.pair_decision(&info.ticket_id, true);
        self.finish_pairing().await;
        fx.settle().await;
    }

    /// After the Mac allowed: wait for `pair.accept` and confirm with `pair.done`.
    pub async fn finish_pairing(&mut self) {
        let _ = self.recv(t::PAIR_ACCEPT).await;
        let with_push = self.with_push;
        self.send(&wire::PairDone {
            name: "Test iPhone".into(),
            handle: with_push.then(|| "handle-1".to_string()),
            env: with_push.then(|| "dev".to_string()),
            expires_at: with_push.then_some(BASE_MS / 1000 + 86_400 * 100),
        })
        .await;
    }

    // ------------------------------------------------------------ messaging

    pub async fn send<M: Msg>(&mut self, body: &M) -> String {
        let plain = Plain::new(M::TYPE, body.to_value(), BASE_MS);
        let id = plain.i.clone();
        self.send_plain(plain).await;
        id
    }

    pub async fn send_plain(&mut self, plain: Plain) {
        let keys = self.keys.clone().expect("paired");
        let topic = wire::topic_up(USER, &self.mac_id);
        let bytes = envelope::seal_plain(
            &keys.p2m,
            Scope::Topic(&topic),
            &self.phone_id,
            keys.kid,
            &plain,
        );
        self.transport.publish(&topic, bytes, false).await.unwrap();
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    /// Publishes raw bytes on the Mac's up topic (garbage, replays, foreign senders).
    pub async fn send_raw_up(&self, bytes: Vec<u8>) {
        let topic = wire::topic_up(USER, &self.mac_id);
        self.transport.publish(&topic, bytes, false).await.unwrap();
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    fn absorb(&mut self, event: TransportEvent) {
        let TransportEvent::Message { topic, payload, .. } = event else {
            return;
        };
        let Ok(env) = envelope::parse(&payload) else {
            return;
        };
        if env.f != self.mac_id {
            return;
        }
        if topic == wire::topic_down(USER, &self.phone_id) {
            if let Some(keys) = &self.keys {
                if let Ok(plain) = envelope::open_plain(&keys.m2p, Scope::Topic(&topic), &env) {
                    self.queue.push_back(plain);
                }
            }
        } else if Some(&topic) == self.pair_topic.as_ref() {
            if let Some(k_t) = &self.ticket_key {
                if let Ok(plain) = envelope::open_plain(k_t, Scope::Topic(&topic), &env) {
                    self.pair_queue.push_back(plain);
                }
            }
        }
    }

    fn pump_ready(&mut self) {
        while let Ok(event) = self.events.try_recv() {
            self.absorb(event);
        }
    }

    pub async fn recv(&mut self, ty: &str) -> Plain {
        self.recv_within(ty, Duration::from_secs(5))
            .await
            .unwrap_or_else(|| panic!("no `{ty}` message; queued: {:?}", self.queued_types()))
    }

    pub async fn recv_within(&mut self, ty: &str, within: Duration) -> Option<Plain> {
        let deadline = tokio::time::Instant::now() + within;
        loop {
            self.pump_ready();
            if let Some(position) = self.queue.iter().position(|p| p.t == ty) {
                return self.queue.remove(position);
            }
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                return None;
            }
            match tokio::time::timeout(remaining, self.events.recv()).await {
                Ok(Some(event)) => self.absorb(event),
                _ => return None,
            }
        }
    }

    pub async fn recv_body<T: DeserializeOwned>(&mut self, ty: &str) -> T {
        let plain = self.recv(ty).await;
        serde_json::from_value(plain.b).expect("body decodes")
    }

    pub async fn recv_pair(&mut self, ty: &str) -> Plain {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        loop {
            self.pump_ready();
            if let Some(position) = self.pair_queue.iter().position(|p| p.t == ty) {
                return self.pair_queue.remove(position).unwrap();
            }
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            assert!(!remaining.is_zero(), "no `{ty}` on the pair topic");
            if let Ok(Some(event)) = tokio::time::timeout(remaining, self.events.recv()).await {
                self.absorb(event);
            }
        }
    }

    /// Whether a `ty` message shows up within `within`.
    pub async fn gets(&mut self, ty: &str, within: Duration) -> bool {
        self.recv_within(ty, within).await.is_some()
    }

    pub fn queued_types(&self) -> Vec<String> {
        self.queue.iter().map(|p| p.t.clone()).collect()
    }

    pub fn clear_queue(&mut self) {
        self.pump_ready();
        self.queue.clear();
    }

    /// Hello handshake; the four snapshots stay in the queue for the test to inspect.
    pub async fn hello(&mut self) -> wire::HelloAck {
        let nonce = crypto::new_msg_id();
        self.send(&wire::Hello {
            nonce: nonce.clone(),
            proto: wire::PROTO,
            app_ver: "1.0".into(),
            device_name: "Test iPhone".into(),
        })
        .await;
        let ack: wire::HelloAck = self.recv_body(t::HELLO_ACK).await;
        assert_eq!(ack.nonce, nonce);
        ack
    }
}
