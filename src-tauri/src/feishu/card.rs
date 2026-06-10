//! 飞书消息卡片 JSON 2.0：组装提问卡片（表单容器：勾选器 + 输入框 + 提交按钮）+ 解析提交回调。
//!
//! 设计（见 `docs/plans/feishu-channel.md`）：
//! - 提问卡片直接以 JSON 下发（无需后台模板），用 `msg_type=interactive`。
//! - 预定义选项用 `checker`（复选框/勾选器，平铺直接勾），置于 `form` 表单容器内；
//!   一个 `input` 输入框收补充文字；一个 `button`(`form_action_type=submit`) 提交。
//! - 用户点「提交」→ 一次 `card.action.trigger` 回调，`action.form_value` 汇总所有组件取值。
//! - 选项 ↔ 组件名映射 `opt_{i}`，便于回调里还原勾选了哪些选项（规避超长/重复选项文案）。

use crate::models::OptionItem;
use serde_json::{json, Value};

/// 选项组件名前缀（`opt_0` / `opt_1` ...）。
const OPT_NAME_PREFIX: &str = "opt_";
/// 输入框组件名。
const INPUT_NAME: &str = "user_input";

/// 一次卡片「提交」回调的解析结果。
pub struct CardSubmit {
    pub open_id: String,
    /// 卡片所在消息 ID（`context.open_message_id`），用于匹配当前题卡片。
    pub message_id: String,
    /// 勾选的预定义选项（选项文本，已按下标还原）。
    pub selected_options: Vec<String>,
    /// 补充文字输入（空则 None）。
    pub user_input: Option<String>,
}

/// 组装提问卡片（卡片 JSON 2.0）。
/// `title` 为题首（空则省略 header）；`text` 为问题正文；`options` 为预定义选项（空则无选项区）；
/// `is_markdown` 决定正文用 markdown 还是 plain_text 组件；`input_placeholder` 为输入框占位提示；
/// `submit_label` 为提交按钮文案；`recommended_prefix` 为推荐选项的显示前缀（与其他文案
/// 一样由渠道层本地化后传入；提交值始终为选项原文）。
#[allow(clippy::too_many_arguments)]
pub fn build_question_card(
    title: &str,
    text: &str,
    options: &[OptionItem],
    is_markdown: bool,
    input_placeholder: &str,
    submit_label: &str,
    recommended_prefix: &str,
) -> Value {
    let mut elements: Vec<Value> = Vec::new();
    if !text.trim().is_empty() {
        elements.push(body_text(text, is_markdown));
    }
    elements.push(build_form(
        options,
        &[],
        None,
        false,
        input_placeholder,
        submit_label,
        recommended_prefix,
    ));
    assemble_card(title, elements, true)
}

/// 终态卡片入参（复刻钉钉「已提交」态）。
pub struct Finalized<'a> {
    pub title: &'a str,
    pub text: &'a str,
    pub is_markdown: bool,
    pub options: &'a [OptionItem],
    /// 用户已选选项（原文；被抢答收尾时为空 → 勾选器都不勾）。
    pub selected: &'a [String],
    /// 补充文字回显（无则 None → 输入框留空）。
    pub user_input: Option<&'a str>,
    pub input_placeholder: &'a str,
    /// 禁用按钮的文案（「已提交」/「已在 X 回答」）。
    pub button_label: &'a str,
    /// 推荐选项的显示前缀（本地化）。
    pub recommended_prefix: &'a str,
}

/// 提示消息（message prompt）的 markdown 卡片：标题（来源头部）+ markdown 正文。
/// 飞书 IM 没有像钉钉 sampleMarkdown 那样的「markdown 文本消息」类型，故以卡片渲染
/// markdown（粗体/标题/列表/代码/部分表格）。正文为空时仅留标题。
pub fn build_message_card(title: &str, markdown_body: &str) -> Value {
    let mut elements: Vec<Value> = Vec::new();
    if !markdown_body.trim().is_empty() {
        elements.push(body_text(markdown_body, true));
    }
    assemble_card(title, elements, false)
}

