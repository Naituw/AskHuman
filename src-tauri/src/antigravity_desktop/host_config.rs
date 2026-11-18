//! Antigravity's `~/.gemini/config/config.json`, as far as AskHuman may touch it: the entries under
//! `sidecars.askhuman-*` that switch AskHuman's sidecars on and off.
//!
//! The file also holds what the user decided inside Antigravity (the permissions they allowed,
//! which plugins are on) and the App writes it too, so every edit here is a read-modify-write that
//! leaves all other bytes of the file as they were, replaces the file in one `rename` (the App
//! once logged a parse failure after a truncate-and-write of it), and starts over when the file
//! changed between the read and the swap. Only one process of AskHuman, the daemon, edits it.

use anyhow::{anyhow, bail, Context, Result};
use jsonc_parser::cst::CstRootNode;
use jsonc_parser::{json, ParseOptions};
use serde_json::Value;
use std::path::{Path, PathBuf};

/// How often an edit starts over because the file changed under it.
const ATTEMPTS: usize = 3;

/// The only sidecar ids this module edits.
pub(super) const PREFIX: &str = "askhuman-";

/// What the App says about one sidecar of ours. It adds the project id when it first starts it.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(super) struct Entry {
    pub enabled: bool,
    pub project_id: Option<String>,
}

/// The file is not (yet) valid JSON: partly written by the App, or damaged.
#[derive(Debug)]
struct Unparseable(String);

impl std::fmt::Display for Unparseable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Antigravity's config.json is not valid JSON: {}", self.0)
    }
}

impl std::error::Error for Unparseable {}

fn check_id(id: &str) -> Result<()> {
    if id.starts_with(PREFIX)
        && id.len() > PREFIX.len()
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
    {
        Ok(())
    } else {
        bail!("{id:?} is not an AskHuman sidecar id")
    }
}

fn parse(text: &str) -> Result<CstRootNode> {
    let source = if text.trim().is_empty() { "{}" } else { text };
    // A file the JSON parser refuses must not be edited on the strength of a laxer one.
    serde_json::from_str::<Value>(source).map_err(|error| Unparseable(error.to_string()))?;
    CstRootNode::parse(source, &ParseOptions::default())
        .map_err(|error| Unparseable(error.to_string()).into())
}

/// What `config.json` says about sidecar `id`.
pub(super) fn read_entry(text: &str, id: &str) -> Option<Entry> {
    let value: Value = serde_json::from_str(text).ok()?;
    let entry = value.get("sidecars")?.get(id)?;
    Some(Entry {
        enabled: entry.get("enabled") == Some(&Value::Bool(true)),
        project_id: entry
            .get("projectId")
            .and_then(Value::as_str)
            .map(str::to_string),
    })
}

/// `text` with `sidecars.<id>.enabled` true, or `None` when it already is. Everything else in the
/// file, including what the App added to the entry, stays as it was.
pub(super) fn enabled_text(text: &str, id: &str) -> Result<Option<String>> {
    check_id(id)?;
    let root = parse(text)?;
    let object = root
        .object_value_or_create()
        .ok_or_else(|| anyhow!("Antigravity's config.json is not a JSON object"))?;
    let sidecars = object
        .object_value_or_create("sidecars")
        .ok_or_else(|| anyhow!("Antigravity's config.json: \"sidecars\" is not an object"))?;
    let entry = sidecars.object_value_or_create(id).ok_or_else(|| {
        anyhow!("Antigravity's config.json: the entry of sidecar {id} is not an object")
    })?;
    match entry.get("enabled") {
        Some(prop) => prop.set_value(json!(true)),
        None => {
            entry.append("enabled", json!(true));
        }
    }
    let edited = root.to_string();
    Ok((edited != text).then_some(edited))
}

/// `text` without the entry of sidecar `id`, or `None` when there is none.
pub(super) fn removed_text(text: &str, id: &str) -> Result<Option<String>> {
    check_id(id)?;
    let root = parse(text)?;
    let Some(sidecars) = root
        .object_value()
        .and_then(|object| object.object_value("sidecars"))
    else {
        return Ok(None);
    };
    let Some(prop) = sidecars.get(id) else {
        return Ok(None);
    };
    prop.remove();
    Ok(Some(root.to_string()))
}

