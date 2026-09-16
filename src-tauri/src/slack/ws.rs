//! Slack Socket Mode 长连接（JSON 帧）。
//!
//! 协议：
//! 1. `POST https://slack.com/api/apps.connections.open`（App Token 走 `Authorization` 头）→ `url`（wss）。
//! 2. 连 wss；帧为 **JSON 文本**。业务帧 `events_api`（事件，如 `message`）/ `interactive`（交互，如
//!    `block_actions`），各含 `envelope_id`；另有 `hello`（建连）、`disconnect`（要求重连）控制帧。
//! 3. 每条含 `envelope_id` 的帧须 **3 秒内回 `{"envelope_id": id}`** ack。本实现**收帧即 ack**
//!    （与卡片更新解耦：卡片更新走 Web API `chat.update`，不绑 3 秒窗口），比飞书延迟回包更简单。
//! 4. WS 协议 Ping 回 Pong；`disconnect`/断开 → 重连（重新取 url）。

use super::SlackError;
use futures_util::stream::{SplitSink, SplitStream};
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use std::time::Duration;
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{connect_async, MaybeTlsStream, WebSocketStream};

type Ws = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// Half-open probe cadence: Socket Mode has no application heartbeat of its own, so after
/// `PROBE_INTERVAL` without any inbound frame we send a WebSocket Ping; a second silent tick means
/// the connection is dead (sleep / network handoff) and gets rebuilt.
const PROBE_INTERVAL: Duration = Duration::from_secs(30);

/// 上抛给上层的业务事件（皆已 ack）。
pub enum WsEvent {
    /// 用户消息事件（`events_api` 的 `payload.event`，`type=message`）。
    Message(Value),
    /// 交互负载（`interactive` 的 `payload`，`type=block_actions`）。
    Interactive(Value),
}

pub struct SlackWs {
    http: reqwest::Client,
    app_token: String,
    write: SplitSink<Ws, Message>,
    read: SplitStream<Ws>,
    /// Half-open detection: WebSocket Ping cadence plus "did anything arrive since the last probe".
    probe: tokio::time::Interval,
    awaiting_pong: bool,
}

impl SlackWs {
    /// 建立 Socket Mode 连接：取 wss url → 连 wss → 拆分读写。
    pub async fn connect(http: reqwest::Client, app_token: &str) -> Result<Self, SlackError> {
        let url = open_socket_url(&http, app_token).await?;
        let (ws, _resp) = connect_async(url)
            .await
            .map_err(|e| SlackError::Network(format!("WebSocket connection failed: {}", e)))?;
        let (write, read) = ws.split();
        Ok(Self {
            http,
            app_token: app_token.to_string(),
            write,
            read,
            probe: probe_interval(),
            awaiting_pong: false,
        })
    }

    /// 收下一个业务事件；内部处理 ack、hello/disconnect、ping/pong、半开探测、断线重连。
    ///
    /// Reconnects indefinitely with capped exponential backoff, so this only returns `None` when
    /// the caller aborts the task; in-flight cards stay valid across outages.
    pub async fn recv(&mut self) -> Option<WsEvent> {
        loop {
            let msg = tokio::select! {
                biased;
                msg = self.read.next() => Some(msg),
                _ = self.probe.tick() => None,
            };
            match msg {
                None => {
                    // Probe tick: nothing arrived since the previous probe (not even its Pong) or
                    // the socket refuses writes → dead, rebuild instead of waiting forever.
                    if self.awaiting_pong
                        || self
                            .write
                            .send(Message::Ping(Vec::new().into()))
                            .await
                            .is_err()
                    {
                        debug_log("[slack-ws] connection unresponsive to probes; reconnecting");
                        self.reconnect().await;
                    } else {
                        self.awaiting_pong = true;
                    }
                }
                Some(Some(Ok(Message::Text(t)))) => {
                    self.awaiting_pong = false;
                    if let Some(ev) = self.handle_text(t.as_str()).await {
                        return Some(ev);
                    }
                }
                Some(Some(Ok(Message::Ping(p)))) => {
                    self.awaiting_pong = false;
                    let _ = self.write.send(Message::Pong(p)).await;
                }
                Some(Some(Ok(_))) => {
                    // Binary / Pong / 其它：忽略，但它们都证明连接活着。
                    self.awaiting_pong = false;
                }
                Some(Some(Err(_))) | Some(None) => self.reconnect().await,
            }
        }
    }

