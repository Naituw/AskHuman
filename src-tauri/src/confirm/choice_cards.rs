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
        comment: Option<String>,
    },
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

fn tool_name(request: &ConfirmRequest) -> &str {
    request
        .context
        .iter()
        .find(|field| field.id == "tool")
        .map(|field| field.value.as_str())
        .unwrap_or("Tool")
}

fn feishu_tool_elements(request: &ConfirmRequest) -> Vec<Value> {
    let mut elements = vec![json!({
        "tag": "markdown",
        "content": format!("**{}**", tool_name(request)),
    })];
    if !request.detail.body_md.trim().is_empty() {
        elements.push(json!({
            "tag": "markdown",
            "content": bounded(&request.detail.body_md, 12_000),
        }));
    }
    if !request.detail.summary.trim().is_empty() {
        elements.push(json!({
            "tag": "div",
            "text": {
                "tag": "plain_text",
                "content": request.detail.summary,
                "text_size": "notation",
                "text_color": "grey",
            },
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

pub fn feishu_card(request: &ConfirmRequest, selected: Option<usize>, comment: &str) -> Value {
    let mut elements = feishu_tool_elements(request);
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

pub fn feishu_final_card(request: &ConfirmRequest, status: &str) -> Value {
    let mut elements = feishu_tool_elements(request);
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
                comment,
            })
        }
        _ => None,
    }
}

fn slack_escape(text: &str) -> String {
    crate::slack::markdown::escape(text)
}

pub fn slack_blocks(
    request: &ConfirmRequest,
    selected: Option<usize>,
    comment: &str,
    lang: Lang,
) -> Value {
    let mut blocks = vec![json!({
        "type": "header",
        "text": { "type": "plain_text", "text": bounded(&request.title, 145) },
    })];
    blocks.push(json!({
        "type": "section",
        "text": { "type": "mrkdwn", "text": crate::slack::markdown::to_mrkdwn(&request_markdown(request, 2800)) },
    }));
    for (index, choice) in request.choices.iter().enumerate() {
        let checked = selected == Some(index);
        let mut text = format!(
            "{} *{}*",
            if checked { "●" } else { "○" },
            slack_escape(&choice.label)
        );
        if !choice.description.trim().is_empty() {
            text.push_str(&format!("\n{}", slack_escape(&choice.description)));
        }
        let control_label = if checked {
            "✓".to_string()
        } else {
            (index + 1).to_string()
        };
        let mut button = json!({
            "type": "button",
            "action_id": format!("{SELECT_PREFIX}{index}"),
            "value": index.to_string(),
            "text": { "type": "plain_text", "text": control_label },
        });
        if choice.role == ActionRole::Destructive {
            button["style"] = json!("danger");
        } else if choice.role == ActionRole::Primary {
            button["style"] = json!("primary");
        }
        blocks.push(json!({
            "type": "section",
            "text": { "type": "mrkdwn", "text": text },
            "accessory": button,
        }));
    }
    if let Some(input) = input_for_selected(request, selected) {
        blocks.push(json!({
            "type": "input",
            "block_id": "confirm_reason",
            "optional": true,
            "label": { "type": "plain_text", "text": bounded(&input.label, 1900) },
            "element": {
                "type": "plain_text_input",
                "action_id": input.id,
                "multiline": true,
                "initial_value": bounded(comment, input.max_chars),
                "placeholder": { "type": "plain_text", "text": bounded(&input.placeholder, 145) },
            },
        }));
    }
    let submit = match lang {
        Lang::Zh => request.presentation.submit_label(),
        Lang::En => request.presentation.submit_label(),
    };
    blocks.push(json!({
        "type": "actions",
        "elements": [{
            "type": "button",
            "action_id": SUBMIT_ACTION,
            "value": "submit",
            "style": "primary",
            "text": { "type": "plain_text", "text": bounded(submit, 70) },
        }],
    }));
    Value::Array(blocks)
}

pub fn slack_final_blocks(request: &ConfirmRequest, status: &str) -> Value {
    json!([
        { "type": "header", "text": { "type": "plain_text", "text": bounded(&request.title, 145) } },
        { "type": "section", "text": { "type": "mrkdwn", "text": crate::slack::markdown::to_mrkdwn(&request_markdown(request, 2800)) } },
        { "type": "context", "elements": [{ "type": "mrkdwn", "text": slack_escape(status) }] },
    ])
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
        comment,
    })
}

