//! Private atomic state writes and shared path identity helpers.

use serde::Serialize;
use std::io::Write;
use std::path::{Path, PathBuf};

pub fn system_home() -> PathBuf {
    crate::paths::home().join(".askhuman")
}

pub fn directory() -> PathBuf {
    system_home().join("state/development-relay")
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

pub fn same_path(a: &Path, b: &Path) -> bool {
    let a = a.canonicalize().unwrap_or_else(|_| a.to_path_buf());
    let b = b.canonicalize().unwrap_or_else(|_| b.to_path_buf());
    crate::path_identity::equivalent(&a.to_string_lossy(), &b.to_string_lossy())
}
