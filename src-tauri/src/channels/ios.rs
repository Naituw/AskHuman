//! iOS as an interaction channel: hands each question to the gateway, which delivers it to the
//! paired phones (MQTT while the app is open, push otherwise), maps their answers back onto the
//! request and tells them when somebody else got there first.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use super::{Channel, ConversationOrigin, Interruption, ResultSink};
use crate::ios::gateway::requests::{AskSink, DeliverRequest, Resolution, Sink};
use crate::ios::gateway::IosGateway;
use crate::ios::views;
use crate::models::{AskRequest, ChannelResult};

pub const CHANNEL_ID: &str = "ios";

pub struct IosChannel {
    gateway: Arc<IosGateway>,
    request_id: String,
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
            sink: Arc::new(OnceLock::new()),
            interrupted: Arc::new(AtomicBool::new(false)),
        }
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
        tauri::async_runtime::spawn(async move {
            // Decoding attachment thumbnails is CPU and disk work.
            let created_at_ms = chrono::Utc::now().timestamp_millis();
            let prepared = {
                let request = request.clone();
                tauri::async_runtime::spawn_blocking(move || {
                    views::ask_view(&request, &origin, created_at_ms)
                })
                .await
            };
            let Ok(built) = prepared else {
                sink.surface_lost(CHANNEL_ID, "failed to prepare the request");
                return;
            };
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
