//! Read-only tail of Cursor Desktop's structured extension logs.
//!
//! When Cursor gives up on a turn after exhausting its retries it sends neither a `stop` nor a
//! `sessionEnd` hook, so the lifecycle registry never learns that the turn failed. The only
//! first-party record of that outcome is the `agent.turn.outcome` line the always-local extension
//! appends to its structured log. This module only **reads** those files: it never opens them for
//! writing, never reads Cursor's databases and never talks to Cursor, so it cannot start or
//! influence a request. See `docs/specs/agent-failure-inbox.md` §4.

use std::collections::HashMap;
use std::fs::{self, File, Metadata};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::Value;

use crate::i18n::Lang;

/// Structured message that closes a Cursor agent turn.
const OUTCOME_MESSAGE: &str = "agent.turn.outcome";
/// Environment override of the log root (tests and diagnostics). Only ever read from.
pub const ROOT_ENV: &str = "ASKHUMAN_CURSOR_LOG_ROOT";
/// Every Cursor extension log of interest starts with this name.
const FILE_PREFIX: &str = "Cursor Structured Logs";
/// Only the newest launches can belong to a running Cursor.
const LAUNCHES: usize = 2;
/// Upper bound of tracked files, newest first.
const MAX_FILES: usize = 32;
/// Bytes read from one file per poll; a larger backlog is caught up over the next polls.
const MAX_READ: u64 = 1 << 20;
/// Longer lines are skipped unread (model lists and similar payloads, never an outcome).
const MAX_LINE: usize = 64 * 1024;
/// Bound of Cursor's own error text kept for display.
const MAX_TEXT_CHARS: usize = 320;
/// Files that appear later are discovered within this interval.
const DISCOVERY_EVERY: Duration = Duration::from_secs(10);
/// A changed log format is reported at most this often.
const DRIFT_REPORT_EVERY: Duration = Duration::from_secs(3600);

/// Where Cursor keeps its logs, or `None` when this platform is not supported.
pub fn default_root() -> Option<PathBuf> {
    root_from(std::env::var_os(ROOT_ENV))
}

fn root_from(overridden: Option<std::ffi::OsString>) -> Option<PathBuf> {
    match overridden.filter(|root| !root.is_empty()) {
        Some(root) => Some(PathBuf::from(root)),
        None => platform_root(),
    }
}

#[cfg(target_os = "macos")]
fn platform_root() -> Option<PathBuf> {
    dirs::config_dir().map(|dir| dir.join("Cursor").join("logs"))
}

#[cfg(not(target_os = "macos"))]
fn platform_root() -> Option<PathBuf> {
    None
}

/// One turn that ended in error, as Cursor recorded it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failure {
    /// Cursor's conversation id, which is the lifecycle session id.
    pub conversation_id: String,
    /// Cursor's request id, which is the hook `generation_id` and so the registry turn id.
    pub request_id: String,
    /// Cursor's own error text, stripped of control characters and bounded.
    pub text: String,
    pub retries: Option<u32>,
}

