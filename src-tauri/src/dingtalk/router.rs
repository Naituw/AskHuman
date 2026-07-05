//! 钉钉长连接 Router：进程内独占一条 `StreamConn`，把事件按 `outTrackId`（卡片回调）/
//! `senderStaffId`（聊天消息）分发到对应会话。
//!
//! 取消安全：Reader 任务**只**在 `stream.recv()` 上阻塞，绝不把它放进会被取消的 `select`，
//! 以免半路丢用户消息。会话经 mpsc 收事件、经共享路由表登记/注销，不再各自持连接。
//!
//! 卡片回调遵循 spec §11：Reader 收到即**空 ACK**（满足钉钉 3 秒约束），卡片置灰由会话经
//! OpenAPI `updateCard` 完成（与「被抢答收尾」走同一条路）。
//!
//! 单进程与 Daemon 复用同一套：Daemon 持**共享且常热**的 Router（跨请求复用，根治多连接抢消息）；
//! 单进程则每进程起一个只挂 1 个会话的同款 Router。

use super::card;
use super::stream::{StreamConn, StreamEvent, TOPIC_BOT_MESSAGE, TOPIC_CARD_CALLBACK};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};
use tokio::sync::oneshot;

/// 提交回调等待会话裁决的上限：会话认出提交后会**立刻**经 oneshot 回包（微秒级），此上限仅兜底
/// 「会话恰好在忙/已退出」的极少数情况；务必 < 钉钉 3 秒回包窗口。
const SUBMIT_ACK_TIMEOUT: Duration = Duration::from_millis(2500);

/// 分发给某个会话的入站事件。
pub enum DdInbound {
    /// 卡片回调（提问卡「提交」/ watch 卡按钮）：认领方裁决后经 `ack` 回包（成功置灰 / 空包），
    /// 由 Router 写回连接。其余回调（选项切换等）由 Router 直接空 ACK、不转发。
    Card {
        data: Value,
        ack: oneshot::Sender<Value>,
    },
    /// 聊天消息（图片/文件/文字；已被底层 `StreamConn` 自动 ACK）。
    Bot(Value),
}

#[derive(Default)]
struct Routes {
    /// outTrackId → route_id（卡片精确路由）。
    cards: HashMap<String, u64>,
    /// senderStaffId → route_id（聊天消息按「最新活动」归属，见 Q4）。
    loose: HashMap<String, u64>,
    /// route_id → 会话入站事件发送端。
    sinks: HashMap<u64, UnboundedSender<DdInbound>>,
    /// 原始 bot 消息观察者（供「自动识别 userId」等无法预知 user_id 的场景）。
    observers: Vec<UnboundedSender<Value>>,
}

/// 进程内钉钉 Router（`Arc` 共享）。
pub struct DdRouter {
    client_id: String,
    routes: Arc<Mutex<Routes>>,
    next_route: AtomicU64,
    alive: Arc<AtomicBool>,
    /// Reader 任务句柄；`Arc` 被全部丢弃（如临时识别连接）时 abort，及时关闭底层连接。
    task: Mutex<Option<tokio::task::JoinHandle<()>>>,
}

impl DdRouter {
    /// 建连并启动 Reader 任务。失败返回英文错误（调用方按界面语言警告并跳过该渠道）。
    /// 仅需 client_id/secret（不需 user_id，便于「自动识别」复用）。
    pub async fn connect(client_id: &str, client_secret: &str) -> Result<Arc<Self>, String> {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|e| e.to_string())?;
        let stream = StreamConn::connect(
            http,
            client_id,
            client_secret,
            &[TOPIC_BOT_MESSAGE, TOPIC_CARD_CALLBACK],
        )
        .await
        .map_err(|e| e.to_string())?;
        let routes = Arc::new(Mutex::new(Routes::default()));
        let alive = Arc::new(AtomicBool::new(true));
        let task = tokio::spawn(reader_task(stream, routes.clone(), alive.clone()));
        Ok(Arc::new(Self {
            client_id: client_id.to_string(),
            routes,
            next_route: AtomicU64::new(1),
            alive,
            task: Mutex::new(Some(task)),
        }))
    }

    /// 本 Router 绑定的 client_id（用作「自动识别」是否复用现有连接的匹配键）。
    pub fn client_id(&self) -> &str {
        &self.client_id
    }

    /// 连接是否仍然存活（Reader 任务未退出）。
    pub fn is_alive(&self) -> bool {
        self.alive.load(Ordering::SeqCst)
    }

    /// 为一个会话登记一条路由，返回其句柄。
    pub fn register(self: &Arc<Self>) -> RoutedDd {
        let route_id = self.next_route.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = unbounded_channel();
        self.routes.lock().unwrap().sinks.insert(route_id, tx);
        RoutedDd {
            route_id,
            routes: self.routes.clone(),
            rx,
        }
    }

    /// 登记一个原始 bot 消息观察者（用于「自动识别 userId」：此时 user_id 未知，需看全部消息）。
    /// 返回接收端；丢弃即自动注销（下次分发时清理失效发送端）。
    pub fn observe_bot(&self) -> UnboundedReceiver<Value> {
        let (tx, rx) = unbounded_channel();
        self.routes.lock().unwrap().observers.push(tx);
        rx
    }
}

