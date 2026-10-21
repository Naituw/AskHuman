//! Files dropped onto an answer popup: a read that says why it failed, and the diagnostic log of each drag.
//!
//! A drop that added nothing used to leave no trace: the page wrote to a console nobody reads, and the Popup Host's
//! stderr is /dev/null. The popup now tells the user what became of the files (docs/specs/popup-inbox-collapse.md
//! §6.9) and hands every drag to `popup_drop_log`, which appends one line to `daemon.log`. Like `log_runtime_event`,
//! a line holds fixed labels, counts and an opaque request id only: the paths the page passes in are classed by
//! place and counted, never written, and a label the page invents does not deserialize.

use std::collections::BTreeMap;
use std::path::{Component, Path};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

/// The most failed files one line names; the counts are exact either way.
const MAX_FAILED: usize = 20;
/// The most dragged paths classed by place for one line.
const MAX_PATHS: usize = 10_000;
/// The longest request id taken as opaque: a longer one, or one with odd characters, is left out of the line.
const MAX_ID_LEN: usize = 64;

/// Where a file lives, as far as the system treats the place specially: macOS asks the user before an app may read
/// the Desktop, Documents and Downloads folders, iCloud Drive, and removable or network volumes. A label, never a path.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Place {
    Desktop,
    Documents,
    Downloads,
    Icloud,
    Volume,
    Other,
}

/// Why a file could not be read.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReadFailure {
    PermissionDenied,
    NotFound,
    Other,
}

/// What `popup_drop_read_image` rejects with: enough for the page to tell the user what to do.
#[derive(Debug, Serialize)]
pub struct DropReadError {
    pub reason: ReadFailure,
    #[serde(rename = "where")]
    pub place: Place,
    /// The system's own words (`Is a directory (os error 21)`), for a reason the page has none of its own for.
    pub message: String,
}

/// The place a path is in. `home` is the user's home folder; a path outside it is a mounted volume (macOS), a network
/// share (Windows UNC) or anywhere else.
pub fn place_of(path: &Path, home: &Path) -> Place {
    if let Ok(rest) = path.strip_prefix(home) {
        let mut names = rest.components().filter_map(|part| match part {
            Component::Normal(name) => name.to_str(),
            _ => None,
        });
        let first = names.next().unwrap_or("");
        return if first.eq_ignore_ascii_case("Desktop") {
            Place::Desktop
        } else if first.eq_ignore_ascii_case("Documents") {
            Place::Documents
        } else if first.eq_ignore_ascii_case("Downloads") {
            Place::Downloads
        } else if first.eq_ignore_ascii_case("Library")
            && names
                .next()
                .is_some_and(|second| second.eq_ignore_ascii_case("Mobile Documents"))
        {
            Place::Icloud
        } else {
            Place::Other
        };
    }
    if path.starts_with("/Volumes") || path.to_str().is_some_and(|s| s.starts_with(r"\\")) {
        Place::Volume
    } else {
        Place::Other
    }
}

/// How an `io::ErrorKind` reads to the user. macOS answers a read that its privacy controls refuse with EPERM, which
/// Rust reports as `PermissionDenied`, as it does for an ordinary EACCES.
pub fn failure_of(kind: std::io::ErrorKind) -> ReadFailure {
    match kind {
        std::io::ErrorKind::PermissionDenied => ReadFailure::PermissionDenied,
        std::io::ErrorKind::NotFound => ReadFailure::NotFound,
        _ => ReadFailure::Other,
    }
}

fn read_dropped_image(path: &str, home: &Path) -> Result<String, DropReadError> {
    crate::commands::image_data_url(path).map_err(|e| DropReadError {
        reason: failure_of(e.kind()),
        place: place_of(Path::new(path), home),
        message: e.to_string(),
    })
}

/// Read a dropped picture into a data URL, like `read_image_data_url`, but a failure says why and where the file was.
/// Still one synchronous whole-file read.
#[tauri::command]
pub fn popup_drop_read_image(path: String) -> Result<String, DropReadError> {
    read_dropped_image(&path, &crate::paths::home())
}

/// What a drag event is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DragAction {
    /// A drag came into the window.
    Enter,
    /// A drag left the window without dropping.
    Leave,
    /// Files were dropped.
    Drop,
    /// The drag controller failed.
    Error,
}

/// The native event a drag was first seen by: `over` when the `enter` was missed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DragFrom {
    Enter,
    Over,
}

/// The window's form when the event happened.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DragView {
    Expanded,
    Collapsed,
}

/// What the drop landed on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DropTarget {
    Card,
    Body,
    Row,
    Preview,
    None,
}

