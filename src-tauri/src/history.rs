//! Reply history store: append-only JSONL at `~/.askhuman/history.jsonl` (one entry per line).
//!
//! Recording is a best-effort side channel invoked from the per-request coordinator after a
//! terminal result is produced; it must never affect the main flow (stdout / exit code). Image and
//! reply-file values are stored as paths only (never base64); display is best-effort (a missing
//! file just renders a placeholder). Mutating ops (record / trim / clear) hold a cross-process
//! file lock and write atomically (temp + rename). Reads tolerate malformed lines.

use crate::models::{ChannelAction, MessagePrompt, Question};
use crate::paths;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

/// One question's recorded answer (paths only, no base64).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryAnswer {
    #[serde(default)]
    pub selected_options: Vec<String>,
    #[serde(default)]
    pub user_input: Option<String>,
    /// Saved image file paths (already written to disk by `render_result`).
    #[serde(default)]
    pub images: Vec<String>,
    /// Reply file absolute paths (passed through, not copied).
    #[serde(default)]
    pub files: Vec<String>,
}

/// One recorded reply (one per request: the winning terminal result).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryEntry {
    pub id: String,
    pub timestamp_ms: i64,
    #[serde(default)]
    pub project: String,
    /// Caller source name (`ASKHUMAN_ENV_SOURCE_NAME`).
    #[serde(default)]
    pub source: String,
    /// Channel id that submitted / cancelled (popup / dingding / feishu / telegram).
    #[serde(default)]
    pub channel: String,
    pub action: ChannelAction,
    #[serde(default)]
    pub is_markdown: bool,
    #[serde(default)]
    pub message: MessagePrompt,
    #[serde(default)]
    pub questions: Vec<Question>,
    /// Per-question answers (empty for a cancel).
    #[serde(default)]
    pub answers: Vec<HistoryAnswer>,
}

/// Aggregated project info for the history window's project picker.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectInfo {
    pub key: String,
    pub name: String,
    pub count: usize,
    pub last_ms: i64,
}

/// Scope for clearing history.
pub enum ClearScope {
    /// Remove all entries.
    All,
    /// Remove entries of one project key.
    Project(String),
}

/// Current unix time in milliseconds.
pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

// ===== Public API (locks + default file path) =====

/// Append one entry then trim to the most recent `limit`. No-op when `limit == 0` (recording off:
/// existing entries are preserved, never truncated to zero here). Best-effort: errors are ignored.
pub fn record(entry: HistoryEntry, limit: u32) {
    if limit == 0 {
        return;
    }
    let _guard = lock();
    record_at(&paths::history_file(), entry, limit);
}

/// Load entries (optionally filtered to one project), most recent first.
pub fn load(project: Option<&str>, all: bool) -> Vec<HistoryEntry> {
    load_at(&paths::history_file(), project, all)
}

/// Distinct projects present in history, most recently active first.
pub fn projects() -> Vec<ProjectInfo> {
    projects_at(&paths::history_file())
}

/// Total number of entries.
pub fn count() -> usize {
    read_all_at(&paths::history_file()).len()
}

/// Trim to the most recent `limit` entries. `limit == 0` keeps all (no truncation). Returns the
/// remaining entry count.
pub fn trim(limit: u32) -> usize {
    let _guard = lock();
    trim_at(&paths::history_file(), limit)
}

/// Clear history by scope (all, or one project).
pub fn clear(scope: ClearScope) {
    let _guard = lock();
    clear_at(&paths::history_file(), scope);
}

// ===== Core logic (path-parameterized, lock-free; unit-testable) =====

fn record_at(path: &Path, entry: HistoryEntry, limit: u32) {
    if limit == 0 {
        return;
    }
    let mut entries = read_all_at(path);
    entries.push(entry);
    trim_vec(&mut entries, limit as usize);
    let _ = write_all_at(path, &entries);
}

fn load_at(path: &Path, project: Option<&str>, all: bool) -> Vec<HistoryEntry> {
    let mut entries = read_all_at(path);
    if !all {
        let key = project.unwrap_or("");
        entries.retain(|e| e.project == key);
    }
    entries.sort_by(|a, b| b.timestamp_ms.cmp(&a.timestamp_ms));
    entries
}

fn projects_at(path: &Path) -> Vec<ProjectInfo> {
    let entries = read_all_at(path);
    let mut map: HashMap<String, (usize, i64)> = HashMap::new();
    for e in &entries {
        let slot = map.entry(e.project.clone()).or_insert((0, 0));
        slot.0 += 1;
        if e.timestamp_ms > slot.1 {
            slot.1 = e.timestamp_ms;
        }
    }
    let mut out: Vec<ProjectInfo> = map
        .into_iter()
        .map(|(key, (count, last_ms))| ProjectInfo {
            name: crate::project::display_name(&key),
            key,
            count,
            last_ms,
        })
        .collect();
    out.sort_by(|a, b| b.last_ms.cmp(&a.last_ms));
    out
}

fn trim_at(path: &Path, limit: u32) -> usize {
    let mut entries = read_all_at(path);
    if limit == 0 {
        return entries.len();
    }
    trim_vec(&mut entries, limit as usize);
    let _ = write_all_at(path, &entries);
    entries.len()
}

