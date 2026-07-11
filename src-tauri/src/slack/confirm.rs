//! Generic confirm card for Slack (Block Kit).

use crate::confirm::{ActionRole, ConfirmSlot, ConfirmView};
use serde_json::{json, Value};

/// Wire slot action_id values (fixed protocol, not business semantics).
const WIRE_SLOT_PRIMARY: &str = "confirm_ok";
const WIRE_SLOT_SECONDARY: &str = "confirm_cancel";

pub fn build_blocks(view: &ConfirmView) -> (Value, String) {
    let primary_style = match view.confirm.role {
        ActionRole::Primary => Some("primary"),
        ActionRole::Destructive => Some("danger"),
        ActionRole::Default => None,
    };
    let secondary_style = match view.cancel.role {
        ActionRole::Destructive => Some("danger"),
        ActionRole::Primary => Some("primary"),
        ActionRole::Default => None,
    };

    let mut primary_btn = json!({
        "type": "button",
        "text": { "type": "plain_text", "text": truncate(view.confirm_label(), 75) },
        "action_id": WIRE_SLOT_PRIMARY,
        "value": "ok"
    });
    if let Some(style) = primary_style {
        primary_btn["style"] = json!(style);
    }

    let mut secondary_btn = json!({
        "type": "button",
        "text": { "type": "plain_text", "text": truncate(view.cancel_label(), 75) },
        "action_id": WIRE_SLOT_SECONDARY,
        "value": "cancel"
    });
    if let Some(style) = secondary_style {
        secondary_btn["style"] = json!(style);
    }

    let blocks = json!([
        {
            "type": "header",
            "text": { "type": "plain_text", "text": truncate(&view.title, 150), "emoji": true }
        },
        {
            "type": "section",
            "text": { "type": "mrkdwn", "text": truncate(&view.body, 2900) }
        },
        {
            "type": "actions",
            "elements": [primary_btn, secondary_btn]
        }
    ]);
    (blocks, view.title.clone())
}

pub fn build_final_blocks(title: &str, text: &str) -> (Value, String) {
    let blocks = json!([
        {
            "type": "section",
            "text": { "type": "mrkdwn", "text": format!("*{}*\n{}", truncate(title, 100), truncate(text, 2800)) }
        }
    ]);
    (blocks, title.to_string())
}

/// Parse Slack interaction payload → (message_ts, slot).  Non-confirm actions → None.
pub fn parse_confirm_action(payload: &Value) -> Option<(String, ConfirmSlot)> {
    let actions = payload.get("actions")?.as_array()?;
    let act = actions.first()?;
    let id = act.get("action_id")?.as_str()?;
    let slot = match id {
        s if s == WIRE_SLOT_PRIMARY => ConfirmSlot::Primary,
        s if s == WIRE_SLOT_SECONDARY => ConfirmSlot::Secondary,
        _ => return None,
    };
    let ts = payload
        .get("message")
        .and_then(|m| m.get("ts"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    if ts.is_empty() {
        return None;
    }
    Some((ts, slot))
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        format!(
            "{}…",
            s.chars().take(max.saturating_sub(1)).collect::<String>()
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::confirm::{ActionRole, ConfirmAction};

    fn test_view() -> ConfirmView {
        ConfirmView {
            title: "Stage all?".into(),
            body: "3 files changed".into(),
            confirm: ConfirmAction {
                id: "stage_confirm".into(),
                label: "Stage".into(),
                role: ActionRole::Primary,
            },
            cancel: ConfirmAction {
                id: "stage_cancel".into(),
                label: "Cancel".into(),
                role: ActionRole::Default,
            },
        }
    }

    #[test]
    fn blocks_use_role_based_style() {
        let (blocks, _) = build_blocks(&test_view());
        let actions = &blocks[2]["elements"];
        assert_eq!(actions[0]["style"], "primary");
        assert!(actions[1].get("style").is_none());
    }

    #[test]
    fn blocks_with_destructive_cancel() {
        let view = ConfirmView {
            title: "Approve?".into(),
            body: "Run command".into(),
            confirm: ConfirmAction {
                id: "approve".into(),
                label: "Approve".into(),
                role: ActionRole::Primary,
            },
            cancel: ConfirmAction {
                id: "deny".into(),
                label: "Deny".into(),
                role: ActionRole::Destructive,
            },
        };
        let (blocks, _) = build_blocks(&view);
        let actions = &blocks[2]["elements"];
        assert_eq!(actions[0]["style"], "primary");
        assert_eq!(actions[1]["style"], "danger");
    }

    #[test]
    fn parse_primary_slot() {
        let payload = serde_json::json!({
            "actions": [{ "action_id": "confirm_ok", "value": "ok" }],
            "message": { "ts": "12345.6" }
        });
        assert_eq!(
            parse_confirm_action(&payload),
            Some(("12345.6".into(), ConfirmSlot::Primary))
        );
    }

    #[test]
    fn parse_secondary_slot() {
        let payload = serde_json::json!({
            "actions": [{ "action_id": "confirm_cancel", "value": "cancel" }],
            "message": { "ts": "12345.6" }
        });
        assert_eq!(
            parse_confirm_action(&payload),
            Some(("12345.6".into(), ConfirmSlot::Secondary))
        );
    }

    #[test]
    fn parse_unrelated_action_returns_none() {
        let payload = serde_json::json!({
            "actions": [{ "action_id": "select_3", "value": "x" }],
            "message": { "ts": "111.0" }
        });
        assert_eq!(parse_confirm_action(&payload), None);
    }

    #[test]
    fn truncate_short_unchanged() {
        assert_eq!(truncate("hello", 10), "hello");
    }

    #[test]
    fn truncate_long_adds_ellipsis() {
        let result = truncate("abcdef", 4);
        assert_eq!(result, "abc…");
    }
}
