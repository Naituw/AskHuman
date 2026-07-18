//! Project-scoped todo queue (spec `docs/specs/todo-whats-next.md` D1).
//!
//! `~/.askhuman/state/todos.json` is the single source of truth: every process reads and
//! writes the file directly (no daemon-resident state — todos have no hot path). Mutations
//! take an exclusive advisory lock (`todos.lock`, same pattern as the history write lock;
//! best-effort no-op off Unix) around the read-modify-write, and the file itself is written
//! atomically (tmp + rename). Empty project keys are pruned on write.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// One pending todo entry. FIFO order is the `Vec` order in the file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TodoEntry {
    pub id: String,
    pub text: String,
    #[serde(default)]
    pub created_at_ms: u64,
    /// Agent family that invoked CLI `todo add`; absent for human/GUI/IM additions and old data.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_kind: Option<String>,
    /// 自动执行（第 17 轮定案）：whats-next 时不提问、直接把最靠前的自动待办作为下一个任务返回。
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub auto: bool,
}

/// 一条已执行的历史待办（第 16 轮定案：仅「执行出队」进历史，手动删除/清空不记）。
/// 文件内按完成时间正序追加，展示端倒序。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DoneTodoEntry {
    pub id: String,
    pub text: String,
    #[serde(default)]
    pub created_at_ms: u64,
    /// Preserved from the pending entry so execution history retains its origin.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_kind: Option<String>,
    #[serde(default)]
    pub done_at_ms: u64,
}

/// 选项类展示点（whats-next 卡 / Stop 卡 / IM `/todo-rm` 删除卡）最多列出的待办条数
/// （第 14 轮定案：前 10 条 + 溢出提示；TG 键盘 ≤100、Slack actions ≤25 等渠道硬限制的安全值）。
/// 顺序靠 GUI 待办窗口拖拽调整，头部优先展示。
pub const MAX_OPTION_TODOS: usize = 10;

/// 溢出提示（第 14 轮定案）：附在问题/卡片正文尾部；`total` ≤ 上限时 None。
pub fn overflow_note(total: usize, lang: crate::i18n::Lang) -> Option<String> {
    (total > MAX_OPTION_TODOS).then(|| {
        crate::i18n::tr(lang, "todo.moreNote")
            .replace("{n}", &(total - MAX_OPTION_TODOS).to_string())
    })
}

/// On-disk shape: project key (git root path) → FIFO entries, plus per-project execution
/// history (round 16; capped by the `todo_history_limit` setting at record time).
#[derive(Default, Serialize, Deserialize)]
struct TodoFile {
    #[serde(default)]
    projects: HashMap<String, Vec<TodoEntry>>,
    #[serde(default)]
    history: HashMap<String, Vec<DoneTodoEntry>>,
}

fn todos_file() -> PathBuf {
    crate::paths::state_dir().join("todos.json")
}

