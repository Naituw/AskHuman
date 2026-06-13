//! 把 `session_id` 解析成「对话标题」，复刻各家恢复对话列表里显示的标题（FINDINGS / spec D10）。
//!
//! - Cursor：`~/.cursor/chats/*/<sid>/meta.json` 的 `title`；缺失回退 transcript 首条用户消息。
//! - Codex：`~/.codex/sessions/**/rollout-*-<sid>.jsonl` 首条**真实**用户消息（跳过注入块）。
//! - Claude：`~/.claude/projects/*/<sid>.jsonl` 最后一条 `summary`，否则首条真实用户消息。
//!
//! 全部 best-effort：文件可能不存在 / 正在写 / 巨大，任何失败都返回 `None`（窗口显示「未命名」）。
//! 读 jsonl 有行数与字节上限，避免拖慢 daemon。

use crate::paths;
use serde_json::Value;
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use super::AgentKind;

/// 标题最大展示长度（字符）。
const MAX_TITLE_CHARS: usize = 80;
/// 扫描 jsonl 的行数上限。
const MAX_LINES: usize = 4000;

/// 解析指定家族某 session 的标题。取不到返回 `None`。
pub fn resolve_title(kind: AgentKind, session_id: &str) -> Option<String> {
    if session_id.is_empty() {
        return None;
    }
    let raw = match kind {
        AgentKind::Cursor => cursor_title(session_id),
        AgentKind::Codex => codex_title(session_id),
        AgentKind::Claude => claude_title(session_id),
    }?;
    Some(clean_title(&raw))
}

fn clean_title(s: &str) -> String {
    let collapsed = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.chars().count() > MAX_TITLE_CHARS {
        let truncated: String = collapsed.chars().take(MAX_TITLE_CHARS).collect();
        format!("{}…", truncated.trim_end())
    } else {
        collapsed
    }
}

// ── Cursor ──

fn cursor_title(session_id: &str) -> Option<String> {
    // 1) ~/.cursor/chats/*/<sid>/meta.json 的 title
    let chats = paths::cursor_dir().join("chats");
    if let Ok(entries) = fs::read_dir(&chats) {
        for e in entries.flatten() {
            let meta = e.path().join(session_id).join("meta.json");
            if let Some(t) = read_json_field(&meta, "title") {
                if !t.trim().is_empty() {
                    return Some(t);
                }
            }
        }
    }
    // 2) 回退：transcript 首条用户消息
    // ~/.cursor/projects/*/agent-transcripts/<sid>/<sid>.jsonl
    let projects = paths::cursor_dir().join("projects");
    if let Ok(entries) = fs::read_dir(&projects) {
        for e in entries.flatten() {
            let f = e
                .path()
                .join("agent-transcripts")
                .join(session_id)
                .join(format!("{session_id}.jsonl"));
            if f.is_file() {
                if let Some(t) = first_user_message(&f) {
                    return Some(t);
                }
            }
        }
    }
    None
}

// ── Codex ──

fn codex_title(session_id: &str) -> Option<String> {
    let sessions = paths::codex_dir().join("sessions");
    let needle = format!("-{session_id}.jsonl");
    let file = find_file_recursive(&sessions, &needle, 4)?;
    // 优先：`event_msg{payload.type=="user_message"}` 的 message——这是用户真正键入的内容，
    // 绕开会话开头作为 role=user 注入的 AGENTS.md 指令块与 `<environment_context>` 等。
    if let Some(t) = codex_user_message(&file) {
        return Some(t);
    }
    // 回退：response_item 首条真实用户消息（已跳过注入块）。
    first_user_message(&file)
}

/// Codex：扫描取首条 `event_msg{payload.type=="user_message"}` 的 `message`。
/// Codex 只为用户真实输入发出该事件（注入的上下文走 response_item），故无需再过滤注入块。
fn codex_user_message(path: &Path) -> Option<String> {
    let file = fs::File::open(path).ok()?;
    let reader = BufReader::new(file);
    for (i, line) in reader.lines().enumerate() {
        if i >= MAX_LINES {
            break;
        }
        let Ok(line) = line else { break };
        if !line.contains("user_message") {
            continue;
        }
        let Ok(v) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        if v.get("type").and_then(|t| t.as_str()) != Some("event_msg") {
            continue;
        }
        let Some(payload) = v.get("payload") else {
            continue;
        };
        if payload.get("type").and_then(|t| t.as_str()) != Some("user_message") {
            continue;
        }
        if let Some(msg) = payload.get("message").and_then(|m| m.as_str()) {
            let t = msg.trim();
            if !t.is_empty() {
                return Some(t.to_string());
            }
        }
    }
    None
}

