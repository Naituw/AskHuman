//! Hidden dev-only subcommand `__demo-cards`: push prototype cards (single / strict /
//! native-recommended) through the real IM channels so the proposed styles can be reviewed
//! visually before the strict-choice / structured-output feature is implemented.
//!
//! This is NOT a user-facing command. It builds raw card JSON inline (independent of the
//! production card builders, which do not yet support single / strict / native recommended)
//! and sends it via the existing channel clients using the user's configured credentials.
//! It only SENDS (no interaction wiring); clicking submit is a no-op for these one-off cards.
//!
//! Usage: `AskHuman __demo-cards [feishu|slack|telegram|all]` (default: all configured).
//! Remove this module once the card styles are finalized (plan phase 0).

use crate::config::AppConfig;
use serde_json::{json, Value};

/// Entry point for the hidden `__demo-cards` subcommand.
pub fn run(args: &[String]) {
    let which = args.first().map(|s| s.as_str()).unwrap_or("all");
    let cfg = AppConfig::load();
    let rt = match tokio::runtime::Runtime::new() {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("demo-cards: failed to start runtime: {e}");
            return;
        }
    };
    rt.block_on(async {
        // Plain-text delivery diagnostic: `__demo-cards ping <label>` sends a plain message to
        // each channel and prints the resolved destination, to confirm the link end-to-end.
        if which == "ping" {
            let label = args.get(1..).map(|r| r.join(" ")).unwrap_or_default();
            ping(&cfg, &label).await;
            return;
        }
        if which == "all" || which == "slack" {
            demo_slack(&cfg).await;
        }
        if which == "all" || which == "feishu" {
            demo_feishu(&cfg).await;
        }
        if which == "all" || which == "telegram" {
            demo_telegram(&cfg).await;
        }
        if which == "all" || which == "dingtalk" {
            demo_dingtalk(&cfg).await;
        }
    });
}

/// Send a plain timestamped text to each channel and print the resolved destination.
async fn ping(cfg: &AppConfig, label: &str) {
    let msg = format!("[AskHuman demo] ping {label}");

    match crate::slack::client::SlackClient::new(&cfg.channels.slack) {
        Ok(c) => {
            println!("slack: user_id={}", c.user_id());
            match c.open_dm().await {
                Ok(ch) => {
                    println!("slack: dm_channel={ch}");
                    match c.post_text(&ch, &msg).await {
                        Ok(ts) => println!("slack: ping sent ts={ts}"),
                        Err(e) => eprintln!("slack: ping failed: {e}"),
                    }
                }
                Err(e) => eprintln!("slack: open_dm failed: {e}"),
            }
        }
        Err(e) => eprintln!("slack: skip ({e})"),
    }

    match crate::feishu::client::FeishuClient::new(&cfg.channels.feishu) {
        Ok(c) => {
            println!("feishu: app_id={} open_id={}", c.app_id(), c.open_id());
            match c.send_text(&msg).await {
                Ok(id) => println!("feishu: ping sent message_id={id}"),
                Err(e) => eprintln!("feishu: ping failed: {e}"),
            }
        }
        Err(e) => eprintln!("feishu: skip ({e})"),
    }

    let tg = &cfg.channels.telegram;
    match crate::telegram::TelegramClient::new(
        tg.bot_token.clone(),
        tg.chat_id.clone(),
        tg.api_base_url.clone(),
    ) {
        Ok(c) => {
            println!("telegram: chat_id={}", c.chat_id());
            match c.send_message(&msg, None, None).await {
                Ok(id) => println!("telegram: ping sent message_id={id}"),
                Err(e) => eprintln!("telegram: ping failed: {e}"),
            }
        }
        Err(e) => eprintln!("telegram: skip ({e})"),
    }
}

// ===================================================================================
// Slack — native widgets: checkboxes (multi) / radio_buttons (single); recommended shown
// via the option's `description` ("👍 推荐") plus bold mrkdwn option `text`.
// ===================================================================================

