//! Files a request's text links to, registered when the user clicks or right-clicks the link
//! (docs/specs/popup-file-links.md §4).
//!
//! The preview commands take an owner id and an index; the file behind the index comes from
//! `attachment_owner::files`. A linked file is appended to that list, after the owner's real
//! attachments, the first time [`LinkedFiles::add`] sees its path. Every command that works on an
//! index — reading, system preview, find, browser snapshot, reveal, the context menu — then
//! handles it unchanged.
//!
//! [`resolve`] is where a preview path comes from the frontend, which makes it the one exception
//! to "the frontend never names a file". It is bounded the way `local_image_prepare` is, only a
//! user gesture reaches it, and what it unlocks is the same bounded, read-only preview an
//! attachment gets.
use crate::attachment_preview::HISTORY_OWNER_PREFIX;
use crate::models::FileAttachment;
use serde::Serialize;
use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
use std::sync::Mutex;

/// Linked files one owner may register.
pub const MAX_PER_OWNER: usize = 64;
/// Longest path the frontend may name.
const MAX_PATH_BYTES: usize = 4096;

/// What the path the user clicked turned out to be.
#[derive(Debug, PartialEq)]
pub enum Resolved {
    /// A regular file, or a path with nothing readable at it (the panel then says so).
    File(FileAttachment),
    /// A directory, a device, a pipe…: nothing to preview, the caller hands it to the system.
    Other,
}

/// The command's answer.
#[derive(Debug, PartialEq, Serialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum Linked {
    File { index: usize, file: FileAttachment },
    Other,
}

/// Look at an absolute path the frontend named. Blocking: touches the file system.
pub fn resolve(path: &str) -> Result<Resolved, &'static str> {
    if path.is_empty() || path.len() > MAX_PATH_BYTES || path.contains('\0') {
        return Err("invalid path");
    }
    let raw = Path::new(path);
    if !raw.is_absolute() {
        return Err("path must be absolute");
    }
    // The real path, as the CLI's `-f` records it, so one file has one identity whatever it was
    // called. A path with nothing at it keeps its lexically tidied form.
    let real = std::fs::canonicalize(raw).unwrap_or_else(|_| tidy(raw));
    match std::fs::metadata(&real) {
        Ok(meta) if !meta.is_file() => Ok(Resolved::Other),
        meta => {
            let path = real.to_string_lossy().into_owned();
            let name = real
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| path.clone());
            Ok(Resolved::File(FileAttachment {
                is_image: crate::cli::file_attachment::is_image_ext(&path),
                size: meta.map_or(0, |meta| meta.len()),
                path,
                name,
            }))
        }
    }
}

