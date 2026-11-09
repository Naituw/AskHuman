//! Project-owned development identity. Ordinary users have no account-level routing state.

use super::state;
use serde::{Deserialize, Serialize};
use std::io::Read;
use std::path::{Path, PathBuf};

pub const MANIFEST: &str = "workspace.json";
pub const VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Workspace {
    pub version: u32,
    pub worktree: PathBuf,
    pub system_binary: PathBuf,
    pub system_home: PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fixture: Option<super::routing::Fixture>,
}

pub fn path(root: &Path) -> PathBuf {
    root.join(crate::dev_instance::DEV_DIR).join(MANIFEST)
}

pub fn read(root: &Path) -> Result<Workspace, String> {
    let workspace = read_structural(root)?;
    validate(root, &workspace)?;
    Ok(workspace)
}

/// Explicit enable may repair the formal binding after structural ownership checks succeed.
pub fn read_structural(root: &Path) -> Result<Workspace, String> {
    let root = root
        .canonicalize()
        .map_err(|error| format!("invalid development worktree: {error}"))?;
    let path = path(&root);
    for ancestor in path.ancestors().take(2) {
        if std::fs::symlink_metadata(ancestor)
            .is_ok_and(|metadata| metadata.file_type().is_symlink())
        {
            return Err("linked development manifest requires manual review".into());
        }
    }
    let mut bytes = Vec::new();
    std::fs::File::open(&path)
        .map_err(|error| format!("cannot read {}: {error}; run dev enable", path.display()))?
        .take(1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() > 1024 * 1024 {
        return Err("development manifest exceeds 1 MiB".into());
    }
    let workspace: Workspace = serde_json::from_slice(&bytes)
        .map_err(|error| format!("invalid development manifest: {error}"))?;
    validate_structure(&root, &workspace)?;
    Ok(workspace)
}

pub fn validate(root: &Path, workspace: &Workspace) -> Result<(), String> {
    validate_structure(root, workspace)?;
    if workspace.fixture != super::routing::fixture()?
        || !state::same_path(&workspace.system_home, &state::system_home()?)
    {
        return Err("development manifest formal binding is inconsistent; run dev enable to repair an old v1 account binding".into());
    }
    Ok(())
}

fn validate_structure(root: &Path, workspace: &Workspace) -> Result<(), String> {
    if root.ancestors().skip(1).any(is_enabled) {
        return Err(
            "nested development workspaces or an ancestor legacy marker require review".into(),
        );
    }
    let version_valid = match (&workspace.fixture, workspace.version) {
        (None, 1) => true,
        (Some(fixture), 2) => fixture.version == 2 && fixture.account_home.is_absolute(),
        _ => false,
    };
    if !version_valid
        || !workspace.worktree.is_absolute()
        || !state::same_path(&workspace.worktree, root)
        || !workspace.system_binary.is_absolute()
        || !workspace.system_home.is_absolute()
        || workspace
            .system_binary
            .canonicalize()
            .unwrap_or_else(|_| workspace.system_binary.clone())
            .components()
            .any(|component| component.as_os_str() == crate::dev_instance::DEV_DIR)
        || workspace
            .system_home
            .canonicalize()
            .unwrap_or_else(|_| workspace.system_home.clone())
            .components()
            .any(|component| component.as_os_str() == crate::dev_instance::DEV_DIR)
    {
        return Err("development manifest identity is inconsistent".into());
    }
    Ok(())
}

pub fn is_project(root: &Path) -> bool {
    std::fs::symlink_metadata(path(root)).is_ok()
}

pub fn is_enabled(root: &Path) -> bool {
    is_project(root)
        || root
            .join(crate::dev_instance::DEV_DIR)
            .join(crate::dev_instance::ENABLED_MARKER)
            .is_file()
}

pub fn find_root(start: &Path) -> Option<PathBuf> {
    let start = if start.is_file() {
        start.parent()?
    } else {
        start
    };
    start
        .ancestors()
        .find(|root| is_enabled(root))
        .map(Path::to_path_buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest(root: &Path) -> Workspace {
        Workspace {
            version: VERSION,
            worktree: root.to_path_buf(),
            system_binary: root.parent().unwrap().join("system/AskHuman"),
            system_home: state::system_home().unwrap(),
            fixture: None,
        }
    }

    #[test]
    fn a_project_manifest_does_not_enable_legacy_cwd_redirection() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        state::write_json(&path(&root), &manifest(&root)).unwrap();
        assert!(is_enabled(&root));
        assert_eq!(find_root(&root), Some(root.clone()));
        assert!(crate::dev_instance::find_dev_root(&root).is_none());
        assert!(read(&root).is_ok());
    }

    #[test]
    fn a_copied_manifest_cannot_claim_another_worktree_or_dev_system_home() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let mut workspace = manifest(&root);
        workspace.worktree = root.join("other");
        state::write_json(&path(&root), &workspace).unwrap();
        assert!(read(&root).is_err());
        workspace.worktree = root.clone();
        workspace.system_home = root.join(crate::dev_instance::DEV_DIR).join("home");
        state::write_json(&path(&root), &workspace).unwrap();
        assert!(read(&root).is_err());
    }
}
