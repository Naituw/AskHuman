//! Telegram Channel：发送提问 + 长轮询接收回复（不接收图片），逐项对齐 Swift 版。
//!
//! 编排逻辑（单/多题、收集答案、投递）已上移到 `channels::conversation::run_conversation`；
//! 本文件提供传输相关实现 `TelegramSession`（`MessagingChannel`）+ 薄外层 `TelegramChannel`。

use super::conversation::{run_conversation, MessagingChannel, QuestionCtx};
use super::{Channel, Preemption, ResultSink};
use crate::config::TelegramChannelConfig;
use crate::i18n::{self, Lang};
use crate::models::{AskRequest, MessagePrompt, QuestionAnswer};
use crate::telegram::{markdown, TelegramClient};
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Duration;

/// 提交按钮回传数据（inline 键盘）。
const SUBMIT_CALLBACK: &str = "submit";

/// 薄外层：接 Coordinator（并行抢答），把会话委托给 `run_conversation` + `TelegramSession`。
pub struct TelegramChannel {
    config: TelegramChannelConfig,
    preempt: Arc<Preemption>,
}

impl TelegramChannel {
    pub fn new(config: TelegramChannelConfig) -> Self {
        Self {
            config,
            preempt: Arc::new(Preemption::new()),
        }
    }
}

impl Channel for TelegramChannel {
    fn id(&self) -> &str {
        "telegram"
    }

    fn start(&self, request: &AskRequest, sink: ResultSink) {
        let config = self.config.clone();
        let preempt = self.preempt.clone();
        let request = request.clone();
        tauri::async_runtime::spawn(async move {
            let mut session = TelegramSession::new(config);
            if let Err(e) = session.open().await {
                let lang = Lang::current();
                eprintln!(
                    "{}{}",
                    i18n::warn_prefix(lang),
                    i18n::tr(lang, "channel.tgConfigInvalidSkip").replace("{e}", &e.to_string())
                );
                return;
            }
            run_conversation(&mut session, &request, preempt, sink).await;
        });
    }

    fn cancel_by_other(&self, winner: &str) {
        self.preempt.cancel(winner);
    }
}

/// 传输实现：持有 client 与跨题长轮询 offset。
pub struct TelegramSession {
    config: TelegramChannelConfig,
    client: Option<TelegramClient>,
    offset: i64,
}

impl TelegramSession {
    pub fn new(config: TelegramChannelConfig) -> Self {
        Self {
            config,
            client: None,
            offset: 0,
        }
    }
}

#[async_trait::async_trait]
impl MessagingChannel for TelegramSession {
    fn id(&self) -> &str {
        "telegram"
    }

    async fn open(&mut self) -> Result<(), String> {
        let client = TelegramClient::new(
            self.config.bot_token.clone(),
            self.config.chat_id.clone(),
            self.config.api_base_url.clone(),
        )
        .map_err(|e| e.to_string())?;
        self.client = Some(client);
        Ok(())
    }

    async fn send_message_prompt(
        &mut self,
        message: &MessagePrompt,
        is_markdown: bool,
        source: &str,
        lang: Lang,
    ) {
        if let Some(client) = self.client.as_ref() {
            send_message_prompt(client, message, is_markdown, source, lang).await;
        }
    }

    async fn ask_question(
        &mut self,
        ctx: &QuestionCtx<'_>,
        preempt: &Preemption,
    ) -> Option<QuestionAnswer> {
        // 拆分借用：client 不可变 + offset 可变。
        let Self { client, offset, .. } = self;
        let client = client.as_ref()?;
        ask_question(
            client,
            ctx.header,
            ctx.text,
            ctx.options,
            ctx.is_markdown,
            ctx.lang,
            preempt,
            offset,
        )
        .await
    }

    async fn close(&mut self) {}
}

/// 发送共享 Message：头部「Question from {名}」+（文本，若有）+ 其展示文件。
async fn send_message_prompt(
    client: &TelegramClient,
    message: &MessagePrompt,
    is_markdown: bool,
    source: &str,
    lang: Lang,
) {
    let header = format!(
        "「{}」",
        i18n::tr(lang, "channel.messageFrom").replace("{source}", source)
    );
    send_composed(client, &header, &message.text, is_markdown, None).await;

    // 发送 Message 的展示文件（图片→sendPhoto，其它→sendDocument）。
    for file in &message.files {
        let result = if file.is_image {
            client.send_photo(&file.path, &file.name).await
        } else {
            client.send_document(&file.path, &file.name).await
        };
        if let Err(e) = result {
            eprintln!(
                "{}{}",
                i18n::warn_prefix(lang),
                i18n::tr(lang, "channel.fileSendFailedLog")
                    .replace("{path}", &file.path)
                    .replace("{e}", &e.to_string())
            );
            let _ = client
                .send_message(
                    &i18n::tr(lang, "channel.fileSendFailed").replace("{name}", &file.path),
                    None,
                    None,
                )
                .await;
        }
    }
}

