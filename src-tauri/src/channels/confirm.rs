//! Structured confirmation sessions for interactive IM surfaces.

use crate::app::confirm_coordinator::ConfirmTerminalKind;
use crate::confirm::choice_cards::{self, CardAction};
use crate::daemon::request::ConfirmEntry;
use crate::i18n::{self, Lang};
use crate::models::ConfirmFallbackReason;
use std::sync::Arc;
use std::time::Duration;

const DELIVERY_TIMEOUT: Duration = Duration::from_secs(60);
const DEFAULT_DINGTALK_PERMISSION_TEMPLATE_ID: &str = "3a5ce2de-99b8-4a79-a4ea-622897526645.schema";

fn fail(entry: &ConfirmEntry, channel: &str, reason: impl Into<String>) {
    if entry.mark_failed(channel, reason) {
        entry
            .coordinator
            .fallback(ConfirmFallbackReason::NoAvailableChannel);
        entry.cancel.notify_waiters();
    }
}

fn source_name(channel: &str, lang: Lang) -> String {
    match channel {
        "popup" => i18n::tr(lang, "channel.sourcePopup"),
        "feishu" => i18n::tr(lang, "channel.sourceFeishu"),
        "dingding" => i18n::tr(lang, "channel.sourceDingTalk"),
        "telegram" => i18n::tr(lang, "channel.sourceTelegram"),
        "slack" => i18n::tr(lang, "channel.sourceSlack"),
        other => other,
    }
    .to_string()
}

fn final_status(entry: &ConfirmEntry, lang: Lang) -> String {
    match entry.coordinator.terminal_kind() {
        Some(ConfirmTerminalKind::Decision(result)) => {
            let denied = entry
                .request
                .choices
                .iter()
                .find(|choice| choice.id == result.action_id)
                .map(|choice| choice.role == crate::confirm::ActionRole::Destructive)
                .unwrap_or(false);
            let source = source_name(&result.source_channel_id, lang);
            match (lang, denied) {
                (Lang::Zh, true) => format!("已通过 {source} 提交拒绝决定"),
                (Lang::Zh, false) => format!("已通过 {source} 允许"),
                (Lang::En, true) => format!("Denial decision submitted via {source}"),
                (Lang::En, false) => format!("Allowed via {source}"),
            }
        }
        Some(ConfirmTerminalKind::Fallback(ConfirmFallbackReason::Expired)) => match lang {
            Lang::Zh => "请求已过期".to_string(),
            Lang::En => "Request expired".to_string(),
        },
        Some(ConfirmTerminalKind::Fallback(_)) => match lang {
            Lang::Zh => "请求已失效".to_string(),
            Lang::En => "Request is no longer available".to_string(),
        },
        Some(ConfirmTerminalKind::Cancelled) => match lang {
            Lang::Zh => "请求已取消".to_string(),
            Lang::En => "Request cancelled".to_string(),
        },
        None => match lang {
            Lang::Zh => "渠道已失效".to_string(),
            Lang::En => "Channel unavailable".to_string(),
        },
    }
}

async fn keep_feishu_tombstone(
    mut events: crate::feishu::router::RoutedFs,
    client: crate::feishu::client::FeishuClient,
    message_id: String,
    target: String,
    final_card: serde_json::Value,
    deadline: tokio::time::Instant,
) {
    events.clear_loose(&target);
    loop {
        tokio::select! {
            _ = tokio::time::sleep_until(deadline) => break,
            inbound = events.recv() => match inbound {
                Some(crate::feishu::router::FsInbound::Card { data, ack }) => {
                    let actor = data.get("operator").and_then(|v| v.get("open_id")).and_then(serde_json::Value::as_str);
                    let mid = data.get("context").and_then(|v| v.get("open_message_id")).and_then(serde_json::Value::as_str);
                    if actor == Some(target.as_str()) && mid == Some(message_id.as_str()) {
                        let _ = ack.send(Some(crate::feishu::card::callback_update_card(final_card.clone())));
                        let _ = client.patch_card(&message_id, &final_card).await;
                    } else {
                        let _ = ack.send(None);
                    }
                }
                Some(_) => {}
                None => break,
            }
        }
    }
    events.clear_active(Some(&message_id), &target);
}

