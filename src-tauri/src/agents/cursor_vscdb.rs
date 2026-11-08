//! Cursor IDE（Agent Window）会话的实时数据源（spec gui-agent-console 反馈记录 2026-07-25）。
//!
//! IDE 形态的会话对话**实时**持久化在 Cursor 全局 SQLite（`state.vscdb`，WAL）的
//! `cursorDiskKV` 表：`composerData:<sid>`（官方标题 + 有序消息头 + lastUpdatedAt）与
//! `bubbleId:<sid>:<id>`（逐条消息：助手文字 / `toolFormerData` 工具调用状态 / 用户输入）。
//! 相比 agent-transcripts jsonl（IDE 长回合内冻结数小时、AskHuman 协议下回合永不结束），
//! 这是唯一实时的内容来源；CLI 会话不写此库（其 jsonl 本就逐工具落盘），查不到 key 时
//! 调用方回退既有 jsonl 路径，天然区分两种形态。
//!
//! 访问约定：**只读**打开（WAL 下不阻塞 IDE 写入）+ busy timeout，全程 best-effort——
//! 库缺失 / schema 变化 / 解析失败一律返回 None/Err 触发回退，绝不影响既有行为。
//! 活动帧走**增量缓存**：每次只取新增消息头对应的 bubble（点查），维持滚动事件窗 +
//! TODO 台账，符合 watch tick 的热路径成本要求（C16 精神）。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use rusqlite::{Connection, OpenFlags};
use serde_json::Value;

use super::activity::{self, Activity, Ev, TodoItem};
use super::transcript_full::{self, AskHumanBlock, TranscriptDoc, TranscriptEvent};

/// 活动帧滚动事件窗上限（Text/Tool/ToolResult；TODO 走独立台账不受限）。
const MAX_RECENT_EVS: usize = 60;
/// 首次同步一个会话时，最多回看的历史 bubble 数（含 TODO 台账重建；再早的忽略）。
const MAX_BACKFILL: usize = 400;
/// 尾部「活动区」条数：bubble 行是**原地流式更新**的（文字逐段补全、status loading→completed），
/// 这段每拍整体重读、不入定稿缓存；早于此边界的 bubble 视为定稿、增量消化一次。
const TAIL_LIVE: usize = 20;
/// 完整会话解析的事件上限（与 transcript_full::MAX_EVENTS 对齐）。
const MAX_TX_EVENTS: usize = 2000;

/// 定位 Cursor 全局 state.vscdb（存在才返回）。
fn db_path() -> Option<PathBuf> {
    db_path_from(dirs::config_dir().as_deref(), dirs::home_dir().as_deref())
}

fn db_path_from(config_dir: Option<&Path>, home: Option<&Path>) -> Option<PathBuf> {
    db_candidates(config_dir, home)
        .into_iter()
        .find(|path| path.is_file())
}

fn db_candidates(config_dir: Option<&Path>, home: Option<&Path>) -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(config_dir) = config_dir {
        candidates.push(
            config_dir
                .join("Cursor")
                .join("User")
                .join("globalStorage")
                .join("state.vscdb"),
        );
    }
    // Retain explicit legacy fallbacks for non-standard environment providers.
    if let Some(home) = home {
        candidates
            .push(home.join("Library/Application Support/Cursor/User/globalStorage/state.vscdb"));
        candidates.push(home.join(".config/Cursor/User/globalStorage/state.vscdb"));
    }
    candidates.dedup();
    candidates
}