fn todos_lock() -> PathBuf {
    crate::paths::state_dir().join("todos.lock")
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn load_at(path: &Path) -> TodoFile {
    let Ok(text) = std::fs::read_to_string(path) else {
        return TodoFile::default();
    };
    serde_json::from_str(&text).unwrap_or_default()
}

/// Atomic write; prunes projects with no entries.
/// Returns `true` only when the file was fully replaced (tmp write + rename both ok).
/// Callers that must not report a false success (CLI / MCP `todo_add`) should surface
/// `false` as a hard error; other mutators keep best-effort behavior.
fn store_at(path: &Path, mut data: TodoFile) -> bool {
    data.projects.retain(|_, entries| !entries.is_empty());
    data.history.retain(|_, entries| !entries.is_empty());
    let Ok(json) = serde_json::to_string_pretty(&data) else {
        return false;
    };
    if let Some(dir) = path.parent() {
        if std::fs::create_dir_all(dir).is_err() {
            return false;
        }
    }
    let tmp = path.with_extension(format!("json.tmp-{}", uuid::Uuid::new_v4()));
    if std::fs::write(&tmp, json.as_bytes()).is_err() {
        return false;
    }
    if std::fs::rename(&tmp, path).is_ok() {
        true
    } else {
        let _ = std::fs::remove_file(&tmp);
        false
    }
}

// ===== Cross-process write lock (same pattern as history.rs) =====

#[cfg(unix)]
struct LockGuard {
    _file: std::fs::File,
}

#[cfg(unix)]
fn lock_at(path: &Path) -> Option<LockGuard> {
    use std::os::unix::io::AsRawFd;
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let file = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(false)
        .open(path)
        .ok()?;
    unsafe {
        libc::flock(file.as_raw_fd(), libc::LOCK_EX);
    }
    Some(LockGuard { _file: file })
}

#[cfg(not(unix))]
fn lock_at(_path: &Path) -> Option<()> {
    None
}

/// Normalize a project key/text pair for storage; `None` when unusable.
fn normalized(project: &str, text: &str) -> Option<(String, String)> {
    let project = project.trim();
    let text = text.trim();
    (!project.is_empty() && !text.is_empty()).then(|| (project.to_string(), text.to_string()))
}

// ===== Public API (default paths) =====

/// Pending todos of a project (FIFO). Missing file / unknown project → empty.
pub fn list(project: &str) -> Vec<TodoEntry> {
    list_at(&todos_file(), project)
}

/// Full snapshot: project key → entries (GUI window / project selector).
pub fn all() -> HashMap<String, Vec<TodoEntry>> {
    load_at(&todos_file()).projects
}

/// Why [`add`] / [`add_auto`] / [`add_from_agent`] failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AddError {
    /// Project key or text empty after trim.
    EmptyInput,
    /// Could not persist `todos.json` (permissions, sandbox, disk full, …).
    Persist,
}

/// Append one entry. Errors when input is empty or the write did not land on disk.
pub fn add(project: &str, text: &str) -> Result<TodoEntry, AddError> {
    add_at(&todos_file(), &todos_lock(), project, text)
}

/// Append one auto-run entry (round 17; CLI `todo add --auto` / GUI toggle / IM `/todo-auto`).
pub fn add_auto(project: &str, text: &str) -> Result<TodoEntry, AddError> {
    add_auto_at(&todos_file(), &todos_lock(), project, text)
}

/// Append an entry created by a recognized Agent CLI invocation.
pub fn add_from_agent(
    project: &str,
    text: &str,
    auto: bool,
    agent: crate::agents::AgentKind,
) -> Result<TodoEntry, AddError> {
    add_impl(
        &todos_file(),
        &todos_lock(),
        project,
        text,
        auto,
        Some(agent.as_str()),
    )
}

/// 1-based list index of `id` in the project's pending queue, if present.
pub fn index_of(project: &str, id: &str) -> Option<usize> {
    list(project).iter().position(|e| e.id == id).map(|i| i + 1)
}

/// Toggle the auto-run flag of one entry (round 17). Returns the new flag, `None` if missing.
pub fn set_auto(project: &str, id: &str, auto: bool) -> Option<bool> {
    set_auto_at(&todos_file(), &todos_lock(), project, id, auto)
}

/// Front-most auto-run entry of a project (whats-next auto-dispatch, round 17).
pub fn first_auto(project: &str) -> Option<TodoEntry> {
    list(project).into_iter().find(|e| e.auto)
}

/// Remove one entry by id. Returns whether it existed.
pub fn remove(project: &str, id: &str) -> bool {
    remove_at(&todos_file(), &todos_lock(), project, id)
}

/// Clear a project's queue; returns how many entries were removed.
pub fn clear(project: &str) -> usize {
    clear_at(&todos_file(), &todos_lock(), project)
}

/// Dequeue entries by id (best-effort: missing ids are skipped, spec D11). Returns the
/// entries actually removed. This is the "started executing → auto-clear" point, and the only
/// path that records execution history (round 16; capped by the `todo_history_limit` setting,
/// `0` = stop recording, existing history kept — same semantics as the reply history limit).
pub fn take(project: &str, ids: &[String]) -> Vec<TodoEntry> {
    let limit = crate::config::AppConfig::load_without_secrets()
        .general
        .todo_history_limit as usize;
    take_at(&todos_file(), &todos_lock(), project, ids, limit)
}