/// What became of a drop.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DropOutcome {
    /// Everything new was attached.
    Added,
    /// Some files were attached, some could not be read.
    Partial,
    /// Nothing was attached; every new file failed to read.
    ReadFailed,
    /// Nothing was attached; every file was already there.
    Duplicates,
    /// The drag carried no files (a picture from a web page, text, ...).
    NoPaths,
    /// It landed on the attachment preview or its divider, which take no files.
    OnPreview,
    /// The request takes no files (a confirmation, a select-only question).
    NotAccepting,
    /// A dialog or a window change had the window.
    Busy,
    /// An attachment was dropped back on the request it came from, which has it already.
    OwnDragOut,
    /// The request could not be made ready to take files.
    NoReceiver,
    /// The request ended while the files were being read.
    RequestGone,
    /// There was no request to take them.
    NoTarget,
}

/// A dropped file that could not be read.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DropFailure {
    /// Classed by place for the log, never written.
    pub path: String,
    pub reason: ReadFailure,
}

/// One drag event as the page reports it. Every field but `action` is optional: a `leave` has no files and an `enter`
/// no outcome.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DropLogEntry {
    pub action: DragAction,
    #[serde(default)]
    pub request_id: Option<String>,
    #[serde(default)]
    pub view: Option<DragView>,
    #[serde(default)]
    pub from: Option<DragFrom>,
    /// The dragged files of an `enter` or a `drop`: counted and classed by place, never written.
    #[serde(default)]
    pub paths: Option<Vec<String>>,
    #[serde(default)]
    pub outcome: Option<DropOutcome>,
    #[serde(default)]
    pub target: Option<DropTarget>,
    /// The files are an attachment one of the window's own requests was dragged out of, not files from another app.
    #[serde(default)]
    pub own: Option<bool>,
    #[serde(default)]
    pub added: Option<u32>,
    #[serde(default)]
    pub images: Option<u32>,
    #[serde(default)]
    pub files: Option<u32>,
    #[serde(default)]
    pub duplicates: Option<u32>,
    #[serde(default)]
    pub failed: Vec<DropFailure>,
}

#[derive(Serialize)]
struct FailedLine {
    reason: ReadFailure,
    #[serde(rename = "where")]
    place: Place,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DropLine {
    timestamp_ms: u64,
    pid: u32,
    event: &'static str,
    action: DragAction,
    #[serde(skip_serializing_if = "Option::is_none")]
    request_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    view: Option<DragView>,
    #[serde(skip_serializing_if = "Option::is_none")]
    from: Option<DragFrom>,
    #[serde(skip_serializing_if = "Option::is_none")]
    paths: Option<usize>,
    #[serde(rename = "where", skip_serializing_if = "BTreeMap::is_empty")]
    places: BTreeMap<Place, usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    outcome: Option<DropOutcome>,
    #[serde(skip_serializing_if = "Option::is_none")]
    target: Option<DropTarget>,
    #[serde(skip_serializing_if = "Option::is_none")]
    own: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    added: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    images: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    files: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    duplicates: Option<u32>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    failed: Vec<FailedLine>,
}

/// A request id the page may name in the log: the opaque id it was given, nothing a person typed.
fn opaque_id(id: &str) -> Option<String> {
    let fits = !id.is_empty()
        && id.len() <= MAX_ID_LEN
        && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-');
    fits.then(|| id.to_string())
}

fn build_line(entry: &DropLogEntry, timestamp_ms: u64, pid: u32, home: &Path) -> Option<String> {
    let mut places = BTreeMap::new();
    for path in entry.paths.iter().flatten().take(MAX_PATHS) {
        *places.entry(place_of(Path::new(path), home)).or_insert(0) += 1;
    }
    let failed = entry
        .failed
        .iter()
        .take(MAX_FAILED)
        .map(|f| FailedLine {
            reason: f.reason,
            place: place_of(Path::new(&f.path), home),
        })
        .collect();
    serde_json::to_string(&DropLine {
        timestamp_ms,
        pid,
        event: "askhuman_drop",
        action: entry.action,
        request_id: entry.request_id.as_deref().and_then(opaque_id),
        view: entry.view,
        from: entry.from,
        paths: entry.paths.as_ref().map(Vec::len),
        places,
        outcome: entry.outcome,
        target: entry.target,
        own: entry.own,
        added: entry.added,
        images: entry.images,
        files: entry.files,
        duplicates: entry.duplicates,
        failed,
    })
    .ok()
}

/// Append one drag event to `daemon.log` (best-effort; a failure here must never reach the user's drop).
#[tauri::command]
pub fn popup_drop_log(entry: DropLogEntry) {
    let timestamp_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0);
    if let Some(line) = build_line(
        &entry,
        timestamp_ms,
        std::process::id(),
        &crate::paths::home(),
    ) {
        crate::daemon::lifecycle::append_log_line(&line);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::ErrorKind;
    use std::path::PathBuf;

    fn home() -> PathBuf {
        PathBuf::from("/Users/alice")
    }

    fn place(path: &str) -> Place {
        place_of(Path::new(path), &home())
    }

    #[test]
    fn classes_the_places_the_system_guards() {
        assert_eq!(place("/Users/alice/Desktop/shot.png"), Place::Desktop);
        assert_eq!(place("/Users/alice/Documents/a/b/c.png"), Place::Documents);
        assert_eq!(place("/Users/alice/Downloads/x.png"), Place::Downloads);
        assert_eq!(
            place("/Users/alice/Library/Mobile Documents/com~apple~CloudDocs/x.png"),
            Place::Icloud
        );
        assert_eq!(place("/Volumes/Backup/x.png"), Place::Volume);
        assert_eq!(place(r"\\server\share\x.png"), Place::Volume);
    }

    #[test]
    fn leaves_everything_else_unclassed() {
        assert_eq!(place("/Users/alice/projects/x.png"), Place::Other);
        assert_eq!(place("/Users/alice/Library/Caches/x.png"), Place::Other);
        assert_eq!(place("/tmp/x.png"), Place::Other);
        // Another user's Desktop is not this user's.
        assert_eq!(place("/Users/bob/Desktop/x.png"), Place::Other);
        // A folder that only starts like one.
        assert_eq!(place("/Users/alice/Desktops/x.png"), Place::Other);
        // The home folder itself.
        assert_eq!(place("/Users/alice"), Place::Other);
    }

    #[test]
    fn matches_the_folder_names_whatever_their_case() {
        assert_eq!(place("/Users/alice/desktop/x.png"), Place::Desktop);
        assert_eq!(place("/Users/alice/DOWNLOADS/x.png"), Place::Downloads);
    }

    #[test]
    fn tells_a_refusal_from_a_missing_file() {
        assert_eq!(
            failure_of(ErrorKind::PermissionDenied),
            ReadFailure::PermissionDenied
        );
        assert_eq!(failure_of(ErrorKind::NotFound), ReadFailure::NotFound);
        assert_eq!(failure_of(ErrorKind::InvalidData), ReadFailure::Other);
        assert_eq!(failure_of(ErrorKind::TimedOut), ReadFailure::Other);
    }

    #[cfg(unix)]
    #[test]
    fn reads_eperm_from_the_system_as_a_refusal() {
        // What macOS answers when its privacy controls turn an app away from the Desktop folder.
        let error = std::io::Error::from_raw_os_error(1);
        assert_eq!(failure_of(error.kind()), ReadFailure::PermissionDenied);
    }

    #[test]
    fn reads_a_picture_into_a_data_url() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("shot.png");
        std::fs::write(&file, b"png").unwrap();
        let url = read_dropped_image(file.to_str().unwrap(), dir.path()).unwrap();
        assert_eq!(url, "data:image/png;base64,cG5n");
    }

