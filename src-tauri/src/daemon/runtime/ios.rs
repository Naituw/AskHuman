//! iOS channel glue: owns the gateway's lifecycle inside the daemon, adapts daemon state to the
//! gateway's `Host`, and serves the management operations of the settings window and the CLI
//! (docs/plans/ios-channel.md §4.3).

use std::sync::Weak;

use super::ios_agents::{self, ProjectCache};
use super::*;
use crate::config::IosChannelConfig;
use crate::ios::gateway::{
    FrameData, GatewayDeps, Host, InterjectResult, IosGateway, StagedFile, TaskChoices, TaskLaunch,
    TodoChoices,
};
use crate::ios::push::HttpPushClient;
use crate::ios::store::Store;
use crate::ios::transport::{ConnectParams, RumqttcTransport, Will};
use crate::ios::wire::{self, Caps, Presence, SlotInfo};

/// Settings that need a fresh broker connection when they change.
#[derive(Clone, PartialEq, Eq)]
struct ConnKey {
    host: String,
    port: u16,
    ws_port: u16,
    username: String,
    password: String,
    relay_url: String,
    mac_name: String,
    insecure_local: bool,
}

impl ConnKey {
    fn of(cfg: &IosChannelConfig) -> Self {
        Self {
            host: cfg.host.trim().to_string(),
            port: cfg.port,
            ws_port: cfg.ws_port,
            username: cfg.username.trim().to_string(),
            password: cfg.password.clone(),
            relay_url: cfg.relay_url.trim().to_string(),
            mac_name: cfg.mac_name.trim().to_string(),
            insecure_local: cfg.insecure_local,
        }
    }
}

/// A running gateway and the settings it was started with.
pub(super) struct IosSlot {
    gateway: Arc<IosGateway>,
    key: ConnKey,
}

/// What the gateway asks the daemon about.
struct DaemonHost {
    state: Weak<ServerState>,
    computer_name: String,
    projects: ProjectCache,
}

#[async_trait::async_trait]
impl Host for DaemonHost {
    fn mac_name(&self) -> String {
        self.computer_name.clone()
    }

    fn caps(&self) -> Caps {
        Caps {
            new_task: true,
            interject: true,
            watch: true,
        }
    }

    fn agents(&self) -> Vec<wire::AgentView> {
        match self.state.upgrade() {
            Some(state) => ios_agents::agent_views(&state, &self.projects),
            None => Vec::new(),
        }
    }

    fn watch_frame(&self, agent_key: &str) -> Option<FrameData> {
        ios_agents::watch_frame(&self.state.upgrade()?, agent_key)
    }

    async fn interject(
        &self,
        phone_id: &str,
        agent_key: &str,
        text: &str,
        files: Vec<StagedFile>,
    ) -> InterjectResult {
        match self.state.upgrade() {
            Some(state) => ios_agents::interject(&state, phone_id, agent_key, text, files),
            None => InterjectResult::rejected("unavailable"),
        }
    }

    fn interject_clear(&self, agent_key: &str) -> bool {
        self.state
            .upgrade()
            .is_some_and(|state| ios_agents::interject_clear(&state, agent_key))
    }

    fn interject_state(&self, agent_key: &str) -> (u64, String) {
        match self.state.upgrade() {
            Some(state) => ios_agents::interject_state(&state, agent_key),
            None => (0, String::new()),
        }
    }

    async fn task_choices(&self) -> TaskChoices {
        match self.state.upgrade() {
            Some(state) => ios_tasks::choices(&state).await,
            None => TaskChoices::blocked("agentDefault", "unavailable"),
        }
    }

    async fn task_todos(&self, workspace: &str) -> TodoChoices {
        let workspace = workspace.to_string();
        tokio::task::spawn_blocking(move || ios_tasks::todos(&workspace))
            .await
            .unwrap_or_default()
    }

    async fn task_create(&self, launch: TaskLaunch) -> Result<(), String> {
        match self.state.upgrade() {
            Some(state) => ios_tasks::create(&state, launch).await,
            None => Err("unavailable".to_string()),
        }
    }

    async fn take_slot(&self) {
        if let Some(state) = self.state.upgrade() {
            let _ = set_active_channel(&state, "ios").await;
        }
    }

    fn slot(&self) -> SlotInfo {
        let Some(state) = self.state.upgrade() else {
            return SlotInfo {
                active: false,
                auto_activation: false,
            };
        };
        let active = state.active_channel.lock().unwrap().as_deref() == Some("ios");
        let auto_activation = state.config.lock().unwrap().channels.auto_activation;
        SlotInfo {
            active,
            auto_activation,
        }
    }
}

/// Name this computer goes by in the Finder / Settings, for the phone's device list.
fn computer_name() -> String {
    #[cfg(target_os = "macos")]
    {
        for args in [
            &["--get", "ComputerName"][..],
            &["--get", "LocalHostName"][..],
        ] {
            if let Ok(out) = std::process::Command::new("scutil").args(args).output() {
                let name = String::from_utf8_lossy(&out.stdout).trim().to_string();
                if out.status.success() && !name.is_empty() {
                    return name;
                }
            }
        }
    }
    for var in ["COMPUTERNAME", "HOSTNAME", "HOST"] {
        if let Ok(name) = std::env::var(var) {
            if !name.trim().is_empty() {
                return name.trim().to_string();
            }
        }
    }
    if let Ok(out) = std::process::Command::new("hostname").output() {
        let name = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if !name.is_empty() {
            return name;
        }
    }
    "Computer".to_string()
}

