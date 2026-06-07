//! Project identification for reply history.
//!
//! The "current project" is derived from the CLI invocation's working directory: walk up from
//! `cwd` to the first ancestor containing a `.git` entry (the repo root); fall back to `cwd` when
//! no repo is found. The canonicalized absolute path is the project key; its basename is the
//! display name. This is computed by the CLI (and standalone GUI processes) and carried through to
//! the recording point so history can be filtered per project.

use std::path::{Path, PathBuf};

/// Detect the current project key (absolute path). Returns an empty string only when the working
/// directory can't be determined.
pub fn detect() -> String {
    let cwd = match std::env::current_dir() {
        Ok(p) => p,
        Err(_) => return String::new(),
    };
    let root = git_root(&cwd).unwrap_or(cwd);
    canonical_string(&root)
}

/// Walk up from `start` to the first ancestor that contains a `.git` entry (file or dir).
fn git_root(start: &Path) -> Option<PathBuf> {
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
