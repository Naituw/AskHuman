//! Codex heartbeat results: notification intent and readable text from the same output.

#[derive(Debug, PartialEq, Eq)]
pub struct HeartbeatOutput<'a> {
    pub notify: bool,
    pub display_text: &'a str,
}

/// Interpret a complete result envelope, not a heartbeat mentioned inside ordinary prose.
/// Codex's template permits prose before NOTIFY, but DONT_NOTIFY occupies the whole reply.
pub fn parse(text: &str) -> Option<HeartbeatOutput<'_>> {
    let text = text.trim();
    let lower = text.to_ascii_lowercase();
    let start = lower.rfind("<heartbeat>")?;
    let mut body = text[..start].trim_end();
    let mut block = text[start..].trim_end();
    if block.ends_with("```") {
        block = block.strip_suffix("```")?.trim_end();
        let fence = body.rsplit_once('\n').map_or(body, |(_, line)| line);
        if !matches!(fence.to_ascii_lowercase().as_str(), "```" | "```xml") {
            return None;
        }
        body = body[..body.len() - fence.len()].trim_end();
    }
    if !block.to_ascii_lowercase().ends_with("</heartbeat>")
        || body.contains("```")
        || body.to_ascii_lowercase().contains("<heartbeat>")
    {
        return None;
    }
    let lower = block.to_ascii_lowercase();
    let field = |name| {
        let open = format!("<{name}>");
        let close = format!("</{name}>");
        let start = lower.find(&open)? + open.len();
        let end = start + lower[start..].find(&close)?;
        Some(block[start..end].trim())
    };
    let notify = match field("decision")?.to_ascii_uppercase().as_str() {
        "NOTIFY" => true,
        "DONT_NOTIFY" if body.is_empty() => false,
        _ => return None,
    };
    let message = field("message")?;
    let message = message
        .strip_prefix("<![CDATA[")
        .and_then(|text| text.strip_suffix("]]>"))
        .unwrap_or(message)
        .trim();
    let display_text = if body.is_empty() { message } else { body };
    (!display_text.is_empty()).then_some(HeartbeatOutput {
        notify,
        display_text,
    })
}

pub fn display_text(text: &str) -> &str {
    parse(text).map_or(text, |output| output.display_text)
}

#[cfg(test)]
pub const QUIET: &str = "<heartbeat>\n<automation_id>gerrit-186127</automation_id>\n<decision>DONT_NOTIFY</decision>\n<message>重跑已进入 Unit 测试阶段，目前无需你处理，继续静默跟踪。</message>\n</heartbeat>";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heartbeat_quiet_result_and_client_wrappers() {
        for text in [
            QUIET.to_string(),
            format!("```xml\n{QUIET}\n```"),
            QUIET
                .replace("<message>", "<message><![CDATA[")
                .replace("</message>", "]]></message>"),
        ] {
            let result = parse(&text).unwrap();
            assert!(!result.notify);
            assert_eq!(
                result.display_text,
                "重跑已进入 Unit 测试阶段，目前无需你处理，继续静默跟踪。"
            );
        }
    }

    #[test]
    fn heartbeat_notify_preserves_prose_or_uses_message() {
        let notify = QUIET.replace("DONT_NOTIFY", "NOTIFY");
        assert!(parse(&notify).unwrap().notify);
        for text in [
            format!("Tests passed.\n{notify}"),
            format!("Tests passed.\n```xml\n{notify}\n```"),
        ] {
            assert_eq!(parse(&text).unwrap().display_text, "Tests passed.");
        }
        let text = notify.replace(
            "重跑已进入 Unit 测试阶段，目前无需你处理，继续静默跟踪。",
            "Build & tests passed <today>",
        );
        assert_eq!(
            parse(&text).unwrap().display_text,
            "Build & tests passed <today>"
        );
    }

    #[test]
    fn heartbeat_examples_and_incomplete_results_stay_ordinary_text() {
        for text in [
            "DONT_NOTIFY".to_string(),
            format!("Example:\n{QUIET}"),
            format!("Example:\n```xml\n{QUIET}\n```"),
            format!("{QUIET}\nThis means no notification."),
            QUIET.replace("DONT_NOTIFY", "UNKNOWN"),
            QUIET.replace("</heartbeat>", ""),
            QUIET.replace("</message>", ""),
            format!("> {}", QUIET.replace('\n', "\n> ")),
        ] {
            assert_eq!(parse(&text), None, "{text}");
            assert_eq!(display_text(&text), text);
        }
    }
}
