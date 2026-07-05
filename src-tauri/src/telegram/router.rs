//! Telegram 长轮询 Router：进程内**独占一个 `getUpdates` 轮询器 + 单一 offset**，把更新按
//! 卡片 `message_id`（callback_query）/「最新活动卡片」（自由文字，见 Q4）分发到对应会话。
//!
//! 这正是 TODO#1 在 Telegram 上的根因修复：旧实现每个会话各自 `getUpdates`、各持 offset，
//! 并发/连续提问时互相吞更新。现在全进程只有 Router 的 Reader 任务在轮询。
//!
//! 与钉钉/飞书 Router 同构，但 Telegram 无「3 秒强制 ACK」：callback 由会话自行
//! `answerCallbackQuery`（仅为消除客户端转圈）；匹配不到的孤儿 callback 由 Reader 兜底应答。
//!
//! 单进程与 Daemon 复用：Daemon 持共享且常热的 Router；单进程每进程起一个仅挂 1 个会话的同款 Router。

use super::TelegramClient;
use crate::config::TelegramChannelConfig;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};

/// 长轮询挂起秒数（须 < TelegramClient 的 HTTP 超时 30s）。
const LONGPOLL_SECS: u64 = 25;

/// 分发给某个会话的入站事件。
pub enum TgInbound {
    /// 卡片回调（`callback_query` 对象；会话据此切换选项/提交，并自行应答）。
    Callback(Value),
    /// 卡片之后用户发的文字（归属「最新活动卡片」的请求，见 Q4）。
    Text { text: String, message_id: i64 },
}

/// 一条活动会话的归属信息（用于「最新活动卡片」文字路由）。
struct ActiveInfo {
    chat_id: i64,
    /// 活动序号：越大越「新」；自由文字归给同 chat 下序号最大的活动会话。
    seq: u64,
}

#[derive(Default)]
struct Routes {
    /// card_message_id → route_id（callback 精确路由）。
    cards: HashMap<i64, u64>,
    /// route_id → 活动信息（自由文字按「最新活动」归属）。
    active: HashMap<u64, ActiveInfo>,
    /// route_id → 会话入站事件发送端。
    sinks: HashMap<u64, UnboundedSender<TgInbound>>,
    /// 原始文字消息观察者（供 daemon 级入站监听，如「IM 会话期自动激活」收 /here）。
    observers: Vec<UnboundedSender<Value>>,
}

/// 进程内 Telegram Router（`Arc` 共享）。
pub struct TgRouter {
    routes: Arc<Mutex<Routes>>,
    next_route: AtomicU64,
    /// 活动序号源（跨会话单调递增，决定「最新活动卡片」）。
    next_seq: Arc<AtomicU64>,
    alive: Arc<AtomicBool>,
    /// 长轮询任务句柄；`Arc` 被全部丢弃（如单进程会话结束）时 abort，停止轮询。
    task: Mutex<Option<tokio::task::JoinHandle<()>>>,
}

impl TgRouter {
    /// 校验配置并启动单一长轮询 Reader 任务。失败返回英文错误（调用方按界面语言警告并跳过）。
    pub async fn connect(config: &TelegramChannelConfig) -> Result<Arc<Self>, String> {
        let client = TelegramClient::new(
            config.bot_token.clone(),
            config.chat_id.clone(),
            config.api_base_url.clone(),
        )
        .map_err(|e| e.to_string())?;
        let routes = Arc::new(Mutex::new(Routes::default()));
        let task = tokio::spawn(reader_task(client, routes.clone()));
        Ok(Arc::new(Self {
            routes,
            next_route: AtomicU64::new(1),
            next_seq: Arc::new(AtomicU64::new(1)),
            // Telegram 长轮询会自愈瞬时错误、永不「正常结束」，故恒为存活（无需重连）。
            alive: Arc::new(AtomicBool::new(true)),
            task: Mutex::new(Some(task)),
        }))
    }

    /// 轮询器是否仍在运行（Telegram 长轮询会自愈瞬时错误，故一旦建连通常恒为 true）。
    pub fn is_alive(&self) -> bool {
        self.alive.load(Ordering::SeqCst)
    }