/// Execution history of a project, newest first (GUI history section).
pub fn history(project: &str) -> Vec<DoneTodoEntry> {
    history_at(&todos_file(), project)
}

/// Move a history entry back to the end of the pending queue (GUI "restore", round 16).
/// Returns whether the entry existed in history.
pub fn restore(project: &str, id: &str) -> bool {
    restore_at(&todos_file(), &todos_lock(), project, id)
}

/// Clear a project's execution history (GUI history section, round 18). Returns the number
/// of entries removed.
pub fn clear_history(project: &str) -> usize {
    clear_history_at(&todos_file(), &todos_lock(), project)
}

/// Reorder a project's queue to match `ids` (GUI drag handle, round 14). Best-effort under
/// concurrent add/remove: unknown ids are ignored, entries missing from `ids` keep their
/// relative order after the listed ones. Returns whether the stored order changed.
pub fn reorder(project: &str, ids: &[String]) -> bool {
    reorder_at(&todos_file(), &todos_lock(), project, ids)
}

/// Collect the todo ids a terminal answer consumed (pure function, spec D2/D5/D7).
///
/// Two sources, deduplicated:
/// - options carrying a `todo_id` whose text was selected (whats-next / Stop-card chips;
///   channels only report the option text, so ids are recovered from the request);
/// - explicit `QuestionAnswer.todo_ids` (popup collapsible todo section).
///
/// The caller (Coordinator, at the first-terminal convergence point) passes the result to
/// [`take`]; missing ids are skipped there (best-effort, spec D11).
pub fn ids_to_dequeue(
    request: &crate::models::AskRequest,
    result: &crate::models::ChannelResult,
) -> Vec<String> {
    if result.action != crate::models::ChannelAction::Send {
        return Vec::new();
    }
    let mut ids: Vec<String> = Vec::new();
    for (i, answer) in result.answers.iter().enumerate() {
        let options = request
            .questions
            .get(i)
            .map(|q| q.predefined_options.as_slice())
            .unwrap_or(&[]);
        for sel in &answer.selected_options {
            if let Some(id) = options
                .iter()
                .find(|o| &o.text == sel)
                .and_then(|o| o.todo_id.clone())
            {
                if !ids.contains(&id) {
                    ids.push(id);
                }
            }
        }
        for id in &answer.todo_ids {
            if !ids.contains(id) {
                ids.push(id.clone());
            }
        }
    }
    ids
}

// ===== Path-parameterized implementations (unit-testable without touching the real home) =====

pub fn list_at(path: &Path, project: &str) -> Vec<TodoEntry> {
    load_at(path)
        .projects
        .get(project.trim())
        .cloned()
        .unwrap_or_default()
}

pub fn add_at(path: &Path, lock: &Path, project: &str, text: &str) -> Result<TodoEntry, AddError> {
    add_impl(path, lock, project, text, false, None)
}

pub fn add_auto_at(
    path: &Path,
    lock: &Path,
    project: &str,
    text: &str,
) -> Result<TodoEntry, AddError> {
    add_impl(path, lock, project, text, true, None)
}

fn add_impl(
    path: &Path,
    lock: &Path,
    project: &str,
    text: &str,
    auto: bool,
    agent_kind: Option<&str>,
) -> Result<TodoEntry, AddError> {
    let (project, text) = normalized(project, text).ok_or(AddError::EmptyInput)?;
    let _guard = lock_at(lock);
    let mut data = load_at(path);
    let entry = TodoEntry {
        id: uuid::Uuid::new_v4().to_string(),
        text,
        created_at_ms: now_ms(),
        agent_kind: agent_kind.map(str::to_string),
        auto,
    };
    data.projects
        .entry(project.clone())
        .or_default()
        .push(entry.clone());
    if !store_at(path, data) {
        return Err(AddError::Persist);
    }
    // Defense in depth: never claim success unless the entry is readable back.
    if !list_at(path, &project).iter().any(|e| e.id == entry.id) {
        return Err(AddError::Persist);
    }
    Ok(entry)
}

