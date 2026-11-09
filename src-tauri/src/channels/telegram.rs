//! Telegram Channel：发送提问 + 长轮询接收回复（不接收图片），逐项对齐 Swift 版。
//!
//! 编排逻辑（单/多题、收集答案、投递）已上移到 `channels::conversation::run_conversation`；
//! 本文件提供传输相关实现 `TelegramSession`（`MessagingChannel`）+ 薄外层 `TelegramChannel`。

use super::conversation::{
    run_conversation, InboundReply, MessagingChannel, QuestionCtx, QuestionOutcome,
    QuestionProgress,
};
use super::{Channel, ConversationOrigin, Interruption, Preemption, ResultSink};
use crate::config::TelegramChannelConfig;
use crate::i18n::{self, Lang};
use crate::models::{AskRequest, MessagePrompt, QuestionAnswer};
use crate::telegram::router::{RoutedTg, TgInbound, TgRouter};
use crate::telegram::{markdown, TelegramClient};
use serde_json::{json, Value};
use sha2::Digest;
use std::sync::Arc;
use std::time::Duration;

/// 提交按钮回传数据（inline 键盘）。
const SUBMIT_CALLBACK: &str = "submit";

/// 事件源轮询间隔：每隔此时长从 Router 句柄取一次事件，以便分片检查抢答信号。
const POLL_INTERVAL: Duration = Duration::from_millis(200);

async fn send_inbound_reply(client: &TelegramClient, reply: InboundReply, lang: Lang) {
    match reply {
        InboundReply::Text(text) => {
            let _ = client.send_message(&text, None, None).await;
        }
        InboundReply::Help(view) => {
            let html = crate::telegram::help::render(&view, lang);
            if client
                .send_message(&html, Some("HTML"), None)
                .await
                .is_err()
            {
                let plain = crate::autochannel::render_help_plain(&view, lang);
                let _ = client.send_message(&plain, None, None).await;
            }
        }
    }
}

/// Router 归属：单进程自建一个仅挂本会话的 Router；Daemon 复用共享且常热的 Router。
#[derive(Clone)]
enum TgTransport {
    Own,
    Shared(Arc<TgRouter>),
}

/// 薄外层：接 Coordinator（并行抢答），把会话委托给 `run_conversation` + `TelegramSession`。
pub struct TelegramChannel {
    config: TelegramChannelConfig,
    preempt: Arc<Preemption>,
    transport: TgTransport,
}

impl TelegramChannel {
    /// 单进程外层：本会话自建并独占一个轮询器（每进程一个 Router、仅挂本会话）。
    pub fn new(config: TelegramChannelConfig) -> Self {
        Self {
            config,
            preempt: Arc::new(Preemption::new()),
            transport: TgTransport::Own,
        }
    }

    /// Daemon 外层：复用共享且常热的 Router（单一 offset 轮询，根治多轮询互吞更新）。
    pub fn shared(config: TelegramChannelConfig, router: Arc<TgRouter>) -> Self {
        Self {
            config,
            preempt: Arc::new(Preemption::new()),
            transport: TgTransport::Shared(router),
        }
    }
}

impl Channel for TelegramChannel {
    fn id(&self) -> &str {
        "telegram"
    }

    fn start(&self, request: &AskRequest, origin: &ConversationOrigin, sink: ResultSink) {
        let config = self.config.clone();
        let preempt = self.preempt.clone();
        let request = request.clone();
        let origin = origin.clone();
        let transport = self.transport.clone();
        tauri::async_runtime::spawn(async move {
            let lang = Lang::current();
            // 取得本会话的事件源句柄（Own：现起一个 Router；Shared：复用）。`_keep` 持有
            // Own Router 直至会话结束；Shared 的 Router 由 Daemon 持有。
            let (events, _keep): (RoutedTg, Option<Arc<TgRouter>>) = match transport {
                TgTransport::Own => match TgRouter::connect(&config).await {
                    Ok(router) => (router.register(), Some(router)),
                    Err(e) => {
                        eprintln!(
                            "{}{}",
                            i18n::warn_prefix(lang),
                            i18n::tr(lang, "channel.tgConfigInvalidSkip").replace("{e}", &e)
                        );
                        sink.surface_lost("telegram", &e);
                        return;
                    }
                },
                TgTransport::Shared(router) => (router.register(), None),
            };
            let mut session = TelegramSession::new(config, events);
            if let Err(e) = session.open().await {
                eprintln!(
                    "{}{}",
                    i18n::warn_prefix(lang),
                    i18n::tr(lang, "channel.tgConfigInvalidSkip").replace("{e}", &e.to_string())
                );
                sink.surface_lost("telegram", &e.to_string());
                return;
            }
            run_conversation(&mut session, &request, &origin, preempt, sink).await;
        });
    }