impl Failure {
    /// The reason shown with the failure card and the Agents page.
    pub fn summary(&self, lang: Lang) -> Option<String> {
        let note = self
            .retries
            .filter(|retries| *retries > 0)
            .map(|retries| match lang {
                Lang::Zh => format!("Cursor 已重试 {retries} 次。"),
                Lang::En => format!("Cursor retried {retries} time(s)."),
            });
        let text = if self.text.is_empty() {
            note.as_ref().map(|_| match lang {
                Lang::Zh => "Cursor 在重试后放弃了本轮。".to_string(),
                Lang::En => "Cursor gave up on this turn after retrying.".to_string(),
            })
        } else {
            Some(self.text.clone())
        };
        match (text, note) {
            (Some(text), Some(note)) => Some(format!("{text}\n{note}")),
            (text, _) => text,
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Line {
    /// A turn that ended in error.
    Failure(Failure),
    /// Anything else: another message, or an outcome that is not an error.
    Other,
    /// Names the outcome message but lacks what this reader depends on: the format may have
    /// changed.
    Malformed,
}

/// Classify one log line. Pure; the line may carry a `<time> [level]` prefix before the JSON.
pub fn parse_line(line: &str) -> Line {
    if !line.contains(OUTCOME_MESSAGE) {
        return Line::Other;
    }
    let Some(start) = line.find('{') else {
        return Line::Malformed;
    };
    let Ok(value) = serde_json::from_str::<Value>(line[start..].trim()) else {
        return Line::Malformed;
    };
    if value["message"] != OUTCOME_MESSAGE {
        return Line::Other;
    }
    let Some(meta) = value["metadata"].as_object() else {
        return Line::Malformed;
    };
    let Some(outcome) = meta.get("outcome").and_then(Value::as_str) else {
        return Line::Malformed;
    };
    if outcome != "error" {
        return Line::Other;
    }
    let id = |key: &str| {
        meta.get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|id| {
                !id.is_empty()
                    && id.len() <= 128
                    && id
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
            })
            .map(str::to_string)
    };
    let (Some(conversation_id), Some(request_id)) = (id("conversation_id"), id("request_id"))
    else {
        return Line::Malformed;
    };
    let text: String = meta
        .get("error_text")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .chars()
        .filter(|c| !c.is_control() || *c == '\n')
        .collect::<String>()
        .trim()
        .chars()
        .take(MAX_TEXT_CHARS)
        .collect();
    // Cursor writes every metadata value as a string.
    let retries = match meta.get("retries_attempted") {
        Some(Value::String(count)) => count.trim().parse().ok(),
        Some(Value::Number(count)) => count.as_u64().and_then(|count| u32::try_from(count).ok()),
        _ => None,
    };
    Line::Failure(Failure {
        conversation_id,
        request_id,
        text,
        retries,
    })
}

/// Follows the structured logs of the running Cursor. Files that exist when they are first seen are
/// positioned at their end: only lines written while this reader runs matter.
pub struct Tailer {
    root: PathBuf,
    files: HashMap<PathBuf, Position>,
    discovered: Option<Instant>,
    discovery_every: Duration,
    malformed: u64,
    reported: Option<Instant>,
}

struct Position {
    id: Option<u64>,
    offset: u64,
    /// The rest of an over-long (or half-seen) line is being skipped up to its newline.
    discarding: bool,
}

impl Tailer {
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            files: HashMap::new(),
            discovered: None,
            discovery_every: DISCOVERY_EVERY,
            malformed: 0,
            reported: None,
        }
    }

    /// Complete lines appended since the previous poll, reduced to the failed turns among them.
    pub fn poll(&mut self, now: Instant) -> Vec<Failure> {
        if self
            .discovered
            .is_none_or(|at| now.saturating_duration_since(at) >= self.discovery_every)
        {
            self.discover(now);
        }
        let mut failures = Vec::new();
        for (path, position) in self.files.iter_mut() {
            for line in read_new(path, position) {
                match parse_line(&line) {
                    Line::Failure(failure) => failures.push(failure),
                    Line::Malformed => self.malformed += 1,
                    Line::Other => {}
                }
            }
        }
        failures
    }

    /// How many outcome lines could not be understood since the last report; at most hourly.
    pub fn take_malformed(&mut self, now: Instant) -> Option<u64> {
        if self.malformed == 0
            || self
                .reported
                .is_some_and(|at| now.saturating_duration_since(at) < DRIFT_REPORT_EVERY)
        {
            return None;
        }
        self.reported = Some(now);
        Some(std::mem::take(&mut self.malformed))
    }

    fn discover(&mut self, now: Instant) {
        self.discovered = Some(now);
        let found = structured_log_files(&self.root);
        for path in &found {
            if self.files.contains_key(path) {
                continue;
            }
            if let Some(position) = Position::at_end(path) {
                self.files.insert(path.clone(), position);
            }
        }
        self.files.retain(|path, _| found.contains(path));
    }
}

impl Position {
    fn at_end(path: &Path) -> Option<Self> {
        let mut file = File::open(path).ok()?;
        let meta = file.metadata().ok()?;
        let length = meta.len();
        let mut mid_line = false;
        if length > 0 && file.seek(SeekFrom::Start(length - 1)).is_ok() {
            let mut last = [0u8; 1];
            mid_line = file.read_exact(&mut last).is_ok() && last[0] != b'\n';
        }
        Some(Self {
            id: file_id(&meta),
            offset: length,
            discarding: mid_line,
        })
    }
}

