//! Pure structured-confirmation card builders and callback parsers.
//!
//! Interactive controls carry only short wire indices. Full labels/descriptions remain in static
//! content so platform control limits cannot truncate the security-relevant permission scope.

use crate::confirm::ActionRole;
use crate::i18n::Lang;
use crate::models::{ConfirmFieldKind, ConfirmRequest};
use serde_json::{json, Value};

const SELECT_PREFIX: &str = "confirm_select_";
const SUBMIT_ACTION: &str = "confirm_submit";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CardAction {
    Select {
        actor: String,
        message_id: String,
        index: usize,
        comment: Option<String>,
    },
    Submit {
        actor: String,
        message_id: String,
        index: Option<usize>,
        comment: Option<String>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TelegramAction {
    Decide(usize),
}

fn bounded(input: &str, max: usize) -> String {
    let mut chars = input.chars();
    let head: String = chars.by_ref().take(max).collect();
    if chars.next().is_some() {
        format!("{head}\n\n… [truncated]")
    } else {
        head
    }
}

fn context_markdown(request: &ConfirmRequest) -> String {
    request
        .context
        .iter()
        .map(|field| {
            let value = match field.kind {
                ConfirmFieldKind::Path => format!("`{}`", field.value.replace('`', "\\`")),
                _ => field.value.clone(),
            };
            format!("**{}:** {}", field.label, value)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn reason_label(lang: Lang) -> &'static str {
    match lang {
        Lang::Zh => "理由：",
        Lang::En => "Reason:",
    }
}

pub(crate) fn compact_tool_markdown(request: &ConfirmRequest, max: usize, lang: Lang) -> String {
    let mut body = String::new();
    if !request.detail.summary.trim().is_empty() {
        body.push_str(&format!(
            "**{}** {}\n\n",
            reason_label(lang),
            request.detail.summary
        ));
    }
    body.push_str(&format!("**{}**", tool_name(request)));
    if !request.detail.body_md.trim().is_empty() {
        body.push_str("\n\n");
        body.push_str(&request.detail.body_md);
    }
    bounded(&body, max)
}

pub(crate) fn request_markdown(request: &ConfirmRequest, max: usize) -> String {
    let mut body = context_markdown(request);
    if !body.is_empty() {
        body.push_str("\n\n");
    }
    body.push_str(&format!("**{}**", request.detail.summary));
    if !request.detail.body_md.trim().is_empty() {
        body.push_str("\n\n");
        body.push_str(&request.detail.body_md);
    }
    bounded(&body, max)
}

fn input_for_selected(
    request: &ConfirmRequest,
    selected: Option<usize>,
) -> Option<&crate::models::ConfirmInput> {
    let selected_id = selected
        .and_then(|index| request.choices.get(index))
        .map(|choice| choice.id.as_str());
    request
        .presentation
        .input()
        .filter(|input| selected_id == Some(input.visible_when_action_id.as_str()))
}

pub(crate) fn tool_name(request: &ConfirmRequest) -> &str {
    request
        .context
        .iter()
        .find(|field| field.id == "tool")
        .map(|field| field.value.as_str())
        .unwrap_or("Tool")
}

fn feishu_tool_elements(request: &ConfirmRequest, lang: Lang) -> Vec<Value> {
    let mut elements = Vec::new();
    if !request.detail.summary.trim().is_empty() {
        elements.push(json!({
            "tag": "markdown",
            "content": format!("**{}** {}", reason_label(lang), request.detail.summary),
        }));
    }
    elements.push(json!({
        "tag": "markdown",
        "content": format!("**{}**", tool_name(request)),
    }));
    if !request.detail.body_md.trim().is_empty() {
        elements.push(json!({
            "tag": "markdown",
            "content": bounded(&request.detail.body_md, 12_000),
        }));
    }
    elements
}

fn feishu_choice_text(choice: &crate::models::ConfirmChoice) -> String {
    if choice.description.trim().is_empty() {
        choice.label.clone()
    } else {
        format!(
            "**{}**\n<font color='grey'>{}</font>",
            choice.label, choice.description
        )
    }
}

pub fn feishu_card(
    request: &ConfirmRequest,
    selected: Option<usize>,
    comment: &str,
    lang: Lang,
) -> Value {
    let mut elements = feishu_tool_elements(request, lang);
    elements.push(json!({ "tag": "hr", "margin": "0px 0px 0px 0px" }));
    for (index, choice) in request.choices.iter().enumerate() {
        let checked = selected == Some(index);
        let color = if checked {
            Some(if choice.role == ActionRole::Destructive {
                "red"
            } else {
                "blue"
            })
        } else {
            None
        };
        elements.push(crate::feishu::card::styled_checker(
            &format!("confirm_choice_{index}"),
            &feishu_choice_text(choice),
            checked,
            false,
            Some(json!({ "confirm": "select", "index": index })),
            color,
        ));
    }
    let mut form_elements = Vec::new();
    if let Some(input) = input_for_selected(request, selected) {
        form_elements.push(json!({
            "tag": "input",
            "name": input.id,
            "label": { "tag": "plain_text", "content": input.label },
            "placeholder": { "tag": "plain_text", "content": input.placeholder },
            "default_value": bounded(comment, input.max_chars),
        }));
    }
    form_elements.push(json!({
        "tag": "button",
        "name": "confirm_submit",
        "form_action_type": "submit",
        "type": "primary",
        "disabled": selected.is_none(),
        "text": { "tag": "plain_text", "content": request.presentation.submit_label() },
        "behaviors": [{ "type": "callback", "value": { "confirm": "submit" } }],
    }));
    elements.push(json!({ "tag": "form", "name": "confirm_form", "elements": form_elements }));
    crate::feishu::card::assemble_styled_card(&request.title, elements)
}

pub fn feishu_final_card(request: &ConfirmRequest, status: &str, lang: Lang) -> Value {
    let mut elements = feishu_tool_elements(request, lang);
    elements.push(json!({ "tag": "hr", "margin": "0px 0px 0px 0px" }));
    elements.push(json!({
        "tag": "div",
        "text": {
            "tag": "plain_text",
            "content": status,
            "text_size": "notation",
            "text_color": "grey",
        },
    }));
    crate::feishu::card::assemble_styled_card(&request.title, elements)
}

fn value_object(value: &Value) -> Option<Value> {
    match value {
        Value::String(text) => serde_json::from_str(text).ok(),
        other => Some(other.clone()),
    }
}

pub fn parse_feishu_action(event: &Value, input_id: Option<&str>) -> Option<CardAction> {
    let actor = event.get("operator")?.get("open_id")?.as_str()?.to_string();
    let message_id = event
        .get("context")?
        .get("open_message_id")?
        .as_str()?
        .to_string();
    let action = event.get("action")?;
    let value = value_object(action.get("value")?)?;
    match value.get("confirm").and_then(Value::as_str)? {
        "select" => Some(CardAction::Select {
            actor,
            message_id,
            index: value.get("index")?.as_u64()? as usize,
            comment: input_id
                .and_then(|id| action.get("form_value").and_then(|form| form.get(id)))
                .and_then(Value::as_str)
                .map(str::trim)
                .map(str::to_string),
        }),
        "submit" => {
            let comment = input_id
                .and_then(|id| action.get("form_value").and_then(|form| form.get(id)))
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|text| !text.is_empty())
                .map(str::to_string);
            Some(CardAction::Submit {
                actor,
                message_id,
                index: None,
                comment,
            })
        }
        _ => None,
    }
}

fn slack_escape(text: &str) -> String {
    crate::slack::markdown::escape(text)
}

fn slack_control_text(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut out: String = text.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

pub fn slack_blocks(
    request: &ConfirmRequest,
    selected: Option<usize>,
    comment: &str,
    lang: Lang,
) -> Value {
    let mut blocks = vec![crate::slack::blockkit::title_block(&request.title)];
    if !request.detail.summary.trim().is_empty() {
        blocks.push(crate::slack::blockkit::mrkdwn_section(&format!(
            "*{}* {}",
            slack_escape(reason_label(lang)),
            slack_escape(&request.detail.summary)
        )));
    }
    blocks.push(crate::slack::blockkit::mrkdwn_section(&format!(
        "*{}*",
        slack_escape(tool_name(request))
    )));
    if !request.detail.body_md.trim().is_empty() {
        blocks.push(crate::slack::blockkit::mrkdwn_section(
            &crate::slack::markdown::to_mrkdwn(&bounded(&request.detail.body_md, 2800)),
        ));
    }

    // Security-relevant scope details stay static and untruncated by radio option limits.
    for choice in &request.choices {
        let inline_detail = choice.description.replace('\n', " · ");
        if inline_detail.chars().count() > 75 || choice.label.chars().count() > 75 {
            let mut detail = format!("*{}*", slack_escape(&choice.label));
            if !choice.description.trim().is_empty() {
                detail.push_str(&format!("\n{}", slack_escape(&choice.description)));
            }
            blocks.push(crate::slack::blockkit::mrkdwn_section(&detail));
        }
    }

    let options: Vec<Value> = request
        .choices
        .iter()
        .enumerate()
        .map(|(index, choice)| {
            let mut option = json!({
                "text": { "type": "plain_text", "text": slack_control_text(&choice.label, 75) },
                "value": index.to_string(),
            });
            if !choice.description.trim().is_empty() {
                option["description"] = json!({
                    "type": "plain_text",
                    "text": slack_control_text(&choice.description.replace('\n', " · "), 75),
                });
            }
            option
        })
        .collect();
    let mut radio = json!({
        "type": "radio_buttons",
        "action_id": "confirm_choice",
        "options": options,
    });
    if let Some(index) = selected.filter(|index| *index < request.choices.len()) {
        radio["initial_option"] = radio["options"][index].clone();
    }
    blocks.push(json!({
        "type": "input",
        "block_id": "confirm_choice_block",
        "optional": true,
        "label": {
            "type": "plain_text",
            "text": if lang == Lang::Zh { "决定" } else { "Decision" },
        },
        "element": radio,
    }));

    if let Some(input) = request.presentation.input() {
        blocks.push(json!({
            "type": "input",
            "block_id": "confirm_reason",
            "optional": true,
            "label": {
                "type": "plain_text",
                "text": if lang == Lang::Zh {
                    "拒绝原因（可选，仅拒绝时发送）"
                } else {
                    "Denial reason (optional; sent only when denying)"
                },
            },
            "element": {
                "type": "plain_text_input",
                "action_id": input.id,
                "multiline": true,
                "initial_value": bounded(comment, input.max_chars),
                "placeholder": { "type": "plain_text", "text": bounded(&input.placeholder, 145) },
            },
        }));
    }
    blocks.push(json!({
        "type": "actions",
        "elements": [{
            "type": "button",
            "action_id": SUBMIT_ACTION,
            "value": "submit",
            "style": "primary",
            "text": { "type": "plain_text", "text": bounded(request.presentation.submit_label(), 70) },
        }],
    }));
    Value::Array(blocks)
}

pub fn slack_final_blocks(request: &ConfirmRequest, status: &str, lang: Lang) -> Value {
    let mut blocks = vec![crate::slack::blockkit::title_block(&request.title)];
    if !request.detail.summary.trim().is_empty() {
        blocks.push(crate::slack::blockkit::mrkdwn_section(&format!(
            "*{}* {}",
            slack_escape(reason_label(lang)),
            slack_escape(&request.detail.summary)
        )));
    }
    blocks.push(crate::slack::blockkit::mrkdwn_section(&format!(
        "*{}*",
        slack_escape(tool_name(request))
    )));
    if !request.detail.body_md.trim().is_empty() {
        blocks.push(crate::slack::blockkit::mrkdwn_section(
            &crate::slack::markdown::to_mrkdwn(&bounded(&request.detail.body_md, 2800)),
        ));
    }
    blocks.push(json!({
        "type": "context",
        "elements": [{ "type": "mrkdwn", "text": slack_escape(status) }],
    }));
    Value::Array(blocks)
}

pub fn parse_slack_action(payload: &Value, input_id: Option<&str>) -> Option<CardAction> {
    let actor = payload.get("user")?.get("id")?.as_str()?.to_string();
    let message_id = payload
        .get("container")
        .and_then(|container| container.get("message_ts"))
        .or_else(|| payload.get("message").and_then(|message| message.get("ts")))?
        .as_str()?
        .to_string();
    let action_id = payload
        .get("actions")?
        .as_array()?
        .first()?
        .get("action_id")?
        .as_str()?;
    if let Some(index) = action_id
        .strip_prefix(SELECT_PREFIX)
        .and_then(|value| value.parse().ok())
    {
        return Some(CardAction::Select {
            actor,
            message_id,
            index,
            comment: input_id
                .and_then(|id| {
                    payload
                        .get("state")?
                        .get("values")?
                        .as_object()?
                        .values()
                        .find_map(|actions| actions.get(id)?.get("value")?.as_str())
                })
                .map(str::trim)
                .map(str::to_string),
        });
    }
    if action_id != SUBMIT_ACTION {
        return None;
    }
    let index = payload
        .get("state")
        .and_then(|state| state.get("values"))
        .and_then(Value::as_object)
        .and_then(|blocks| {
            blocks.values().find_map(|actions| {
                actions
                    .get("confirm_choice")?
                    .get("selected_option")?
                    .get("value")?
                    .as_str()?
                    .parse::<usize>()
                    .ok()
            })
        });
    let comment = input_id
        .and_then(|id| {
            payload
                .get("state")?
                .get("values")?
                .as_object()
                .and_then(|blocks| {
                    blocks
                        .values()
                        .find_map(|actions| actions.get(id)?.get("value")?.as_str())
                })
        })
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_string);
    Some(CardAction::Submit {
        actor,
        message_id,
        index,
        comment,
    })
}

pub fn telegram_html(
    request: &ConfirmRequest,
    selected: Option<usize>,
    comment: &str,
    status: Option<&str>,
    lang: Lang,
) -> String {
    use crate::telegram::markdown::{escape_html, to_html};
    let mut out = format!("<b>❓ {}</b>", escape_html(&request.title));
    if !request.detail.summary.trim().is_empty() {
        out.push_str(&format!(
            "\n\n<b>{}</b> {}",
            escape_html(reason_label(lang)),
            escape_html(&request.detail.summary)
        ));
    }
    out.push_str(&format!("\n\n<b>{}</b>", escape_html(tool_name(request))));
    if !request.detail.body_md.trim().is_empty() {
        out.push_str("\n\n");
        out.push_str(&to_html(&bounded(&request.detail.body_md, 2200)));
    }
    let full_labels = telegram_uses_full_labels(request);
    let show_choice_list = !full_labels
        || request
            .choices
            .iter()
            .any(|choice| !choice.description.trim().is_empty());
    if show_choice_list {
        for (index, choice) in request.choices.iter().enumerate() {
            let marker = if full_labels {
                "•".to_string()
            } else {
                crate::channels::telegram::option_label(index)
            };
            out.push_str(&format!(
                "\n\n{} <b>{}</b>",
                marker,
                escape_html(&choice.label)
            ));
            if !choice.description.trim().is_empty() {
                out.push_str(&format!("\n<i>{}</i>", escape_html(&choice.description)));
            }
        }
    }
    if let Some(input) = input_for_selected(request, selected) {
        if !comment.trim().is_empty() {
            out.push_str(&format!(
                "\n\n<b>{}:</b> {}",
                escape_html(&input.label),
                escape_html(&bounded(comment, input.max_chars))
            ));
        }
    }
    if status.is_none() && comment.trim().is_empty() && request.presentation.input().is_some() {
        let hint = match lang {
            Lang::Zh => "如需说明拒绝原因，请先回复本消息，再点“拒绝”。",
            Lang::En => "To include a denial reason, reply to this message before tapping Deny.",
        };
        out.push_str(&format!("\n\n<i>{}</i>", escape_html(hint)));
    }
    if let Some(status) = status {
        out.push_str(&format!("\n\n<i>{}</i>", escape_html(status)));
    }
    bounded(&out, 3900)
}

fn telegram_uses_full_labels(request: &ConfirmRequest) -> bool {
    request.choices.len() <= 3
        && request
            .choices
            .iter()
            .all(|choice| choice.label.chars().count() <= 12)
        && request
            .choices
            .iter()
            .map(|choice| choice.label.chars().count())
            .sum::<usize>()
            <= 24
}

pub fn telegram_keyboard(request: &ConfirmRequest, selected: Option<usize>) -> Value {
    let indices: Vec<usize> = (0..request.choices.len()).collect();
    let full_labels = telegram_uses_full_labels(request);
    let width = if full_labels {
        request.choices.len().max(1)
    } else {
        crate::channels::telegram::KEYBOARD_ROW_WIDTH
    };
    let rows = indices
        .chunks(width)
        .map(|indices| {
            Value::Array(
            indices
                .iter()
                .map(|index| {
                    let label = if full_labels {
                        request.choices[*index].label.clone()
                    } else {
                        crate::channels::telegram::option_label(*index)
                    };
                    json!({
                        "text": if selected == Some(*index) { format!("✅ {label}") } else { label },
                        "callback_data": format!("pc:do:{index}"),
                    })
                })
                .collect(),
        )
        })
        .collect::<Vec<_>>();
    json!({ "inline_keyboard": rows })
}

pub fn parse_telegram_callback(data: &str) -> Option<TelegramAction> {
    data.strip_prefix("pc:do:")
        .and_then(|value| value.parse().ok())
        .map(TelegramAction::Decide)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{
        ConfirmChoice, ConfirmDetail, ConfirmField, ConfirmFieldKind, ConfirmInput,
        ConfirmPresentation, ConfirmSpec,
    };

    fn request() -> ConfirmRequest {
        ConfirmSpec {
            title: "Permission".into(),
            context: vec![ConfirmField {
                id: "tool".into(),
                label: "Tool".into(),
                value: "Bash".into(),
                kind: ConfirmFieldKind::Text,
            }],
            detail: ConfirmDetail {
                summary: "Run".into(),
                body_md: "```sh\ngit status\n```".into(),
            },
            choices: vec![
                ConfirmChoice {
                    id: "approve_once".into(),
                    label: "A".repeat(100),
                    description: "full scope".into(),
                    role: ActionRole::Primary,
                },
                ConfirmChoice {
                    id: "deny".into(),
                    label: "Deny".into(),
                    description: String::new(),
                    role: ActionRole::Destructive,
                },
            ],
            presentation: ConfirmPresentation::SingleSelectSubmit {
                input: Some(ConfirmInput {
                    id: "reason".into(),
                    visible_when_action_id: "deny".into(),
                    label: "Reason".into(),
                    placeholder: "Tell the Agent what it should do".into(),
                    max_chars: 1000,
                }),
                submit_label: "Submit".into(),
                default_action_id: None,
            },
            dismiss_action_id: "deny".into(),
        }
        .into_request("r1".into(), 1, 2)
        .unwrap()
    }

    #[test]
    fn feishu_uses_ask_checker_and_tool_first_hierarchy() {
        let card = feishu_card(&request(), Some(1), "", Lang::En);
        let elements = card["body"]["elements"].as_array().unwrap();
        assert_eq!(elements[0]["tag"], "div");
        assert_eq!(elements[0]["text"]["content"], "Permission");
        assert_eq!(elements[2]["tag"], "markdown");
        assert_eq!(elements[2]["content"], "**Reason:** Run");
        assert_eq!(elements[3]["content"], "**Bash**");
        assert!(elements[4]["content"]
            .as_str()
            .unwrap()
            .contains("git status"));
        let checkers: Vec<&Value> = elements
            .iter()
            .filter(|element| element["tag"] == "checker")
            .collect();
        assert_eq!(checkers.len(), 2);
        assert_eq!(checkers[0]["checked"], false);
        assert_eq!(checkers[0]["behaviors"][0]["value"]["confirm"], "select");
        assert_eq!(checkers[1]["checked"], true);
        assert_eq!(checkers[1]["text"]["text_color"], "red");
        assert!(elements
            .iter()
            .all(|element| element["tag"] != "column_set"));
        assert!(!card.to_string().contains("**Tool:**"));
    }

    #[test]
    fn feishu_final_keeps_compact_tool_hierarchy_without_context() {
        let card = feishu_final_card(&request(), "Submitted", Lang::En);
        let text = card.to_string();
        assert!(text.contains("**Bash**"));
        assert!(text.contains("git status"));
        assert!(text.contains("Submitted"));
        assert!(!text.contains("**Tool:**"));
        assert!(!text.contains("confirm_choice_"));
    }

    #[test]
    fn slack_uses_native_radio_full_labels_and_static_scope_details() {
        let blocks = slack_blocks(&request(), None, "", Lang::En);
        let text = blocks.to_string();
        assert!(text.contains(&"A".repeat(100)));
        let blocks = blocks.as_array().unwrap();
        assert_eq!(blocks[0]["text"]["text"], "❓ Permission");
        let radio = blocks
            .iter()
            .find(|block| block["element"]["type"] == "radio_buttons")
            .unwrap();
        assert_eq!(radio["element"]["options"].as_array().unwrap().len(), 2);
        assert!(
            radio["element"]["options"][0]["text"]["text"]
                .as_str()
                .unwrap()
                .chars()
                .count()
                <= 75
        );
        assert_eq!(
            radio["element"]["options"][0]["description"]["text"],
            "full scope"
        );
        let reason = blocks
            .iter()
            .find(|block| block["element"]["type"] == "plain_text_input")
            .unwrap();
        assert_eq!(
            reason["label"]["text"],
            "Denial reason (optional; sent only when denying)"
        );
        assert!(blocks
            .iter()
            .all(|block| block["accessory"]["type"] != "button"));
    }

    #[test]
    fn slack_multi_rule_choice_uses_compact_label_and_inline_rule_details() {
        let mut request = request();
        request.choices[0].label = "Always allow (Project): 2 rules".into();
        request.choices[0].description = "Bash: git status\nRead: entire tool".into();
        let blocks = slack_blocks(&request, None, "", Lang::En);
        let blocks = blocks.as_array().unwrap();
        let radio = blocks
            .iter()
            .find(|block| block["element"]["type"] == "radio_buttons")
            .unwrap();
        assert_eq!(
            radio["element"]["options"][0]["text"]["text"],
            "Always allow (Project): 2 rules"
        );
        assert_eq!(
            radio["element"]["options"][0]["description"]["text"],
            "Bash: git status · Read: entire tool"
        );
        let text = Value::Array(blocks.clone()).to_string();
        assert!(text.contains("Bash: git status"));
        assert!(text.contains("Read: entire tool"));
    }

    #[test]
    fn slack_submit_reads_native_radio_and_reason() {
        let payload = json!({
            "user": { "id": "U1" },
            "container": { "message_ts": "1.2" },
            "actions": [{ "action_id": "confirm_submit" }],
            "state": { "values": {
                "confirm_choice_block": {
                    "confirm_choice": {
                        "selected_option": { "value": "1" }
                    }
                },
                "confirm_reason": {
                    "reason": { "value": " use read-only mode " }
                }
            } }
        });
        assert_eq!(
            parse_slack_action(&payload, Some("reason")),
            Some(CardAction::Submit {
                actor: "U1".into(),
                message_id: "1.2".into(),
                index: Some(1),
                comment: Some("use read-only mode".into()),
            })
        );
    }

    #[test]
    fn telegram_callbacks_carry_only_wire_indices() {
        assert_eq!(
            parse_telegram_callback("pc:do:7"),
            Some(TelegramAction::Decide(7))
        );
        assert_eq!(parse_telegram_callback("pc:s:7"), None);
        assert_eq!(parse_telegram_callback("pc:submit"), None);
        assert_eq!(parse_telegram_callback("approve_once"), None);
    }

    #[test]
    fn telegram_uses_tool_first_hierarchy_and_ask_keycaps() {
        let request = request();
        let html = telegram_html(&request, Some(1), "", None, Lang::En);
        assert!(html.starts_with("<b>❓ Permission</b>"));
        let summary = html.find("<b>Reason:</b> Run").unwrap();
        let tool = html.find("<b>Bash</b>").unwrap();
        let body = html.find("git status").unwrap();
        assert!(summary < tool && tool < body);
        assert!(!html.contains("<b>Tool:</b>"));
        let keyboard = telegram_keyboard(&request, Some(1));
        let rows = keyboard["inline_keyboard"].as_array().unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].as_array().unwrap().len(), 2);
        assert_eq!(rows[0][0]["text"], "1️⃣");
        assert_eq!(rows[0][1]["text"], "✅ 2️⃣");
        assert_eq!(rows[0][1]["callback_data"], "pc:do:1");
        assert!(html.contains("reply to this message before tapping Deny"));
    }

    #[test]
    fn telegram_short_choices_use_direct_full_label_buttons() {
        let mut request = request();
        request.choices[0].label = "Approve".into();
        request.choices[0].description.clear();
        let html = telegram_html(&request, None, "", None, Lang::En);
        assert!(!html.contains("1️⃣ <b>Approve</b>"));
        let keyboard = telegram_keyboard(&request, None);
        let rows = keyboard["inline_keyboard"].as_array().unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0][0]["text"], "Approve");
        assert_eq!(rows[0][1]["text"], "Deny");
        assert_eq!(rows[0][0]["callback_data"], "pc:do:0");
    }

    #[test]
    fn feishu_parser_never_accepts_action_ids() {
        let event = json!({
            "operator": { "open_id": "u1" },
            "context": { "open_message_id": "m1" },
            "action": { "value": { "confirm": "select", "index": 1 } },
        });
        assert_eq!(
            parse_feishu_action(&event, None),
            Some(CardAction::Select {
                actor: "u1".into(),
                message_id: "m1".into(),
                index: 1,
                comment: None,
            })
        );
    }
}
