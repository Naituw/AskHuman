//! The folders Antigravity conversations ran in, for the first list of folders to start a task in
//! (spec antigravity-deep-integration §12.7, P4 ③).
//!
//! The App and `agy` each keep one summary row per conversation, with the `file://` URIs of its
//! folders, in a SQLite file of their own. The format is private, so this only seeds a list for
//! someone who has used nothing else, and nothing depends on it: the file is opened read-only and
//! immutable (it is in WAL mode and may have no `-shm` beside it, which a plain read-only open
//! cannot cope with), and a missing file, another schema or a row that does not parse is an empty
//! answer, never an error.

use rusqlite::{Connection, OpenFlags};
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Rows read from one database, newest first.
const MAX_ROWS: usize = 300;
/// A summary row's folder list is a short JSON array; anything bigger is not one.
const MAX_FOLDERS_BYTES: usize = 16 * 1024;

/// The folders of the recent conversations of both products (plus enabled extra Antigravity
/// profiles such as an external host's ACP homes, spec P8), with the time each was last used (Unix seconds).
pub(crate) fn recent_workspaces() -> Vec<(PathBuf, u64)> {
    let mut latest = HashMap::<PathBuf, u64>::new();
    for (folder, used) in recent_workspaces_in(&crate::paths::integration_home()) {
        let slot = latest.entry(folder).or_insert(0);
        *slot = (*slot).max(used);
    }
    for root in crate::integrations::agent_profiles::enabled_roots(super::AgentKind::Antigravity) {
        for database in crate::paths::antigravity_summary_dbs_in_root(&root) {
            for (folder, used) in read(&database) {
                let slot = latest.entry(folder).or_insert(0);
                *slot = (*slot).max(used);
            }
        }
    }
    latest.into_iter().collect()
}

fn recent_workspaces_in(home: &Path) -> Vec<(PathBuf, u64)> {
    let mut latest = HashMap::<PathBuf, u64>::new();
    for database in crate::paths::antigravity_summary_dbs_in(home) {
        for (folder, used) in read(&database) {
            let slot = latest.entry(folder).or_insert(0);
            *slot = (*slot).max(used);
        }
    }
    latest.into_iter().collect()
}

fn read(database: &Path) -> Vec<(PathBuf, u64)> {
    if !database.is_file() {
        return Vec::new();
    }
    let Ok(conn) = Connection::open_with_flags(
        immutable_uri(database),
        OpenFlags::SQLITE_OPEN_READ_ONLY
            | OpenFlags::SQLITE_OPEN_URI
            | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    ) else {
        return Vec::new();
    };
    let _ = conn.busy_timeout(std::time::Duration::from_millis(200));
    let Ok(mut statement) = conn.prepare(
        "SELECT workspace_uris, last_modified_time FROM conversation_summaries \
         WHERE workspace_uris <> '' ORDER BY last_modified_time DESC LIMIT ?1",
    ) else {
        return Vec::new();
    };
    let Ok(rows) = statement.query_map([MAX_ROWS as i64], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    }) else {
        return Vec::new();
    };
    let mut folders = Vec::new();
    for (uris, modified) in rows.flatten() {
        let Some(used) = parse_time(&modified) else {
            continue;
        };
        folders.extend(parse_folders(&uris).into_iter().map(|path| (path, used)));
    }
    folders
}

/// `file:<path>?immutable=1`, the path percent-encoded so that `?`, `#` and `%` in a folder name
/// stay part of it.
fn immutable_uri(database: &Path) -> String {
    let mut uri = String::from("file:");
    for byte in database.to_string_lossy().bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' => {
                uri.push(byte as char)
            }
            other => uri.push_str(&format!("%{other:02X}")),
        }
    }
    uri.push_str("?immutable=1");
    uri
}

/// The local folders in a row's `workspace_uris` (a JSON array of `file://` URIs).
fn parse_folders(raw: &str) -> Vec<PathBuf> {
    if raw.len() > MAX_FOLDERS_BYTES {
        return Vec::new();
    }
    let Ok(Value::Array(items)) = serde_json::from_str::<Value>(raw) else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(Value::as_str)
        .filter_map(path_from_uri)
        .collect()
}

/// The absolute path of a local `file://` URI.
fn path_from_uri(uri: &str) -> Option<PathBuf> {
    let rest = uri.strip_prefix("file://")?;
    // `file://<authority>/<path>`: only this machine's own files count.
    let path = rest.strip_prefix("localhost").unwrap_or(rest);
    if !path.starts_with('/') {
        return None;
    }
    let decoded = percent_decode(path)?;
    // `file:///C:/work` names `C:/work`.
    let text = match decoded.as_bytes() {
        [b'/', drive, b':', ..] if cfg!(windows) && drive.is_ascii_alphabetic() => &decoded[1..],
        _ => decoded.as_str(),
    };
    let path = PathBuf::from(text);
    path.is_absolute().then_some(path)
}

fn percent_decode(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let pair = bytes.get(index + 1..index + 3)?;
            let value = u8::from_str_radix(std::str::from_utf8(pair).ok()?, 16).ok()?;
            out.push(value);
            index += 3;
        } else {
            out.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(out)
        .ok()
        .filter(|text| !text.contains('\0'))
}

/// `2026-11-16 03:31:23.714692+00:00` (what the products write) as Unix seconds. The zero time
/// they write for a row nobody has touched is no time at all.
fn parse_time(text: &str) -> Option<u64> {
    let parsed = chrono::DateTime::parse_from_str(text, "%Y-%m-%d %H:%M:%S%.f%:z").ok()?;
    u64::try_from(parsed.timestamp())
        .ok()
        .filter(|&seconds| seconds > 0)
}