/// 卡片回调的同步「更新卡片」回包体：`{card:{type:"raw",data:<新卡片>}}`。
/// 点提交时由会话经 Router 同步回此包 → 按钮 Loading 直接变终态（否则空 ACK 会令按钮先弹回 Submit，
/// 再由 OpenAPI `patch_card` 异步置灰，出现可见闪烁）。
pub fn callback_update_card(card: Value) -> Value {
    json!({ "card": { "type": "raw", "data": card } })
}

/// 组装终态卡片（复刻钉钉「已提交」态）：沿用同一表单结构，但全部禁用——
/// 勾选器按用户选择 `checked` 且 `disabled`、输入框 `default_value` 回显补充文字且 `disabled`、
/// 提交按钮 `disabled` 并改文案。
pub fn build_finalized_card(p: &Finalized) -> Value {
    let mut elements: Vec<Value> = Vec::new();
    if !p.text.trim().is_empty() {
        elements.push(body_text(p.text, p.is_markdown));
    }
    elements.push(build_form(
        p.options,
        p.selected,
        p.user_input,
        true,
        p.input_placeholder,
        p.button_label,
        p.recommended_prefix,
    ));
    assemble_card(p.title, elements, true)
}

/// 组装表单容器：每选项一个勾选器 + 输入框 + 提交按钮。
/// `disabled=false`（提问态）：可交互，按钮带 `callback` behaviors。
/// `disabled=true`（终态）：禁用全部交互，勾选器按 `selected` 勾上，输入框用 `user_input` 回显，按钮无 behaviors。
/// 勾选器展示文本为「前缀（仅推荐项）+ 原文」；`selected` 与 `checked` 的比对用原文。
#[allow(clippy::too_many_arguments)]
fn build_form(
    options: &[OptionItem],
    selected: &[String],
    user_input: Option<&str>,
    disabled: bool,
    input_placeholder: &str,
    button_label: &str,
    recommended_prefix: &str,
) -> Value {
    let mut form_elements: Vec<Value> = Vec::new();
    for (i, opt) in options.iter().enumerate() {
        let display = if opt.recommended {
            format!("{}{}", recommended_prefix, opt.text)
        } else {
            opt.text.clone()
        };
        let mut checker = json!({
            "tag": "checker",
            "name": format!("{}{}", OPT_NAME_PREFIX, i),
            "checked": selected.contains(&opt.text),
            "text": { "tag": "plain_text", "content": display },
        });
        if disabled {
            checker["disabled"] = Value::Bool(true);
        }
        form_elements.push(checker);
    }

    let mut input = json!({
        "tag": "input",
        "name": INPUT_NAME,
        "placeholder": { "tag": "plain_text", "content": input_placeholder },
    });
    if let Some(v) = user_input {
        input["default_value"] = Value::String(v.to_string());
    }
    if disabled {
        input["disabled"] = Value::Bool(true);
    }
    form_elements.push(input);

    let mut button = json!({
        "tag": "button",
        "name": "submit",
        "form_action_type": "submit",
        "text": { "tag": "plain_text", "content": button_label },
        "type": "primary",
    });
    if disabled {
        button["disabled"] = Value::Bool(true);
    } else {
        button["behaviors"] = json!([ { "type": "callback", "value": { "action": "submit" } } ]);
    }
    form_elements.push(button);

    json!({
        "tag": "form",
        "name": "answer_form",
        "elements": form_elements,
    })
}

/// Assemble the card skeleton: schema 2.0 + body.elements. `config.update_multi` is on to allow later updates.
///
/// `styled_header=true` (question/finalized cards): mimic DingTalk's "icon container" style — put a single
/// plain-text component row at the top of the body "question icon (left) + small blue title (right)" plus a
/// divider, separating it from the body (no native banner, since the banner title font is fixed and large and
/// renders as a full color strip). `false` (message card): keep the plain native header title.
fn assemble_card(title: &str, elements: Vec<Value>, styled_header: bool) -> Value {
    if styled_header && !title.trim().is_empty() {
        // Style matches the card exported from Feishu's card builder: small (notation) title + filled
        // question icon (maybe_filled); blue color (the builder has no blue option, user confirmed blue).
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
        let mut body = vec![
            header_row,
            json!({ "tag": "hr", "margin": "0px 0px 0px 0px" }),
        ];
        body.extend(elements);
        return json!({
            "schema": "2.0",
            "config": { "update_multi": true },
            "body": { "elements": body },
        });
    }
    let mut card = json!({
        "schema": "2.0",
        "config": { "update_multi": true },
        "body": { "elements": elements },
    });
    if !title.trim().is_empty() {
        card["header"] = json!({ "title": { "tag": "plain_text", "content": title } });
    }
    card
}