fn read_text(path: &Path) -> Result<String> {
    match std::fs::read(path) {
        Ok(bytes) => {
            String::from_utf8(bytes).map_err(|_| Unparseable("it is not valid UTF-8".into()).into())
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(error) => Err(error).with_context(|| format!("Could not read {}", path.display())),
    }
}

/// Replaces `path` with `bytes` in one `rename`, keeping the file's permissions (0600 for a file
/// that does not exist yet) and writing through a symbolic link instead of replacing it.
fn replace_file(path: &Path, bytes: &[u8]) -> Result<()> {
    use std::io::Write;
    let target = if path.is_symlink() {
        std::fs::canonicalize(path)
            .with_context(|| format!("Could not resolve {}", path.display()))?
    } else {
        path.to_path_buf()
    };
    let parent = target
        .parent()
        .ok_or_else(|| anyhow!("{} has no parent directory", target.display()))?;
    if !parent.is_dir() {
        bail!("{} does not exist", parent.display());
    }
    #[cfg(unix)]
    let mode = {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(&target)
            .map(|meta| meta.permissions().mode() & 0o777)
            .unwrap_or(0o600)
    };
    let name = target
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let temporary: PathBuf = parent.join(format!(".{name}.askhuman-{}.tmp", uuid::Uuid::new_v4()));
    let written = (|| -> std::io::Result<()> {
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(mode);
        }
        let mut file = options.open(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&temporary, &target)
    })();
    if written.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    written.with_context(|| format!("Could not write {}", target.display()))
}

/// Whether an edit changed the file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Outcome {
    Unchanged,
    Written,
}

/// Applies `change` to the current text of the file at `path`. `change` returns the new text, or
/// `None` when there is nothing to change; it runs again on what a write left behind, which must
/// then need no change (or the App overwrote the edit and the edit starts over). When the file
/// keeps changing, nothing is forced: the edit fails after [`ATTEMPTS`] tries.
pub(super) fn edit(
    path: &Path,
    change: &dyn Fn(&str) -> Result<Option<String>>,
) -> Result<Outcome> {
    edit_with(path, change, &mut || {})
}

/// [`edit`] with a hook that runs after the text was read and edited and before the file is
/// compared and replaced: where the App's own write may land.
fn edit_with(
    path: &Path,
    change: &dyn Fn(&str) -> Result<Option<String>>,
    between: &mut dyn FnMut(),
) -> Result<Outcome> {
    let mut unparseable = None;
    for _ in 0..ATTEMPTS {
        let before = match read_text(path) {
            Ok(text) => text,
            Err(error) if error.downcast_ref::<Unparseable>().is_some() => {
                unparseable = Some(error);
                continue;
            }
            Err(error) => return Err(error),
        };
        let after = match change(&before) {
            Ok(Some(after)) => after,
            Ok(None) => return Ok(Outcome::Unchanged),
            Err(error) if error.downcast_ref::<Unparseable>().is_some() => {
                unparseable = Some(error);
                continue;
            }
            Err(error) => return Err(error),
        };
        // Nothing of ours may leave this function unless it is valid JSON.
        serde_json::from_str::<Value>(&after)
            .context("The edited config.json is not valid JSON")?;
        between();
        if read_text(path).ok().as_deref() != Some(before.as_str()) {
            continue;
        }
        replace_file(path, after.as_bytes())?;
        if matches!(change(&read_text(path)?), Ok(None)) {
            return Ok(Outcome::Written);
        }
    }
    match unparseable {
        Some(error) => Err(error),
        None => bail!(
            "Antigravity's config.json kept changing while it was being edited; nothing more was written. Try again."
        ),
    }
}

/// Switches sidecar `id` on.
pub(super) fn enable(path: &Path, id: &str) -> Result<Outcome> {
    edit(path, &|text| enabled_text(text, id))
}