/// 发送一道题（单卡片：正文 + 补充提示 + inline 选项/提交键盘）并长轮询直到用户点「提交」。
/// `header` 为题首加粗行（来源头部或 `Question i/n`），为空则只发问题正文。
/// 卡片发出后、提交前用户在聊天里发的文字会累积进 `user_input`。
/// 终态：本端胜出→卡片改「✅ 已回复」；被抢答→改「✅ 已在{赢家}回答」并去键盘后返回 None。
async fn ask_question(
    client: &TelegramClient,
    header: &str,
    question_text: &str,
    options: &[String],
    is_markdown: bool,
    lang: Lang,
    preempt: &Preemption,
    offset: &mut i64,
) -> Option<QuestionAnswer> {
    let options = options.to_vec();
    let mut selected: Vec<String> = Vec::new();
    let mut user_input = String::new();

    // 单卡片：正文 = 题干 + 选项清单（A. xxx，按钮只放字母规避超长选项显示不全）+ 补充提示；
    // inline 键盘 = 字母选项（可多选）+「提交」。
    let content = card_content(question_text, &options);
    let hint = i18n::tr(lang, "channel.tgActionHint");
    let body = if content.is_empty() {
        hint.to_string()
    } else {
        format!("{}\n\n{}", content, hint)
    };
    let keyboard = card_keyboard(&options, &selected, lang);
    let card_message_id = send_composed(client, header, &body, is_markdown, Some(keyboard)).await;

    while !preempt.is_cancelled() {
        match client.get_updates(*offset).await {
            Ok(updates) => {
                for update in updates {
                    if let Some(uid) = update.get("update_id").and_then(|v| v.as_i64()) {
                        *offset = uid + 1;
                    }
                    if handle_update(
                        &update,
                        client,
                        &options,
                        &mut selected,
                        &mut user_input,
                        card_message_id,
                        lang,
                    )
                    .await
                    {
                        // 本端胜出：卡片改「已回复」终态、去键盘。
                        let status = i18n::tr(lang, "channel.tgReplied");
                        finalize_card(client, card_message_id, header, &content, &status).await;
                        return Some(QuestionAnswer {
                            selected_options: selected,
                            user_input: {
                                let t = user_input.trim();
                                if t.is_empty() {
                                    None
                                } else {
                                    Some(t.to_string())
                                }
                            },
                            images: Vec::new(),
                            files: Vec::new(),
                        });
                    }
                }
            }
            Err(_) => {
                tokio::time::sleep(Duration::from_secs(5)).await;
                continue;
            }
        }
        // 轮询间隔：分片检查抢答信号，被抢答后尽快跳出去收尾（降低延迟）。
        for _ in 0..10 {
            if preempt.is_cancelled() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    // 被抢答：卡片改「已在{赢家}回答」终态、去键盘。
    let status = i18n::tr(lang, "channel.tgAnsweredVia").replace("{source}", &preempt.winner());
    finalize_card(client, card_message_id, header, &content, &status).await;
    None
}

/// 把卡片编辑为终态：保留头部 + 内容（题干 + 选项清单），追加状态行，并移除按钮（不传 reply_markup）。
async fn finalize_card(
    client: &TelegramClient,
    card_message_id: i64,
    header: &str,
    content: &str,
    status: &str,
) {
    let mut text = String::new();
    if !header.is_empty() {
        text.push_str(header);
        text.push_str("\n\n");
    }
    if !content.trim().is_empty() {
        text.push_str(content);
        text.push_str("\n\n");
    }
    text.push_str(status);
    client.edit_message_text(card_message_id, &text, None).await;
}

/// 卡片正文内容：题干 +（选项清单，每行「字母. 选项全文」）。无题干/无选项各自省略。
fn card_content(question_text: &str, options: &[String]) -> String {
    let mut s = String::new();
    if !question_text.trim().is_empty() {
        s.push_str(question_text);
    }
    if !options.is_empty() {
        if !s.is_empty() {
            s.push_str("\n\n");
        }
        for (idx, opt) in options.iter().enumerate() {
            s.push_str(&format!("{} {}\n", option_label(idx), opt));
        }
        s = s.trim_end().to_string();
    }
    s
}

/// 选项标签：数字键帽 emoji（1️⃣2️⃣3️⃣…，彩色且渲染可靠，避免与模型输出里的纯文本 1./2. 混淆）；
/// 第 10 个用 🔟；超过 10 个用纯序号（无对应键帽 emoji）。
fn option_label(idx: usize) -> String {
    match idx {
        // 1️⃣–9️⃣：数字 + U+FE0F(emoji 变体) + U+20E3(组合键帽)。
        0..=8 => format!("{}\u{fe0f}\u{20e3}", idx + 1),
        9 => "🔟".to_string(),
        _ => (idx + 1).to_string(),
    }
}

/// 组装「加粗头部 + 正文」并发送（MarkdownV2 失败回退纯文本），返回消息 id。
/// `header`/`body` 任一为空时自动省略对应部分；都为空时用占位符避免空消息。
async fn send_composed(
    client: &TelegramClient,
    header: &str,
    body: &str,
    is_markdown: bool,
    inline: Option<Value>,
) -> i64 {
    let plain = match (header.is_empty(), body.is_empty()) {
        (true, true) => "…".to_string(),
        (false, true) => header.to_string(),
        (true, false) => body.to_string(),
        (false, false) => format!("{}\n\n{}", header, body),
    };
    // markdown 正文交给 markdown::process；非 markdown 正文整体转义；头部始终加粗。
    let md = if is_markdown {
        match (header.is_empty(), body.is_empty()) {
            (true, true) => "…".to_string(),
            (false, true) => markdown::process(&format!("**{}**", header)),
            (true, false) => markdown::process(body),
            (false, false) => markdown::process(&format!("**{}**\n\n{}", header, body)),
        }
    } else {
        match (header.is_empty(), body.is_empty()) {
            (true, true) => "…".to_string(),
            (false, true) => format!("*{}*", markdown::escape_all(header)),
            (true, false) => markdown::escape_all(body),
            (false, false) => format!(
                "*{}*\n\n{}",
                markdown::escape_all(header),
                markdown::escape_all(body)
            ),
        }
    };
    match client
        .send_message(&md, Some("MarkdownV2"), inline.clone())
        .await
    {
        Ok(id) => id,
        Err(_) => client.send_message(&plain, None, inline).await.unwrap_or(0),
    }
}

/// 处理一条 update；返回 true 表示已终结（用户点「提交」）。
/// 选项切换走 callback（`toggle:`）；提交走 callback（`submit`）；卡片之后的文字消息累积进 `user_input`。
async fn handle_update(
    update: &Value,
    client: &TelegramClient,
    options: &[String],
    selected: &mut Vec<String>,
    user_input: &mut String,
    card_message_id: i64,
    lang: Lang,
) -> bool {
    // callback_query：切换选项 / 提交。仅处理本卡片的回调。
    if let Some(cb) = update.get("callback_query") {
        let msg = cb.get("message");
        let chat_ok = msg
            .and_then(|m| m.get("chat"))
            .and_then(|c| c.get("id"))
            .and_then(|v| v.as_i64())
            == Some(client.chat_id());
        let same_card = msg
            .and_then(|m| m.get("message_id"))
            .and_then(|v| v.as_i64())
            == Some(card_message_id);
        if !chat_ok || !same_card {
            if let Some(cb_id) = cb.get("id").and_then(|i| i.as_str()) {
                client.answer_callback_query(cb_id).await;
            }
            return false;
        }
        let mut finished = false;
        if let Some(data) = cb.get("data").and_then(|d| d.as_str()) {
            if data == SUBMIT_CALLBACK {
                finished = true;
            } else if let Some(idx) = data
                .strip_prefix("toggle:")
                .and_then(|s| s.parse::<usize>().ok())
            {
                if let Some(opt) = options.get(idx) {
                    toggle(selected, opt);
                    client
                        .edit_message_reply_markup(
                            card_message_id,
                            card_keyboard(options, selected.as_slice(), lang),
                        )
                        .await;
                }
            }
        }
        if let Some(cb_id) = cb.get("id").and_then(|i| i.as_str()) {
            client.answer_callback_query(cb_id).await;
        }
        return finished;
    }

    // message：卡片之后用户发的文字 → 累积为补充输入。
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
            if msg_id <= card_message_id {
                return false;
            }
        }
        if let Some(text) = message.get("text").and_then(|t| t.as_str()) {
            if !user_input.is_empty() {
                user_input.push('\n');
            }
            user_input.push_str(text);
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

/// 每行字母按钮个数（字母短，可密排）。
const KEYBOARD_ROW_WIDTH: usize = 4;

/// 单卡片 inline 键盘：选项行（按钮只放字母 A/B/C…，选中加 ✅，每行 4 个）+ 末行「提交」按钮。
/// 选项全文列在卡片正文里；按钮放字母既规避超长选项显示不全，也让 callback_data 短小。
/// callback_data 用选项下标（`toggle:{i}`）：Telegram 限制其 ≤ 64 字节。
fn card_keyboard(options: &[String], selected: &[String], lang: Lang) -> Value {
    let mut rows: Vec<Value> = Vec::new();
    let mut i = 0;
    while i < options.len() {
        let end = (i + KEYBOARD_ROW_WIDTH).min(options.len());
        let mut row: Vec<Value> = Vec::new();
        for idx in i..end {
            let option = &options[idx];
            let label = option_label(idx);
            let text = if selected.iter().any(|s| s == option) {
                format!("✅ {}", label)
            } else {
                label
            };
            row.push(json!({ "text": text, "callback_data": format!("toggle:{}", idx) }));
        }
        rows.push(Value::Array(row));
        i += KEYBOARD_ROW_WIDTH;
    }
    rows.push(json!([
        { "text": i18n::tr(lang, "channel.tgSendButton"), "callback_data": SUBMIT_CALLBACK }
    ]));
    json!({ "inline_keyboard": rows })
}
