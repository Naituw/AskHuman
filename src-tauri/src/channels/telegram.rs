//! Telegram Channel：发送提问 + 长轮询接收回复（不接收图片），逐项对齐 Swift 版。

use super::{Channel, ResultSink};
use crate::config::TelegramChannelConfig;
use crate::models::{AskRequest, ChannelAction, ChannelResult};
use crate::telegram::{markdown, TelegramClient};
use serde_json::{json, Value};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

const SEND_BUTTON: &str = "↗️发送";

pub struct TelegramChannel {
    config: TelegramChannelConfig,
    cancelled: Arc<AtomicBool>,
}

impl TelegramChannel {
    pub fn new(config: TelegramChannelConfig) -> Self {
        Self {
            config,
            cancelled: Arc::new(AtomicBool::new(false)),
        }
    }
}

impl Channel for TelegramChannel {
    fn id(&self) -> &str {
        "telegram"
    }

    fn start(&self, request: &AskRequest, sink: ResultSink) {
        let config = self.config.clone();
        let cancelled = self.cancelled.clone();
        let request = request.clone();
        tauri::async_runtime::spawn(async move {
            run_session(request, config, cancelled, sink).await;
        });
    }

    fn cancel_by_other(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
    }
}

async fn run_session(
    request: AskRequest,
    config: TelegramChannelConfig,
    cancelled: Arc<AtomicBool>,
    sink: ResultSink,
) {
    let client = match TelegramClient::new(config.bot_token, config.chat_id, config.api_base_url) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("警告: Telegram 配置无效，已跳过该 Channel: {}", e);
            return;
        }
    };

    let options = request.predefined_options.clone();
    let mut selected: Vec<String> = Vec::new();
    let mut user_input = String::new();

    // 1. 选项消息（MarkdownV2 失败回退纯文本）
    let inline = if options.is_empty() {
        None
    } else {
        Some(inline_keyboard(&options, &selected))
    };
    let options_message_id = if request.is_markdown {
        let processed = markdown::process(&request.message);
        match client
            .send_message(&processed, Some("MarkdownV2"), inline.clone())
            .await
        {
            Ok(id) => id,
            Err(_) => client
                .send_message(&request.message, None, inline.clone())
                .await
                .unwrap_or(0),
        }
    } else {
        client
            .send_message(&request.message, None, inline.clone())
            .await
            .unwrap_or(0)
    };

    // 2. 操作消息（含「发送」按钮）
    let operation_message_id = client
        .send_message(
            "在键盘上点「发送」完成回复，或直接回复文字补充说明",
            None,
            Some(reply_keyboard()),
        )
        .await
        .unwrap_or(0);

    // 3. 长轮询
    let mut offset: i64 = 0;
    while !cancelled.load(Ordering::SeqCst) {
        match client.get_updates(offset).await {
            Ok(updates) => {
                for update in updates {
                    if let Some(uid) = update.get("update_id").and_then(|v| v.as_i64()) {
                        offset = uid + 1;
                    }
                    if handle_update(
                        &update,
                        &client,
                        &options,
                        &mut selected,
                        &mut user_input,
                        options_message_id,
                        operation_message_id,
                    )
                    .await
                    {
                        sink.submit(ChannelResult {
                            action: ChannelAction::Send,
                            selected_options: selected.clone(),
                            user_input: if user_input.is_empty() {
                                None
                            } else {
                                Some(user_input.clone())
                            },
                            images: Vec::new(),
                            source_channel_id: "telegram".to_string(),
                        });
                        return;
                    }
                }
            }
            Err(_) => {
                tokio::time::sleep(Duration::from_secs(5)).await;
                continue;
            }
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

/// 处理一条 update；返回 true 表示已终结（用户点「发送」）。
async fn handle_update(
    update: &Value,
    client: &TelegramClient,
    options: &[String],
    selected: &mut Vec<String>,
    user_input: &mut String,
    options_message_id: i64,
    operation_message_id: i64,
) -> bool {
    // callback_query：切换选项
    if let Some(cb) = update.get("callback_query") {
        if let Some(chat_id) = cb
            .get("message")
            .and_then(|m| m.get("chat"))
            .and_then(|c| c.get("id"))
            .and_then(|v| v.as_i64())
        {
            if chat_id != client.chat_id() {
                return false;
            }
        }
        if let Some(data) = cb.get("data").and_then(|d| d.as_str()) {
            if let Some(opt) = data.strip_prefix("toggle:") {
                toggle(selected, opt);
                client
                    .edit_message_reply_markup(
                        options_message_id,
                        inline_keyboard(options, selected.as_slice()),
                    )
                    .await;
            }
        }
        if let Some(cb_id) = cb.get("id").and_then(|i| i.as_str()) {
            client.answer_callback_query(cb_id).await;
        }
        return false;
    }

    // message：文本回复 / 发送
    if let Some(message) = update.get("message") {
        let chat_ok = message
            .get("chat")
            .and_then(|c| c.get("id"))
            .and_then(|v| v.as_i64())
            == Some(client.chat_id());
        if !chat_ok {
            return false;
        }
        if let Some(msg_id) = message.get("message_id").and_then(|v| v.as_i64()) {
            if msg_id <= operation_message_id {
                return false;
            }
        }
        if let Some(text) = message.get("text").and_then(|t| t.as_str()) {
            if text == SEND_BUTTON {
                return true;
            }
            *user_input = text.to_string();
        }
    }
    false
}

fn toggle(selected: &mut Vec<String>, option: &str) {
    if let Some(i) = selected.iter().position(|s| s == option) {
        selected.remove(i);
    } else {
        selected.push(option.to_string());
    }
}

fn inline_keyboard(options: &[String], selected: &[String]) -> Value {
    let mut rows: Vec<Value> = Vec::new();
    let mut i = 0;
    while i < options.len() {
        let mut row: Vec<Value> = Vec::new();
        for option in &options[i..(i + 2).min(options.len())] {
            let text = if selected.iter().any(|s| s == option) {
                format!("✅ {}", option)
            } else {
                option.clone()
            };
            row.push(json!({ "text": text, "callback_data": format!("toggle:{}", option) }));
        }
        rows.push(Value::Array(row));
        i += 2;
    }
    json!({ "inline_keyboard": rows })
}

fn reply_keyboard() -> Value {
    json!({
        "keyboard": [[{ "text": SEND_BUTTON }]],
        "resize_keyboard": true,
        "one_time_keyboard": true
    })
}