async fn keep_slack_tombstone(
    mut events: crate::slack::router::RoutedSl,
    client: crate::slack::client::SlackClient,
    dm: String,
    message_id: String,
    target: String,
    title: String,
    final_blocks: serde_json::Value,
    deadline: tokio::time::Instant,
) {
    events.clear_loose(&target);
    loop {
        tokio::select! {
            _ = tokio::time::sleep_until(deadline) => break,
            inbound = events.recv() => match inbound {
                Some(crate::slack::router::SlInbound::Interactive(payload)) => {
                    let actor = payload.get("user").and_then(|v| v.get("id")).and_then(serde_json::Value::as_str);
                    let mid = payload.get("container").and_then(|v| v.get("message_ts")).and_then(serde_json::Value::as_str)
                        .or_else(|| payload.get("message").and_then(|v| v.get("ts")).and_then(serde_json::Value::as_str));
                    if actor == Some(target.as_str()) && mid == Some(message_id.as_str()) {
                        let _ = client.update_message(&dm, &message_id, Some(&final_blocks), &title).await;
                    }
                }
                Some(_) => {}
                None => break,
            }
        }
    }
    events.clear_active(Some(&message_id), &target);
}

async fn keep_telegram_tombstone(
    mut events: crate::telegram::router::RoutedTg,
    client: crate::telegram::TelegramClient,
    message_id: i64,
    final_html: String,
    deadline: tokio::time::Instant,
) {
    events.clear_loose();
    loop {
        tokio::select! {
            _ = tokio::time::sleep_until(deadline) => break,
            inbound = events.recv() => match inbound {
                Some(crate::telegram::router::TgInbound::Callback(callback)) => {
                    if let Some(id) = callback.get("id").and_then(serde_json::Value::as_str) {
                        client.answer_callback_query(id).await;
                    }
                    let _ = client.edit_message_text(message_id, &final_html, Some("HTML"), None).await;
                }
                Some(_) => {}
                None => break,
            }
        }
    }
    events.clear_active(message_id);
}

async fn keep_dingtalk_tombstone(
    mut events: crate::dingtalk::router::RoutedDd,
    client: crate::dingtalk::client::DingTalkClient,
    out_track_id: String,
    target: String,
    status: String,
    deadline: tokio::time::Instant,
) {
    events.clear_loose(&target);
    loop {
        tokio::select! {
            _ = tokio::time::sleep_until(deadline) => break,
            inbound = events.recv() => match inbound {
                Some(crate::dingtalk::router::DdInbound::Card { data, ack }) => {
                    let submit = crate::dingtalk::card::parse_card_submit(&data);
                    if submit.as_ref().is_some_and(|submit| submit.user_id == target && submit.out_track_id == out_track_id) {
                        let _ = ack.send(crate::dingtalk::card::submit_ack_success());
                        let _ = client.update_card_private(
                            &out_track_id,
                            serde_json::json!({ "submit_status": status }),
                            serde_json::json!({ "submitted": "true" }),
                        ).await;
                    } else {
                        let _ = ack.send(serde_json::json!({}));
                    }
                }
                Some(_) => {}
                None => break,
            }
        }
    }
    events.clear_active(Some(&out_track_id), &target);
}

fn dingtalk_param_map(request: &crate::models::ConfirmRequest, lang: Lang) -> serde_json::Value {
    let options: Vec<crate::models::OptionItem> = request
        .choices
        .iter()
        .map(|choice| {
            let text = if choice.description.trim().is_empty() {
                choice.label.clone()
            } else {
                format!("{}\n{}", choice.label, choice.description)
            };
            crate::models::OptionItem::new(text, false)
        })
        .collect();
    let mut public = crate::dingtalk::card::build_card_param_map(
        &request.title,
        &choice_cards::compact_tool_markdown(request, 12_000, lang),
        &options,
        true,
        false,
        if lang == Lang::Zh {
            "【👍推荐】"
        } else {
            "[Recommended]"
        },
    );
    if let Some(map) = public.as_object_mut() {
        map.remove("single");
        map.remove("allow_input");
    }
    public["deny_index"] = serde_json::Value::String(request.dismiss_index().to_string());
    public["reason_label"] = serde_json::Value::String(
        if lang == Lang::Zh {
            "拒绝原因（可选）"
        } else {
            "Denial reason (optional)"
        }
        .to_string(),
    );
    public["reason_placeholder"] = serde_json::Value::String(
        if lang == Lang::Zh {
            "告诉 Agent 应该怎么做"
        } else {
            "Tell the Agent what it should do"
        }
        .to_string(),
    );
    public["submit_label"] =
        serde_json::Value::String(request.presentation.submit_label().to_string());
    public
}

