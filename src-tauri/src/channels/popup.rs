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
}

impl Channel for GuiHelperPopupChannel {
    fn id(&self) -> &str {
        "popup"
    }

    fn start(&self, _request: &AskRequest, _origin: &ConversationOrigin, _sink: ResultSink) {
        // GUI Helper 进程由请求处理器 spawn；题目经 `show` 下发，答案经 GUI 连接回传协调器。
    }

    fn interrupt(&self, reason: &Interruption) {
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
        if let Ok(slot) = self.gui.lock() {
            if let Some(tx) = slot.as_ref() {
                let _ = tx.send(crate::ipc::ServerMsg::Cancel {
                    request_id: self.request_id.clone(),
                    winner,
                });
            }
        }
    }
}
