//! T3 Code delegated child-thread detection and silence markers
//! (spec `docs/specs/t3-code-and-multi-profile.md`, L3).
//!
//! When an agent runs inside T3 Code, T3 sets `T3CODE_THREAD_ID` and `T3CODE_CLI_PATH` in the
//! child environment. Interactive main threads (`parentThreadId == null && relationshipToParent ==
//! null`) retain full AskHuman functionality, whereas delegated child threads
//! (`parentThreadId != null || relationshipToParent != null`) have no human attending them
//! directly and must be silenced across every AskHuman surface (hooks, CLI, MCP, Stop,
//! PermissionRequest, Claude AskUserQuestion, failure confirmation).
//!
//! All checks fail open: missing `T3CODE_THREAD_ID` incurs zero I/O, and CLI timeouts or parse
//! failures treat the thread as interactive while caching the transient error briefly so hooks
//! never stall repeatedly.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use super::AgentKind;

pub const T3CODE_THREAD_ID_ENV: &str = "T3CODE_THREAD_ID";
pub const T3CODE_CLI_PATH_ENV: &str = "T3CODE_CLI_PATH";

pub const REASON_ENV_DELEGATED: &str = "t3_delegated_child_thread";
pub const REASON_MARKER_DELEGATED: &str = "t3_delegated_marker";

/// Guidance returned to models calling AskHuman CLI or MCP from a T3 delegated child thread.
pub const DELEGATED_MESSAGE: &str =
    "AskHuman is disabled in this T3 Code delegated child thread because no human is attending it directly.\n\
Do not retry or contact the human; complete the delegated task and return your result to the parent thread.";

const CACHE_TTL_MS: u64 = 24 * 60 * 60 * 1000;
const TRANSIENT_ERROR_TTL_MS: u64 = 30 * 1000;
const CACHE_PRUNE_AGE_MS: u64 = 7 * 24 * 60 * 60 * 1000;
const MARKER_TTL_MS: u64 = 14 * 24 * 60 * 60 * 1000;
const PRUNE_SCAN_LIMIT: usize = 200;
const CLI_TIMEOUT: Duration = Duration::from_millis(1_500);
const MAX_CLI_OUTPUT_BYTES: usize = 256 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct T3ThreadMeta {
    pub delegated: bool,
    pub parent_thread_id: Option<String>,
    pub relationship_to_parent: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ThreadCacheEntry {
    thread_id: String,
    delegated: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    parent_thread_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    relationship_to_parent: Option<String>,
    checked_at_ms: u64,
    #[serde(default)]
    transient_error: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SessionMarker {
    agent: String,
    session_id: String,
    thread_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    parent_thread_id: Option<String>,
    ts_ms: u64,
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn valid_thread_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

fn sha256_hex(input: &str) -> String {
    format!("{:x}", Sha256::digest(input.as_bytes()))
}

fn cache_file(dir: &Path, thread_id: &str) -> PathBuf {
    dir.join(format!("{}.json", sha256_hex(thread_id)))
}

fn marker_file(dir: &Path, kind: AgentKind, session_id: &str) -> PathBuf {
    let key = format!("{}:{}", kind.as_str(), session_id.trim());
    dir.join(format!("{}.json", sha256_hex(&key)))
}

/// Parse `t3 thread show --json --messages 0 <id>` output.
///
/// Only `parentThreadId` (non-empty string) or `relationshipToParent` (non-null) marks a thread
/// as delegated; a top-level thread with `createdBy == "agent"` and `parentThreadId == null` is
/// NOT delegated.
pub fn parse_t3_thread_delegated(bytes: &[u8]) -> Option<T3ThreadMeta> {
    let value: Value = serde_json::from_slice(bytes).ok()?;
    let thread = value.get("thread").unwrap_or(&value);
    if !thread.is_object() {
        return None;
    }
    let parent_thread_id = thread
        .get("parentThreadId")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let relationship_to_parent = thread.get("relationshipToParent").and_then(|v| {
        if v.is_null() {
            None
        } else if let Some(s) = v.as_str() {
            let trimmed = s.trim();
            (!trimmed.is_empty()).then(|| trimmed.to_string())
        } else {
            Some(v.to_string())
        }
    });
    let delegated = parent_thread_id.is_some() || relationship_to_parent.is_some();
    Some(T3ThreadMeta {
        delegated,
        parent_thread_id,
        relationship_to_parent,
    })
}

fn resolve_t3_cli_path() -> Option<PathBuf> {
    if let Ok(raw) = std::env::var(T3CODE_CLI_PATH_ENV) {
        let trimmed = raw.trim();
        if !trimmed.is_empty() && !trimmed.contains('\0') {
            let candidate = PathBuf::from(trimmed);
            if candidate.is_absolute() && candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    let fallback = crate::paths::system_account_home().ok()?.join(".t3/bin/t3");
    fallback.is_file().then_some(fallback)
}

fn run_t3_cli(cli: &Path, thread_id: &str, timeout: Duration) -> Option<Vec<u8>> {
    let mut child = Command::new(cli)
        .args(["thread", "show", "--json", "--messages", "0", thread_id])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;

    let mut stdout = child.stdout.take()?;
    let reader = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stdout
            .by_ref()
            .take((MAX_CLI_OUTPUT_BYTES + 1) as u64)
            .read_to_end(&mut buf);
        buf
    });

    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) if started.elapsed() < timeout => {
                std::thread::sleep(Duration::from_millis(10));
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
        }
    };

    let bytes = reader.join().ok()?;
    let status = status?;
    if !status.success() || bytes.len() > MAX_CLI_OUTPUT_BYTES {
        return None;
    }
    Some(bytes)
}

fn write_json_atomic(path: &Path, bytes: &[u8]) {
    let Some(parent) = path.parent() else {
        return;
    };
    if std::fs::create_dir_all(parent).is_err() {
        return;
    }
    let tmp = path.with_extension(format!("tmp-{}", uuid::Uuid::new_v4()));
    if std::fs::write(&tmp, bytes).is_ok() {
        let _ = std::fs::rename(&tmp, path);
    } else {
        let _ = std::fs::remove_file(&tmp);
    }
}

fn prune_old_files(dir: &Path, max_age_ms: u64, now: u64) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let max_age = Duration::from_millis(max_age_ms);
    let now_sys = UNIX_EPOCH + Duration::from_millis(now);
    for entry in entries.flatten().take(PRUNE_SCAN_LIMIT) {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let stale = entry
            .metadata()
            .ok()
            .and_then(|m| m.modified().ok())
            .and_then(|modified| now_sys.duration_since(modified).ok())
            .is_some_and(|age| age > max_age);
        if stale {
            let _ = std::fs::remove_file(path);
        }
    }
}

