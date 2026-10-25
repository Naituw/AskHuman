//! Delegated Codex run detection (spec `docs/specs/codex-delegated-run-silence.md`).
//!
//! A *delegated run* is a Codex run that another agent, script, or plugin started without a human
//! attending it: `codex exec` / `e` / `review`, and non-persisted app-server threads. In such a
//! run AskHuman must behave as if it were not integrated (D1), so every AskHuman surface asks this
//! module before it interacts:
//!
//! * SessionStart (the protocol carrier) classifies the run once per session and leaves a marker
//!   file when the run is delegated;
//! * the MCP server, Stop confirmation, and PermissionRequest read that marker (Stop and
//!   permission also fall back to a bounded ancestor probe when the marker is missing);
//! * everything fails open: a missing, unreadable, or conflicting signal means "interactive".
//!
//! Codex does not tell hooks whether a run is delegated (`thread_source` / `originator` are absent
//! from hook inputs, F14), so the verdict is built from the signals below.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::Value;
use sha2::{Digest, Sha256};

use super::detect;

/// Hop budget of the ancestor probe: AskHuman → shell wrapper → (node wrapper) → codex.
const PROBE_MAX_HOPS: usize = 6;
/// Upper bound for reading a rollout's first line (`session_meta` embeds the base instructions and
/// is about 22 KiB in practice).
const ROLLOUT_FIRST_LINE_LIMIT: u64 = 256 * 1024;
/// `originator` values of persisted threads that no human is attending. `codex_exec` is the
/// `codex exec` client; `Claude Code` is the name the official `codex-plugin-cc` initializes its
/// app-server connection with. Deliberately excludes `askhuman` and `Codex Desktop`: AskHuman's own
/// Codex tasks and desktop threads are interactive.
pub const DELEGATED_ORIGINATORS: [&str; 2] = ["codex_exec", "Claude Code"];
/// Markers of finished sessions are removed after this long.
const MARKER_TTL: Duration = Duration::from_secs(14 * 24 * 60 * 60);
/// Pruning scans at most this many directory entries per write.
const MARKER_SCAN_LIMIT: usize = 500;

/// Why a run was judged delegated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Signal {
    /// Hook input carries `transcript_path: null`: an ephemeral / non-persisted thread.
    TranscriptNull,
    /// The nearest Agent ancestor is `codex exec` / `e` / `review`.
    ExecProcess,
    /// The rollout's first line says `source == "exec"`.
    RolloutSource,
    /// The rollout's first line carries an `originator` from [`DELEGATED_ORIGINATORS`].
    RolloutOriginator,
}

impl Signal {
    /// Stable name stored in marker files.
    pub fn name(self) -> &'static str {
        match self {
            Signal::TranscriptNull => "transcript_null",
            Signal::ExecProcess => "exec_process",
            Signal::RolloutSource => "rollout_source",
            Signal::RolloutOriginator => "rollout_originator",
        }
    }

    fn parse(name: &str) -> Option<Self> {
        [
            Signal::TranscriptNull,
            Signal::ExecProcess,
            Signal::RolloutSource,
            Signal::RolloutOriginator,
        ]
        .into_iter()
        .find(|signal| signal.name() == name)
    }

    /// `askhuman_guard` audit reason.
    pub fn reason(self) -> &'static str {
        match self {
            Signal::TranscriptNull => "codex_transcript_path_null",
            Signal::ExecProcess => "codex_delegated_exec_process",
            Signal::RolloutSource => "codex_delegated_rollout_source",
            Signal::RolloutOriginator => "codex_delegated_rollout_originator",
        }
    }
}

/// Audit reason for a run silenced through its marker; keeps the original signal visible.
pub fn marker_reason(signal: Signal) -> String {
    format!("codex_delegated_marker:{}", signal.name())
}