    /// 为一个会话登记一条路由，返回其句柄。
    pub fn register(self: &Arc<Self>) -> RoutedTg {
        let route_id = self.next_route.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = unbounded_channel();
        self.routes.lock().unwrap().sinks.insert(route_id, tx);
        RoutedTg {
            route_id,
            routes: self.routes.clone(),
            seq: self.next_seq.clone(),
            rx,
        }
    }

    /// 登记一个原始文字消息观察者（用于 daemon 级入站监听）。返回接收端；丢弃即自动注销。
    /// 每条观察到的事件是本 chat 收到的 `message` 对象（含 `text` / `chat`）。
    pub fn observe_message(&self) -> UnboundedReceiver<Value> {
        let (tx, rx) = unbounded_channel();
        self.routes.lock().unwrap().observers.push(tx);
        rx
    }
}

impl Drop for TgRouter {
    fn drop(&mut self) {
        if let Some(h) = self.task.lock().unwrap().take() {
            h.abort();
        }
    }
}

/// 一个会话的事件源句柄：经它收事件、登记/注销路由。
pub struct RoutedTg {
    route_id: u64,
    routes: Arc<Mutex<Routes>>,
    seq: Arc<AtomicU64>,
    rx: UnboundedReceiver<TgInbound>,
}

impl RoutedTg {
    /// 标记本会话「当前活动」：登记卡片精确路由，并把本会话置为该 chat 的「最新活动卡片」。
    /// 每次调用都会刷新活动序号 → 后发卡片的请求接管该 chat 的自由文字（符合 Q4 直觉）。
    pub fn set_active(&self, chat_id: i64, card_message_id: i64) {
        let s = self.seq.fetch_add(1, Ordering::SeqCst);
        let mut r = self.routes.lock().unwrap();
        r.cards.insert(card_message_id, self.route_id);
        r.active
            .insert(self.route_id, ActiveInfo { chat_id, seq: s });
    }

    /// 仅登记卡片精确路由（callback），**不**认领该 chat 的自由文字。
    /// 供 watch 卡使用：watch 卡只收按钮回调，不得抢走提问卡的文字答案。
    pub fn set_card_route(&self, card_message_id: i64) {
        self.routes
            .lock()
            .unwrap()
            .cards
            .insert(card_message_id, self.route_id);
    }

    /// 注销一条卡片精确路由（仅当归属仍是自己时）。
    pub fn clear_card_route(&self, card_message_id: i64) {
        let mut r = self.routes.lock().unwrap();
        if r.cards.get(&card_message_id) == Some(&self.route_id) {
            r.cards.remove(&card_message_id);
        }
    }

    /// 取消本会话的活动登记（仅当卡片归属仍是自己时才清除该卡片路由）。
    pub fn clear_active(&self, card_message_id: i64) {
        let mut r = self.routes.lock().unwrap();
        if r.cards.get(&card_message_id) == Some(&self.route_id) {
            r.cards.remove(&card_message_id);
        }
        r.active.remove(&self.route_id);
    }

    /// 收下一个分发给本会话的事件；`None` 表示轮询器已停止。
    pub async fn recv(&mut self) -> Option<TgInbound> {
        self.rx.recv().await
    }
}

impl Drop for RoutedTg {
    fn drop(&mut self) {
        let mut r = self.routes.lock().unwrap();
        r.sinks.remove(&self.route_id);
        r.cards.retain(|_, v| *v != self.route_id);
        r.active.remove(&self.route_id);
    }
}

/// Reader 任务：独占长轮询，单一 offset，循环收更新并按路由表分发。瞬时错误退避重试（自愈）。
async fn reader_task(client: TelegramClient, routes: Arc<Mutex<Routes>>) {
    let mut offset: i64 = 0;
    loop {
        let updates = match client.get_updates(offset, LONGPOLL_SECS).await {
            Ok(u) => u,
            Err(_) => {
                tokio::time::sleep(Duration::from_secs(5)).await;
                continue;
            }
        };
        for update in updates {
            if let Some(uid) = update.get("update_id").and_then(|v| v.as_i64()) {
                offset = uid + 1;
            }
            dispatch(&client, &routes, update).await;
        }
    }
}

