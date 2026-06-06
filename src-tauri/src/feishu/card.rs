//! 飞书消息卡片 JSON 2.0：组装提问卡片（表单容器：勾选器 + 输入框 + 提交按钮）+ 解析提交回调。
//!
//! 设计（见 `docs/plans/feishu-channel.md`）：
//! - 提问卡片直接以 JSON 下发（无需后台模板），用 `msg_type=interactive`。
//! - 预定义选项用 `checker`（复选框/勾选器，平铺直接勾），置于 `form` 表单容器内；
//!   一个 `input` 输入框收补充文字；一个 `button`(`form_action_type=submit`) 提交。
//! - 用户点「提交」→ 一次 `card.action.trigger` 回调，`action.form_value` 汇总所有组件取值。
//! - 选项 ↔ 组件名映射 `opt_{i}`，便于回调里还原勾选了哪些选项（规避超长/重复选项文案）。

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
/// `submit_label` 为提交按钮文案。
pub fn build_question_card(
    title: &str,
    text: &str,
    options: &[String],
    is_markdown: bool,
    input_placeholder: &str,
    submit_label: &str,
) -> Value {
    let mut elements: Vec<Value> = Vec::new();

    if !text.trim().is_empty() {
        elements.push(body_text(text, is_markdown));
    }

    // 表单容器：选项勾选器 + 输入框 + 提交按钮。
    let mut form_elements: Vec<Value> = Vec::new();
    for (i, opt) in options.iter().enumerate() {
        form_elements.push(json!({
            "tag": "checker",
            "name": format!("{}{}", OPT_NAME_PREFIX, i),
            "checked": false,
            "text": { "tag": "plain_text", "content": opt },
        }));
    }
    form_elements.push(json!({
        "tag": "input",
        "name": INPUT_NAME,
        "placeholder": { "tag": "plain_text", "content": input_placeholder },
    }));
    form_elements.push(json!({
        "tag": "button",
        "name": "submit",
        "form_action_type": "submit",
        "text": { "tag": "plain_text", "content": submit_label },
        "type": "primary",
        "behaviors": [ { "type": "callback", "value": { "action": "submit" } } ],
    }));
    elements.push(json!({
        "tag": "form",
        "name": "answer_form",
        "elements": form_elements,
    }));

    assemble_card(title, elements)
}

/// 组装终态卡片（无表单）：题首 + 正文 + 状态行（如「✅ 已提交」/「✅ 已在 X 回答」）。
/// 用于提交后 / 被抢答收尾时 PATCH 更新卡片。
pub fn build_finalized_card(title: &str, text: &str, status: &str) -> Value {
    let mut elements: Vec<Value> = Vec::new();
    if !text.trim().is_empty() {
        // 终态正文统一用 markdown 组件展示（原文若非 markdown 亦可接受）。
        elements.push(json!({ "tag": "markdown", "content": text }));
    }
    elements.push(json!({ "tag": "markdown", "content": format!("**{}**", status) }));
    assemble_card(title, elements)
}

/// 组装卡片骨架：schema 2.0 + 可选 header + body.elements。`config.update_multi` 开启以支持后续更新。
fn assemble_card(title: &str, elements: Vec<Value>) -> Value {
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
pub fn parse_card_submit(event: &Value, options: &[String]) -> Option<CardSubmit> {
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
            selected.push(opt.clone());
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

    #[test]
    fn build_card_has_form_and_options() {
        let card = build_question_card(
            "Question 1/2",
            "要继续吗？",
            &["继续".into(), "停止".into()],
            true,
            "补充说明（可选）",
            "提交",
        );
        assert_eq!(card["schema"], "2.0");
        assert_eq!(card["header"]["title"]["content"], "Question 1/2");
        let elements = card["body"]["elements"].as_array().unwrap();
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
    fn build_card_without_options_omits_checkers() {
        let card = build_question_card("", "随便说点什么", &[], false, "请输入", "提交");
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
        let opts = vec!["A".to_string(), "B".to_string(), "C".to_string()];
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