fn build_gateway(state: &Arc<ServerState>, cfg: &IosChannelConfig) -> Arc<IosGateway> {
    let store = Store::open(crate::paths::ios_dir());
    let mac_id = store.state.mac_id.clone();
    let user = cfg.username.trim().to_string();
    let will = Will {
        topic: wire::topic_presence(&user, &mac_id),
        payload: serde_json::to_vec(&Presence {
            o: 0,
            t: 0,
            r: "mac".into(),
            fg: 0,
            e: None,
        })
        .expect("presence serializes"),
        retain: true,
    };
    let (transport, events) = RumqttcTransport::spawn(ConnectParams {
        host: cfg.host.trim().to_string(),
        tls_port: cfg.port,
        wss_port: cfg.ws_port,
        username: user,
        password: cfg.password.clone(),
        client_id: format!("mac-{mac_id}"),
        will,
        prefer: store.state.last_transport.clone(),
        insecure_local: cfg.insecure_local,
    });
    let host = Arc::new(DaemonHost {
        state: Arc::downgrade(state),
        computer_name: computer_name(),
        projects: ProjectCache::default(),
    });
    Arc::new(IosGateway::start(GatewayDeps {
        host,
        cfg: cfg.clone(),
        store,
        transport,
        events,
        pusher: Arc::new(HttpPushClient::new()),
    }))
}

/// The running gateway, if any (no side effects).
pub(super) fn ios_gateway(state: &ServerState) -> Option<Arc<IosGateway>> {
    state
        .ios
        .lock()
        .unwrap()
        .as_ref()
        .map(|slot| slot.gateway.clone())
}

/// Whether a phone is paired and the channel is configured, i.e. an iOS surface is worth
/// attaching to a request.
pub(super) fn ios_can_deliver(state: &ServerState, config: &AppConfig) -> bool {
    crate::app::is_ios_active(config)
        && ios_gateway(state).is_some_and(|gateway| gateway.has_active_phone())
}

/// Idempotently brings the gateway in line with the current configuration: starts it when the
/// channel is enabled and complete, replaces it when the connection settings changed, stops it
/// when the channel was disabled. Requests held by a replaced gateway go to its successor.
pub(super) async fn ensure_ios_gateway(state: &Arc<ServerState>) -> Option<Arc<IosGateway>> {
    let config = state.config_snapshot();
    let wanted = crate::app::is_ios_active(&config).then(|| ConnKey::of(&config.channels.ios));

    let (current, retired) = {
        let mut slot = state.ios.lock().unwrap();
        match (&wanted, slot.as_ref()) {
            (Some(key), Some(running)) if *key == running.key => {
                return Some(running.gateway.clone());
            }
            (Some(key), _) => {
                let retired = slot.take();
                let gateway = build_gateway(state, &config.channels.ios);
                *slot = Some(IosSlot {
                    gateway: gateway.clone(),
                    key: key.clone(),
                });
                (Some(gateway), retired)
            }
            (None, _) => (None, slot.take()),
        }
    };
    let replaced = retired.is_some();
    if let Some(old) = retired {
        old.gateway.retire().await;
    }
    match &current {
        Some(_) => log("iOS gateway started"),
        None if replaced => log("iOS gateway stopped"),
        None => {}
    }
    // Requests the retired gateway handed back are delivered again by the new one.
    if replaced && current.is_some() {
        let active = state.active_channel.lock().unwrap().clone();
        if active.as_deref() == Some("ios") {
            backfill_inflight(state, "ios", &config).await;
        }
    }
    current
}

/// Graceful end of the daemon: tell the phones this Mac is going offline.
pub(super) async fn stop_ios_gateway(state: &Arc<ServerState>) {
    let slot = state.ios.lock().unwrap().take();
    if let Some(slot) = slot {
        slot.gateway.shutdown().await;
    }
}

/// Management operations (settings window and CLI).
pub(super) async fn handle_ios_op(
    state: &Arc<ServerState>,
    op: ipc::IosOp,
) -> Result<ipc::IosResult, String> {
    let gateway = ensure_ios_gateway(state).await;
    if let ipc::IosOp::State = op {
        return Ok(ipc::IosResult::State {
            state: match gateway {
                Some(gateway) => gateway.state().await.map(Box::new),
                None => None,
            },
        });
    }
    let gateway = gateway.ok_or_else(|| {
        "the iOS channel is not enabled or its connection settings are incomplete".to_string()
    })?;
    match op {
        ipc::IosOp::State => unreachable!("handled above"),
        ipc::IosOp::PairStart => {
            let info = gateway.pair_start().await?;
            Ok(ipc::IosResult::Pairing {
                ticket_id: info.ticket_id,
                code: info.code,
                expires_at_ms: info.expires_at_ms,
            })
        }
        ipc::IosOp::PairCancel => {
            gateway.pair_cancel();
            Ok(ipc::IosResult::Done { found: true })
        }
        ipc::IosOp::PairDecision { ticket_id, allow } => {
            gateway.pair_decision(&ticket_id, allow);
            Ok(ipc::IosResult::Done { found: true })
        }
        ipc::IosOp::Revoke { phone_id } => Ok(ipc::IosResult::Done {
            found: gateway.revoke(&phone_id).await,
        }),
        ipc::IosOp::Rename { phone_id, name } => {
            gateway.rename(&phone_id, &name);
            Ok(ipc::IosResult::Done { found: true })
        }
    }
}

/// The surface to attach to a request. Only used for delivery candidates, which exist only while
/// the gateway runs, so this never has to (re)start it: that keeps the async call graph acyclic
/// (`ensure_ios_gateway` re-delivers in-flight requests through the attach functions).
pub(super) fn ios_channel_for(state: &ServerState, request_id: &str) -> Option<Arc<dyn Channel>> {
    let gateway = ios_gateway(state)?;
    Some(Arc::new(crate::channels::ios::IosChannel::new(
        gateway, request_id,
    )))
}
