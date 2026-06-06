//! 飞书 Channel：长连接(WebSocket)收（卡片回调 card.action.trigger + 用户消息 im.message.receive_v1）
//! + OpenAPI 发（文本 / 图片 / 文件 / 互动卡片）。
//!
//! 方案 A（默认）：提问以**互动卡片**（卡片 JSON 2.0）下发——表单容器内放勾选器(多选)、输入框(补充文字)、
//! 提交按钮；用户点「提交」产生一次 `card.action.trigger` 回调（含 `form_value`）完成该题；
//! 作答期间在聊天里发的图片/文件会被累积进答案（纯文字忽略，请用卡片输入框）。卡片回调走长连接，零公网。
//!
//! 方案 B（兜底）：当卡片投放失败时，回退「纯文本 + 编号选项」——用户回一条消息即完成该题。
//!
//! 编排逻辑复用 `conversation::run_conversation`，本文件提供传输实现 `FeishuSession`
//! （`MessagingChannel`）+ 薄外层 `FeishuChannel`。

use super::conversation::{run_conversation, MessagingChannel, QuestionCtx};
use super::{Channel, Preemption, ResultSink};
use crate::config::FeishuChannelConfig;
use crate::feishu::card;
use crate::feishu::client::FeishuClient;
use crate::feishu::ws::{FeishuWs, WsEvent};
use crate::i18n::{self, Lang};
use crate::models::{ImageAttachment, MessagePrompt, QuestionAnswer};
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Duration;

/// 抢答轮询粒度：每隔此时长检查一次抢答信号。
const POLL_INTERVAL: Duration = Duration::from_secs(1);

/// Message（正文+附件）发完到发第一道题之间的等待时长，保证「先 message 后题目」的视觉顺序。
const MESSAGE_SETTLE_DELAY: Duration = Duration::from_millis(500);

/// 薄外层：接 Coordinator（并行抢答），把会话委托给 `run_conversation` + `FeishuSession`。
pub struct FeishuChannel {
    config: FeishuChannelConfig,
    preempt: Arc<Preemption>,
}

impl FeishuChannel {
    pub fn new(config: FeishuChannelConfig) -> Self {
        Self {
            config,
            preempt: Arc::new(Preemption::new()),
        }
    }
}

impl Channel for FeishuChannel {
    fn id(&self) -> &str {
        "feishu"
    }

