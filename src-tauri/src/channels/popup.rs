//! 本地弹窗 Channel。
//!
//! `GuiHelperPopupChannel`：弹窗在独立 GUI Helper 进程；该 adapter 仅在被抢答时
//! 经 IPC 向 Helper 下发 `cancel`（窗口由 Helper 自行收尾关闭）。投递答案由 GUI 连接处理器
//! 直接调用协调器 `submit`，不经此 adapter。

use super::{Channel, ConversationOrigin, Interruption, ResultSink};
use crate::models::AskRequest;

/// Daemon 模式弹窗 adapter：被其它渠道抢答时，经共享的 GUI 发送端向 Helper 下发 `cancel`。
pub struct GuiHelperPopupChannel {
    request_id: String,
    /// GUI 连接的发送端槽位（Helper 连上后由连接处理器填入；未连上时为 None）。
    gui: std::sync::Arc<
        std::sync::Mutex<Option<tokio::sync::mpsc::UnboundedSender<crate::ipc::ServerMsg>>>,
    >,
}

impl GuiHelperPopupChannel {
    pub fn new(
        request_id: String,
        gui: std::sync::Arc<
            std::sync::Mutex<Option<tokio::sync::mpsc::UnboundedSender<crate::ipc::ServerMsg>>>,
        >,
    ) -> Self {
        Self { request_id, gui }
    }

    /// `keep_draft` asks the host to save what the user had typed before the card closes (spec
    /// `popup-draft-on-cancel`); it is only set when the ask was abandoned, never when a person
    /// answered or cancelled.
    fn interrupt_with(&self, reason: &Interruption, keep_draft: bool) {
        // The popup ignores the reason text and just closes; pass a string for diagnostics only.
        let winner = match reason {
            Interruption::Handoff => {
                if let Ok(slot) = self.gui.lock() {
                    if let Some(tx) = slot.as_ref() {
                        let _ = tx.send(crate::ipc::ServerMsg::Recovery {
                            version: crate::daemon::recovery::VERSION,
                            event: serde_json::json!({"event": "updating"}),
                        });
                    }
                }
                return;
            }
            Interruption::AnsweredBy(w) => w.clone(),
            Interruption::Cancelled(src) => src.clone(),
        };
        let keep_draft = keep_draft && matches!(reason, Interruption::Cancelled(_));
        if let Ok(slot) = self.gui.lock() {
            if let Some(tx) = slot.as_ref() {
                let _ = tx.send(crate::ipc::ServerMsg::Cancel {
                    request_id: self.request_id.clone(),
                    winner,
                    keep_draft,
                });
            }
        }
    }
}

impl Channel for GuiHelperPopupChannel {
    fn id(&self) -> &str {
        "popup"
    }

    fn start(&self, _request: &AskRequest, _origin: &ConversationOrigin, _sink: ResultSink) {
        // GUI Helper 进程由请求处理器 spawn；题目经 `show` 下发，答案经 GUI 连接回传协调器。
    }

    fn interrupt(&self, reason: &Interruption) {
        self.interrupt_with(reason, false);
    }

    fn interrupt_abandoned(&self, reason: &Interruption) {
        self.interrupt_with(reason, true);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ipc::ServerMsg;
    use std::sync::{Arc, Mutex};
    use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver};

    fn channel() -> (GuiHelperPopupChannel, UnboundedReceiver<ServerMsg>) {
        let (tx, rx) = unbounded_channel();
        let gui = Arc::new(Mutex::new(Some(tx)));
        (GuiHelperPopupChannel::new("r1".into(), gui), rx)
    }

    fn cancel_of(rx: &mut UnboundedReceiver<ServerMsg>) -> (String, bool) {
        match rx.try_recv().expect("a message for the host") {
            ServerMsg::Cancel {
                winner, keep_draft, ..
            } => (winner, keep_draft),
            _ => panic!("expected a Cancel"),
        }
    }

    /// Spec popup-draft-on-cancel D1: the host is only told to keep the draft when the ask was
    /// abandoned, never when a person answered it or cancelled it.
    #[test]
    fn the_host_keeps_the_draft_only_when_the_ask_was_abandoned() {
        let (popup, mut rx) = channel();
        popup.interrupt_abandoned(&Interruption::Cancelled("Caller".into()));
        assert_eq!(cancel_of(&mut rx), ("Caller".to_string(), true));

        popup.interrupt(&Interruption::Cancelled("Slack".into()));
        assert_eq!(cancel_of(&mut rx), ("Slack".to_string(), false));

        popup.interrupt(&Interruption::AnsweredBy("Slack".into()));
        assert_eq!(cancel_of(&mut rx), ("Slack".to_string(), false));
        popup.interrupt_abandoned(&Interruption::AnsweredBy("Slack".into()));
        assert_eq!(
            cancel_of(&mut rx),
            ("Slack".to_string(), false),
            "an answer is never an abandonment"
        );
    }

    /// A handoff keeps the card alive for the next daemon: nothing is cancelled and no draft is saved.
    #[test]
    fn a_handoff_closes_nothing_and_saves_nothing() {
        let (popup, mut rx) = channel();
        popup.interrupt_abandoned(&Interruption::Handoff);
        assert!(matches!(rx.try_recv(), Ok(ServerMsg::Recovery { .. })));
        assert!(rx.try_recv().is_err());
    }

    /// The flag rides on the wire only when set, so a host built before it reads the same message.
    #[test]
    fn the_flag_is_written_only_when_set_and_missing_reads_as_false() {
        let kept = serde_json::to_value(ServerMsg::Cancel {
            request_id: "r1".into(),
            winner: "Caller".into(),
            keep_draft: true,
        })
        .unwrap();
        assert_eq!(kept["keep_draft"], true);
        let plain = serde_json::to_value(ServerMsg::Cancel {
            request_id: "r1".into(),
            winner: "Slack".into(),
            keep_draft: false,
        })
        .unwrap();
        assert!(plain.get("keep_draft").is_none());

        let older: ServerMsg =
            serde_json::from_str(r#"{"type":"cancel","request_id":"r1","winner":"Slack"}"#)
                .unwrap();
        assert!(matches!(
            older,
            ServerMsg::Cancel {
                keep_draft: false,
                ..
            }
        ));
    }
}
