//! 请求登记表：每个活动 `request_id` 一套 Coordinator + Channel 任务 + GUI 关联。
//!
//! Phase 1 仅挂 popup adapter：提交时建 Coordinator（Ipc 退出）、注册弹窗 adapter、分配一次性 token；
//! GUI Helper 连上后凭 token 找到该请求、收 `show`、回 `answer`。IM 渠道在 Phase 2 接入。

use crate::app::coordinator::Coordinator;
use crate::app::RenderOutcome;
use crate::channels::popup::GuiHelperPopupChannel;
use crate::i18n::Lang;
use crate::ipc::{ServerMsg, ShowPayload, TaskRequest};
use crate::models::AskRequest;
use std::collections::HashMap;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use tokio::sync::Notify;

/// GUI 连接的发送端槽位：Helper 连上后填入，供 popup adapter 下发 `cancel`。
pub type GuiSlot = Arc<Mutex<Option<UnboundedSender<ServerMsg>>>>;

/// 一个活动请求的共享状态。
pub struct RequestEntry {
    pub request_id: String,
    /// 一次性 token：GUI Helper 连回握手出示。
    pub token: String,
    /// 抢答协调器（Ipc 退出，结果经 `final_tx` 回传）。
    pub coordinator: Arc<Coordinator>,
    /// 给 GUI Helper 的题目下发负载。
    pub show: ShowPayload,
    /// GUI 发送端槽位（adapter 与连接处理器共享）。
    pub gui: GuiSlot,
    /// GUI Helper 是否已连上（用于看门狗判定弹窗是否成功拉起）。
    pub gui_connected: AtomicBool,
    /// CLI 断开 / 请求结束时通知 GUI 连接处理器收尾。
    pub cancel: Arc<Notify>,
    /// 渲染结果发送端（协调器 finish 与看门狗共用；连接处理器从对应 rx 取）。
    pub final_tx: UnboundedSender<RenderOutcome>,
}

#[derive(Default)]
struct Inner {
    by_id: HashMap<String, Arc<RequestEntry>>,
    by_token: HashMap<String, String>,
}

/// 全局请求登记表（Daemon 内唯一）。
pub struct RequestRegistry {
    inner: Mutex<Inner>,
}

impl RequestRegistry {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            inner: Mutex::new(Inner::default()),
        })
    }

    /// 建立一个请求：分配 request_id / token，建 Coordinator（Ipc）+ popup adapter。
    /// 返回登记项与「渲染结果接收端」（连接处理器据此写 IPC `final`）。
    pub fn create(&self, task: TaskRequest) -> (Arc<RequestEntry>, UnboundedReceiver<RenderOutcome>) {
        let request_id = uuid::Uuid::new_v4().to_string();
        let token = uuid::Uuid::new_v4().to_string();
        let lang = Lang::resolve(&task.lang);

        // Daemon 分配权威 request_id（用于临时目录）。
        let mut request = AskRequest::new(task.message, task.questions, task.is_markdown);
        request.id = request_id.clone();

        let (final_tx, final_rx) = tokio::sync::mpsc::unbounded_channel();
        let coordinator = Coordinator::new_ipc(
            request.clone(),
            lang,
            final_tx.clone(),
            task.project.clone(),
            task.source.clone(),
        );

        let gui: GuiSlot = Arc::new(Mutex::new(None));
        coordinator.register(Arc::new(GuiHelperPopupChannel::new(
            request_id.clone(),
            gui.clone(),
        )));

        let show = ShowPayload {
            request_id: request_id.clone(),
            request,
            source: task.source,
            lang: task.lang,
            project: task.project,
        };

        let entry = Arc::new(RequestEntry {
            request_id: request_id.clone(),
            token: token.clone(),
            coordinator,
            show,
            gui,
            gui_connected: AtomicBool::new(false),
            cancel: Arc::new(Notify::new()),
            final_tx,
        });

        let mut inner = self.inner.lock().unwrap();
        inner.by_id.insert(request_id, entry.clone());
        inner.by_token.insert(token, entry.request_id.clone());
        (entry, final_rx)
    }

    /// GUI Helper 凭 token 关联请求（token 一次性，关联后即注销）。
    pub fn attach_gui(&self, token: &str) -> Option<Arc<RequestEntry>> {
        let mut inner = self.inner.lock().unwrap();
        let request_id = inner.by_token.remove(token)?;
        inner.by_id.get(&request_id).cloned()
    }

    /// 移除请求（结束 / 取消）。
    pub fn remove(&self, request_id: &str) {
        let mut inner = self.inner.lock().unwrap();
        if let Some(entry) = inner.by_id.remove(request_id) {
            inner.by_token.remove(&entry.token);
        }
    }

    /// 活动请求数（供 status）。
    pub fn active_count(&self) -> usize {
        self.inner.lock().unwrap().by_id.len()
    }

    /// Cancel every active request (daemon shutdown): interrupt all their channels as a generic
    /// `Cancelled` (no source) so IM cards finalize and popups close, and wake their GUI handlers.
    /// Returns the number of requests affected, so the caller can decide whether to wait for the
    /// IM finalize HTTP calls to land before the runtime exits.
    pub fn cancel_all_requests(&self) -> usize {
        let inner = self.inner.lock().unwrap();
        for entry in inner.by_id.values() {
            entry.coordinator.cancel_request(String::new());
            entry.cancel.notify_waiters();
        }
        inner.by_id.len()
    }

    /// 向所有已连上的活动 GUI Helper 广播一条消息（如 `ConfigChanged` 实时切主题/语言，A12）。
    pub fn broadcast_to_guis(&self, msg: ServerMsg) {
        let inner = self.inner.lock().unwrap();
        for entry in inner.by_id.values() {
            if let Ok(slot) = entry.gui.lock() {
                if let Some(tx) = slot.as_ref() {
                    let _ = tx.send(msg.clone());
                }
            }
        }
    }
}

/// 看门狗等待时长：GUI Helper 在此时长内未连上即判定弹窗拉起失败。
pub const GUI_CONNECT_TIMEOUT_SECS: u64 = 10;

/// 弹窗拉起失败时给 CLI 的退出码（无可用 Channel）。
pub const EXIT_NO_CHANNEL: i32 = crate::app::EXIT_NO_CHANNEL;

/// 构造「弹窗拉起失败」的渲染结果（→ CLI stderr + 退出码 3）。
pub fn popup_failed_outcome(lang: Lang) -> RenderOutcome {
    RenderOutcome {
        stdout: String::new(),
        stderr: Some(format!(
            "{}{}",
            crate::i18n::err_prefix(lang),
            "GUI popup failed to start",
        )),
        exit_code: EXIT_NO_CHANNEL,
    }
}

/// 用于 ServerMsg::Show 的便捷封装。
pub fn show_msg(entry: &RequestEntry) -> ServerMsg {
    ServerMsg::Show(entry.show.clone())
}