    /// 处理一条 JSON 文本帧；业务帧返回事件，控制帧返回 None。含 `envelope_id` 即先 ack。
    async fn handle_text(&mut self, text: &str) -> Option<WsEvent> {
        let v: Value = serde_json::from_str(text).ok()?;
        let frame_type = v.get("type").and_then(|t| t.as_str()).unwrap_or("");
        debug_log(&format!("[slack-ws] frame type={}", frame_type));

        // 控制帧：建连确认 / 要求重连。
        if frame_type == "hello" {
            return None;
        }
        if frame_type == "disconnect" {
            // 服务端要求重连（reason: warning / refresh_requested / too_many_connections）。
            self.reconnect().await;
            return None;
        }

        // 业务帧：收帧即 ack（满足 3 秒，且与卡片更新解耦）。
        if let Some(id) = v.get("envelope_id").and_then(|e| e.as_str()) {
            self.ack(id).await;
        }

        match frame_type {
            "events_api" => {
                let event = v.get("payload").and_then(|p| p.get("event"))?;
                if event.get("type").and_then(|t| t.as_str()) != Some("message") {
                    return None;
                }
                // 跳过机器人自身消息与编辑/删除等噪音子类型（file_share 等保留）。
                if event.get("bot_id").is_some() {
                    return None;
                }
                if let Some(sub) = event.get("subtype").and_then(|s| s.as_str()) {
                    if matches!(
                        sub,
                        "bot_message" | "message_changed" | "message_deleted" | "message_replied"
                    ) {
                        return None;
                    }
                }
                Some(WsEvent::Message(event.clone()))
            }
            "interactive" => {
                let payload = v.get("payload")?;
                if payload.get("type").and_then(|t| t.as_str()) == Some("block_actions") {
                    Some(WsEvent::Interactive(payload.clone()))
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    /// 回 ack：`{"envelope_id": id}`。
    async fn ack(&mut self, envelope_id: &str) {
        let body = json!({ "envelope_id": envelope_id }).to_string();
        let _ = self.write.send(Message::Text(body.into())).await;
    }

    /// 断线重连：重新取 url + 连接，指数退避（0.5 s 起、上限 30 s）直到成功。
    /// 期间在渠道健康表登记「重连中」，成功即清除；首三次及之后每十次记一行日志。
    async fn reconnect(&mut self) {
        use crate::channels::health;
        let mut attempt: u32 = 0;
        loop {
            tokio::time::sleep(health::reconnect_delay(attempt)).await;
            match self.reconnect_once().await {
                Ok(()) => {
                    health::clear("slack");
                    eprintln!(
                        "[slack-ws] reconnected after {} attempt(s)",
                        attempt.saturating_add(1)
                    );
                    return;
                }
                Err(e) => {
                    if health::should_log_reconnect(attempt) {
                        eprintln!(
                            "[slack-ws] reconnect attempt {} failed: {e}; next try in {:?}",
                            attempt.saturating_add(1),
                            health::reconnect_delay(attempt.saturating_add(1))
                        );
                    }
                    health::report("slack", health::reconnecting_message(attempt, &e));
                    attempt = attempt.saturating_add(1);
                }
            }
        }
    }

    async fn reconnect_once(&mut self) -> Result<(), String> {
        let url = open_socket_url(&self.http, &self.app_token)
            .await
            .map_err(|e| e.to_string())?;
        let (ws, _) = connect_async(url)
            .await
            .map_err(|e| format!("WebSocket connection failed: {e}"))?;
        let (write, read) = ws.split();
        self.write = write;
        self.read = read;
        self.probe = probe_interval();
        self.awaiting_pong = false;
        Ok(())
    }
}

fn probe_interval() -> tokio::time::Interval {
    let mut interval =
        tokio::time::interval_at(tokio::time::Instant::now() + PROBE_INTERVAL, PROBE_INTERVAL);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    interval
}

/// 取 Socket Mode wss URL：`apps.connections.open`（App Token 必须放 `Authorization` 头）。
pub async fn open_socket_url(
    http: &reqwest::Client,
    app_token: &str,
) -> Result<String, SlackError> {
    let resp = http
        .post(format!("{}/apps.connections.open", super::api_base()))
        .bearer_auth(app_token)
        .send()
        .await
        .map_err(|e| SlackError::Network(e.to_string()))?;
    let v: Value = resp.json().await.map_err(|_| SlackError::BadResponse)?;
    if v.get("ok").and_then(|o| o.as_bool()) != Some(true) {
        let msg = v
            .get("error")
            .and_then(|m| m.as_str())
            .unwrap_or("failed to open Socket Mode connection")
            .to_string();
        return Err(SlackError::Api(msg));
    }
    v.get("url")
        .and_then(|u| u.as_str())
        .map(|s| s.to_string())
        .ok_or(SlackError::BadResponse)
}

/// 是否开启 Slack 长连接诊断日志（环境变量 `ASKHUMAN_SLACK_DEBUG` 非空且非 "0"）。
pub fn debug_enabled() -> bool {
    std::env::var("ASKHUMAN_SLACK_DEBUG")
        .map(|v| !v.is_empty() && v != "0")
        .unwrap_or(false)
}

/// 诊断日志：写入 `~/.askhuman/slack-debug.log`（GUI 模式 stderr 被静默，文件更可靠）。
pub fn debug_log(msg: &str) {
    if !debug_enabled() {
        return;
    }
    use std::io::Write;
    let dir = crate::paths::config_dir();
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("slack-debug.log");
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    {
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let _ = writeln!(f, "[{}] {}", ts, msg);
    }
    eprintln!("{}", msg);
}