/// 把一条 update 分发到对应会话；匹配不到的孤儿 callback 由本任务兜底应答（消除转圈）。
async fn dispatch(client: &TelegramClient, routes: &Arc<Mutex<Routes>>, update: Value) {
    let our_chat = client.chat_id();

    if let Some(cb) = update.get("callback_query") {
        let chat = cb
            .get("message")
            .and_then(|m| m.get("chat"))
            .and_then(|c| c.get("id"))
            .and_then(|v| v.as_i64());
        let mid = cb
            .get("message")
            .and_then(|m| m.get("message_id"))
            .and_then(|v| v.as_i64());
        // 非本 chat：应答消除转圈后丢弃。
        if chat != Some(our_chat) {
            if let Some(id) = cb.get("id").and_then(|i| i.as_str()) {
                client.answer_callback_query(id).await;
            }
            return;
        }
        let sink = {
            let r = routes.lock().unwrap();
            mid.and_then(|m| r.cards.get(&m).copied())
                .and_then(|rid| r.sinks.get(&rid).cloned())
        };
        match sink {
            Some(tx) => {
                let _ = tx.send(TgInbound::Callback(cb.clone()));
            }
            // 孤儿 callback（卡片已收尾/无主）：兜底应答，消除客户端转圈。
            None => {
                if let Some(id) = cb.get("id").and_then(|i| i.as_str()) {
                    client.answer_callback_query(id).await;
                }
            }
        }
        return;
    }

    if let Some(message) = update.get("message") {
        let chat = message
            .get("chat")
            .and_then(|c| c.get("id"))
            .and_then(|v| v.as_i64());
        if chat != Some(our_chat) {
            return;
        }
        let mid = message
            .get("message_id")
            .and_then(|v| v.as_i64())
            .unwrap_or(0);
        let text = match message.get("text").and_then(|t| t.as_str()) {
            Some(t) => t.to_string(),
            None => return, // 仅处理文字；图片/文件 Telegram 渠道不收
        };
        // 广播给 daemon 级观察者（如「IM 会话期自动激活」入站监听），与卡片路由并行。
        let observed = dispatch_observers(routes, message);
        // 边界（仅 armed=有观察者时）：Telegram 自由文字既是卡片答案、又会被观察者收到。
        // 命令文字（`/` 前缀一律、`!` 前缀且为已知命令，见 `autochannel::classify`）不应被当成
        // 在途卡片的答案——故仅交观察者，不路由到卡片会话。其余文字仍正常作答
        // （同时被观察者收到，daemon 端按「普通消息」静默处理）。
        if observed && crate::autochannel::classify(&text) != crate::autochannel::Parsed::Text {
            return;
        }
        // 归给该 chat 下「最新活动卡片」的会话（活动序号最大者）。
        let sink = {
            let r = routes.lock().unwrap();
            let best = r
                .active
                .iter()
                .filter(|(_, info)| info.chat_id == our_chat)
                .max_by_key(|(_, info)| info.seq)
                .map(|(rid, _)| *rid);
            best.and_then(|rid| r.sinks.get(&rid).cloned())
        };
        if let Some(tx) = sink {
            let _ = tx.send(TgInbound::Text {
                text,
                message_id: mid,
            });
        }
    }
}

/// 向所有存活的消息观察者广播一条原始 `message` 对象（顺带清理已失效的发送端）。
/// 返回是否存在至少一个存活观察者（即本渠道当前是否处于「IM 自动激活 armed」态）。
fn dispatch_observers(routes: &Arc<Mutex<Routes>>, message: &Value) -> bool {
    let mut r = routes.lock().unwrap();
    if r.observers.is_empty() {
        return false;
    }
    r.observers.retain(|tx| tx.send(message.clone()).is_ok());
    !r.observers.is_empty()
}
