//! Validated runtime transcript-path hints for agents whose session directory is configurable.
//!
//! Hooks and MCP metadata can report a conversation's transcript path when an agent runs under a
//! non-default configuration root (for example T3 Code's `--gemini-dir ... --app-data-dir
//! antigravity-acp`, custom `CODEX_HOME` / `CLAUDE_CONFIG_DIR` / `GROK_HOME` /
//! `PI_CODING_AGENT_DIR` / `CURSOR_CONFIG_DIR`). Every path is canonicalized and structurally
//! verified against the agent's session layout before being cached or read, so an untrusted
//! payload can never point AskHuman at arbitrary files.

use super::AgentKind;
use serde_json::Value;
use std::collections::{HashMap, VecDeque};
use std::io::{BufRead, BufReader, Read};
use std::path::{Component, Path, PathBuf};
use std::sync::{Mutex, OnceLock};

const MAX_PATH_CHARS: usize = 8_192;
const MAX_HEADER_BYTES: u64 = 64 * 1024;
const MAX_CACHED_PATHS: usize = 1024;

type Key = (AgentKind, String);

#[derive(Default)]
struct PathCache {
    entries: HashMap<Key, PathBuf>,
    order: VecDeque<Key>,
}

fn paths() -> &'static Mutex<PathCache> {
    static PATHS: OnceLock<Mutex<PathCache>> = OnceLock::new();
    PATHS.get_or_init(|| Mutex::new(PathCache::default()))
}

fn valid_session_component(session_id: &str) -> bool {
    let trimmed = session_id.trim();
    !trimmed.is_empty()
        && trimmed.len() <= 256
        && !trimmed.contains('/')
        && !trimmed.contains('\\')
        && !trimmed.contains('\0')
        && trimmed != "."
        && trimmed != ".."
}

#[cfg(unix)]
fn owned_by_current_user(path: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    let Ok(meta) = std::fs::metadata(path) else {
        return false;
    };
    // SAFETY: `geteuid` has no preconditions and cannot fail.
    let uid = unsafe { libc::geteuid() };
    meta.uid() == uid
}

#[cfg(not(unix))]
fn owned_by_current_user(_path: &Path) -> bool {
    true
}

