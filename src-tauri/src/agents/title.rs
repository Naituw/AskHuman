//! 把 `session_id` 解析成「对话标题」，复刻各家恢复对话列表里显示的标题（FINDINGS / spec D10）。
//!
//! - Cursor: the IDE database name, then CLI chat metadata, then the first user message.
//! - Codex：`~/.codex/sessions/**/rollout-*-<sid>.jsonl` 首条**真实**用户消息（跳过注入块；
//!   paginated 会话读 `item_completed` `UserMessage`，legacy 读 `event_msg/user_message`）。
//! - Claude: the latest manual title, automatic title, or legacy summary; then the first user message.
//!
//! 全部 best-effort：文件可能不存在 / 正在写 / 巨大，任何失败都返回 `None`（窗口显示「未命名」）。
//! 读 jsonl 有行数与字节上限，避免拖慢 daemon。

use crate::paths;
use serde_json::Value;
use std::fs;
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use super::AgentKind;

/// 标题最大展示长度（字符）。
const MAX_TITLE_CHARS: usize = 80;
/// 扫描 jsonl 的行数上限。
const MAX_LINES: usize = 4000;
const MAX_NAME_TAIL_BYTES: u64 = 1024 * 1024;
/// How much of an Antigravity conversation log is read to find its first request.
const MAX_ANTIGRAVITY_HEAD_BYTES: u64 = 4 * 1024 * 1024;
const MAX_PI_HEADER_BYTES: u64 = 64 * 1024;
const MAX_PI_SESSION_FILES: usize = 4096;

// A bound test session may only resolve files inside its own native client profile.
fn development_title(kind: AgentKind, session_id: &str) -> Option<Option<String>> {
    if crate::development::session::for_native(kind, session_id).is_some() {
        let path = crate::development::session::transcript_for(kind, session_id);
        return Some(path.and_then(|path| {
            match kind {
                AgentKind::Codex => codex_user_message(&path).or_else(|| first_user_message(&path)),
                AgentKind::Claude => {
                    claude_session_name(&path).or_else(|| first_user_message(&path))
                }
                AgentKind::Grok => path
                    .parent()
                    .and_then(grok_session_name)
                    .or_else(|| grok_first_query(&path)),
                AgentKind::Pi => pi_session_name(&path)
                    .flatten()
                    .or_else(|| pi_first_user(&path)),
                _ => None,
            }
        }));
    }
    crate::development::session::requires_test_profile().then_some(None)
}

/// 解析指定家族某 session 的标题。取不到返回 `None`。
pub fn resolve_title(kind: AgentKind, session_id: &str) -> Option<String> {
    if let Some(title) = development_title(kind, session_id) {
        return title.map(|s| clean_title(&s));
    }
    if session_id.is_empty() {
        return None;
    }
    let raw = match kind {
        AgentKind::Cursor => cursor_title(session_id),
        AgentKind::Codex => codex_title(session_id),
        AgentKind::Claude => claude_title(session_id),
        AgentKind::Grok => grok_title(session_id),
        AgentKind::Pi => pi_title(session_id),
        AgentKind::Antigravity => antigravity_title(session_id),
    }?;
    Some(clean_title(&raw))
}

/// Refresh native names once per popup presentation, retaining cached titles if metadata is absent.
pub(crate) fn resolve_popup_title(
    kind: AgentKind,
    session_id: &str,
    cached: Option<String>,
    codex_config: &crate::codex_desktop::Config,
) -> Option<String> {
    if let Some(title) = development_title(kind, session_id) {
        return title.or(cached);
    }
    if session_id.trim().is_empty() {
        return None;
    }
    let saved = match kind {
        AgentKind::Cursor => cursor_session_name(session_id).map(Some),
        AgentKind::Codex => crate::codex_desktop::session_title(codex_config, session_id).map(Some),
        AgentKind::Claude => claude_session_file(session_id)
            .and_then(|path| claude_session_name(&path))
            .map(Some),
        AgentKind::Grok => grok_session_dir(session_id)
            .and_then(|path| grok_session_name(&path))
            .map(Some),
        AgentKind::Pi => pi_session_file(session_id).and_then(|path| pi_session_name(&path)),
        // A conversation has no name of its own; the fallback below uses its first request.
        AgentKind::Antigravity => None,
    };
    // Pi records an explicit empty name when the user clears it. Do not resurrect a cached name.
    let cached = if saved == Some(None) { None } else { cached };
    saved
        .flatten()
        .or_else(|| cached.filter(|title| !title.trim().is_empty()))
        .or_else(|| resolve_title(kind, session_id))
        // The sidebar truncates visually; preserve the full saved name for its tooltip.
        .map(|title| title.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|title| !title.is_empty())
}

