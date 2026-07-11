//! 请求登记表：每个活动 `request_id` 一套 Coordinator + Channel 任务 + GUI 关联。
//!
//! Phase 1 仅挂 popup adapter：提交时建 Coordinator（Ipc 退出）、注册弹窗 adapter、分配一次性 token；
//! GUI Helper 连上后凭 token 找到该请求、收 `show`、回 `answer`。IM 渠道在 Phase 2 接入。

use crate::app::coordinator::Coordinator;
use crate::app::RenderOutcome;
use crate::channels::popup::GuiHelperPopupChannel;
use crate::i18n::Lang;
use crate::ipc::{PendingRequestInfo, ServerMsg, ShowPayload, TaskRequest};
use crate::models::AskRequest;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use tokio::sync::Notify;

/// GUI 连接的发送端槽位：Helper 连上后填入，供 popup adapter 下发 `cancel`。
pub type GuiSlot = Arc<Mutex<Option<UnboundedSender<ServerMsg>>>>;

/// 异步解析出的调用方 agent 信息（方案5/b）：daemon 从 `caller_pid` 向上 walk 进程树后填入，
/// 经 `ServerMsg::AgentResolved` 后推弹窗 badge；helper 连接时若已就绪也会随握手补发（覆盖竞态）。
#[derive(Clone, Default)]
pub struct ResolvedAgent {
    pub kind: Option<String>,
    pub pid: Option<u32>,
}

/// 一个活动请求的共享状态。
pub struct RequestEntry {
    pub request_id: String,
    /// 单调递增序号（创建顺序）：托盘「待答」子菜单据此稳定按时间排序，不随 HashMap 迭代乱序。
    pub seq: u64,
    /// 一次性 token：GUI Helper 连回握手出示。
    pub token: String,
    /// 抢答协调器（Ipc 退出，结果经 `final_tx` 回传）。
    pub coordinator: Arc<Coordinator>,
    /// 给 GUI Helper 的题目下发负载。
    pub show: ShowPayload,
    /// 调用方 agent 会话 ID（CLI 从 env 探测、`TaskRequest.agent_session_id` 透传；MCP `env_clear`
    /// 时为 None）。供 daemon「在途 AskHuman 豁免」按 session_id 刷新——覆盖无 pid 的 agent
    /// （Codex 共享 app-server / Claude 被 scrub），使其等待人类回答期间不被「工作中兜底超时」降级。
    pub agent_session_id: Option<String>,
    /// GUI 发送端槽位（adapter 与连接处理器共享）。
    pub gui: GuiSlot,
    /// 调用方 agent 异步解析结果（方案5/b）：daemon walk 完成后填入，helper 连接握手时若已就绪则补发。
    pub resolved_agent: Arc<Mutex<Option<ResolvedAgent>>>,
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
    /// 下一个请求序号（创建顺序，单调递增）。
    next_seq: AtomicU64,
}

