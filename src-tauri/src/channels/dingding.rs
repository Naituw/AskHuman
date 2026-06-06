//! 钉钉 Channel：Stream 长连接收（卡片回调 + 用户文字/图片/文件）+ OpenAPI 发（Message、互动卡片）。
//!
//! 方案 A（默认）：提问以**互动卡片高级版**下发，用户在卡片内勾选预定义选项（多选）、可补充文字，
//! 点「提交」完成该题；作答期间在聊天里发的图片/文件会被累积进答案（纯文字忽略，请用卡片输入框）。
//! 卡片回调走 Stream（topic `/v1.0/card/instances/callback`），零公网。
//!
//! 方案 B（兜底）：当卡片投放失败时，回退为「纯文本 + 编号选项」——用户回复一条消息即完成该题
//! （回复编号映射选项，或直接输入文字，或发图片/文件）。
//!
//! 编排逻辑复用 `conversation::run_conversation`，本文件提供传输实现 `DingTalkSession`
//! （`MessagingChannel`）+ 薄外层 `DingTalkChannel`。

use super::conversation::{run_conversation, MessagingChannel, QuestionCtx};
use super::{Channel, Preemption, ResultSink};
use crate::config::DingTalkChannelConfig;
use crate::dingtalk::card;
use crate::dingtalk::client::DingTalkClient;
use crate::dingtalk::stream::{StreamConn, StreamEvent, TOPIC_BOT_MESSAGE, TOPIC_CARD_CALLBACK};
use crate::i18n::{self, Lang};
use crate::models::{ImageAttachment, MessagePrompt, QuestionAnswer};
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Duration;

/// 抢答轮询粒度：每隔此时长检查一次抢答信号。
const POLL_INTERVAL: Duration = Duration::from_secs(1);

/// 内置默认卡片模板 ID（设置项 `cardTemplateId` 留空时使用）。
const DEFAULT_CARD_TEMPLATE_ID: &str = "748d7d3c-232c-4671-a7c4-cce94790d9e1.schema";

/// 取生效的卡片模板 ID：配置非空用配置，否则用内置默认。
fn effective_template_id(config: &DingTalkChannelConfig) -> &str {
    let t = config.card_template_id.trim();
    if t.is_empty() {
        DEFAULT_CARD_TEMPLATE_ID
    } else {
        t
    }
}

/// 薄外层：接 Coordinator（并行抢答），把会话委托给 `run_conversation` + `DingTalkSession`。
pub struct DingTalkChannel {
    config: DingTalkChannelConfig,
    preempt: Arc<Preemption>,
}

impl DingTalkChannel {
    pub fn new(config: DingTalkChannelConfig) -> Self {
        Self {
            config,
            preempt: Arc::new(Preemption::new()),
        }
    }
}

impl Channel for DingTalkChannel {
    fn id(&self) -> &str {
        "dingding"
    }

