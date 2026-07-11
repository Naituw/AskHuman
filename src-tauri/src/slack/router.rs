//! Slack Socket Mode Router：进程内独占一条 `SlackWs`，把事件按
//! `message_ts`（交互回调）/ `user_id`（聊天消息）分发到对应会话。
//!
//! 设计与飞书 `feishu::router` 同构，但更简单：Slack 的 ack（回 `envelope_id`）在 `ws` 层收帧即完成
//! （与卡片更新解耦），故这里**无需** oneshot 延迟回包；Router 只做纯分发。
//!
//! 单进程与 Daemon 复用：Daemon 持共享且常热的 Router；单进程每进程起一个仅挂 1 个会话的同款 Router。

use super::client::SlackClient;
use super::ws::{SlackWs, WsEvent};
use crate::config::SlackChannelConfig;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};

/// 分发给某个会话的入站事件（皆已在 `ws` 层 ack）。
pub enum SlInbound {
    /// 交互回调（`block_actions` payload，含 `state.values`）。
    Interactive(Value),
    /// 聊天消息事件（图片/文件/文字）。
    Message(Value),
}

#[derive(Default)]
struct Routes {
    /// message_ts → route_id（卡片交互精确路由）。
    cards: HashMap<String, u64>,
    /// user_id → route_id（聊天消息按「最新活动」归属）。
    loose: HashMap<String, u64>,
    /// route_id → 会话入站事件发送端。
    sinks: HashMap<u64, UnboundedSender<SlInbound>>,
    /// 原始消息观察者（供「自动识别 user_id」等无法预知 user_id 的场景）。
    observers: Vec<UnboundedSender<Value>>,
}

/// 进程内 Slack Router（`Arc` 共享）。
pub struct SlRouter {
    /// 绑定的 App Token：用作「自动识别」是否复用现有连接的匹配键（Socket 由 app_token 建连）。
    app_token: String,
    routes: Arc<Mutex<Routes>>,
    next_route: AtomicU64,
    alive: Arc<AtomicBool>,
    /// Reader 任务句柄；`Arc` 全部丢弃时 abort，及时关闭底层连接。
    task: Mutex<Option<tokio::task::JoinHandle<()>>>,
}

impl SlRouter {
    /// 建连并启动 Reader 任务。失败返回英文错误（调用方按界面语言警告并跳过该渠道）。
    /// 仅需 bot/app token（不需 user_id，便于「自动识别」复用）。
    pub async fn connect(config: &SlackChannelConfig) -> Result<Arc<Self>, String> {
        let client = SlackClient::new(config).map_err(|e| e.to_string())?;
        let app_token = client.app_token().to_string();
        let ws = SlackWs::connect(client.http().clone(), client.app_token())
            .await
            .map_err(|e| e.to_string())?;
        let routes = Arc::new(Mutex::new(Routes::default()));
        let alive = Arc::new(AtomicBool::new(true));
        let task = tokio::spawn(reader_task(ws, routes.clone(), alive.clone()));
        Ok(Arc::new(Self {
            app_token,
            routes,
            next_route: AtomicU64::new(1),
            alive,
            task: Mutex::new(Some(task)),
        }))
    }

    /// 本 Router 绑定的 App Token（「自动识别」是否复用现有连接的匹配键）。
    pub fn app_token(&self) -> &str {
        &self.app_token
    }

    /// 连接是否仍然存活（Reader 任务未退出）。
    pub fn is_alive(&self) -> bool {
        self.alive.load(Ordering::SeqCst)
    }

    /// 为一个会话登记一条路由，返回其句柄。
    pub fn register(self: &Arc<Self>) -> RoutedSl {
        let route_id = self.next_route.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = unbounded_channel();
        self.routes.lock().unwrap().sinks.insert(route_id, tx);
        RoutedSl {
            route_id,
            routes: self.routes.clone(),
            rx,
        }
    }