// Antigravity's integration is Unix-only (spec antigravity-deep-integration §14), and these
// fixtures name folders the POSIX way, which is not an absolute path on Windows.
#[cfg(all(test, unix))]
mod tests {
    use super::*;

    fn database(dir: &Path, name: &str, rows: &[(&str, &str)]) -> PathBuf {
        let path = dir.join(name);
        let conn = Connection::open(&path).unwrap();
        // The products keep this file in WAL mode, which is what a plain read-only open fails on.
        conn.pragma_update(None, "journal_mode", "WAL").unwrap();
        conn.execute_batch(
            "CREATE TABLE conversation_summaries (conversation_id text PRIMARY KEY, \
             workspace_uris text NOT NULL, last_modified_time datetime NOT NULL)",
        )
        .unwrap();
        for (index, (uris, modified)) in rows.iter().enumerate() {
            conn.execute(
                "INSERT INTO conversation_summaries VALUES (?1, ?2, ?3)",
                rusqlite::params![format!("c{index}"), uris, modified],
            )
            .unwrap();
        }
        drop(conn);
        path
    }

    #[test]
    fn a_folder_uri_becomes_its_path() {
        assert_eq!(
            path_from_uri("file:///Users/me/My%20Project/%C3%A4%281%29%23"),
            Some(PathBuf::from("/Users/me/My Project/ä(1)#"))
        );
        assert_eq!(
            path_from_uri("file://localhost/Users/me/p"),
            Some(PathBuf::from("/Users/me/p"))
        );
    }

    #[test]
    fn what_is_not_a_local_folder_is_dropped() {
        assert_eq!(path_from_uri("https://example.com/x"), None);
        assert_eq!(path_from_uri("file://server/share/x"), None);
        assert_eq!(path_from_uri("file:///Users/me/%zz"), None);
        assert_eq!(path_from_uri("file:///Users/me/%00"), None);
        assert_eq!(path_from_uri("file://relative"), None);
        assert!(parse_folders("not json").is_empty());
        assert!(parse_folders("{\"a\":1}").is_empty());
        assert!(parse_folders(&format!("[\"{}\"]", "a".repeat(MAX_FOLDERS_BYTES))).is_empty());
    }

    #[test]
    fn a_row_may_name_several_folders() {
        assert_eq!(
            parse_folders(r#"["file:///a/b","file:///c","https://x/y"]"#),
            vec![PathBuf::from("/a/b"), PathBuf::from("/c")]
        );
    }

    #[test]
    fn times_are_read_the_way_the_products_write_them() {
        assert_eq!(
            parse_time("2026-11-16 03:31:23.714692+00:00"),
            Some(1_794_799_883)
        );
        assert_eq!(parse_time("2026-11-16 11:31:23+08:00"), Some(1_794_799_883));
        // The time of a row nobody has touched.
        assert_eq!(parse_time("0001-01-01 00:00:00+00:00"), None);
        assert_eq!(parse_time("yesterday"), None);
    }

    #[test]
    fn both_databases_are_read_and_the_newest_use_of_a_folder_wins() {
        let home = tempfile::tempdir().unwrap();
        let [app, cli] = crate::paths::antigravity_summary_dbs_in(home.path());
        std::fs::create_dir_all(app.parent().unwrap()).unwrap();
        std::fs::create_dir_all(cli.parent().unwrap()).unwrap();
        database(
            app.parent().unwrap(),
            "conversation_summaries.db",
            &[
                (r#"["file:///work/a"]"#, "2026-11-16 03:00:00+00:00"),
                (r#"["file:///work/b"]"#, "2026-11-10 03:00:00+00:00"),
            ],
        );
        database(
            cli.parent().unwrap(),
            "conversation_summaries.db",
            &[
                (r#"["file:///work/a"]"#, "2026-11-17 03:00:00+00:00"),
                // A conversation `agy` ran outside any folder it knows records none.
                ("", "2026-11-18 03:00:00+00:00"),
            ],
        );
        let mut found = recent_workspaces_in(home.path());
        found.sort();
        assert_eq!(
            found,
            vec![
                (PathBuf::from("/work/a"), 1_794_884_400),
                (PathBuf::from("/work/b"), 1_794_279_600),
            ]
        );
    }

    #[test]
    fn a_missing_or_foreign_file_is_an_empty_answer() {
        let home = tempfile::tempdir().unwrap();
        assert!(recent_workspaces_in(home.path()).is_empty());
        let [app, _] = crate::paths::antigravity_summary_dbs_in(home.path());
        std::fs::create_dir_all(app.parent().unwrap()).unwrap();
        std::fs::write(&app, b"this is not a database").unwrap();
        assert!(recent_workspaces_in(home.path()).is_empty());
        // A database with another schema.
        let other = home.path().join("other.db");
        let conn = Connection::open(&other).unwrap();
        conn.execute_batch("CREATE TABLE t (x)").unwrap();
        drop(conn);
        assert!(read(&other).is_empty());
    }

    #[test]
    fn a_folder_name_with_uri_punctuation_still_opens() {
        let home = tempfile::tempdir().unwrap();
        let dir = home.path().join("a b#c?d%e");
        std::fs::create_dir_all(&dir).unwrap();
        let path = database(
            &dir,
            "conversation_summaries.db",
            &[(r#"["file:///work/a"]"#, "2026-11-16 03:00:00+00:00")],
        );
        assert_eq!(read(&path).len(), 1);
    }
}
