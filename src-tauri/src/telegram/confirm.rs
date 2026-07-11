//! Generic confirm card for Telegram (HTML + inline keyboard).

use crate::confirm::{ConfirmSlot, ConfirmView};
use crate::telegram::markdown;
use serde_json::{json, Value};

/// Wire slot callback_data values (fixed protocol, not business semantics).
const WIRE_SLOT_PRIMARY: &str = "confirm:ok";
const WIRE_SLOT_SECONDARY: &str = "confirm:cancel";

pub fn build_html(view: &ConfirmView) -> String {
    format!(
        "<b>{}</b>\n\n{}",
        markdown::escape_html(&view.title),
        markdown::escape_html(&view.body)
    )
}

pub fn inline_keyboard(view: &ConfirmView) -> Value {
    json!({
        "inline_keyboard": [[
            { "text": view.confirm_label(), "callback_data": WIRE_SLOT_PRIMARY },
            { "text": view.cancel_label(), "callback_data": WIRE_SLOT_SECONDARY }
        ]]
    })
}

/// Parse callback_data → wire slot.  Non-confirm callbacks → None.
pub fn parse_confirm_action(data: &str) -> Option<ConfirmSlot> {
    match data {
        s if s == WIRE_SLOT_PRIMARY => Some(ConfirmSlot::Primary),
        s if s == WIRE_SLOT_SECONDARY => Some(ConfirmSlot::Secondary),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::confirm::{ActionRole, ConfirmAction};

    fn test_view() -> ConfirmView {
        ConfirmView {
            title: "Stage all?".into(),
            body: "3 files".into(),
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
    fn html_contains_title_and_body() {
        let html = build_html(&test_view());
        assert!(html.contains("<b>Stage all?</b>"));
        assert!(html.contains("3 files"));
    }

    #[test]
    fn keyboard_uses_labels() {
        let kb = inline_keyboard(&test_view());
        let btns = kb["inline_keyboard"][0].as_array().unwrap();
        assert_eq!(btns[0]["text"], "Stage");
        assert_eq!(btns[1]["text"], "Cancel");
    }

    #[test]
    fn parse_primary_slot() {
        assert_eq!(
            parse_confirm_action("confirm:ok"),
            Some(ConfirmSlot::Primary)
        );
    }

    #[test]
    fn parse_secondary_slot() {
        assert_eq!(
            parse_confirm_action("confirm:cancel"),
            Some(ConfirmSlot::Secondary)
        );
    }

    #[test]
    fn parse_unrelated_returns_none() {
        assert_eq!(parse_confirm_action("sel:3"), None);
        assert_eq!(parse_confirm_action(""), None);
    }
}