/// 正文组件：markdown → `markdown` 组件；纯文本 → `div` + plain_text。
fn body_text(text: &str, is_markdown: bool) -> Value {
    if is_markdown {
        json!({ "tag": "markdown", "content": text })
    } else {
        json!({ "tag": "div", "text": { "tag": "plain_text", "content": text } })
    }
}

/// 把一条 `card.action.trigger` 的 `event` 解析为「提交」结果；非表单提交 / 缺字段返回 None。
/// `options` 用于把 `opt_{i}` 还原为选项文本。
pub fn parse_card_submit(event: &Value, options: &[OptionItem]) -> Option<CardSubmit> {
    let open_id = event
        .get("operator")
        .and_then(|o| o.get("open_id"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let message_id = event
        .get("context")
        .and_then(|c| c.get("open_message_id"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let action = event.get("action")?;
    // 必须是表单提交（含 form_value）。
    let form_value = action.get("form_value")?;

    let mut selected: Vec<String> = Vec::new();
    for (i, opt) in options.iter().enumerate() {
        let key = format!("{}{}", OPT_NAME_PREFIX, i);
        if is_checked(form_value.get(&key)) {
            selected.push(opt.text.clone());
        }
    }
    let user_input = form_value
        .get(INPUT_NAME)
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    Some(CardSubmit {
        open_id,
        message_id,
        selected_options: selected,
        user_input,
    })
}

/// 勾选状态判定：兼容布尔 `true` 或字符串 `"true"`。
fn is_checked(v: Option<&Value>) -> bool {
    match v {
        Some(Value::Bool(b)) => *b,
        Some(Value::String(s)) => s == "true",
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain(items: &[&str]) -> Vec<OptionItem> {
        items.iter().map(|s| OptionItem::new(*s, false)).collect()
    }

    #[test]
    fn build_card_has_form_and_options() {
        let card = build_question_card(
            "Question 1/2",
            "要继续吗？",
            &plain(&["继续", "停止"]),
            true,
            "补充说明（可选）",
            "提交",
            "👍推荐 ",
        );
        assert_eq!(card["schema"], "2.0");
        // Question card: title is now a body-top "question icon + blue title" row + divider (DingTalk-style), no native header.
        assert!(card.get("header").is_none());
        let elements = card["body"]["elements"].as_array().unwrap();
        assert_eq!(elements[0]["text"]["content"], "Question 1/2");
        assert_eq!(elements[0]["icon"]["token"], "maybe_filled");
        assert_eq!(elements[1]["tag"], "hr");
        // 正文 + 表单容器。
        let form = elements.iter().find(|e| e["tag"] == "form").unwrap();
        let fe = form["elements"].as_array().unwrap();
        // 两个 checker + 一个 input + 一个 submit button。
        assert_eq!(fe.iter().filter(|e| e["tag"] == "checker").count(), 2);
        assert!(fe.iter().any(|e| e["tag"] == "input" && e["name"] == "user_input"));
        assert!(fe.iter().any(|e| e["tag"] == "button" && e["form_action_type"] == "submit"));
        assert_eq!(fe[0]["name"], "opt_0");
        assert_eq!(fe[1]["name"], "opt_1");
    }

    #[test]
    fn recommended_option_gets_display_prefix_but_keeps_plain_value() {
        let opts = vec![OptionItem::new("继续", true), OptionItem::new("停止", false)];
        let card = build_question_card("T", "Q", &opts, true, "ph", "提交", "👍推荐 ");
        let form = card["body"]["elements"]
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["tag"] == "form")
            .unwrap();
        let fe = form["elements"].as_array().unwrap();
        let checkers: Vec<&Value> = fe.iter().filter(|e| e["tag"] == "checker").collect();
        // 显示文本带前缀，普通选项不带。
        assert_eq!(checkers[0]["text"]["content"], "👍推荐 继续");
        assert_eq!(checkers[1]["text"]["content"], "停止");
        // 提交按下标还原，回传原文。
        let event = json!({
            "operator": { "open_id": "ou_1" },
            "context": { "open_message_id": "om_1" },
            "action": { "form_value": { "opt_0": true } }
        });
        let s = parse_card_submit(&event, &opts).unwrap();
        assert_eq!(s.selected_options, vec!["继续".to_string()]);
    }

    #[test]
    fn build_card_without_options_omits_checkers() {
        let card = build_question_card("", "随便说点什么", &[], false, "请输入", "提交", "👍推荐 ");
        assert!(card.get("header").is_none());
        let form = card["body"]["elements"]
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["tag"] == "form")
            .unwrap();
        let fe = form["elements"].as_array().unwrap();
        assert_eq!(fe.iter().filter(|e| e["tag"] == "checker").count(), 0);
        // 非 markdown 正文用 div + plain_text。
        let div = card["body"]["elements"]
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["tag"] == "div");
        assert!(div.is_some());
    }

    #[test]
    fn finalized_card_disables_form_and_reflects_selection() {
        // 「停止」为推荐项：显示带前缀，但勾选比对仍按原文命中。
        let opts = vec![OptionItem::new("继续", false), OptionItem::new("停止", true)];
        let sel = vec!["停止".to_string()];
        let card = build_finalized_card(&Finalized {
            title: "Question 1/2",
            text: "要继续吗？",
            is_markdown: true,
            options: &opts,
            selected: &sel,
            user_input: Some("再想想"),
            input_placeholder: "补充说明（可选）",
            button_label: "已提交",
            recommended_prefix: "👍推荐 ",
        });
        let form = card["body"]["elements"]
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["tag"] == "form")
            .unwrap();
        let fe = form["elements"].as_array().unwrap();
        // 勾选器：均禁用；仅「停止」勾上。
        let checkers: Vec<&Value> = fe.iter().filter(|e| e["tag"] == "checker").collect();
        assert_eq!(checkers.len(), 2);
        assert_eq!(checkers[0]["checked"], false);
        assert_eq!(checkers[0]["disabled"], true);
        assert_eq!(checkers[1]["checked"], true);
        assert_eq!(checkers[1]["disabled"], true);
        assert_eq!(checkers[1]["text"]["content"], "👍推荐 停止");
        // 输入框：禁用 + 回显补充文字。
        let input = fe.iter().find(|e| e["tag"] == "input").unwrap();
        assert_eq!(input["disabled"], true);
        assert_eq!(input["default_value"], "再想想");
        // 按钮：禁用 + 改文案 + 无 behaviors。
        let button = fe.iter().find(|e| e["tag"] == "button").unwrap();
        assert_eq!(button["disabled"], true);
        assert_eq!(button["text"]["content"], "已提交");
        assert!(button.get("behaviors").is_none());
    }

    #[test]
    fn parse_submit_maps_checked_options_and_input() {
        let event = json!({
            "operator": { "open_id": "ou_1" },
            "context": { "open_message_id": "om_1" },
            "action": {
                "tag": "button",
                "name": "submit",
                "form_value": {
                    "opt_0": true,
                    "opt_1": false,
                    "opt_2": "true",
                    "user_input": "  hi  "
                }
            }
        });
        let opts = plain(&["A", "B", "C"]);
        let s = parse_card_submit(&event, &opts).unwrap();
        assert_eq!(s.open_id, "ou_1");
        assert_eq!(s.message_id, "om_1");
        assert_eq!(s.selected_options, vec!["A".to_string(), "C".to_string()]);
        assert_eq!(s.user_input.as_deref(), Some("hi"));
    }

    #[test]
    fn parse_submit_empty_input_is_none() {
        let event = json!({
            "operator": { "open_id": "ou_1" },
            "context": { "open_message_id": "om_1" },
            "action": { "form_value": { "user_input": "" } }
        });
        let s = parse_card_submit(&event, &[]).unwrap();
        assert!(s.user_input.is_none());
        assert!(s.selected_options.is_empty());
    }

    #[test]
    fn parse_non_form_returns_none() {
        let event = json!({
            "operator": { "open_id": "ou_1" },
            "context": { "open_message_id": "om_1" },
            "action": { "tag": "button", "value": { "action": "noop" } }
        });
        assert!(parse_card_submit(&event, &[]).is_none());
    }
}