// ── Claude ──

fn claude_title(session_id: &str) -> Option<String> {
    let projects = paths::claude_dir().join("projects");
    let target = format!("{session_id}.jsonl");
    let file = find_file_recursive(&projects, &target, 3)?;
    // 优先：最后一条 summary。
    if let Some(s) = last_summary(&file) {
        return Some(s);
    }
    first_user_message(&file)
}

// ── 通用 jsonl 解析 ──

/// 读 JSON 文件取顶层字符串字段。
fn read_json_field(path: &Path, field: &str) -> Option<String> {
    let text = fs::read_to_string(path).ok()?;
    let v: Value = serde_json::from_str(&text).ok()?;
    v.get(field).and_then(|x| x.as_str()).map(|s| s.to_string())
}

/// 扫描 jsonl 取「首条真实用户消息」（跳过 `<...>` 注入块与空文本）。
fn first_user_message(path: &Path) -> Option<String> {
    let file = fs::File::open(path).ok()?;
    let reader = BufReader::new(file);
    for (i, line) in reader.lines().enumerate() {
        if i >= MAX_LINES {
            break;
        }
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        if !is_user_line(&v) {
            continue;
        }
        if let Some(text) = extract_text(&v) {
            let t = text.trim();
            if is_injected_block(t) {
                continue; // 跳过注入块（见 is_injected_block）
            }
            return Some(t.to_string());
        }
    }
    None
}

/// 是否为会话开头注入的上下文块（非用户真实输入），用于回退路径过滤。
/// - 以 `<` 开头：`<environment_context>` / `<user_instructions>` / `<turn_aborted>` 等。
/// - Codex 把项目 AGENTS.md 作为 role=user 注入，文本以 `# AGENTS.md instructions` 开头。
fn is_injected_block(t: &str) -> bool {
    t.is_empty() || t.starts_with('<') || t.starts_with("# AGENTS.md instructions")
}

/// Claude：扫描取最后一条 summary。
fn last_summary(path: &Path) -> Option<String> {
    let file = fs::File::open(path).ok()?;
    let reader = BufReader::new(file);
    let mut last: Option<String> = None;
    for (i, line) in reader.lines().enumerate() {
        if i >= MAX_LINES {
            break;
        }
        let Ok(line) = line else { break };
        if !line.contains("summary") {
            continue;
        }
        let Ok(v) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        if v.get("type").and_then(|t| t.as_str()) == Some("summary") {
            if let Some(s) = v.get("summary").and_then(|s| s.as_str()) {
                if !s.trim().is_empty() {
                    last = Some(s.to_string());
                }
            }
        }
    }
    last
}

/// 判断一行 jsonl 是否「用户消息」。兼容三家不同结构。
fn is_user_line(v: &Value) -> bool {
    // claude: {type:"user", isMeta?:bool, message:{role:"user",...}}
    if v.get("type").and_then(|t| t.as_str()) == Some("user") {
        if v.get("isMeta").and_then(|b| b.as_bool()) == Some(true) {
            return false;
        }
        return true;
    }
    // cursor: {role:"user", ...} 或 {type:"user"}
    if v.get("role").and_then(|r| r.as_str()) == Some("user") {
        return true;
    }
    // codex rollout: {payload:{role:"user"|type:"message"...}} 或 {type:"response_item"...}
    if let Some(p) = v.get("payload") {
        if p.get("role").and_then(|r| r.as_str()) == Some("user") {
            return true;
        }
    }
    false
}

/// 从一行 jsonl 提取用户文本（兼容 string / {content} / [{text}] 等多种结构）。
fn extract_text(v: &Value) -> Option<String> {
    // 优先 message.content；其次 payload.content；其次顶层 content / text。
    let candidates = [
        v.get("message").and_then(|m| m.get("content")),
        v.get("payload").and_then(|p| p.get("content")),
        v.get("content"),
        v.get("text"),
        v.get("message").and_then(|m| m.get("text")),
    ];
    for c in candidates.into_iter().flatten() {
        if let Some(t) = content_to_text(c) {
            if !t.trim().is_empty() {
                return Some(t);
            }
        }
    }
    None
}

