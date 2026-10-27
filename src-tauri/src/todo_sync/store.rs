//! Persistence of the sidecar `state.json` and the tick mutex (plan §2, §3.1).
//!
//! `state.json` is the only place the sync core keeps data; `todos.json` stays untouched by anything
//! that is not a real todo change. The file is replaced atomically (tmp + rename), so a crash leaves
//! either the old or the new version. A file written by a *newer* schema is never rewritten, and a
//! file that does not parse is moved aside rather than deleted.

use super::model::{State, SCHEMA};
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

pub fn state_file(dir: &Path) -> PathBuf {
    dir.join("state.json")
}

pub fn lock_path(dir: &Path) -> PathBuf {
    dir.join("todo-sync.lock")
}

#[derive(Debug)]
pub enum Loaded {
    /// No state file: sync was never enabled on this device.
    Absent,
    State(Box<State>),
    /// The file belongs to a newer schema; it must be neither interpreted nor overwritten.
    Newer(u32),
    /// The file did not parse; it was renamed to `backup` (when that worked).
    Corrupt {
        backup: Option<PathBuf>,
    },
}

pub fn load(dir: &Path) -> io::Result<Loaded> {
    let path = state_file(dir);
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Loaded::Absent),
        Err(e) => return Err(e),
    };
    match serde_json::from_str::<State>(&text) {
        Ok(state) if state.schema > SCHEMA => Ok(Loaded::Newer(state.schema)),
        Ok(state) => Ok(Loaded::State(Box::new(state))),
        Err(_) => {
            // A newer schema may not even fit our struct: peek at the version before giving up.
            let schema = serde_json::from_str::<serde_json::Value>(&text)
                .ok()
                .and_then(|v| v.get("schema").and_then(|s| s.as_u64()))
                .unwrap_or(0);
            if schema > u64::from(SCHEMA) {
                return Ok(Loaded::Newer(schema as u32));
            }
            Ok(Loaded::Corrupt {
                backup: move_aside(&path),
            })
        }
    }
}

fn move_aside(path: &Path) -> Option<PathBuf> {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let backup = path.with_file_name(format!("state.json.corrupt-{stamp}"));
    std::fs::rename(path, &backup).ok().map(|_| backup)
}

/// Atomic replace of `state.json`.
pub fn save(dir: &Path, state: &State) -> io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let json = serde_json::to_vec(state).map_err(io::Error::other)?;
    let path = state_file(dir);
    let tmp = dir.join(format!("state.json.tmp-{}", uuid::Uuid::new_v4()));
    std::fs::write(&tmp, &json)?;
    match std::fs::rename(&tmp, &path) {
        Ok(()) => Ok(()),
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            Err(e)
        }
    }
}

/// Whether sync was ever enabled on this device (cheap: one `stat`).
pub fn exists(dir: &Path) -> bool {
    state_file(dir).exists()
}

/// Held for the whole of a tick: only one tick runs at a time per device.
pub struct TickLock {
    _inner: crate::file_lock::FileLock,
}

pub fn lock_blocking(dir: &Path) -> io::Result<TickLock> {
    Ok(TickLock {
        _inner: crate::file_lock::FileLock::exclusive(&lock_path(dir))?,
    })
}

/// Wait up to `wait` for the tick lock; `None` when someone else still holds it.
pub fn lock_timeout(dir: &Path, wait: Duration) -> io::Result<Option<TickLock>> {
    let deadline = Instant::now() + wait;
    loop {
        if let Some(inner) = crate::file_lock::FileLock::try_exclusive(&lock_path(dir))? {
            return Ok(Some(TickLock { _inner: inner }));
        }
        if Instant::now() >= deadline {
            return Ok(None);
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh() -> State {
        State::new("device-1".into(), "Studio".into(), 5)
    }

    #[test]
    fn absent_until_first_save_and_roundtrips_atomically() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("todo-sync");
        assert!(matches!(load(&dir).unwrap(), Loaded::Absent));
        assert!(!exists(&dir), "loading must not create the directory");
        let mut s = fresh();
        s.last_pull_ok_ms = 99;
        save(&dir, &s).unwrap();
        assert!(exists(&dir));
        match load(&dir).unwrap() {
            Loaded::State(back) => assert_eq!(*back, s),
            other => panic!("unexpected {other:?}"),
        }
        // No tmp litter after a save.
        let litter: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().contains("tmp-"))
            .collect();
        assert!(litter.is_empty());
    }

    #[test]
    fn corrupt_file_is_moved_aside_not_deleted() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        std::fs::write(state_file(dir), b"{ this is not json").unwrap();
        match load(dir).unwrap() {
            Loaded::Corrupt { backup: Some(b) } => {
                assert_eq!(std::fs::read(&b).unwrap(), b"{ this is not json");
            }
            other => panic!("unexpected {other:?}"),
        }
        assert!(!state_file(dir).exists());
        assert!(matches!(load(dir).unwrap(), Loaded::Absent));
    }

    #[test]
    fn newer_schema_is_reported_and_left_untouched() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        // Shape we cannot parse at all, but the version is readable.
        let text = br#"{"schema": 9, "somethingNew": [1,2,3]}"#;
        std::fs::write(state_file(dir), text).unwrap();
        assert!(matches!(load(dir).unwrap(), Loaded::Newer(9)));
        assert_eq!(std::fs::read(state_file(dir)).unwrap(), text);
        // Parsable but newer.
        let mut s = fresh();
        s.schema = SCHEMA + 1;
        save(dir, &s).unwrap();
        assert!(matches!(load(dir).unwrap(), Loaded::Newer(v) if v == SCHEMA + 1));
    }

    #[test]
    fn tick_lock_excludes_a_second_holder() {
        let tmp = tempfile::tempdir().unwrap();
        let first = lock_blocking(tmp.path()).unwrap();
        assert!(lock_timeout(tmp.path(), Duration::from_millis(30))
            .unwrap()
            .is_none());
        drop(first);
        assert!(lock_timeout(tmp.path(), Duration::from_millis(30))
            .unwrap()
            .is_some());
    }
}
