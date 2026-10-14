//! Project identification for reply history.
//!
//! A trusted native session workspace owns the project. Without one, use the CLI invocation's
//! working directory. Walk up to the first `.git` entry and fall back to that directory itself.
//! Keep the actual invocation cwd separately so attachment resolution and directory actions do
//! not change when the stable project differs. Existing history is never migrated.

use std::path::{Path, PathBuf};

/// Detect the current project key (absolute path). Returns an empty string only when the working
/// directory can't be determined.
pub fn detect() -> String {
    let cwd = match std::env::current_dir() {
        Ok(p) => p,
        Err(_) => return String::new(),
    };
    let caller = crate::cli::caller_context();
    for_session(
        caller.agent_kind.as_deref(),
        caller.agent_session_id.as_deref(),
        &cwd,
    )
}

/// A trusted native workspace owns the project even when a tool changes its cwd.
/// Plain CLI calls and unavailable session metadata retain the original directory rule.
pub fn for_session(kind: Option<&str>, session_id: Option<&str>, cwd: &Path) -> String {
    let workspace = kind
        .and_then(crate::agents::AgentKind::parse)
        .zip(session_id)
        .and_then(|(kind, sid)| crate::agents::session_info::workspace(kind, sid));
    with_workspace(cwd, workspace.as_deref().map(Path::new))
}

fn with_workspace(cwd: &Path, workspace: Option<&Path>) -> String {
    if workspace.is_none() && cwd.as_os_str().is_empty() {
        return String::new();
    }
    detect_from(workspace.unwrap_or(cwd))
}

/// Project key for an explicit directory (e.g. the cwd a hook reported on stdin): its git root,
/// falling back to the directory itself.
pub fn detect_from(dir: &Path) -> String {
    let root = git_root(dir).unwrap_or_else(|| dir.to_path_buf());
    canonical_string(&root)
}

/// Walk up from `start` to the first ancestor that contains a `.git` entry (file or dir).
pub fn git_root(start: &Path) -> Option<PathBuf> {
    let mut cur: Option<&Path> = Some(start);
    while let Some(dir) = cur {
        if dir.join(".git").exists() {
            return Some(dir.to_path_buf());
        }
        cur = dir.parent();
    }
    None
}

/// Canonicalize when possible (resolves symlinks); fall back to the lossy display path.
fn canonical_string(path: &Path) -> String {
    std::fs::canonicalize(path)
        .unwrap_or_else(|_| path.to_path_buf())
        .to_string_lossy()
        .to_string()
}

/// Display name for a project key: the final path component; empty key yields an empty string
/// (callers localize an "unknown project" label).
pub fn display_name(key: &str) -> String {
    if key.is_empty() {
        return String::new();
    }
    Path::new(key)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| key.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn subdir_resolves_to_git_root() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        fs::create_dir_all(root.join(".git")).unwrap();
        let sub = root.join("a").join("b");
        fs::create_dir_all(&sub).unwrap();
        let found = git_root(&sub).unwrap();
        // Compare canonicalized to tolerate /private symlink on macOS temp dirs.
        assert_eq!(
            fs::canonicalize(&found).unwrap(),
            fs::canonicalize(root).unwrap()
        );
    }

    #[test]
    fn session_workspace_owns_temp_and_other_repository_calls() {
        let a = tempdir().unwrap();
        let b = tempdir().unwrap();
        let temporary = tempdir().unwrap();
        fs::create_dir(a.path().join(".git")).unwrap();
        fs::create_dir(b.path().join(".git")).unwrap();
        let expected = detect_from(a.path());
        assert_eq!(with_workspace(temporary.path(), Some(a.path())), expected);
        assert_eq!(with_workspace(b.path(), Some(a.path())), expected);
        assert_eq!(with_workspace(b.path(), None), detect_from(b.path()));
        assert_eq!(
            with_workspace(temporary.path(), None),
            detect_from(temporary.path())
        );
    }

    #[test]
    fn no_git_returns_none() {
        let dir = tempdir().unwrap();
        let sub = dir.path().join("x");
        fs::create_dir_all(&sub).unwrap();
        assert!(git_root(&sub).is_none());
    }

    #[test]
    fn display_name_is_basename() {
        assert_eq!(display_name("/home/u/my-proj"), "my-proj");
        assert_eq!(display_name(""), "");
    }
}
