//! 飞书长连接 Router：进程内独占一条 `FeishuWs`，把事件按 `open_message_id`（卡片回调）/
//! `open_id`（聊天消息）分发到对应会话。
//!
//! 设计与钉钉 `dingtalk::router` 同构：Reader 任务**只**阻塞在 `ws.recv()`，不放进会被取消的
//! `select`（避免丢消息）；卡片回调遵循 spec §11：收到即**空 ACK**（满足 3 秒），卡片置灰由会话
//! 经 OpenAPI `patch_card` 完成（提交 toast 因此从略，与「被抢答收尾」走同一条路）。
//!
//! 单进程与 Daemon 复用：Daemon 持共享且常热的 Router；单进程每进程起一个仅挂 1 个会话的同款 Router。

use super::client::FeishuClient;
use super::ws::{FeishuWs, WsEvent};
use crate::config::FeishuChannelConfig;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};
use tokio::sync::oneshot;

/// 提交回调等待会话裁决的上限：会话认出提交后会**立刻**经 oneshot 回包；此上限仅兜底
/// 「会话恰好在忙/已退出」的极少数情况；务必 < 飞书 3 秒回包窗口。
const CARD_ACK_TIMEOUT: Duration = Duration::from_millis(2500);

/// 分发给某个会话的入站事件。
pub enum FsInbound {
    /// 卡片回调：会话裁决后经 `ack` 回包——`Some(body)` 同步更新卡片（按钮 Loading 直接变终态）、
    /// `None` 回空 ACK（非本卡片/未作答）。由 Router 写回连接（满足飞书 3 秒约束）。
    Card {
        data: Value,
        ack: oneshot::Sender<Option<Value>>,
    },
    /// 聊天消息（图片/文件/文字；已被底层 `FeishuWs` 自动 ACK）。
    Message(Value),
}

#[derive(Default)]
struct Routes {
    /// open_message_id → route_id（卡片精确路由）。
    cards: HashMap<String, u64>,
    /// open_id → route_id（聊天消息按「最新活动」归属，见 Q4）。
    loose: HashMap<String, u64>,
    /// route_id → 会话入站事件发送端。
    sinks: HashMap<u64, UnboundedSender<FsInbound>>,
    /// 原始消息观察者（供「自动识别 open_id」等无法预知 open_id 的场景）。
    observers: Vec<UnboundedSender<Value>>,
}

/// 进程内飞书 Router（`Arc` 共享）。
pub struct FsRouter {
    app_id: String,
    routes: Arc<Mutex<Routes>>,
    next_route: AtomicU64,
    alive: Arc<AtomicBool>,
    /// Reader 任务句柄；`Arc` 被全部丢弃（如临时识别连接）时 abort，及时关闭底层连接。
    task: Mutex<Option<tokio::task::JoinHandle<()>>>,
}

impl FsRouter {
    /// 建连并启动 Reader 任务。失败返回英文错误（调用方按界面语言警告并跳过该渠道）。
    /// 仅需 app_id/secret/base_url（不需 open_id，便于「自动识别」复用）。
    pub async fn connect(config: &FeishuChannelConfig) -> Result<Arc<Self>, String> {
        let client = FeishuClient::new(config).map_err(|e| e.to_string())?;
        let app_id = client.app_id().to_string();
        let ws = FeishuWs::connect(
            client.http().clone(),
            client.base_url(),
            client.app_id(),
            client.app_secret(),
        )
        .await
        .map_err(|e| e.to_string())?;
        let routes = Arc::new(Mutex::new(Routes::default()));
        let alive = Arc::new(AtomicBool::new(true));
        let task = tokio::spawn(reader_task(ws, routes.clone(), alive.clone()));
        Ok(Arc::new(Self {
            app_id,
            routes,
            next_route: AtomicU64::new(1),
            alive,
            task: Mutex::new(Some(task)),
        }))
    }

    /// 本 Router 绑定的 app_id（用作「自动识别」是否复用现有连接的匹配键）。
    pub fn app_id(&self) -> &str {
        &self.app_id
    }

    /// 连接是否仍然存活（Reader 任务未退出）。
    pub fn is_alive(&self) -> bool {
        self.alive.load(Ordering::SeqCst)
    }

    /// 为一个会话登记一条路由，返回其句柄。
    pub fn register(self: &Arc<Self>) -> RoutedFs {
        let route_id = self.next_route.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = unbounded_channel();
        self.routes.lock().unwrap().sinks.insert(route_id, tx);
        RoutedFs {
            route_id,
            routes: self.routes.clone(),
            rx,
        }
    }

    /// 登记一个原始消息观察者（用于「自动识别 open_id」：此时 open_id 未知，需看全部消息）。
    pub fn observe_message(&self) -> UnboundedReceiver<Value> {
        let (tx, rx) = unbounded_channel();
        self.routes.lock().unwrap().observers.push(tx);
        rx
    }
}

