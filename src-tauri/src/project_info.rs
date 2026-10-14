//! Bounded, read-only Git facts and directory-only terminal actions for popup source cards.
use serde::Serialize;
use std::{
    io::Read,
    path::Path,
    process::{Command, Stdio},
    time::{Duration, Instant},
};
#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitInfo {
    pub branch: Option<String>,
    pub detached: bool,
    pub worktree: Option<String>,
    pub changes: Option<Changes>,
}
#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Changes {
    pub total: usize,
    pub staged: usize,
    pub unstaged: usize,
    pub untracked: usize,
}
fn git(root: &Path, args: &[&str]) -> Option<Vec<u8>> {
    let mut command = Command::new("git");
    command
        .args(["--no-optional-locks", "-c", "core.fsmonitor=false", "-C"])
        .arg(root)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let mut child = command.spawn().ok()?;
    let out = child.stdout.take()?;
    let reader = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        out.take(1024 * 1024 + 1).read_to_end(&mut bytes).ok()?;
        Some(bytes)
    });
    let began = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Err(_) => break None,
            _ => {}
        }
        if began.elapsed() > Duration::from_millis(600) {
            let _ = child.kill();
            let _ = child.wait();
            break None;
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    let bytes = reader.join().ok()??;
    if !status?.success() || bytes.len() > 1024 * 1024 {
        return None;
    }
    Some(bytes)
}
fn git_text(root: &Path, args: &[&str]) -> Option<String> {
    String::from_utf8(git(root, args)?)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}
fn parse_changes(bytes: &[u8]) -> Option<Changes> {
    let mut changes = Changes::default();
    let mut rows = bytes.split(|b| *b == 0).filter(|row| !row.is_empty());
    while let Some(row) = rows.next() {
        if row.len() < 3 {
            return None;
        }
        changes.total += 1;
        if &row[..2] == b"??" {
            changes.untracked += 1;
            continue;
        }
        if row[0] != b' ' && row[0] != b'?' {
            changes.staged += 1;
        }
        if row[1] != b' ' && row[1] != b'?' {
            changes.unstaged += 1;
        }
        if row[..2].iter().any(|b| *b == b'R' || *b == b'C') {
            rows.next()?;
        }
    }
    Some(changes)
}
pub fn read(root: &Path) -> GitInfo {
    if crate::project::git_root(root).is_none() {
        return GitInfo::default();
    }
    let mut info = GitInfo {
        branch: git_text(root, &["symbolic-ref", "--quiet", "--short", "HEAD"]),
        ..Default::default()
    };
    if info.branch.is_none() {
        info.branch = git_text(root, &["rev-parse", "--short", "HEAD"]);
        info.detached = info.branch.is_some();
    }
    if let (Some(dir), Some(common)) = (
        git_text(root, &["rev-parse", "--git-dir"]),
        git_text(root, &["rev-parse", "--git-common-dir"]),
    ) {
        let dir = root.join(dir);
        let common = root.join(common);
        if let (Ok(dir), Ok(common)) = (std::fs::canonicalize(dir), std::fs::canonicalize(common)) {
            if dir != common {
                info.worktree = root.file_name().map(|s| s.to_string_lossy().into_owned());
            }
        }
    }
    info.changes = git(
        root,
        &["status", "--porcelain=v1", "-z", "--untracked-files=all"],
    )
    .and_then(|bytes| parse_changes(&bytes));
    info
}

/// The caller supplies a directory selected from its own pending request, never shell code.
pub fn open_terminal(path: &Path) -> Result<(), String> {
    if !path.is_absolute() || !path.is_dir() {
        return Err("directory is unavailable".into());
    }
    #[cfg(target_os = "macos")]
    let status = Command::new("/usr/bin/open")
        .args(["-a", "Terminal"])
        .arg(path)
        .status();
    #[cfg(windows)]
    let status = {
        use std::os::windows::process::CommandExt;
        Command::new("wt.exe")
            .args(["-w", "new", "new-tab", "--startingDirectory"])
            .arg(path)
            .creation_flags(0x08000000)
            .status()
    };
    #[cfg(all(not(target_os = "macos"), not(windows)))]
    {
        let terminal = [
            ("gnome-terminal", "--working-directory"),
            ("konsole", "--workdir"),
            ("xfce4-terminal", "--working-directory"),
        ]
        .into_iter()
        .find(|(name, _)| {
            std::env::var_os("PATH").is_some_and(|value| {
                std::env::split_paths(&value).any(|dir| dir.join(name).is_file())
            })
        })
        .ok_or("no supported terminal is available")?;
        Command::new(terminal.0)
            .arg(terminal.1)
            .arg(path)
            .spawn()
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
    #[cfg(any(target_os = "macos", windows))]
    if status.map_err(|e| e.to_string())?.success() {
        Ok(())
    } else {
        Err("terminal could not open this directory".into())
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn real_git_reports_branch_worktree_and_changes_without_writing_the_index() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("repo");
        std::fs::create_dir(&root).unwrap();
        let run = |root: &Path, args: &[&str]| {
            assert!(Command::new("git")
                .arg("-C")
                .arg(root)
                .args(args)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .unwrap()
                .success());
        };
        run(&root, &["init", "-b", "main"]);
        std::fs::write(root.join("tracked"), "base").unwrap();
        run(&root, &["add", "."]);
        run(
            &root,
            &[
                "-c",
                "user.name=QA",
                "-c",
                "user.email=qa@example.invalid",
                "-c",
                "commit.gpgsign=false",
                "commit",
                "-m",
                "fixture",
            ],
        );
        std::fs::write(root.join("tracked"), "changed").unwrap();
        std::fs::write(root.join("new file"), "new").unwrap();
        let before = std::fs::read(root.join(".git/index")).unwrap();
        let info = read(&root);
        assert_eq!(info.branch.as_deref(), Some("main"));
        assert!(!info.detached);
        assert!(info.worktree.is_none());
        let changes = info.changes.unwrap();
        assert_eq!(
            (
                changes.total,
                changes.staged,
                changes.unstaged,
                changes.untracked
            ),
            (2, 0, 1, 1)
        );
        assert_eq!(std::fs::read(root.join(".git/index")).unwrap(), before);
        let linked = dir.path().join("linked");
        run(
            &root,
            &["worktree", "add", "-b", "feature", linked.to_str().unwrap()],
        );
        let info = read(&linked);
        assert_eq!(info.branch.as_deref(), Some("feature"));
        assert_eq!(info.worktree.as_deref(), Some("linked"));
        assert_eq!(info.changes.unwrap().total, 0);
        run(&linked, &["checkout", "--detach"]);
        assert!(read(&linked).detached);
    }

    use super::*;
    #[test]
    fn nul_status_counts_renames_once_and_handles_unusual_names() {
        let c =
            parse_changes(b"M  staged\0 M unstaged\0MM both\0?? name\nwith newline\0R  new\0old\0")
                .unwrap();
        assert_eq!((c.total, c.staged, c.unstaged, c.untracked), (5, 3, 2, 1));
        assert!(parse_changes(b"R  missing-original\0").is_none());
    }
    #[test]
    fn unavailable_repository_has_no_fake_zero_metrics() {
        let dir = tempfile::tempdir().unwrap();
        let info = read(dir.path());
        assert!(info.branch.is_none());
        assert!(info.changes.is_none());
    }
}
