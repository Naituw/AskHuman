//! Recoverable callers wait on durable requests instead of owning their lifetime through a socket.

use super::*;
use crate::daemon::recovery::{
    Call, ClientOperation, Event, Outcome, Request, RequestContext, Store, Task, VERSION,
};
use anyhow::{Context, Result};
use sha2::Digest;
use tokio::sync::watch;

pub(super) struct Runtime {
    pub store: Arc<Store>,
    workers: Mutex<HashMap<String, Arc<Worker>>>,
    submissions: tokio::sync::Mutex<()>,
    admission: Arc<tokio::sync::RwLock<()>>,
    restoring: AtomicBool,
    initialized: AtomicBool,
    update_running: AtomicBool,
    pub committed: AtomicBool,
    transition: tokio::sync::Mutex<()>,
    pub suspending: AtomicBool,
}

struct Worker {
    tasks: Mutex<Vec<tokio::task::AbortHandle>>,
    context: Arc<RequestContext>,
    events: watch::Sender<Option<Event>>,
    caller: Mutex<Option<Writer>>,
}

pub(super) fn saved_winner(state: &ServerState, id: &str) -> Option<String> {
    let saved = state.recovery.as_ref()?.store.request(id).ok()??;
    let decision = saved.decision?;
    if let Some(answer) = decision.get("answer") {
        return serde_json::from_value::<ChannelResult>(answer.clone())
            .ok()
            .map(|answer| answer.source_channel_id);
    }
    serde_json::from_value::<crate::models::ConfirmResult>(decision.get("confirm")?.clone())
        .ok()
        .map(|answer| answer.source_channel_id)
}

pub(super) struct LegacyWaiter {
    store: Arc<Store>,
    call: Call,
    pub delivered: bool,
}

impl Drop for LegacyWaiter {
    fn drop(&mut self) {
        let saved = if self.delivered {
            self.store.delivered(&self.call)
        } else {
            self.store.cancel_call(&self.call).map(|_| ())
        };
        if let Err(error) = saved {
            log(&format!("cannot finish legacy recovery waiter: {error}"));
            let _ = self.call.persist_cancellation();
        }
    }
}

pub(super) fn legacy_waiter(
    task: &TaskRequest,
    entry: &request::RequestEntry,
) -> Result<Option<LegacyWaiter>> {
    let Some(context) = &entry.request().recovery else {
        return Ok(None);
    };
    let mut call = Call::new();
    call.pid = task.caller_pid;
    call.daemon_owned = false;
    call.legacy = true;
    context.store.attach(
        &entry.request_id,
        &call,
        &crate::daemon::ask_dedup::fingerprint(task),
    )?;
    Ok(Some(LegacyWaiter {
        store: context.store.clone(),
        call,
        delivered: false,
    }))
}

impl Runtime {
    pub fn release_workers(&self) {
        let workers = std::mem::take(&mut *self.workers.lock().unwrap());
        for worker in workers.into_values() {
            for task in worker.tasks.lock().unwrap().drain(..) {
                task.abort();
            }
        }
    }

    pub async fn abort_update(&self, state: &Arc<ServerState>) {
        let _transition = self.transition.lock().await;
        if self.suspending.load(Ordering::SeqCst) {
            self.resume_surfaces(state).await;
        }
    }
    pub fn cancel_pending(&self) -> Result<()> {
        let workers: Vec<_> = self.workers.lock().unwrap().values().cloned().collect();
        for worker in workers {
            worker.context.suspend();
            if self
                .store
                .request(&worker.context.id)?
                .is_some_and(|saved| saved.decision.is_none() && saved.outcome.is_none())
            {
                self.store.finish(&worker.context.id, &Outcome::Cancelled)?;
                worker.events.send_replace(Some(Event::Final {
                    outcome: Outcome::Cancelled,
                }));
                self.stop_waiter(&worker.context.id);
            }
        }
        Ok(())
    }

