//! DingTalk/Slack docx for transcripts.

use crate::agents::transcript_full::{TranscriptDoc, TranscriptEvent};
use crate::dingtalk::docx;
use crate::watch;

pub fn render(doc: &TranscriptDoc, meta_title: &str) -> Result<Vec<u8>, String> {
    let mut md = String::new();
    md.push_str("# ");
    md.push_str(meta_title);
    md.push_str("\n\n");
    if doc.truncated_head || doc.partial {
        md.push_str("_Note: ");
        if doc.truncated_head {
            md.push_str("earlier events truncated. ");
        }
        if doc.partial {
            md.push_str("some events could not be parsed. ");
        }
        md.push_str("_\n\n");
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let mut first = true;
    for ev in &doc.events {
        match ev {
            TranscriptEvent::UserText { text, at, at_label }
            | TranscriptEvent::AssistantText { text, at, at_label } => {
                if !first {
                    md.push_str("\n---\n\n");
                }
                first = false;
                let role = match ev {
                    TranscriptEvent::UserText { .. } => "User",
                    _ => "Assistant",
                };
                let when = if let Some(l) = at_label.as_ref().filter(|s| !s.is_empty()) {
                    format!(" · {l}")
                } else {
                    at.map(|ts| format!(" · {}", watch::fmt_local_time(ts, now)))
                        .unwrap_or_default()
                };
                md.push_str(&format!("### {role}{when}\n\n"));
                md.push_str(text);
                md.push_str("\n\n");
            }
            TranscriptEvent::Thinking { text, .. } => {
                let one: String = text.chars().take(120).collect();
                md.push_str(&format!("_thinking: {one}…_\n\n"));
            }
            TranscriptEvent::ToolCall {
                args_summary,
                is_error,
                ..
            } => {
                let mark = if *is_error { "✕" } else { "●" };
                let (label, obj) = crate::agents::transcript_full::split_tool_line(args_summary);
                match obj {
                    Some(o) => md.push_str(&format!("{mark} **{label}**: *{o}*\n\n")),
                    None => md.push_str(&format!("{mark} **{label}**\n\n")),
                }
            }
            TranscriptEvent::Meta(t) => {
                md.push_str("_");
                md.push_str(t);
                md.push_str("_\n\n");
            }
        }
    }
    docx::build_markdown_docx(&md).map_err(|e| e.to_string())
}