pub fn start_dingtalk(
    entry: Arc<ConfirmEntry>,
    config: crate::config::DingTalkChannelConfig,
    router: Arc<crate::dingtalk::router::DdRouter>,
) {
    tokio::spawn(async move {
        let channel = "dingding";
        let lang = Lang::resolve(&entry.lang);
        let client = match crate::dingtalk::client::DingTalkClient::new(&config) {
            Ok(client) => client,
            Err(error) => {
                fail(&entry, channel, error.to_string());
                return;
            }
        };
        let target = client.user_id().to_string();
        let template = config.permission_confirm_card_template_id.trim();
        let template = if template.is_empty() {
            DEFAULT_DINGTALK_PERMISSION_TEMPLATE_ID
        } else {
            template
        };
        let public = dingtalk_param_map(&entry.request, lang);
        let private = crate::dingtalk::card::build_card_private_map();
        let out_track_id = format!("permission-{}", uuid::Uuid::new_v4());
        let mut events = router.register();
        match tokio::time::timeout(
            DELIVERY_TIMEOUT,
            client.create_and_deliver_card(&out_track_id, template, public, private),
        )
        .await
        {
            Ok(Ok(())) => {}
            Ok(Err(error)) => {
                fail(&entry, channel, error.to_string());
                return;
            }
            Err(_) => {
                fail(&entry, channel, "DingTalk delivery timed out");
                return;
            }
        }
        events.set_active(Some(&out_track_id), &target);
        if !entry.mark_ready(channel, out_track_id.clone()) {
            let status = final_status(&entry, lang);
            let _ = client
                .update_card_private(
                    &out_track_id,
                    serde_json::json!({ "submit_status": status }),
                    serde_json::json!({ "submitted": "true" }),
                )
                .await;
            let deadline = entry.deadline;
            drop(entry);
            keep_dingtalk_tombstone(events, client, out_track_id, target, status, deadline).await;
            return;
        }
        let mut disconnected = false;
        loop {
            tokio::select! {
                _ = entry.cancel.notified() => break,
                inbound = events.recv() => match inbound {
                    Some(crate::dingtalk::router::DdInbound::Card { data, ack }) => {
                        let Some(submit) = crate::dingtalk::card::parse_card_submit(&data) else {
                            let _ = ack.send(serde_json::json!({}));
                            continue;
                        };
                        if submit.user_id != target || submit.out_track_id != out_track_id || submit.selected_indices.len() != 1 {
                            let _ = ack.send(serde_json::json!({}));
                            continue;
                        }
                        let index = submit.selected_indices[0];
                        if entry.coordinator.submit_wire(index, submit.user_input, channel).is_ok() {
                            let _ = ack.send(crate::dingtalk::card::submit_ack_success());
                            break;
                        }
                        let _ = ack.send(serde_json::json!({}));
                    }
                    Some(_) => {}
                    None => { disconnected = true; break; }
                }
            }
        }
        if disconnected {
            fail(&entry, channel, "DingTalk router disconnected");
        }
        let status = final_status(&entry, lang);
        let _ = client
            .update_card_private(
                &out_track_id,
                serde_json::json!({ "submit_status": status }),
                serde_json::json!({ "submitted": "true" }),
            )
            .await;
        let deadline = entry.deadline;
        drop(entry);
        keep_dingtalk_tombstone(events, client, out_track_id, target, status, deadline).await;
    });
}