pub fn telegram_html(
    request: &ConfirmRequest,
    selected: Option<usize>,
    comment: &str,
    status: Option<&str>,
) -> String {
    use crate::telegram::markdown::{escape_html, to_html};
    let mut out = format!("<b>{}</b>", escape_html(&request.title));
    for field in &request.context {
        out.push_str(&format!(
            "\n<b>{}:</b> {}",
            escape_html(&field.label),
            escape_html(&field.value)
        ));
    }
    out.push_str(&format!(
        "\n\n<b>{}</b>",
        escape_html(&request.detail.summary)
    ));
    if !request.detail.body_md.trim().is_empty() {
        out.push_str("\n\n");
        out.push_str(&to_html(&bounded(&request.detail.body_md, 2200)));
    }
    for (index, choice) in request.choices.iter().enumerate() {
        out.push_str(&format!(
            "\n\n{} <b>{}</b>",
            if selected == Some(index) {
                "●"
            } else {
                "○"
            },
            escape_html(&choice.label)
        ));
        if !choice.description.trim().is_empty() {
            out.push_str(&format!("\n<i>{}</i>", escape_html(&choice.description)));
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
    if let Some(status) = status {
        out.push_str(&format!("\n\n<i>{}</i>", escape_html(status)));
    }
    bounded(&out, 3900)
}

pub fn telegram_keyboard(request: &ConfirmRequest, selected: Option<usize>) -> Value {
    let mut rows: Vec<Value> = request
        .choices
        .iter()
        .enumerate()
        .map(|(index, _)| {
            json!([{ "text": if selected == Some(index) { format!("✓ {}", index + 1) } else { (index + 1).to_string() }, "callback_data": format!("pc:s:{index}") }])
        })
        .collect();
    rows.push(
        json!([{ "text": request.presentation.submit_label(), "callback_data": "pc:submit" }]),
    );
    json!({ "inline_keyboard": rows })
}

pub fn parse_telegram_callback(data: &str) -> Option<Option<usize>> {
    if data == "pc:submit" {
        return Some(None);
    }
    data.strip_prefix("pc:s:")
        .and_then(|value| value.parse().ok())
        .map(Some)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{
        ConfirmChoice, ConfirmDetail, ConfirmField, ConfirmFieldKind, ConfirmPresentation,
        ConfirmSpec,
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
                input: None,
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
        let card = feishu_card(&request(), Some(1), "");
        let elements = card["body"]["elements"].as_array().unwrap();
        assert_eq!(elements[0]["tag"], "div");
        assert_eq!(elements[0]["text"]["content"], "Permission");
        assert_eq!(elements[2]["tag"], "markdown");
        assert_eq!(elements[2]["content"], "**Bash**");
        assert!(elements[3]["content"]
            .as_str()
            .unwrap()
            .contains("git status"));
        assert_eq!(elements[4]["text"]["content"], "Run");
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
        let card = feishu_final_card(&request(), "Submitted");
        let text = card.to_string();
        assert!(text.contains("**Bash**"));
        assert!(text.contains("git status"));
        assert!(text.contains("Submitted"));
        assert!(!text.contains("**Tool:**"));
        assert!(!text.contains("confirm_choice_"));
    }

    #[test]
    fn slack_uses_short_controls_and_full_static_labels() {
        let blocks = slack_blocks(&request(), None, "", Lang::En);
        let text = blocks.to_string();
        assert!(text.contains(&"A".repeat(100)));
        assert!(text.contains("confirm_select_0"));
        assert!(!text.contains(&format!("\\\"text\\\":\\\"{}\\\"", "A".repeat(100))));
    }

    #[test]
    fn telegram_callbacks_carry_only_wire_indices() {
        assert_eq!(parse_telegram_callback("pc:s:7"), Some(Some(7)));
        assert_eq!(parse_telegram_callback("pc:submit"), Some(None));
        assert_eq!(parse_telegram_callback("approve_once"), None);
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