async fn demo_slack(cfg: &AppConfig) {
    let client = match crate::slack::client::SlackClient::new(&cfg.channels.slack) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("demo-cards[slack]: skip ({e})");
            return;
        }
    };
    let channel = match client.open_dm().await {
        Ok(c) => c,
        Err(e) => {
            eprintln!("demo-cards[slack]: open_dm failed: {e}");
            return;
        }
    };

    // Card 1: multi-select (checkboxes) + free-text input (non-strict) + native recommended.
    let card1 = json!([
        header("❓ [Demo] Slack 多选 · 含补充输入 · 原生推荐"),
        section_mrkdwn("选择要执行的操作（可多选）"),
        {
            "type": "input",
            "optional": true,
            "block_id": "demo_opts_1",
            "label": { "type": "plain_text", "text": "选项", "emoji": true },
            "element": {
                "type": "checkboxes",
                "action_id": "options_0",
                "options": [
                    slack_opt_plain("保持原状", "opt_0"),
                    slack_opt_recommended("重构该模块", "opt_1"),
                    slack_opt_plain("删除旧实现", "opt_2"),
                ],
            },
        },
        {
            "type": "input",
            "optional": true,
            "block_id": "demo_userinput_1",
            "label": { "type": "plain_text", "text": "补充说明", "emoji": true },
            "element": {
                "type": "plain_text_input",
                "action_id": "user_input",
                "multiline": true,
                "placeholder": { "type": "plain_text", "text": "可补充说明（可选）", "emoji": true },
            },
        },
        slack_submit(),
    ]);

    // Card 2: single-select (radio_buttons) + strict (no input) + native recommended.
    let card2 = json!([
        header("❓ [Demo] Slack 单选(radio) · 严格(无输入) · 原生推荐"),
        section_mrkdwn("选择部署环境（单选）"),
        {
            "type": "input",
            "block_id": "demo_radio_2",
            "label": { "type": "plain_text", "text": "环境", "emoji": true },
            "element": {
                "type": "radio_buttons",
                "action_id": "options_0",
                "options": [
                    slack_opt_plain("staging", "opt_0"),
                    slack_opt_recommended("production", "opt_1"),
                    slack_opt_plain("canary", "opt_2"),
                ],
            },
        },
        slack_submit(),
    ]);

    for (label, card) in [("多选/推荐", card1), ("单选/严格", card2)] {
        match client.post_message(&channel, Some(&card), "AskHuman 卡片样式 Demo").await {
            Ok(ts) => println!("demo-cards[slack] sent {label}: ts={ts}"),
            Err(e) => eprintln!("demo-cards[slack] {label} failed: {e}"),
        }
    }
}

fn header(text: &str) -> Value {
    json!({ "type": "header", "text": { "type": "plain_text", "text": text, "emoji": true } })
}

fn section_mrkdwn(text: &str) -> Value {
    json!({ "type": "section", "text": { "type": "mrkdwn", "text": text } })
}

fn slack_opt_plain(text: &str, value: &str) -> Value {
    json!({ "text": { "type": "plain_text", "text": text, "emoji": true }, "value": value })
}

/// Recommended option: bold mrkdwn text + a mrkdwn `description` "👍 推荐" line below it.
fn slack_opt_recommended(text: &str, value: &str) -> Value {
    json!({
        "text": { "type": "mrkdwn", "text": format!("*{}*", text) },
        "description": { "type": "mrkdwn", "text": "👍 推荐" },
        "value": value,
    })
}

fn slack_submit() -> Value {
    json!({
        "type": "actions",
        "block_id": "actions",
        "elements": [ {
            "type": "button",
            "action_id": "submit",
            "text": { "type": "plain_text", "text": "提交", "emoji": true },
            "style": "primary",
            "value": "submit",
        } ],
    })
}