fn lookup_thread_with(
    cache_dir: &Path,
    thread_id: &str,
    now: u64,
    query: &dyn Fn(&str) -> Option<T3ThreadMeta>,
) -> Option<T3ThreadMeta> {
    if !valid_thread_id(thread_id) {
        return None;
    }
    let path = cache_file(cache_dir, thread_id);
    if let Ok(bytes) = std::fs::read(&path) {
        if let Ok(entry) = serde_json::from_slice::<ThreadCacheEntry>(&bytes) {
            let ttl = if entry.transient_error {
                TRANSIENT_ERROR_TTL_MS
            } else {
                CACHE_TTL_MS
            };
            if entry.thread_id == thread_id && now.saturating_sub(entry.checked_at_ms) < ttl {
                return (!entry.transient_error).then_some(T3ThreadMeta {
                    delegated: entry.delegated,
                    parent_thread_id: entry.parent_thread_id,
                    relationship_to_parent: entry.relationship_to_parent,
                });
            }
        }
    }

    let result = query(thread_id);
    let entry = match &result {
        Some(meta) => ThreadCacheEntry {
            thread_id: thread_id.to_string(),
            delegated: meta.delegated,
            parent_thread_id: meta.parent_thread_id.clone(),
            relationship_to_parent: meta.relationship_to_parent.clone(),
            checked_at_ms: now,
            transient_error: false,
        },
        None => ThreadCacheEntry {
            thread_id: thread_id.to_string(),
            delegated: false,
            parent_thread_id: None,
            relationship_to_parent: None,
            checked_at_ms: now,
            transient_error: true,
        },
    };
    if let Ok(bytes) = serde_json::to_vec(&entry) {
        write_json_atomic(&path, &bytes);
        prune_old_files(cache_dir, CACHE_PRUNE_AGE_MS, now);
    }
    result
}

fn query_real_thread(thread_id: &str) -> Option<T3ThreadMeta> {
    let cli = resolve_t3_cli_path()?;
    let bytes = run_t3_cli(&cli, thread_id, CLI_TIMEOUT)?;
    parse_t3_thread_delegated(&bytes)
}

fn current_env_thread_id() -> Option<String> {
    std::env::var(T3CODE_THREAD_ID_ENV)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| valid_thread_id(s))
}

