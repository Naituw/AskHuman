//! Channel-agnostic confirm card send/finalize transport.
//!
//! Dispatches to the four IM channel adapters (feishu/dingtalk/telegram/slack) for
//! sending and finalizing double-action confirmation cards.  Callers pass the generic
//! `ConfirmView` / `ConfirmFinalView` and a `channel_id` string; this module handles
//! the per-platform rendering and API calls.

use crate::config::AppConfig;
use crate::confirm::{ConfirmFinalView, ConfirmView};
use serde_json::Value;

/// Feishu callback ack sender type (used when finalizing via card callback response).
pub type FsAck = tokio::sync::oneshot::Sender<Option<Value>>;

/// Send a confirm card to the given channel.  Returns the platform message ID on success.
pub async fn send(channel_id: &str, config: &AppConfig, view: &ConfirmView) -> Option<String> {
    match channel_id {
        "feishu" => {
            let client = crate::feishu::client::FeishuClient::new(&config.channels.feishu).ok()?;
            let card = crate::feishu::card::build_confirm_card(view);
            client.send_card(&card).await.ok()
        }
        "dingding" => {
            let client =
                crate::dingtalk::client::DingTalkClient::new(&config.channels.dingding).ok()?;
            let otid = uuid::Uuid::new_v4().to_string();
            let map = crate::dingtalk::confirm::build_param_map(view);
            let private = serde_json::json!({});
            let tpl = {
                let t = config.channels.dingding.confirm_card_template_id.trim();
                if t.is_empty() {
                    crate::dingtalk::confirm::DEFAULT_CONFIRM_CARD_TEMPLATE_ID
                } else {
                    t
                }
            };
            client
                .create_and_deliver_card(&otid, tpl, map, private)
                .await
                .ok()?;
            Some(otid)
        }
        "telegram" => {
            let tg = &config.channels.telegram;
            let client = crate::telegram::TelegramClient::new(
                tg.bot_token.clone(),
                tg.chat_id.clone(),
                tg.api_base_url.clone(),
            )
            .ok()?;
            let html = crate::telegram::confirm::build_html(view);
            let markup = crate::telegram::confirm::inline_keyboard(view);
            client
                .send_message(&html, Some("HTML"), Some(markup))
                .await
                .ok()
                .map(|mid| mid.to_string())
        }
        "slack" => {
            let client = crate::slack::client::SlackClient::new(&config.channels.slack).ok()?;
            let dm = client.open_dm().await.ok()?;
            let (blocks, fallback) = crate::slack::confirm::build_blocks(view);
            client
                .post_message(&dm, Some(&blocks), &fallback)
                .await
                .ok()
        }
        _ => None,
    }
}

/// Finalize (update) a confirm card to its terminal state.
///
/// `ack` is an optional feishu callback oneshot — if present, the finalized card is
/// sent as the callback response (avoiding a separate PATCH).  For other channels or
/// when `ack` is None, we use the platform's edit/update API.
pub async fn finalize(
    channel_id: &str,
    config: &AppConfig,
    message_id: &str,
    final_view: &ConfirmFinalView,
    ack: Option<FsAck>,
) {
    match channel_id {
        "feishu" => {
            let card = crate::feishu::card::build_confirm_final_card(
                &final_view.title,
                &final_view.body,
                &final_view.label,
            );
            if let Some(ack) = ack {
                let _ = ack.send(Some(crate::feishu::card::callback_update_card(card)));
            } else if let Ok(client) =
                crate::feishu::client::FeishuClient::new(&config.channels.feishu)
            {
                let _ = client.patch_card(message_id, &card).await;
            }
        }
        "telegram" => {
            let tg = &config.channels.telegram;
            if let (Ok(client), Ok(mid_i)) = (
                crate::telegram::TelegramClient::new(
                    tg.bot_token.clone(),
                    tg.chat_id.clone(),
                    tg.api_base_url.clone(),
                ),
                message_id.parse::<i64>(),
            ) {
                let html = format!(
                    "<b>{}</b>\n{}",
                    crate::telegram::markdown::escape_html(&final_view.title),
                    crate::telegram::markdown::escape_html(&final_view.body)
                );
                let _ = client
                    .edit_message_text(mid_i, &html, Some("HTML"), None)
                    .await;
            }
        }
        "slack" => {
            if let Ok(client) = crate::slack::client::SlackClient::new(&config.channels.slack) {
                if let Ok(dm) = client.open_dm().await {
                    let (blocks, fallback) = crate::slack::confirm::build_final_blocks(
                        &final_view.title,
                        &final_view.body,
                    );
                    let _ = client
                        .update_message(&dm, message_id, Some(&blocks), &fallback)
                        .await;
                }
            }
        }
        "dingding" => {
            let map = crate::dingtalk::confirm::build_final_param_map(
                &final_view.title,
                &final_view.body,
                &final_view.label,
            );
            if let Ok(client) =
                crate::dingtalk::client::DingTalkClient::new(&config.channels.dingding)
            {
                let _ = client
                    .update_card_private(message_id, map, serde_json::json!({}))
                    .await;
            }
        }
        _ => {}
    }
}

/// Truncate text for use as a finalized button label (capped at 40 chars).
pub fn truncate_for_label(text: &str) -> String {
    let t = text.trim();
    let chars: String = t.chars().take(40).collect();
    if t.chars().count() > 40 {
        format!("{chars}…")
    } else {
        chars
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_text_unchanged() {
        assert_eq!(truncate_for_label("ok"), "ok");
    }

    #[test]
    fn trims_whitespace() {
        assert_eq!(truncate_for_label("  hello  "), "hello");
    }

    #[test]
    fn long_text_truncated_with_ellipsis() {
        let long = "a".repeat(50);
        let result = truncate_for_label(&long);
        assert_eq!(result.chars().count(), 41); // 40 + ellipsis
        assert!(result.ends_with('…'));
    }

    #[test]
    fn exactly_40_chars_no_ellipsis() {
        let exact = "b".repeat(40);
        assert_eq!(truncate_for_label(&exact), exact);
    }
}