/// Everything detection reads from the outside world, so the decision logic is testable.
pub trait Probes {
    /// The "allow delegated Codex runs to use AskHuman" preference.
    fn allowed(&self) -> bool;
    fn marker(&self, session_id: &str) -> Option<Signal>;
    fn write_marker(&self, session_id: &str, signal: Signal);
    fn clear_marker(&self, session_id: &str);
    /// Whether the nearest Agent ancestor is a non-interactive `codex exec` run.
    fn exec_process(&self) -> bool;
    /// Delegated signal from a rollout's first line, if readable and conclusive.
    fn rollout_signal(&self, transcript_path: &str) -> Option<Signal>;
}

/// Real implementation backed by the preference file, marker directory, process table, and rollout.
pub struct System;

impl Probes for System {
    fn allowed(&self) -> bool {
        crate::integrations::agent_delegated::allowed(super::AgentKind::Codex)
    }

    fn marker(&self, session_id: &str) -> Option<Signal> {
        read_marker(&crate::paths::codex_delegated_marker_dir(), session_id)
    }

    fn write_marker(&self, session_id: &str, signal: Signal) {
        write_marker(
            &crate::paths::codex_delegated_marker_dir(),
            session_id,
            signal,
            now_ms(),
        );
    }

    fn clear_marker(&self, session_id: &str) {
        clear_marker(&crate::paths::codex_delegated_marker_dir(), session_id);
    }

    fn exec_process(&self) -> bool {
        probe_exec_process()
    }

    fn rollout_signal(&self, transcript_path: &str) -> Option<Signal> {
        rollout_signal(Path::new(transcript_path))
    }
}

/// True when the nearest Agent ancestor of this process is `codex exec` / `e` / `review`.
pub fn probe_exec_process() -> bool {
    detect::nearest_agent_invocation(std::process::id(), PROBE_MAX_HOPS)
        .is_some_and(|invocation| invocation.is_delegated_codex_command())
}

// ── Hook input facts ────────────────────────────────────────────────────────────────────────

/// Codex session id from a hook input.
pub fn session_id_of(input: &Value) -> Option<String> {
    ["session_id", "sessionId"]
        .into_iter()
        .filter_map(|key| input.get(key).and_then(Value::as_str))
        .map(str::trim)
        .find(|value| !value.is_empty())
        .map(str::to_string)
}

/// `transcript_path` is present **and null** (an ephemeral thread without a rollout). A missing key
/// is not evidence of anything.
pub fn transcript_path_is_null(input: &Value) -> bool {
    input
        .get("transcript_path")
        .or_else(|| input.get("transcriptPath"))
        .is_some_and(Value::is_null)
}