impl RequestRegistry {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            inner: Mutex::new(Inner::default()),
            next_seq: AtomicU64::new(0),
        })
    }

    /// 建立一个请求：分配 request_id / token，建 Coordinator（Ipc）+ popup adapter。
    /// 返回登记项与「渲染结果接收端」（连接处理器据此写 IPC `final`）。
    pub fn create(
        &self,
        task: TaskRequest,
    ) -> (Arc<RequestEntry>, UnboundedReceiver<RenderOutcome>) {
        let request_id = uuid::Uuid::new_v4().to_string();
        let token = uuid::Uuid::new_v4().to_string();
        let lang = Lang::resolve(&task.lang);
        // 在 task 被逐字段 move 前取出会话 ID（供在途豁免按 session_id 刷新）。
        let agent_session_id = task
            .agent_session_id
            .clone()
            .filter(|s| !s.trim().is_empty());

        // Daemon 分配权威 request_id（用于临时目录）。
        let mut request = AskRequest::new(task.message, task.questions, task.is_markdown);
        request.id = request_id.clone();
        request.select_only = task.select_only;
        request.single = task.single;
        request.output_format = task.output_format;

        let (final_tx, final_rx) = tokio::sync::mpsc::unbounded_channel();
        let coordinator = Coordinator::new_ipc(
            request.clone(),
            lang,
            final_tx.clone(),
            task.project.clone(),
            task.source.clone(),
            task.agent_kind.clone(),
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
            agent_kind: task.agent_kind,
            agent_pid: task.agent_pid,
            // 方案6：透传 perf 上下文，热 helper 领用时据此开启埋点（无 env 也能量化热路径）。
            perf_id: task.perf_id,
            perf_autodismiss: task.perf_autodismiss,
            // 提问创建时刻：弹窗相对时间的锚点（预热弹窗领用时即为真正到达时刻）。
            created_at_ms: crate::perf::now_ms() as u64,
        };

        let entry = Arc::new(RequestEntry {
            request_id: request_id.clone(),
            seq: self.next_seq.fetch_add(1, Ordering::SeqCst),
            token: token.clone(),
            coordinator,
            show,
            agent_session_id,
            gui,
            resolved_agent: Arc::new(Mutex::new(None)),
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

    /// 当前所有在途请求项的快照（供「补推在途」把已发问题补发到新激活的渠道）。
    pub fn in_flight_entries(&self) -> Vec<Arc<RequestEntry>> {
        self.inner.lock().unwrap().by_id.values().cloned().collect()
    }

    /// 所有在途请求关联的调用方 agent pid（去重）。供 daemon「工作中兜底超时」豁免——
    /// 凡有在途 AskHuman 请求的 agent（正等待人类回答）都持续刷新活动、不被降级为空闲。
    /// 优先用异步解析出的 `resolved_agent.pid`（最准、当次现取），否则回退 `show.agent_pid`。
    pub fn in_flight_agent_pids(&self) -> Vec<u32> {
        let inner = self.inner.lock().unwrap();
        let mut pids: Vec<u32> = Vec::new();
        for entry in inner.by_id.values() {
            let pid = entry
                .resolved_agent
                .lock()
                .ok()
                .and_then(|g| g.as_ref().and_then(|a| a.pid))
                .or(entry.show.agent_pid);
            if let Some(pid) = pid {
                if pid != 0 && !pids.contains(&pid) {
                    pids.push(pid);
                }
            }
        }
        pids
    }

    /// 所有在途请求关联的调用方 agent 会话 ID（去重）。供 daemon「工作中兜底超时」豁免的
    /// **session_id 版**——覆盖无 pid 的 agent（Codex 共享 app-server / Claude 被 scrub），使其
    /// 等待人类回答期间不被降级为空闲（有 pid 的由 `in_flight_agent_pids` 覆盖）。
    pub fn in_flight_agent_session_ids(&self) -> Vec<String> {
        let inner = self.inner.lock().unwrap();
        let mut ids: Vec<String> = Vec::new();
        for entry in inner.by_id.values() {
            if let Some(sid) = entry.agent_session_id.as_ref() {
                if !sid.is_empty() && !ids.contains(sid) {
                    ids.push(sid.clone());
                }
            }
        }
        ids
    }

    /// 在途请求摘要（按创建顺序，托盘「待答」子菜单用）：每条 `{id, 预览}`。
    pub fn pending_infos(&self) -> Vec<PendingRequestInfo> {
        let mut entries: Vec<Arc<RequestEntry>> = {
            let inner = self.inner.lock().unwrap();
            inner.by_id.values().cloned().collect()
        };
        entries.sort_by_key(|e| e.seq);
        entries
            .iter()
            .map(|e| PendingRequestInfo {
                id: e.request_id.clone(),
                preview: preview_of(&e.show.request),
            })
            .collect()
    }

    /// 聚焦某请求的弹窗：向其 GUI 连接下发 `FocusPopup`。返回是否成功投递（无弹窗连接则 false）。
    pub fn focus_popup(&self, request_id: &str) -> bool {
        let inner = self.inner.lock().unwrap();
        let Some(entry) = inner.by_id.get(request_id) else {
            return false;
        };
        let Ok(slot) = entry.gui.lock() else {
            return false;
        };
        match slot.as_ref() {
            Some(tx) => tx
                .send(ServerMsg::FocusPopup {
                    request_id: request_id.to_string(),
                })
                .is_ok(),
            None => false,
        }
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

/// 托盘「待答」子菜单预览：取 Message 首个非空行，空则取第一题题干，截断到 24 个字符（超出加 …）。
const PREVIEW_MAX_CHARS: usize = 24;

pub fn preview_of(req: &AskRequest) -> String {
    let source = first_nonempty_line(&req.message.text)
        .or_else(|| {
            req.questions
                .first()
                .and_then(|q| first_nonempty_line(&q.message))
        })
        .unwrap_or_default();
    truncate_chars(&source, PREVIEW_MAX_CHARS)
}

fn first_nonempty_line(s: &str) -> Option<String> {
    s.lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .map(|l| l.to_string())
}

/// 按 Unicode 字符（而非字节）截断，超出追加省略号。
fn truncate_chars(s: &str, max: usize) -> String {
    let mut chars = s.chars();
    let head: String = chars.by_ref().take(max).collect();
    if chars.next().is_some() {
        format!("{head}…")
    } else {
        head
    }
}
