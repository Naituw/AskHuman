//! Request-scoped state and IPC bridge for the instance's single popup window.
use crate::history::DraftKey;
use crate::ipc::{ClientMsg, ServerMsg, ShowPayload};
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::sync::Mutex;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::{mpsc::UnboundedSender, oneshot, Notify};
use tokio::time::Instant;

/// How long the host holds still for the draft of a card whose caller went away (spec
/// `popup-draft-on-cancel` D5/D6): from the moment the card closed. The page reads the card the
/// instant it hears of the close and the history takes the draft within milliseconds, so this is a
/// bound on a page that never answers, not an expected wait. Past it the draft counts as given up,
/// so one lost reply cannot stall the host, or a later card, twice.
const DRAFT_SETTLE: Duration = Duration::from_secs(3);

type SubmissionReply = oneshot::Sender<Result<(), String>>;
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum CompletionFeedback {
    Sent,
    Submitted,
}
struct PendingSubmission {
    reply: Option<SubmissionReply>,
    completion: Option<CompletionFeedback>,
}
/// A card that closed because its caller went away: the page is reading what it held and the host
/// is keeping it in the history (spec `popup-draft-on-cancel` D5).
struct PendingDraft {
    key: DraftKey,
    since: Instant,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct Terminal {
    request_id: String,
    winner: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    completion: Option<CompletionFeedback>,
    /// The page should read what this card held and hand it back (`popup_inbox_draft`) before the
    /// card goes. Absent for every other way a request ends.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    keep_draft: bool,
}
#[derive(Default)]
struct Inner {
    updating: bool,
    requests: HashMap<String, ShowPayload>,
    terminal: HashSet<String>,
    active: Option<String>,
    submissions: HashMap<String, PendingSubmission>,
    presented: bool,
    launch_ids: HashMap<String, String>,
    focus: Option<String>,
    cycle: u64,
    arrival: u64,
    drafts: HashMap<String, PendingDraft>,
}
impl Inner {
    fn insert(&mut self, show: ShowPayload) -> bool {
        if self.terminal.contains(&show.request_id) {
            return false;
        }
        self.requests.insert(show.request_id.clone(), show);
        true
    }
    fn set_session_title(&mut self, id: &str, title: &str) -> bool {
        let Some(show) = self.requests.get_mut(id) else {
            return false;
        };
        if title.trim().is_empty() {
            return false;
        }
        show.agent_session_title = Some(title.to_string());
        true
    }
    fn finish(&mut self, id: &str, winner: &str) -> Option<Terminal> {
        self.finish_with(id, winner, false)
    }
    /// `keep_draft`: the caller went away, so what the card held is to be kept. Only a card that
    /// names its question (`draft_key`: an Ask, not a permission card) can have a draft; it is
    /// remembered as pending until `settle_draft` or until `DRAFT_SETTLE` has passed.
    fn finish_with(&mut self, id: &str, winner: &str, keep_draft: bool) -> Option<Terminal> {
        if !self.terminal.insert(id.to_owned()) {
            return None;
        }
        let key = keep_draft
            .then(|| {
                self.requests
                    .get(id)
                    .and_then(|show| show.draft_key.clone())
            })
            .flatten();
        self.requests.remove(id);
        self.launch_ids.remove(id);
        if self.active.as_deref() == Some(id) {
            self.active = None;
        }
        if self.focus.as_deref() == Some(id) {
            self.focus = None;
        }
        let submission = self.submissions.remove(id);
        let completion = submission
            .as_ref()
            .and_then(|submission| submission.completion)
            .filter(|_| winner == "popup");
        if let Some(reply) = submission.and_then(|submission| submission.reply) {
            let _ = reply.send(Ok(()));
        }
        let keep_draft = key.is_some();
        if let Some(key) = key {
            self.drafts.insert(
                id.to_owned(),
                PendingDraft {
                    key,
                    since: Instant::now(),
                },
            );
        }
        Some(Terminal {
            request_id: id.to_owned(),
            winner: winner.to_owned(),
            completion,
            keep_draft,
        })
    }
    /// When the last draft still being kept (only those under `key`, or all of them for `None`)
    /// stops holding anything up; `None` when nothing is being kept. A draft whose time is up
    /// counts as given up.
    fn drafts_due(&self, key: Option<&DraftKey>, now: Instant) -> Option<Instant> {
        self.drafts
            .values()
            .filter(|draft| key.is_none_or(|key| &draft.key == key))
            .map(|draft| draft.since + DRAFT_SETTLE)
            .filter(|due| *due > now)
            .max()
    }
    fn reject(&mut self, id: &str, error: String) {
        if let Some(reply) = self
            .submissions
            .remove(id)
            .and_then(|submission| submission.reply)
        {
            let _ = reply.send(Err(error));
        }
    }
    fn expire_submission(&mut self, id: &str) {
        // A late terminal still identifies the committed action after its caller timed out.
        if let Some(submission) = self.submissions.get_mut(id) {
            submission.reply = None;
        }
    }
}
pub struct Inbox {
    recoverable: bool,
    inner: Mutex<Inner>,
    tx: UnboundedSender<ClientMsg>,
    generation: u64,
    recovered: bool,
    /// Woken when a draft stops being kept, for whoever waits on `drafts_kept`.
    draft_change: Notify,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    updating: bool,
    pub requests: Vec<ShowPayload>,
    recovered: bool,
    presented: bool,
    focused_request_id: Option<String>,
}
impl Inbox {
    pub fn new(tx: UnboundedSender<ClientMsg>, generation: u64, recovered: bool) -> Self {
        Self {
            recoverable: false,
            inner: Mutex::new(Inner::default()),
            tx,
            generation,
            recovered,
            draft_change: Notify::new(),
        }
    }
    fn with_recovery(mut self, recoverable: bool) -> Self {
        self.recoverable = recoverable;
        self
    }
    pub fn snapshot(&self) -> Snapshot {
        let state = self.inner.lock().unwrap();
        let mut requests: Vec<_> = state.requests.values().cloned().collect();
        requests.sort_by(|a, b| {
            a.sequence
                .cmp(&b.sequence)
                .then_with(|| a.request_id.cmp(&b.request_id))
        });
        let recovered = self.recovered && !requests.is_empty();
        Snapshot {
            updating: state.updating,
            requests,
            recovered,
            presented: state.presented,
            focused_request_id: state.focus.clone(),
        }
    }
    pub fn request(&self, id: &str) -> Result<ShowPayload, String> {
        self.inner
            .lock()
            .unwrap()
            .requests
            .get(id)
            .cloned()
            .ok_or_else(|| "popup request is no longer pending".into())
    }
    pub fn launch_id(&self, id: &str) -> Option<String> {
        self.inner.lock().unwrap().launch_ids.get(id).cloned()
    }
    pub fn active(&self, id: &str) -> bool {
        let state = self.inner.lock().unwrap();
        state.presented && state.active.as_deref() == Some(id) && state.requests.contains_key(id)
    }
    pub fn presented(&self) -> bool {
        self.inner.lock().unwrap().presented
    }
    pub fn activate(&self, id: &str) -> Result<(), String> {
        let mut state = self.inner.lock().unwrap();
        if !state.requests.contains_key(id) {
            return Err("popup request is no longer pending".into());
        }
        state.active = Some(id.to_string());
        Ok(())
    }
    pub fn send(&self, message: ClientMsg) -> Result<(), String> {
        self.tx
            .send(message)
            .map_err(|_| "popup daemon connection is unavailable".into())
    }
    pub fn begin_idle(&self, app: &AppHandle) -> bool {
        let mut state = self.inner.lock().unwrap();
        if !state.requests.is_empty() {
            return false;
        }
        state.active = None;
        state.presented = false;
        state.cycle = state.cycle.wrapping_add(1);
        drop(state);
        if let Some(window) = app.get_webview_window("popup") {
            if let Err(error) = window.hide() {
                crate::daemon::lifecycle::log_runtime_event("popup_host", "idle_hide_failed", None);
                super::stderr_redirect::eprintln_real(&format!(
                    "popup host idle hide failed: {error}"
                ));
            }
        }
        #[cfg(target_os = "macos")]
        {
            let app = app.clone();
            let app_for_policy = app.clone();
            let _ = app.run_on_main_thread(move || {
                if !app_for_policy.state::<Inbox>().presented() {
                    let _ =
                        app_for_policy.set_activation_policy(tauri::ActivationPolicy::Accessory);
                }
            });
        }
        true
    }
    pub fn idle(&self, app: &AppHandle) {
        if self.begin_idle(app) {
            let _ = self.send(ClientMsg::PopupHostIdle {
                generation: self.generation,
            });
        }
    }
    pub async fn submit(&self, id: &str, message: ClientMsg) -> Result<(), String> {
        self.submit_inner(id, message, None).await
    }
    pub async fn submit_completed(
        &self,
        id: &str,
        message: ClientMsg,
        completion: CompletionFeedback,
    ) -> Result<(), String> {
        self.submit_inner(id, message, Some(completion)).await
    }
    async fn submit_inner(
        &self,
        id: &str,
        message: ClientMsg,
        completion: Option<CompletionFeedback>,
    ) -> Result<(), String> {
        let (reply, result) = oneshot::channel();
        {
            let mut state = self.inner.lock().unwrap();
            if !state.requests.contains_key(id) {
                return Err("popup request is no longer pending".into());
            }
            if state
                .submissions
                .get(id)
                .is_some_and(|submission| submission.reply.is_some())
            {
                return Err("popup answer is already being sent".into());
            }
            state.submissions.insert(
                id.to_string(),
                PendingSubmission {
                    reply: Some(reply),
                    completion,
                },
            );
        }
        if let Err(error) = self.send(message) {
            self.inner.lock().unwrap().submissions.remove(id);
            return Err(error);
        }
        let mut result = result;
        loop {
            match tokio::time::timeout(std::time::Duration::from_secs(5), &mut result).await {
                Ok(Ok(result)) => return result,
                Ok(Err(_)) => {
                    return Err(
                        "popup daemon connection closed before acknowledging the answer".into(),
                    );
                }
                Err(_) => {
                    if self.recoverable || self.inner.lock().unwrap().updating {
                        continue;
                    }
                    self.inner.lock().unwrap().expire_submission(id);
                    return Err("popup answer acknowledgement timed out".into());
                }
            }
        }
    }
    fn finish(&self, app: &AppHandle, id: String, winner: String, keep_draft: bool) {
        let mut state = self.inner.lock().unwrap();
        let Some(terminal) = state.finish_with(&id, &winner, keep_draft) else {
            return;
        };
        drop(state);
        let _ = app.emit("popup-inbox-terminal", terminal);
    }
    /// The question a closed card's draft is being kept under, while it is being kept. The page's
    /// report (`popup_inbox_draft`) is only taken for a request the host is waiting on.
    pub fn pending_draft(&self, id: &str) -> Option<DraftKey> {
        self.inner
            .lock()
            .unwrap()
            .drafts
            .get(id)
            .map(|draft| draft.key.clone())
    }
    /// The draft of `id` is kept, or there was nothing to keep: nothing waits on it any more.
    pub fn settle_draft(&self, id: &str) {
        if self.inner.lock().unwrap().drafts.remove(id).is_some() {
            self.draft_change.notify_waiters();
        }
    }
    /// Hold still while drafts are being kept (only those under `key`, or all for `None`): until
    /// each is settled, or `DRAFT_SETTLE` after its card closed. Returns whether they all were
    /// kept in time. Event-driven: woken by `settle_draft`, never polling.
    pub async fn drafts_kept(&self, key: Option<&DraftKey>) -> bool {
        loop {
            let changed = self.draft_change.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            let due = {
                let state = self.inner.lock().unwrap();
                state.drafts_due(key, Instant::now())
            };
            let Some(due) = due else {
                return true;
            };
            tokio::select! {
                _ = &mut changed => {}
                _ = tokio::time::sleep_until(due) => return false,
            }
        }
    }
    /// End the host process. A draft still being kept holds it for at most `DRAFT_SETTLE`;
    /// with none, it ends at once, as it always did (spec `popup-draft-on-cancel` D5).
    fn exit(&self, app: &AppHandle) {
        let busy = self
            .inner
            .lock()
            .unwrap()
            .drafts_due(None, Instant::now())
            .is_some();
        if !busy {
            app.exit(0);
            return;
        }
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            if !app.state::<Inbox>().drafts_kept(None).await {
                crate::daemon::lifecycle::log_runtime_event(
                    "popup_host",
                    "draft_wait_expired",
                    None,
                );
            }
            app.exit(0);
        });
    }
    pub fn receive(&self, app: &AppHandle, message: ServerMsg) {
        match message {
            ServerMsg::Recovery {
                version: crate::daemon::recovery::VERSION,
                event,
            } => {
                use crate::daemon::recovery::Event;
                let updating = match serde_json::from_value::<Event>(event) {
                    Ok(Event::Updating) => true,
                    Ok(Event::Updated) => false,
                    _ => return,
                };
                self.inner.lock().unwrap().updating = updating;
                let _ = app.emit("popup-connection", updating);
            }
            ServerMsg::Show(show) => {
                let mut state = self.inner.lock().unwrap();
                if !state.insert(show.clone()) {
                    return;
                }
                drop(state);
                let _ = app.emit("popup-inbox-show", show);
            }
            ServerMsg::PopupSessionTitle { request_id, title } => {
                if self
                    .inner
                    .lock()
                    .unwrap()
                    .set_session_title(&request_id, &title)
                {
                    let _ = app.emit(
                        "popup-session-title",
                        serde_json::json!({ "requestId": request_id, "title": title }),
                    );
                }
            }
            ServerMsg::PopupSubmissionAck {
                request_id,
                winner,
                error,
            } => {
                if let Some(error) = error {
                    self.inner.lock().unwrap().reject(&request_id, error);
                } else {
                    self.finish(
                        app,
                        request_id,
                        winner.unwrap_or_else(|| "system".into()),
                        false,
                    );
                }
            }
            ServerMsg::Cancel {
                request_id,
                winner,
                keep_draft,
            } => self.finish(app, request_id, winner, keep_draft),
            ServerMsg::PresentPopup { request_id, .. } => {
                let mut state = self.inner.lock().unwrap();
                if state.presented || state.active.as_deref() != Some(&request_id) {
                    return;
                }
                state.presented = true;
                let cycle = state.cycle;
                drop(state);
                let app = app.clone();
                tauri::async_runtime::spawn(async move {
                    if let Some(window) = app.get_webview_window("popup") {
                        let inbox = app.state::<Inbox>();
                        if !inbox.current_cycle(cycle) {
                            return;
                        }
                        #[cfg(target_os = "macos")]
                        {
                            let app_for_policy = app.clone();
                            let _ = app.run_on_main_thread(move || {
                                let _ = app_for_policy
                                    .set_activation_policy(tauri::ActivationPolicy::Regular);
                                crate::macos_dock_icon::set_dock_icon();
                            });
                        }
                        let config = crate::config::AppConfig::load_without_secrets();
                        let _ = window.set_always_on_top(config.general.always_on_top);
                        #[cfg(target_os = "macos")]
                        let _ = super::popup_transition::appear(
                            &window,
                            config.general.appear_animation.ns_animation_behavior(),
                        )
                        .await;
                        #[cfg(not(target_os = "macos"))]
                        let _ = super::popup_transition::front(&window).await;
                        let _ = app.emit("popup-inbox-presented", ());
                        inbox.notify_arrival(&window);
                    }
                });
            }
            ServerMsg::FocusPopup { request_id } => {
                self.inner.lock().unwrap().focus = Some(request_id.clone());
                let _ = app.emit("popup-inbox-focus", request_id);
            }
            ServerMsg::AgentResolved {
                request_id: Some(id),
                kind,
                pid,
                launch_id,
            } => {
                let mut state = self.inner.lock().unwrap();
                if let Some(launch) = launch_id.as_ref() {
                    state.launch_ids.insert(id.clone(), launch.clone());
                }
                if let Some(show) = state.requests.get_mut(&id) {
                    show.agent_kind = kind.clone().or(show.agent_kind.take());
                    show.agent_pid = pid.or(show.agent_pid);
                    drop(state);
                    let _ = app.emit("agent-resolved", serde_json::json!({ "requestId": id, "kind": kind, "pid": pid, "launchId": launch_id }));
                }
            }
            ServerMsg::PopupHostIdleAck {
                generation,
                keep_warm: false,
            } if generation == self.generation => self.exit(app),
            ServerMsg::PopupHostShutdown => self.exit(app),
            ServerMsg::UpdateState {
                available,
                latest_version,
                pending,
            } => {
                let update = crate::commands::PushedUpdateState {
                    available,
                    latest_version,
                    pending,
                    apply_mode: crate::update::apply_mode(),
                };
                crate::commands::set_pushed_update(update.clone());
                let _ = app.emit("update-state", update);
            }
            ServerMsg::ConfigChanged { general } => {
                // Native materials must follow the theme before the WebView updates its colors.
                if let Some(theme) = general.get("theme").and_then(serde_json::Value::as_str) {
                    crate::commands::apply_theme_to_windows(app, theme);
                }
                if let Some(effect) = general
                    .get("windowEffect")
                    .and_then(serde_json::Value::as_str)
                    .and_then(super::parse_window_effect)
                {
                    super::apply_window_effect_to_all(app, effect);
                }
                let _ = app.emit("settings-updated", general);
            }
            _ => {}
        }
    }
    fn current_cycle(&self, cycle: u64) -> bool {
        let state = self.inner.lock().unwrap();
        state.presented && state.cycle == cycle && !state.requests.is_empty()
    }
    pub fn notify_arrival(&self, window: &tauri::WebviewWindow) {
        let (cycle, arrival) = {
            let mut state = self.inner.lock().unwrap();
            state.arrival = state.arrival.wrapping_add(1);
            (state.cycle, state.arrival)
        };
        let window = window.clone();
        tauri::async_runtime::spawn(async move {
            // A burst produces one sound after the final geometry transaction.
            tokio::time::sleep(std::time::Duration::from_millis(180)).await;
            let inbox = window.state::<Inbox>();
            if !inbox.current_cycle(cycle) || inbox.inner.lock().unwrap().arrival != arrival {
                return;
            }
            let config = crate::config::AppConfig::load_without_secrets();
            crate::sound::play(&config.general.popup_sound);
        });
    }
}