    fn start(&self, request: &crate::models::AskRequest, sink: ResultSink) {
        let config = self.config.clone();
        let preempt = self.preempt.clone();
        let request = request.clone();
        tauri::async_runtime::spawn(async move {
            let mut session = FeishuSession::new(config);
            if let Err(e) = session.open().await {
                let lang = Lang::current();
                eprintln!(
                    "{}{}",
                    i18n::warn_prefix(lang),
                    i18n::tr(lang, "channel.fsConfigInvalidSkip").replace("{e}", &e)
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

/// 传输实现：持有 client 与长连接（跨题复用）。
pub struct FeishuSession {
    config: FeishuChannelConfig,
    client: Option<FeishuClient>,
    ws: Option<FeishuWs>,
}

impl FeishuSession {
    pub fn new(config: FeishuChannelConfig) -> Self {
        Self {
            config,
            client: None,
            ws: None,
        }
    }
}

#[async_trait::async_trait]
impl MessagingChannel for FeishuSession {
    fn id(&self) -> &str {
        "feishu"
    }

    async fn open(&mut self) -> Result<(), String> {
        let client = FeishuClient::new(&self.config).map_err(|e| e.to_string())?;
        if client.open_id().is_empty() {
            return Err(i18n::tr(Lang::current(), "err.fsEmptyConfig").replace("{field}", "Open ID"));
        }
        let ws = FeishuWs::connect(
            client.http().clone(),
            client.base_url(),
            client.app_id(),
            client.app_secret(),
        )
        .await
        .map_err(|e| e.to_string())?;
        self.client = Some(client);
        self.ws = Some(ws);
        Ok(())
    }

    async fn send_message_prompt(
        &mut self,
        message: &MessagePrompt,
        _is_markdown: bool,
        source: &str,
        lang: Lang,
    ) {
        let Some(client) = self.client.as_ref() else {
            return;
        };
        let header = i18n::tr(lang, "channel.messageFrom").replace("{source}", source);
        let body = if message.text.trim().is_empty() {
            header.clone()
        } else {
            format!("{}\n\n{}", header, message.text)
        };
        if let Err(e) = client.send_text(&body).await {
            eprintln!(
                "{}{}",
                i18n::warn_prefix(lang),
                i18n::tr(lang, "channel.fsMessageSendFailed").replace("{e}", &e.to_string())
            );
        }

        // 展示文件：图片用 image 消息，其余用 file 消息（原生收发，不做文本转 docx）。
        for file in &message.files {
            if let Err(e) = send_attachment(client, &file.path, &file.name, file.is_image).await {
                eprintln!(
                    "{}{}",
                    i18n::warn_prefix(lang),
                    i18n::tr(lang, "channel.fsFileSendFailedLog")
                        .replace("{path}", &file.path)
                        .replace("{e}", &e.to_string())
                );
                let _ = client
                    .send_text(
                        &i18n::tr(lang, "channel.fileSendFailed").replace("{name}", &file.name),
                    )
                    .await;
            }
        }

        tokio::time::sleep(MESSAGE_SETTLE_DELAY).await;
    }

    async fn ask_question(
        &mut self,
        ctx: &QuestionCtx<'_>,
        preempt: &Preemption,
    ) -> Option<QuestionAnswer> {
        let title = if ctx.header.is_empty() {
            i18n::tr(ctx.lang, "channel.fsTitleFallback")
        } else {
            ctx.header
        };

        let Self { client, ws, config } = self;
        let client = client.as_ref()?;
        let ws = ws.as_mut()?;
        let open_id = config.open_id.trim().to_string();

        let placeholder = i18n::tr(ctx.lang, "channel.fsInputPlaceholder");
        let submit_label = i18n::tr(ctx.lang, "channel.fsSubmitButton");
        let question_card = card::build_question_card(
            title,
            ctx.text,
            ctx.options,
            ctx.is_markdown,
            placeholder,
            submit_label,
        );

        // 1. 投放互动卡片；失败 → 回退纯文本编号方案。
        let message_id = match client.send_card(&question_card).await {
            Ok(id) => id,
            Err(e) => {
                eprintln!(
                    "{}{}",
                    i18n::warn_prefix(ctx.lang),
                    i18n::tr(ctx.lang, "channel.fsCardDeliverFailed").replace("{e}", &e.to_string())
                );
                return ask_question_text(client, ws, &open_id, ctx, preempt).await;
            }
        };

        // 2. 等卡片「提交」；作答期间累积聊天里的图片/文件；被抢答则收尾返回 None。
        let mut images: Vec<ImageAttachment> = Vec::new();
        let mut files: Vec<String> = Vec::new();
        while !preempt.is_cancelled() {
            let ev = match tokio::time::timeout(POLL_INTERVAL, ws.recv()).await {
                Ok(Some(ev)) => ev,
                Ok(None) => break,  // 长连接彻底断开
                Err(_) => continue, // 超时：回到循环顶部重新检查 cancelled
            };
            match ev {
                WsEvent::CardAction { data, frame } => {
                    match card::parse_card_submit(&data, ctx.options) {
                        Some(s) if s.message_id == message_id && s.open_id == open_id => {
                            // 3 秒内回包：toast 提示「已提交」。
                            let toast = i18n::tr(ctx.lang, "channel.fsSubmitted");
                            ws.respond_card(
                                &frame,
                                &json!({ "toast": { "type": "success", "content": toast } }),
                            )
                            .await;
                            // best-effort 把卡片 PATCH 成终态（灰显，去掉表单）。
                            let finalized = card::build_finalized_card(
                                title,
                                ctx.text,
                                i18n::tr(ctx.lang, "channel.fsSubmitted"),
                            );
                            let _ = client.patch_card(&message_id, &finalized).await;
                            return Some(QuestionAnswer {
                                selected_options: s.selected_options,
                                user_input: s.user_input,
                                images,
                                files,
                            });
                        }
                        // 非本卡片 / 非提交 → 空 ACK 确认，继续等待。
                        _ => ws.respond_ack(&frame).await,
                    }
                }
                WsEvent::Message(event) => {
                    if event_open_id(&event) == open_id {
                        accumulate_attachment(client, &event, &mut images, &mut files, ctx.lang)
                            .await;
                    }
                }
            }
        }

        // 被抢答 / 断连：尽力把卡片 PATCH 为终态（失败忽略）。
        let status = if preempt.is_cancelled() {
            i18n::tr(ctx.lang, "channel.fsAnsweredVia").replace("{source}", &preempt.winner())
        } else {
            i18n::tr(ctx.lang, "channel.fsSubmitted").to_string()
        };
        let finalized = card::build_finalized_card(title, ctx.text, &status);
        let _ = client.patch_card(&message_id, &finalized).await;
        None
    }

    async fn close(&mut self) {
        self.ws = None;
    }
}

/// 兜底：纯文本 + 编号选项问一题（卡片投放失败时使用）。用户回一条消息即完成该题。
async fn ask_question_text(
    client: &FeishuClient,
    ws: &mut FeishuWs,
    open_id: &str,
    ctx: &QuestionCtx<'_>,
    preempt: &Preemption,
) -> Option<QuestionAnswer> {
    let body = build_question_text(ctx);
    if let Err(e) = client.send_text(&body).await {
        eprintln!(
            "{}{}",
            i18n::warn_prefix(ctx.lang),
            i18n::tr(ctx.lang, "channel.fsQuestionSendFailed").replace("{e}", &e.to_string())
        );
    }

    while !preempt.is_cancelled() {
        let ev = match tokio::time::timeout(POLL_INTERVAL, ws.recv()).await {
            Ok(Some(ev)) => ev,
            Ok(None) => break,
            Err(_) => continue,
        };
        match ev {
            WsEvent::Message(event) => {
                if event_open_id(&event) != open_id {
                    continue;
                }
                if let Some(answer) = message_to_answer(client, &event, ctx.options).await {
                    return Some(answer);
                }
            }
            // 文本兜底路径不期望卡片回调；空 ACK 确认跳过。
            WsEvent::CardAction { frame, .. } => ws.respond_ack(&frame).await,
        }
    }
    None
}

/// 累积聊天里收到的图片/文件（卡片作答期间）；纯文字等忽略。
async fn accumulate_attachment(
    client: &FeishuClient,
    event: &Value,
    images: &mut Vec<ImageAttachment>,
    files: &mut Vec<String>,
    lang: Lang,
) {
    let Some((msg_type, message_id, content)) = parse_message(event) else {
        return;
    };
    match msg_type.as_str() {
        "image" => {
            let Some(key) = content.get("image_key").and_then(|v| v.as_str()) else {
                return;
            };
            match download_image(client, &message_id, key).await {
                Ok(img) => images.push(img),
                Err(e) => eprintln!(
                    "{}{}",
                    i18n::warn_prefix(lang),
                    i18n::tr(lang, "channel.fsImageDownloadFailed").replace("{e}", &e)
                ),
            }
        }
        "file" => {
            let Some(key) = content.get("file_key").and_then(|v| v.as_str()) else {
                return;
            };
            let file_name = content
                .get("file_name")
                .and_then(|v| v.as_str())
                .unwrap_or("file");
            let ext = ext_of(file_name);
            match client
                .download_resource_to(&message_id, key, "file", ext)
                .await
            {
                Ok(path) => files.push(path),
                Err(e) => eprintln!(
                    "{}{}",
                    i18n::warn_prefix(lang),
                    i18n::tr(lang, "channel.fsFileDownloadFailed").replace("{e}", &e.to_string())
                ),
            }
        }
        _ => {} // 文字 / 富文本 等：卡片模式下忽略（请用卡片输入框）。
    }
}

/// 组装提问正文（纯文本兜底）：头部（加粗）+ 正文 + 编号选项 + 作答提示。
fn build_question_text(ctx: &QuestionCtx<'_>) -> String {
    let mut s = String::new();
    if !ctx.header.is_empty() {
        s.push_str(ctx.header);
        s.push_str("\n\n");
    }
    if !ctx.text.is_empty() {
        s.push_str(ctx.text);
        s.push_str("\n\n");
    }
    if ctx.options.is_empty() {
        s.push_str(i18n::tr(ctx.lang, "channel.ddHintFree"));
    } else {
        for (i, opt) in ctx.options.iter().enumerate() {
            s.push_str(&format!("{}. {}\n", i + 1, opt));
        }
        s.push('\n');
        s.push_str(i18n::tr(ctx.lang, "channel.ddHintOptions"));
    }
    s.trim_end().to_string()
}

/// 把一条用户消息转成回答；非可作答类型返回 None（继续等待）。
async fn message_to_answer(
    client: &FeishuClient,
    event: &Value,
    options: &[String],
) -> Option<QuestionAnswer> {
    let (msg_type, message_id, content) = parse_message(event)?;
    match msg_type.as_str() {
        "text" => {
            let text = content.get("text").and_then(|t| t.as_str()).unwrap_or("").trim();
            if text.is_empty() {
                return None;
            }
            let (selected, user_input) = parse_reply(text, options);
            Some(QuestionAnswer {
                selected_options: selected,
                user_input,
                images: Vec::new(),
                files: Vec::new(),
            })
        }
        "image" => {
            let key = content.get("image_key").and_then(|v| v.as_str())?;
            match download_image(client, &message_id, key).await {
                Ok(img) => Some(QuestionAnswer {
                    selected_options: Vec::new(),
                    user_input: None,
                    images: vec![img],
                    files: Vec::new(),
                }),
                Err(e) => {
                    let lang = Lang::current();
                    eprintln!(
                        "{}{}",
                        i18n::warn_prefix(lang),
                        i18n::tr(lang, "channel.fsImageDownloadFailed").replace("{e}", &e)
                    );
                    None
                }
            }
        }
        "file" => {
            let key = content.get("file_key").and_then(|v| v.as_str())?;
            let file_name = content
                .get("file_name")
                .and_then(|v| v.as_str())
                .unwrap_or("file");
            let ext = ext_of(file_name);
            match client
                .download_resource_to(&message_id, key, "file", ext)
                .await
            {
                Ok(path) => Some(QuestionAnswer {
                    selected_options: Vec::new(),
                    user_input: None,
                    images: Vec::new(),
                    files: vec![path],
                }),
                Err(e) => {
                    let lang = Lang::current();
                    eprintln!(
                        "{}{}",
                        i18n::warn_prefix(lang),
                        i18n::tr(lang, "channel.fsFileDownloadFailed").replace("{e}", &e.to_string())
                    );
                    None
                }
            }
        }
        _ => None,
    }
}

/// 解析文字回复：纯编号（含逗号/空格分隔）→ 映射选项；否则整条作为自由文本。
fn parse_reply(text: &str, options: &[String]) -> (Vec<String>, Option<String>) {
    let trimmed = text.trim();
    let selection_only = !options.is_empty()
        && trimmed.chars().all(|c| {
            c.is_ascii_digit()
                || c.is_whitespace()
                || matches!(c, ',' | '，' | '、' | '/' | ';' | '；' | '.' | '。')
        });
    if selection_only {
        let mut selected: Vec<String> = Vec::new();
        for tok in trimmed.split(|c: char| !c.is_ascii_digit()) {
            if tok.is_empty() {
                continue;
            }
            if let Ok(n) = tok.parse::<usize>() {
                if n >= 1 && n <= options.len() {
                    let opt = options[n - 1].clone();
                    if !selected.contains(&opt) {
                        selected.push(opt);
                    }
                }
            }
        }
        if !selected.is_empty() {
            return (selected, None);
        }
    }
    (Vec::new(), Some(trimmed.to_string()))
}

/// 取消息发送者 open_id。
fn event_open_id(event: &Value) -> &str {
    event
        .get("sender")
        .and_then(|s| s.get("sender_id"))
        .and_then(|i| i.get("open_id"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
}

/// 解析 `event.message`：返回 (message_type, message_id, 解析后的 content)。
fn parse_message(event: &Value) -> Option<(String, String, Value)> {
    let message = event.get("message")?;
    let msg_type = message.get("message_type").and_then(|v| v.as_str())?.to_string();
    let message_id = message
        .get("message_id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let content_str = message.get("content").and_then(|v| v.as_str()).unwrap_or("{}");
    let content: Value = serde_json::from_str(content_str).unwrap_or(Value::Null);
    Some((msg_type, message_id, content))
}

/// 下载图片消息并转为 `ImageAttachment`（raw base64 + media_type）。
async fn download_image(
    client: &FeishuClient,
    message_id: &str,
    image_key: &str,
) -> Result<ImageAttachment, String> {
    use base64::engine::general_purpose::STANDARD as B64;
    use base64::Engine;
    let path = client
        .download_resource_to(message_id, image_key, "image", "png")
        .await
        .map_err(|e| e.to_string())?;
    let bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
    Ok(ImageAttachment {
        data: B64.encode(bytes),
        media_type: "image/png".to_string(),
        filename: None,
    })
}

/// 上传并发送一个附件（图片/文件）。
async fn send_attachment(
    client: &FeishuClient,
    path: &str,
    name: &str,
    is_image: bool,
) -> Result<(), crate::feishu::FeishuError> {
    if is_image {
        let image_key = client.upload_image(path).await?;
        client.send_image(&image_key).await?;
    } else {
        let file_key = client.upload_file(path, name).await?;
        client.send_file(&file_key).await?;
    }
    Ok(())
}

/// 取文件扩展名（无则空串）。
fn ext_of(name: &str) -> &str {
    std::path::Path::new(name)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
}