// ===================================================================================
// Feishu — the checker component has NO prefix `icon` (rejected by the API). Two viable
// native-ish recommended styles are demoed: (A) a `button_area` "👍 推荐" chip on the
// checker (closest to a native widget badge); (B) lark_md bold + colored "推荐" text in the
// checker's own text. Form component `name` must be unique per card. Single-select is shown
// visually (one pre-checked); the real version uses click callbacks for mutual exclusion.
// ===================================================================================

async fn demo_feishu(cfg: &AppConfig) {
    let client = match crate::feishu::client::FeishuClient::new(&cfg.channels.feishu) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("demo-cards[feishu]: skip ({e})");
            return;
        }
    };

    // Final Feishu recommended style: a LEFT colored-text prefix with green brackets, via lark_md.
    let prefix = "<font color='green'>【👍推荐】</font> ";
    // A deliberately long option text to observe wrapping behavior.
    let long = "重构该模块并补充单元测试与回归测试，确保与旧接口完全兼容（这是一个故意写长用于观察换行的选项）";

    // Card 1: short recommended option.
    let card1 = feishu_card(
        "[Demo] 飞书 推荐=绿色括号左前缀 · 短文本",
        "选择要执行的操作（可多选）",
        &[
            checker_plain("保持原状", "opt_0", false),
            checker_larkmd(&format!("{prefix}重构该模块"), "opt_1", false),
            checker_plain("删除旧实现", "opt_2", false),
        ],
        true,
    );

    // Card 2: long recommended option — see how the second wrapped line returns to the left margin.
    let card2 = feishu_card(
        "[Demo] 飞书 推荐=绿色括号左前缀 · 长文本换行",
        "选择要执行的操作（可多选）",
        &[
            checker_plain("保持原状", "opt_0", false),
            checker_larkmd(&format!("{prefix}{long}"), "opt_1", false),
            checker_plain("删除旧实现", "opt_2", false),
        ],
        true,
    );

    for (label, card) in [("绿色括号/短", card1), ("绿色括号/长", card2)] {
        match client.send_card(&card).await {
            Ok(id) => println!("demo-cards[feishu] sent {label}: message_id={id}"),
            Err(e) => eprintln!("demo-cards[feishu] {label} failed: {e}"),
        }
    }
}

/// Plain checker (no recommendation).
fn checker_plain(text: &str, name: &str, checked: bool) -> Value {
    json!({
        "tag": "checker",
        "name": name,
        "checked": checked,
        "text": { "tag": "plain_text", "content": text },
    })
}

/// Checker whose text uses lark_md (bold / colored / emoji) to mark the recommendation.
fn checker_larkmd(content_md: &str, name: &str, checked: bool) -> Value {
    json!({
        "tag": "checker",
        "name": name,
        "checked": checked,
        "text": { "tag": "lark_md", "content": content_md },
    })
}

/// Checker with a `button_area` "👍 推荐" chip (a thumbs-up green icon + "推荐" text button,
/// always shown on PC, no-op on click) — the most native-widget-like recommended badge.
fn checker_chip(text: &str, name: &str, checked: bool) -> Value {
    json!({
        "tag": "checker",
        "name": name,
        "checked": checked,
        "text": { "tag": "plain_text", "content": text },
        "button_area": {
            "pc_display_rule": "always",
            "buttons": [ {
                "tag": "button",
                "type": "text",
                "size": "small",
                "text": { "tag": "plain_text", "content": "推荐" },
                "icon": { "tag": "standard_icon", "token": "thumbsup_filled", "color": "green" },
                "behaviors": [],
            } ],
        },
    })
}