/// Removes the entry of sidecar `id`.
pub(super) fn remove(path: &Path, id: &str) -> Result<Outcome> {
    edit(path, &|text| removed_text(text, id))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    const ID: &str = "askhuman-ws-1a2b3c4d";

    /// The shape of a real file: the App's plugin entries and the user's permissions.
    const REAL: &str = r#"{
  "plugins": {
    "swarm": {
      "enabled": true
    }
  },
  "sidecars": {
    "swarm/swarm_board": {
      "projectId": "00000000-0000-4000-8000-000000000001",
      "resolvedPermissions": {},
      "schemaVersion": 1
    }
  },
  "userSettings": {
    "globalPermissionGrants": {
      "allow": [
        "command(ls)"
      ]
    },
    "remoteControlHostname": "example"
  }
}
"#;

    fn file(text: &str) -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        std::fs::write(&path, text).unwrap();
        (dir, path)
    }

    #[test]
    fn enabling_a_sidecar_touches_nothing_else_in_the_file() {
        let edited = enabled_text(REAL, ID).unwrap().unwrap();
        let value: Value = serde_json::from_str(&edited).unwrap();
        assert_eq!(value["sidecars"][ID]["enabled"], true);
        // The user's permissions and the App's own entries are as they were, key for key.
        let original: Value = serde_json::from_str(REAL).unwrap();
        let mut now = value.clone();
        now["sidecars"].as_object_mut().unwrap().remove(ID);
        assert_eq!(now, original);
        // And the bytes before the new entry are untouched, formatting included.
        let prefix = &REAL[..REAL.find("    \"swarm/swarm_board\"").unwrap()];
        assert!(edited.starts_with(prefix));
        // Enabling again is a no-op.
        assert_eq!(enabled_text(&edited, ID).unwrap(), None);
    }

    #[test]
    fn what_the_app_added_to_our_entry_survives_and_a_disabled_entry_is_switched_on() {
        let text = r#"{
  "sidecars": {
    "askhuman-ws-1a2b3c4d": {
      "enabled": false,
      "projectId": "p-1",
      "resolvedPermissions": {"grants": ["mcp(askhuman_mcp/*)"]},
      "schemaVersion": 1
    }
  }
}"#;
        let edited = enabled_text(text, ID).unwrap().unwrap();
        let value: Value = serde_json::from_str(&edited).unwrap();
        assert_eq!(value["sidecars"][ID]["enabled"], true);
        assert_eq!(value["sidecars"][ID]["projectId"], "p-1");
        assert_eq!(
            value["sidecars"][ID]["resolvedPermissions"]["grants"][0],
            "mcp(askhuman_mcp/*)"
        );
        assert_eq!(
            read_entry(&edited, ID),
            Some(Entry {
                enabled: true,
                project_id: Some("p-1".into())
            })
        );
    }

    #[test]
    fn a_missing_or_empty_file_gets_just_our_entry() {
        for text in ["", "  \n", "{}"] {
            let edited = enabled_text(text, ID).unwrap().unwrap();
            let value: Value = serde_json::from_str(&edited).unwrap();
            assert_eq!(
                value,
                serde_json::json!({"sidecars": {ID: {"enabled": true}}})
            );
        }
    }

    #[test]
    fn removing_deletes_our_entry_only_and_says_nothing_to_do_when_it_is_gone() {
        let with = enabled_text(REAL, ID).unwrap().unwrap();
        let removed = removed_text(&with, ID).unwrap().unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(&removed).unwrap(),
            serde_json::from_str::<Value>(REAL).unwrap()
        );
        assert_eq!(removed_text(REAL, ID).unwrap(), None);
        assert_eq!(removed_text("{}", ID).unwrap(), None);
        assert_eq!(removed_text("", ID).unwrap(), None);
    }

    #[test]
    fn only_askhuman_sidecars_can_be_named_and_the_files_shape_is_checked() {
        for id in [
            "swarm/swarm_board",
            "askhuman-",
            "askhuman-../x",
            "other",
            "",
        ] {
            assert!(enabled_text("{}", id).is_err(), "{id:?}");
            assert!(removed_text("{}", id).is_err(), "{id:?}");
        }
        assert!(enabled_text("[]", ID).is_err());
        assert!(enabled_text(r#"{"sidecars": []}"#, ID).is_err());
        assert!(enabled_text(r#"{"sidecars": {"askhuman-ws-1a2b3c4d": 3}}"#, ID).is_err());
        // Not JSON at all is told apart from a wrong shape: it may be the App mid-write.
        let error = enabled_text("{\"sidecars\": {", ID).unwrap_err();
        assert!(error.downcast_ref::<Unparseable>().is_some());
        // JSON with comments is accepted by neither this nor the App's parser.
        assert!(enabled_text("{ /* c */ }", ID).is_err());
    }

    #[test]
    fn an_edit_replaces_the_file_keeps_its_mode_and_leaves_no_temporary_file() {
        let (dir, path) = file(REAL);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o640)).unwrap();
        }
        assert_eq!(enable(&path, ID).unwrap(), Outcome::Written);
        assert_eq!(enable(&path, ID).unwrap(), Outcome::Unchanged);
        let value: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(value["sidecars"][ID]["enabled"], true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o640
            );
        }
        assert_eq!(remove(&path, ID).unwrap(), Outcome::Written);
        assert_eq!(remove(&path, ID).unwrap(), Outcome::Unchanged);
        let names: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, vec!["config.json".to_string()]);
    }

    #[test]
    fn a_file_that_does_not_exist_yet_is_created_private() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        assert_eq!(enable(&path, ID).unwrap(), Outcome::Written);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        // Without a directory there is nowhere to write, and nothing is invented.
        let missing = dir.path().join("absent").join("config.json");
        assert!(enable(&missing, ID).is_err());
        assert!(!dir.path().join("absent").exists());
    }

    #[cfg(unix)]
    #[test]
    fn a_symbolic_link_is_written_through_and_stays_a_link() {
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("dotfiles-config.json");
        std::fs::write(&real, REAL).unwrap();
        let link = dir.path().join("config.json");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        assert_eq!(enable(&link, ID).unwrap(), Outcome::Written);
        assert!(link.is_symlink());
        let value: Value = serde_json::from_str(&std::fs::read_to_string(&real).unwrap()).unwrap();
        assert_eq!(value["sidecars"][ID]["enabled"], true);
    }

    #[test]
    fn a_write_by_the_app_between_read_and_swap_is_not_lost() {
        let (_dir, path) = file(REAL);
        let app = Cell::new(true);
        // The App adds a grant after we read but before we swap, once.
        let mut between = || {
            if app.replace(false) {
                let mut value: Value =
                    serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
                value["userSettings"]["globalPermissionGrants"]["allow"]
                    .as_array_mut()
                    .unwrap()
                    .push("command(AskHuman)".into());
                std::fs::write(&path, serde_json::to_string_pretty(&value).unwrap()).unwrap();
            }
        };
        let outcome = edit_with(&path, &|text| enabled_text(text, ID), &mut between).unwrap();
        assert_eq!(outcome, Outcome::Written);
        let value: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(value["sidecars"][ID]["enabled"], true);
        assert_eq!(
            value["userSettings"]["globalPermissionGrants"]["allow"],
            serde_json::json!(["command(ls)", "command(AskHuman)"])
        );
    }

    #[test]
    fn a_file_that_keeps_changing_is_never_forced() {
        let (_dir, path) = file(REAL);
        let writes = Cell::new(0);
        let mut between = || {
            writes.set(writes.get() + 1);
            let mut value: Value =
                serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
            value["userSettings"]["remoteControlHostname"] =
                format!("host-{}", writes.get()).into();
            std::fs::write(&path, serde_json::to_string_pretty(&value).unwrap()).unwrap();
        };
        let error = edit_with(&path, &|text| enabled_text(text, ID), &mut between).unwrap_err();
        assert!(error.to_string().contains("kept changing"), "{error}");
        assert_eq!(writes.get(), ATTEMPTS);
        // Nothing of ours was written.
        let value: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert!(value["sidecars"].get(ID).is_none());
    }

    #[test]
    fn an_edit_the_app_overwrites_straight_away_starts_over() {
        let (_dir, path) = file(REAL);
        let calls = Cell::new(0);
        let change = |text: &str| -> Result<Option<String>> {
            calls.set(calls.get() + 1);
            if calls.get() == 2 {
                // The check right after our swap: the App has just written the copy it held in
                // memory over our edit, so the file is as it was and the edit is still to do.
                std::fs::write(&path, REAL).unwrap();
                return enabled_text(REAL, ID);
            }
            enabled_text(text, ID)
        };
        assert_eq!(edit(&path, &change).unwrap(), Outcome::Written);
        // Edit, check (overwritten), edit again, check (stands).
        assert_eq!(calls.get(), 4);
        let value: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(value["sidecars"][ID]["enabled"], true);
    }

    #[test]
    fn a_file_that_is_not_json_is_reported_not_overwritten() {
        let (_dir, path) = file("{\"sidecars\": {");
        let error = enable(&path, ID).unwrap_err();
        assert!(error.downcast_ref::<Unparseable>().is_some(), "{error}");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{\"sidecars\": {");
    }

    #[test]
    fn a_reader_never_sees_a_torn_file_while_it_is_edited() {
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::sync::Arc;
        let (_dir, path) = file(REAL);
        let done = Arc::new(AtomicBool::new(false));
        let reader = {
            let (path, done) = (path.clone(), done.clone());
            std::thread::spawn(move || {
                let mut reads = 0_u32;
                while !done.load(Ordering::Relaxed) {
                    let text = std::fs::read_to_string(&path).unwrap();
                    serde_json::from_str::<Value>(&text)
                        .unwrap_or_else(|error| panic!("torn read: {error}\n{text}"));
                    reads += 1;
                }
                reads
            })
        };
        for round in 0..150 {
            let id = format!("askhuman-ws-{round:08x}");
            enable(&path, &id).unwrap();
            if round % 2 == 1 {
                remove(&path, &id).unwrap();
            }
        }
        done.store(true, Ordering::Relaxed);
        assert!(reader.join().unwrap() > 0);
        let value: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(value["sidecars"].as_object().unwrap().len(), 1 + 75);
        assert_eq!(
            value["userSettings"]["globalPermissionGrants"]["allow"][0],
            "command(ls)"
        );
    }
}