/// content 可能是字符串，或数组 `[{type:"text"|"input_text", text:"..."}]`。
fn content_to_text(c: &Value) -> Option<String> {
    match c {
        Value::String(s) => Some(s.clone()),
        Value::Array(arr) => {
            let mut parts = Vec::new();
            for item in arr {
                if let Some(s) = item.as_str() {
                    parts.push(s.to_string());
                } else if let Some(s) = item.get("text").and_then(|t| t.as_str()) {
                    parts.push(s.to_string());
                }
            }
            if parts.is_empty() {
                None
            } else {
                Some(parts.join(" "))
            }
        }
        _ => None,
    }
}

/// 在目录下递归（限深度）查找文件名以 `suffix` 结尾的第一个文件。
fn find_file_recursive(root: &Path, suffix: &str, max_depth: usize) -> Option<PathBuf> {
    fn walk(dir: &Path, suffix: &str, depth: usize, max_depth: usize) -> Option<PathBuf> {
        let entries = fs::read_dir(dir).ok()?;
        let mut subdirs = Vec::new();
        for e in entries.flatten() {
            let p = e.path();
            let is_dir = e.file_type().map(|t| t.is_dir()).unwrap_or(false);
            if is_dir {
                subdirs.push(p);
            } else if p
                .file_name()
                .and_then(|n| n.to_str())
                .map(|n| n.ends_with(suffix))
                .unwrap_or(false)
            {
                return Some(p);
            }
        }
        if depth < max_depth {
            for d in subdirs {
                if let Some(found) = walk(&d, suffix, depth + 1, max_depth) {
                    return Some(found);
                }
            }
        }
        None
    }
    walk(root, suffix, 0, max_depth)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_truncates_and_collapses() {
        assert_eq!(clean_title("  a\n b   c "), "a b c");
        let long = "x".repeat(200);
        let out = clean_title(&long);
        assert!(out.chars().count() <= MAX_TITLE_CHARS + 1);
        assert!(out.ends_with('…'));
    }

    #[test]
    fn extract_text_handles_array_and_string() {
        let v: Value = serde_json::json!({"message":{"content":"hello"}});
        assert_eq!(extract_text(&v).as_deref(), Some("hello"));
        let v: Value = serde_json::json!({"payload":{"content":[{"type":"input_text","text":"hi there"}]}});
        assert_eq!(extract_text(&v).as_deref(), Some("hi there"));
    }

    #[test]
    fn first_user_message_skips_injected_blocks() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("s.jsonl");
        let lines = [
            r#"{"type":"user","message":{"content":"<environment_context>ctx</environment_context>"}}"#,
            r#"{"type":"user","isMeta":true,"message":{"content":"meta only"}}"#,
            r#"{"type":"user","message":{"content":"实际的第一句话"}}"#,
        ];
        std::fs::write(&f, lines.join("\n")).unwrap();
        assert_eq!(first_user_message(&f).as_deref(), Some("实际的第一句话"));
    }

    #[test]
    fn codex_title_skips_agents_md_injection() {
        // 复刻 Codex rollout 开头结构：先注入 AGENTS.md(role=user) + environment_context，
        // 再是用户真实问题（既有 response_item 也有 event_msg/user_message）。
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("rollout-2026-06-13T21-00-16-sid.jsonl");
        let lines = [
            r#"{"type":"session_meta","payload":{"id":"sid"}}"#,
            r#"{"type":"response_item","payload":{"type":"message","role":"developer","content":[{"type":"input_text","text":"<permissions instructions>...</permissions instructions>"}]}}"#,
            r##"{"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"# AGENTS.md instructions for /x\n\n<INSTRUCTIONS>\n...\n</INSTRUCTIONS>"},{"type":"input_text","text":"<environment_context>\n  <cwd>/x</cwd>\n</environment_context>"}]}}"##,
            r#"{"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"帮我修一个 bug"}]}}"#,
            r#"{"type":"event_msg","payload":{"type":"user_message","message":"帮我修一个 bug","images":[]}}"#,
        ];
        std::fs::write(&f, lines.join("\n")).unwrap();
        // 主路径：event_msg/user_message。
        assert_eq!(codex_user_message(&f).as_deref(), Some("帮我修一个 bug"));
        // 回退路径：response_item 也要跳过 AGENTS.md 注入块、取到真实问题。
        assert_eq!(first_user_message(&f).as_deref(), Some("帮我修一个 bug"));
    }

    #[test]
    fn last_summary_picks_last() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("s.jsonl");
        let lines = [
            r#"{"type":"summary","summary":"first"}"#,
            r#"{"type":"user","message":{"content":"hi"}}"#,
            r#"{"type":"summary","summary":"second"}"#,
        ];
        std::fs::write(&f, lines.join("\n")).unwrap();
        assert_eq!(last_summary(&f).as_deref(), Some("second"));
    }
}