/// Assemble a Feishu question card (DingTalk-style title row + divider + body + form).
fn feishu_card(title: &str, body: &str, checkers: &[Value], include_input: bool) -> Value {
    let header_row = json!({
        "tag": "div",
        "text": {
            "tag": "plain_text",
            "content": title,
            "text_size": "notation",
            "text_align": "left",
            "text_color": "blue",
        },
        "icon": { "tag": "standard_icon", "token": "maybe_filled", "color": "blue" },
        "margin": "0px 0px 0px 0px",
    });

    let mut form_elements: Vec<Value> = checkers.to_vec();
    if include_input {
        form_elements.push(json!({
            "tag": "input",
            "name": "user_input",
            "placeholder": { "tag": "plain_text", "content": "可补充说明（可选）" },
        }));
    }
    form_elements.push(json!({
        "tag": "button",
        "name": "submit",
        "form_action_type": "submit",
        "text": { "tag": "plain_text", "content": "提交" },
        "type": "primary",
        "behaviors": [ { "type": "callback", "value": { "action": "submit" } } ],
    }));

    json!({
        "schema": "2.0",
        "config": { "update_multi": true },
        "body": { "elements": [
            header_row,
            { "tag": "hr", "margin": "0px 0px 0px 0px" },
            { "tag": "markdown", "content": body },
            { "tag": "form", "name": "answer_form", "elements": form_elements },
        ] },
    })
}

// ===================================================================================
// Telegram — no native widget styling: recommended shown via a 👍 emoji on the body line;
// options listed in the body, letter buttons below. Single-select highlight is behavior-only.
// ===================================================================================

async fn demo_telegram(cfg: &AppConfig) {
    let tg = &cfg.channels.telegram;
    let client = match crate::telegram::TelegramClient::new(
        tg.bot_token.clone(),
        tg.chat_id.clone(),
        tg.api_base_url.clone(),
    ) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("demo-cards[telegram]: skip ({})", e);
            return;
        }
    };

    // Card 1: multi-select, recommended via the current text prefix「【👍推荐】 」(unchanged).
    let multi_body = "❓ [Demo] Telegram 多选 · 推荐=现状前缀\n\n\
        A. 保持原状\n\
        B. 【👍推荐】 重构该模块\n\
        C. 删除旧实现\n\n\
        请选择后点「提交」";
    let multi_keyboard = json!({
        "inline_keyboard": [
            [
                { "text": "A", "callback_data": "toggle:0" },
                { "text": "B", "callback_data": "toggle:1" },
                { "text": "C", "callback_data": "toggle:2" },
            ],
            [ { "text": "提交", "callback_data": "submit" } ],
        ],
    });

    // Card 2: single-select look — one option pre-selected (✅) to show the highlight; the real
    // version clears the others on click (mutual exclusion).
    let single_body = "❓ [Demo] Telegram 单选 · 已选 B 示意\n\n\
        A. staging\n\
        B. 【👍推荐】 production\n\
        C. canary\n\n\
        单选：选一个会自动清掉其它（正式版交互）。请选择后点「提交」";
    let single_keyboard = json!({
        "inline_keyboard": [
            [
                { "text": "A", "callback_data": "pick:0" },
                { "text": "✅ B", "callback_data": "pick:1" },
                { "text": "C", "callback_data": "pick:2" },
            ],
            [ { "text": "提交", "callback_data": "submit" } ],
        ],
    });

    for (label, body, keyboard) in [
        ("多选", multi_body, multi_keyboard),
        ("单选", single_body, single_keyboard),
    ] {
        match client.send_message(body, None, Some(keyboard)).await {
            Ok(id) => println!("demo-cards[telegram] sent {label}: message_id={id}"),
            Err(e) => eprintln!("demo-cards[telegram] {label} failed: {}", e),
        }
    }
}

// ===================================================================================
// DingTalk — user-built template `d5dc7ac5-…` (CheckboxList / CheckboxListMulti switched by the
// `single` bool, `Input` shown by `allow_input`, options rendered from `options[].md` rich text).
// Recommended marker is a green `<font colorTokenV2=common_green1_color>👍推荐</font>` prefix
// inside each option's `md`. Option `id` = its 0-based index, so the submit callback returns the
// indices directly. `single`/`allow_input`/`submitted` MUST be real booleans (the template uses
// `@triple{@data{single}}`, where the string "false" would wrongly count as truthy).
// ===================================================================================