/// Drop `.` and resolve `..` without touching the file system: the path may not exist.
fn tidy(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

#[derive(Default)]
pub struct LinkedFiles(Mutex<Inner>);

#[derive(Default)]
struct Inner {
    /// Popup request id → linked files, in registration order.
    popup: HashMap<String, Vec<FileAttachment>>,
    /// The History window has one owner at a time, like its read generation: registering for
    /// another entry drops the previous entry's files.
    history: Option<(String, Vec<FileAttachment>)>,
}

impl Inner {
    fn list(&mut self, owner: &str) -> &mut Vec<FileAttachment> {
        if owner.starts_with(HISTORY_OWNER_PREFIX) {
            if self.history.as_ref().is_none_or(|(id, _)| id != owner) {
                self.history = Some((owner.to_owned(), Vec::new()));
            }
            &mut self.history.as_mut().unwrap().1
        } else {
            self.popup.entry(owner.to_owned()).or_default()
        }
    }
}

impl LinkedFiles {
    /// The owner's linked files, in the order they follow its attachments.
    pub fn of(&self, owner: &str) -> Vec<FileAttachment> {
        let inner = self.0.lock().unwrap();
        if owner.starts_with(HISTORY_OWNER_PREFIX) {
            inner
                .history
                .as_ref()
                .filter(|(id, _)| id == owner)
                .map(|(_, files)| files.clone())
                .unwrap_or_default()
        } else {
            inner.popup.get(owner).cloned().unwrap_or_default()
        }
    }

    /// Register `file` after the owner's `attachments` and return its index in the owner's list.
    /// A path that is already an attachment, or already registered, keeps the index it has: one
    /// file is one entry, so its cache, reading state and selection are shared.
    pub fn add(
        &self,
        owner: &str,
        attachments: &[FileAttachment],
        file: FileAttachment,
    ) -> Result<usize, &'static str> {
        if let Some(index) = attachments.iter().position(|a| a.path == file.path) {
            return Ok(index);
        }
        let mut inner = self.0.lock().unwrap();
        let list = inner.list(owner);
        if let Some(position) = list.iter().position(|f| f.path == file.path) {
            return Ok(attachments.len() + position);
        }
        if list.len() >= MAX_PER_OWNER {
            return Err("too many linked files");
        }
        list.push(file);
        Ok(attachments.len() + list.len() - 1)
    }

    /// The owner is finished with: a Popup request that ended.
    pub fn forget(&self, owner: &str) {
        let mut inner = self.0.lock().unwrap();
        inner.popup.remove(owner);
        if inner.history.as_ref().is_some_and(|(id, _)| id == owner) {
            inner.history = None;
        }
    }

    /// The History window closed.
    pub fn forget_history(&self) {
        self.0.lock().unwrap().history = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(path: &str) -> FileAttachment {
        FileAttachment {
            path: path.into(),
            name: path.rsplit('/').next().unwrap().into(),
            size: 1,
            is_image: false,
        }
    }

    #[test]
    fn linked_files_follow_the_attachments_in_one_index_space() {
        let linked = LinkedFiles::default();
        let attachments = [file("/a/one.md"), file("/a/two.md")];
        assert_eq!(linked.add("r", &attachments, file("/b/x.md")), Ok(2));
        assert_eq!(linked.add("r", &attachments, file("/b/y.md")), Ok(3));
        assert_eq!(linked.of("r"), vec![file("/b/x.md"), file("/b/y.md")]);
        // Another owner has its own list; its indexes count from its own attachments.
        assert_eq!(
            linked.add("other", &attachments[..1], file("/b/x.md")),
            Ok(1)
        );
    }

    #[test]
    fn one_file_is_one_entry_whether_attached_or_linked_twice() {
        let linked = LinkedFiles::default();
        let attachments = [file("/a/one.md"), file("/a/two.md")];
        // Already an attachment: its own index, nothing registered.
        assert_eq!(linked.add("r", &attachments, file("/a/two.md")), Ok(1));
        assert!(linked.of("r").is_empty());
        assert_eq!(linked.add("r", &attachments, file("/b/x.md")), Ok(2));
        assert_eq!(linked.add("r", &attachments, file("/b/x.md")), Ok(2));
        assert_eq!(linked.of("r").len(), 1);
    }

    #[test]
    fn an_owner_may_register_a_bounded_number_of_files() {
        let linked = LinkedFiles::default();
        for i in 0..MAX_PER_OWNER {
            assert_eq!(linked.add("r", &[], file(&format!("/b/{i}.md"))), Ok(i));
        }
        assert!(linked.add("r", &[], file("/b/one-too-many.md")).is_err());
        // What is registered stays reachable, and another owner has its own budget.
        assert_eq!(linked.add("r", &[], file("/b/3.md")), Ok(3));
        assert_eq!(linked.add("s", &[], file("/b/one-too-many.md")), Ok(0));
    }

    #[test]
    fn owners_are_forgotten_one_by_one() {
        let linked = LinkedFiles::default();
        linked.add("r", &[], file("/b/x.md")).unwrap();
        linked.add("s", &[], file("/b/y.md")).unwrap();
        linked.add("history:1", &[], file("/b/z.md")).unwrap();
        linked.forget("r");
        assert!(linked.of("r").is_empty());
        assert_eq!(linked.of("s").len(), 1);
        // A Popup request id never reaches the History slot, nor the other way round.
        assert_eq!(linked.of("history:1").len(), 1);
        linked.forget("s");
        assert_eq!(linked.of("history:1").len(), 1);
        linked.forget_history();
        assert!(linked.of("history:1").is_empty());
    }

    #[test]
    fn the_history_window_keeps_one_entry_at_a_time() {
        let linked = LinkedFiles::default();
        linked.add("history:1", &[], file("/b/x.md")).unwrap();
        assert_eq!(linked.add("history:2", &[], file("/b/y.md")), Ok(0));
        assert!(linked.of("history:1").is_empty());
        assert_eq!(linked.of("history:2"), vec![file("/b/y.md")]);
        // Coming back starts from nothing: the frontend resets its own mirror on the same switch.
        assert_eq!(linked.add("history:1", &[], file("/b/y.md")), Ok(0));
        assert!(linked.of("history:2").is_empty());
    }

    #[test]
    fn rejects_paths_the_frontend_should_never_send() {
        assert!(resolve("").is_err());
        assert!(resolve("docs/report.md").is_err());
        assert!(resolve("./report.md").is_err());
        assert!(resolve("/tmp/a\0b").is_err());
        assert!(resolve(&format!("/{}", "a".repeat(MAX_PATH_BYTES))).is_err());
    }

    #[test]
    fn a_regular_file_resolves_to_its_real_path_with_its_facts() {
        let dir = tempfile::tempdir().unwrap();
        let real = std::fs::canonicalize(dir.path()).unwrap();
        std::fs::write(real.join("shot.PNG"), b"12345").unwrap();
        let Ok(Resolved::File(found)) = resolve(real.join("shot.PNG").to_str().unwrap()) else {
            panic!("a regular file resolves to a file");
        };
        assert_eq!(found.path, real.join("shot.PNG").to_string_lossy());
        assert_eq!(found.name, "shot.PNG");
        assert_eq!(found.size, 5);
        assert!(found.is_image);
    }

    #[cfg(unix)]
    #[test]
    fn a_symlink_and_its_target_are_one_file() {
        let dir = tempfile::tempdir().unwrap();
        let real = std::fs::canonicalize(dir.path()).unwrap();
        std::fs::write(real.join("run-1.log"), b"x").unwrap();
        std::os::unix::fs::symlink(real.join("run-1.log"), real.join("latest.log")).unwrap();
        let Ok(Resolved::File(by_link)) = resolve(real.join("latest.log").to_str().unwrap()) else {
            panic!("a symlink to a file resolves to a file");
        };
        let Ok(Resolved::File(direct)) = resolve(real.join("run-1.log").to_str().unwrap()) else {
            panic!("a regular file resolves to a file");
        };
        assert_eq!(by_link, direct);
        // So the CLI's canonical attachment and a link written through the symlink share an index.
        let linked = LinkedFiles::default();
        assert_eq!(
            linked.add("r", std::slice::from_ref(&direct), by_link),
            Ok(0)
        );
    }

    #[test]
    fn parent_segments_resolve_against_the_real_file_system_when_the_file_exists() {
        let dir = tempfile::tempdir().unwrap();
        let real = std::fs::canonicalize(dir.path()).unwrap();
        std::fs::create_dir(real.join("docs")).unwrap();
        std::fs::write(real.join("report.md"), b"x").unwrap();
        let spelled = format!("{}/docs/../report.md", real.display());
        let Ok(Resolved::File(found)) = resolve(&spelled) else {
            panic!("an existing file resolves to a file");
        };
        assert_eq!(found.path, real.join("report.md").to_string_lossy());
    }

    #[test]
    fn a_missing_file_is_still_registered_in_tidy_form() {
        let dir = tempfile::tempdir().unwrap();
        let real = std::fs::canonicalize(dir.path()).unwrap();
        let spelled = format!("{}/gone/./deeper/../report.md", real.display());
        let Ok(Resolved::File(found)) = resolve(&spelled) else {
            panic!("a missing file is still a file the panel can report on");
        };
        assert_eq!(
            found.path,
            real.join("gone").join("report.md").to_string_lossy()
        );
        assert_eq!(found.name, "report.md");
        assert_eq!(found.size, 0);
        assert!(!found.is_image);
    }

    #[test]
    fn a_directory_is_not_a_preview() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(resolve(dir.path().to_str().unwrap()), Ok(Resolved::Other));
    }

    #[cfg(unix)]
    #[test]
    fn tidy_never_climbs_above_the_root() {
        assert_eq!(tidy(Path::new("/a/b/../../../c")), PathBuf::from("/c"));
        assert_eq!(tidy(Path::new("/a/./b")), PathBuf::from("/a/b"));
    }
}