    fn interrupt(&self, reason: &Interruption) {
        self.preempt.interrupt(reason.clone());
    }
}

/// 传输实现：持有 client（发送/编辑/应答）与 Router 事件源句柄（接收，轮询由 Router 独占）。
pub struct TelegramSession {
    config: TelegramChannelConfig,
    client: Option<TelegramClient>,
    events: Option<RoutedTg>,
}

impl TelegramSession {
    pub fn new(config: TelegramChannelConfig, events: RoutedTg) -> Self {
        Self {
            config,
            client: None,
            events: Some(events),
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
        header: &str,
        lang: Lang,
    ) {
        if let Some(client) = self.client.as_ref() {
            send_message_prompt(client, message, is_markdown, header, lang).await;
        }
    }

    async fn ask_question(
        &mut self,
        ctx: &QuestionCtx<'_>,
        preempt: &Preemption,
    ) -> QuestionOutcome {
        let Self {
            client,
            events,
            config,
        } = self;
        let (Some(client), Some(events)) = (client.as_ref(), events.as_mut()) else {
            return QuestionOutcome::Lost;
        };
        let route = format!(
            "{:x}",
            sha2::Sha256::digest(format!("{}|{}", config.api_base_url, config.bot_token))
        );
        ask_question(client, events, ctx, preempt, &route).await
    }

    async fn close(&mut self) {
        // 丢弃事件源句柄 → 从 Router 注销路由（Daemon 下及时清理，避免陈旧路由堆积）。
        self.events = None;
    }
}

/// Send the shared Message with the fully assembled per-request origin title and attachments.
async fn send_message_prompt(
    client: &TelegramClient,
    message: &MessagePrompt,
    is_markdown: bool,
    header: &str,
    lang: Lang,
) {
    send_composed(client, header, &message.text, is_markdown, None).await;

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

/// Restore the original question card and its accumulated input.
async fn ask_question(
    client: &TelegramClient,
    events: &mut RoutedTg,
    ctx: &QuestionCtx<'_>,
    preempt: &Preemption,
    route: &str,
) -> QuestionOutcome {
    let saved = match ctx.saved::<QuestionProgress>("telegram") {
        Ok(saved) => saved,
        Err(error) => {
            eprintln!("[telegram] cannot restore question: {error}");
            return QuestionOutcome::Lost;
        }
    };
    let target = client.chat_id().to_string();
    if saved.target_id.as_ref().is_some_and(|id| id != &target)
        || saved.route_id.as_ref().is_some_and(|id| id != route)
    {
        return QuestionOutcome::Lost;
    }
    if !ctx
        .commit(
            "telegram",
            &json!({"targetId": target, "routeId": route}),
            preempt,
        )
        .await
    {
        return QuestionOutcome::Interrupted;
    }
    let displays = ctx
        .options
        .iter()
        .map(|option| super::conversation::display_text(option, ctx.lang))
        .collect::<Vec<_>>();
    let options = ctx
        .options
        .iter()
        .map(|option| option.text.clone())
        .collect::<Vec<_>>();
    let mut selected = saved.selected_options;
    let mut input = saved.input;
    let mut processed = saved.processed_events;
    let title = if ctx.header.trim().is_empty() {
        i18n::tr(ctx.lang, "channel.tgTitleFallback")
    } else {
        ctx.header
    };
    let header = format!("❓ {title}");
    let content = card_content(ctx.text, &displays);
    let hint = if ctx.select_only {
        if ctx.single {
            i18n::tr(ctx.lang, "channel.tgActionHintSelectSingle")
        } else {
            i18n::tr(ctx.lang, "channel.tgActionHintSelectMulti")
        }
    } else {
        i18n::tr(ctx.lang, "channel.tgActionHint")
    };
    let body = if content.is_empty() {
        hint.to_string()
    } else {
        format!("{content}\n\n{hint}")
    };
    let id = match saved.message_id {
        Some(id) => match id.parse::<i64>() {
            Ok(id) => id,
            Err(_) => return QuestionOutcome::Lost,
        },
        None => {
            if preempt.is_cancelled() {
                return QuestionOutcome::Interrupted;
            }
            let keyboard = card_keyboard(&options, &selected, ctx.lang);
            let id = send_composed(client, &header, &body, ctx.is_markdown, Some(keyboard)).await;
            if id == 0 {
                return QuestionOutcome::Lost;
            }
            if !ctx
                .commit("telegram", &json!({"messageId": id.to_string()}), preempt)
                .await
            {
                return QuestionOutcome::Interrupted;
            }
            id
        }
    };
    if let Some(answer) = saved.answer {
        finalize_card(
            client,
            id,
            &header,
            &content,
            i18n::tr(ctx.lang, "channel.tgReplied"),
            ctx.is_markdown,
        )
        .await;
        return QuestionOutcome::Answered(answer);
    }
    events.set_active(client.chat_id(), id);
    loop {
        let event = if preempt.is_handoff() {
            match events.try_recv() {
                Some(event) => event,
                None => break,
            }
        } else {
            if preempt.is_cancelled() {
                break;
            }
            match tokio::time::timeout(POLL_INTERVAL, events.recv()).await {
                Ok(Some(event)) => event,
                Ok(None) => {
                    if preempt.is_cancelled() {
                        break;
                    }
                    events.clear_active(id);
                    return QuestionOutcome::Lost;
                }
                Err(_) => continue,
            }
        };
        if handle_event(
            event,
            client,
            ctx,
            preempt,
            &options,
            &mut selected,
            &mut input,
            id,
            &mut processed,
        )
        .await
        {
            let answer = question_answer(selected.clone(), &input);
            finalize_card(
                client,
                id,
                &header,
                &content,
                i18n::tr(ctx.lang, "channel.tgReplied"),
                ctx.is_markdown,
            )
            .await;
            events.clear_active(id);
            return QuestionOutcome::Answered(answer);
        }
    }
    if preempt.is_handoff() {
        events.clear_active(id);
        return QuestionOutcome::Interrupted;
    }
    let status = match preempt.reason() {
        Some(Interruption::AnsweredBy(winner)) => {
            i18n::tr(ctx.lang, "channel.tgAnsweredVia").replace("{source}", &winner)
        }
        Some(Interruption::Cancelled(source)) if !source.is_empty() => {
            i18n::tr(ctx.lang, "channel.tgCancelledBy").replace("{source}", &source)
        }
        _ => i18n::tr(ctx.lang, "channel.tgCancelled").to_string(),
    };
    finalize_card(client, id, &header, &content, &status, ctx.is_markdown).await;
    events.clear_active(id);
    QuestionOutcome::Interrupted
}

fn question_answer(selected: Vec<String>, input: &str) -> QuestionAnswer {
    QuestionAnswer {
        selected_options: selected,
        user_input: (!input.trim().is_empty()).then(|| input.trim().to_string()),
        ..Default::default()
    }
}

/// 把卡片编辑为终态：保留头部 + 内容（题干 + 选项清单），追加状态行，并移除按钮（不传 reply_markup）。
/// 优先按 HTML 渲染（与活动态一致）；解析失败时回退纯文本，确保终态一定写入。
async fn finalize_card(
    client: &TelegramClient,
    card_message_id: i64,
    header: &str,
    content: &str,
    status: &str,
    is_markdown: bool,
) {
    let body = if content.trim().is_empty() {
        status.to_string()
    } else {
        format!("{}\n\n{}", content, status)
    };
    let html = compose_html(header, &body, is_markdown);
    if client
        .edit_message_text(card_message_id, &html, Some("HTML"), None)
        .await
        .is_err()
    {
        let mut plain = String::new();
        if !header.is_empty() {
            plain.push_str(header);
            plain.push_str("\n\n");
        }
        if !content.trim().is_empty() {
            plain.push_str(content);
            plain.push_str("\n\n");
        }
        plain.push_str(status);
        let _ = client
            .edit_message_text(card_message_id, &plain, None, None)
            .await;
    }
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
pub(crate) fn option_label(idx: usize) -> String {
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
    // 用 Telegram HTML：头部始终加粗（仅转义）；markdown 正文走 to_html，非 markdown 正文仅转义。
    let html = compose_html(header, body, is_markdown);
    match client
        .send_message(&html, Some("HTML"), inline.clone())
        .await
    {
        Ok(id) => id,
        Err(_) => client.send_message(&plain, None, inline).await.unwrap_or(0),
    }
}

/// 组装「加粗头部 + 正文」为 Telegram HTML。头部为纯文本（仅转义），正文按 `is_markdown` 决定是否解析。
fn compose_html(header: &str, body: &str, is_markdown: bool) -> String {
    let body_html = |b: &str| {
        if is_markdown {
            markdown::to_html(b)
        } else {
            markdown::escape_html(b)
        }
    };
    match (header.is_empty(), body.is_empty()) {
        (true, true) => "…".to_string(),
        (false, true) => format!("<b>{}</b>", markdown::escape_html(header)),
        (true, false) => body_html(body),
        (false, false) => format!(
            "<b>{}</b>\n\n{}",
            markdown::escape_html(header),
            body_html(body)
        ),
    }
}

/// 处理一个 Router 分发来的事件；返回 true 表示已终结（用户点「提交」）。
/// 选项切换走 callback（`toggle:`）；提交走 callback（`submit`）；卡片之后的文字累积进 `user_input`。
/// 路由（chat / 卡片匹配）已由 Router 完成，这里只处理业务语义。
#[allow(clippy::too_many_arguments)]
async fn handle_event(
    event: TgInbound,
    client: &TelegramClient,
    ctx: &QuestionCtx<'_>,
    preempt: &Preemption,
    options: &[String],
    selected: &mut Vec<String>,
    input: &mut String,
    card_message_id: i64,
    processed: &mut Vec<String>,
) -> bool {
    match event {
        TgInbound::Callback(callback) => {
            let callback_id = callback.get("id").and_then(Value::as_str).unwrap_or("");
            let identity = format!("callback:{callback_id}");
            if processed.contains(&identity) {
                client.answer_callback_query(callback_id).await;
                return false;
            }
            let data = callback.get("data").and_then(Value::as_str).unwrap_or("");
            if data == SUBMIT_CALLBACK {
                if ctx.select_only && selected.is_empty() {
                    client
                        .answer_callback_query_alert(
                            callback_id,
                            i18n::tr(ctx.lang, "channel.tgSelectRequired"),
                        )
                        .await;
                    return false;
                }
                remember_event(processed, identity);
                let answer = question_answer(selected.clone(), input);
                if !ctx
                    .commit_response(
                        "telegram",
                        &json!({"answer": answer, "processedEvents": processed}),
                        preempt,
                    )
                    .await
                {
                    return false;
                }
                client.answer_callback_query(callback_id).await;
                return true;
            }
            if let Some(index) = data
                .strip_prefix("toggle:")
                .and_then(|index| index.parse::<usize>().ok())
            {
                if let Some(option) = options.get(index) {
                    if ctx.single {
                        toggle_single(selected, option);
                    } else {
                        toggle(selected, option);
                    }
                    remember_event(processed, identity);
                    if !ctx.commit_response("telegram", &json!({"selectedOptions": selected, "input": input, "processedEvents": processed}), preempt).await { return false; }
                    client
                        .edit_message_reply_markup(
                            card_message_id,
                            card_keyboard(options, selected, ctx.lang),
                        )
                        .await;
                }
            }
            client.answer_callback_query(callback_id).await;
            false
        }
        TgInbound::Text {
            text, message_id, ..
        } => {
            if ctx.select_only {
                if let Some(reply) = super::conversation::answer_inbound_reply(
                    None,
                    crate::autochannel::AckMode::Card,
                    &text,
                    "telegram",
                    ctx.lang,
                ) {
                    send_inbound_reply(client, reply, ctx.lang).await;
                }
                return false;
            }
            let identity = format!("text:{message_id}");
            if message_id <= card_message_id || processed.contains(&identity) {
                return false;
            }
            if !input.is_empty() {
                input.push('\n');
            }
            input.push_str(&text);
            remember_event(processed, identity);
            if !ctx.commit_response("telegram", &json!({"selectedOptions": selected, "input": input, "processedEvents": processed}), preempt).await { return false; }
            if let Some(reply) = super::conversation::answer_inbound_reply(
                Some(crate::autochannel::AckKind::Text),
                crate::autochannel::AckMode::Card,
                &text,
                "telegram",
                ctx.lang,
            ) {
                send_inbound_reply(client, reply, ctx.lang).await;
            }
            false
        }
    }
}

fn remember_event(processed: &mut Vec<String>, identity: String) {
    const KEEP: usize = 512;
    processed.push(identity);
    if processed.len() > KEEP {
        processed.drain(..processed.len() - KEEP);
    }
}

fn toggle(selected: &mut Vec<String>, option: &str) {
    if let Some(i) = selected.iter().position(|s| s == option) {
        selected.remove(i);
    } else {
        selected.push(option.to_string());
    }
}

/// 单选互斥：点中已选项则清空；否则清空后只保留该项（真 radio 语义）。
fn toggle_single(selected: &mut Vec<String>, option: &str) {
    if selected.iter().any(|s| s == option) {
        selected.clear();
    } else {
        selected.clear();
        selected.push(option.to_string());
    }
}

/// 每行字母按钮个数（字母短，可密排）。
pub(crate) const KEYBOARD_ROW_WIDTH: usize = 4;

/// 单卡片 inline 键盘：选项行（按钮只放字母 A/B/C…，选中加 ✅，每行 4 个）+ 末行「提交」按钮。
/// 选项全文列在卡片正文里；按钮放字母既规避超长选项显示不全，也让 callback_data 短小。
/// callback_data 用选项下标（`toggle:{i}`）：Telegram 限制其 ≤ 64 字节。
fn card_keyboard(options: &[String], selected: &[String], lang: Lang) -> Value {
    let mut rows: Vec<Value> = Vec::new();
    let mut i = 0;
    while i < options.len() {
        let end = (i + KEYBOARD_ROW_WIDTH).min(options.len());
        let mut row: Vec<Value> = Vec::new();
        for (idx, option) in options.iter().enumerate().take(end).skip(i) {
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