/// New complete lines of `path`, advancing `position`. An unreadable file yields nothing and
/// keeps its position.
fn read_new(path: &Path, position: &mut Position) -> Vec<String> {
    let Ok(mut file) = File::open(path) else {
        return Vec::new();
    };
    let Ok(meta) = file.metadata() else {
        return Vec::new();
    };
    let id = file_id(&meta);
    // A shorter file, or another file under the same name, is a rotation: start over.
    let rotated = meta.len() < position.offset
        || position
            .id
            .zip(id)
            .is_some_and(|(known, current)| known != current);
    if rotated {
        *position = Position {
            id,
            offset: 0,
            discarding: false,
        };
    }
    position.id = id;
    if meta.len() == position.offset || file.seek(SeekFrom::Start(position.offset)).is_err() {
        return Vec::new();
    }
    let mut buffer = Vec::new();
    if file.take(MAX_READ).read_to_end(&mut buffer).is_err() {
        return Vec::new();
    }
    let mut lines = Vec::new();
    let mut start = 0;
    while start < buffer.len() {
        let Some(length) = buffer[start..].iter().position(|byte| *byte == b'\n') else {
            // No newline yet: the writer is mid-line, or the line is too long to keep.
            if position.discarding || buffer.len() - start > MAX_LINE {
                position.discarding = true;
                start = buffer.len();
            }
            break;
        };
        if position.discarding {
            position.discarding = false;
        } else if length <= MAX_LINE {
            lines.push(
                String::from_utf8_lossy(&buffer[start..start + length])
                    .trim_end()
                    .to_string(),
            );
        }
        start += length + 1;
    }
    position.offset += start as u64;
    lines
}

#[cfg(unix)]
fn file_id(meta: &Metadata) -> Option<u64> {
    use std::os::unix::fs::MetadataExt;
    Some(meta.ino())
}

#[cfg(not(unix))]
fn file_id(_: &Metadata) -> Option<u64> {
    None
}

fn names(dir: &Path) -> Vec<(String, PathBuf)> {
    fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| Some((entry.file_name().into_string().ok()?, entry.path())))
        .collect()
}

