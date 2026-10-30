//! Non-secret routing state, always located independently of ASKHUMAN_HOME.

use serde::{Deserialize, Serialize};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RoutingMode {
    #[default]
    Legacy,
    Explicit,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct State {
    pub version: u32,
    pub mode: RoutingMode,
    pub system_binary: Option<PathBuf>,
    pub worktrees: Vec<PathBuf>,
}

pub fn system_home() -> PathBuf {
    crate::paths::home().join(".askhuman")
}

pub fn directory() -> PathBuf {
    system_home().join("development")
}

pub fn path() -> PathBuf {
    directory().join("routing.json")
}

pub fn read() -> Result<State, String> {
    read_at(&path())
}

pub fn read_at(path: &Path) -> Result<State, String> {
    let data = match std::fs::File::open(path) {
        Ok(file) => {
            let mut data = Vec::new();
            file.take(1024 * 1024 + 1)
                .read_to_end(&mut data)
                .map_err(|e| e.to_string())?;
            if data.len() > 1024 * 1024 {
                return Err("development routing state exceeds 1 MiB".into());
            }
            data
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(State::default()),
        Err(e) => return Err(format!("cannot read {}: {e}", path.display())),
    };
    let state: State = serde_json::from_slice(&data)
        .map_err(|e| format!("invalid development routing state {}: {e}", path.display()))?;
    if state.version > 1 {
        return Err("development routing state requires a newer AskHuman".into());
    }
    Ok(state)
}

pub fn update(change: impl FnOnce(&mut State) -> Result<(), String>) -> Result<State, String> {
    update_at(&path(), change)
}

pub fn update_at(
    path: &Path,
    change: impl FnOnce(&mut State) -> Result<(), String>,
) -> Result<State, String> {
    let _lock = crate::file_lock::FileLock::exclusive(&path.with_extension("lock"))
        .map_err(|e| e.to_string())?;
    let mut state = read_at(path)?;
    change(&mut state)?;
    state.version = 1;
    write_json(path, &state)?;
    Ok(state)
}

pub fn write_json(path: &Path, value: &impl Serialize) -> Result<(), String> {
    let parent = path.parent().ok_or("state path has no parent")?;
    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700))
            .map_err(|e| e.to_string())?;
    }
    let mut staged = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
    serde_json::to_writer_pretty(&mut staged, value).map_err(|e| e.to_string())?;
    staged.write_all(b"\n").map_err(|e| e.to_string())?;
    staged.as_file().sync_all().map_err(|e| e.to_string())?;
    staged.persist(path).map_err(|e| e.to_string())?;
    Ok(())
}

pub fn register_worktree(root: &Path) -> Result<(), String> {
    let root = root.canonicalize().map_err(|e| e.to_string())?;
    update(|state| {
        if !state.worktrees.iter().any(|p| same_path(p, &root)) {
            if state.worktrees.len() >= 128 {
                return Err(
                    "too many registered worktrees; remove obsolete routing entries first".into(),
                );
            }
            state.worktrees.push(root);
        }
        Ok(())
    })?;
    Ok(())
}

pub fn same_path(a: &Path, b: &Path) -> bool {
    let a = a.canonicalize().unwrap_or_else(|_| a.to_path_buf());
    let b = b.canonicalize().unwrap_or_else(|_| b.to_path_buf());
    crate::path_identity::equivalent(&a.to_string_lossy(), &b.to_string_lossy())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn absent_state_preserves_legacy_but_corrupt_state_never_resets_silently() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("routing.json");
        assert_eq!(read_at(&path).unwrap().mode, RoutingMode::Legacy);
        std::fs::write(&path, "broken").unwrap();
        assert!(read_at(&path).is_err());
    }

    #[test]
    fn serialized_updates_preserve_registered_worktrees() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("routing.json");
        update_at(&path, |state| {
            state.worktrees.push(temp.path().join("a"));
            Ok(())
        })
        .unwrap();
        update_at(&path, |state| {
            state.mode = RoutingMode::Explicit;
            Ok(())
        })
        .unwrap();
        let state = read_at(&path).unwrap();
        assert_eq!(state.mode, RoutingMode::Explicit);
        assert_eq!(state.worktrees.len(), 1);
        assert_eq!(state.version, 1);
    }
}