pub fn set_auto_at(path: &Path, lock: &Path, project: &str, id: &str, auto: bool) -> Option<bool> {
    let _guard = lock_at(lock);
    let mut data = load_at(path);
    let entry = data
        .projects
        .get_mut(project.trim())?
        .iter_mut()
        .find(|e| e.id == id)?;
    if entry.auto == auto {
        return Some(auto);
    }
    entry.auto = auto;
    store_at(path, data);
    Some(auto)
}

pub fn remove_at(path: &Path, lock: &Path, project: &str, id: &str) -> bool {
    let _guard = lock_at(lock);
    let mut data = load_at(path);
    let Some(entries) = data.projects.get_mut(project.trim()) else {
        return false;
    };
    let before = entries.len();
    entries.retain(|e| e.id != id);
    let removed = entries.len() != before;
    if removed {
        store_at(path, data);
    }
    removed
}

pub fn clear_at(path: &Path, lock: &Path, project: &str) -> usize {
    let _guard = lock_at(lock);
    let mut data = load_at(path);
    let removed = data
        .projects
        .remove(project.trim())
        .map(|e| e.len())
        .unwrap_or(0);
    if removed > 0 {
        store_at(path, data);
    }
    removed
}

pub fn clear_history_at(path: &Path, lock: &Path, project: &str) -> usize {
    let _guard = lock_at(lock);
    let mut data = load_at(path);
    let removed = data
        .history
        .remove(project.trim())
        .map(|e| e.len())
        .unwrap_or(0);
    if removed > 0 {
        store_at(path, data);
    }
    removed
}

pub fn reorder_at(path: &Path, lock: &Path, project: &str, ids: &[String]) -> bool {
    let _guard = lock_at(lock);
    let mut data = load_at(path);
    let Some(entries) = data.projects.get_mut(project.trim()) else {
        return false;
    };
    let before: Vec<String> = entries.iter().map(|e| e.id.clone()).collect();
    let mut rest = std::mem::take(entries);
    let mut listed: Vec<TodoEntry> = Vec::with_capacity(rest.len());
    for id in ids {
        if let Some(pos) = rest.iter().position(|e| &e.id == id) {
            listed.push(rest.remove(pos));
        }
    }
    // `ids` 之外的条目（并发新增等）按原相对顺序压后。
    listed.append(&mut rest);
    let changed = !listed.iter().map(|e| &e.id).eq(before.iter());
    *entries = listed;
    if changed {
        store_at(path, data);
    }
    changed
}

pub fn take_at(
    path: &Path,
    lock: &Path,
    project: &str,
    ids: &[String],
    history_limit: usize,
) -> Vec<TodoEntry> {
    if ids.is_empty() {
        return Vec::new();
    }
    let _guard = lock_at(lock);
    let mut data = load_at(path);
    let Some(entries) = data.projects.get_mut(project.trim()) else {
        return Vec::new();
    };
    let mut taken = Vec::new();
    entries.retain(|e| {
        if ids.iter().any(|id| id == &e.id) {
            taken.push(e.clone());
            false
        } else {
            true
        }
    });
    if !taken.is_empty() {
        // Record execution history (chronological append; trim oldest beyond the cap).
        if history_limit > 0 {
            let done_at = now_ms();
            let hist = data.history.entry(project.trim().to_string()).or_default();
            for e in &taken {
                hist.push(DoneTodoEntry {
                    id: e.id.clone(),
                    text: e.text.clone(),
                    created_at_ms: e.created_at_ms,
                    agent_kind: e.agent_kind.clone(),
                    done_at_ms: done_at,
                });
            }
            if hist.len() > history_limit {
                let overflow = hist.len() - history_limit;
                hist.drain(..overflow);
            }
        }
        store_at(path, data);
    }
    taken
}

pub fn history_at(path: &Path, project: &str) -> Vec<DoneTodoEntry> {
    let mut entries = load_at(path)
        .history
        .get(project.trim())
        .cloned()
        .unwrap_or_default();
    entries.reverse(); // newest first
    entries
}

