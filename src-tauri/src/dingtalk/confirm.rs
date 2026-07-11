//! 钉钉通用确认卡（互动卡片高级版）。
//!
//! 模板：`docs/assets/dingtalk-confirm-card-template.json`  
//! 变量：`title` / `markdown` / `btn_primary` / `btn_secondary` / `finalized` / `final_label`  
//! 按钮 actionId：`confirm_ok` / `confirm_cancel`（固定 wire slot，不代表业务语义）  
//! 解析方式与 `watch::parse_watch_action` 同构（content JSON 字符串 + actionIds）。

use crate::confirm::{ConfirmSlot, ConfirmView};
use serde_json::{json, Value};

/// 内置默认确认卡模板 ID。
pub const DEFAULT_CONFIRM_CARD_TEMPLATE_ID: &str = "2f07e765-6e46-4fca-8b95-36888f175dcb.schema";

/// Wire slot action IDs (fixed by the published DingTalk template).
const WIRE_SLOT_PRIMARY: &str = "confirm_ok";
const WIRE_SLOT_SECONDARY: &str = "confirm_cancel";

pub fn build_param_map(view: &ConfirmView) -> Value {
    json!({
        "title": view.title,
        "markdown": view.body,
        "btn_primary": view.confirm_label(),
        "btn_secondary": view.cancel_label(),
        "finalized": "false",
        "final_label": "",
    })
}

pub fn build_final_param_map(title: &str, body: &str, final_label: &str) -> Value {
    json!({
        "title": title,
        "markdown": body,
        "btn_primary": "",
        "btn_secondary": "",
        "finalized": "true",
        "final_label": final_label,
    })
}

/// 解析确认回调 → (outTrackId, slot)。非本卡按钮 → None。
pub fn parse_confirm_action(data: &Value) -> Option<(String, ConfirmSlot)> {
    let otid = data
        .get("outTrackId")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())?
        .to_string();
    let inner: Value = match data.get("content").or_else(|| data.get("value"))? {
        Value::String(s) => serde_json::from_str(s).ok()?,
        other => other.clone(),
    };
    let action = inner
        .get("cardPrivateData")
        .and_then(|p| p.get("actionIds"))
        .and_then(|a| a.as_array())
        .and_then(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str())
                .find(|id| *id == WIRE_SLOT_PRIMARY || *id == WIRE_SLOT_SECONDARY)
        })?;
    let slot = if action == WIRE_SLOT_PRIMARY {
        ConfirmSlot::Primary
    } else {
        ConfirmSlot::Secondary
    };
    Some((otid, slot))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_confirm_roundtrip() {
        let data = json!({
            "outTrackId": "c1",
            "content": "{\"cardPrivateData\":{\"actionIds\":[\"confirm_ok\"],\"params\":{}}}",
        });
        assert_eq!(
            parse_confirm_action(&data),
            Some(("c1".into(), ConfirmSlot::Primary))
        );
        let data = json!({
            "outTrackId": "c1",
            "content": "{\"cardPrivateData\":{\"actionIds\":[\"confirm_cancel\"],\"params\":{}}}",
        });
        assert_eq!(
            parse_confirm_action(&data),
            Some(("c1".into(), ConfirmSlot::Secondary))
        );
        let ask = json!({
            "outTrackId": "a1",
            "content": "{\"cardPrivateData\":{\"actionIds\":[\"submit_action\"],\"params\":{}}}",
        });
        assert_eq!(parse_confirm_action(&ask), None);
    }
}