pub fn start_feishu(
    entry: Arc<ConfirmEntry>,
    config: crate::config::FeishuChannelConfig,
    router: Arc<crate::feishu::router::FsRouter>,
) {
    tokio::spawn(async move {
        let channel = "feishu";
        let lang = Lang::resolve(&entry.lang);
        let client = match crate::feishu::client::FeishuClient::new(&config) {
            Ok(client) if !client.open_id().is_empty() => client,
            Ok(_) => {
                fail(&entry, channel, "missing target open_id");
                return;
            }
            Err(error) => {
                fail(&entry, channel, error.to_string());
                return;
            }
        };
        let target = client.open_id().to_string();
        let mut events = router.register();
        let mut selected = None;
        let mut comment = String::new();
        let initial = choice_cards::feishu_card(&entry.request, selected, &comment, lang);
        let message_id =
            match tokio::time::timeout(DELIVERY_TIMEOUT, client.send_card(&initial)).await {
                Ok(Ok(message_id)) if !message_id.is_empty() => message_id,
                Ok(Ok(_)) => {
                    fail(&entry, channel, "empty Feishu message id");
                    return;
                }
                Ok(Err(error)) => {
                    fail(&entry, channel, error.to_string());
                    return;
                }
                Err(_) => {
                    fail(&entry, channel, "Feishu delivery timed out");
                    return;
                }
            };
        events.set_active(Some(&message_id), &target);
        if !entry.mark_ready(channel, message_id.clone()) {
            let final_card =
                choice_cards::feishu_final_card(&entry.request, &final_status(&entry, lang), lang);
            let _ = client.patch_card(&message_id, &final_card).await;
            let deadline = entry.deadline;
            drop(entry);
            keep_feishu_tombstone(events, client, message_id, target, final_card, deadline).await;
            return;
        }

        let mut disconnected = false;
        loop {
            tokio::select! {
                _ = entry.cancel.notified() => break,
                inbound = events.recv() => match inbound {
                    Some(crate::feishu::router::FsInbound::Card { data, ack }) => {
                        let input_id = entry.request.presentation.input().map(|input| input.id.as_str());
                        match choice_cards::parse_feishu_action(&data, input_id) {
                            Some(CardAction::Select { actor, message_id: mid, index, comment: draft })
                                if actor == target && mid == message_id && index < entry.request.choices.len() =>
                            {
                                if let Some(draft) = draft { comment = draft; }
                                selected = Some(index);
                                let card = choice_cards::feishu_card(&entry.request, selected, &comment, lang);
                                let _ = ack.send(Some(crate::feishu::card::callback_update_card(card)));
                            }
                            Some(CardAction::Submit { actor, message_id: mid, index: _, comment: submitted })
                                if actor == target && mid == message_id =>
                            {
                                let Some(index) = selected else {
                                    let _ = ack.send(None);
                                    continue;
                                };
                                if let Some(value) = submitted {
                                    comment = value;
                                }
                                match entry.coordinator.submit_wire(index, Some(comment.clone()), channel) {
                                    Ok(_) => {
                                        let final_card = choice_cards::feishu_final_card(&entry.request, &final_status(&entry, lang), lang);
                                        let _ = ack.send(Some(crate::feishu::card::callback_update_card(final_card)));
                                        break;
                                    }
                                    Err(_) => {
                                        let card = choice_cards::feishu_card(&entry.request, selected, &comment, lang);
                                        let _ = ack.send(Some(crate::feishu::card::callback_update_card(card)));
                                    }
                                }
                            }
                            _ => { let _ = ack.send(None); }
                        }
                    }
                    Some(_) => {}
                    None => { disconnected = true; break; }
                }
            }
        }
        if disconnected {
            fail(&entry, channel, "Feishu router disconnected");
        }
        let final_card =
            choice_cards::feishu_final_card(&entry.request, &final_status(&entry, lang), lang);
        let _ = client.patch_card(&message_id, &final_card).await;
        let deadline = entry.deadline;
        drop(entry);
        keep_feishu_tombstone(events, client, message_id, target, final_card, deadline).await;
    });
}

