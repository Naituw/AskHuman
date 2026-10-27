//! iOS as an interaction channel: hands each question to the gateway, which delivers it to the
//! paired phones (MQTT while the app is open, push otherwise), maps their answers back onto the
//! request and tells them when somebody else got there first.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use super::{Channel, ConversationOrigin, Interruption, ResultSink};
use crate::agents::AgentKind;
use crate::ios::gateway::requests::{AskSink, DeliverRequest, Resolution, Sink};
use crate::ios::gateway::IosGateway;
use crate::ios::views;
use crate::models::{AskRequest, ChannelResult};

pub const CHANNEL_ID: &str = "ios";

pub struct IosChannel {
    gateway: Arc<IosGateway>,
    request_id: String,
    /// The Agent session that asks, for the title the phone shows.
    session: Option<SessionRef>,
    /// Set by `start`; shared with the finalization task spawned by `interrupt`.
    sink: Arc<OnceLock<ResultSink>>,
    /// The request ended while the view was still being prepared: nothing may be delivered.
    interrupted: Arc<AtomicBool>,
}

impl IosChannel {
    pub fn new(gateway: Arc<IosGateway>, request_id: &str) -> Self {
        Self {
            gateway,
            request_id: request_id.to_string(),
            session: None,
            sink: Arc::new(OnceLock::new()),
            interrupted: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Looks the title of this Agent session up when the request is delivered.
    pub fn with_session(mut self, session: Option<SessionRef>) -> Self {
        self.session = session;
        self
    }
}

/// How long a request waits for the title of its session before it goes out with what is known.
pub const TITLE_WAIT: Duration = Duration::from_millis(300);

/// The Agent session a request belongs to: what it takes to find the session's title.
#[derive(Clone)]
pub struct SessionRef {
    pub kind: AgentKind,
    pub session_id: String,
    /// The title the daemon already remembers (cheap): used when a fresh read is too slow or
    /// finds nothing.
    pub cached_title: Option<String>,
    pub codex: crate::codex_desktop::Config,
}

impl SessionRef {
    /// The title for a phone: what the Agent itself calls the session (a fresh read of its files,
    /// so a rename the daemon has not heard of yet still shows), else what the daemon remembers,
    /// else the session's first message. Never waits longer than `limit`; one line, clipped.
    pub async fn title(&self, limit: Duration) -> Option<String> {
        self.title_with(limit, |session| {
            crate::agents::title::resolve_popup_title(
                session.kind,
                &session.session_id,
                session.cached_title.clone(),
                &session.codex,
            )
        })
        .await
    }

    async fn title_with(
        &self,
        limit: Duration,
        read: impl FnOnce(&SessionRef) -> Option<String> + Send + 'static,
    ) -> Option<String> {
        let session = self.clone();
        // The read is file and database work: off the async threads. A read that outlives the
        // limit is left to finish by itself and its result dropped.
        let fresh =
            tokio::time::timeout(limit, tokio::task::spawn_blocking(move || read(&session))).await;
        let title = match fresh {
            Ok(Ok(Some(title))) => Some(title),
            _ => self.cached_title.clone(),
        };
        title.as_deref().and_then(views::clean_session_title)
    }
}

/// The coordinator seen from the gateway.
struct CoordSink(ResultSink);

impl AskSink for CoordSink {
    fn try_submit(&self, result: ChannelResult) -> bool {
        self.0.try_submit(result)
    }

    fn winner(&self) -> Option<String> {
        self.0.winner_channel_id()
    }

    fn surface_lost(&self, reason: &str) {
        self.0.surface_lost(CHANNEL_ID, reason);
    }
}

impl Channel for IosChannel {
    fn id(&self) -> &str {
        CHANNEL_ID
    }

    fn start(&self, request: &AskRequest, origin: &ConversationOrigin, sink: ResultSink) {
        let _ = self.sink.set(sink.clone());
        let gateway = self.gateway.clone();
        let request = Arc::new(request.clone());
        let origin = origin.clone();
        let interrupted = self.interrupted.clone();
        let session = self.session.clone();
        tauri::async_runtime::spawn(async move {
            // Decoding attachment thumbnails is CPU and disk work.
            let created_at_ms = chrono::Utc::now().timestamp_millis();
            let view = {
                let request = request.clone();
                tauri::async_runtime::spawn_blocking(move || {
                    views::ask_view(&request, &origin, created_at_ms)
                })
            };
            let title = async {
                match &session {
                    Some(session) => session.title(TITLE_WAIT).await,
                    None => None,
                }
            };
            let (prepared, title) = tokio::join!(view, title);
            let Ok(mut built) = prepared else {
                sink.surface_lost(CHANNEL_ID, "failed to prepare the request");
                return;
            };
            built.view.session_title = title;
            if interrupted.load(Ordering::SeqCst) || sink.is_finalizing() {
                return;
            }
            gateway.deliver(DeliverRequest {
                view: built.view,
                sink: Sink::Ask(Arc::new(CoordSink(sink))),
                ask: Some(request),
                deadline_ms: None,
                assets: built.assets,
            });
        });
    }

    fn interrupt(&self, reason: &Interruption) {
        let resolution = match reason {
            Interruption::AnsweredBy(name) => Resolution::AnsweredElsewhere(name.clone()),
            Interruption::Cancelled(name) => Resolution::Cancelled(name.clone()),
        };
        self.interrupted.store(true, Ordering::SeqCst);
        let gateway = self.gateway.clone();
        let request_id = self.request_id.clone();
        let sink = self.sink.clone();
        tauri::async_runtime::spawn(async move {
            // The coordinator counts the finalizers it waits for right after interrupting the
            // losing channels; reporting in before that would be lost.
            tokio::time::sleep(Duration::from_millis(20)).await;
            gateway.resolve_and_wait(&request_id, resolution).await;
            if let Some(sink) = sink.get() {
                sink.notify_finalized();
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    fn session(cached: Option<&str>) -> SessionRef {
        SessionRef {
            kind: AgentKind::Claude,
            session_id: "session-1".into(),
            cached_title: cached.map(str::to_string),
            codex: Default::default(),
        }
    }

    /// A fast read answers at once, so this only has to be longer than a slow machine's hiccup.
    const LIMIT: Duration = Duration::from_millis(500);

    #[tokio::test]
    async fn a_fresh_read_beats_what_the_daemon_remembers() {
        let title = session(Some("Old name"))
            .title_with(LIMIT, |_| Some("New name".into()))
            .await;
        assert_eq!(title.as_deref(), Some("New name"));
    }

    #[tokio::test]
    async fn the_remembered_title_stands_in_when_the_read_finds_nothing() {
        let title = session(Some("Old name")).title_with(LIMIT, |_| None).await;
        assert_eq!(title.as_deref(), Some("Old name"));
    }

    #[tokio::test]
    async fn a_read_that_takes_too_long_is_left_behind() {
        let started = Instant::now();
        let title = session(Some("Old name"))
            .title_with(Duration::from_millis(50), |_| {
                std::thread::sleep(Duration::from_millis(1_500));
                Some("Too late".into())
            })
            .await;
        assert_eq!(title.as_deref(), Some("Old name"));
        assert!(
            started.elapsed() < Duration::from_millis(1_000),
            "the request does not wait for a slow read"
        );
    }

    #[tokio::test]
    async fn a_session_nobody_has_named_has_no_title() {
        assert_eq!(session(None).title_with(LIMIT, |_| None).await, None);
        assert_eq!(
            session(Some("  \n ")).title_with(LIMIT, |_| None).await,
            None
        );
    }

    #[tokio::test]
    async fn the_title_is_one_line_and_clipped() {
        let title = session(None)
            .title_with(LIMIT, |_| Some("Fix the\n flaky   login test".into()))
            .await;
        assert_eq!(title.as_deref(), Some("Fix the flaky login test"));

        let long = "x".repeat(500);
        let title = session(None)
            .title_with(LIMIT, move |_| Some(long))
            .await
            .unwrap();
        assert!(title.chars().count() <= crate::ios::wire::SESSION_TITLE_MAX_CHARS + 1);
        assert!(title.ends_with('…'));
    }
}
