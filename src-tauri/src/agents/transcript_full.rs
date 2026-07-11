//! Full-session transcript parse for IM `/transcript` (best-effort, four agent families).
//!
//! Separate from `activity.rs` (tail-only “what now”). Spec: im-diff-stage-transcript D17–D21.

use super::title::transcript_path;
use super::AgentKind;
use serde_json::Value;
use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

/// Max bytes read from a transcript file (prefer tail when larger).
pub const MAX_READ_BYTES: u64 = 2 * 1024 * 1024;
/// Max normalized events retained (drop oldest).
pub const MAX_EVENTS: usize = 2000;
const MAX_TEXT_CHARS: usize = 8_000;
/// Tool results: short summary only — full dumps bloat IM export.
const MAX_TOOL_RESULT_CHARS: usize = 400;
const MAX_ARG_CHARS: usize = 200;

#[derive(Debug, Clone, PartialEq)]
pub struct TranscriptDoc {
    pub events: Vec<TranscriptEvent>,
    pub truncated_head: bool,
    pub partial: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TranscriptEvent {
    UserText {
        text: String,
        /// Unix seconds when known (Claude/Codex ISO timestamps).
        at: Option<u64>,
        /// Preformatted local label when source only has a human string
        /// (Cursor embeds `<timestamp>…</timestamp>` in user text).
        at_label: Option<String>,
    },
    AssistantText {
        text: String,
        at: Option<u64>,
        at_label: Option<String>,
    },
    Thinking {
        text: String,
        at: Option<u64>,
        at_label: Option<String>,
    },
    ToolCall {
        name: String,
        args_summary: String,
        result_summary: Option<String>,
        is_error: bool,
        ask_human: Option<AskHumanBlock>,
        at: Option<u64>,
        at_label: Option<String>,
    },
    Meta(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct AskHumanBlock {
    pub question: String,
    pub answer: Option<String>,
}

/// Best-effort event time (unix seconds) from common transcript fields.
/// Real formats seen: Claude/Codex `"2026-06-13T10:09:57.062Z"` (RFC3339 string);
/// numeric epoch is rare.
fn event_time(v: &Value) -> Option<u64> {
    for key in [
        "timestamp",
        "ts",
        "created_at",
        "createdAt",
        "time",
        "event_time",
    ] {
        if let Some(n) = v.get(key).and_then(|x| x.as_u64()) {
            return Some(if n > 10_000_000_000 { n / 1000 } else { n });
        }
        if let Some(f) = v.get(key).and_then(|x| x.as_f64()) {
            let n = f as u64;
            return Some(if n > 10_000_000_000 { n / 1000 } else { n });
        }
        if let Some(s) = v.get(key).and_then(|x| x.as_str()) {
            if let Some(secs) = parse_iso8601_secs(s) {
                return Some(secs);
            }
            if let Ok(n) = s.parse::<u64>() {
                return Some(if n > 10_000_000_000 { n / 1000 } else { n });
            }
        }
    }
    // Claude snapshot.timestamp
    if let Some(s) = v
        .get("snapshot")
        .and_then(|p| p.get("timestamp"))
        .and_then(|x| x.as_str())
    {
        if let Some(secs) = parse_iso8601_secs(s) {
            return Some(secs);
        }
    }
    if let Some(p) = v.get("payload") {
        if let Some(t) = event_time(p) {
            return Some(t);
        }
    }
    if let Some(m) = v.get("message") {
        if let Some(t) = event_time(m) {
            return Some(t);
        }
    }
    None
}

/// Parse `2026-06-13T10:09:57.062Z` / `2026-06-13T10:09:57+08:00` → unix seconds (UTC).
fn parse_iso8601_secs(s: &str) -> Option<u64> {
    let s = s.trim();
    if s.len() < 19 {
        return None;
    }
    // date
    let y: i64 = s.get(0..4)?.parse().ok()?;
    let mo: i64 = s.get(5..7)?.parse().ok()?;
    let d: i64 = s.get(8..10)?.parse().ok()?;
    if s.as_bytes().get(10).copied()? != b'T' {
        return None;
    }
    let h: i64 = s.get(11..13)?.parse().ok()?;
    let mi: i64 = s.get(14..16)?.parse().ok()?;
    let sec: i64 = s.get(17..19)?.parse().ok()?;
    // optional fractional seconds then Z or ±HH:MM
    let rest = s.get(19..).unwrap_or("");
    let rest = rest.trim_start_matches(|c: char| c == '.' || c.is_ascii_digit());
    let mut offset_secs: i64 = 0;
    if rest.starts_with('Z') || rest.is_empty() {
        offset_secs = 0;
    } else if let Some(sign) = rest.chars().next().filter(|c| *c == '+' || *c == '-') {
        let body = &rest[1..];
        let oh: i64 = body.get(0..2)?.parse().ok()?;
        let om: i64 = if body.len() >= 5 {
            body.get(3..5)?.parse().ok()?
        } else {
            0
        };
        let off = oh * 3600 + om * 60;
        offset_secs = if sign == '+' { off } else { -off };
    }
    let days = days_from_civil(y, mo, d)?;
    let utc = days * 86400 + h * 3600 + mi * 60 + sec - offset_secs;
    if utc < 0 {
        None
    } else {
        Some(utc as u64)
    }
}

/// Howard Hinnant civil-from-days inverse (proleptic Gregorian) → days since 1970-01-01.
fn days_from_civil(y: i64, m: i64, d: i64) -> Option<i64> {
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let doy = (153 * (m + if m > 2 { -3 } else { 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some(era * 146097 + doe - 719468)
}

pub fn load_events(kind: AgentKind, session_id: &str) -> Result<TranscriptDoc, String> {
    let path =
        transcript_path(kind, session_id).ok_or_else(|| "transcript not found".to_string())?;
    let mut doc = load_path(kind, &path)?;
    // Grok chat_history has no per-line times; backfill from sibling updates.jsonl when present.
    if kind == AgentKind::Grok {
        grok_backfill_times(path.parent(), &mut doc.events);
    }
    Ok(doc)
}

pub fn load_path(kind: AgentKind, path: &Path) -> Result<TranscriptDoc, String> {
    let (lines, truncated_head) = read_lines_bounded(path, MAX_READ_BYTES)?;
    let mut events = Vec::new();
    let mut partial = false;
    // Open tool calls waiting for result (order pairing fallback).
    let mut open_tools: Vec<usize> = Vec::new();

    for line in lines {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            partial = true;
            continue;
        };
        let before = events.len();
        push_full(kind, &v, &mut events, &mut open_tools);
        if events.len() == before {
            // unrecognized line — soft partial if it looked like json with type
            if v.get("type").is_some() || v.get("role").is_some() {
                // ignore known noise silently
            }
        }
    }

    if events.len() > MAX_EVENTS {
        let skip = events.len() - MAX_EVENTS;
        events.drain(0..skip);
    }
    // Grok: chat_history has no timestamps — try sibling updates.jsonl.
    if kind == AgentKind::Grok {
        grok_backfill_times(path.parent(), &mut events);
    }
    Ok(TranscriptDoc {
        events,
        truncated_head,
        partial,
    })
}

/// Grok: assign times from `updates.jsonl` (`params._meta.agentTimestampMs` / `timestamp`)
/// onto User/Assistant/Tool events in order of appearance.
fn grok_backfill_times(session_dir: Option<&Path>, events: &mut [TranscriptEvent]) {
    let Some(dir) = session_dir else {
        return;
    };
    let path = dir.join("updates.jsonl");
    let Ok(text) = fs::read_to_string(&path) else {
        return;
    };
    let mut user_ts: Vec<u64> = Vec::new();
    let mut asst_ts: Vec<u64> = Vec::new();
    let mut tool_ts: Vec<u64> = Vec::new();
    // agent_message_chunk fires many times per turn — take first ms of each contiguous run.
    let mut last_kind = String::new();
    for line in text.lines() {
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let su = v
            .pointer("/params/update/sessionUpdate")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string();
        let ms = v
            .pointer("/params/_meta/agentTimestampMs")
            .and_then(|x| x.as_u64())
            .or_else(|| {
                v.get("timestamp").and_then(|x| {
                    x.as_u64()
                        .map(|n| if n < 10_000_000_000 { n * 1000 } else { n })
                })
            });
        let Some(ms) = ms else {
            continue;
        };
        let secs = ms / 1000;
        match su.as_str() {
            "user_message_chunk" if last_kind != su => user_ts.push(secs),
            "agent_message_chunk" if last_kind != su => asst_ts.push(secs),
            "tool_call" if last_kind != su => tool_ts.push(secs),
            _ => {}
        }
        if matches!(
            su.as_str(),
            "user_message_chunk" | "agent_message_chunk" | "tool_call"
        ) {
            last_kind = su;
        } else if !su.is_empty() {
            last_kind.clear();
        }
    }
    let mut ui = 0usize;
    let mut ai = 0usize;
    let mut ti = 0usize;
    for ev in events.iter_mut() {
        match ev {
            TranscriptEvent::UserText { at, .. } if at.is_none() => {
                if ui < user_ts.len() {
                    *at = Some(user_ts[ui]);
                    ui += 1;
                }
            }
            TranscriptEvent::AssistantText { at, .. } if at.is_none() => {
                if ai < asst_ts.len() {
                    *at = Some(asst_ts[ai]);
                    ai += 1;
                }
            }
            TranscriptEvent::ToolCall { at, .. } if at.is_none() => {
                if ti < tool_ts.len() {
                    *at = Some(tool_ts[ti]);
                    ti += 1;
                }
            }
            _ => {}
        }
    }
}

fn read_lines_bounded(path: &Path, max_bytes: u64) -> Result<(Vec<String>, bool), String> {
    let mut f = fs::File::open(path).map_err(|e| e.to_string())?;
    let len = f.metadata().map_err(|e| e.to_string())?.len();
    let truncated_head = len > max_bytes;
    let start = len.saturating_sub(max_bytes);
    f.seek(SeekFrom::Start(start)).map_err(|e| e.to_string())?;
    let mut buf = Vec::new();
    f.read_to_end(&mut buf).map_err(|e| e.to_string())?;
    let text = String::from_utf8_lossy(&buf);
    let mut lines: Vec<String> = text.lines().map(|s| s.to_string()).collect();
    if truncated_head && !lines.is_empty() {
        lines.remove(0);
    }
    Ok((lines, truncated_head))
}

fn push_full(
    kind: AgentKind,
    v: &Value,
    out: &mut Vec<TranscriptEvent>,
    open_tools: &mut Vec<usize>,
) {
    match kind {
        AgentKind::Cursor | AgentKind::Claude => push_msg(v, out, open_tools),
        AgentKind::Codex => push_codex(v, out, open_tools),
        AgentKind::Grok => push_grok(v, out, open_tools),
    }
}

fn push_msg(v: &Value, out: &mut Vec<TranscriptEvent>, open_tools: &mut Vec<usize>) {
    let role = v
        .get("role")
        .and_then(|r| r.as_str())
        .or_else(|| v.get("type").and_then(|t| t.as_str()))
        .unwrap_or("");
    // Export focus: agent behaviour + user turns. Skip system / meta roles.
    if role == "system" || role == "system_prompt" {
        return;
    }
    let content = v
        .get("message")
        .and_then(|m| m.get("content"))
        .or_else(|| v.get("content"));
    let Some(arr) = content.and_then(|c| c.as_array()) else {
        // plain string content
        if role == "user" || role == "human" {
            if let Some(t) = content.and_then(|c| c.as_str()) {
                let (t, label) = clean_user(t);
                if !t.is_empty() {
                    out.push(TranscriptEvent::UserText {
                        text: trunc(&t, MAX_TEXT_CHARS),
                        at: event_time(v),
                        at_label: label,
                    });
                }
            }
        }
        return;
    };
    let is_assistant = role == "assistant";
    let is_user = role == "user" || role == "human";
    for item in arr {
        let t = item.get("type").and_then(|x| x.as_str()).unwrap_or("");
        match t {
            "text" | "input_text" | "output_text" => {
                if let Some(text) = item.get("text").and_then(|x| x.as_str()) {
                    let text = text.trim();
                    if text.is_empty() {
                        continue;
                    }
                    if is_assistant {
                        if is_noise_assistant(text) {
                            continue;
                        }
                        out.push(TranscriptEvent::AssistantText {
                            text: trunc(text, MAX_TEXT_CHARS),
                            at: event_time(v),
                            at_label: None,
                        });
                    } else if is_user {
                        let (t, label) = clean_user(text);
                        if !t.is_empty() {
                            out.push(TranscriptEvent::UserText {
                                text: trunc(&t, MAX_TEXT_CHARS),
                                at: event_time(v),
                                at_label: label,
                            });
                        }
                    }
                }
            }
            // Thinking kept but heavily truncated — user cares more about output + tools.
            "thinking" | "reasoning" => {
                if let Some(text) = item
                    .get("thinking")
                    .or_else(|| item.get("text"))
                    .and_then(|x| x.as_str())
                {
                    let text = text.trim();
                    if !text.is_empty() {
                        out.push(TranscriptEvent::Thinking {
                            text: trunc(text, 800),
                            at: event_time(v),
                            at_label: None,
                        });
                    }
                }
            }
            "tool_use" => {
                let name = item.get("name").and_then(|x| x.as_str()).unwrap_or("tool");
                if super::activity::is_todo_tool(name) {
                    // TodoWrite / update_plan：不入行为时间线（同 watch）。
                    continue;
                }
                let args = item.get("input");
                // 与 watch 卡同源：归一化「读取/写入/运行 + 对象」；不单独解析 AskHuman。
                let td = super::activity::classify_tool(name, args);
                out.push(TranscriptEvent::ToolCall {
                    name: name.to_string(),
                    args_summary: format_tool_line(&td),
                    result_summary: None,
                    is_error: false,
                    ask_human: None,
                    at: event_time(v),
                    at_label: None,
                });
                open_tools.push(out.len() - 1);
            }
            "tool_result" => {
                let err = item
                    .get("is_error")
                    .and_then(|x| x.as_bool())
                    .unwrap_or(false);
                let content = tool_result_text(item);
                close_tool(out, open_tools, content, err);
            }
            _ => {}
        }
    }
}

fn push_codex(v: &Value, out: &mut Vec<TranscriptEvent>, open_tools: &mut Vec<usize>) {
    let ttype = v.get("type").and_then(|t| t.as_str()).unwrap_or("");
    let Some(payload) = v.get("payload") else {
        return;
    };
    let ptype = payload.get("type").and_then(|t| t.as_str()).unwrap_or("");
    match (ttype, ptype) {
        ("response_item", "message") => {
            let role = payload.get("role").and_then(|r| r.as_str()).unwrap_or("");
            if let Some(t) = value_text(payload.get("content")) {
                let t = t.trim();
                if t.is_empty() {
                    return;
                }
                if role == "user" {
                    let (t, label) = clean_user(t);
                    if !t.is_empty() {
                        out.push(TranscriptEvent::UserText {
                            text: trunc(&t, MAX_TEXT_CHARS),
                            at: event_time(v),
                            at_label: label,
                        });
                    }
                } else if role == "assistant" {
                    out.push(TranscriptEvent::AssistantText {
                        text: trunc(t, MAX_TEXT_CHARS),
                        at: event_time(v),
                        at_label: None,
                    });
                }
            }
        }
        ("response_item", "reasoning") => {
            if let Some(t) = value_text(payload.get("summary"))
                .or_else(|| value_text(payload.get("content")))
                .or_else(|| {
                    payload
                        .get("text")
                        .and_then(|x| x.as_str())
                        .map(|s| s.to_string())
                })
            {
                let t = t.trim();
                if !t.is_empty() {
                    out.push(TranscriptEvent::Thinking {
                        text: trunc(t, MAX_TEXT_CHARS),
                        at: event_time(v),
                        at_label: None,
                    });
                }
            }
        }
        ("response_item", "function_call") => {
            let name = payload
                .get("name")
                .and_then(|x| x.as_str())
                .unwrap_or("tool");
            if super::activity::is_todo_tool(name) {
                return;
            }
            let args_val = parse_args_value(payload.get("arguments"));
            let td = super::activity::classify_tool(name, args_val.as_ref());
            out.push(TranscriptEvent::ToolCall {
                name: name.to_string(),
                args_summary: format_tool_line(&td),
                result_summary: None,
                is_error: false,
                ask_human: None,
                at: event_time(v),
                at_label: None,
            });
            open_tools.push(out.len() - 1);
        }
        ("response_item", "function_call_output") => {
            let content = payload
                .get("output")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .to_string();
            close_tool(out, open_tools, content, false);
        }
        ("event_msg", "agent_message") => {
            if let Some(t) = payload.get("message").and_then(|m| m.as_str()) {
                let t = t.trim();
                if !t.is_empty() {
                    out.push(TranscriptEvent::AssistantText {
                        text: trunc(t, MAX_TEXT_CHARS),
                        at: event_time(v),
                        at_label: None,
                    });
                }
            }
        }
        ("event_msg", "user_message") => {
            if let Some(t) = payload.get("message").and_then(|m| m.as_str()) {
                let (t, label) = clean_user(t.trim());
                if !t.is_empty() {
                    out.push(TranscriptEvent::UserText {
                        text: trunc(&t, MAX_TEXT_CHARS),
                        at: event_time(v),
                        at_label: label,
                    });
                }
            }
        }
        _ => {}
    }
}

fn push_grok(v: &Value, out: &mut Vec<TranscriptEvent>, open_tools: &mut Vec<usize>) {
    match v.get("type").and_then(|t| t.as_str()).unwrap_or("") {
        "user" => {
            if let Some(t) = value_text(v.get("content")) {
                let (t, label) = clean_user(t.trim());
                if !t.is_empty() {
                    out.push(TranscriptEvent::UserText {
                        text: trunc(&t, MAX_TEXT_CHARS),
                        at: event_time(v),
                        at_label: label,
                    });
                }
            }
        }
        "assistant" => {
            if let Some(t) = value_text(v.get("content")) {
                let t = t.trim();
                if !t.is_empty() {
                    out.push(TranscriptEvent::AssistantText {
                        text: trunc(t, MAX_TEXT_CHARS),
                        at: event_time(v),
                        at_label: None,
                    });
                }
            }
            if let Some(r) = v.get("reasoning").and_then(|x| x.as_str()) {
                let r = r.trim();
                if !r.is_empty() {
                    out.push(TranscriptEvent::Thinking {
                        text: trunc(r, MAX_TEXT_CHARS),
                        at: event_time(v),
                        at_label: None,
                    });
                }
            }
            if let Some(arr) = v.get("tool_calls").and_then(|x| x.as_array()) {
                for tc in arr {
                    let func = tc.get("function");
                    let name = func
                        .and_then(|f| f.get("name"))
                        .or_else(|| tc.get("name"))
                        .and_then(|x| x.as_str())
                        .unwrap_or("tool");
                    if super::activity::is_todo_tool(name) {
                        continue;
                    }
                    let args_val = parse_args_value(
                        func.and_then(|f| f.get("arguments"))
                            .or_else(|| tc.get("arguments")),
                    );
                    let td = super::activity::classify_tool(name, args_val.as_ref());
                    out.push(TranscriptEvent::ToolCall {
                        name: name.to_string(),
                        args_summary: format_tool_line(&td),
                        result_summary: None,
                        is_error: false,
                        ask_human: None,
                        at: event_time(v),
                        at_label: None,
                    });
                    open_tools.push(out.len() - 1);
                }
            }
        }
        "tool_result" => {
            let err = v.get("is_error").and_then(|x| x.as_bool()).unwrap_or(false);
            let content = v
                .get("content")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .to_string();
            close_tool(out, open_tools, content, err);
        }
        "thinking" | "reasoning" => {
            if let Some(t) = value_text(v.get("content")).or_else(|| {
                v.get("text")
                    .and_then(|x| x.as_str())
                    .map(|s| s.to_string())
            }) {
                let t = t.trim();
                if !t.is_empty() {
                    out.push(TranscriptEvent::Thinking {
                        text: trunc(t, MAX_TEXT_CHARS),
                        at: event_time(v),
                        at_label: None,
                    });
                }
            }
        }
        _ => {}
    }
}

fn close_tool(
    out: &mut [TranscriptEvent],
    open_tools: &mut Vec<usize>,
    content: String,
    is_error: bool,
) {
    let _ = content;
    if let Some(idx) = open_tools.pop() {
        if let Some(TranscriptEvent::ToolCall { is_error: ie, .. }) = out.get_mut(idx) {
            // 与 watch 一致：不展示 tool result；仅标记失败。
            *ie = is_error;
        }
    }
}

/// Watch 同款：类别词 + 对象。`args_summary` 存 **纯文本** `读取: file.rs`；
/// 渲染层负责 **粗体类别** / *斜体对象*（与 watch 卡 `**类别**: *对象*` 一致）。
fn format_tool_line(td: &super::activity::ToolDisplay) -> String {
    use super::activity::ToolLabel;
    let label = match &td.label {
        ToolLabel::Run => "运行",
        ToolLabel::Read => "读取",
        ToolLabel::Write => "写入",
        ToolLabel::Other(n) => n.as_str(),
    };
    match &td.object {
        Some(o) => format!("{label}: {o}"),
        None => label.to_string(),
    }
}

/// Split `读取: file.rs` → (`读取`, `Some(file.rs)`).
pub fn split_tool_line(s: &str) -> (&str, Option<&str>) {
    if let Some((a, b)) = s.split_once(": ") {
        (a, Some(b))
    } else if let Some((a, b)) = s.split_once(':') {
        (a.trim(), Some(b.trim()))
    } else {
        (s, None)
    }
}

fn tool_result_text(item: &Value) -> String {
    if let Some(s) = item.get("content").and_then(|c| c.as_str()) {
        return s.to_string();
    }
    if let Some(arr) = item.get("content").and_then(|c| c.as_array()) {
        let mut parts = Vec::new();
        for it in arr {
            if let Some(t) = it.get("text").and_then(|x| x.as_str()) {
                parts.push(t.to_string());
            }
        }
        return parts.join("\n");
    }
    String::new()
}

fn summarize_args(name: &str, args: Option<&Value>) -> String {
    let Some(a) = args else {
        return String::new();
    };
    // Prefer common fields.
    for key in [
        "command",
        "file_path",
        "path",
        "pattern",
        "query",
        "message",
        "description",
    ] {
        if let Some(s) = a.get(key).and_then(|v| v.as_str()) {
            return format!("{}: {}", key, trunc(s, MAX_ARG_CHARS));
        }
        if let Some(arr) = a.get(key).and_then(|v| v.as_array()) {
            if key == "command" {
                let joined: Vec<&str> = arr.iter().filter_map(|x| x.as_str()).collect();
                if !joined.is_empty() {
                    return format!("command: {}", trunc(&joined.join(" "), MAX_ARG_CHARS));
                }
            }
        }
    }
    if name.eq_ignore_ascii_case("ask") {
        if let Some(s) = a.get("message").and_then(|v| v.as_str()) {
            return trunc(s, MAX_ARG_CHARS);
        }
    }
    let s = a.to_string();
    trunc(&s, MAX_ARG_CHARS)
}

fn detect_askhuman(
    name: &str,
    args: Option<&Value>,
    result: Option<&str>,
) -> Option<AskHumanBlock> {
    let name_l = name.to_ascii_lowercase();
    let mut question = String::new();
    let mut is_ah = name_l == "ask" || name_l.contains("askhuman");

    if let Some(a) = args {
        if let Some(cmd) = a.get("command").and_then(|v| v.as_str()) {
            if cmd.to_ascii_lowercase().contains("askhuman") {
                is_ah = true;
                question = extract_askhuman_cli_question(cmd);
            }
        }
        if let Some(arr) = a.get("command").and_then(|v| v.as_array()) {
            let joined: Vec<&str> = arr.iter().filter_map(|x| x.as_str()).collect();
            let s = joined.join(" ");
            if s.to_ascii_lowercase().contains("askhuman") {
                is_ah = true;
                question = extract_askhuman_cli_question(&s);
            }
        }
        if name_l == "ask" {
            is_ah = true;
            if let Some(m) = a.get("message").and_then(|v| v.as_str()) {
                question = m.to_string();
            }
            if let Some(qs) = a.get("questions").and_then(|v| v.as_array()) {
                for q in qs {
                    if let Some(qt) = q.get("question").and_then(|x| x.as_str()) {
                        if !question.is_empty() {
                            question.push('\n');
                        }
                        question.push_str(qt);
                    }
                }
            }
        }
    }
    if !is_ah {
        return None;
    }
    if question.is_empty() {
        question = summarize_args(name, args);
    }
    let answer = result.and_then(parse_askhuman_answer);
    Some(AskHumanBlock {
        question: trunc(&question, MAX_TEXT_CHARS),
        answer,
    })
}

fn extract_askhuman_cli_question(cmd: &str) -> String {
    // Best-effort: last quoted string or text after -m / message.
    if let Some(i) = cmd.find(" -m ") {
        return cmd[i + 4..]
            .trim()
            .trim_matches('"')
            .trim_matches('\'')
            .to_string();
    }
    if let Some(i) = cmd.find(" --message ") {
        return cmd[i + 11..]
            .trim()
            .trim_matches('"')
            .trim_matches('\'')
            .to_string();
    }
    // positional after AskHuman
    if let Some(i) = cmd.to_ascii_lowercase().find("askhuman") {
        let rest = cmd[i..]
            .split_whitespace()
            .skip(1)
            .collect::<Vec<_>>()
            .join(" ");
        return rest.trim_matches('"').to_string();
    }
    cmd.to_string()
}

fn parse_askhuman_answer(content: &str) -> Option<String> {
    // JSON result
    if let Ok(v) = serde_json::from_str::<Value>(content) {
        let mut parts = Vec::new();
        if let Some(s) = v.get("user_input").and_then(|x| x.as_str()) {
            if !s.is_empty() {
                parts.push(s.to_string());
            }
        }
        if let Some(arr) = v.get("selected_options").and_then(|x| x.as_array()) {
            for o in arr {
                if let Some(s) = o.as_str() {
                    parts.push(s.to_string());
                }
            }
        }
        if !parts.is_empty() {
            return Some(trunc(&parts.join("\n"), MAX_TOOL_RESULT_CHARS));
        }
    }
    // Text markers
    if let Some(i) = content.find("[user_input]") {
        let rest = &content[i + "[user_input]".len()..];
        let end = rest.find('[').unwrap_or(rest.len());
        let s = rest[..end].trim();
        if !s.is_empty() {
            return Some(trunc(s, MAX_TOOL_RESULT_CHARS));
        }
    }
    if let Some(i) = content.find("[selected_options]") {
        let rest = &content[i + "[selected_options]".len()..];
        let end = rest.find('[').unwrap_or(rest.len());
        let s = rest[..end].trim();
        if !s.is_empty() {
            return Some(trunc(s, MAX_TOOL_RESULT_CHARS));
        }
    }
    None
}

/// Clean user text; also return Cursor `<timestamp>` label if present.
fn clean_user(text: &str) -> (String, Option<String>) {
    let t = text.trim();
    if t.is_empty() {
        return (String::new(), None);
    }
    let (t, cursor_label) = extract_cursor_timestamp(t);
    let t = t.trim();
    // Prefer real user payload when wrapped.
    if let Some(inner) = extract_tag(t, "user_query") {
        let inner = inner.trim();
        if !inner.is_empty() {
            return (strip_system_reminders(inner), cursor_label);
        }
    }
    // Skip pure injection / system-instruction blobs (not agent behaviour).
    if is_injected_or_system_blob(t) {
        return (String::new(), cursor_label);
    }
    (strip_system_reminders(t), cursor_label)
}

fn is_injected_or_system_blob(t: &str) -> bool {
    let head = t.chars().take(200).collect::<String>().to_ascii_lowercase();
    if t.starts_with('<') {
        if t.starts_with("<environment_context>")
            || t.starts_with("<user_info>")
            || t.starts_with("<git_status>")
            || t.starts_with("<INSTRUCTIONS>")
            || t.starts_with("<system>")
            || t.starts_with("<system-reminder>")
            || t.starts_with("<agent_skills>")
            || t.starts_with("<available_skills>")
            || t.starts_with("<mcp_")
            || t.starts_with("<functions>")
        {
            return true;
        }
    }
    // Common rule-file dumps / CLI system prompts.
    if head.contains("# agents.md")
        || head.contains("# claude.md")
        || head.contains("you are a coding assistant")
        || head.contains("you are claude")
        || head.contains("mandatory interaction protocol")
        || head.contains("askhuman managed skill")
        || head.starts_with("system:")
        || head.starts_with("# system")
    {
        return true;
    }
    // Huge rule dumps without a real user ask.
    if t.len() > 4000
        && (head.contains("follow these instructions")
            || head.contains("project instructions")
            || head.contains("always apply these"))
        && !t.contains("<user_query>")
    {
        return true;
    }
    false
}

fn is_noise_assistant(t: &str) -> bool {
    let head = t.chars().take(120).collect::<String>().to_ascii_lowercase();
    head.starts_with("i'll follow") && head.contains("instruction")
        || head == "ok"
        || head == "understood."
}

/// Strip trailing/leading system-reminder blocks while keeping the real ask.

/// Cursor embeds wall-clock labels inside user text:
/// `<timestamp>Monday, May 25, 2026, 7:57 AM (UTC+8)</timestamp>`
fn extract_cursor_timestamp(text: &str) -> (String, Option<String>) {
    if let Some(inner) = extract_tag(text, "timestamp") {
        let label = inner.trim().to_string();
        // strip tag from text
        let open = "<timestamp>";
        let close = "</timestamp>";
        let mut s = text.to_string();
        if let Some(start) = s.find(open) {
            if let Some(rel) = s[start..].find(close) {
                let end = start + rel + close.len();
                s.replace_range(start..end, "");
            }
        }
        (s.trim().to_string(), Some(label))
    } else {
        (text.to_string(), None)
    }
}

fn strip_system_reminders(t: &str) -> String {
    let mut s = t.to_string();
    // Remove <system-reminder>…</system-reminder> chunks.
    while let Some(start) = s.find("<system-reminder>") {
        if let Some(rel) = s[start..].find("</system-reminder>") {
            let end = start + rel + "</system-reminder>".len();
            s.replace_range(start..end, "");
        } else {
            break;
        }
    }
    s.trim().to_string()
}

fn extract_tag(text: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let start = text.find(&open)? + open.len();
    let end = text[start..].find(&close)? + start;
    Some(text[start..end].trim().to_string())
}

fn value_text(v: Option<&Value>) -> Option<String> {
    let v = v?;
    if let Some(s) = v.as_str() {
        return Some(s.to_string());
    }
    if let Some(arr) = v.as_array() {
        let mut parts = Vec::new();
        for it in arr {
            if let Some(t) = it.get("text").and_then(|x| x.as_str()) {
                parts.push(t.to_string());
            } else if let Some(t) = it
                .get("type")
                .and_then(|t| t.as_str())
                .filter(|t| *t == "output_text" || *t == "input_text" || *t == "text")
                .and_then(|_| it.get("text").and_then(|x| x.as_str()))
            {
                parts.push(t.to_string());
            }
        }
        if !parts.is_empty() {
            return Some(parts.join("\n"));
        }
    }
    None
}

fn parse_args_value(v: Option<&Value>) -> Option<Value> {
    let v = v?;
    if let Some(s) = v.as_str() {
        return serde_json::from_str(s)
            .ok()
            .or_else(|| Some(Value::String(s.to_string())));
    }
    Some(v.clone())
}

fn trunc(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let t: String = s.chars().take(max).collect();
    format!("{t}…")
}

// Re-export path helper visibility: title::transcript_path is pub(super) — same module tree OK.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_claude_style_assistant_and_tool() {
        let lines = vec![
            r#"{"type":"user","message":{"content":[{"type":"text","text":"hello"}]}}"#.to_string(),
            r#"{"type":"assistant","message":{"content":[{"type":"text","text":"hi"},{"type":"tool_use","name":"Bash","input":{"command":"ls"}}]}}"#.to_string(),
            r#"{"type":"user","message":{"content":[{"type":"tool_result","content":"a\nb"}]}}"#.to_string(),
        ];
        let mut events = Vec::new();
        let mut open = Vec::new();
        for l in lines {
            let v: Value = serde_json::from_str(&l).unwrap();
            push_full(AgentKind::Claude, &v, &mut events, &mut open);
        }
        assert!(matches!(&events[0], TranscriptEvent::UserText { text, .. } if text == "hello"));
        assert!(matches!(&events[1], TranscriptEvent::AssistantText { text, .. } if text == "hi"));
        match &events[2] {
            TranscriptEvent::ToolCall {
                name,
                args_summary,
                result_summary,
                ..
            } => {
                assert_eq!(name, "Bash");
                // Watch-style one-liner; results omitted unless error / AskHuman.
                assert!(args_summary.contains("运行") || args_summary.contains("ls"));
                assert_eq!(result_summary.as_deref(), None);
            }
            _ => panic!("expected tool"),
        }
    }

    #[test]
    fn detect_askhuman_cli() {
        let args = serde_json::json!({"command": "AskHuman -m \"pick one\""});
        let ah = detect_askhuman("Bash", Some(&args), None).unwrap();
        assert!(ah.question.contains("pick one"));
    }

    #[test]
    fn parse_answer_markers() {
        let c = "[status] answered\n[user_input]\nyes please\n[files]\n";
        assert_eq!(parse_askhuman_answer(c).as_deref(), Some("yes please"));
    }

    #[test]
    fn parse_iso8601_z_and_offset() {
        // 2026-06-13T10:09:57Z
        let t = parse_iso8601_secs("2026-06-13T10:09:57.062Z").unwrap();
        assert!(t > 1_700_000_000);
        // with +08:00 should be 8h earlier in UTC epoch than local wall for same digits
        let t2 = parse_iso8601_secs("2026-06-13T18:09:57+08:00").unwrap();
        assert_eq!(t2, t); // 18:09+08 == 10:09Z
    }

    #[test]
    fn event_time_from_claude_style() {
        let v = serde_json::json!({
            "type": "user",
            "timestamp": "2026-06-13T10:09:57.062Z",
            "message": { "content": [{"type":"text","text":"hi"}] }
        });
        assert!(event_time(&v).is_some());
    }

    #[test]
    fn real_claude_file_has_times() {
        let home = dirs::home_dir().expect("home");
        let projects = home.join(".claude/projects");
        let mut found = None;
        if let Ok(walk) = std::fs::read_dir(&projects) {
            for e in walk.flatten() {
                if let Ok(rd) = std::fs::read_dir(e.path()) {
                    for f in rd.flatten() {
                        let path = f.path();
                        if path.extension().and_then(|x| x.to_str()) == Some("jsonl") {
                            if path.metadata().map(|m| m.len()).unwrap_or(0) > 5000 {
                                found = Some(path);
                                break;
                            }
                        }
                    }
                }
                if found.is_some() {
                    break;
                }
            }
        }
        let path = found.expect("need a claude jsonl sample");
        let doc = load_path(AgentKind::Claude, &path).expect("load");
        let with_time = doc
            .events
            .iter()
            .filter(|e| match e {
                TranscriptEvent::UserText { at, .. }
                | TranscriptEvent::AssistantText { at, .. } => at.is_some(),
                _ => false,
            })
            .count();
        let total_ua = doc
            .events
            .iter()
            .filter(|e| {
                matches!(
                    e,
                    TranscriptEvent::UserText { .. } | TranscriptEvent::AssistantText { .. }
                )
            })
            .count();
        eprintln!(
            "file={path:?} events={} user/assistant={total_ua} with_time={with_time}",
            doc.events.len()
        );
        assert!(
            with_time > 0,
            "expected some User/Assistant events to have timestamps from real claude file"
        );
        // also render md snippet
        let md = crate::export::render_transcript_md(&doc, "test");
        eprintln!("md head:\n{}", md.chars().take(800).collect::<String>());
        assert!(
            md.contains("·") || md.contains(":"),
            "expected time in md headings"
        );
    }

    #[test]
    fn real_grok_and_claude_times() {
        let home = dirs::home_dir().unwrap();
        // Claude
        let claude = home.join(".claude/projects");
        let mut claude_path = None;
        if let Ok(walk) = std::fs::read_dir(&claude) {
            for e in walk.flatten() {
                if let Ok(rd) = std::fs::read_dir(e.path()) {
                    for f in rd.flatten() {
                        let p = f.path();
                        if p.extension().and_then(|x| x.to_str()) == Some("jsonl")
                            && p.metadata().map(|m| m.len()).unwrap_or(0) > 10000
                        {
                            claude_path = Some(p);
                            break;
                        }
                    }
                }
                if claude_path.is_some() {
                    break;
                }
            }
        }
        if let Some(p) = claude_path {
            let doc = load_path(AgentKind::Claude, &p).unwrap();
            let md = crate::export::render_transcript_md(&doc, "claude");
            let has = md.lines().any(|l| l.starts_with("### ") && l.contains("·"));
            eprintln!("CLAUDE has_time_in_heading={has}");
            assert!(has);
        }
        // Grok this workspace
        let grok = home.join(".grok/sessions");
        let mut grok_path = None;
        if let Ok(walk) = std::fs::read_dir(&grok) {
            for e in walk.flatten() {
                let p = e
                    .path()
                    .join("019f4c59-48ad-7462-b148-e50634641c3e")
                    .join("chat_history.jsonl");
                if p.is_file() {
                    grok_path = Some(p);
                    break;
                }
                // also any chat_history
                if let Ok(rd) = std::fs::read_dir(e.path()) {
                    for f in rd.flatten() {
                        let ch = f.path().join("chat_history.jsonl");
                        if ch.is_file() && ch.metadata().map(|m| m.len()).unwrap_or(0) > 100000 {
                            grok_path = Some(ch);
                            break;
                        }
                    }
                }
                if grok_path.is_some() {
                    break;
                }
            }
        }
        if let Some(p) = grok_path {
            let doc = load_path(AgentKind::Grok, &p).unwrap();
            let md = crate::export::render_transcript_md(&doc, "grok");
            let has = md.lines().any(|l| l.starts_with("### ") && l.contains("·"));
            eprintln!(
                "GROK file={p:?} events={} has_time_in_heading={has}",
                doc.events.len()
            );
            eprintln!("GROK sample headings:");
            for l in md.lines().filter(|l| l.starts_with("### ")).take(8) {
                eprintln!("  {l}");
            }
        }
    }
}