pub fn start_slack(
    entry: Arc<ConfirmEntry>,
    config: crate::config::SlackChannelConfig,
    router: Arc<crate::slack::router::SlRouter>,
) {
    tokio::spawn(async move {
        let channel = "slack";
        let lang = Lang::resolve(&entry.lang);
        let client = match crate::slack::client::SlackClient::new(&config) {
            Ok(client) if !client.user_id().is_empty() => client,
            Ok(_) => {
                fail(&entry, channel, "missing target Slack user");
                return;
            }
            Err(error) => {
                fail(&entry, channel, error.to_string());
                return;
            }
        };
        let target = client.user_id().to_string();
        let dm = match tokio::time::timeout(DELIVERY_TIMEOUT, client.open_dm()).await {
            Ok(Ok(dm)) => dm,
            Ok(Err(error)) => {
                fail(&entry, channel, error.to_string());
                return;
            }
            Err(_) => {
                fail(&entry, channel, "Slack DM lookup timed out");
                return;
            }
        };
        let mut selected = None;
        let mut comment = String::new();
        let mut events = router.register();
        let initial = choice_cards::slack_blocks(&entry.request, selected, &comment, lang);
        let message_id = match tokio::time::timeout(
            DELIVERY_TIMEOUT,
            client.post_message(&dm, Some(&initial), &entry.request.title),
        )
        .await
        {
            Ok(Ok(message_id)) if !message_id.is_empty() => message_id,
            Ok(Ok(_)) => {
                fail(&entry, channel, "empty Slack message ts");
                return;
            }
            Ok(Err(error)) => {
                fail(&entry, channel, error.to_string());
                return;
            }
            Err(_) => {
                fail(&entry, channel, "Slack delivery timed out");
                return;
            }
        };
        events.set_active(Some(&message_id), &target);
        if !entry.mark_ready(channel, message_id.clone()) {
            let blocks =
                choice_cards::slack_final_blocks(&entry.request, &final_status(&entry, lang), lang);
            let _ = client
                .update_message(&dm, &message_id, Some(&blocks), &entry.request.title)
                .await;
            let deadline = entry.deadline;
            let title = entry.request.title.clone();
            drop(entry);
            keep_slack_tombstone(
                events, client, dm, message_id, target, title, blocks, deadline,
            )
            .await;
            return;
        }

        let mut disconnected = false;
        loop {
            tokio::select! {
                _ = entry.cancel.notified() => break,
                inbound = events.recv() => match inbound {
                    Some(crate::slack::router::SlInbound::Interactive(payload)) => {
                        let input_id = entry.request.presentation.input().map(|input| input.id.as_str());
                        match choice_cards::parse_slack_action(&payload, input_id) {
                            Some(CardAction::Select { actor, message_id: mid, index, comment: draft })
                                if actor == target && mid == message_id && index < entry.request.choices.len() =>
                            {
                                if let Some(draft) = draft { comment = draft; }
                                selected = Some(index);
                                let blocks = choice_cards::slack_blocks(&entry.request, selected, &comment, lang);
                                let _ = client.update_message(&dm, &message_id, Some(&blocks), &entry.request.title).await;
                            }
                            Some(CardAction::Submit { actor, message_id: mid, index: submitted_index, comment: submitted })
                                if actor == target && mid == message_id =>
                            {
                                let Some(index) = submitted_index.or(selected) else { continue; };
                                selected = Some(index);
                                if let Some(value) = submitted { comment = value; }
                                if entry.coordinator.submit_wire(index, Some(comment.clone()), channel).is_ok() {
                                    break;
                                }
                            }
                            _ => {}
                        }
                    }
                    Some(crate::slack::router::SlInbound::Message(event)) => {
                        let actor = event.get("user").and_then(|value| value.as_str()).unwrap_or("");
                        let thread = event.get("thread_ts").and_then(|value| value.as_str()).unwrap_or("");
                        let text = event.get("text").and_then(|value| value.as_str()).unwrap_or("").trim();
                        if actor == target && thread == message_id && !text.is_empty() {
                            let deny = entry.request.dismiss_index();
                            let extra = usize::from(!comment.is_empty());
                            if comment.chars().count() + extra + text.chars().count() <= 1000 {
                                if !comment.is_empty() { comment.push('\n'); }
                                comment.push_str(text);
                                selected = Some(deny);
                                let blocks = choice_cards::slack_blocks(&entry.request, selected, &comment, lang);
                                let _ = client.update_message(&dm, &message_id, Some(&blocks), &entry.request.title).await;
                            } else {
                                let warning = if lang == Lang::Zh { "拒绝原因最多 1000 字；本条回复未保存。" } else { "The denial reason is limited to 1000 characters; this reply was not saved." };
                                let _ = client.post_thread_text(&dm, &message_id, warning).await;
                            }
                        }
                    }
                    None => { disconnected = true; break; }
                }
            }
        }
        if disconnected {
            fail(&entry, channel, "Slack router disconnected");
        }
        let blocks =
            choice_cards::slack_final_blocks(&entry.request, &final_status(&entry, lang), lang);
        let _ = client
            .update_message(&dm, &message_id, Some(&blocks), &entry.request.title)
            .await;
        let deadline = entry.deadline;
        let title = entry.request.title.clone();
        drop(entry);
        keep_slack_tombstone(
            events, client, dm, message_id, target, title, blocks, deadline,
        )
        .await;
    });
}