    /// 登记一个原始消息观察者（用于「自动识别 user_id」：此时 user_id 未知，需看全部消息）。
    pub fn observe_message(&self) -> UnboundedReceiver<Value> {
        let (tx, rx) = unbounded_channel();
        self.routes.lock().unwrap().observers.push(tx);
        rx
    }
}

impl Drop for SlRouter {
    fn drop(&mut self) {
        if let Some(h) = self.task.lock().unwrap().take() {
            h.abort();
        }
    }
}

/// 一个会话的事件源句柄：经它收事件、登记/注销路由。
pub struct RoutedSl {
    route_id: u64,
    routes: Arc<Mutex<Routes>>,
    rx: UnboundedReceiver<SlInbound>,
}

impl RoutedSl {
    /// 标记本会话「当前活动」：登记卡片精确路由（如有 `message_ts`）并认领该 user_id 的聊天消息。
    pub fn set_active(&self, message_ts: Option<&str>, user_id: &str) {
        let mut r = self.routes.lock().unwrap();
        if let Some(ts) = message_ts {
            if !ts.is_empty() {
                r.cards.insert(ts.to_string(), self.route_id);
            }
        }
        if !user_id.is_empty() {
            r.loose.insert(user_id.to_string(), self.route_id);
        }
    }

    /// 取消本会话的活动登记（仅当当前归属仍是自己时才清除）。
    pub fn clear_active(&self, message_ts: Option<&str>, user_id: &str) {
        let mut r = self.routes.lock().unwrap();
        if let Some(ts) = message_ts {
            if r.cards.get(ts) == Some(&self.route_id) {
                r.cards.remove(ts);
            }
        }
        if !user_id.is_empty() && r.loose.get(user_id) == Some(&self.route_id) {
            r.loose.remove(user_id);
        }
    }

    /// Stop routing loose user messages while retaining the exact card callback route.
    pub fn clear_loose(&self, user_id: &str) {
        let mut routes = self.routes.lock().unwrap();
        if !user_id.is_empty() && routes.loose.get(user_id) == Some(&self.route_id) {
            routes.loose.remove(user_id);
        }
    }

    /// 收下一个分发给本会话的事件；`None` 表示连接关闭。
    pub async fn recv(&mut self) -> Option<SlInbound> {
        self.rx.recv().await
    }
}

impl Drop for RoutedSl {
    fn drop(&mut self) {
        let mut r = self.routes.lock().unwrap();
        r.sinks.remove(&self.route_id);
        r.cards.retain(|_, v| *v != self.route_id);
        r.loose.retain(|_, v| *v != self.route_id);
    }
}

/// Reader 任务：独占 `SlackWs`，循环收事件并按路由表分发。
async fn reader_task(mut ws: SlackWs, routes: Arc<Mutex<Routes>>, alive: Arc<AtomicBool>) {
    while let Some(ev) = ws.recv().await {
        match ev {
            WsEvent::Interactive(payload) => {
                let ts = payload
                    .get("container")
                    .and_then(|c| c.get("message_ts"))
                    .and_then(|v| v.as_str())
                    .or_else(|| {
                        payload
                            .get("message")
                            .and_then(|m| m.get("ts"))
                            .and_then(|v| v.as_str())
                    })
                    .unwrap_or("");
                if ts.is_empty() {
                    continue;
                }
                let sink = {
                    let r = routes.lock().unwrap();
                    r.cards.get(ts).and_then(|rid| r.sinks.get(rid).cloned())
                };
                if let Some(tx) = sink {
                    let _ = tx.send(SlInbound::Interactive(payload));
                }
            }
            WsEvent::Message(event) => {
                dispatch_observers(&routes, &event);
                let uid = event.get("user").and_then(|v| v.as_str()).unwrap_or("");
                if uid.is_empty() {
                    continue;
                }
                let sink = {
                    let r = routes.lock().unwrap();
                    r.loose.get(uid).and_then(|rid| r.sinks.get(rid).cloned())
                };
                if let Some(tx) = sink {
                    let _ = tx.send(SlInbound::Message(event));
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