fn clear_at(path: &Path, scope: ClearScope) {
    match scope {
        ClearScope::All => {
            let _ = std::fs::remove_file(path);
        }
        ClearScope::Project(key) => {
            let mut entries = read_all_at(path);
            entries.retain(|e| e.project != key);
            let _ = write_all_at(path, &entries);
        }
    }
}

/// Keep only the most recent `limit` entries (entries are in append order: oldest first).
fn trim_vec(entries: &mut Vec<HistoryEntry>, limit: usize) {
    if limit > 0 && entries.len() > limit {
        let drop = entries.len() - limit;
        entries.drain(0..drop);
    }
}

fn read_all_at(path: &Path) -> Vec<HistoryEntry> {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(_) => return Vec::new(),
    };
    text.lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| serde_json::from_str::<HistoryEntry>(l).ok())
        .collect()
}

fn write_all_at(path: &Path, entries: &[HistoryEntry]) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut buf = String::new();
    for e in entries {
        if let Ok(line) = serde_json::to_string(e) {
            buf.push_str(&line);
            buf.push('\n');
        }
    }
    let tmp = path.with_extension(format!("jsonl.tmp-{}", uuid::Uuid::new_v4()));
    std::fs::write(&tmp, buf.as_bytes())?;
    harden(&tmp);
    std::fs::rename(&tmp, path)?;
    harden(path);
    Ok(())
}

/// Restrict the history file to owner read/write (0600) on Unix; no-op elsewhere.
#[cfg(unix)]
fn harden(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    if let Ok(meta) = std::fs::metadata(path) {
        if meta.permissions().mode() & 0o777 != 0o600 {
            let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
        }
    }
}
#[cfg(not(unix))]
fn harden(_path: &Path) {}

// ===== Cross-process write lock =====

#[cfg(unix)]
struct LockGuard {
    _file: std::fs::File,
}

/// Acquire an exclusive (blocking) advisory lock for the duration of a write. Released on drop.
#[cfg(unix)]
fn lock() -> Option<LockGuard> {
    use std::os::unix::io::AsRawFd;
    if let Some(dir) = paths::history_lock().parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let file = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(false)
        .open(paths::history_lock())
        .ok()?;
    unsafe {
        libc::flock(file.as_raw_fd(), libc::LOCK_EX);
    }
    Some(LockGuard { _file: file })
}

#[cfg(not(unix))]
fn lock() -> Option<()> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn entry(id: &str, project: &str, ts: i64) -> HistoryEntry {
        HistoryEntry {
            id: id.to_string(),
            timestamp_ms: ts,
            project: project.to_string(),
            source: "the Loop".to_string(),
            channel: "popup".to_string(),
            action: ChannelAction::Send,
            is_markdown: true,
            message: MessagePrompt::default(),
            questions: Vec::new(),
            answers: Vec::new(),
        }
    }

    #[test]
    fn roundtrip_and_order() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("history.jsonl");
        record_at(&path, entry("a", "/p", 1), 200);
        record_at(&path, entry("b", "/p", 2), 200);
        let loaded = load_at(&path, Some("/p"), false);
        assert_eq!(loaded.len(), 2);
        // Most recent first.
        assert_eq!(loaded[0].id, "b");
        assert_eq!(loaded[1].id, "a");
    }

    #[test]
    fn trim_keeps_most_recent() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("history.jsonl");
        for i in 0..5 {
            record_at(&path, entry(&format!("e{i}"), "/p", i), 3);
        }
        let loaded = load_at(&path, None, true);
        assert_eq!(loaded.len(), 3);
        // Kept e4,e3,e2 (desc).
        assert_eq!(loaded[0].id, "e4");
        assert_eq!(loaded[2].id, "e2");
    }

    #[test]
    fn limit_zero_records_nothing() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("history.jsonl");
        record_at(&path, entry("a", "/p", 1), 0);
        assert_eq!(read_all_at(&path).len(), 0);
    }

    #[test]
    fn filter_by_project() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("history.jsonl");
        record_at(&path, entry("a", "/p1", 1), 200);
        record_at(&path, entry("b", "/p2", 2), 200);
        assert_eq!(load_at(&path, Some("/p1"), false).len(), 1);
        assert_eq!(load_at(&path, None, true).len(), 2);
        let projects = projects_at(&path);
        assert_eq!(projects.len(), 2);
        // Most recent project first.
        assert_eq!(projects[0].key, "/p2");
    }

    #[test]
    fn clear_scopes() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("history.jsonl");
        record_at(&path, entry("a", "/p1", 1), 200);
        record_at(&path, entry("b", "/p2", 2), 200);
        clear_at(&path, ClearScope::Project("/p1".to_string()));
        assert_eq!(load_at(&path, None, true).len(), 1);
        clear_at(&path, ClearScope::All);
        assert_eq!(read_all_at(&path).len(), 0);
    }

    #[test]
    fn skips_malformed_lines() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("history.jsonl");
        std::fs::write(&path, "not json\n{}\n").unwrap();
        // Both lines fail to parse into a full entry (missing required fields) → skipped.
        assert_eq!(read_all_at(&path).len(), 0);
    }

    #[test]
    fn trim_zero_keeps_all() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("history.jsonl");
        for i in 0..4 {
            record_at(&path, entry(&format!("e{i}"), "/p", i), 200);
        }
        assert_eq!(trim_at(&path, 0), 4);
        assert_eq!(read_all_at(&path).len(), 4);
        assert_eq!(trim_at(&path, 2), 2);
        assert_eq!(read_all_at(&path).len(), 2);
    }
}