pub fn start_telegram(
    entry: Arc<ConfirmEntry>,
    config: crate::config::TelegramChannelConfig,
    router: Arc<crate::telegram::router::TgRouter>,
) {
    tokio::spawn(async move {
        let channel = "telegram";
        let lang = Lang::resolve(&entry.lang);
        let client = match crate::telegram::TelegramClient::new(
            config.bot_token,
            config.chat_id,
            config.api_base_url,
        ) {
            Ok(client) => client,
            Err(error) => {
                fail(&entry, channel, error.to_string());
                return;
            }
        };
        let mut selected = None;
        let mut comment = String::new();
        let mut events = router.register();
        let initial = choice_cards::telegram_html(&entry.request, selected, &comment, None, lang);
        let keyboard = choice_cards::telegram_keyboard(&entry.request, selected);
        let message_id = match tokio::time::timeout(
            DELIVERY_TIMEOUT,
            client.send_message(&initial, Some("HTML"), Some(keyboard)),
        )
        .await
        {
            Ok(Ok(message_id)) if message_id != 0 => message_id,
            Ok(Ok(_)) => {
                fail(&entry, channel, "empty Telegram message id");
                return;
            }
            Ok(Err(error)) => {
                fail(&entry, channel, error.to_string());
                return;
            }
            Err(_) => {
                fail(&entry, channel, "Telegram delivery timed out");
                return;
            }
        };
        events.set_active(client.chat_id(), message_id);
        if !entry.mark_ready(channel, message_id.to_string()) {
            let html = choice_cards::telegram_html(
                &entry.request,
                None,
                &comment,
                Some(&final_status(&entry, lang)),
                lang,
            );
            let _ = client
                .edit_message_text(message_id, &html, Some("HTML"), None)
                .await;
            let deadline = entry.deadline;
            drop(entry);
            keep_telegram_tombstone(events, client, message_id, html, deadline).await;
            return;
        }

        let mut disconnected = false;
        loop {
            tokio::select! {
                _ = entry.cancel.notified() => break,
                inbound = events.recv() => match inbound {
                    Some(crate::telegram::router::TgInbound::Callback(callback)) => {
                        let callback_id = callback.get("id").and_then(|value| value.as_str()).unwrap_or("");
                        let data = callback.get("data").and_then(|value| value.as_str()).unwrap_or("");
                        match choice_cards::parse_telegram_callback(data) {
                            Some(choice_cards::TelegramAction::Decide(index)) if index < entry.request.choices.len() => {
                                selected = Some(index);
                                client.answer_callback_query(callback_id).await;
                                if entry.coordinator.submit_wire(index, Some(comment.clone()), channel).is_ok() { break; }
                            }
                            None => client.answer_callback_query(callback_id).await,
                            _ => client.answer_callback_query(callback_id).await,
                        }
                    }
                    Some(crate::telegram::router::TgInbound::Text { text, reply_to_message_id, .. }) => {
                        if reply_to_message_id == Some(message_id) {
                            let text = text.trim();
                            let extra = usize::from(!comment.is_empty());
                            if !text.is_empty() && comment.chars().count() + extra + text.chars().count() <= 1000 {
                                if !comment.is_empty() { comment.push('\n'); }
                                comment.push_str(text);
                                selected = Some(entry.request.dismiss_index());
                                let keyboard = choice_cards::telegram_keyboard(&entry.request, selected);
                                let html = choice_cards::telegram_html(&entry.request, selected, &comment, None, lang);
                                let _ = client.edit_message_text(message_id, &html, Some("HTML"), Some(keyboard)).await;
                            } else if !text.is_empty() {
                                let warning = if lang == Lang::Zh { "拒绝原因最多 1000 字；本条回复未保存。" } else { "The denial reason is limited to 1000 characters; this reply was not saved." };
                                let _ = client.send_reply_message(message_id, warning).await;
                            }
                        }
                    }
                    None => { disconnected = true; break; }
                }
            }
        }
        if disconnected {
            fail(&entry, channel, "Telegram router stopped");
        }
        let html = choice_cards::telegram_html(
            &entry.request,
            selected,
            &comment,
            Some(&final_status(&entry, lang)),
            lang,
        );
        let _ = client
            .edit_message_text(message_id, &html, Some("HTML"), None)
            .await;
        let deadline = entry.deadline;
        drop(entry);
        keep_telegram_tombstone(events, client, message_id, html, deadline).await;
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{
        ConfirmChoice, ConfirmDetail, ConfirmField, ConfirmFieldKind, ConfirmPresentation,
        ConfirmSpec,
    };

    #[test]
    fn channel_names_are_localized_for_terminal_copy() {
        assert!(!source_name("popup", Lang::En).is_empty());
        assert!(!source_name("feishu", Lang::Zh).is_empty());
    }

    #[test]
    fn dingtalk_permission_payload_matches_dedicated_template_contract() {
        let request = ConfirmSpec {
            title: "Permission".into(),
            context: vec![
                ConfirmField {
                    id: "agent".into(),
                    label: "Agent".into(),
                    value: "Codex".into(),
                    kind: ConfirmFieldKind::Text,
                },
                ConfirmField {
                    id: "tool".into(),
                    label: "Tool".into(),
                    value: "Bash".into(),
                    kind: ConfirmFieldKind::Text,
                },
            ],
            detail: ConfirmDetail {
                summary: "Run command".into(),
                body_md: "`git status`".into(),
            },
            choices: vec![
                ConfirmChoice {
                    id: "approve_once".into(),
                    label: "Approve once".into(),
                    description: String::new(),
                    role: crate::confirm::ActionRole::Primary,
                },
                ConfirmChoice {
                    id: "permission_suggestion_0".into(),
                    label: "Update permission".into(),
                    description: "Session".into(),
                    role: crate::confirm::ActionRole::Default,
                },
                ConfirmChoice {
                    id: "deny".into(),
                    label: "Deny".into(),
                    description: String::new(),
                    role: crate::confirm::ActionRole::Destructive,
                },
            ],
            presentation: ConfirmPresentation::SingleSelectSubmit {
                input: None,
                submit_label: "Submit".into(),
                default_action_id: None,
            },
            dismiss_action_id: "deny".into(),
        }
        .into_request("r".into(), 1, 2)
        .unwrap();
        let payload = dingtalk_param_map(&request, Lang::En);
        assert_eq!(payload["deny_index"], "2");
        assert_eq!(payload["submit_label"], "Submit");
        assert_eq!(
            payload["reason_placeholder"],
            "Tell the Agent what it should do"
        );
        let markdown = payload["markdown"].as_str().unwrap();
        let reason = markdown.find("**Reason:** Run command").unwrap();
        let tool = markdown.find("**Bash**").unwrap();
        let body = markdown.find("`git status`").unwrap();
        assert!(reason < tool && tool < body);
        assert!(!markdown.contains("Codex"));
        assert!(!markdown.contains("**Agent:**"));
        assert!(payload.get("single").is_none());
        assert!(payload.get("allow_input").is_none());
        let options: serde_json::Value =
            serde_json::from_str(payload["options"].as_str().unwrap()).unwrap();
        assert_eq!(options.as_array().unwrap().len(), 3);
        assert!(options.as_array().unwrap().iter().all(|option| option["md"]
            .as_str()
            .is_some_and(|text| !text.contains("Recommended"))));
    }

    #[test]
    fn dingtalk_permission_uses_published_dedicated_template() {
        assert_eq!(
            DEFAULT_DINGTALK_PERMISSION_TEMPLATE_ID,
            "3a5ce2de-99b8-4a79-a4ea-622897526645.schema"
        );
    }
}