/// The title the registry keeps for the lists built from it (status window, tray, IM and phone
/// lists): the same native-name-first rule as [`resolve_popup_title`], clipped like
/// [`resolve_title`] so those surfaces keep their bounded titles.
pub(crate) fn resolve_list_title(
    kind: AgentKind,
    session_id: &str,
    cached: Option<String>,
    codex_config: &crate::codex_desktop::Config,
) -> Option<String> {
    resolve_popup_title(kind, session_id, cached, codex_config).map(|title| clean_title(&title))
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

fn cursor_session_name(session_id: &str) -> Option<String> {
    // 0) IDE 形态：全局 state.vscdb 的官方标题（实时、与 IDE 恢复列表一致）。
    if let Some(t) = super::cursor_vscdb::resolve_title(session_id) {
        return Some(t);
    }
    // 1) ~/.cursor/chats/*/<sid>/meta.json (and enabled extra Cursor profiles)
    let mut roots = vec![paths::cursor_dir()];
    roots.extend(crate::integrations::agent_profiles::enabled_roots(
        AgentKind::Cursor,
    ));
    for root in roots {
        let chats = root.join("chats");
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
    }
    None
}

fn cursor_title(session_id: &str) -> Option<String> {
    if let Some(title) = cursor_session_name(session_id) {
        return Some(title);
    }
    if let Some(path) = transcript_path(AgentKind::Cursor, session_id) {
        if let Some(t) = first_user_message(&path) {
            return Some(t);
        }
    }
    None
}

// ── Codex ──

fn codex_title(session_id: &str) -> Option<String> {
    let file = transcript_path(AgentKind::Codex, session_id)?;
    // 优先：`event_msg{payload.type=="user_message"}` 的 message——这是用户真正键入的内容，
    // 绕开会话开头作为 role=user 注入的 AGENTS.md 指令块与 `<environment_context>` 等。
    if let Some(t) = codex_user_message(&file) {
        return Some(t);
    }
    // 回退：response_item 首条真实用户消息（已跳过注入块）。
    first_user_message(&file)
}

/// Codex：扫描取首条真实用户消息。
///
/// Legacy rollout: `event_msg{payload.type=="user_message"}` 的 `message`。
/// Paginated rollout (0.147+): `event_msg/item_completed` `UserMessage` 的 content。
/// Codex 只为用户真实输入发出这些事件（注入的上下文走 response_item），故无需再过滤注入块。
fn codex_user_message(path: &Path) -> Option<String> {
    let file = fs::File::open(path).ok()?;
    let reader = BufReader::new(file);
    for (i, line) in reader.lines().enumerate() {
        if i >= MAX_LINES {
            break;
        }
        let Ok(line) = line else { break };
        if !line.contains("user_message") && !line.contains("UserMessage") {
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
        match payload.get("type").and_then(|t| t.as_str()) {
            Some("user_message") => {
                if let Some(msg) = payload.get("message").and_then(|m| m.as_str()) {
                    let cleaned = super::transcript_full::strip_host_wrappers(msg);
                    if !cleaned.is_empty() {
                        return Some(cleaned);
                    }
                }
            }
            Some("item_completed") => {
                let item = payload.get("item");
                if item
                    .and_then(|item| item.get("type"))
                    .and_then(|t| t.as_str())
                    == Some("UserMessage")
                {
                    if let Some(text) = item.and_then(super::transcript_full::codex_turn_item_text)
                    {
                        let cleaned = super::transcript_full::strip_host_wrappers(&text);
                        if !cleaned.is_empty() {
                            return Some(cleaned);
                        }
                    }
                }
            }
            _ => {}
        }
    }
    None
}

// ── Claude ──

fn claude_session_file(session_id: &str) -> Option<PathBuf> {
    if let Some(path) = super::session_paths::get(AgentKind::Claude, session_id) {
        return Some(path);
    }
    let target = format!("{session_id}.jsonl");
    let mut roots = vec![paths::claude_dir()];
    roots.extend(crate::integrations::agent_profiles::enabled_roots(
        AgentKind::Claude,
    ));
    for root in roots {
        if let Some(found) = find_file_recursive(&root.join("projects"), &target, 3) {
            return Some(found);
        }
    }
    None
}

fn claude_title(session_id: &str) -> Option<String> {
    let file = claude_session_file(session_id)?;
    claude_session_name(&file).or_else(|| first_user_message(&file))
}

// ── Grok ──

/// Grok 会话布局：`~/.grok/sessions/<url编码的 cwd>/<session_id>/`，内含 `summary.json`
/// （`session_summary` / `generated_title` 即恢复列表标题）与 `chat_history.jsonl`。
/// Prefer generated_title (including manual renames), then session_summary and the first user input.
///
/// 注意 Grok（cursor harness）会把用户真实输入包在 `<user_query>…</user_query>` 里，而其它以 `<`
/// 开头的块（`<user_info>`/`<git_status>` 等）才是注入上下文；故回退需先**解包** `<user_query>`，
/// 不能直接套用 [`first_user_message`]（它会把所有以 `<` 开头的文本当注入块跳过）。
fn grok_title(session_id: &str) -> Option<String> {
    let dir = grok_session_dir(session_id)?;
    grok_session_name(&dir).or_else(|| grok_first_query(&dir.join("chat_history.jsonl")))
}

pub(super) fn grok_session_dir(session_id: &str) -> Option<PathBuf> {
    if let Some(dir) = super::session_paths::get(AgentKind::Grok, session_id)
        .and_then(|path| path.parent().map(Path::to_path_buf))
        .filter(|dir| dir.is_dir())
    {
        return Some(dir);
    }
    let mut session_roots = vec![paths::grok_sessions_dir()];
    session_roots.extend(
        crate::integrations::agent_profiles::enabled_roots(AgentKind::Grok)
            .into_iter()
            .map(|root| root.join("sessions")),
    );
    for sessions in session_roots {
        // 遍历各 cwd 目录，定位含本 session_id 子目录者（量小、best-effort）。
        let Ok(entries) = fs::read_dir(&sessions) else {
            continue;
        };
        for e in entries.flatten() {
            let dir = e.path().join(session_id);
            if dir.is_dir() {
                return Some(dir);
            }
        }
    }
    None
}

fn grok_session_name(dir: &Path) -> Option<String> {
    // The generated_title field also holds manual /rename titles (title_is_manual).
    let file = fs::File::open(dir.join("summary.json")).ok()?;
    let value: Value = serde_json::from_reader(file.take(MAX_NAME_TAIL_BYTES)).ok()?;
    ["generated_title", "session_summary"]
        .into_iter()
        .find_map(|key| {
            value
                .get(key)
                .and_then(Value::as_str)
                .filter(|s| !s.trim().is_empty())
                .map(str::to_string)
        })
}

// ── Pi ──

fn pi_title(session_id: &str) -> Option<String> {
    let path = pi_session_file(session_id)?;
    pi_session_name(&path)
        .flatten()
        .or_else(|| pi_first_user(&path))
}

fn pi_session_name(path: &Path) -> Option<Option<String>> {
    let value = name_tail(path)
        .into_iter()
        .rev()
        .find(|value| value.get("type").and_then(Value::as_str) == Some("session_info"))?;
    Some(
        value
            .get("name")
            .and_then(Value::as_str)
            .filter(|s| !s.trim().is_empty())
            .map(str::to_string),
    )
}

fn pi_first_user(path: &Path) -> Option<String> {
    let file = fs::File::open(path).ok()?;
    let reader = BufReader::new(file);
    let mut first_user = None;
    for (index, line) in reader.lines().enumerate() {
        if index >= MAX_LINES {
            break;
        }
        let Ok(line) = line else { break };
        let Ok(value) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        if first_user.is_none()
            && value.get("type").and_then(Value::as_str) == Some("message")
            && value.pointer("/message/role").and_then(Value::as_str) == Some("user")
        {
            if let Some(text) = value.pointer("/message/content").and_then(content_to_text) {
                let cleaned = super::transcript_full::strip_host_wrappers(&text);
                if !cleaned.is_empty() && !is_injected_block(&cleaned) {
                    first_user = Some(cleaned);
                }
            }
        }
    }
    first_user
}

fn pi_session_file(session_id: &str) -> Option<PathBuf> {
    super::session_paths::get(AgentKind::Pi, session_id).or_else(|| {
        let mut remaining = MAX_PI_SESSION_FILES;
        let mut roots = vec![paths::pi_sessions_dir()];
        roots.extend(
            crate::integrations::agent_profiles::enabled_roots(AgentKind::Pi)
                .into_iter()
                .map(|root| root.join("sessions")),
        );
        for root in roots {
            if let Some(found) = find_file_by_header(&root, session_id, 5, &mut remaining) {
                return Some(found);
            }
        }
        None
    })
}

fn find_file_by_header(
    root: &Path,
    session_id: &str,
    depth: usize,
    remaining: &mut usize,
) -> Option<PathBuf> {
    if depth == 0 || *remaining == 0 {
        return None;
    }
    for entry in fs::read_dir(root).ok()?.flatten() {
        if *remaining == 0 {
            return None;
        }
        *remaining -= 1;
        let path = entry.path();
        if path.is_dir() {
            if let Some(found) = find_file_by_header(&path, session_id, depth - 1, remaining) {
                return Some(found);
            }
            continue;
        }
        if path.extension().and_then(|value| value.to_str()) != Some("jsonl") {
            continue;
        }
        let Ok(file) = fs::File::open(&path) else {
            continue;
        };
        let mut first = String::new();
        if BufReader::new(file)
            .take(MAX_PI_HEADER_BYTES)
            .read_line(&mut first)
            .is_err()
        {
            continue;
        }
        let Ok(header) = serde_json::from_str::<Value>(&first) else {
            continue;
        };
        if header.get("type").and_then(Value::as_str) == Some("session")
            && header.get("id").and_then(Value::as_str) == Some(session_id)
        {
            return Some(path);
        }
    }
    None
}

/// 扫 Grok `chat_history.jsonl` 取首条真实用户输入：优先解包 `<user_query>…</user_query>` 的内层文本；
/// 若某条用户文本本身不以 `<` 开头（Build harness 可能不加包裹）则直接取用。均跳过纯注入块。
fn grok_first_query(path: &Path) -> Option<String> {
    let file = fs::File::open(path).ok()?;
    let reader = BufReader::new(file);
    for (i, line) in reader.lines().enumerate() {
        if i >= MAX_LINES {
            break;
        }
        let Ok(line) = line else { break };
        if !line.contains("\"user\"") {
            continue;
        }
        let Ok(v) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        if !is_user_line(&v) {
            continue;
        }
        let Some(text) = extract_text(&v) else {
            continue;
        };
        let t = super::transcript_full::strip_host_wrappers(&text);
        if !t.is_empty() && !is_injected_block(&t) {
            return Some(t);
        }
    }
    None
}

/// 提取 `<tag>…</tag>` 之间的内层文本（找不到成对标签返回 `None`）。
#[cfg(test)]
pub(super) fn unwrap_tag(text: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let start = text.find(&open)? + open.len();
    let end = text[start..].find(&close)? + start;
    Some(text[start..end].to_string())
}

// ── Antigravity ──

/// Antigravity has no title the console can read, so a conversation is named after what the person
/// first asked.
fn antigravity_title(session_id: &str) -> Option<String> {
    let path = transcript_path(AgentKind::Antigravity, session_id)?;
    antigravity_first_request(&path)
}

/// Where the App (`antigravity`), `agy` (`antigravity-cli`), and ACP (`antigravity-acp`) keep a
/// conversation's step log under one Gemini root:
/// `<gemini_root>/<surface>/brain/<conversation id>/.system_generated/logs/transcript_full.jsonl`.
pub(crate) fn antigravity_transcript_in_root(
    gemini_root: &Path,
    session_id: &str,
) -> Option<PathBuf> {
    let id = uuid::Uuid::parse_str(session_id)
        .ok()?
        .hyphenated()
        .to_string();
    paths::antigravity_brain_dirs_in_root(gemini_root)
        .into_iter()
        .map(|brain| {
            brain
                .join(&id)
                .join(".system_generated/logs/transcript_full.jsonl")
        })
        .find(|path| path.is_file())
}

/// Where the App (`antigravity`), `agy` (`antigravity-cli`), and ACP (`antigravity-acp`) keep a
/// conversation's step log below a home directory.
/// The id arrives from a hook or from MCP metadata, so only its canonical UUID form is ever joined
/// into a path.
pub(crate) fn antigravity_transcript_in(home: &Path, session_id: &str) -> Option<PathBuf> {
    antigravity_transcript_in_root(&home.join(".gemini"), session_id)
}

/// The first request the person typed: the first `USER_INPUT` step the user wrote, with the
/// `<USER_REQUEST>` and external-host wrappers removed. A conversation started by another one (a
/// subagent) begins with a system message instead and has no such step.
fn antigravity_first_request(path: &Path) -> Option<String> {
    let file = fs::File::open(path).ok()?;
    let reader = BufReader::new(file.take(MAX_ANTIGRAVITY_HEAD_BYTES));
    for (index, line) in reader.lines().enumerate() {
        if index >= MAX_LINES {
            break;
        }
        let Ok(line) = line else { break };
        if !line.contains("USER_INPUT") {
            continue;
        }
        let Ok(step) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        if step.get("type").and_then(Value::as_str) != Some("USER_INPUT")
            || step.get("source").and_then(Value::as_str) != Some("USER_EXPLICIT")
        {
            continue;
        }
        let Some(content) = step.get("content").and_then(Value::as_str) else {
            continue;
        };
        let request = super::transcript_full::strip_host_wrappers(content);
        if !request.is_empty() {
            return Some(request);
        }
    }
    None
}

// ── transcript 文件定位（供 activity.rs 复用）──

/// 按 `session_id` 定位某家 agent 的 transcript（jsonl）文件路径。取不到返回 `None`。
/// 与 `resolve_title` 的取标题不同，这里定位的是**对话流水**文件（供解析尾部「当前活动」）。
pub(crate) fn transcript_path(kind: AgentKind, session_id: &str) -> Option<PathBuf> {
    if let Some(path) = crate::development::session::transcript_for(kind, session_id) {
        return Some(path);
    }
    if session_id.is_empty() || crate::development::session::requires_test_profile() {
        return None;
    }
    if let Some(path) = super::session_paths::get(kind, session_id) {
        return Some(path);
    }
    match kind {
        // The conversation log of the App, `agy`, or enabled extra profiles (such as an external host's ACP homes),
        // found by the id a hook or MCP reported; the activity, latest-text and transcript views
        // all read it.
        AgentKind::Antigravity => antigravity_transcript_in(&paths::integration_home(), session_id)
            .or_else(|| {
                crate::integrations::agent_profiles::enabled_roots(AgentKind::Antigravity)
                    .into_iter()
                    .find_map(|root| antigravity_transcript_in_root(&root, session_id))
            }),
        // ~/.cursor/projects/*/agent-transcripts/<sid>/<sid>.jsonl (and enabled extra profiles)
        AgentKind::Cursor => {
            let mut roots = vec![paths::cursor_dir()];
            roots.extend(crate::integrations::agent_profiles::enabled_roots(
                AgentKind::Cursor,
            ));
            for root in roots {
                let projects = root.join("projects");
                let Ok(entries) = fs::read_dir(&projects) else {
                    continue;
                };
                for e in entries.flatten() {
                    let f = e
                        .path()
                        .join("agent-transcripts")
                        .join(session_id)
                        .join(format!("{session_id}.jsonl"));
                    if f.is_file() {
                        return Some(f);
                    }
                }
            }
            None
        }
        // ~/.codex/sessions/**/rollout-*-<sid>.jsonl (and enabled extra profiles)
        AgentKind::Codex => {
            let needle = format!("-{session_id}.jsonl");
            let mut roots = vec![paths::codex_dir()];
            roots.extend(crate::integrations::agent_profiles::enabled_roots(
                AgentKind::Codex,
            ));
            for root in roots {
                if let Some(found) = find_file_recursive(&root.join("sessions"), &needle, 4) {
                    return Some(found);
                }
            }
            None
        }
        // ~/.claude/projects/*/<sid>.jsonl (and enabled extra profiles)
        AgentKind::Claude => claude_session_file(session_id),
        // ~/.grok/sessions/<url编码 cwd>/<sid>/chat_history.jsonl (and enabled extra profiles)
        AgentKind::Grok => grok_session_dir(session_id)
            .map(|dir| dir.join("chat_history.jsonl"))
            .filter(|f| f.is_file()),
        AgentKind::Pi => pi_session_file(session_id),
    }
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
            // Cursor 把用户输入包在 `<timestamp>…</timestamp>` + `<user_query>` 里：先用
            // transcript 同款清洗剥壳（其它家族原样通过），再判注入块，否则真实输入会被
            // 「`<` 开头＝注入块」误滤而永远取不到标题（用户验收反馈）。
            let (cleaned, _) = super::transcript_full::clean_user(&text);
            let t = cleaned.trim();
            if t.is_empty() || is_injected_block(t) {
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

/// Read bounded metadata from the tail so a rename after a long conversation is visible.
fn name_tail(path: &Path) -> Vec<Value> {
    let Some(mut file) = fs::File::open(path).ok() else {
        return Vec::new();
    };
    let Some(len) = file.metadata().ok().map(|m| m.len()) else {
        return Vec::new();
    };
    let start = len.saturating_sub(MAX_NAME_TAIL_BYTES);
    if file.seek(SeekFrom::Start(start)).is_err() {
        return Vec::new();
    }
    let mut bytes = Vec::new();
    if file
        .take(MAX_NAME_TAIL_BYTES)
        .read_to_end(&mut bytes)
        .is_err()
    {
        return Vec::new();
    }
    let text = String::from_utf8_lossy(&bytes);
    text.lines()
        .skip(usize::from(start > 0))
        .filter_map(|line| {
            if !line.contains("custom-title")
                && !line.contains("ai-title")
                && !line.contains("summary")
                && !line.contains("session_info")
            {
                return None;
            }
            serde_json::from_str(line).ok()
        })
        .collect()
}

fn claude_session_name(path: &Path) -> Option<String> {
    let values = name_tail(path);
    // A manual name takes precedence over automatic titles and legacy summaries.
    [
        ("custom-title", "customTitle"),
        ("ai-title", "aiTitle"),
        ("summary", "summary"),
    ]
    .into_iter()
    .find_map(|(kind, field)| {
        values.iter().rev().find_map(|value| {
            (value.get("type").and_then(Value::as_str) == Some(kind))
                .then(|| {
                    value
                        .get(field)
                        .and_then(Value::as_str)
                        .filter(|s| !s.trim().is_empty())
                        .map(str::to_string)
                })
                .flatten()
        })
    })
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
pub(super) fn find_file_recursive(root: &Path, suffix: &str, max_depth: usize) -> Option<PathBuf> {
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
    fn popup_title_preserves_full_saved_names_and_keeps_cache_when_unavailable() {
        let home = tempfile::tempdir().unwrap();
        let config = crate::codex_desktop::Config {
            codex_home: home.path().to_string_lossy().into_owned(),
            ..Default::default()
        };
        let db = rusqlite::Connection::open(home.path().join("state_5.sqlite")).unwrap();
        db.execute_batch("CREATE TABLE threads(id TEXT PRIMARY KEY,name TEXT,title TEXT);")
            .unwrap();
        let name = "长会话名称".repeat(30);
        db.execute(
            "INSERT INTO threads VALUES ('session',?1,'Original prompt')",
            [&name],
        )
        .unwrap();
        assert_eq!(
            resolve_popup_title(
                AgentKind::Codex,
                "session",
                Some("Old cached name".into()),
                &config
            ),
            Some(name)
        );
        db.execute_batch("DROP TABLE threads;").unwrap();
        assert_eq!(
            resolve_popup_title(
                AgentKind::Codex,
                "session",
                Some("Cached name".into()),
                &config
            ),
            Some("Cached name".into())
        );
    }

    #[test]
    fn claude_manual_name_wins_over_later_automatic_names_and_refreshes_at_the_tail() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.jsonl");
        let mut lines =
            vec![r#"{"type":"ai-title","aiTitle":"Original automatic name"}"#.to_string()];
        // Names appended after the old line cap must still be read on the next request.
        for _ in 0..MAX_LINES + 10 {
            lines.push(r#"{"type":"progress"}"#.into());
        }
        lines.push(r#"{"type":"custom-title","customTitle":"Manual name"}"#.into());
        lines.push(r#"{"type":"ai-title","aiTitle":"Later automatic name"}"#.into());
        fs::write(&path, lines.join("\n")).unwrap();
        assert_eq!(claude_session_name(&path).as_deref(), Some("Manual name"));
        let mut file = fs::OpenOptions::new().append(true).open(&path).unwrap();
        use std::io::Write;
        writeln!(
            file,
            "\n{}",
            serde_json::json!({"type":"custom-title","customTitle":"Renamed manual name"})
        )
        .unwrap();
        assert_eq!(
            claude_session_name(&path).as_deref(),
            Some("Renamed manual name")
        );
    }

    #[test]
    fn grok_uses_the_native_title_instead_of_the_session_summary_and_rereads_renames() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("summary.json");
        fs::write(&path, r#"{"session_summary":"Long description","generated_title":"Manual name","title_is_manual":true}"#).unwrap();
        assert_eq!(
            grok_session_name(dir.path()).as_deref(),
            Some("Manual name")
        );
        fs::write(
            &path,
            r#"{"session_summary":"Long description","generated_title":"Renamed"}"#,
        )
        .unwrap();
        assert_eq!(grok_session_name(dir.path()).as_deref(), Some("Renamed"));
        fs::write(
            &path,
            r#"{"session_summary":"Legacy name","generated_title":" "}"#,
        )
        .unwrap();
        assert_eq!(
            grok_session_name(dir.path()).as_deref(),
            Some("Legacy name")
        );
    }

    #[test]
    fn name_tail_drops_partial_json_and_reads_only_its_byte_budget() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.jsonl");
        let mut text = format!("{}\n", "x".repeat(MAX_NAME_TAIL_BYTES as usize + 20));
        text.push_str(r#"{"type":"ai-title","aiTitle":"Latest automatic name"}"#);
        fs::write(&path, text).unwrap();
        assert_eq!(
            claude_session_name(&path).as_deref(),
            Some("Latest automatic name")
        );
    }

    /// A conversation log below `home` for the given surface (`antigravity` or `antigravity-cli`).
    fn antigravity_log(home: &Path, surface: &str, id: &str, lines: &[&str]) -> PathBuf {
        let dir = home
            .join(".gemini")
            .join(surface)
            .join("brain")
            .join(id)
            .join(".system_generated/logs");
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("transcript_full.jsonl");
        fs::write(&path, lines.join("\n")).unwrap();
        path
    }

    const AGY_USER_STEP: &str = r#"{"step_index":0,"source":"USER_EXPLICIT","type":"USER_INPUT","status":"DONE","created_at":"2026-11-15T18:06:23Z","content":"<USER_REQUEST>\nStep 1: call the lab_echo tool once.\nStep 2: reply with done.\n</USER_REQUEST>\n<ADDITIONAL_METADATA>\nThe current local time is later.\n</ADDITIONAL_METADATA>"}"#;
    const AGY_REPLY_STEP: &str = r#"{"step_index":1,"source":"MODEL","type":"PLANNER_RESPONSE","status":"DONE","content":"done"}"#;
    /// What a subagent's log starts with: a message from the parent, not something the user typed.
    const AGY_SUBAGENT_STEP: &str = r#"{"step_index":0,"source":"SYSTEM","type":"SYSTEM_MESSAGE","status":"DONE","content":"The following is a <SYSTEM_MESSAGE> not sent by the user. USER_INPUT is not here."}"#;

    #[test]
    fn antigravity_conversation_is_named_after_its_first_request() {
        let home = tempfile::tempdir().unwrap();
        let id = "b9fee8f0-784e-4a24-8b56-fb29c435e8bd";
        let path = antigravity_log(
            home.path(),
            "antigravity",
            id,
            &[AGY_USER_STEP, AGY_REPLY_STEP],
        );
        assert_eq!(
            antigravity_transcript_in(home.path(), id).as_deref(),
            Some(path.as_path())
        );
        // The wrapper and the metadata after it are not part of what the person asked.
        let request = antigravity_first_request(&path).unwrap();
        assert_eq!(
            request,
            "Step 1: call the lab_echo tool once.\nStep 2: reply with done."
        );
        assert_eq!(
            clean_title(&request),
            "Step 1: call the lab_echo tool once. Step 2: reply with done."
        );
    }

    #[test]
    fn antigravity_cli_conversations_are_found_next_to_the_apps() {
        let home = tempfile::tempdir().unwrap();
        let (app_id, cli_id) = (
            "b9fee8f0-784e-4a24-8b56-fb29c435e8bd",
            "0d1bf56b-618e-4476-b7df-04ae3ff77a3a",
        );
        let app = antigravity_log(home.path(), "antigravity", app_id, &[AGY_USER_STEP]);
        let cli = antigravity_log(home.path(), "antigravity-cli", cli_id, &[AGY_USER_STEP]);
        assert_eq!(
            antigravity_transcript_in(home.path(), app_id).as_deref(),
            Some(app.as_path())
        );
        assert_eq!(
            antigravity_transcript_in(home.path(), cli_id).as_deref(),
            Some(cli.as_path())
        );
        assert_eq!(
            antigravity_transcript_in(home.path(), "c2c12c86-2d21-47d7-97a8-552e6c1aec81"),
            None
        );
    }

    #[test]
    fn only_a_canonical_uuid_ever_becomes_part_of_an_antigravity_path() {
        let home = tempfile::tempdir().unwrap();
        let id = "b9fee8f0-784e-4a24-8b56-fb29c435e8bd";
        let path = antigravity_log(home.path(), "antigravity", id, &[AGY_USER_STEP]);
        // Another spelling of the same UUID names the same directory.
        for same in [
            "B9FEE8F0-784E-4A24-8B56-FB29C435E8BD",
            "b9fee8f0784e4a248b56fb29c435e8bd",
        ] {
            assert_eq!(
                antigravity_transcript_in(home.path(), same).as_deref(),
                Some(path.as_path()),
                "{same}"
            );
        }
        for bad in [
            "",
            "not-a-uuid",
            "../antigravity/brain/b9fee8f0-784e-4a24-8b56-fb29c435e8bd",
            "b9fee8f0-784e-4a24-8b56-fb29c435e8bd/..",
            "/etc/passwd",
        ] {
            assert_eq!(antigravity_transcript_in(home.path(), bad), None, "{bad:?}");
        }
    }

    #[test]
    fn antigravity_titles_skip_what_the_person_did_not_type() {
        let home = tempfile::tempdir().unwrap();
        let id = "0d1bf56b-618e-4476-b7df-04ae3ff77a3a";
        let first = |lines: &[&str]| {
            let path = antigravity_log(home.path(), "antigravity", id, lines);
            antigravity_first_request(&path)
        };
        // A subagent's log starts with a message from its parent, so it stays unnamed.
        assert_eq!(first(&[AGY_SUBAGENT_STEP, AGY_REPLY_STEP]), None);
        assert_eq!(first(&[]), None);
        // Steps that only mention the type, are not JSON, or are cut off are passed over.
        assert_eq!(
            first(&[
                r#"{"step_index":0,"source":"MODEL","type":"PLANNER_RESPONSE","content":"USER_INPUT"}"#,
                "USER_INPUT but not JSON",
                r#"{"step_index":1,"source":"USER_EXPLICIT","type":"USER_INPUT","content":"<USER_REQUEST>\n \n</USER_REQUEST>"}"#,
                r#"{"step_index":2,"source":"USER_EXPLICIT","type":"USER_INPUT"}"#,
                r#"{"step_index":3,"source":"USER_EXPLICIT","type":"USER_INPUT","content":"plain text, no wrapper"}"#,
                AGY_USER_STEP,
            ])
            .as_deref(),
            Some("plain text, no wrapper")
        );
        // The first request wins over a later one.
        assert_eq!(
            first(&[
                AGY_REPLY_STEP,
                AGY_USER_STEP,
                r#"{"step_index":9,"source":"USER_EXPLICIT","type":"USER_INPUT","content":"<USER_REQUEST>\nlater\n</USER_REQUEST>"}"#,
            ])
            .as_deref(),
            Some("Step 1: call the lab_echo tool once.\nStep 2: reply with done.")
        );
    }

    #[test]
    fn an_oversized_antigravity_log_is_not_read_past_its_byte_budget() {
        let home = tempfile::tempdir().unwrap();
        let id = "b9fee8f0-784e-4a24-8b56-fb29c435e8bd";
        let padding = format!(
            r#"{{"step_index":0,"source":"MODEL","type":"PLANNER_RESPONSE","content":"{}"}}"#,
            "x".repeat(MAX_ANTIGRAVITY_HEAD_BYTES as usize)
        );
        let path = antigravity_log(
            home.path(),
            "antigravity",
            id,
            &[padding.as_str(), AGY_USER_STEP],
        );
        assert_eq!(antigravity_first_request(&path), None);
    }

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
        let v: Value =
            serde_json::json!({"payload":{"content":[{"type":"input_text","text":"hi there"}]}});
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

    /// Cursor 把用户输入包在 `<timestamp>…</timestamp>` + `<user_query>` 里（用户验收反馈：
    /// 全部消息被当注入块滤掉 → 永远「未命名」）：回退路径须剥壳后取到真实问题。
    #[test]
    fn first_user_message_unwraps_cursor_timestamp_and_user_query() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("s.jsonl");
        let lines = [
            r#"{"role":"user","message":{"content":[{"type":"text","text":"<timestamp>Friday, Jul 24, 2026, 11:07 PM (UTC+8)</timestamp>\n<user_query>\n我想优化一下 agent 状态窗口\n</user_query>"}]}}"#,
        ];
        std::fs::write(&f, lines.join("\n")).unwrap();
        assert_eq!(
            first_user_message(&f).as_deref(),
            Some("我想优化一下 agent 状态窗口")
        );
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
    fn grok_first_query_unwraps_user_query_and_skips_injection() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("chat_history.jsonl");
        let lines = [
            r#"{"type":"system","content":"You are ..."}"#,
            r#"{"type":"user","content":[{"type":"text","text":"<user_info>\nOS...\n</user_info>\n<git_status>...</git_status>"}]}"#,
            r#"{"type":"user","content":[{"type":"text","text":"<user_query>\n帮我加一个功能\n</user_query>"}]}"#,
        ];
        std::fs::write(&f, lines.join("\n")).unwrap();
        assert_eq!(grok_first_query(&f).as_deref(), Some("帮我加一个功能"));
    }

    #[test]
    fn grok_first_query_falls_back_to_plain_text() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("chat_history.jsonl");
        // Build harness 可能不加 <user_query> 包裹：直接取首条非注入用户文本。
        let lines = [
            r#"{"type":"user","content":[{"type":"text","text":"<user_info>ctx</user_info>"}]}"#,
            r#"{"type":"user","content":"直接的问题"}"#,
        ];
        std::fs::write(&f, lines.join("\n")).unwrap();
        assert_eq!(grok_first_query(&f).as_deref(), Some("直接的问题"));
    }

    #[test]
    fn unwrap_tag_extracts_inner() {
        assert_eq!(
            unwrap_tag("a<user_query>\nhi\n</user_query>b", "user_query").as_deref(),
            Some("\nhi\n")
        );
        assert_eq!(unwrap_tag("no tags here", "user_query"), None);
    }

    /// Codex 0.148 paginated exec: no `event_msg/user_message`; authority is
    /// `item_completed` + `UserMessage`. Primary title path reads that event;
    /// response_item fallback still finds the real prompt after skipping injection.
    #[test]
    fn paginated_codex_title_reads_item_completed_user_message() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("rollout.jsonl");
        let lines = [
            r#"{"timestamp":"t","ordinal":0,"type":"session_meta","payload":{"id":"sid","history_mode":"paginated"}}"#,
            r#"{"timestamp":"t","ordinal":1,"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"<recommended_plugins>skip me</recommended_plugins>"}]}}"#,
            r#"{"timestamp":"t","ordinal":2,"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"ASKHUMAN_PAGINATED_PROBE_20260820"}]}}"#,
            r#"{"timestamp":"t","ordinal":3,"type":"event_msg","payload":{"type":"item_completed","item":{"type":"UserMessage","id":"um-1","content":[{"type":"text","text":"ASKHUMAN_PAGINATED_PROBE_20260820","text_elements":[]}]}}}"#,
        ];
        std::fs::write(&f, lines.join("\n")).unwrap();
        assert_eq!(
            codex_user_message(&f).as_deref(),
            Some("ASKHUMAN_PAGINATED_PROBE_20260820")
        );
        assert_eq!(
            first_user_message(&f).as_deref(),
            Some("ASKHUMAN_PAGINATED_PROBE_20260820")
        );
    }

    #[test]
    fn paginated_codex_148_real_session_title_when_present() {
        let sid = "01a01ec8-3867-7730-8acf-8911ef19b587";
        let Some(path) = transcript_path(AgentKind::Codex, sid) else {
            eprintln!("skip: paginated probe session not on disk");
            return;
        };
        let title = resolve_title(AgentKind::Codex, sid);
        eprintln!("real paginated title={title:?} path={path:?}");
        assert!(
            title
                .as_deref()
                .is_some_and(|text| text.contains("ASKHUMAN_PAGINATED_PROBE_20260820")),
            "expected fallback title from response_item, got {title:?}"
        );
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
        assert_eq!(claude_session_name(&f).as_deref(), Some("second"));
    }

    #[test]
    fn pi_title_prefers_latest_session_name_from_custom_path() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("pi.jsonl");
        let lines = [
            serde_json::json!({"type":"session","version":3,"id":"pi-title-test","cwd":dir.path()}),
            serde_json::json!({"type":"message","message":{"role":"user","content":[{"type":"text","text":"first prompt"}]}}),
            serde_json::json!({"type":"session_info","name":"Named Pi session"}),
        ];
        std::fs::write(
            &path,
            lines
                .iter()
                .map(Value::to_string)
                .collect::<Vec<_>>()
                .join("\n"),
        )
        .unwrap();
        super::super::session_paths::register(
            AgentKind::Pi,
            "pi-title-test",
            &path.to_string_lossy(),
            Some(&dir.path().to_string_lossy()),
        )
        .unwrap();
        assert_eq!(
            pi_title("pi-title-test").as_deref(),
            Some("Named Pi session")
        );
        let mut file = fs::OpenOptions::new().append(true).open(&path).unwrap();
        use std::io::Write;
        for _ in 0..MAX_LINES + 10 {
            writeln!(file, "\n{}", serde_json::json!({"type":"message"})).unwrap();
        }
        let renamed = "Full renamed Pi session ".repeat(10).trim().to_string();
        writeln!(
            file,
            "{}",
            serde_json::json!({"type":"session_info","name":renamed})
        )
        .unwrap();
        assert_eq!(
            resolve_popup_title(
                AgentKind::Pi,
                "pi-title-test",
                Some("Nonempty stale cache".into()),
                &Default::default()
            ),
            Some(renamed)
        );
        writeln!(
            file,
            "{}",
            serde_json::json!({"type":"session_info","name":""})
        )
        .unwrap();
        assert_eq!(
            resolve_popup_title(
                AgentKind::Pi,
                "pi-title-test",
                Some("Stale manual name".into()),
                &Default::default()
            )
            .as_deref(),
            Some("first prompt")
        );
    }

    #[test]
    fn list_titles_are_clipped_like_resolved_titles_while_popup_titles_are_not() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("pi-long.jsonl");
        let long = "Long Pi session name ".repeat(10).trim().to_string();
        let lines = [
            serde_json::json!({"type":"session","version":3,"id":"pi-long-title-test","cwd":dir.path()}),
            serde_json::json!({"type":"session_info","name":long}),
        ];
        std::fs::write(
            &path,
            lines
                .iter()
                .map(Value::to_string)
                .collect::<Vec<_>>()
                .join("\n"),
        )
        .unwrap();
        super::super::session_paths::register(
            AgentKind::Pi,
            "pi-long-title-test",
            &path.to_string_lossy(),
            Some(&dir.path().to_string_lossy()),
        )
        .unwrap();
        let config = Default::default();
        assert_eq!(
            resolve_popup_title(AgentKind::Pi, "pi-long-title-test", None, &config).as_deref(),
            Some(long.as_str())
        );
        let listed =
            resolve_list_title(AgentKind::Pi, "pi-long-title-test", None, &config).unwrap();
        assert_eq!(listed, clean_title(&long));
        assert!(listed.chars().count() <= MAX_TITLE_CHARS + 1);
        assert!(listed.ends_with('…'));
        // A title the registry already holds stays when no native name exists, tidied up.
        assert_eq!(
            resolve_list_title(
                AgentKind::Antigravity,
                "no-native-name",
                Some("  Held   name ".into()),
                &config
            )
            .as_deref(),
            Some("Held name")
        );
    }
}