    pub fn open() -> Result<Arc<Self>> {
        let store = Store::open(&crate::paths::config_dir().join("state/requests.sqlite3"))?;
        Ok(Arc::new(Self {
            store: Arc::new(store),
            workers: Mutex::new(HashMap::new()),
            submissions: tokio::sync::Mutex::new(()),
            admission: Arc::new(tokio::sync::RwLock::new(())),
            restoring: AtomicBool::new(true),
            initialized: AtomicBool::new(false),
            update_running: AtomicBool::new(false),
            committed: AtomicBool::new(false),
            transition: tokio::sync::Mutex::new(()),
            suspending: AtomicBool::new(false),
        }))
    }

    pub fn is_restoring(&self) -> bool {
        self.restoring.load(Ordering::SeqCst)
    }

    pub async fn admit(&self) -> tokio::sync::OwnedRwLockReadGuard<()> {
        self.admission.clone().read_owned().await
    }

    pub async fn close_admission(&self) -> tokio::sync::OwnedRwLockWriteGuard<()> {
        self.admission.clone().write_owned().await
    }

    pub async fn restore(self: &Arc<Self>, state: &Arc<ServerState>) -> Result<()> {
        if !self.initialized.load(Ordering::SeqCst) {
            self.store.discard_unowned()?;
            self.store.recover_owned_effects()?;
            state
                .failure_confirmations
                .reconcile_deliveries(&self.store)?;
            let saved: Option<crate::daemon::popup_inbox::SavedHost> = self
                .store
                .metadata::<Option<crate::daemon::popup_inbox::SavedHost>>("popup/host")?
                .flatten();
            if let Some(saved) = saved.filter(|saved| {
                crate::daemon::recovery::process_alive(saved.pid, saved.birth.as_deref())
            }) {
                *state.popup_inbox.lock().unwrap() =
                    crate::daemon::popup_inbox::Host::restore(saved);
            }
            if let Some(mut focus) = self.store.metadata::<PopupFocusArbiter>("popup/focus")? {
                let mut live = std::collections::HashSet::new();
                for saved in self.store.requests()? {
                    if saved.outcome.is_some() {
                        continue;
                    }
                    let binding = self
                        .store
                        .load_checkpoint(&saved.request.id, "popup/independent")?
                        .map(serde_json::from_value::<inbox::PopupBinding>)
                        .transpose()?;
                    if binding.is_some_and(|binding| {
                        crate::daemon::recovery::process_alive(
                            binding.pid,
                            binding.birth.as_deref(),
                        )
                    }) {
                        live.insert(saved.request.id);
                    }
                }
                focus.retain_live(&live);
                *state.popup_focus.lock().unwrap() = focus;
            }
            self.initialized.store(true, Ordering::SeqCst);
        }
        self.maintain()?;
        for saved in self.store.requests()? {
            if let (Task::Ask(task), Some(Outcome::Ask(outcome)), Some(finished)) =
                (&saved.request.task, &saved.outcome, saved.finished_at_ms)
            {
                let ago = Duration::from_millis(now_ms().saturating_sub(finished));
                let answered = saved
                    .decision
                    .as_ref()
                    .and_then(|decision| decision.get("answer"))
                    .and_then(|value| serde_json::from_value::<ChannelResult>(value.clone()).ok())
                    .is_some_and(|answer| answer.action == ChannelAction::Send);
                if answered && ago <= ask_dedup::REPLAY_WINDOW {
                    if let Some(key) = ask_dedup::session_key(task) {
                        state.replay.lock().unwrap().put(
                            &key,
                            ask_dedup::ReplayEntry {
                                fingerprint: ask_dedup::fingerprint(task),
                                outcome: outcome.clone(),
                                request_id: saved.request.id.clone(),
                                finished_at: Instant::now()
                                    .checked_sub(ago)
                                    .unwrap_or_else(Instant::now),
                            },
                        );
                    }
                }
            }
            if saved.outcome.is_none()
                && (saved.decision.is_some() || self.store.active_calls(&saved.request.id)? > 0)
            {
                let worker = self.start(saved.request, true, state);
                wait_started(worker.events.subscribe()).await;
                if matches!(*worker.events.borrow(), Some(Event::Unavailable)) {
                    anyhow::bail!("request restoration is temporarily unavailable");
                }
            }
        }
        let generation = state.popup_inbox.lock().unwrap().generation();
        if let Some(generation) = generation {
            let state = state.clone();
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_secs(5)).await;
                let waiting = {
                    let host = state.popup_inbox.lock().unwrap();
                    host.generation() == Some(generation) && host.sender().is_none()
                };
                if waiting && !state.closing.load(Ordering::SeqCst) {
                    inbox::recover_inbox(&state, generation);
                }
            });
        }
        self.restoring.store(false, Ordering::SeqCst);
        Ok(())
    }

    async fn submit(
        self: &Arc<Self>,
        call: &Call,
        task: Task,
        state: &Arc<ServerState>,
    ) -> Result<watch::Receiver<Option<Event>>> {
        let _submitting = self.submissions.lock().await;
        if self.is_restoring() || self.suspending.load(Ordering::SeqCst) {
            anyhow::bail!("daemon update is in progress");
        }
        let hash = format!("{:x}", sha2::Sha256::digest(serde_json::to_vec(&task)?));
        if let Some(resumed) = self.store.resume(call, Some(&hash))? {
            return self.subscribe(
                resumed.request_id,
                resumed.outcome,
                resumed.cancelled,
                state,
            );
        }
        if let Task::Ask(task) = &task {
            if let Some(key) = crate::daemon::ask_dedup::session_key(task) {
                let fingerprint = crate::daemon::ask_dedup::coalesce_fingerprint(task);
                let existing = state
                    .registry
                    .in_flight_entries()
                    .into_iter()
                    .find(|entry| {
                        entry.session_key.as_deref() == Some(&key)
                            && entry.coalesce_fingerprint == fingerprint
                            && !entry.coordinator.is_finalizing()
                            && entry.request().recovery.is_some()
                    });
                if let Some(entry) = existing {
                    self.store.attach(&entry.request_id, call, &hash)?;
                    return self.subscribe(entry.request_id.clone(), None, false, state);
                }
            }
        }
        let request = Request {
            id: call.id.clone(),
            task,
            show: None,
            created_at_ms: now_ms(),
        };
        self.store.accept(&request, call, &hash)?;
        let worker = self.start(request, false, state);
        let receiver = worker.events.subscribe();
        wait_started(receiver.clone()).await;
        Ok(receiver)
    }

    fn subscribe(
        self: &Arc<Self>,
        request_id: String,
        outcome: Option<Outcome>,
        cancelled: bool,
        state: &Arc<ServerState>,
    ) -> Result<watch::Receiver<Option<Event>>> {
        if let Some(outcome) = outcome {
            let (_, receiver) = watch::channel(Some(Event::Final { outcome }));
            return Ok(receiver);
        }
        if cancelled {
            anyhow::bail!("the caller cancelled this request");
        }
        if let Some(worker) = self.workers.lock().unwrap().get(&request_id) {
            return Ok(worker.events.subscribe());
        }
        let request = self
            .store
            .request(&request_id)?
            .context("saved request is missing")?
            .request;
        Ok(self.start(request, true, state).events.subscribe())
    }

    fn start(
        self: &Arc<Self>,
        request: Request,
        restoring: bool,
        state: &Arc<ServerState>,
    ) -> Arc<Worker> {
        let mut workers = self.workers.lock().unwrap();
        if let Some(worker) = workers.get(&request.id) {
            return worker.clone();
        }
        let (caller, handler) = tokio::io::duplex(64 * 1024);
        let (caller_read, caller_write) = tokio::io::split(caller);
        let (handler_read, handler_write) = tokio::io::split(handler);
        let (events, _) = watch::channel(None);
        let worker = Arc::new(Worker {
            tasks: Mutex::new(Vec::new()),
            context: RequestContext::new(self.store.clone(), request.id.clone(), restoring),
            events,
            caller: Mutex::new(Some(Box::new(caller_write))),
        });
        workers.insert(request.id.clone(), worker.clone());
        drop(workers);
        let setup = worker.context.operation();
        let context = worker.context.clone();
        let run_state = state.clone();
        let handler = tokio::spawn(async move {
            let reader: Reader = Box::new(BufReader::new(handler_read));
            let writer: Writer = Box::new(handler_write);
            match request.task {
                Task::Ask(task) => {
                    handle_submit_recoverable(*task, reader, writer, &run_state, Some(context))
                        .await
                }
                Task::Confirm(task) => {
                    handle_submit_confirm_recoverable(
                        *task,
                        reader,
                        writer,
                        &run_state,
                        Some(context),
                    )
                    .await
                }
            }
        });
        worker.tasks.lock().unwrap().push(handler.abort_handle());
        let runtime = self.clone();
        let observe_worker = worker.clone();
        let observer = tokio::spawn(async move {
            let mut setup = setup;
            let mut reader = BufReader::new(caller_read);
            let mut warning = None;
            loop {
                let mut outcome = match ipc::read_msg::<_, ServerMsg>(&mut reader).await {
                    Ok(Some(
                        ServerMsg::Accepted { request_id }
                        | ServerMsg::ConfirmAccepted { request_id },
                    )) => {
                        observe_worker
                            .events
                            .send_replace(Some(Event::Accepted { request_id }));
                        setup.take();
                        continue;
                    }
                    Ok(Some(ServerMsg::Warn { text })) => {
                        warning = Some(text);
                        continue;
                    }
                    Ok(Some(ServerMsg::Final { stdout, exit_code })) => {
                        Outcome::Ask(crate::app::RenderOutcome {
                            stdout,
                            stderr: warning.take(),
                            exit_code,
                        })
                    }
                    Ok(Some(ServerMsg::ConfirmFinal { result })) => Outcome::Confirm(result),
                    Ok(Some(ServerMsg::ConfirmFallback { reason })) => Outcome::Fallback(reason),
                    Ok(Some(_)) => continue,
                    Ok(None) | Err(_) => {
                        // During a handoff the next daemon resumes the saved request.
                        if runtime.suspending.load(Ordering::SeqCst) {
                            return;
                        }
                        observe_worker.events.send_replace(Some(Event::Unavailable));
                        runtime
                            .workers
                            .lock()
                            .unwrap()
                            .remove(&observe_worker.context.id);
                        return;
                    }
                };
                loop {
                    match runtime.store.finish(&observe_worker.context.id, &outcome) {
                        Ok(()) => break,
                        Err(error) => {
                            if let Ok(Some(saved)) =
                                runtime.store.request(&observe_worker.context.id)
                            {
                                if let Some(saved_outcome) = saved.outcome {
                                    outcome = saved_outcome;
                                    break;
                                }
                            }
                            log(&format!("cannot save recovery result: {error}"));
                        }
                    }
                    tokio::time::sleep(Duration::from_secs(1)).await;
                }
                observe_worker
                    .events
                    .send_replace(Some(Event::Final { outcome }));
                runtime
                    .workers
                    .lock()
                    .unwrap()
                    .remove(&observe_worker.context.id);
                return;
            }
        });
        worker.tasks.lock().unwrap().push(observer.abort_handle());
        worker
    }

    fn cancel(&self, call: &Call) -> Result<()> {
        if let Some(id) = self.store.cancel_call(call)? {
            self.stop_waiter(&id);
        }
        Ok(())
    }

    fn stop_waiter(&self, id: &str) {
        if let Some(worker) = self.workers.lock().unwrap().get(id) {
            if let Some(mut caller) = worker.caller.lock().unwrap().take() {
                tokio::spawn(async move {
                    use tokio::io::AsyncWriteExt;
                    let _ = caller.shutdown().await;
                });
            }
        }
    }

    pub fn maintain(&self) -> Result<()> {
        if self.suspending.load(Ordering::SeqCst) {
            return Ok(());
        }
        for id in self.store.consume_cancellations()? {
            self.stop_waiter(&id);
        }
        for call in self.store.process_calls()? {
            if !crate::daemon::recovery::process_alive(call.pid, call.birth.as_deref()) {
                if let Some(id) = self.store.cancel_absent_call(&call.id)? {
                    self.stop_waiter(&id);
                }
            }
        }
        self.store.prune(now_ms() as i64)?;
        Ok(())
    }
}

