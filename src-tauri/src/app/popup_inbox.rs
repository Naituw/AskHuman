//! Request-scoped state and IPC bridge for the instance's single popup window.
use crate::ipc::{ClientMsg, ServerMsg, ShowPayload};
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::{mpsc::UnboundedSender, oneshot};

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
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct Terminal {
    request_id: String,
    winner: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    completion: Option<CompletionFeedback>,
}
#[derive(Default)]
struct Inner {
    requests: HashMap<String, ShowPayload>,
    terminal: HashSet<String>,
    active: Option<String>,
    submissions: HashMap<String, PendingSubmission>,
    presented: bool,
    launch_ids: HashMap<String, String>,
    focus: Option<String>,
    cycle: u64,
    arrival: u64,
}
impl Inner {
    fn insert(&mut self, show: ShowPayload) -> bool {
        if self.terminal.contains(&show.request_id) {
            return false;
        }
        self.requests.insert(show.request_id.clone(), show);
        true
    }
    fn finish(&mut self, id: &str, winner: &str) -> Option<Terminal> {
        if !self.terminal.insert(id.to_owned()) {
            return None;
        }
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
        Some(Terminal {
            request_id: id.to_owned(),
            winner: winner.to_owned(),
            completion,
        })
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
    inner: Mutex<Inner>,
    tx: UnboundedSender<ClientMsg>,
    generation: u64,
    recovered: bool,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub requests: Vec<ShowPayload>,
    recovered: bool,
    focused_request_id: Option<String>,
}
impl Inbox {
    pub fn new(tx: UnboundedSender<ClientMsg>, generation: u64, recovered: bool) -> Self {
        Self {
            inner: Mutex::new(Inner::default()),
            tx,
            generation,
            recovered,
        }
    }
    pub fn snapshot(&self) -> Snapshot {
        let state = self.inner.lock().unwrap();
        let mut requests: Vec<_> = state.requests.values().cloned().collect();
        requests.sort_by(|a, b| {
            a.sequence
                .cmp(&b.sequence)
                .then_with(|| a.request_id.cmp(&b.request_id))
        });
        Snapshot {
            requests,
            recovered: self.recovered,
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
        super::popup_pulse::cancel();
        if let Some(window) = app.get_webview_window("popup") {
            let _ = window.hide();
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
        match tokio::time::timeout(std::time::Duration::from_secs(5), result).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => {
                Err("popup daemon connection closed before acknowledging the answer".into())
            }
            Err(_) => {
                self.inner.lock().unwrap().expire_submission(id);
                Err("popup answer acknowledgement timed out".into())
            }
        }
    }
    fn finish(&self, app: &AppHandle, id: String, winner: String) {
        let mut state = self.inner.lock().unwrap();
        let Some(terminal) = state.finish(&id, &winner) else {
            return;
        };
        drop(state);
        let _ = app.emit("popup-inbox-terminal", terminal);
    }
    pub fn receive(&self, app: &AppHandle, message: ServerMsg) {
        match message {
            ServerMsg::Show(show) => {
                let mut state = self.inner.lock().unwrap();
                if !state.insert(show.clone()) {
                    return;
                }
                drop(state);
                let _ = app.emit("popup-inbox-show", show);
            }
            ServerMsg::PopupSubmissionAck {
                request_id,
                winner,
                error,
            } => {
                if let Some(error) = error {
                    self.inner.lock().unwrap().reject(&request_id, error);
                } else {
                    self.finish(app, request_id, winner.unwrap_or_else(|| "system".into()));
                }
            }
            ServerMsg::Cancel { request_id, winner } => self.finish(app, request_id, winner),
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
                        let _ = super::popup_transition::front(&window).await;
                        let config = crate::config::AppConfig::load_without_secrets();
                        let _ = window.set_always_on_top(config.general.always_on_top);
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
            } if generation == self.generation => app.exit(0),
            ServerMsg::PopupHostShutdown => app.exit(0),
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
            // A burst produces one sound and one pulse after the final geometry transaction.
            tokio::time::sleep(std::time::Duration::from_millis(180)).await;
            let inbox = window.state::<Inbox>();
            if !inbox.current_cycle(cycle) || inbox.inner.lock().unwrap().arrival != arrival {
                return;
            }
            let config = crate::config::AppConfig::load_without_secrets();
            crate::sound::play(&config.general.popup_sound);
            let _ = super::popup_pulse::pulse(&window).await;
        });
    }
}

pub(super) fn setup(app: &mut tauri::App, ipc: super::PopupIpc) -> tauri::Result<()> {
    let (generation, recovered) = ipc.host.expect("shared popup requires a host lease");
    app.manage(Inbox::new(ipc.gui_tx, generation, recovered));
    let app = app.handle().clone();
    tauri::async_runtime::spawn(async move {
        let mut reader = ipc.reader;
        loop {
            match crate::ipc::read_msg::<_, ServerMsg>(&mut reader).await {
                Ok(Some(message)) => app.state::<Inbox>().receive(&app, message),
                Ok(None) | Err(_) => {
                    app.exit(0);
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
        }
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
}