    #[test]
    fn says_a_missing_file_is_missing_and_where_it_was() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("Desktop").join("gone.png");
        let error = read_dropped_image(file.to_str().unwrap(), dir.path()).unwrap_err();
        assert_eq!(error.reason, ReadFailure::NotFound);
        assert_eq!(error.place, Place::Desktop);
        assert!(!error.message.is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn says_an_unreadable_file_is_refused() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("Downloads").join("locked.png");
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, b"png").unwrap();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o000)).unwrap();
        // Root reads anything, which says nothing about the code under test.
        if std::fs::read(&file).is_ok() {
            return;
        }
        let error = read_dropped_image(file.to_str().unwrap(), dir.path()).unwrap_err();
        assert_eq!(error.reason, ReadFailure::PermissionDenied);
        assert_eq!(error.place, Place::Downloads);
    }

    #[test]
    fn serializes_a_read_error_the_way_the_page_reads_it() {
        let error = DropReadError {
            reason: ReadFailure::PermissionDenied,
            place: Place::Desktop,
            message: "m".into(),
        };
        assert_eq!(
            serde_json::to_value(&error).unwrap(),
            serde_json::json!({ "reason": "permission_denied", "where": "desktop", "message": "m" })
        );
    }

    fn line(json: serde_json::Value) -> serde_json::Value {
        let entry: DropLogEntry = serde_json::from_value(json).unwrap();
        let built = build_line(&entry, 1_700_000_000_000, 42, &home()).unwrap();
        serde_json::from_str(&built).unwrap()
    }

    #[test]
    fn logs_a_drag_coming_in_with_a_count_and_the_places() {
        let written = line(serde_json::json!({
            "action": "enter", "from": "enter", "view": "collapsed",
            "paths": ["/Users/alice/Desktop/a.png", "/Users/alice/Desktop/b.png", "/tmp/c.png"],
        }));
        assert_eq!(
            written,
            serde_json::json!({
                "timestampMs": 1_700_000_000_000u64, "pid": 42, "event": "askhuman_drop",
                "action": "enter", "view": "collapsed", "from": "enter", "paths": 3,
                "where": { "desktop": 2, "other": 1 },
            })
        );
    }

    #[test]
    fn logs_a_drop_with_what_became_of_it() {
        let written = line(serde_json::json!({
            "action": "drop", "requestId": "0f8f3c1e-1b0a-4a55-9d7e-1c2f3a4b5c6d", "view": "expanded",
            "paths": ["/Users/alice/Downloads/a.png", "/Users/alice/Downloads/b.png"],
            "outcome": "partial", "target": "card", "added": 1, "images": 1, "files": 0, "duplicates": 0,
            "failed": [{ "path": "/Users/alice/Downloads/b.png", "reason": "permission_denied" }],
        }));
        assert_eq!(written["outcome"], "partial");
        assert_eq!(written["target"], "card");
        assert_eq!(written["paths"], 2);
        assert_eq!(written["where"], serde_json::json!({ "downloads": 2 }));
        assert_eq!(written["requestId"], "0f8f3c1e-1b0a-4a55-9d7e-1c2f3a4b5c6d");
        assert_eq!(
            written["failed"],
            serde_json::json!([{ "reason": "permission_denied", "where": "downloads" }])
        );
    }

    #[test]
    fn marks_a_drag_of_the_windows_own_attachment_and_says_nothing_for_other_files() {
        let own = line(serde_json::json!({
            "action": "drop", "own": true, "outcome": "added", "target": "row", "paths": ["/tmp/a.png"],
        }));
        assert_eq!(own["own"], true);
        let other = line(
            serde_json::json!({ "action": "drop", "outcome": "added", "paths": ["/tmp/a.png"] }),
        );
        assert!(other.get("own").is_none());
    }

    #[test]
    fn records_a_drop_that_carried_no_files_as_zero_paths() {
        let written =
            line(serde_json::json!({ "action": "drop", "paths": [], "outcome": "no_paths" }));
        assert_eq!(written["paths"], 0);
        assert!(written.get("where").is_none());
    }

    #[test]
    fn leaves_out_what_an_event_does_not_have() {
        let written = line(serde_json::json!({ "action": "leave" }));
        assert!(written.get("paths").is_none());
        assert!(written.get("requestId").is_none());
        assert!(written.get("failed").is_none());
    }

    #[test]
    fn never_writes_a_path_or_a_file_name() {
        let entry: DropLogEntry = serde_json::from_value(serde_json::json!({
            "action": "drop",
            "paths": ["/Users/alice/Desktop/secret-name.png", "/Users/alice/Documents/Tax 2026/return.pdf"],
            "outcome": "read_failed",
            "failed": [{ "path": "/Users/alice/Desktop/secret-name.png", "reason": "other" }],
        }))
        .unwrap();
        let built = build_line(&entry, 1, 1, &home()).unwrap();
        for leaked in ["secret", "alice", "Tax", "return", ".png", ".pdf", "/Users"] {
            assert!(!built.contains(leaked), "{leaked} leaked into {built}");
        }
    }

    #[test]
    fn names_only_an_opaque_request_id() {
        assert_eq!(
            opaque_id("0f8f3c1e-1b0a-4a55"),
            Some("0f8f3c1e-1b0a-4a55".into())
        );
        assert_eq!(opaque_id(""), None);
        assert_eq!(opaque_id("has space"), None);
        assert_eq!(opaque_id("quote\"inside"), None);
        assert_eq!(opaque_id("line\nbreak"), None);
        assert_eq!(opaque_id(&"a".repeat(65)), None);
        let written = line(serde_json::json!({ "action": "leave", "requestId": "not an id!" }));
        assert!(written.get("requestId").is_none());
    }

    #[test]
    fn rejects_a_label_the_page_made_up() {
        for bad in [
            serde_json::json!({ "action": "drop", "outcome": "my secret text" }),
            serde_json::json!({ "action": "typed by a person" }),
            serde_json::json!({ "action": "drop", "target": "/Users/alice" }),
            serde_json::json!({ "action": "drop", "own": "/Users/alice" }),
            serde_json::json!({ "action": "drop", "failed": [{ "path": "/x", "reason": "because" }] }),
        ] {
            assert!(serde_json::from_value::<DropLogEntry>(bad).is_err());
        }
    }

    #[test]
    fn caps_the_failures_one_line_names() {
        let failed: Vec<_> = (0..50)
            .map(|i| serde_json::json!({ "path": format!("/tmp/{i}.png"), "reason": "other" }))
            .collect();
        let written = line(serde_json::json!({ "action": "drop", "failed": failed }));
        assert_eq!(written["failed"].as_array().unwrap().len(), MAX_FAILED);
    }
}