/// User-built template for the strict-choice / single / recommended feature (not yet published).
const DINGTALK_TEMPLATE_ID: &str = "d5dc7ac5-1fca-443a-8230-d33ce63e837f.schema";

async fn demo_dingtalk(cfg: &AppConfig) {
    let client = match crate::dingtalk::client::DingTalkClient::new(&cfg.channels.dingding) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("demo-cards[dingtalk]: skip ({e})");
            return;
        }
    };

    let card_single = dingtalk_card(
        "[Demo] 钉钉 单选 · 含输入",
        "选择部署环境（单选）",
        true,
        true,
        &[("staging", false), ("production", true), ("canary", false)],
    );
    let card_multi = dingtalk_card(
        "[Demo] 钉钉 多选 · 含输入",
        "选择要执行的操作（可多选）",
        false,
        true,
        &[
            ("保持原状", false),
            (
                "重构该模块：这是一个很长很长很长很长很长很长很长很长很长很长很长很长的推荐选项，用来观察推荐徽标加长文本换行后的缩进效果",
                true,
            ),
            ("删除旧实现", false),
        ],
    );
    let card_strict = dingtalk_card(
        "[Demo] 钉钉 多选 · 严格(无输入)",
        "勾选要清理的缓存（严格）",
        false,
        false,
        &[("npm", false), ("cargo", true), ("docker", false)],
    );

    for (label, public, private) in [
        ("单选/含输入", card_single.0, card_single.1),
        ("多选/含输入", card_multi.0, card_multi.1),
        ("多选/严格", card_strict.0, card_strict.1),
    ] {
        let out_track_id = uuid::Uuid::new_v4().to_string();
        match client
            .create_and_deliver_card(&out_track_id, DINGTALK_TEMPLATE_ID, public, private)
            .await
        {
            Ok(()) => println!("demo-cards[dingtalk] sent {label}: out_track_id={out_track_id}"),
            Err(e) => eprintln!("demo-cards[dingtalk] {label} failed: {e}"),
        }
    }
}

/// Build (public cardParamMap, private cardParamMap) for the strict-choice template.
/// `options` is `(text, recommended)`; option `id` = index; recommended gets a green prefix in `md`.
/// Booleans are emitted as real JSON booleans (required by the template's `@triple` conditions).
fn dingtalk_card(
    title: &str,
    question: &str,
    single: bool,
    allow_input: bool,
    options: &[(&str, bool)],
) -> (Value, Value) {
    // DingTalk card markdown only accepts preset size tokens (no custom px). h5 = 15px (PC &
    // mobile), the middle ground between footnote 12 and body 14(PC)/17(mobile). Recommended gets
    // a green bracketed prefix (like Feishu).
    const SIZE: &str = "common_h5_text_style__font_size";
    let opts: Vec<Value> = options
        .iter()
        .enumerate()
        .map(|(i, (text, recommended))| {
            let md = if *recommended {
                format!(
                    "<font sizeToken={SIZE} colorTokenV2=common_green1_color>【👍推荐】</font> <font sizeToken={SIZE}>{text}</font>"
                )
            } else {
                format!("<font sizeToken={SIZE}>{text}</font>")
            };
            json!({ "id": i, "md": md })
        })
        .collect();
    // DingTalk's cardParamMap demands string values; booleans go as "true"/"false" and the
    // template coerces them back via the variable's declared boolean type.
    let public = json!({
        "title": title,
        "markdown": question,
        // Complex value → JSON string; the template's loop parses it back into an array.
        "options": Value::Array(opts).to_string(),
        "single": single.to_string(),
        "allow_input": allow_input.to_string(),
        "submit_status": "",
    });
    let private = json!({
        "submitted": "false",
        "private_input": "",
    });
    (public, private)
}