fn transcript_path_of(input: &Value) -> Option<&str> {
    input
        .get("transcript_path")
        .or_else(|| input.get("transcriptPath"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|path| !path.is_empty())
}

/// SessionStart `source`; Codex omits nothing, but an absent value is treated as `startup`.
pub fn session_start_source(input: &Value) -> String {
    ["source", "session_start_source", "sessionStartSource"]
        .into_iter()
        .filter_map(|key| input.get(key).and_then(Value::as_str))
        .map(|source| source.trim().to_ascii_lowercase())
        .find(|source| !source.is_empty())
        .unwrap_or_else(|| "startup".to_string())
}

// ── Classification ──────────────────────────────────────────────────────────────────────────

/// Signals describing how the process is running *now* (valid for every SessionStart source).
fn current_run_signal(probes: &dyn Probes, input: &Value) -> Option<Signal> {
    if transcript_path_is_null(input) {
        return Some(Signal::TranscriptNull);
    }
    probes.exec_process().then_some(Signal::ExecProcess)
}

/// Classify a run at SessionStart. `None` = interactive (or unknown).
///
/// Rollout signals describe how a thread was *created*, not who is running it now, so they are
/// used only for brand-new threads (`startup` / `clear`): a human resuming an old `codex exec`
/// thread from the TUI (`resume` / `fork`) is interactive. `compact` keeps whatever verdict the
/// session already has.
pub fn classify_session_start(
    probes: &dyn Probes,
    source: &str,
    session_id: &str,
    input: &Value,
) -> Option<Signal> {
    match source {
        "startup" | "clear" => current_run_signal(probes, input)
            .or_else(|| transcript_path_of(input).and_then(|path| probes.rollout_signal(path))),
        "compact" => (!session_id.is_empty())
            .then(|| probes.marker(session_id))
            .flatten()
            .or_else(|| current_run_signal(probes, input)),
        _ => current_run_signal(probes, input),
    }
}

/// Whether Stop / PermissionRequest must stay out of the way. Returns the audit reason.
///
/// Order: preference allows → interactive; marker → delegated; `transcript_path: null` →
/// delegated; bounded ancestor probe → delegated (and the marker is written so later calls are
/// cheap and the MCP guard sees it). The probe makes the exec case work even when the SessionStart
/// hook has not been migrated or trusted yet.
pub fn silenced_with(
    probes: &dyn Probes,
    session_id: &str,
    input: Option<&Value>,
) -> Option<String> {
    if probes.allowed() {
        return None;
    }
    if !session_id.is_empty() {
        if let Some(signal) = probes.marker(session_id) {
            return Some(marker_reason(signal));
        }
    }
    if input.is_some_and(transcript_path_is_null) {
        return Some(Signal::TranscriptNull.reason().to_string());
    }
    if probes.exec_process() {
        if !session_id.is_empty() {
            probes.write_marker(session_id, Signal::ExecProcess);
        }
        return Some(Signal::ExecProcess.reason().to_string());
    }
    None
}

/// [`silenced_with`] against the real environment.
pub fn silenced(session_id: &str, input: Option<&Value>) -> Option<String> {
    silenced_with(&System, session_id, input)
}

/// Marker-only check for the MCP call guard (the exec case is already handled by hiding the tools
/// at startup). Returns the audit reason.
pub fn marker_silenced_with(probes: &dyn Probes, session_id: &str) -> Option<String> {
    if session_id.is_empty() || probes.allowed() {
        return None;
    }
    probes.marker(session_id).map(marker_reason)
}

pub fn marker_silenced(session_id: &str) -> Option<String> {
    marker_silenced_with(&System, session_id)
}

// ── Rollout first line ──────────────────────────────────────────────────────────────────────

/// Delegated signal from the `session_meta` payload of a rollout's first line.
pub fn rollout_signal(path: &Path) -> Option<Signal> {
    let line = read_first_line(path, ROLLOUT_FIRST_LINE_LIMIT)?;
    rollout_signal_from_line(&line)
}

fn rollout_signal_from_line(line: &[u8]) -> Option<Signal> {
    let value: Value = serde_json::from_slice(line).ok()?;
    if value.get("type").and_then(Value::as_str) != Some("session_meta") {
        return None;
    }
    let payload = value.get("payload")?;
    if payload.get("source").and_then(Value::as_str) == Some("exec") {
        return Some(Signal::RolloutSource);
    }
    let originator = payload.get("originator").and_then(Value::as_str)?;
    DELEGATED_ORIGINATORS
        .contains(&originator)
        .then_some(Signal::RolloutOriginator)
}

/// First line without its newline; `None` when the file is unreadable, empty, or the line is
/// longer than `limit`.
fn read_first_line(path: &Path, limit: u64) -> Option<Vec<u8>> {
    use std::io::{BufRead, BufReader, Read};
    let file = std::fs::File::open(path).ok()?;
    let mut reader = BufReader::new(file.take(limit + 1));
    let mut line = Vec::new();
    reader.read_until(b'\n', &mut line).ok()?;
    if line.is_empty() || line.len() as u64 > limit {
        return None;
    }
    if line.last() == Some(&b'\n') {
        line.pop();
    }
    Some(line)
}

// ── Marker store ────────────────────────────────────────────────────────────────────────────

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

/// File stem for a session id: the id itself when it is a short safe token, otherwise a hash.
fn marker_stem(session_id: &str) -> String {
    let safe = !session_id.is_empty()
        && session_id.len() <= 64
        && session_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_');
    if safe {
        return session_id.to_string();
    }
    let digest = Sha256::digest(session_id.as_bytes());
    format!("{digest:x}")[..32].to_string()
}

fn marker_path(dir: &Path, session_id: &str) -> PathBuf {
    dir.join(format!("{}.json", marker_stem(session_id)))
}

/// A readable, well-formed marker means delegated; anything else means unknown.
pub fn read_marker(dir: &Path, session_id: &str) -> Option<Signal> {
    if session_id.is_empty() {
        return None;
    }
    let bytes = std::fs::read(marker_path(dir, session_id)).ok()?;
    let value: Value = serde_json::from_slice(&bytes).ok()?;
    if value.get("v").and_then(Value::as_u64) != Some(1) {
        return None;
    }
    Signal::parse(value.get("signal").and_then(Value::as_str)?)
}

pub fn write_marker(dir: &Path, session_id: &str, signal: Signal, now_ms: u64) {
    if session_id.is_empty() {
        return;
    }
    let body = serde_json::json!({"v": 1, "signal": signal.name(), "ts": now_ms});
    let Ok(bytes) = serde_json::to_vec(&body) else {
        return;
    };
    if crate::integrations::hook_edit::atomic_write(&marker_path(dir, session_id), &bytes).is_ok() {
        prune_markers(dir, SystemTime::now());
    }
}

pub fn clear_marker(dir: &Path, session_id: &str) {
    if !session_id.is_empty() {
        let _ = std::fs::remove_file(marker_path(dir, session_id));
    }
}

/// Remove markers older than [`MARKER_TTL`], scanning a bounded number of entries.
fn prune_markers(dir: &Path, now: SystemTime) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten().take(MARKER_SCAN_LIMIT) {
        let expired = entry
            .metadata()
            .and_then(|metadata| metadata.modified())
            .ok()
            .and_then(|modified| now.duration_since(modified).ok())
            .is_some_and(|age| age > MARKER_TTL);
        if expired {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::cell::{Cell, RefCell};
    use std::collections::HashMap;

    #[derive(Default)]
    struct Fake {
        allowed: bool,
        markers: RefCell<HashMap<String, Signal>>,
        exec: bool,
        exec_probes: Cell<usize>,
        rollout: Option<Signal>,
        rollout_reads: Cell<usize>,
    }

    impl Probes for Fake {
        fn allowed(&self) -> bool {
            self.allowed
        }
        fn marker(&self, session_id: &str) -> Option<Signal> {
            self.markers.borrow().get(session_id).copied()
        }
        fn write_marker(&self, session_id: &str, signal: Signal) {
            self.markers
                .borrow_mut()
                .insert(session_id.to_string(), signal);
        }
        fn clear_marker(&self, session_id: &str) {
            self.markers.borrow_mut().remove(session_id);
        }
        fn exec_process(&self) -> bool {
            self.exec_probes.set(self.exec_probes.get() + 1);
            self.exec
        }
        fn rollout_signal(&self, _transcript_path: &str) -> Option<Signal> {
            self.rollout_reads.set(self.rollout_reads.get() + 1);
            self.rollout
        }
    }

    fn input(transcript: Value) -> Value {
        json!({"session_id": "s1", "transcript_path": transcript, "source": "startup"})
    }

    #[test]
    fn startup_uses_every_signal_in_order() {
        let interactive = Fake::default();
        assert_eq!(
            classify_session_start(&interactive, "startup", "s1", &input(json!("/r.jsonl"))),
            None
        );

        let ephemeral = Fake {
            exec: true,
            rollout: Some(Signal::RolloutSource),
            ..Fake::default()
        };
        assert_eq!(
            classify_session_start(&ephemeral, "startup", "s1", &input(Value::Null)),
            Some(Signal::TranscriptNull),
            "null transcript wins and needs no probe"
        );
        assert_eq!(ephemeral.exec_probes.get(), 0);

        let exec = Fake {
            exec: true,
            rollout: Some(Signal::RolloutSource),
            ..Fake::default()
        };
        assert_eq!(
            classify_session_start(&exec, "startup", "s1", &input(json!("/r.jsonl"))),
            Some(Signal::ExecProcess)
        );
        assert_eq!(exec.rollout_reads.get(), 0, "no rollout read once decided");

        let plugin = Fake {
            rollout: Some(Signal::RolloutOriginator),
            ..Fake::default()
        };
        assert_eq!(
            classify_session_start(&plugin, "clear", "s1", &input(json!("/r.jsonl"))),
            Some(Signal::RolloutOriginator)
        );
    }

    #[test]
    fn resume_and_fork_ignore_how_the_thread_was_created() {
        // A human resuming an old `codex exec` thread from the TUI: rollout says exec, process says
        // interactive.
        let resumed = Fake {
            rollout: Some(Signal::RolloutSource),
            ..Fake::default()
        };
        for source in ["resume", "fork", "something-new"] {
            assert_eq!(
                classify_session_start(&resumed, source, "s1", &input(json!("/r.jsonl"))),
                None,
                "{source}"
            );
        }
        assert_eq!(resumed.rollout_reads.get(), 0);

        // `codex exec resume` is a delegated process even though the thread is old.
        let exec_resume = Fake {
            exec: true,
            ..Fake::default()
        };
        assert_eq!(
            classify_session_start(&exec_resume, "resume", "s1", &input(json!("/r.jsonl"))),
            Some(Signal::ExecProcess)
        );
    }

    #[test]
    fn compact_keeps_the_sessions_existing_verdict() {
        let probes = Fake::default();
        probes.write_marker("s1", Signal::RolloutOriginator);
        assert_eq!(
            classify_session_start(&probes, "compact", "s1", &input(json!("/r.jsonl"))),
            Some(Signal::RolloutOriginator)
        );
        // Without a marker it falls back to the current-run signals, never to the rollout.
        let fresh = Fake {
            rollout: Some(Signal::RolloutSource),
            ..Fake::default()
        };
        assert_eq!(
            classify_session_start(&fresh, "compact", "s1", &input(json!("/r.jsonl"))),
            None
        );
        assert_eq!(fresh.rollout_reads.get(), 0);
    }

    #[test]
    fn a_missing_transcript_key_is_not_a_null_transcript() {
        let probes = Fake::default();
        let missing = json!({"session_id": "s1"});
        assert_eq!(
            classify_session_start(&probes, "startup", "s1", &missing),
            None
        );
        assert!(!transcript_path_is_null(&missing));
        assert!(transcript_path_is_null(&json!({"transcriptPath": null})));
    }

    #[test]
    fn stop_and_permission_check_preference_marker_null_transcript_then_probe() {
        let allowed = Fake {
            allowed: true,
            exec: true,
            ..Fake::default()
        };
        allowed.write_marker("s1", Signal::ExecProcess);
        assert_eq!(
            silenced_with(&allowed, "s1", Some(&input(Value::Null))),
            None,
            "the preference lets delegated runs interact"
        );

        let marked = Fake::default();
        marked.write_marker("s1", Signal::RolloutOriginator);
        assert_eq!(
            silenced_with(&marked, "s1", Some(&input(json!("/r.jsonl")))).as_deref(),
            Some("codex_delegated_marker:rollout_originator")
        );
        assert_eq!(marked.exec_probes.get(), 0);

        let null = Fake::default();
        assert_eq!(
            silenced_with(&null, "s1", Some(&input(Value::Null))).as_deref(),
            Some("codex_transcript_path_null")
        );

        let exec = Fake {
            exec: true,
            ..Fake::default()
        };
        assert_eq!(
            silenced_with(&exec, "s1", Some(&input(json!("/r.jsonl")))).as_deref(),
            Some("codex_delegated_exec_process")
        );
        assert_eq!(
            exec.marker("s1"),
            Some(Signal::ExecProcess),
            "the probe result is remembered"
        );
        assert_eq!(
            silenced_with(&exec, "s1", None).as_deref(),
            Some("codex_delegated_marker:exec_process")
        );

        let interactive = Fake::default();
        assert_eq!(
            silenced_with(&interactive, "s1", Some(&input(json!("/r.jsonl")))),
            None
        );
        assert_eq!(interactive.exec_probes.get(), 1);
    }

    #[test]
    fn an_empty_session_id_never_reads_or_writes_markers() {
        let exec = Fake {
            exec: true,
            ..Fake::default()
        };
        assert_eq!(
            silenced_with(&exec, "", None).as_deref(),
            Some("codex_delegated_exec_process")
        );
        assert!(exec.markers.borrow().is_empty());
        assert_eq!(marker_silenced_with(&exec, ""), None);
    }

    #[test]
    fn mcp_guard_uses_only_the_marker_and_the_preference() {
        let probes = Fake {
            exec: true,
            ..Fake::default()
        };
        assert_eq!(marker_silenced_with(&probes, "s1"), None);
        assert_eq!(probes.exec_probes.get(), 0, "no probe on a tool call");
        probes.write_marker("s1", Signal::TranscriptNull);
        assert_eq!(
            marker_silenced_with(&probes, "s1").as_deref(),
            Some("codex_delegated_marker:transcript_null")
        );
        let allowed = Fake {
            allowed: true,
            ..Fake::default()
        };
        allowed.write_marker("s1", Signal::TranscriptNull);
        assert_eq!(marker_silenced_with(&allowed, "s1"), None);
    }

    #[test]
    fn rollout_first_line_signals() {
        let exec =
            json!({"type":"session_meta","payload":{"source":"exec","originator":"codex_exec"}});
        assert_eq!(
            rollout_signal_from_line(exec.to_string().as_bytes()),
            Some(Signal::RolloutSource)
        );
        let plugin =
            json!({"type":"session_meta","payload":{"source":"vscode","originator":"Claude Code"}});
        assert_eq!(
            rollout_signal_from_line(plugin.to_string().as_bytes()),
            Some(Signal::RolloutOriginator)
        );
        for payload in [
            json!({"source":"vscode","originator":"Codex Desktop"}),
            json!({"source":"vscode","originator":"askhuman"}),
            json!({"source":{"subagent":"review"},"originator":"codex_cli_rs"}),
            json!({"source":"cli"}),
            json!({}),
        ] {
            let line = json!({"type":"session_meta","payload":payload}).to_string();
            assert_eq!(rollout_signal_from_line(line.as_bytes()), None, "{payload}");
        }
        assert_eq!(rollout_signal_from_line(b"not json"), None);
        assert_eq!(
            rollout_signal_from_line(
                json!({"type":"response_item","payload":{"source":"exec"}})
                    .to_string()
                    .as_bytes()
            ),
            None
        );
    }

    #[test]
    fn rollout_first_line_reading_is_bounded() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("r.jsonl");
        std::fs::write(
            &path,
            "{\"type\":\"session_meta\",\"payload\":{\"source\":\"exec\"}}\n{\"second\":1}\n",
        )
        .unwrap();
        assert_eq!(rollout_signal(&path), Some(Signal::RolloutSource));

        let long = dir.path().join("long.jsonl");
        std::fs::write(&long, "x".repeat(ROLLOUT_FIRST_LINE_LIMIT as usize + 10)).unwrap();
        assert_eq!(read_first_line(&long, ROLLOUT_FIRST_LINE_LIMIT), None);
        assert_eq!(rollout_signal(&long), None);
        assert_eq!(rollout_signal(&dir.path().join("missing.jsonl")), None);
        let empty = dir.path().join("empty.jsonl");
        std::fs::write(&empty, "").unwrap();
        assert_eq!(rollout_signal(&empty), None);
    }

    #[test]
    fn marker_round_trip_clear_and_corruption() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(read_marker(dir.path(), "s1"), None);
        write_marker(dir.path(), "s1", Signal::ExecProcess, 1);
        assert_eq!(read_marker(dir.path(), "s1"), Some(Signal::ExecProcess));
        write_marker(dir.path(), "s1", Signal::TranscriptNull, 2);
        assert_eq!(read_marker(dir.path(), "s1"), Some(Signal::TranscriptNull));
        clear_marker(dir.path(), "s1");
        assert_eq!(read_marker(dir.path(), "s1"), None);

        std::fs::write(marker_path(dir.path(), "bad"), "{not json").unwrap();
        assert_eq!(read_marker(dir.path(), "bad"), None);
        std::fs::write(
            marker_path(dir.path(), "future"),
            r#"{"v":2,"signal":"exec_process"}"#,
        )
        .unwrap();
        assert_eq!(read_marker(dir.path(), "future"), None);
        std::fs::write(
            marker_path(dir.path(), "unknown"),
            r#"{"v":1,"signal":"mystery"}"#,
        )
        .unwrap();
        assert_eq!(read_marker(dir.path(), "unknown"), None);
    }

    #[test]
    fn marker_names_are_sanitized_and_cannot_escape_the_directory() {
        let dir = Path::new("/state/codex-delegated");
        assert_eq!(marker_path(dir, "019c-AbC_1"), dir.join("019c-AbC_1.json"));
        for hostile in [
            "../../etc/passwd",
            "a/b",
            "a\\b",
            "x y",
            "é",
            &"a".repeat(65),
        ] {
            let path = marker_path(dir, hostile);
            assert_eq!(path.parent(), Some(dir), "{hostile}");
            let stem = path.file_stem().unwrap().to_string_lossy().to_string();
            assert_eq!(stem.len(), 32, "{hostile}");
            assert!(stem.bytes().all(|byte| byte.is_ascii_hexdigit()));
        }
        assert_ne!(marker_path(dir, "a/b"), marker_path(dir, "a\\b"));
    }

    #[test]
    fn old_markers_are_pruned_on_write_and_fresh_ones_survive() {
        let dir = tempfile::tempdir().unwrap();
        write_marker(dir.path(), "old", Signal::ExecProcess, 1);
        write_marker(dir.path(), "fresh", Signal::ExecProcess, 1);
        let old = std::fs::OpenOptions::new()
            .write(true)
            .open(marker_path(dir.path(), "old"))
            .unwrap();
        old.set_modified(SystemTime::now() - MARKER_TTL - Duration::from_secs(60))
            .unwrap();
        drop(old);

        // The next write prunes the expired marker and keeps the recent ones.
        write_marker(dir.path(), "after", Signal::TranscriptNull, 2);
        assert_eq!(read_marker(dir.path(), "old"), None);
        assert_eq!(read_marker(dir.path(), "fresh"), Some(Signal::ExecProcess));
        assert_eq!(
            read_marker(dir.path(), "after"),
            Some(Signal::TranscriptNull)
        );
    }

    #[test]
    fn session_start_source_defaults_to_startup() {
        assert_eq!(session_start_source(&json!({})), "startup");
        assert_eq!(
            session_start_source(&json!({"source": " Compact "})),
            "compact"
        );
        assert_eq!(
            session_start_source(&json!({"sessionStartSource": "resume"})),
            "resume"
        );
        assert_eq!(session_start_source(&json!({"source": ""})), "startup");
    }

    #[test]
    fn session_id_accepts_both_spellings() {
        assert_eq!(
            session_id_of(&json!({"session_id": " a "})).as_deref(),
            Some("a")
        );
        assert_eq!(
            session_id_of(&json!({"sessionId": "b"})).as_deref(),
            Some("b")
        );
        assert_eq!(session_id_of(&json!({"session_id": ""})), None);
        assert_eq!(session_id_of(&json!({})), None);
    }
}