    fn start(&self, request: &crate::models::AskRequest, sink: ResultSink) {
        let config = self.config.clone();
        let preempt = self.preempt.clone();
        let request = request.clone();
        tauri::async_runtime::spawn(async move {
            let mut session = DingTalkSession::new(config);
            if let Err(e) = session.open().await {
                let lang = Lang::current();
                eprintln!(
                    "{}{}",
                    i18n::warn_prefix(lang),
                    i18n::tr(lang, "channel.ddConfigInvalidSkip").replace("{e}", &e.to_string())
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

/// 传输实现：持有 client 与 Stream 长连接（跨题复用）。
pub struct DingTalkSession {
    config: DingTalkChannelConfig,
    client: Option<DingTalkClient>,
    stream: Option<StreamConn>,
}

impl DingTalkSession {
    pub fn new(config: DingTalkChannelConfig) -> Self {
        Self {
            config,
            client: None,
            stream: None,
        }
    }
}

#[async_trait::async_trait]
impl MessagingChannel for DingTalkSession {
    fn id(&self) -> &str {
        "dingding"
    }

    async fn open(&mut self) -> Result<(), String> {
        let client = DingTalkClient::new(&self.config).map_err(|e| e.to_string())?;
        // 同时订阅 bot 消息（图片/文件累积）+ 卡片回调（提交）两个 topic。
        let stream = StreamConn::connect(
            client.http().clone(),
            self.config.client_id.trim(),
            self.config.client_secret.trim(),
            &[TOPIC_BOT_MESSAGE, TOPIC_CARD_CALLBACK],
        )
        .await
        .map_err(|e| e.to_string())?;
        self.client = Some(client);
        self.stream = Some(stream);
        Ok(())
    }

    async fn send_message_prompt(
        &mut self,
        message: &MessagePrompt,
        is_markdown: bool,
        source: &str,
        lang: Lang,
    ) {
        let Some(client) = self.client.as_ref() else {
            return;
        };
        let header = i18n::tr(lang, "channel.messageFrom").replace("{source}", source);
        let result = if is_markdown {
            let body = if message.text.trim().is_empty() {
                format!("**{}**", header)
            } else {
                format!("**{}**\n\n{}", header, message.text)
            };
            client.send_oto_markdown(&header, &body).await
        } else {
            let body = if message.text.trim().is_empty() {
                header.clone()
            } else {
                format!("{}\n\n{}", header, message.text)
            };
            client.send_oto_text(&body).await
        };
        if let Err(e) = result {
            eprintln!(
                "{}{}",
                i18n::warn_prefix(lang),
                i18n::tr(lang, "channel.ddMessageSendFailed").replace("{e}", &e.to_string())
            );
        }

        // 展示文件：上传媒体后图片→sampleImageMsg，其它→sampleFile。
        for file in &message.files {
            if let Err(e) = send_attachment(client, &file.path, &file.name, file.is_image).await {
                eprintln!(
                    "{}{}",
                    i18n::warn_prefix(lang),
                    i18n::tr(lang, "channel.ddFileSendFailedLog")
                        .replace("{path}", &file.path)
                        .replace("{e}", &e.to_string())
                );
                let _ = client
                    .send_oto_text(
                        &i18n::tr(lang, "channel.fileSendFailed").replace("{name}", &file.name),
                    )
                    .await;
            }
        }
    }

    async fn ask_question(
        &mut self,
        ctx: &QuestionCtx<'_>,
        preempt: &Preemption,
    ) -> Option<QuestionAnswer> {
        // 题首：无则用兜底标题。
        let title = if ctx.header.is_empty() {
            i18n::tr(ctx.lang, "channel.ddTitleFallback")
        } else {
            ctx.header
        };
        let template_id = effective_template_id(&self.config).to_string();
        let out_track_id = uuid::Uuid::new_v4().to_string();
        let param_map = card::build_card_param_map(title, ctx.text, ctx.options);
        let private_param_map = card::build_card_private_map();

        let Self {
            client,
            stream,
            config,
        } = self;
        let client = client.as_ref()?;
        let stream = stream.as_mut()?;
        let user_id = config.user_id.trim().to_string();

        // 1. 投放互动卡片；失败 → 回退纯文本编号方案。
        if let Err(e) = client
            .create_and_deliver_card(&out_track_id, &template_id, param_map, private_param_map)
            .await
        {
            eprintln!(
                "{}{}",
                i18n::warn_prefix(ctx.lang),
                i18n::tr(ctx.lang, "channel.ddCardDeliverFailed").replace("{e}", &e.to_string())
            );
            return ask_question_text(client, stream, &user_id, ctx, preempt).await;
        }

        // 2. 等卡片「提交」；作答期间累积聊天里的图片/文件；被抢答则收尾返回 None。
        let mut images: Vec<ImageAttachment> = Vec::new();
        let mut files: Vec<String> = Vec::new();
        while !preempt.is_cancelled() {
            let ev = match tokio::time::timeout(POLL_INTERVAL, stream.recv()).await {
                Ok(Some(ev)) => ev,
                Ok(None) => break,  // 连接彻底断开
                Err(_) => continue, // 超时：回到循环顶部重新检查 cancelled
            };
            match ev {
                StreamEvent::CardCallback { data, message_id } => {
                    match card::parse_card_submit(&data) {
                        Some(s) if s.out_track_id == out_track_id && s.user_id == user_id => {
                            // 回包置 submitted=true（私有）让卡片灰显，并写终态文案 submit_status（公有）。
                            // 公私分开：私有进 userPrivateData，公有进 cardData，避免整份数据被拒。
                            let submitted_text = i18n::tr(ctx.lang, "channel.ddSubmitted");
                            stream
                                .respond(
                                    &message_id,
                                    json!({
                                        "cardUpdateOptions": {
                                            "updateCardDataByKey": true,
                                            "updatePrivateDataByKey": true,
                                        },
                                        "cardData": { "cardParamMap": { "submit_status": submitted_text } },
                                        "userPrivateData": { "cardParamMap": { "submitted": "true" } },
                                    }),
                                )
                                .await;
                            return Some(QuestionAnswer {
                                selected_options: s.selected_options,
                                user_input: s.user_input,
                                images,
                                files,
                            });
                        }
                        // 非本卡片 / 非提交 → 空回包确认，继续等待。
                        _ => stream.respond(&message_id, json!({})).await,
                    }
                }
                StreamEvent::BotMessage(data) => {
                    if bot_message_belongs(&data, &user_id) {
                        accumulate_attachment(client, &data, &mut images, &mut files, ctx.lang)
                            .await;
                    }
                }
            }
        }

        // 被抢答 / 断连：尽力把卡片置为终态（失败忽略）。被抢答点名赢家「已在X回答」，
        // 断连兜底用「已提交」。
        let status = if preempt.is_cancelled() {
            i18n::tr(ctx.lang, "channel.ddAnsweredVia").replace("{source}", &preempt.winner())
        } else {
            i18n::tr(ctx.lang, "channel.ddSubmitted").to_string()
        };
        let _ = client
            .update_card_private(
                &out_track_id,
                json!({ "submit_status": status }),
                json!({ "submitted": "true" }),
            )
            .await;
        None
    }

    async fn close(&mut self) {
        self.stream = None;
    }
}

/// 兜底：纯文本 + 编号选项问一题（卡片投放失败时使用）。用户回一条消息即完成该题。
async fn ask_question_text(
    client: &DingTalkClient,
    stream: &mut StreamConn,
    user_id: &str,
    ctx: &QuestionCtx<'_>,
    preempt: &Preemption,
) -> Option<QuestionAnswer> {
    let title = if ctx.header.is_empty() {
        i18n::tr(ctx.lang, "channel.ddTitleFallback")
    } else {
        ctx.header
    };
    let body = build_question_text(ctx, ctx.is_markdown);
    let send_res = if ctx.is_markdown {
        client.send_oto_markdown(title, &body).await
    } else {
        client.send_oto_text(&body).await
    };
    if let Err(e) = send_res {
        eprintln!(
            "{}{}",
            i18n::warn_prefix(ctx.lang),
            i18n::tr(ctx.lang, "channel.ddQuestionSendFailed").replace("{e}", &e.to_string())
        );
    }

    while !preempt.is_cancelled() {
        let ev = match tokio::time::timeout(POLL_INTERVAL, stream.recv()).await {
            Ok(Some(ev)) => ev,
            Ok(None) => break,
            Err(_) => continue,
        };
        match ev {
            StreamEvent::BotMessage(data) => {
                if !bot_message_belongs(&data, user_id) {
                    continue;
                }
                if let Some(answer) = message_to_answer(client, &data, ctx.options).await {
                    return Some(answer);
                }
            }
            // 文本兜底路径不期望卡片回调；空回包确认跳过。
            StreamEvent::CardCallback { message_id, .. } => {
                stream.respond(&message_id, json!({})).await;
            }
        }
    }
    None
}

/// 累积聊天里收到的图片/文件（卡片作答期间）；纯文字等忽略。
async fn accumulate_attachment(
    client: &DingTalkClient,
    data: &Value,
    images: &mut Vec<ImageAttachment>,
    files: &mut Vec<String>,
    lang: Lang,
) {
    let msgtype = data.get("msgtype").and_then(|v| v.as_str()).unwrap_or("");
    match msgtype {
        "picture" => {
            let Some(code) = data
                .get("content")
                .and_then(|c| c.get("downloadCode"))
                .and_then(|v| v.as_str())
            else {
                return;
            };
            match download_image(client, code).await {
                Ok(img) => images.push(img),
                Err(e) => eprintln!(
                    "{}{}",
                    i18n::warn_prefix(lang),
                    i18n::tr(lang, "channel.ddImageDownloadFailed").replace("{e}", &e)
                ),
            }
        }
        "file" => {
            let content = data.get("content");
            let Some(code) = content
                .and_then(|c| c.get("downloadCode"))
                .and_then(|v| v.as_str())
            else {
                return;
            };
            let file_name = content
                .and_then(|c| c.get("fileName"))
                .and_then(|v| v.as_str())
                .unwrap_or("file");
            let ext = ext_of(file_name);
            match client.download_message_file_to(code, ext).await {
                Ok(path) => files.push(path),
                Err(e) => eprintln!(
                    "{}{}",
                    i18n::warn_prefix(lang),
                    i18n::tr(lang, "channel.ddFileDownloadFailed").replace("{e}", &e.to_string())
                ),
            }
        }
        _ => {} // 文字 / 贴纸等：卡片模式下忽略（请用卡片输入框）。
    }
}

/// 组装提问正文：头部（加粗）+ 正文 + 编号选项 + 作答提示。
fn build_question_text(ctx: &QuestionCtx<'_>, is_markdown: bool) -> String {
    let mut s = String::new();
    if !ctx.header.is_empty() {
        if is_markdown {
            s.push_str(&format!("**{}**", ctx.header));
        } else {
            s.push_str(ctx.header);
        }
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

/// 把一条 bot 消息转成回答；非可作答类型返回 None（继续等待）。
async fn message_to_answer(
    client: &DingTalkClient,
    data: &Value,
    options: &[String],
) -> Option<QuestionAnswer> {
    let msgtype = data.get("msgtype").and_then(|v| v.as_str()).unwrap_or("");
    match msgtype {
        "text" => {
            let content = data
                .get("text")
                .and_then(|t| t.get("content"))
                .and_then(|c| c.as_str())
                .unwrap_or("")
                .trim();
            if content.is_empty() {
                return None;
            }
            let (selected, user_input) = parse_reply(content, options);
            Some(QuestionAnswer {
                selected_options: selected,
                user_input,
                images: Vec::new(),
                files: Vec::new(),
            })
        }
        "picture" => {
            let code = data
                .get("content")
                .and_then(|c| c.get("downloadCode"))
                .and_then(|v| v.as_str())?;
            match download_image(client, code).await {
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
                        i18n::tr(lang, "channel.ddImageDownloadFailed").replace("{e}", &e)
                    );
                    None
                }
            }
        }
        "file" => {
            let content = data.get("content");
            let code = content
                .and_then(|c| c.get("downloadCode"))
                .and_then(|v| v.as_str())?;
            let file_name = content
                .and_then(|c| c.get("fileName"))
                .and_then(|v| v.as_str())
                .unwrap_or("file");
            let ext = ext_of(file_name);
            match client.download_message_file_to(code, ext).await {
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
                        i18n::tr(lang, "channel.ddFileDownloadFailed").replace("{e}", &e.to_string())
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

/// 判定 bot 消息是否来自目标用户。
fn bot_message_belongs(data: &Value, user_id: &str) -> bool {
    data.get("senderStaffId").and_then(|v| v.as_str()).unwrap_or("") == user_id
}

/// 下载图片消息并转为 `ImageAttachment`（raw base64 + media_type）。
async fn download_image(client: &DingTalkClient, code: &str) -> Result<ImageAttachment, String> {
    use base64::engine::general_purpose::STANDARD as B64;
    use base64::Engine;
    let path = client
        .download_message_file_to(code, "png")
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
    client: &DingTalkClient,
    path: &str,
    name: &str,
    is_image: bool,
) -> Result<(), crate::dingtalk::DingTalkError> {
    if is_image {
        let media_id = client.upload_media(path, "image").await?;
        client.send_oto_image(&media_id).await
    } else {
        let media_id = client.upload_media(path, "file").await?;
        let ext = ext_of(name);
        client.send_oto_file(&media_id, name, ext).await
    }
}

/// 取文件扩展名（无则空串）。
fn ext_of(name: &str) -> &str {
    std::path::Path::new(name)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
}
