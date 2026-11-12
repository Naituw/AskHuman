//! iOS channel glue: owns the gateway's lifecycle inside the daemon, adapts daemon state to the
//! gateway's `Host`, and serves the management operations of the settings window and the CLI
//! (docs/plans/ios-channel.md §4.3).

use serde_json::Value;
use std::sync::Weak;

use super::ios_agents::{self, ProjectCache};
use super::*;
use crate::config::IosChannelConfig;
use crate::ios::gateway::{
    AwayView, FrameData, GatewayDeps, Host, InterjectResult, IosGateway, StagedFile, TaskChoices,
    TaskLaunch, TodoApplied, TodoChoices, TodoPick,
};
use crate::ios::push::HttpPushClient;
use crate::ios::store::Store;
use crate::ios::transport::{ConnectParams, RumqttcTransport, Will};
use crate::ios::views::AssetSource;
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
    fn update_status(&self) -> Option<String> {
        let state = self.state.upgrade()?;
        if *state.exit_intent.lock().unwrap() == super::super::handoff::Intent::Stop {
            return None;
        }
        if let Some(runtime) = &state.recovery {
            if runtime.is_restoring() {
                return Some("recovering".into());
            }
            if runtime.suspending.load(Ordering::SeqCst) {
                return Some("updating".into());
            }
        }
        let pending = state.update.lock().unwrap().pending;
        pending.then(|| "pending".into())
    }
    fn pending_ios_requests(&self) -> Result<Vec<String>, String> {
        let Some(state) = self.state.upgrade() else {
            return Err("daemon unavailable".into());
        };
        state
            .recovery
            .as_ref()
            .map(|runtime| {
                runtime
                    .store
                    .awaiting_channel("ios")
                    .map_err(|error| error.to_string())
            })
            .unwrap_or(Ok(Vec::new()))
    }

    fn load_recovery(&self) -> Result<Option<Value>, String> {
        let Some(state) = self.state.upgrade() else {
            return Err("daemon unavailable".into());
        };
        state
            .recovery
            .as_ref()
            .map(|runtime| {
                runtime
                    .store
                    .metadata("ios/gateway")
                    .map_err(|error| error.to_string())
            })
            .unwrap_or(Ok(None))
    }

    fn save_recovery(&self, value: &Value) -> Result<(), String> {
        let Some(state) = self.state.upgrade() else {
            return Err("daemon unavailable".into());
        };
        state
            .recovery
            .as_ref()
            .map(|runtime| {
                runtime
                    .store
                    .set_metadata("ios/gateway", value)
                    .map_err(|error| error.to_string())
            })
            .unwrap_or(Ok(()))
    }
    fn updating(&self) -> bool {
        self.state.upgrade().is_none_or(|state| {
            state.recovery.as_ref().is_some_and(|runtime| {
                runtime.is_restoring() || runtime.suspending.load(Ordering::SeqCst)
            })
        })
    }

    fn submission_receipt(
        &self,
        id: &str,
        phone: &str,
        hash: &str,
    ) -> Result<crate::ios::store::JournalLookup, String> {
        let Some(state) = self.state.upgrade() else {
            return Err("daemon unavailable".into());
        };
        state
            .recovery
            .as_ref()
            .map(|runtime| {
                runtime
                    .store
                    .receipt(id, phone, hash)
                    .map_err(|error| error.to_string())
            })
            .unwrap_or(Ok(crate::ios::store::JournalLookup::New))
    }
    fn mac_name(&self) -> String {
        self.computer_name.clone()
    }

    fn caps(&self) -> Caps {
        Caps {
            traffic_usage: true,
            traffic_sync: false,
            live_update: true,
            new_task: true,
            interject: true,
            watch: true,
            todos: true,
            answer_todos: true,
            away: true,
            annotations: true,
            session_history: true,
            history_pictures: true,
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
        client_msg_id: &str,
        agent_key: &str,
        text: &str,
        files: Vec<StagedFile>,
    ) -> InterjectResult {
        match self.state.upgrade() {
            Some(state) => {
                ios_agents::interject(&state, phone_id, client_msg_id, agent_key, text, files).await
            }
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

    async fn todo_projects(&self) -> wire::TodoProjects {
        match self.state.upgrade() {
            Some(state) => ios_todos::projects(&state).await,
            None => wire::TodoProjects::empty(),
        }
    }

    async fn todo_list(&self, project_id: &str) -> wire::TodoList {
        match self.state.upgrade() {
            Some(state) => ios_todos::list(&state, project_id).await,
            None => wire::TodoList::missing(project_id, 0),
        }
    }

    async fn todo_apply(&self, op: &wire::TodoOp) -> TodoApplied {
        match self.state.upgrade() {
            Some(state) => ios_todos::apply(&state, op).await,
            None => TodoApplied::refused("failed", "unavailable", None),
        }
    }

    fn todo_rev(&self) -> u64 {
        ios_todos::rev()
    }

    fn todo_refresh(&self) {
        ios_todos::refresh();
    }

    fn todo_picks(&self, project_key: &str, ids: &[String]) -> Vec<TodoPick> {
        // The local phase of a sync tick, like every other read of the project's todos.
        crate::todo_sync::service::before_read(project_key);
        let entries = crate::todos::list(project_key);
        ids.iter()
            .filter_map(|id| entries.iter().find(|entry| &entry.id == id))
            .map(|entry| TodoPick {
                id: entry.id.clone(),
                text: entry.text.clone(),
                attachments: entry
                    .attachments
                    .iter()
                    .map(|attachment| attachment.snapshot())
                    .collect(),
            })
            .collect()
    }

    fn todo_merge_line(&self, text: &str) -> String {
        crate::i18n::tr(crate::i18n::Lang::current(), "todo.mergeLine").replace("{text}", text)
    }

    async fn session_history(&self, get: &wire::SessionHistoryGet) -> wire::SessionHistory {
        ios_history::session_history(get).await
    }

    async fn history_exchange(&self, get: &wire::HistoryExchangeGet) -> wire::HistoryExchange {
        ios_history::history_exchange(get).await
    }

    async fn history_picture(&self, asset_id: &str) -> Option<AssetSource> {
        ios_history::history_picture(asset_id).await
    }

    fn slot(&self) -> SlotInfo {
        let Some(state) = self.state.upgrade() else {
            return slot_info(false);
        };
        let active = state.active_channel.lock().unwrap().as_deref() == Some("ios");
        slot_info(active)
    }

    fn mac_idle_secs(&self) -> Option<u64> {
        crate::idle::secs_since_last_input()
    }

    fn alert_after_secs(&self) -> u64 {
        let Some(state) = self.state.upgrade() else {
            return 0;
        };
        let (ios_secs, away) = {
            let config = state.config.lock().unwrap();
            (
                config.channels.ios.effective_alert_after_secs(),
                config.general.away.clone(),
            )
        };
        // While away, an escalated question reaches the phone sooner (special-replies D14).
        away.alert_after_secs(ios_secs, state.away.any_away())
    }

    fn away(&self) -> Option<AwayView> {
        let state = self.state.upgrade()?;
        Some(AwayView::from_snapshot(&super::away_ops::away_snapshot(
            &state,
        )))
    }

    fn away_apply(&self, op: &crate::daemon::away::AwayOp) -> bool {
        let Some(state) = self.state.upgrade() else {
            return false;
        };
        super::away_ops::apply_away_op(&state, op);
        true
    }
}

/// What the phone is told about the slot. Questions reach iOS whichever channel is active
/// (`docs/specs/ios-alert-timing.md` D1), so the phone is always told that nothing needs taking
/// over (`auto_activation: false`: it shows "every question is sent to this phone" and no
/// take-over button); `active` only says which channel last answered.
fn slot_info(active: bool) -> SlotInfo {
    SlotInfo {
        active,
        auto_activation: false,
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
            u: None,
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
    if matches!(
        &op,
        ipc::IosOp::PairStart
            | ipc::IosOp::PairDecision { .. }
            | ipc::IosOp::Revoke { .. }
            | ipc::IosOp::Rename { .. }
    ) {
        crate::development::routing::require_config_writes()?;
    }
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
///
/// `session` is the Agent session that asks, for the title the phone shows.
pub(super) fn ios_channel_for(
    state: &ServerState,
    request_id: &str,
    session: Option<crate::channels::ios::SessionRef>,
) -> Option<Arc<dyn Channel>> {
    let gateway = ios_gateway(state)?;
    Some(Arc::new(
        crate::channels::ios::IosChannel::new(gateway, request_id)
            .with_session(session)
            .with_quiet_feedback(state.registry.quiet_feedback(request_id)),
    ))
}

/// Where to find the title of the Agent session a request comes from; `None` when the caller is
/// no known Agent or did not say which session it is.
pub(super) fn ios_session_ref(
    state: &ServerState,
    kind: Option<AgentKind>,
    session_id: Option<&str>,
) -> Option<crate::channels::ios::SessionRef> {
    let kind = kind?;
    let session_id = session_id.map(str::trim).filter(|id| !id.is_empty())?;
    Some(crate::channels::ios::SessionRef {
        kind,
        session_id: session_id.to_string(),
        cached_title: state.agents.cached_session_title(kind, session_id),
        codex: state.config.lock().unwrap().codex_desktop.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_phone_is_never_asked_to_take_over() {
        // Whatever channel answered last, iOS gets every question: no take-over to offer.
        assert_eq!(
            slot_info(true),
            SlotInfo {
                active: true,
                auto_activation: false
            }
        );
        assert_eq!(
            slot_info(false),
            SlotInfo {
                active: false,
                auto_activation: false
            }
        );
    }
}