async fn wait_started(mut receiver: watch::Receiver<Option<Event>>) {
    loop {
        if receiver.borrow().is_some() {
            return;
        }
        if receiver.changed().await.is_err() {
            return;
        }
    }
}

async fn send(writer: &mut Writer, event: Event) -> std::io::Result<()> {
    ipc::write_msg(
        writer,
        &ServerMsg::Recovery {
            version: VERSION,
            event: serde_json::to_value(event)?,
        },
    )
    .await
}

async fn save_outcome(context: &RequestContext, outcome: Outcome) -> Outcome {
    loop {
        match context.store.finish(&context.id, &outcome) {
            Ok(()) => return outcome,
            Err(error) => {
                if let Ok(Some(saved)) = context.store.request(&context.id) {
                    if let Some(terminal) = saved.outcome {
                        return terminal;
                    }
                }
                log(&format!("cannot retain terminal request result: {error}"));
            }
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

pub(super) async fn save_ask(
    entry: &request::RequestEntry,
    outcome: RenderOutcome,
) -> RenderOutcome {
    let Some(context) = entry.request().recovery.as_ref() else {
        return outcome;
    };
    match save_outcome(context, Outcome::Ask(outcome)).await {
        Outcome::Ask(outcome) => outcome,
        _ => RenderOutcome {
            stdout: String::new(),
            stderr: None,
            exit_code: 3,
        },
    }
}

pub(super) async fn save_confirm(
    entry: &request::ConfirmEntry,
    outcome: ConfirmOutcome,
) -> ConfirmOutcome {
    let Some(context) = entry.coordinator.recovery() else {
        return outcome;
    };
    let outcome = match outcome {
        ConfirmOutcome::Final(result) => Outcome::Confirm(result),
        ConfirmOutcome::Fallback(reason) => Outcome::Fallback(reason),
    };
    match save_outcome(&context, outcome).await {
        Outcome::Confirm(result) => ConfirmOutcome::Final(result),
        Outcome::Fallback(reason) => ConfirmOutcome::Fallback(reason),
        _ => ConfirmOutcome::Fallback(ConfirmFallbackReason::InternalError),
    }
}

pub(super) async fn handle(
    version: u32,
    operation: serde_json::Value,
    reader: Reader,
    mut writer: Writer,
    state: &Arc<ServerState>,
) {
    let Some(runtime) = state.recovery.clone().filter(|_| version == VERSION) else {
        let _ = send(&mut writer, Event::Unavailable).await;
        return;
    };
    let operation = match serde_json::from_value::<ClientOperation>(operation) {
        Ok(operation) => operation,
        Err(_) => {
            let _ = send(&mut writer, Event::Unavailable).await;
            return;
        }
    };
    let (call, subscribed) = match operation {
        ClientOperation::Popup {
            token,
            pid,
            request_id,
        } => {
            inbox::reconnect_popup(token, pid, request_id, reader, writer, state).await;
            return;
        }
        ClientOperation::PopupHost {
            token,
            pid,
            known_ids,
        } => {
            inbox::handle_popup_host_supported(token, Some(pid), known_ids, reader, writer, state)
                .await;
            return;
        }
        ClientOperation::WarmPopup { pid } => {
            handle_gui_warm_supported(reader, writer, state, Some(pid)).await;
            return;
        }
        ClientOperation::Submit { call, task } => {
            let subscribed = runtime.submit(&call, task, state).await;
            (call, subscribed)
        }
        ClientOperation::Resume { call } => {
            let subscribed = runtime.store.resume(&call, None).and_then(|resumed| {
                let resumed = resumed.context("unknown recovery call")?;
                runtime.subscribe(
                    resumed.request_id,
                    resumed.outcome,
                    resumed.cancelled,
                    state,
                )
            });
            (call, subscribed)
        }
        ClientOperation::Cancel { call } => {
            if runtime.cancel(&call).is_ok() {
                let _ = send(&mut writer, Event::Updated).await;
            }
            return;
        }
        ClientOperation::Delivered { call } => {
            if runtime.store.delivered(&call).is_ok() {
                let _ = send(&mut writer, Event::Updated).await;
            }
            return;
        }
    };
    let mut events = match subscribed {
        Ok(receiver) => receiver,
        Err(error) => {
            log(&format!("recovery subscription failed: {error}"));
            let _ = send(&mut writer, Event::Unavailable).await;
            return;
        }
    };
    let mut reader = ipc::MessageReader::new(reader);
    loop {
        let event = events.borrow_and_update().clone();
        if let Some(event) = event {
            let finished = matches!(event, Event::Final { .. } | Event::Unavailable);
            if send(&mut writer, event).await.is_err() || finished {
                return;
            }
        }
        tokio::select! {
            changed = events.changed() => { if changed.is_err() { return; } }
            message = reader.read::<ClientMsg>() => {
                if matches!(message, Ok(Some(ClientMsg::Recover { .. }))) { continue; }
                if !runtime.suspending.load(Ordering::SeqCst) && !state.closing.load(Ordering::SeqCst)
                    && !call.daemon_owned && !crate::daemon::recovery::process_alive(call.pid, call.birth.as_deref()) {
                    if let Err(error) = runtime.cancel(&call) { log(&format!("cannot cancel departed recovery caller: {error}")); }
                }
                return;
            }
        }
    }
}

/// An unsupported legacy waiter postpones replacement while this daemon keeps accepting work.
pub(super) fn begin(state: &Arc<ServerState>) {
    let Some(runtime) = state.recovery.clone() else {
        begin_drain(state);
        return;
    };
    if runtime.update_running.swap(true, Ordering::SeqCst) {
        return;
    }
    let state = state.clone();
    tokio::spawn(async move {
        loop {
            if state.closing.load(Ordering::SeqCst)
                || *state.exit_intent.lock().unwrap()
                    != super::super::handoff::Intent::ReplaceOwnBinary
            {
                break;
            }
            match runtime.prepare(&state).await {
                Ok(true) => {
                    state.shutdown.notify_one();
                    return;
                }
                Ok(false) => {}
                Err(error) => log(&format!("waiting to complete the daemon update: {error}")),
            }
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
        runtime.update_running.store(false, Ordering::SeqCst);
    });
}

impl Runtime {
    fn supported_requests(&self, state: &ServerState) -> Result<bool> {
        if self.is_restoring()
            || self.store.has_legacy_waiters()?
            || self.store.pending_daemon_results()?
        {
            return Ok(false);
        }
        if state
            .registry
            .in_flight_entries()
            .iter()
            .any(|entry| entry.request().recovery.is_none())
        {
            return Ok(false);
        }
        if state
            .registry
            .in_flight_confirm_entries()
            .iter()
            .any(|entry| entry.coordinator.recovery().is_none())
        {
            return Ok(false);
        }
        if !state.popup_inbox.lock().unwrap().can_suspend() {
            return Ok(false);
        }
        for entry in state
            .registry
            .in_flight_entries()
            .into_iter()
            .map(InteractionEntry::Ask)
            .chain(
                state
                    .registry
                    .in_flight_confirm_entries()
                    .into_iter()
                    .map(InteractionEntry::Confirm),
            )
        {
            let gui = match &entry {
                InteractionEntry::Ask(entry) => entry.gui_connected.load(Ordering::SeqCst),
                InteractionEntry::Confirm(entry) => entry.gui_connected.load(Ordering::SeqCst),
            };
            if gui
                && state
                    .popup_inbox
                    .lock()
                    .unwrap()
                    .get(entry.request_id())
                    .is_none()
                && !inbox::retained_popup(&entry)
            {
                return Ok(false);
            }
        }
        Ok(true)
    }

    async fn prepare(&self, state: &Arc<ServerState>) -> Result<bool> {
        let _transition = self.transition.lock().await;
        if !self.supported_requests(state)? {
            return Ok(false);
        }
        let target = crate::executable::launch_path()?;
        let fingerprint = lifecycle::fingerprint_at(&target);
        let mut command = tokio::process::Command::new(&target);
        crate::development::clear_client_context(command.as_std_mut());
        command.arg("--recovery-capabilities");
        command.kill_on_drop(true);
        let output = tokio::time::timeout(Duration::from_secs(5), command.output()).await??;
        let capabilities: serde_json::Value = serde_json::from_slice(&output.stdout)?;
        let current = crate::daemon::recovery::capabilities();
        if !output.status.success()
            || capabilities["recovery"] != VERSION
            || capabilities["maxSchema"].as_u64().unwrap_or(0)
                < current["maxSchema"].as_u64().unwrap_or(0)
            || capabilities["minSchema"].as_u64().unwrap_or(u64::MAX)
                > current["maxSchema"].as_u64().unwrap_or(0)
        {
            return Ok(false);
        }
        let admission =
            tokio::time::timeout(Duration::from_secs(15), self.close_admission()).await?;
        if !self.supported_requests(state)?
            || *state.exit_intent.lock().unwrap() != super::super::handoff::Intent::ReplaceOwnBinary
        {
            return Ok(false);
        }
        self.suspending.store(true, Ordering::SeqCst);
        crate::daemon::recovery::set_handoff(true);
        state.draining.store(true, Ordering::SeqCst);
        let contexts: Vec<_> = self
            .workers
            .lock()
            .unwrap()
            .values()
            .map(|worker| worker.context.clone())
            .collect();
        for context in &contexts {
            context.begin_recovery();
            context.suspend();
        }
        let updating = ServerMsg::Recovery {
            version: VERSION,
            event: serde_json::to_value(Event::Updating)?,
        };
        broadcast_popup_state(state, updating);
        broadcast_tray_state(state);
        if let Some(router) = state.dd_router.lock().await.as_ref() {
            router.pause().await;
        }
        if let Some(router) = state.fs_router.lock().await.as_ref() {
            router.pause().await;
        }
        if let Some(router) = state.tg_router.lock().await.as_ref() {
            router.pause().await;
        }
        if let Some(router) = state.sl_router.lock().await.as_ref() {
            router.pause().await;
        }
        for entry in state.registry.in_flight_entries() {
            if !entry.coordinator.is_finalizing() {
                entry.coordinator.suspend();
            }
        }
        for entry in state.registry.in_flight_confirm_entries() {
            if !entry.coordinator.is_terminal() {
                entry.pause.notify_waiters();
            }
        }
        let prepared = async {
            let boundary_started = Instant::now();
            loop {
                anyhow::ensure!(
                    boundary_started.elapsed() < Duration::from_secs(30),
                    "waiting for active state writes before updating"
                );
                if *state.exit_intent.lock().unwrap()
                    != super::super::handoff::Intent::ReplaceOwnBinary
                {
                    anyhow::bail!("update interrupted by explicit stop");
                }
                let finalizing = self
                    .store
                    .requests()?
                    .iter()
                    .any(|saved| saved.decision.is_some() && saved.outcome.is_none());
                if contexts.iter().all(|context| context.is_quiescent())
                    && !finalizing
                    && !self.store.pending_daemon_results()?
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            for context in &contexts {
                context.flush()?;
            }
            self.store
                .set_metadata("popup/focus", &*state.popup_focus.lock().unwrap())?;
            if let Some(gateway) = ios_gateway(state) {
                gateway.suspend().await.map_err(anyhow::Error::msg)?;
                state.ios.lock().unwrap().take();
            }
            anyhow::ensure!(
                lifecycle::fingerprint_at(&target) == fingerprint,
                "update target changed during handoff"
            );
            anyhow::ensure!(
                *state.exit_intent.lock().unwrap()
                    == super::super::handoff::Intent::ReplaceOwnBinary,
                "update interrupted by explicit stop"
            );
            #[cfg(windows)]
            super::super::handoff::prepare()?;
            self.committed.store(true, Ordering::SeqCst);
            Ok::<_, anyhow::Error>(())
        }
        .await;
        if let Err(error) = prepared {
            self.resume_surfaces(state).await;
            drop(admission);
            return Err(error);
        }
        // Requests accepted at the final boundary remain on disk for the successor.
        drop(admission);
        Ok(true)
    }

    pub async fn repair_surfaces(&self, state: &Arc<ServerState>) {
        if self.is_restoring()
            || self.suspending.load(Ordering::SeqCst)
            || state.closing.load(Ordering::SeqCst)
        {
            return;
        }
        let config = state.config_snapshot();
        let popup = popup_should_dispatch(&config, has_display());
        for entry in state.registry.in_flight_entries() {
            let Some(context) = entry.request().recovery.as_ref() else {
                continue;
            };
            if !context.is_recovering()
                || !context.is_ready()
                || !context.can_restart_surfaces()
                || entry.coordinator.is_finalizing()
            {
                continue;
            }
            let Some(_operation) = context.operation() else {
                continue;
            };
            let mut warnings: Writer = Box::new(tokio::io::sink());
            attach_im_channels(
                &entry,
                state,
                &mut warnings,
                Lang::resolve(&entry.show.lang),
                popup,
                None,
                false,
                None,
                None,
            )
            .await;
            if popup && !entry.coordinator.has_channel("popup") {
                entry
                    .coordinator
                    .register(Arc::new(GuiHelperPopupChannel::new(
                        entry.request_id.clone(),
                        entry.gui.clone(),
                    )));
                let _ = dispatch_popup(&entry, state, "", false);
            }
        }
        for entry in state.registry.in_flight_confirm_entries() {
            let Some(context) = entry.coordinator.recovery() else {
                continue;
            };
            if !context.is_recovering()
                || !context.is_ready()
                || !context.can_restart_surfaces()
                || entry.coordinator.is_terminal()
            {
                continue;
            }
            let Some(_operation) = context.operation() else {
                continue;
            };
            let candidates = confirm_im_candidates(&entry, state, &config, popup);
            let retry: Vec<_> = candidates
                .into_iter()
                .filter(|channel| entry.retry_delivery(channel))
                .collect();
            attach_confirm_im_channels(&entry, state, &config, &retry).await;
            if popup && entry.retry_delivery("popup") {
                dispatch_interaction_popup(InteractionEntry::Confirm(entry), state, "", false);
            }
        }
    }

    pub async fn resume_surfaces(&self, state: &Arc<ServerState>) {
        self.committed.store(false, Ordering::SeqCst);
        for worker in self.workers.lock().unwrap().values() {
            worker.context.resume();
        }
        self.suspending.store(false, Ordering::SeqCst);
        crate::daemon::recovery::set_handoff(false);
        if *state.exit_intent.lock().unwrap() != super::super::handoff::Intent::Stop {
            state.draining.store(false, Ordering::SeqCst);
        }
        *state.dd_router.lock().await = None;
        *state.fs_router.lock().await = None;
        *state.tg_router.lock().await = None;
        *state.sl_router.lock().await = None;
        ensure_ios_gateway(state).await;
        let config = state.config_snapshot();
        for entry in state.registry.in_flight_confirm_entries() {
            if !entry.coordinator.is_terminal() {
                let popup_ready =
                    entry.is_ready("popup") && entry.gui_connected.load(Ordering::SeqCst);
                entry.delivery.lock().unwrap().clear();
                if popup_ready && popup_should_dispatch(&config, has_display()) {
                    entry.start_delivery("popup");
                    entry.mark_ready("popup", String::new());
                }
            }
        }
        self.repair_surfaces(state).await;
        broadcast_popup_state(
            state,
            ServerMsg::Recovery {
                version: VERSION,
                event: serde_json::to_value(Event::Updated).unwrap(),
            },
        );
        broadcast_tray_state(state);
    }
}