fn read_marker_in(dir: &Path, kind: AgentKind, session_id: &str, now: u64) -> bool {
    if session_id.trim().is_empty() {
        return false;
    }
    let path = marker_file(dir, kind, session_id);
    let Ok(bytes) = std::fs::read(&path) else {
        return false;
    };
    let Ok(marker) = serde_json::from_slice::<SessionMarker>(&bytes) else {
        return false;
    };
    marker.agent == kind.as_str()
        && marker.session_id == session_id.trim()
        && now.saturating_sub(marker.ts_ms) < MARKER_TTL_MS
}

fn write_marker_in(
    dir: &Path,
    kind: AgentKind,
    session_id: &str,
    thread_id: &str,
    parent_thread_id: Option<String>,
    now: u64,
) {
    if session_id.trim().is_empty() {
        return;
    }
    let path = marker_file(dir, kind, session_id);
    let marker = SessionMarker {
        agent: kind.as_str().to_string(),
        session_id: session_id.trim().to_string(),
        thread_id: thread_id.to_string(),
        parent_thread_id,
        ts_ms: now,
    };
    if let Ok(bytes) = serde_json::to_vec(&marker) {
        write_json_atomic(&path, &bytes);
        prune_old_files(dir, MARKER_TTL_MS, now);
    }
}

fn clear_marker_in(dir: &Path, kind: AgentKind, session_id: &str) {
    if session_id.trim().is_empty() {
        return;
    }
    let _ = std::fs::remove_file(marker_file(dir, kind, session_id));
}

/// Check only the current process environment (`T3CODE_THREAD_ID`). Zero I/O when unset.
pub fn env_delegated_reason() -> Option<&'static str> {
    let thread_id = current_env_thread_id()?;
    let meta = lookup_thread_with(
        &crate::paths::t3_threads_cache_dir(),
        &thread_id,
        now_ms(),
        &query_real_thread,
    )?;
    meta.delegated.then_some(REASON_ENV_DELEGATED)
}

/// Record (or clear) the session-keyed T3 delegated marker from a hook invocation when
/// `T3CODE_THREAD_ID` is present. Returns the silence reason if this session is a T3 delegated
/// child thread (via env or existing marker).
pub fn classify_and_record_hook(kind: AgentKind, session_id: &str) -> Option<&'static str> {
    let now = now_ms();
    let marker_dir = crate::paths::t3_delegated_marker_dir();
    if let Some(thread_id) = current_env_thread_id() {
        if let Some(meta) = lookup_thread_with(
            &crate::paths::t3_threads_cache_dir(),
            &thread_id,
            now,
            &query_real_thread,
        ) {
            if meta.delegated {
                write_marker_in(
                    &marker_dir,
                    kind,
                    session_id,
                    &thread_id,
                    meta.parent_thread_id,
                    now,
                );
                return Some(REASON_ENV_DELEGATED);
            } else {
                clear_marker_in(&marker_dir, kind, session_id);
                return None;
            }
        }
    }
    read_marker_in(&marker_dir, kind, session_id, now).then_some(REASON_MARKER_DELEGATED)
}

/// Check only the session-keyed marker (for daemon failure cards or env-stripped MCP servers).
pub fn marker_silenced(kind: AgentKind, session_id: &str) -> Option<&'static str> {
    read_marker_in(
        &crate::paths::t3_delegated_marker_dir(),
        kind,
        session_id,
        now_ms(),
    )
    .then_some(REASON_MARKER_DELEGATED)
}