/// `<root>/<launch>/window*/exthost/anysphere.cursor-always-local/Cursor Structured Logs*.log`
/// of the newest launches, most recently written first.
fn structured_log_files(root: &Path) -> Vec<PathBuf> {
    let mut launches: Vec<(String, PathBuf)> = names(root)
        .into_iter()
        .filter(|(name, _)| name.starts_with(|c: char| c.is_ascii_digit()))
        .collect();
    launches.sort_by(|a, b| b.0.cmp(&a.0));
    launches.truncate(LAUNCHES);
    let mut files: Vec<(SystemTime, PathBuf)> = Vec::new();
    for (_, launch) in launches {
        for (name, window) in names(&launch) {
            if !name.starts_with("window") {
                continue;
            }
            let dir = window.join("exthost").join("anysphere.cursor-always-local");
            for (name, path) in names(&dir) {
                if name.starts_with(FILE_PREFIX) && name.ends_with(".log") {
                    let modified = fs::metadata(&path)
                        .and_then(|meta| meta.modified())
                        .unwrap_or(UNIX_EPOCH);
                    files.push((modified, path));
                }
            }
        }
    }
    files.sort_by_key(|(modified, _)| std::cmp::Reverse(*modified));
    files.truncate(MAX_FILES);
    files.into_iter().map(|(_, path)| path).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::io::Write;

    const CONVERSATION: &str = "11111111-2222-4333-8444-555555555555";
    const REQUEST: &str = "66666666-7777-4888-9999-000000000000";

    /// Shaped like a real outcome line: a `<time> [level]` prefix and string-valued metadata.
    fn line(outcome: &str, conversation: &str, request: &str, text: &str, retries: &str) -> String {
        format!(
            "2026-11-03 05:47:50.114 [info] {}",
            json!({
                "level": "info",
                "key": "abcd1234",
                "message": OUTCOME_MESSAGE,
                "metadata": {
                    "turn_type": "new",
                    "request_id": request,
                    "conversation_id": conversation,
                    "outcome": outcome,
                    "error_type": "transient",
                    "error_code": "unknown",
                    "error_text": text,
                    "turn_number": "2",
                    "retries_attempted": retries,
                    "duration_ms": "1234.5",
                }
            })
        )
    }

    fn error_line(text: &str) -> String {
        line("error", CONVERSATION, REQUEST, text, "3")
    }

    fn truncated_after_message() -> String {
        let whole = error_line("x");
        let cut = whole.find("\"metadata\"").unwrap() + 12;
        whole[..cut].to_string()
    }

    fn failure(text: &str) -> Failure {
        Failure {
            conversation_id: CONVERSATION.into(),
            request_id: REQUEST.into(),
            text: text.into(),
            retries: Some(3),
        }
    }

    #[test]
    fn an_error_outcome_names_the_session_the_turn_and_what_cursor_said() {
        assert_eq!(
            parse_line(&error_line("Connection failed repeatedly")),
            Line::Failure(failure("Connection failed repeatedly"))
        );
        // The prefix is not part of the contract; bare JSON works too.
        let bare = error_line("x").split_once("[info] ").unwrap().1.to_string();
        assert_eq!(parse_line(&bare), Line::Failure(failure("x")));
    }

    #[test]
    fn only_error_outcomes_of_the_outcome_message_are_failures() {
        for outcome in ["success", "cancelled", "Error", ""] {
            assert_eq!(
                parse_line(&line(outcome, CONVERSATION, REQUEST, "t", "0")),
                Line::Other,
                "{outcome}"
            );
        }
        assert_eq!(
            parse_line("2026-11-03 05:47:50.114 [info] {\"message\":\"model refresh\"}"),
            Line::Other
        );
        // Mentioning the name elsewhere does not make a line an outcome.
        let other =
            json!({"message": "x", "metadata": {"note": OUTCOME_MESSAGE, "outcome": "error"}});
        assert_eq!(parse_line(&other.to_string()), Line::Other);
        assert_eq!(parse_line(""), Line::Other);
    }

    #[test]
    fn an_outcome_line_this_reader_cannot_use_is_malformed_not_a_failure() {
        let broken = [
            // No JSON at all.
            format!("[info] {OUTCOME_MESSAGE} failed"),
            // Truncated JSON that still names the message.
            truncated_after_message(),
            // The metadata moved.
            json!({"message": OUTCOME_MESSAGE, "outcome": "error"}).to_string(),
            // The outcome key moved.
            json!({"message": OUTCOME_MESSAGE, "metadata": {"result": "error"}}).to_string(),
            // The ids are missing, empty or not identifier-shaped.
            line("error", "", REQUEST, "t", "1"),
            line("error", CONVERSATION, "", "t", "1"),
            line("error", "../../etc", REQUEST, "t", "1"),
            line("error", CONVERSATION, &"a".repeat(129), "t", "1"),
            json!({"message": OUTCOME_MESSAGE, "metadata": {"outcome": "error"}}).to_string(),
        ];
        for line in broken {
            assert_eq!(parse_line(&line), Line::Malformed, "{line}");
        }
    }

    #[test]
    fn text_and_retries_are_cleaned_and_bounded() {
        let long = "字".repeat(1000);
        let Line::Failure(parsed) = parse_line(&line(
            "error",
            CONVERSATION,
            REQUEST,
            &format!(" \u{7}bell\r\nsecond\u{1b}[31m{long}"),
            "x",
        )) else {
            panic!("expected a failure");
        };
        assert_eq!(parsed.retries, None);
        assert_eq!(parsed.text.chars().count(), MAX_TEXT_CHARS);
        assert!(parsed.text.starts_with("bell\nsecond[31m"));
        assert!(!parsed.text.contains(|c: char| c.is_control() && c != '\n'));
        // A numeric count is understood as well.
        let numeric = json!({"message": OUTCOME_MESSAGE, "metadata": {
            "outcome": "error", "conversation_id": CONVERSATION, "request_id": REQUEST,
            "retries_attempted": 7}});
        let Line::Failure(parsed) = parse_line(&numeric.to_string()) else {
            panic!("expected a failure");
        };
        assert_eq!((parsed.text.as_str(), parsed.retries), ("", Some(7)));
    }

    #[test]
    fn the_summary_reads_naturally_with_or_without_cursors_text_and_retries() {
        let mut failure = failure("Connection failed repeatedly");
        assert_eq!(
            failure.summary(Lang::En).as_deref(),
            Some("Connection failed repeatedly\nCursor retried 3 time(s).")
        );
        assert_eq!(
            failure.summary(Lang::Zh).as_deref(),
            Some("Connection failed repeatedly\nCursor 已重试 3 次。")
        );
        failure.retries = Some(0);
        assert_eq!(
            failure.summary(Lang::En).as_deref(),
            Some("Connection failed repeatedly")
        );
        failure.text.clear();
        assert_eq!(failure.summary(Lang::En), None);
        failure.retries = Some(2);
        assert_eq!(
            failure.summary(Lang::En).as_deref(),
            Some("Cursor gave up on this turn after retrying.\nCursor retried 2 time(s).")
        );
    }

    /// `<root>/<launch>/window1_wbN/exthost/anysphere.cursor-always-local/`.
    fn window(root: &Path, launch: &str, window: &str) -> PathBuf {
        let dir = root
            .join(launch)
            .join(window)
            .join("exthost")
            .join("anysphere.cursor-always-local");
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn append(path: &Path, text: &str) {
        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .unwrap();
        file.write_all(text.as_bytes()).unwrap();
    }

    fn eager(root: &Path) -> Tailer {
        let mut tailer = Tailer::new(root.to_path_buf());
        tailer.discovery_every = Duration::ZERO;
        tailer
    }

    fn texts(failures: Vec<Failure>) -> Vec<String> {
        failures.into_iter().map(|failure| failure.text).collect()
    }

    #[test]
    fn history_is_never_replayed_and_appended_failures_are_reported_once() {
        let root = tempfile::tempdir().unwrap();
        let log = window(root.path(), "20261102T221046", "window1_wb0")
            .join("Cursor Structured Logs.20261102T221050_c788447b.log");
        append(&log, &format!("{}\n", error_line("history")));
        let mut tailer = eager(root.path());
        assert!(tailer.poll(Instant::now()).is_empty());
        append(&log, &format!("unrelated\n{}\n", error_line("first")));
        assert_eq!(texts(tailer.poll(Instant::now())), ["first"]);
        assert!(tailer.poll(Instant::now()).is_empty());
        append(
            &log,
            &format!(
                "{}\n{}\n",
                line("success", CONVERSATION, REQUEST, "", "0"),
                error_line("second")
            ),
        );
        assert_eq!(texts(tailer.poll(Instant::now())), ["second"]);
    }

    #[test]
    fn a_line_still_being_written_waits_for_its_newline() {
        let root = tempfile::tempdir().unwrap();
        let log = window(root.path(), "20261102T221046", "window1_wb0")
            .join("Cursor Structured Logs.log");
        append(&log, "");
        let mut tailer = eager(root.path());
        tailer.poll(Instant::now());
        let whole = error_line("split");
        let (head, tail) = whole.split_at(whole.len() / 2);
        append(&log, head);
        assert!(tailer.poll(Instant::now()).is_empty());
        assert!(tailer.poll(Instant::now()).is_empty());
        append(&log, &format!("{tail}\n"));
        assert_eq!(texts(tailer.poll(Instant::now())), ["split"]);
    }

    #[test]
    fn a_file_found_in_the_middle_of_a_line_skips_the_rest_of_that_line() {
        let root = tempfile::tempdir().unwrap();
        let log = window(root.path(), "20261102T221046", "window1_wb0")
            .join("Cursor Structured Logs.log");
        let whole = error_line("cut");
        append(&log, &format!("{}\n{}", error_line("whole"), &whole[..40]));
        let mut tailer = eager(root.path());
        tailer.poll(Instant::now());
        append(&log, &format!("{}\n{}\n", &whole[40..], error_line("next")));
        assert_eq!(texts(tailer.poll(Instant::now())), ["next"]);
        assert_eq!(tailer.take_malformed(Instant::now()), None);
    }

    #[test]
    fn rotation_and_truncation_restart_from_the_new_files_beginning() {
        let root = tempfile::tempdir().unwrap();
        let dir = window(root.path(), "20261102T221046", "window1_wb0");
        let log = dir.join("Cursor Structured Logs.log");
        append(&log, &format!("{}\n", error_line("old and long enough")));
        let mut tailer = eager(root.path());
        tailer.poll(Instant::now());
        // Cursor renames the full file and starts a fresh one under the same name.
        fs::rename(&log, dir.join("Cursor Structured Logs.1.log")).unwrap();
        append(&log, &format!("{}\n", error_line("a")));
        assert_eq!(texts(tailer.poll(Instant::now())), ["a"]);
        // The same file starting over (shorter than what was read) is a rotation too.
        fs::write(&log, "").unwrap();
        assert!(tailer.poll(Instant::now()).is_empty());
        append(&log, &format!("{}\n", error_line("b")));
        assert_eq!(texts(tailer.poll(Instant::now())), ["b"]);
    }

    #[cfg(unix)]
    #[test]
    fn a_replacement_that_is_longer_than_the_old_offset_is_detected_by_identity() {
        let root = tempfile::tempdir().unwrap();
        let dir = window(root.path(), "20261102T221046", "window1_wb0");
        let log = dir.join("Cursor Structured Logs.log");
        append(&log, &format!("{}\n", error_line("x")));
        let mut tailer = eager(root.path());
        tailer.poll(Instant::now());
        fs::rename(&log, dir.join("Cursor Structured Logs.1.log")).unwrap();
        append(
            &log,
            &format!(
                "{}\n{}\n",
                error_line("longer one"),
                error_line("longer two")
            ),
        );
        assert_eq!(
            texts(tailer.poll(Instant::now())),
            ["longer one", "longer two"]
        );
    }

    #[test]
    fn a_new_window_is_picked_up_without_replaying_its_first_lines() {
        let root = tempfile::tempdir().unwrap();
        let first = window(root.path(), "20261102T221046", "window1_wb0")
            .join("Cursor Structured Logs.log");
        append(&first, "");
        let mut tailer = eager(root.path());
        tailer.poll(Instant::now());
        let second = window(root.path(), "20261102T221046", "window1_wb1")
            .join("Cursor Structured Logs.workspaceId-abc.log");
        append(&second, &format!("{}\n", error_line("before discovery")));
        assert!(tailer.poll(Instant::now()).is_empty());
        append(&second, &format!("{}\n", error_line("after discovery")));
        assert_eq!(texts(tailer.poll(Instant::now())), ["after discovery"]);
    }

    #[test]
    fn only_the_newest_launches_and_the_structured_logs_are_followed() {
        let root = tempfile::tempdir().unwrap();
        let old = window(root.path(), "20261001T000000", "window1_wb0");
        let previous = window(root.path(), "20261101T000000", "window1_wb0");
        let current = window(root.path(), "20261102T000000", "window1_wb0");
        for dir in [&old, &previous, &current] {
            append(&dir.join("Cursor Structured Logs.log"), "");
            append(&dir.join("Cursor Tab.log"), "");
        }
        append(&root.path().join("not-a-launch"), "");
        let found = structured_log_files(root.path());
        assert_eq!(found.len(), 2, "{found:?}");
        assert!(found.iter().all(|path| path.starts_with(root.path())));
        assert!(!found
            .iter()
            .any(|path| path.starts_with(root.path().join("20261001T000000"))));
        let mut tailer = eager(root.path());
        tailer.poll(Instant::now());
        append(
            &old.join("Cursor Structured Logs.log"),
            &format!("{}\n", error_line("too old")),
        );
        append(
            &previous.join("Cursor Tab.log"),
            &format!("{}\n", error_line("wrong file")),
        );
        append(
            &previous.join("Cursor Structured Logs.log"),
            &format!("{}\n", error_line("previous launch")),
        );
        assert_eq!(texts(tailer.poll(Instant::now())), ["previous launch"]);
    }

    #[test]
    fn a_missing_root_or_vanished_file_is_quiet() {
        let root = tempfile::tempdir().unwrap();
        let mut tailer = eager(&root.path().join("absent"));
        assert!(tailer.poll(Instant::now()).is_empty());
        let log = window(root.path(), "20261102T221046", "window1_wb0")
            .join("Cursor Structured Logs.log");
        append(&log, "");
        let mut tailer = eager(root.path());
        tailer.poll(Instant::now());
        fs::remove_file(&log).unwrap();
        assert!(tailer.poll(Instant::now()).is_empty());
        assert!(tailer.files.is_empty());
    }

    #[test]
    fn over_long_lines_are_skipped_without_losing_the_lines_after_them() {
        let root = tempfile::tempdir().unwrap();
        let log = window(root.path(), "20261102T221046", "window1_wb0")
            .join("Cursor Structured Logs.log");
        append(&log, "");
        let mut tailer = eager(root.path());
        tailer.poll(Instant::now());
        // Longer than a line may be, delivered in two parts around a poll.
        append(&log, &"x".repeat(MAX_LINE + 10));
        assert!(tailer.poll(Instant::now()).is_empty());
        append(
            &log,
            &format!("{}\n{}\n", "y".repeat(100), error_line("kept")),
        );
        assert_eq!(texts(tailer.poll(Instant::now())), ["kept"]);
        // A complete over-long line followed by a good one in the same read.
        append(
            &log,
            &format!(
                "{}\n{}\n",
                "z".repeat(MAX_LINE + 1),
                error_line("also kept")
            ),
        );
        assert_eq!(texts(tailer.poll(Instant::now())), ["also kept"]);
    }

    #[test]
    fn a_large_backlog_is_caught_up_over_several_polls() {
        let root = tempfile::tempdir().unwrap();
        let log = window(root.path(), "20261102T221046", "window1_wb0")
            .join("Cursor Structured Logs.log");
        append(&log, "");
        let mut tailer = eager(root.path());
        tailer.poll(Instant::now());
        let filler = format!("{}\n", "f".repeat(1000));
        let mut text = filler.repeat(1500);
        text.push_str(&format!("{}\n", error_line("at the very end")));
        append(&log, &text);
        let mut seen = Vec::new();
        for _ in 0..4 {
            seen.extend(texts(tailer.poll(Instant::now())));
        }
        assert_eq!(seen, ["at the very end"]);
    }

    #[test]
    fn format_drift_is_counted_and_reported_at_most_hourly() {
        let root = tempfile::tempdir().unwrap();
        let log = window(root.path(), "20261102T221046", "window1_wb0")
            .join("Cursor Structured Logs.log");
        append(&log, "");
        let mut tailer = eager(root.path());
        let start = Instant::now();
        tailer.poll(start);
        append(
            &log,
            &format!("{OUTCOME_MESSAGE} but not json\n{OUTCOME_MESSAGE} {{broken\n"),
        );
        assert!(tailer.poll(start).is_empty());
        assert_eq!(tailer.take_malformed(start), Some(2));
        append(&log, &format!("{OUTCOME_MESSAGE} again\n"));
        tailer.poll(start);
        assert_eq!(tailer.take_malformed(start + Duration::from_secs(60)), None);
        assert_eq!(tailer.take_malformed(start + DRIFT_REPORT_EVERY), Some(1));
        assert_eq!(tailer.take_malformed(start + DRIFT_REPORT_EVERY * 3), None);
    }

    #[cfg(unix)]
    #[test]
    fn an_unreadable_file_is_skipped_and_resumes_when_readable_again() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().unwrap();
        let log = window(root.path(), "20261102T221046", "window1_wb0")
            .join("Cursor Structured Logs.log");
        append(&log, "");
        let mut tailer = eager(root.path());
        tailer.poll(Instant::now());
        append(&log, &format!("{}\n", error_line("while locked")));
        fs::set_permissions(&log, fs::Permissions::from_mode(0o000)).unwrap();
        let locked = tailer.poll(Instant::now());
        fs::set_permissions(&log, fs::Permissions::from_mode(0o644)).unwrap();
        // Root can read anything, so only the second half is asserted unconditionally.
        let unlocked = tailer.poll(Instant::now());
        let all: Vec<String> = texts(locked).into_iter().chain(texts(unlocked)).collect();
        assert_eq!(all, ["while locked"]);
    }

    #[test]
    fn the_root_can_be_overridden_for_tests_and_diagnostics() {
        assert_eq!(
            root_from(Some("/tmp/cursor-logs".into())),
            Some(PathBuf::from("/tmp/cursor-logs"))
        );
        // An empty override is no override.
        assert_eq!(root_from(Some("".into())), platform_root());
        assert_eq!(root_from(None), platform_root());
        assert_eq!(platform_root().is_some(), cfg!(target_os = "macos"));
    }
}