fn path_tail_strings(path: &Path, count: usize) -> Option<Vec<String>> {
    let parts: Vec<String> = path
        .components()
        .filter_map(|component| match component {
            Component::Normal(segment) => Some(segment.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect();
    if parts.len() < count {
        return None;
    }
    Some(parts[parts.len() - count..].to_vec())
}

/// Structurally validate a transcript path for any integrated agent without touching the cache.
/// Returns the canonical path when it exists as a regular file and matches the agent's layout.
pub fn validate_path(
    kind: AgentKind,
    session_id: &str,
    raw_path: &str,
    cwd: Option<&str>,
) -> Option<PathBuf> {
    let session_id = session_id.trim();
    let raw_path = raw_path.trim();
    if !valid_session_component(session_id)
        || raw_path.is_empty()
        || raw_path.contains('\0')
        || raw_path.chars().count() > MAX_PATH_CHARS
    {
        return None;
    }
    let path = std::fs::canonicalize(raw_path).ok()?;
    if !path.is_file() {
        return None;
    }
    match kind {
        AgentKind::Pi => validate_pi_file(&path, session_id, cwd).then_some(path),
        AgentKind::Antigravity => validate_antigravity_file(&path, session_id).then_some(path),
        AgentKind::Claude => {
            let expected = format!("{session_id}.jsonl");
            (path.file_name().and_then(|n| n.to_str()) == Some(expected.as_str())).then_some(path)
        }
        AgentKind::Codex => {
            let name = path.file_name().and_then(|n| n.to_str())?;
            let suffix = format!("-{session_id}.jsonl");
            (name.starts_with("rollout-") && name.ends_with(&suffix)).then_some(path)
        }
        AgentKind::Cursor => {
            let tail = path_tail_strings(&path, 3)?;
            let expected_file = format!("{session_id}.jsonl");
            (tail[0] == "agent-transcripts" && tail[1] == session_id && tail[2] == expected_file)
                .then_some(path)
        }
        AgentKind::Grok => {
            let tail = path_tail_strings(&path, 2)?;
            (tail[0] == session_id && tail[1] == "chat_history.jsonl").then_some(path)
        }
    }
}

fn validate_pi_file(path: &Path, session_id: &str, cwd: Option<&str>) -> bool {
    let Ok(file) = std::fs::File::open(path) else {
        return false;
    };
    let mut header_line = String::new();
    if BufReader::new(file)
        .take(MAX_HEADER_BYTES)
        .read_line(&mut header_line)
        .is_err()
    {
        return false;
    }
    let Ok(header) = serde_json::from_str::<Value>(&header_line) else {
        return false;
    };
    if header.get("type").and_then(Value::as_str) != Some("session")
        || header.get("version").and_then(Value::as_u64) != Some(3)
        || header.get("id").and_then(Value::as_str) != Some(session_id)
    {
        return false;
    }
    if let (Some(expected), Some(actual)) = (cwd, header.get("cwd").and_then(Value::as_str)) {
        if !crate::path_identity::equivalent(expected, actual) {
            return false;
        }
    }
    true
}

fn validate_antigravity_file(path: &Path, session_id: &str) -> bool {
    let Ok(uuid) = uuid::Uuid::parse_str(session_id) else {
        return false;
    };
    if !owned_by_current_user(path) {
        return false;
    }
    let canonical_id = uuid.hyphenated().to_string();
    let Some(tail) = path_tail_strings(path, 6) else {
        return false;
    };
    // .../<surface>/brain/<uuid>/.system_generated/logs/transcript_full.jsonl
    let surface = &tail[0];
    if surface == "antigravity-ide" {
        return false;
    }
    tail[1] == "brain"
        && tail[2].eq_ignore_ascii_case(&canonical_id)
        && tail[3] == ".system_generated"
        && tail[4] == "logs"
        && tail[5] == "transcript_full.jsonl"
}

/// Validate and cache a runtime transcript path for any integrated agent. Returns the canonical
/// path string for persistence.
pub fn register(
    kind: AgentKind,
    session_id: &str,
    raw_path: &str,
    cwd: Option<&str>,
) -> Option<String> {
    let path = validate_path(kind, session_id, raw_path, cwd)?;
    let key = (kind, session_id.trim().to_string());
    let mut cache = paths().lock().unwrap();
    cache.order.retain(|existing| existing != &key);
    cache.order.push_back(key.clone());
    cache.entries.insert(key, path.clone());
    while cache.entries.len() > MAX_CACHED_PATHS {
        if let Some(oldest) = cache.order.pop_front() {
            cache.entries.remove(&oldest);
        }
    }
    Some(path.to_string_lossy().to_string())
}

pub fn get(kind: AgentKind, session_id: &str) -> Option<PathBuf> {
    let key = (kind, session_id.trim().to_string());
    let mut cache = paths().lock().unwrap();
    let path = cache.entries.get(&key).cloned()?;
    cache.order.retain(|existing| existing != &key);
    cache.order.push_back(key);
    path.is_file().then_some(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn validates_pi_header_identity_and_cwd() {
        let dir = tempfile::tempdir().unwrap();
        let transcript = dir.path().join("session.jsonl");
        let mut file = std::fs::File::create(&transcript).unwrap();
        writeln!(
            file,
            "{}",
            serde_json::json!({
                "type": "session",
                "version": 3,
                "id": "pi-session",
                "cwd": dir.path(),
            })
        )
        .unwrap();
        let registered = register(
            AgentKind::Pi,
            "pi-session",
            &transcript.to_string_lossy(),
            Some(&dir.path().to_string_lossy()),
        )
        .unwrap();
        assert_eq!(
            Path::new(&registered),
            std::fs::canonicalize(transcript).unwrap()
        );
        assert!(register(AgentKind::Pi, "other", &registered, None).is_none());
        assert!(register(AgentKind::Pi, "pi-session", &registered, Some("/different")).is_none());
    }

    #[test]
    fn validates_antigravity_acp_and_rejects_ide_and_mismatched_paths() {
        let dir = tempfile::tempdir().unwrap();
        let uuid = "4e200329-9002-48ef-9a74-faf0704f25d4";
        let acp_path = dir
            .path()
            .join("providers/antigravity/hash/antigravity-acp/brain")
            .join(uuid)
            .join(".system_generated/logs/transcript_full.jsonl");
        std::fs::create_dir_all(acp_path.parent().unwrap()).unwrap();
        std::fs::write(&acp_path, "{}\n").unwrap();

        let registered = register(
            AgentKind::Antigravity,
            uuid,
            &acp_path.to_string_lossy(),
            None,
        )
        .unwrap();
        assert_eq!(
            Path::new(&registered),
            std::fs::canonicalize(&acp_path).unwrap()
        );
        assert_eq!(
            get(AgentKind::Antigravity, uuid),
            Some(std::fs::canonicalize(&acp_path).unwrap())
        );

        // Wrong UUID is rejected.
        assert!(validate_path(
            AgentKind::Antigravity,
            "11111111-1111-4111-8111-111111111111",
            &acp_path.to_string_lossy(),
            None
        )
        .is_none());
        // Non-UUID id is rejected.
        assert!(validate_path(
            AgentKind::Antigravity,
            "../etc/passwd",
            &acp_path.to_string_lossy(),
            None
        )
        .is_none());

        // IDE surface is rejected.
        let ide_path = dir
            .path()
            .join("antigravity-ide/brain")
            .join(uuid)
            .join(".system_generated/logs/transcript_full.jsonl");
        std::fs::create_dir_all(ide_path.parent().unwrap()).unwrap();
        std::fs::write(&ide_path, "{}\n").unwrap();
        assert!(validate_path(
            AgentKind::Antigravity,
            uuid,
            &ide_path.to_string_lossy(),
            None
        )
        .is_none());

        // Arbitrary file is rejected.
        let other = dir.path().join("transcript_full.jsonl");
        std::fs::write(&other, "{}\n").unwrap();
        assert!(
            validate_path(AgentKind::Antigravity, uuid, &other.to_string_lossy(), None).is_none()
        );
    }

    #[test]
    fn validates_claude_codex_cursor_grok_layouts() {
        let dir = tempfile::tempdir().unwrap();
        let sid = "sess-1234";

        let claude = dir.path().join("projects/p").join(format!("{sid}.jsonl"));
        std::fs::create_dir_all(claude.parent().unwrap()).unwrap();
        std::fs::write(&claude, "{}\n").unwrap();
        assert!(validate_path(AgentKind::Claude, sid, &claude.to_string_lossy(), None).is_some());
        assert!(
            validate_path(AgentKind::Claude, "other", &claude.to_string_lossy(), None).is_none()
        );

        let codex = dir
            .path()
            .join("sessions/2026/11/24")
            .join(format!("rollout-2026-11-24-{sid}.jsonl"));
        std::fs::create_dir_all(codex.parent().unwrap()).unwrap();
        std::fs::write(&codex, "{}\n").unwrap();
        assert!(validate_path(AgentKind::Codex, sid, &codex.to_string_lossy(), None).is_some());
        assert!(validate_path(AgentKind::Codex, sid, &claude.to_string_lossy(), None).is_none());

        let cursor = dir
            .path()
            .join("projects/p/agent-transcripts")
            .join(sid)
            .join(format!("{sid}.jsonl"));
        std::fs::create_dir_all(cursor.parent().unwrap()).unwrap();
        std::fs::write(&cursor, "{}\n").unwrap();
        assert!(validate_path(AgentKind::Cursor, sid, &cursor.to_string_lossy(), None).is_some());
        assert!(validate_path(AgentKind::Cursor, sid, &claude.to_string_lossy(), None).is_none());

        let grok = dir
            .path()
            .join("sessions/cwd")
            .join(sid)
            .join("chat_history.jsonl");
        std::fs::create_dir_all(grok.parent().unwrap()).unwrap();
        std::fs::write(&grok, "{}\n").unwrap();
        assert!(validate_path(AgentKind::Grok, sid, &grok.to_string_lossy(), None).is_some());
        assert!(validate_path(AgentKind::Grok, "other", &grok.to_string_lossy(), None).is_none());
    }
}