impl Drop for FsRouter {
    fn drop(&mut self) {
        if let Some(h) = self.task.lock().unwrap().take() {
            h.abort();
        }
    }
}

/// 一个会话的事件源句柄：经它收事件、登记/注销路由。
pub struct RoutedFs {
    route_id: u64,
    routes: Arc<Mutex<Routes>>,
    rx: UnboundedReceiver<FsInbound>,
}

impl RoutedFs {
    /// 标记本会话「当前活动」：登记卡片精确路由（如有 `message_id`）并认领该 open_id 的聊天消息。
    pub fn set_active(&self, message_id: Option<&str>, open_id: &str) {
        let mut r = self.routes.lock().unwrap();
        if let Some(mid) = message_id {
            r.cards.insert(mid.to_string(), self.route_id);
        }
        if !open_id.is_empty() {
            r.loose.insert(open_id.to_string(), self.route_id);
        }
    }

    /// 取消本会话的活动登记（仅当当前归属仍是自己时才清除）。
    pub fn clear_active(&self, message_id: Option<&str>, open_id: &str) {
        let mut r = self.routes.lock().unwrap();
        if let Some(mid) = message_id {
            if r.cards.get(mid) == Some(&self.route_id) {
                r.cards.remove(mid);
            }
        }
        if !open_id.is_empty() && r.loose.get(open_id) == Some(&self.route_id) {
            r.loose.remove(open_id);
        }
    }

    /// Stop routing loose user messages while retaining the exact card callback route.
    pub fn clear_loose(&self, open_id: &str) {
        let mut routes = self.routes.lock().unwrap();
        if !open_id.is_empty() && routes.loose.get(open_id) == Some(&self.route_id) {
            routes.loose.remove(open_id);
        }
    }

    /// 收下一个分发给本会话的事件；`None` 表示连接关闭。
    pub async fn recv(&mut self) -> Option<FsInbound> {
        self.rx.recv().await
    }
}

impl Drop for RoutedFs {
    fn drop(&mut self) {
        let mut r = self.routes.lock().unwrap();
        r.sinks.remove(&self.route_id);
        r.cards.retain(|_, v| *v != self.route_id);
        r.loose.retain(|_, v| *v != self.route_id);
    }
}

/// Reader 任务：独占 `FeishuWs`，循环收事件并按路由表分发。
async fn reader_task(mut ws: FeishuWs, routes: Arc<Mutex<Routes>>, alive: Arc<AtomicBool>) {
    while let Some(ev) = ws.recv().await {
        match ev {
            WsEvent::CardAction { data, frame } => {
                let mid = data
                    .get("context")
                    .and_then(|c| c.get("open_message_id"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let sink = if mid.is_empty() {
                    None
                } else {
                    let r = routes.lock().unwrap();
                    r.cards.get(mid).and_then(|rid| r.sinks.get(rid).cloned())
                };
                // 转发给会话并带 oneshot 回执；超时等其裁决后回包（满足 3 秒）。会话回 Some(body)
                // → 同步更新卡片（按钮丝滑变终态）；None / 孤儿 / 超时 → 空 ACK。
                let resp = match sink {
                    Some(tx) => {
                        let (ack_tx, ack_rx) = oneshot::channel();
                        if tx.send(FsInbound::Card { data, ack: ack_tx }).is_ok() {
                            match tokio::time::timeout(CARD_ACK_TIMEOUT, ack_rx).await {
                                Ok(Ok(body)) => body,
                                _ => None,
                            }
                        } else {
                            None
                        }
                    }
                    None => None,
                };
                match resp {
                    Some(body) => ws.respond_card(&frame, &body).await,
                    None => ws.respond_ack(&frame).await,
                }
            }
            WsEvent::Message(event) => {
                dispatch_observers(&routes, &event);
                let oid = event
                    .get("sender")
                    .and_then(|s| s.get("sender_id"))
                    .and_then(|i| i.get("open_id"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                if oid.is_empty() {
                    continue;
                }
                let sink = {
                    let r = routes.lock().unwrap();
                    r.loose.get(oid).and_then(|rid| r.sinks.get(rid).cloned())
                };
                if let Some(tx) = sink {
                    let _ = tx.send(FsInbound::Message(event));
                }
            }
        }
    }
    // 连接彻底断开：标记不可用并清空 sinks → 各会话 recv() 得到 None 而结束。
    alive.store(false, Ordering::SeqCst);
    routes.lock().unwrap().sinks.clear();
}

/// 向所有存活的消息观察者广播一条原始事件（顺带清理已失效的发送端）。
fn dispatch_observers(routes: &Arc<Mutex<Routes>>, event: &Value) {
    let mut r = routes.lock().unwrap();
    if r.observers.is_empty() {
        return;
    }
    r.observers.retain(|tx| tx.send(event.clone()).is_ok());
}