impl Drop for DdRouter {
    fn drop(&mut self) {
        if let Some(h) = self.task.lock().unwrap().take() {
            h.abort();
        }
    }
}

/// 一个会话的事件源句柄：经它收事件、登记/注销路由。
pub struct RoutedDd {
    route_id: u64,
    routes: Arc<Mutex<Routes>>,
    rx: UnboundedReceiver<DdInbound>,
}

impl RoutedDd {
    /// 标记本会话「当前活动」：登记卡片精确路由（如有 `out_track_id`）并认领该 user 的聊天消息。
    pub fn set_active(&self, out_track_id: Option<&str>, user_id: &str) {
        let mut r = self.routes.lock().unwrap();
        if let Some(otid) = out_track_id {
            r.cards.insert(otid.to_string(), self.route_id);
        }
        if !user_id.is_empty() {
            r.loose.insert(user_id.to_string(), self.route_id);
        }
    }

    /// 取消本会话的活动登记（仅当当前归属仍是自己时才清除）。
    pub fn clear_active(&self, out_track_id: Option<&str>, user_id: &str) {
        let mut r = self.routes.lock().unwrap();
        if let Some(otid) = out_track_id {
            if r.cards.get(otid) == Some(&self.route_id) {
                r.cards.remove(otid);
            }
        }
        if !user_id.is_empty() && r.loose.get(user_id) == Some(&self.route_id) {
            r.loose.remove(user_id);
        }
    }

    /// 收下一个分发给本会话的事件；`None` 表示连接关闭。
    pub async fn recv(&mut self) -> Option<DdInbound> {
        self.rx.recv().await
    }
}

impl Drop for RoutedDd {
    fn drop(&mut self) {
        let mut r = self.routes.lock().unwrap();
        r.sinks.remove(&self.route_id);
        r.cards.retain(|_, v| *v != self.route_id);
        r.loose.retain(|_, v| *v != self.route_id);
    }
}

/// Reader 任务：独占 `StreamConn`，循环收事件并按路由表分发。
async fn reader_task(mut stream: StreamConn, routes: Arc<Mutex<Routes>>, alive: Arc<AtomicBool>) {
    while let Some(ev) = stream.recv().await {
        match ev {
            StreamEvent::CardCallback { data, message_id } => {
                // 只转发「提交」（提问卡）与 watch 按钮回调；其余（选项切换等）会话本就忽略
                // → 直接空 ACK、不转发。
                if !card::is_submit(&data) && super::watch::parse_watch_action(&data).is_none() {
                    stream.respond(&message_id, json!({})).await;
                    continue;
                }
                let otid = data
                    .get("outTrackId")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let sink = if otid.is_empty() {
                    None
                } else {
                    let r = routes.lock().unwrap();
                    r.cards.get(otid).and_then(|rid| r.sinks.get(rid).cloned())
                };
                // 提交 / watch 回调：转发给认领方并带 oneshot 回执；超时等其裁决，按裁决回包
                // （满足 3 秒）。孤儿回调（无登记 / 已退出 / 超时）→ 空包：诚实地不显示成功。
                let resp = match sink {
                    Some(tx) => {
                        let (ack_tx, ack_rx) = oneshot::channel();
                        if tx.send(DdInbound::Card { data, ack: ack_tx }).is_ok() {
                            match tokio::time::timeout(SUBMIT_ACK_TIMEOUT, ack_rx).await {
                                Ok(Ok(payload)) => payload,
                                _ => json!({}),
                            }
                        } else {
                            json!({})
                        }
                    }
                    None => json!({}),
                };
                stream.respond(&message_id, resp).await;
            }
            StreamEvent::BotMessage(data) => {
                dispatch_observers(&routes, &data);
                let uid = data
                    .get("senderStaffId")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                if uid.is_empty() {
                    continue;
                }
                let sink = {
                    let r = routes.lock().unwrap();
                    r.loose.get(uid).and_then(|rid| r.sinks.get(rid).cloned())
                };
                if let Some(tx) = sink {
                    let _ = tx.send(DdInbound::Bot(data));
                }
            }
        }
    }
    // 连接彻底断开：标记不可用并清空 sinks → 各会话 recv() 得到 None 而结束。
    alive.store(false, Ordering::SeqCst);
    routes.lock().unwrap().sinks.clear();
}

/// 向所有存活的 bot 观察者广播一条原始消息（顺带清理已失效的发送端）。
fn dispatch_observers(routes: &Arc<Mutex<Routes>>, data: &Value) {
    let mut r = routes.lock().unwrap();
    if r.observers.is_empty() {
        return;
    }
    r.observers.retain(|tx| tx.send(data.clone()).is_ok());
}