/// 只读连接（WAL 并发读安全；短 busy timeout 防 IDE checkpoint 竞争时卡拍）。
fn open() -> Option<Connection> {
    let conn = Connection::open_with_flags(
        db_path()?,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .ok()?;
    let _ = conn.busy_timeout(std::time::Duration::from_millis(200));
    Some(conn)
}

fn kv_get(conn: &Connection, key: &str) -> Option<String> {
    conn.query_row(
        "SELECT value FROM cursorDiskKV WHERE key = ?1",
        [key],
        |row| row.get::<_, String>(0),
    )
    .ok()
}

/// One `ItemTable` value (the IDE's own key/value store), read-only. `None` when the database or
/// the key is missing, the value is not UTF-8 text, or it is longer than `max_bytes`.
pub(crate) fn item_value(key: &str, max_bytes: usize) -> Option<String> {
    let conn = open()?;
    item_value_in(&conn, key, max_bytes)
}

fn item_value_in(conn: &Connection, key: &str, max_bytes: usize) -> Option<String> {
    use rusqlite::types::ValueRef;
    conn.query_row(
        "SELECT value FROM ItemTable WHERE key = ?1 AND length(CAST(value AS BLOB)) <= ?2",
        rusqlite::params![key, i64::try_from(max_bytes).ok()?],
        |row| {
            Ok(match row.get_ref(0)? {
                ValueRef::Text(bytes) | ValueRef::Blob(bytes) => {
                    String::from_utf8(bytes.to_vec()).ok()
                }
                _ => None,
            })
        },
    )
    .ok()
    .flatten()
}

/// Cursor's own project index preserves exact filesystem paths, unlike the lossy directory slug.
/// Prefer it when available so drive letters, UNC shares, spaces, and non-ASCII names need no
/// reconstruction. Returned timestamps are Unix seconds.
pub(crate) fn recent_workspaces() -> Vec<(PathBuf, u64)> {
    let Some(conn) = open() else {
        return Vec::new();
    };
    let Ok(raw) = conn.query_row(
        "SELECT value FROM ItemTable WHERE key = 'glass.localAgentProjects.v1'",
        [],
        |row| row.get::<_, String>(0),
    ) else {
        return Vec::new();
    };
    parse_recent_workspaces(&raw)
}

fn parse_recent_workspaces(raw: &str) -> Vec<(PathBuf, u64)> {
    let Some(items) = serde_json::from_str::<Value>(raw)
        .ok()
        .and_then(|value| value.as_array().cloned())
    else {
        return Vec::new();
    };
    let mut by_path = HashMap::<PathBuf, u64>::new();
    for item in items {
        let Some(uri) = item.pointer("/workspace/uri") else {
            continue;
        };
        if uri
            .get("scheme")
            .and_then(Value::as_str)
            .is_some_and(|scheme| scheme != "file")
        {
            continue;
        }
        let Some(path) = uri
            .get("fsPath")
            .or_else(|| uri.get("path"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|path| !path.is_empty())
            .map(PathBuf::from)
        else {
            continue;
        };
        let updated = item
            .get("lastUpdatedAt")
            .and_then(Value::as_u64)
            .unwrap_or(0)
            / 1000;
        by_path
            .entry(path)
            .and_modify(|current| *current = (*current).max(updated))
            .or_insert(updated);
    }
    let mut paths: Vec<_> = by_path.into_iter().collect();
    paths.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    paths
}

fn composer_value(conn: &Connection, session_id: &str) -> Option<Value> {
    let raw = kv_get(conn, &format!("composerData:{session_id}"))?;
    serde_json::from_str(&raw).ok()
}

fn bubble_value(conn: &Connection, session_id: &str, bubble_id: &str) -> Option<Value> {
    let raw = kv_get(conn, &format!("bubbleId:{session_id}:{bubble_id}"))?;
    serde_json::from_str(&raw).ok()
}

/// A bounded suffix of the authoritative conversation directory.
struct HeaderWindow {
    ids: Vec<String>,
    start: usize,
    total: usize,
    generation: String,
}

fn header_window(
    conn: &Connection,
    session_id: &str,
    composer: &Value,
    limit: usize,
) -> Option<HeaderWindow> {
    let (headers, start, total, generation) =
        if let Some(log) = composer.get("headerLog").filter(|v| !v.is_null()) {
            let size = usize::try_from(log.get("chunkSize")?.as_u64()?).ok()?;
            let chunks = usize::try_from(log.get("chunkCount")?.as_u64()?).ok()?;
            let count = usize::try_from(log.get("count")?.as_u64()?).ok()?;
            if size == 0 || size > 4096 || chunks > 65536 || count > size.checked_mul(chunks)? {
                return None;
            }
            let first = chunks.saturating_sub(limit.div_ceil(size) + 1);
            let mut headers = Vec::new();
            for index in first..chunks {
                let value: Value = serde_json::from_str(&kv_get(
                    conn,
                    &format!("composerConvHeaders:{session_id}:{index:06}"),
                )?)
                .ok()?;
                let chunk = value.as_array()?;
                if chunk.len() > size {
                    return None;
                }
                headers.extend(chunk.iter().cloned());
            }
            if headers.len() > count || (first == 0 && headers.len() != count) {
                return None;
            }
            let start = count - headers.len();
            (
                headers,
                start,
                count,
                log["generation"].as_str().unwrap_or("").to_string(),
            )
        } else {
            let headers = composer.get("fullConversationHeadersOnly")?.as_array()?;
            let total = headers.len();
            let start = total.saturating_sub(limit);
            (headers[start..].to_vec(), start, total, String::new())
        };
    let skip = headers.len().saturating_sub(limit);
    let ids = headers[skip..]
        .iter()
        .map(|h| h.get("bubbleId")?.as_str().map(str::to_string))
        .collect::<Option<Vec<_>>>()?;
    Some(HeaderWindow {
        ids,
        start: start + skip,
        total,
        generation,
    })
}

/// 官方会话标题（IDE 恢复列表同款）。
pub fn resolve_title(session_id: &str) -> Option<String> {
    let conn = open()?;
    let composer = composer_value(&conn, session_id)?;
    composer
        .get("name")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
}

/// Native model/context fields for one exact IDE composer. CLI sessions return None.
pub(crate) fn session_metadata(session_id: &str) -> Option<Value> {
    let conn = open()?;
    let raw: String = conn
        .query_row(
            "SELECT value FROM cursorDiskKV WHERE key=?1 AND length(value)<=1048576",
            [format!("composerData:{session_id}")],
            |row| row.get(0),
        )
        .ok()?;
    serde_json::from_str(&raw).ok()
}

/// Match a composer to Cursor's exact workspace index, never its most recently opened folder.
pub(crate) fn session_workspace(session_id: &str) -> Option<String> {
    let user = db_path()?.parent()?.parent()?.to_path_buf();
    workspace_for_session(&user.join("workspaceStorage"), session_id)
}

fn workspace_for_session(storage: &Path, session_id: &str) -> Option<String> {
    let mut found = None;
    let began = std::time::Instant::now();
    for (index, entry) in std::fs::read_dir(storage)
        .ok()?
        .flatten()
        .take(257)
        .enumerate()
    {
        if index == 256 || began.elapsed() > std::time::Duration::from_millis(250) {
            return None;
        }
        let dir = entry.path();
        let Ok(conn) = Connection::open_with_flags(
            dir.join("state.vscdb"),
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        ) else {
            continue;
        };
        let _ = conn.busy_timeout(std::time::Duration::from_millis(50));
        let Ok(raw) = conn.query_row(
            "SELECT value FROM ItemTable WHERE key='composer.composerData' AND length(value)<=1048576",
            [],
            |row| row.get::<_, String>(0),
        ) else {
            continue;
        };
        let Ok(value) = serde_json::from_str::<Value>(&raw) else {
            continue;
        };
        if !value
            .get("allComposers")
            .and_then(Value::as_array)
            .is_some_and(|entries| {
                entries
                    .iter()
                    .any(|entry| entry["composerId"] == session_id)
            })
        {
            continue;
        }
        let Some(workspace) = std::fs::read(dir.join("workspace.json"))
            .ok()
            .filter(|bytes| bytes.len() < 65536)
            .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
        else {
            continue;
        };
        let Some(path) = workspace
            .get("folder")
            .and_then(Value::as_str)
            .and_then(|s| tauri::Url::parse(s).ok())
            .and_then(|url| url.to_file_path().ok())
            .filter(|path| path.is_dir())
        else {
            continue;
        };
        let path = std::fs::canonicalize(&path)
            .unwrap_or(path)
            .to_string_lossy()
            .into_owned();
        if found.as_ref().is_some_and(|previous| previous != &path) {
            return None;
        }
        found = Some(path);
    }
    found
}

/// 会话最近更新时刻（`lastUpdatedAt` 毫秒 → SystemTime）；控制台完整会话缓存的失效键。
pub fn last_updated(session_id: &str) -> Option<std::time::SystemTime> {
    let conn = open()?;
    let composer = composer_value(&conn, session_id)?;
    let ms = ["lastUpdatedAt", "conversationCheckpointLastUpdatedAt"]
        .iter()
        .filter_map(|key| composer.get(key).and_then(Value::as_u64))
        .max()?;
    Some(std::time::UNIX_EPOCH + std::time::Duration::from_millis(ms))
}

// ── bubble → 事件解码（活动帧与完整会话共用）──

/// 工具状态映射：completed → 完成；error* / fail* → 失败；其余（进行中枚举未知）→ 在跑。
fn tool_finished(status: &str) -> Option<bool> {
    let s = status.to_ascii_lowercase();
    if s == "completed" {
        Some(false)
    } else if s.starts_with("error") || s.starts_with("fail") || s == "cancelled" {
        Some(true)
    } else {
        None // 仍在跑
    }
}

/// bubble 的创建时刻（ISO 字符串 → Unix 秒）。
fn bubble_at(b: &Value) -> Option<u64> {
    b.get("createdAt")
        .and_then(|v| v.as_str())
        .and_then(transcript_full::parse_iso8601_secs)
}

/// toolFormerData 的参数：取**首个非空**的 `rawArgs` / `params` 字符串（不同版本二选一填充：
/// 实测有的会话 rawArgs 是空串、真参数在 params），JSON 解析成对象——`classify_tool` 与
/// `detect_askhuman` 都按对象取键；解析失败保留原串；也容忍参数直接是对象的形态。
fn tool_args(tf: &Value) -> Option<Value> {
    if let Some(raw) = ["rawArgs", "params"].iter().find_map(|k| {
        tf.get(*k)
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
    }) {
        return Some(
            serde_json::from_str::<Value>(raw).unwrap_or_else(|_| Value::String(raw.to_string())),
        );
    }
    ["rawArgs", "params"]
        .iter()
        .find_map(|k| tf.get(*k).filter(|v| v.is_object()).cloned())
}

/// 工具结果文本（result 是 JSON 字符串 `{output, ...}`；取 output，回退原串）。
fn tool_result_text(tf: &Value) -> Option<String> {
    let raw = tf.get("result")?;
    if let Some(s) = raw.as_str() {
        if let Ok(v) = serde_json::from_str::<Value>(s) {
            if let Some(out) = v.get("output").and_then(|o| o.as_str()) {
                return Some(out.to_string());
            }
        }
        return Some(s.to_string());
    }
    raw.get("output")
        .and_then(|o| o.as_str())
        .map(|s| s.to_string())
}

/// 单个 bubble → 活动帧事件（0..=2 条：文字 / 工具 + 结果闭合）。
fn bubble_to_evs(b: &Value, out: &mut Vec<Ev>) {
    // 助手文字（type 2；工具 bubble 的 text 通常为空）。
    if b.get("type").and_then(|v| v.as_u64()) == Some(2) {
        if let Some(t) = b.get("text").and_then(|v| v.as_str()) {
            let t = t.trim();
            if !t.is_empty() {
                out.push(Ev::Text(t.to_string()));
            }
        }
    }
    let Some(tf) = b.get("toolFormerData") else {
        return;
    };
    let name = tf.get("name").and_then(|v| v.as_str()).unwrap_or("");
    if name.is_empty() {
        return;
    }
    let args = tool_args(tf);
    if activity::is_todo_tool(name) {
        if let Some(ev) = activity::parse_todos(args.as_ref()) {
            out.push(ev);
        }
        return;
    }
    out.push(Ev::Tool(activity::classify_tool(name, args.as_ref())));
    let status = tf.get("status").and_then(|v| v.as_str()).unwrap_or("");
    if let Some(failed) = tool_finished(status) {
        out.push(Ev::ToolResult(failed));
    }
}

// ── 活动帧（增量缓存）──

struct SessCache {
    generation: String,
    backfilled_at: Instant,
    anchor: Option<String>,
    latest_text: Option<super::observation::LatestText>,
    last_activity: Option<Activity>,
    /// 已消化的消息头数量（headers 前缀）。
    seen: usize,
    /// 滚动事件窗（Text/Tool/ToolResult；上限 MAX_RECENT_EVS）。
    recent: Vec<Ev>,
    /// TODO 重放台账（不截断；带 id 供 merge 更新）。
    todos: Vec<(Option<String>, TodoItem)>,
    /// 最近 bubble 的时刻。
    last_at: Option<u64>,
}

fn cache() -> &'static Mutex<HashMap<String, SessCache>> {
    static CACHE: std::sync::OnceLock<Mutex<HashMap<String, SessCache>>> =
        std::sync::OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// 活动帧解析（watch tick 热路径）。分两区：
/// - **定稿区**（早于尾部 `TAIL_LIVE` 条）：增量消化一次进滚动窗 / TODO 台账；
/// - **活动区**（尾部 `TAIL_LIVE` 条）：每拍整体重读——bubble 行原地流式更新（文字逐段
///   补全、工具 status 翻转），读一次就定格会让新内容永远停在首见状态（用户实证）。
///
/// 聚合语义与 jsonl 完全同源。会话不在库中（CLI / 旧版本）→ None（调用方回退 jsonl）。
pub fn resolve_activity(session_id: &str, turn_ended: bool) -> Option<Activity> {
    let resolved = open().and_then(|conn| {
        let tx = conn.unchecked_transaction().ok()?;
        resolve_activity_in(&tx, session_id, turn_ended)
    });
    resolved.or_else(|| cache().lock().ok()?.get(session_id)?.last_activity.clone())
}

fn resolve_activity_in(conn: &Connection, session_id: &str, turn_ended: bool) -> Option<Activity> {
    let composer = composer_value(conn, session_id)?;
    let headers = header_window(conn, session_id, &composer, MAX_BACKFILL + TAIL_LIVE)?;
    let finalized = headers.total.saturating_sub(TAIL_LIVE).max(headers.start);
    let mut guard = cache().lock().ok()?;
    if guard.len() > 128 {
        guard.clear();
    }
    let entry = guard
        .entry(session_id.to_string())
        .or_insert_with(|| SessCache {
            generation: headers.generation.clone(),
            backfilled_at: Instant::now(),
            anchor: None,
            latest_text: None,
            last_activity: None,
            seen: headers.start,
            recent: Vec::new(),
            todos: Vec::new(),
            last_at: None,
        });
    let anchor_changed = entry
        .seen
        .checked_sub(1)
        .and_then(|index| index.checked_sub(headers.start))
        .and_then(|index| headers.ids.get(index))
        .is_some_and(|id| entry.anchor.as_ref() != Some(id));
    // Cursor changes its generation on ordinary writes as well as directory rebuilds.
    // Recheck finalized rows at most once per ten seconds; always reread the live suffix.
    if (entry.generation != headers.generation
        && entry.backfilled_at.elapsed() >= Duration::from_secs(10))
        || headers.total < entry.seen
        || entry.seen < headers.start
        || anchor_changed
    {
        *entry = SessCache {
            generation: headers.generation.clone(),
            backfilled_at: Instant::now(),
            anchor: None,
            latest_text: None,
            last_activity: None,
            seen: headers.start,
            recent: Vec::new(),
            todos: Vec::new(),
            last_at: None,
        };
    }
    // Stop at a missing row so a later tick can retry it rather than permanently skipping it.
    while entry.seen < finalized {
        let id = &headers.ids[entry.seen - headers.start];
        let Some(bubble) = bubble_value(conn, session_id, id) else {
            break;
        };
        entry.last_at = bubble_at(&bubble).or(entry.last_at);
        if let Some(text) = completed_bubble_text(&bubble, &format!("{session_id}:{id}"), true) {
            entry.latest_text = Some(text);
        }
        let mut events = Vec::new();
        bubble_to_evs(&bubble, &mut events);
        for event in events {
            match event {
                Ev::Todos { replace, items } => {
                    activity::apply_todo_update(&mut entry.todos, replace, &items)
                }
                other => entry.recent.push(other),
            }
        }
        entry.seen += 1;
        entry.anchor = Some(id.clone());
    }
    if entry.recent.len() > MAX_RECENT_EVS {
        entry.recent.drain(..entry.recent.len() - MAX_RECENT_EVS);
    }
    let mut todos = entry.todos.clone();
    let mut events = entry.recent.clone();
    let mut latest_text = entry.latest_text.clone();
    let mut at = entry.last_at;
    for (offset, id) in headers.ids[entry.seen - headers.start..].iter().enumerate() {
        let Some(bubble) = bubble_value(conn, session_id, id) else {
            continue;
        };
        at = match (at, bubble_at(&bubble)) {
            (Some(a), Some(b)) => Some(a.max(b)),
            (a, b) => a.or(b),
        };
        // A completed root must be at least as recent as this bubble. Older persisted status
        // can survive a retry and must not finalize a new streaming paragraph.
        let started = bubble["startedAtMs"]
            .as_u64()
            .or_else(|| bubble_at(&bubble).map(|at| at * 1000));
        let root_completed = composer["status"] == "completed"
            && composer["lastUpdatedAt"]
                .as_u64()
                .zip(started)
                .is_some_and(|(updated, started)| updated >= started);
        let followed = entry.seen + offset + 1 < headers.total || turn_ended || root_completed;
        if let Some(text) = completed_bubble_text(&bubble, &format!("{session_id}:{id}"), followed)
        {
            latest_text = Some(text);
        }
        let mut live = Vec::new();
        bubble_to_evs(&bubble, &mut live);
        for event in live {
            match event {
                Ev::Todos { replace, items } => {
                    activity::apply_todo_update(&mut todos, replace, &items)
                }
                other => events.push(other),
            }
        }
    }
    // Inject the replayed TODO ledger independently of the bounded event window.
    events.insert(
        0,
        Ev::Todos {
            replace: true,
            items: todos,
        },
    );
    let mut result = activity::aggregate(&events, false).unwrap_or(Activity {
        latest_text: None,
        text: None,
        steps: Vec::new(),
        steps_omitted: 0,
        todos: Vec::new(),
        at: None,
    });
    result.latest_text = latest_text;
    result.at = at;
    let has_content = result.latest_text.is_some()
        || result.text.is_some()
        || !result.steps.is_empty()
        || !result.todos.is_empty();
    if has_content {
        entry.last_activity = Some(result.clone());
    }
    has_content.then_some(result)
}

fn thinking_text(bubble: &Value) -> Option<&str> {
    let thinking = bubble.get("thinking")?;
    thinking
        .as_str()
        .or_else(|| thinking.get("text").and_then(Value::as_str))
        .map(str::trim)
        .filter(|text| !text.is_empty())
}

fn completed_bubble_text(
    bubble: &Value,
    id: &str,
    followed: bool,
) -> Option<super::observation::LatestText> {
    if bubble["type"].as_u64() != Some(2) {
        return None;
    }
    if followed || bubble["completedAtMs"].as_u64().is_some() {
        let prose = bubble["text"].as_str().and_then(|text| {
            super::observation::LatestText::new(
                id.to_string(),
                "assistant",
                text,
                bubble_at(bubble),
            )
        });
        if prose.is_some() {
            return prose;
        }
    }
    if let Some(text) = thinking_text(bubble) {
        if followed
            || bubble["thinkingDurationMs"].as_u64().is_some()
            || bubble["text"]
                .as_str()
                .is_some_and(|text| !text.trim().is_empty())
        {
            return super::observation::LatestText::new(
                id.to_string(),
                "thinking",
                text,
                bubble_at(bubble),
            );
        }
    }
    None
}

// ── 完整会话（控制台分页数据源）──

/// 完整会话事件（旧→新）。会话不在库中返回 Err（调用方回退 jsonl）。
pub fn load_events(session_id: &str) -> Result<TranscriptDoc, String> {
    let conn = open().ok_or("cursor state.vscdb not found")?;
    let tx = conn
        .unchecked_transaction()
        .map_err(|error| error.to_string())?;
    let composer = composer_value(&tx, session_id).ok_or("session not in cursor state.vscdb")?;
    let headers = header_window(&tx, session_id, &composer, MAX_TX_EVENTS)
        .ok_or("conversation directory unavailable")?;
    let truncated_head = headers.start > 0;
    let window = &headers.ids;
    let mut events: Vec<TranscriptEvent> = Vec::new();
    let mut partial = false;
    for bubble_id in window {
        let Some(b) = bubble_value(&tx, session_id, bubble_id) else {
            partial = true;
            continue;
        };
        push_tx_events(&b, &mut events);
    }
    Ok(TranscriptDoc {
        events,
        truncated_head,
        partial,
    })
}

/// 单个 bubble → 完整会话事件（与 jsonl 解析口径一致：用户/助手文字、工具行、AskHuman 块）。
fn push_tx_events(b: &Value, out: &mut Vec<TranscriptEvent>) {
    let at = bubble_at(b);
    let bubble_type = b.get("type").and_then(|v| v.as_u64());
    if let Some(text) = thinking_text(b) {
        if bubble_type == Some(2) {
            out.push(TranscriptEvent::Thinking {
                text: text.to_string(),
                at,
                at_label: None,
            });
        }
    }
    let text = b
        .get("text")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty());
    match bubble_type {
        Some(1) => {
            if let Some(t) = text {
                let (cleaned, at_label) = transcript_full::clean_user(t);
                if !cleaned.is_empty() {
                    out.push(TranscriptEvent::UserText {
                        text: cleaned,
                        at,
                        at_label,
                    });
                }
            }
        }
        Some(2) => {
            if let Some(t) = text {
                out.push(TranscriptEvent::AssistantText {
                    text: t.to_string(),
                    at,
                    at_label: None,
                });
            }
        }
        _ => {}
    }
    let Some(tf) = b.get("toolFormerData") else {
        return;
    };
    let name = tf.get("name").and_then(|v| v.as_str()).unwrap_or("");
    if name.is_empty() || activity::is_todo_tool(name) {
        return; // TODO 更新不入行为时间线（与 jsonl 同口径）。
    }
    let args = tool_args(tf);
    let status = tf.get("status").and_then(|v| v.as_str()).unwrap_or("");
    let is_error = tool_finished(status) == Some(true);
    let result = tool_result_text(tf);
    let ask_human: Option<AskHumanBlock> =
        transcript_full::detect_askhuman(name, args.as_ref(), result.as_deref());
    let td = activity::classify_tool(name, args.as_ref());
    out.push(TranscriptEvent::ToolCall {
        name: name.to_string(),
        args_summary: transcript_full::format_tool_line(&td),
        result_summary: None,
        is_error,
        ask_human,
        at,
        at_label: None,
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn memory_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute(
            "CREATE TABLE cursorDiskKV (key TEXT PRIMARY KEY, value TEXT)",
            [],
        )
        .unwrap();
        conn
    }

    fn put(conn: &Connection, key: &str, value: Value) {
        conn.execute(
            "INSERT OR REPLACE INTO cursorDiskKV VALUES (?1, ?2)",
            [key.to_string(), value.to_string()],
        )
        .unwrap();
    }

    #[test]
    fn chunked_headers_take_precedence_and_retry_missing_chunks() {
        let conn = memory_db();
        let composer = serde_json::json!({
            "fullConversationHeadersOnly": [],
            "headerLog": {"chunkSize":2,"chunkCount":2,"count":3,"generation":"g1"}
        });
        put(
            &conn,
            "composerConvHeaders:s:000000",
            serde_json::json!([{"bubbleId":"a"},{"bubbleId":"b"}]),
        );
        assert!(header_window(&conn, "s", &composer, 2).is_none());
        put(
            &conn,
            "composerConvHeaders:s:000001",
            serde_json::json!([{"bubbleId":"c"}]),
        );
        let window = header_window(&conn, "s", &composer, 2).unwrap();
        assert_eq!(window.ids, ["b", "c"]);
        assert_eq!(window.start, 1);
        assert_eq!(window.total, 3);
        assert_eq!(window.generation, "g1");
        let legacy = serde_json::json!({"fullConversationHeadersOnly":[{"bubbleId":"old"}]});
        assert_eq!(header_window(&conn, "s", &legacy, 20).unwrap().ids, ["old"]);
    }

    #[test]
    fn thoughts_wait_for_completion_and_then_replace_old_prose() {
        let conn = memory_db();
        let sid = uuid::Uuid::new_v4().to_string();
        let composer_key = format!("composerData:{sid}");
        let thought_key = format!("bubbleId:{sid}:thought");
        put(
            &conn,
            &composer_key,
            serde_json::json!({"status":"generating", "fullConversationHeadersOnly":[{"bubbleId":"body"},{"bubbleId":"thought"}]}),
        );
        put(
            &conn,
            &format!("bubbleId:{sid}:body"),
            serde_json::json!({"type":2,"text":"Old prose"}),
        );
        put(
            &conn,
            &thought_key,
            serde_json::json!({"type":2,"thinking":"Growing thought"}),
        );
        assert_eq!(
            resolve_activity_in(&conn, &sid, false)
                .unwrap()
                .latest_text
                .unwrap()
                .text,
            "Old prose"
        );
        put(
            &conn,
            &thought_key,
            serde_json::json!({"type":2,"thinking":{"text":"Completed thought","signature":"private"},"thinkingDurationMs":0}),
        );
        let activity = resolve_activity_in(&conn, &sid, false).unwrap();
        assert_eq!(activity.text.as_deref(), Some("Old prose"));
        assert_eq!(activity.latest_text.unwrap().text, "Completed thought");
        put(
            &conn,
            &thought_key,
            serde_json::json!({"type":2,"thinking":"Followed by a tool"}),
        );
        put(
            &conn,
            &composer_key,
            serde_json::json!({"fullConversationHeadersOnly":[{"bubbleId":"body"},{"bubbleId":"thought"},{"bubbleId":"tool"}]}),
        );
        put(
            &conn,
            &format!("bubbleId:{sid}:tool"),
            serde_json::json!({"type":2,"toolFormerData":{"name":"read_file_v2","status":"loading","params":"{\"path\":\"camera.swift\"}"}}),
        );
        let activity = resolve_activity_in(&conn, &sid, false).unwrap();
        assert_eq!(activity.latest_text.unwrap().text, "Followed by a tool");
        assert_eq!(
            activity.steps.last().unwrap().state,
            activity::StepState::Running
        );
    }

    #[test]
    fn generation_replacement_discards_finalized_cached_thoughts() {
        let conn = memory_db();
        let sid = uuid::Uuid::new_v4().to_string();
        let headers: Vec<_> = (0..25)
            .map(|i| serde_json::json!({"bubbleId":i.to_string()}))
            .collect();
        put(
            &conn,
            &format!("composerConvHeaders:{sid}:000000"),
            serde_json::json!(headers),
        );
        let composer = |generation| serde_json::json!({"headerLog":{"chunkSize":256,"chunkCount":1,"count":25,"generation":generation}});
        put(&conn, &format!("composerData:{sid}"), composer("before"));
        for i in 0..25 {
            put(
                &conn,
                &format!("bubbleId:{sid}:{i}"),
                serde_json::json!({"type":2}),
            );
        }
        put(
            &conn,
            &format!("bubbleId:{sid}:0"),
            serde_json::json!({"type":2,"thinking":"Old thought"}),
        );
        assert_eq!(
            resolve_activity_in(&conn, &sid, false)
                .unwrap()
                .latest_text
                .unwrap()
                .text,
            "Old thought"
        );
        cache().lock().unwrap().get_mut(&sid).unwrap().backfilled_at =
            Instant::now() - Duration::from_secs(10);
        put(&conn, &format!("composerData:{sid}"), composer("after"));
        put(
            &conn,
            &format!("bubbleId:{sid}:0"),
            serde_json::json!({"type":2,"thinking":"Rebuilt thought"}),
        );
        assert_eq!(
            resolve_activity_in(&conn, &sid, false)
                .unwrap()
                .latest_text
                .unwrap()
                .text,
            "Rebuilt thought"
        );
    }

    #[test]
    fn workspace_identity_requires_an_exact_unique_composer_match() {
        let root = tempfile::tempdir().unwrap();
        let storage = root.path().join("storage");
        let a = root.path().join("project a");
        let b = root.path().join("project b");
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(&b).unwrap();
        let add = |key: &str, sid: &str, project: &Path| {
            let dir = storage.join(key);
            std::fs::create_dir_all(&dir).unwrap();
            let conn = Connection::open(dir.join("state.vscdb")).unwrap();
            conn.execute("CREATE TABLE ItemTable (key TEXT, value TEXT)", [])
                .unwrap();
            conn.execute(
                "INSERT INTO ItemTable VALUES ('composer.composerData', ?1)",
                [serde_json::json!({"allComposers":[{"composerId":sid}]}).to_string()],
            )
            .unwrap();
            std::fs::write(
                dir.join("workspace.json"),
                serde_json::json!({
                    "folder": tauri::Url::from_directory_path(project).unwrap().as_str()
                })
                .to_string(),
            )
            .unwrap();
        };
        add("one", "exact", &a);
        add("two", "another", &b);
        assert_eq!(
            workspace_for_session(&storage, "exact"),
            Some(
                std::fs::canonicalize(&a)
                    .unwrap()
                    .to_string_lossy()
                    .into_owned()
            )
        );
        assert!(workspace_for_session(&storage, "unknown").is_none());
        add("ambiguous", "exact", &b);
        assert!(workspace_for_session(&storage, "exact").is_none());
    }

    use crate::agents::activity::{StepState, ToolLabel};

    #[test]
    fn cursor_db_prefers_injected_platform_config_directory() {
        let temp = tempfile::tempdir().unwrap();
        let config = temp.path().join("Roaming AppData");
        let expected = config
            .join("Cursor")
            .join("User")
            .join("globalStorage")
            .join("state.vscdb");
        std::fs::create_dir_all(expected.parent().unwrap()).unwrap();
        std::fs::write(&expected, b"fixture").unwrap();
        assert_eq!(db_path_from(Some(&config), None), Some(expected));
    }

    #[test]
    fn cursor_project_index_preserves_windows_paths_and_latest_timestamp() {
        let raw = serde_json::json!([
            {
                "workspace": {"uri": {"scheme": "file", "fsPath": "C:\\工作\\Repo Space"}},
                "lastUpdatedAt": 1_700_000_000_000u64
            },
            {
                "workspace": {"uri": {"scheme": "file", "fsPath": "\\\\server\\share\\项目"}},
                "lastUpdatedAt": 1_800_000_000_000u64
            },
            {
                "workspace": {"uri": {"scheme": "file", "fsPath": "C:\\工作\\Repo Space"}},
                "lastUpdatedAt": 1_900_000_000_000u64
            },
            {
                "workspace": {"uri": {"scheme": "vscode-remote", "fsPath": "C:\\ignored"}},
                "lastUpdatedAt": 2_000_000_000_000u64
            }
        ])
        .to_string();
        let paths = parse_recent_workspaces(&raw);
        assert_eq!(paths.len(), 2);
        assert_eq!(paths[0].0, PathBuf::from(r"C:\工作\Repo Space"));
        assert_eq!(paths[0].1, 1_900_000_000);
        assert_eq!(paths[1].0, PathBuf::from(r"\\server\share\项目"));
        assert_eq!(paths[1].1, 1_800_000_000);
    }

    #[test]
    fn malformed_cursor_project_index_fails_closed() {
        assert!(parse_recent_workspaces("not-json").is_empty());
        assert!(parse_recent_workspaces("{}").is_empty());
    }

    fn tool_bubble(name: &str, status: &str, raw_args: &str, text: &str) -> Value {
        serde_json::json!({
            "type": 2,
            "text": text,
            "createdAt": "2026-07-25T02:38:49.620Z",
            "toolFormerData": {
                "name": name,
                "status": status,
                "rawArgs": raw_args,
            },
        })
    }

    /// bubble 解码：文字 / 工具（真实状态直传）/ TODO 分流。
    #[test]
    fn bubble_decoding_maps_text_tools_and_todos() {
        let mut evs = Vec::new();
        bubble_to_evs(
            &serde_json::json!({"type": 2, "text": "我先看一下实现。", "createdAt": "2026-07-25T02:38:23.758Z"}),
            &mut evs,
        );
        assert!(matches!(&evs[0], Ev::Text(t) if t == "我先看一下实现。"));

        let mut evs = Vec::new();
        bubble_to_evs(
            &tool_bubble(
                "run_terminal_command_v2",
                "completed",
                r#"{"command":"cargo test"}"#,
                "",
            ),
            &mut evs,
        );
        assert_eq!(evs.len(), 2); // Tool + ToolResult(ok)
        assert!(matches!(&evs[1], Ev::ToolResult(false)));

        let mut evs = Vec::new();
        bubble_to_evs(
            &tool_bubble("edit_file_v2", "error", r#"{"target_file":"a.rs"}"#, ""),
            &mut evs,
        );
        assert!(matches!(&evs[1], Ev::ToolResult(true)));

        // 进行中（未知/loading 状态）：只有 Tool，无结果闭合。
        let mut evs = Vec::new();
        bubble_to_evs(
            &tool_bubble("read_file_v2", "loading", r#"{"path":"b.rs"}"#, ""),
            &mut evs,
        );
        assert_eq!(evs.len(), 1);

        // todo_write 分流为 Todos 事件（不入时间线）。
        let mut evs = Vec::new();
        bubble_to_evs(
            &tool_bubble(
                "todo_write",
                "completed",
                r#"{"todos":[{"id":"a","content":"步骤一","status":"TODO_STATUS_IN_PROGRESS"}]}"#,
                "",
            ),
            &mut evs,
        );
        assert!(matches!(&evs[0], Ev::Todos { .. }));
        assert_eq!(evs.len(), 1);
    }

    /// 聚合（settle_all=false）：vscdb 真实状态保留「进行中」末步。
    #[test]
    fn aggregate_keeps_running_state_from_vscdb() {
        let mut evs = Vec::new();
        bubble_to_evs(
            &serde_json::json!({"type": 2, "text": "开始跑测试。"}),
            &mut evs,
        );
        bubble_to_evs(
            &tool_bubble(
                "run_terminal_command_v2",
                "loading",
                r#"{"command":"cargo test"}"#,
                "",
            ),
            &mut evs,
        );
        let a = activity::aggregate(&evs, false).unwrap();
        assert_eq!(a.text.as_deref(), Some("开始跑测试。"));
        assert_eq!(a.steps.len(), 1);
        assert_eq!(a.steps[0].state, StepState::Running);
        assert!(matches!(a.steps[0].tool.label, ToolLabel::Run));
    }

    /// 完整会话事件：AskHuman 调用（result.output 携带答案标记）解析为问答块。
    #[test]
    fn tx_events_detect_askhuman_from_result_output() {
        let b = serde_json::json!({
            "type": 2,
            "text": "",
            "createdAt": "2026-07-25T02:38:49.620Z",
            "toolFormerData": {
                "name": "run_terminal_command_v2",
                "status": "completed",
                "rawArgs": "{\"command\":\"AskHuman -q \\\"继续吗？\\\" -o \\\"好\\\"\"}",
                "result": "{\"output\":\"[selected_options]\\n好\\n\",\"rejected\":false}",
            },
        });
        let mut out = Vec::new();
        push_tx_events(&b, &mut out);
        assert_eq!(out.len(), 1);
        let TranscriptEvent::ToolCall { ask_human, .. } = &out[0] else {
            panic!("expected tool call");
        };
        let ah = ask_human.as_ref().expect("askhuman detected");
        assert_eq!(ah.questions.len(), 1);
        assert_eq!(ah.questions[0].text, "继续吗？");
        assert_eq!(ah.questions[0].answer.as_deref(), Some("好"));
    }

    /// 用户 bubble（type 1）剥 Cursor 包装后入用户事件。
    #[test]
    fn tx_events_unwrap_user_bubbles() {
        let b = serde_json::json!({
            "type": 1,
            "text": "<timestamp>Friday</timestamp>\n<user_query>\n修一个 bug\n</user_query>",
        });
        let mut out = Vec::new();
        push_tx_events(&b, &mut out);
        assert!(matches!(&out[0], TranscriptEvent::UserText { text, .. } if text == "修一个 bug"));
    }

    /// 参数取非空源：rawArgs 空串时回退 params；edit_file_v2 的 relativeWorkspacePath
    /// 提取为文件名对象（用户实证：某些会话 rawArgs 恒空、真参数在 params）。
    #[test]
    fn tool_args_fall_back_to_params_and_extract_workspace_path() {
        let b = serde_json::json!({
            "type": 2,
            "text": "",
            "toolFormerData": {
                "name": "edit_file_v2",
                "status": "loading",
                "rawArgs": "",
                "params": "{\"relativeWorkspacePath\":\"/x/src/cli/debug_cmd.rs\",\"noCodeblock\":true}",
            },
        });
        let mut evs = Vec::new();
        bubble_to_evs(&b, &mut evs);
        assert_eq!(evs.len(), 1); // loading：无结果闭合
        let Ev::Tool(td) = &evs[0] else {
            panic!("expected tool ev");
        };
        assert!(matches!(
            td.label,
            crate::agents::activity::ToolLabel::Write
        ));
        assert_eq!(td.object.as_deref(), Some("debug_cmd.rs"));
    }

    /// 真机守护冒烟（与 transcript_full 的 real_* 测试同模式）：本机存在 Cursor 全局库时，
    /// 任取一个会话跑通标题 / 活动帧 / 完整会话解析（无库则静默通过）。
    #[test]
    fn real_vscdb_roundtrip_when_available() {
        let Some(conn) = open() else { return };
        let sid: Option<String> = conn
            .query_row(
                "SELECT key FROM cursorDiskKV WHERE key LIKE 'composerData:%' \
                 ORDER BY LENGTH(value) DESC LIMIT 1",
                [],
                |row| row.get::<_, String>(0),
            )
            .ok()
            .and_then(|k| k.strip_prefix("composerData:").map(|s| s.to_string()));
        let Some(sid) = sid else { return };
        // 标题与 lastUpdatedAt 至少一个可得（空标题的新会话可容忍）。
        let _ = resolve_title(&sid);
        assert!(last_updated(&sid).is_some(), "lastUpdatedAt 应可读");
        // 活动帧与完整会话解析不 panic 且完整会话非空。
        let _ = resolve_activity(&sid, false);
        let doc = load_events(&sid).expect("load_events 应成功");
        assert!(!doc.events.is_empty(), "最大会话应至少解析出一条事件");
    }
}