pub(super) fn setup(app: &mut tauri::App, ipc: super::PopupIpc) -> tauri::Result<()> {
    let (generation, recovered) = ipc.host.expect("shared popup requires a host lease");
    app.manage(Inbox::new(ipc.gui_tx, generation, recovered).with_recovery(ipc.recoverable));
    let app = app.handle().clone();
    tauri::async_runtime::spawn(async move {
        let mut reader = ipc.reader;
        loop {
            match crate::ipc::read_msg::<_, ServerMsg>(&mut reader).await {
                Ok(Some(message)) => app.state::<Inbox>().receive(&app, message),
                Ok(None) | Err(_) => {
                    // The daemon is gone (a stop closes the connection after cancelling): a draft
                    // it just handed over is still worth the few moments it takes to keep.
                    app.state::<Inbox>().exit(&app);
                    break;
                }
            }
        }
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    fn show(id: &str) -> ShowPayload {
        serde_json::from_value(serde_json::json!({
            "requestId": id, "source": "test", "lang": "en", "project": "/test",
            "interaction": { "type": "ask", "request": {
                "id": id, "message": { "text": "test", "files": [] }, "questions": [],
                "isMarkdown": true, "selectOnly": false, "single": false, "outputFormat": "text"
            }}
        }))
        .unwrap()
    }
    fn message(id: &str) -> ClientMsg {
        ClientMsg::Answer {
            request_id: id.into(),
            action: crate::models::ChannelAction::Cancel,
            answers: vec![],
            directive: None,
            annotations: Vec::new(),
        }
    }
    fn key(session: &str) -> DraftKey {
        DraftKey {
            session_key: Some(session.into()),
            fingerprint: "f".into(),
        }
    }
    /// An Ask card, which names its question (a permission card does not).
    fn keyed(id: &str, session: &str) -> ShowPayload {
        let mut show = show(id);
        show.draft_key = Some(key(session));
        show
    }
    #[test]
    fn a_reopened_notice_requires_a_pending_request() {
        let (tx, _) = tokio::sync::mpsc::unbounded_channel();
        let inbox = Inbox::new(tx, 1, true);
        assert!(!inbox.snapshot().recovered);
        inbox.inner.lock().unwrap().insert(show("restored"));
        assert!(inbox.snapshot().recovered);
    }

    #[test]
    fn session_titles_survive_snapshots_and_cannot_revive_finished_requests() {
        let (tx, _) = tokio::sync::mpsc::unbounded_channel();
        let inbox = Inbox::new(tx, 1, false);
        let mut state = inbox.inner.lock().unwrap();
        state.insert(show("a"));
        state.insert(show("b"));
        assert!(state.set_session_title("a", "Task A"));
        assert!(!state.set_session_title("b", "  "));
        assert!(!state.set_session_title("missing", "Other task"));
        drop(state);
        let snapshot = inbox.snapshot();
        let a = snapshot
            .requests
            .iter()
            .find(|show| show.request_id == "a")
            .unwrap();
        let b = snapshot
            .requests
            .iter()
            .find(|show| show.request_id == "b")
            .unwrap();
        assert_eq!(a.agent_session_title.as_deref(), Some("Task A"));
        assert!(b.agent_session_title.is_none());
        let mut state = inbox.inner.lock().unwrap();
        state.finish("a", "popup");
        assert!(!state.set_session_title("a", "Late title"));
        assert!(!state.insert(show("a")));
    }
    #[test]
    fn ended_requests_cannot_be_replayed_or_activated() {
        let (tx, _) = tokio::sync::mpsc::unbounded_channel();
        let inbox = Inbox::new(tx, 1, false);
        inbox.inner.lock().unwrap().insert(show("a"));
        inbox.activate("a").unwrap();
        assert!(inbox.inner.lock().unwrap().finish("a", "popup").is_some());
        assert!(!inbox.inner.lock().unwrap().insert(show("a")));
        assert!(inbox.activate("a").is_err());
        assert!(inbox.snapshot().requests.is_empty());
    }
    #[test]
    fn completion_feedback_requires_a_local_committed_answer() {
        for (winner, completion, expected) in [
            ("popup", Some(CompletionFeedback::Sent), Some("sent")),
            (
                "popup",
                Some(CompletionFeedback::Submitted),
                Some("submitted"),
            ),
            ("popup", None, None),
            ("slack", Some(CompletionFeedback::Sent), None),
            ("system", Some(CompletionFeedback::Submitted), None),
        ] {
            let mut state = Inner::default();
            state.insert(show("a"));
            let (reply, _) = oneshot::channel();
            state.submissions.insert(
                "a".into(),
                PendingSubmission {
                    reply: Some(reply),
                    completion,
                },
            );
            let terminal = state.finish("a", winner).unwrap();
            let payload = serde_json::to_value(terminal).unwrap();
            assert_eq!(payload["requestId"], "a");
            assert_eq!(
                payload.get("completion").and_then(|value| value.as_str()),
                expected
            );
            assert!(
                state.finish("a", winner).is_none(),
                "Cancel and Ack must emit only once"
            );
        }
    }
    #[test]
    fn late_terminal_retains_timed_out_intent_but_rejection_clears_it() {
        let mut state = Inner::default();
        for id in ["a", "b"] {
            state.insert(show(id));
            let (reply, _) = oneshot::channel();
            state.submissions.insert(
                id.into(),
                PendingSubmission {
                    reply: Some(reply),
                    completion: Some(CompletionFeedback::Sent),
                },
            );
            state.expire_submission(id);
        }
        assert_eq!(
            state.finish("a", "popup").unwrap().completion,
            Some(CompletionFeedback::Sent)
        );
        state.reject("b", "invalid".into());
        assert!(state.requests.contains_key("b"));
        assert_eq!(state.finish("b", "popup").unwrap().completion, None);
    }
    #[tokio::test]
    async fn rejection_keeps_the_pending_form_and_allows_retry() {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let inbox = Arc::new(Inbox::new(tx, 1, false));
        inbox.inner.lock().unwrap().insert(show("a"));
        let submit = {
            let inbox = inbox.clone();
            tokio::spawn(async move { inbox.submit("a", message("a")).await })
        };
        rx.recv().await.unwrap();
        assert!(inbox
            .submit("a", message("a"))
            .await
            .unwrap_err()
            .contains("already being sent"));
        inbox
            .inner
            .lock()
            .unwrap()
            .reject("a", "invalid choice".into());
        assert_eq!(submit.await.unwrap(), Err("invalid choice".into()));
        assert!(inbox.request("a").is_ok());
        assert!(!inbox.inner.lock().unwrap().submissions.contains_key("a"));
    }
    #[tokio::test]
    async fn acknowledgements_are_per_request_and_unknown_ids_are_rejected() {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let inbox = Arc::new(Inbox::new(tx, 1, false));
        for id in ["a", "b"] {
            inbox.inner.lock().unwrap().insert(show(id));
        }
        let a = {
            let inbox = inbox.clone();
            tokio::spawn(async move { inbox.submit("a", message("a")).await })
        };
        let b = {
            let inbox = inbox.clone();
            tokio::spawn(async move { inbox.submit("b", message("b")).await })
        };
        rx.recv().await.unwrap();
        rx.recv().await.unwrap();
        assert!(inbox.inner.lock().unwrap().finish("b", "popup").is_some());
        assert_eq!(b.await.unwrap(), Ok(()));
        assert!(inbox.request("a").is_ok());
        assert!(!a.is_finished());
        assert!(inbox.inner.lock().unwrap().finish("a", "popup").is_some());
        assert_eq!(a.await.unwrap(), Ok(()));
        assert!(inbox.submit("missing", message("missing")).await.is_err());
    }

    /// Spec popup-draft-on-cancel D1/D5: only an ask card that is closed because its caller went
    /// away is asked for its draft; every other ending is exactly what it was.
    #[test]
    fn only_an_abandoned_ask_card_is_asked_for_its_draft() {
        let mut state = Inner::default();
        for id in ["abandoned", "answered", "permission"] {
            if id == "permission" {
                state.insert(show(id));
            } else {
                state.insert(keyed(id, "sid:a"));
            }
        }
        let abandoned = state.finish_with("abandoned", "caller", true).unwrap();
        assert!(abandoned.keep_draft);
        assert_eq!(
            state.drafts.get("abandoned").map(|draft| draft.key.clone()),
            Some(key("sid:a"))
        );
        let wire = serde_json::to_value(&abandoned).unwrap();
        assert_eq!(wire["keepDraft"], true);
        assert_eq!(wire["winner"], "caller");

        // Answered elsewhere: the flag is not set, so the user already said what they meant.
        let answered = state.finish_with("answered", "telegram", false).unwrap();
        assert!(!answered.keep_draft);
        assert!(serde_json::to_value(&answered)
            .unwrap()
            .get("keepDraft")
            .is_none());
        // A card with no key (a permission card) has no draft to keep, whatever was asked.
        let permission = state.finish_with("permission", "caller", true).unwrap();
        assert!(!permission.keep_draft);
        // A request the host never held has none either.
        let unknown = state.finish_with("unknown", "system", true).unwrap();
        assert!(!unknown.keep_draft);
        assert_eq!(state.drafts.len(), 1);
        // The ordinary finish never keeps a draft.
        state.insert(keyed("plain", "sid:a"));
        assert!(!state.finish("plain", "popup").unwrap().keep_draft);
        assert_eq!(state.drafts.len(), 1);
        // Ending twice reports once, and the pending draft stays as it was.
        assert!(state.finish_with("abandoned", "caller", true).is_none());
        assert_eq!(state.drafts.len(), 1);
    }

    /// What holds the host: drafts of the asked question only, and only until their time is up.
    #[test]
    fn a_draft_holds_things_up_only_for_its_own_question_and_only_for_a_while() {
        let mut state = Inner::default();
        let start = Instant::now();
        assert_eq!(state.drafts_due(None, start), None);
        state.insert(keyed("a", "sid:a"));
        state.insert(keyed("b", "sid:b"));
        state.finish_with("a", "caller", true).unwrap();
        state.drafts.get_mut("a").unwrap().since = start;
        state.finish_with("b", "caller", true).unwrap();
        state.drafts.get_mut("b").unwrap().since = start + Duration::from_secs(1);

        assert_eq!(
            state.drafts_due(Some(&key("sid:a")), start),
            Some(start + DRAFT_SETTLE)
        );
        assert_eq!(
            state.drafts_due(None, start),
            Some(start + Duration::from_secs(1) + DRAFT_SETTLE),
            "the last one to run out"
        );
        assert_eq!(state.drafts_due(Some(&key("sid:c")), start), None);
        let other_question = DraftKey {
            session_key: Some("sid:a".into()),
            fingerprint: "g".into(),
        };
        assert_eq!(state.drafts_due(Some(&other_question), start), None);
        // Time is up for a: it no longer holds anything, b still does.
        let after_a = start + DRAFT_SETTLE;
        assert_eq!(state.drafts_due(Some(&key("sid:a")), after_a), None);
        assert!(state.drafts_due(None, after_a).is_some());
        assert_eq!(
            state.drafts_due(None, start + Duration::from_secs(1) + DRAFT_SETTLE),
            None
        );
    }

    fn inbox_with_drafts(sessions: &[&str]) -> Arc<Inbox> {
        let (tx, _) = tokio::sync::mpsc::unbounded_channel();
        let inbox = Arc::new(Inbox::new(tx, 1, false));
        let mut state = inbox.inner.lock().unwrap();
        for session in sessions {
            state.insert(keyed(session, session));
            state.finish_with(session, "caller", true).unwrap();
        }
        drop(state);
        inbox
    }

    #[tokio::test(start_paused = true)]
    async fn nothing_waits_when_nothing_is_being_kept() {
        let inbox = inbox_with_drafts(&[]);
        let start = Instant::now();
        assert!(inbox.drafts_kept(None).await);
        assert_eq!(start.elapsed(), Duration::ZERO);
        // A late or repeated report for a request that is not being kept changes nothing.
        assert!(inbox.pending_draft("zzz").is_none());
        inbox.settle_draft("zzz");
    }

    #[tokio::test(start_paused = true)]
    async fn the_host_waits_for_the_draft_to_be_kept_and_not_a_moment_longer() {
        let inbox = inbox_with_drafts(&["sid:a"]);
        assert_eq!(inbox.pending_draft("sid:a"), Some(key("sid:a")));
        let start = Instant::now();
        let waiter = {
            let inbox = inbox.clone();
            tokio::spawn(async move { inbox.drafts_kept(None).await })
        };
        tokio::time::sleep(Duration::from_millis(500)).await;
        assert!(!waiter.is_finished(), "still being kept");
        inbox.settle_draft("sid:a");
        assert!(waiter.await.unwrap(), "kept in time");
        assert_eq!(start.elapsed(), Duration::from_millis(500));
        assert!(inbox.pending_draft("sid:a").is_none());
    }

    #[tokio::test(start_paused = true)]
    async fn a_page_that_never_answers_holds_the_host_for_a_bounded_time() {
        let inbox = inbox_with_drafts(&["sid:a"]);
        let start = Instant::now();
        assert!(!inbox.drafts_kept(None).await, "gave up, not kept");
        assert_eq!(start.elapsed(), DRAFT_SETTLE);
        // It is given up for good: the next wait does not start the clock again.
        let again = Instant::now();
        assert!(inbox.drafts_kept(None).await);
        assert_eq!(again.elapsed(), Duration::ZERO);
    }

    #[tokio::test(start_paused = true)]
    async fn a_new_card_waits_only_for_the_draft_of_its_own_question() {
        let inbox = inbox_with_drafts(&["sid:a", "sid:b"]);
        let start = Instant::now();
        let waiter = {
            let inbox = inbox.clone();
            tokio::spawn(async move { inbox.drafts_kept(Some(&key("sid:a"))).await })
        };
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(!waiter.is_finished());
        inbox.settle_draft("sid:b");
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(!waiter.is_finished(), "another question's draft is not its");
        inbox.settle_draft("sid:a");
        assert!(waiter.await.unwrap());
        assert_eq!(start.elapsed(), Duration::from_millis(200));
        // A question with no draft pending at all does not wait either.
        assert!(inbox.drafts_kept(Some(&key("sid:c"))).await);
    }

    #[tokio::test(start_paused = true)]
    async fn every_waiter_hears_that_a_draft_was_kept() {
        let inbox = inbox_with_drafts(&["sid:a"]);
        let waiters: Vec<_> = (0..3)
            .map(|_| {
                let inbox = inbox.clone();
                tokio::spawn(async move { inbox.drafts_kept(Some(&key("sid:a"))).await })
            })
            .collect();
        tokio::time::sleep(Duration::from_millis(50)).await;
        inbox.settle_draft("sid:a");
        for waiter in waiters {
            assert!(waiter.await.unwrap());
        }
    }
}