pub fn restore_at(path: &Path, lock: &Path, project: &str, id: &str) -> bool {
    let _guard = lock_at(lock);
    let mut data = load_at(path);
    let Some(hist) = data.history.get_mut(project.trim()) else {
        return false;
    };
    let Some(pos) = hist.iter().position(|e| e.id == id) else {
        return false;
    };
    let done = hist.remove(pos);
    data.projects
        .entry(project.trim().to_string())
        .or_default()
        .push(TodoEntry {
            id: done.id,
            text: done.text,
            created_at_ms: done.created_at_ms,
            agent_kind: done.agent_kind,
            // 恢复为普通待办：带 auto 恢复会立刻重新触发自动链，违背「找回来看看」的意图。
            auto: false,
        });
    store_at(path, data);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TempStore {
        dir: PathBuf,
    }

    impl TempStore {
        fn new() -> Self {
            let dir = std::env::temp_dir().join(format!("ah-todos-test-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(&dir).unwrap();
            Self { dir }
        }
        fn file(&self) -> PathBuf {
            self.dir.join("todos.json")
        }
        fn lock(&self) -> PathBuf {
            self.dir.join("todos.lock")
        }
    }

    impl Drop for TempStore {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    #[test]
    fn add_list_fifo_roundtrip() {
        let t = TempStore::new();
        assert!(list_at(&t.file(), "/p").is_empty());
        let a = add_at(&t.file(), &t.lock(), "/p", "第一条").unwrap();
        let b = add_at(&t.file(), &t.lock(), "/p", "  second  ").unwrap();
        assert_eq!(b.text, "second"); // trimmed
        let entries = list_at(&t.file(), "/p");
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].id, a.id); // FIFO order preserved
        assert_eq!(entries[0].text, "第一条");
        assert!(entries[0].created_at_ms > 0);
        // Other projects unaffected.
        assert!(list_at(&t.file(), "/q").is_empty());
    }

    #[test]
    fn agent_origin_round_trips_through_history_and_restore() {
        let t = TempStore::new();
        let agent = add_impl(
            &t.file(),
            &t.lock(),
            "/p",
            "agent task",
            false,
            Some("codex"),
        )
        .unwrap();
        let human = add_at(&t.file(), &t.lock(), "/p", "human task").unwrap();
        assert_eq!(agent.agent_kind.as_deref(), Some("codex"));
        assert_eq!(human.agent_kind, None);

        let raw = std::fs::read_to_string(t.file()).unwrap();
        assert_eq!(raw.matches("agentKind").count(), 1);
        take_at(
            &t.file(),
            &t.lock(),
            "/p",
            std::slice::from_ref(&agent.id),
            20,
        );
        assert_eq!(
            history_at(&t.file(), "/p")[0].agent_kind.as_deref(),
            Some("codex")
        );

        assert!(restore_at(&t.file(), &t.lock(), "/p", &agent.id));
        let restored = list_at(&t.file(), "/p")
            .into_iter()
            .find(|entry| entry.id == agent.id)
            .unwrap();
        assert_eq!(restored.agent_kind.as_deref(), Some("codex"));
    }

    #[test]
    fn add_rejects_empty_text_or_project() {
        let t = TempStore::new();
        assert_eq!(
            add_at(&t.file(), &t.lock(), "/p", "   "),
            Err(AddError::EmptyInput)
        );
        assert_eq!(
            add_at(&t.file(), &t.lock(), "  ", "task"),
            Err(AddError::EmptyInput)
        );
        assert!(!t.file().exists());
    }

    #[test]
    fn add_reports_persist_failure_when_path_unwritable() {
        // Parent path is a regular file → create_dir_all / write cannot succeed.
        let t = TempStore::new();
        let blocker = t.dir.join("not-a-dir");
        std::fs::write(&blocker, b"x").unwrap();
        let file = blocker.join("todos.json");
        let lock = t.dir.join("todos.lock");
        assert_eq!(
            add_at(&file, &lock, "/p", "should not land"),
            Err(AddError::Persist)
        );
        assert!(!file.exists());
    }

    #[test]
    fn remove_by_id_and_prune_empty_project() {
        let t = TempStore::new();
        let a = add_at(&t.file(), &t.lock(), "/p", "one").unwrap();
        assert!(remove_at(&t.file(), &t.lock(), "/p", &a.id));
        assert!(!remove_at(&t.file(), &t.lock(), "/p", &a.id)); // already gone
        assert!(list_at(&t.file(), "/p").is_empty());
        // Project key pruned from file.
        let raw = std::fs::read_to_string(t.file()).unwrap();
        assert!(!raw.contains("/p"));
    }

    #[test]
    fn clear_returns_count_and_needs_entries() {
        let t = TempStore::new();
        let _ = add_at(&t.file(), &t.lock(), "/p", "a");
        let _ = add_at(&t.file(), &t.lock(), "/p", "b");
        assert_eq!(clear_at(&t.file(), &t.lock(), "/p"), 2);
        assert_eq!(clear_at(&t.file(), &t.lock(), "/p"), 0);
    }

    #[test]
    fn take_dequeues_best_effort() {
        let t = TempStore::new();
        let a = add_at(&t.file(), &t.lock(), "/p", "a").unwrap();
        let b = add_at(&t.file(), &t.lock(), "/p", "b").unwrap();
        // One real id + one stale id: only the real one is taken, no error (spec D11).
        let taken = take_at(
            &t.file(),
            &t.lock(),
            "/p",
            &[a.id.clone(), "missing".to_string()],
            20,
        );
        assert_eq!(taken.len(), 1);
        assert_eq!(taken[0].text, "a");
        let left = list_at(&t.file(), "/p");
        assert_eq!(left.len(), 1);
        assert_eq!(left[0].id, b.id);
        // Empty ids → no-op.
        assert!(take_at(&t.file(), &t.lock(), "/p", &[], 20).is_empty());
        // Unknown project → no-op.
        assert!(take_at(&t.file(), &t.lock(), "/q", std::slice::from_ref(&b.id), 20).is_empty());
    }

    #[test]
    fn take_records_history_and_restore_moves_back() {
        let t = TempStore::new();
        let a = add_at(&t.file(), &t.lock(), "/p", "a").unwrap();
        let b = add_at(&t.file(), &t.lock(), "/p", "b").unwrap();
        take_at(&t.file(), &t.lock(), "/p", std::slice::from_ref(&a.id), 20);
        take_at(&t.file(), &t.lock(), "/p", std::slice::from_ref(&b.id), 20);
        // 倒序（最新在前）；带完成时间。
        let hist = history_at(&t.file(), "/p");
        assert_eq!(hist.len(), 2);
        assert_eq!(hist[0].id, b.id);
        assert_eq!(hist[1].id, a.id);
        assert!(hist[0].done_at_ms > 0);
        // 恢复：回到待办队列末尾并从历史移除。
        assert!(restore_at(&t.file(), &t.lock(), "/p", &a.id));
        assert!(!restore_at(&t.file(), &t.lock(), "/p", &a.id)); // already restored
        let entries = list_at(&t.file(), "/p");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].id, a.id);
        assert_eq!(entries[0].text, "a");
        assert_eq!(history_at(&t.file(), "/p").len(), 1);
        // 手动删除不进历史。
        remove_at(&t.file(), &t.lock(), "/p", &a.id);
        assert_eq!(history_at(&t.file(), "/p").len(), 1);
    }

    #[test]
    fn history_caps_at_limit_and_zero_disables_recording() {
        let t = TempStore::new();
        for i in 0..4 {
            let e = add_at(&t.file(), &t.lock(), "/p", &format!("t{i}")).unwrap();
            take_at(&t.file(), &t.lock(), "/p", &[e.id], 3);
        }
        // 超上限丢最旧：只剩最近 3 条（t1..t3，倒序 t3 在前）。
        let hist = history_at(&t.file(), "/p");
        assert_eq!(hist.len(), 3);
        assert_eq!(hist[0].text, "t3");
        assert_eq!(hist[2].text, "t1");
        // limit=0：停止新增，但既有历史保留（与回复历史同语义）。
        let e = add_at(&t.file(), &t.lock(), "/p", "t4").unwrap();
        take_at(&t.file(), &t.lock(), "/p", &[e.id], 0);
        let hist = history_at(&t.file(), "/p");
        assert_eq!(hist.len(), 3);
        assert_eq!(hist[0].text, "t3");
    }

    #[test]
    fn ids_to_dequeue_collects_selected_chips_and_explicit_ids() {
        use crate::models::{
            AskRequest, ChannelAction, ChannelResult, MessagePrompt, OptionItem, Question,
            QuestionAnswer,
        };
        let request = AskRequest::new(
            MessagePrompt::default(),
            vec![Question::new(
                "What should we do next?".into(),
                vec![
                    OptionItem::with_todo("修 bug", "id-1"),
                    OptionItem::with_todo("写文档", "id-2"),
                    OptionItem::new("End this turn", false),
                ],
            )],
            true,
        );
        // 选中一条待办 chip + 弹窗折叠区显式 id（含重复）→ 去重合并；「结束」选项无 id。
        let result = ChannelResult {
            action: ChannelAction::Send,
            answers: vec![QuestionAnswer {
                selected_options: vec!["修 bug".into(), "End this turn".into()],
                user_input: None,
                images: Vec::new(),
                files: Vec::new(),
                todo_ids: vec!["id-1".into(), "id-3".into()],
            }],
            source_channel_id: "popup".into(),
        };
        assert_eq!(ids_to_dequeue(&request, &result), vec!["id-1", "id-3"]);
        // 取消路径不出队。
        assert!(ids_to_dequeue(&request, &ChannelResult::cancel("popup")).is_empty());
        // 普通提问（无 todo 选项、无显式 id）不出队。
        let plain = ChannelResult {
            action: ChannelAction::Send,
            answers: vec![QuestionAnswer {
                selected_options: vec!["End this turn".into()],
                ..Default::default()
            }],
            source_channel_id: "popup".into(),
        };
        assert!(ids_to_dequeue(&request, &plain).is_empty());
    }

    #[test]
    fn clear_history_removes_project_history_only() {
        let t = TempStore::new();
        let a = add_at(&t.file(), &t.lock(), "/p", "a").unwrap();
        let _b = add_at(&t.file(), &t.lock(), "/p", "b").unwrap();
        take_at(&t.file(), &t.lock(), "/p", std::slice::from_ref(&a.id), 20);
        assert_eq!(history_at(&t.file(), "/p").len(), 1);
        assert_eq!(clear_history_at(&t.file(), &t.lock(), "/p"), 1);
        assert!(history_at(&t.file(), "/p").is_empty());
        // 待办队列不受影响；未知项目 → 0。
        assert_eq!(list_at(&t.file(), "/p").len(), 1);
        assert_eq!(clear_history_at(&t.file(), &t.lock(), "/q"), 0);
    }

    #[test]
    fn reorder_moves_listed_ids_and_keeps_unlisted_tail() {
        let t = TempStore::new();
        let a = add_at(&t.file(), &t.lock(), "/p", "a").unwrap();
        let b = add_at(&t.file(), &t.lock(), "/p", "b").unwrap();
        let c = add_at(&t.file(), &t.lock(), "/p", "c").unwrap();
        // 完整重排 b, c, a。
        assert!(reorder_at(
            &t.file(),
            &t.lock(),
            "/p",
            &[b.id.clone(), c.id.clone(), a.id.clone()]
        ));
        let ids: Vec<_> = list_at(&t.file(), "/p").into_iter().map(|e| e.id).collect();
        assert_eq!(ids, vec![b.id.clone(), c.id.clone(), a.id.clone()]);
        // 相同顺序 → 未变化，不写盘。
        assert!(!reorder_at(
            &t.file(),
            &t.lock(),
            "/p",
            &[b.id.clone(), c.id.clone(), a.id.clone()]
        ));
        // 部分 id（含过期 id）：列出的排前，其余压后保持相对顺序；过期 id 忽略（best-effort）。
        assert!(reorder_at(
            &t.file(),
            &t.lock(),
            "/p",
            &[a.id.clone(), "missing".to_string()]
        ));
        let ids: Vec<_> = list_at(&t.file(), "/p").into_iter().map(|e| e.id).collect();
        assert_eq!(ids, vec![a.id.clone(), b.id.clone(), c.id.clone()]);
        // 未知项目 → no-op。
        assert!(!reorder_at(&t.file(), &t.lock(), "/q", &[a.id]));
    }

    #[test]
    fn overflow_note_only_above_cap() {
        use crate::i18n::Lang;
        assert!(overflow_note(MAX_OPTION_TODOS, Lang::Zh).is_none());
        let note = overflow_note(MAX_OPTION_TODOS + 3, Lang::Zh).unwrap();
        assert!(note.contains('3'), "{note}");
    }

    #[test]
    fn auto_flag_add_toggle_and_first_auto_order() {
        let t = TempStore::new();
        let a = add_at(&t.file(), &t.lock(), "/p", "normal").unwrap();
        let b = add_auto_at(&t.file(), &t.lock(), "/p", "auto1").unwrap();
        let c = add_auto_at(&t.file(), &t.lock(), "/p", "auto2").unwrap();
        assert!(!a.auto);
        assert!(b.auto && c.auto);
        // 最靠前的自动待办 = 队列顺序里第一条 auto（b 在 c 前）。
        let first = list_at(&t.file(), "/p")
            .into_iter()
            .find(|e| e.auto)
            .unwrap();
        assert_eq!(first.id, b.id);
        // 切换：关掉 b 后轮到 c；开回普通条目 a 后 a 最靠前。
        assert_eq!(
            set_auto_at(&t.file(), &t.lock(), "/p", &b.id, false),
            Some(false)
        );
        let first = list_at(&t.file(), "/p")
            .into_iter()
            .find(|e| e.auto)
            .unwrap();
        assert_eq!(first.id, c.id);
        assert_eq!(
            set_auto_at(&t.file(), &t.lock(), "/p", &a.id, true),
            Some(true)
        );
        let first = list_at(&t.file(), "/p")
            .into_iter()
            .find(|e| e.auto)
            .unwrap();
        assert_eq!(first.id, a.id);
        // 不存在的 id → None。
        assert_eq!(
            set_auto_at(&t.file(), &t.lock(), "/p", "missing", true),
            None
        );
        // 序列化：auto=false 不落盘（skip_serializing_if），旧文件无该字段可读。
        let raw = std::fs::read_to_string(t.file()).unwrap();
        assert_eq!(raw.matches("\"auto\"").count(), 2); // a 与 c

        // 恢复历史不带 auto。
        take_at(&t.file(), &t.lock(), "/p", std::slice::from_ref(&a.id), 20);
        assert!(restore_at(&t.file(), &t.lock(), "/p", &a.id));
        let restored = list_at(&t.file(), "/p")
            .into_iter()
            .find(|e| e.id == a.id)
            .unwrap();
        assert!(!restored.auto);
    }

    #[test]
    fn corrupt_file_degrades_to_empty() {
        let t = TempStore::new();
        std::fs::write(t.file(), "not json").unwrap();
        assert!(list_at(&t.file(), "/p").is_empty());
        // Mutation on top of a corrupt file starts fresh instead of failing.
        add_at(&t.file(), &t.lock(), "/p", "x").unwrap();
        assert_eq!(list_at(&t.file(), "/p").len(), 1);
    }

    #[test]
    fn all_snapshot_groups_by_project() {
        let t = TempStore::new();
        let _ = add_at(&t.file(), &t.lock(), "/p", "a");
        let _ = add_at(&t.file(), &t.lock(), "/q", "b");
        let all = load_at(&t.file()).projects;
        assert_eq!(all.len(), 2);
        assert_eq!(all["/p"][0].text, "a");
        assert_eq!(all["/q"][0].text, "b");
    }
}