/// Emit a structured `askhuman_guard` audit line when a T3 delegated child thread is silenced.
pub fn log_suppressed(
    component: &'static str,
    agent: Option<&str>,
    session_id: Option<&str>,
    tool: Option<&str>,
    reason: &str,
) {
    crate::daemon::lifecycle::log_guard_audit(crate::daemon::lifecycle::GuardAudit {
        component,
        action: "suppressed",
        reason,
        tool,
        agent,
        thread_source: Some("t3_delegated_child"),
        session_id,
        thread_id: current_env_thread_id().as_deref(),
        turn_id: None,
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn parses_parent_and_relationship_without_flagging_top_level_agent_threads() {
        let main_human = br#"{"thread":{"id":"t1","parentThreadId":null,"relationshipToParent":null,"createdBy":"human"}}"#;
        assert_eq!(
            parse_t3_thread_delegated(main_human),
            Some(T3ThreadMeta {
                delegated: false,
                parent_thread_id: None,
                relationship_to_parent: None,
            })
        );

        let top_level_agent = br#"{"thread":{"id":"t2","parentThreadId":null,"relationshipToParent":null,"createdBy":"agent"}}"#;
        assert!(
            !parse_t3_thread_delegated(top_level_agent)
                .unwrap()
                .delegated
        );

        let child_by_parent = br#"{"thread":{"id":"t3","parentThreadId":"t1","relationshipToParent":null,"createdBy":"agent"}}"#;
        let meta = parse_t3_thread_delegated(child_by_parent).unwrap();
        assert!(meta.delegated);
        assert_eq!(meta.parent_thread_id.as_deref(), Some("t1"));

        let child_by_rel = br#"{"thread":{"id":"t4","parentThreadId":null,"relationshipToParent":"delegate","createdBy":"agent"}}"#;
        let meta = parse_t3_thread_delegated(child_by_rel).unwrap();
        assert!(meta.delegated);
        assert_eq!(meta.relationship_to_parent.as_deref(), Some("delegate"));

        assert!(parse_t3_thread_delegated(b"not json").is_none());
    }

    #[test]
    fn caches_conclusive_verdicts_for_24h_and_transient_errors_for_30s() {
        let dir = tempfile::tempdir().unwrap();
        let calls = AtomicUsize::new(0);
        let query_ok = |_: &str| {
            calls.fetch_add(1, Ordering::SeqCst);
            Some(T3ThreadMeta {
                delegated: true,
                parent_thread_id: Some("parent-1".into()),
                relationship_to_parent: None,
            })
        };

        let first = lookup_thread_with(dir.path(), "child-thread-1", 1_000, &query_ok).unwrap();
        assert!(first.delegated);
        assert_eq!(calls.load(Ordering::SeqCst), 1);

        // Cache hit inside 24h TTL does not re-query.
        let second =
            lookup_thread_with(dir.path(), "child-thread-1", 1_000 + 60_000, &query_ok).unwrap();
        assert!(second.delegated);
        assert_eq!(calls.load(Ordering::SeqCst), 1);

        // Cache expires after 24h.
        let third = lookup_thread_with(
            dir.path(),
            "child-thread-1",
            1_000 + CACHE_TTL_MS + 1,
            &query_ok,
        )
        .unwrap();
        assert!(third.delegated);
        assert_eq!(calls.load(Ordering::SeqCst), 2);

        // Transient error caches for 30s and fails open (None).
        let err_calls = AtomicUsize::new(0);
        let query_err = |_: &str| {
            err_calls.fetch_add(1, Ordering::SeqCst);
            None
        };
        assert!(lookup_thread_with(dir.path(), "flaky-1", 5_000, &query_err).is_none());
        assert!(lookup_thread_with(dir.path(), "flaky-1", 15_000, &query_err).is_none());
        assert_eq!(err_calls.load(Ordering::SeqCst), 1);
        assert!(lookup_thread_with(
            dir.path(),
            "flaky-1",
            5_000 + TRANSIENT_ERROR_TTL_MS + 1,
            &query_err
        )
        .is_none());
        assert_eq!(err_calls.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn marker_roundtrip_and_clear_per_agent_session() {
        let dir = tempfile::tempdir().unwrap();
        write_marker_in(
            dir.path(),
            AgentKind::Antigravity,
            "conv-1",
            "t3-child-1",
            Some("t3-parent".into()),
            10_000,
        );
        assert!(read_marker_in(
            dir.path(),
            AgentKind::Antigravity,
            "conv-1",
            20_000
        ));
        assert!(!read_marker_in(
            dir.path(),
            AgentKind::Codex,
            "conv-1",
            20_000
        ));
        assert!(!read_marker_in(
            dir.path(),
            AgentKind::Antigravity,
            "conv-1",
            10_000 + MARKER_TTL_MS + 1
        ));
        clear_marker_in(dir.path(), AgentKind::Antigravity, "conv-1");
        assert!(!read_marker_in(
            dir.path(),
            AgentKind::Antigravity,
            "conv-1",
            20_000
        ));
    }

    #[cfg(unix)]
    #[test]
    fn run_t3_cli_handles_mock_script_and_timeout_fail_open() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();

        let fast = dir.path().join("t3-fast");
        std::fs::write(
            &fast,
            "#!/bin/sh\nprintf '%s' '{\"thread\":{\"id\":\"t-child\",\"parentThreadId\":\"t-root\"}}'\n",
        )
        .unwrap();
        std::fs::set_permissions(&fast, std::fs::Permissions::from_mode(0o755)).unwrap();
        let out = run_t3_cli(&fast, "t-child", Duration::from_secs(2)).unwrap();
        assert!(parse_t3_thread_delegated(&out).unwrap().delegated);

        let slow = dir.path().join("t3-slow");
        std::fs::write(&slow, "#!/bin/sh\nsleep 5\n").unwrap();
        std::fs::set_permissions(&slow, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(run_t3_cli(&slow, "t-slow", Duration::from_millis(80)).is_none());
    }
}
