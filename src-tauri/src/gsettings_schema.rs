//! Build-time GSettings schema discovery and process-startup path merging.
//!
//! This module is compiled by both `build.rs` and the application so its tests run through the
//! normal Cargo test target instead of only through build-script unit tests.

use std::collections::HashSet;
use std::ffi::OsString;
use std::path::PathBuf;

/// Capture only the schema data roots supplied by the Nix GLib setup hook.
pub fn schema_data_dirs(input: Option<OsString>) -> Result<String, String> {
    let Some(input) = input.filter(|value| !value.is_empty()) else {
        return Ok(String::new());
    };
    let text = input.to_str().ok_or("schema metadata must be UTF-8")?;
    if text.contains(['\n', '\r']) {
        return Err("schema metadata must not contain line breaks".into());
    }
    let mut seen = HashSet::new();
    let mut roots: Vec<PathBuf> = Vec::new();
    for root in std::env::split_paths(&input) {
        if root.as_os_str().is_empty() {
            continue;
        }
        if !root.is_absolute() {
            return Err(format!(
                "schema data root must be absolute: {}",
                root.display()
            ));
        }
        if !root.join("glib-2.0/schemas/gschemas.compiled").is_file() {
            return Err(format!(
                "compiled schema file missing under {}",
                root.display()
            ));
        }
        if seen.insert(root.clone()) {
            roots.push(root);
        }
    }
    if roots.is_empty() {
        return Err("nonempty schema metadata contains no data roots".into());
    }
    std::env::join_paths(roots)
        .map_err(|error| error.to_string())?
        .into_string()
        .map_err(|_| "schema metadata must be UTF-8".into())
}

/// Prefix compiled schema roots while preserving the process's existing XDG lookup order.
#[cfg_attr(not(test), allow(dead_code))]
pub fn merged_data_dirs(compiled: &str, existing: Option<&std::ffi::OsStr>) -> Option<OsString> {
    let mut seen = HashSet::new();
    let mut dirs: Vec<PathBuf> = std::env::split_paths(compiled)
        .filter(|path| !path.as_os_str().is_empty() && seen.insert(path.clone()))
        .collect();
    if dirs.is_empty() {
        return None;
    }
    let existing = existing
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| std::ffi::OsStr::new("/usr/local/share:/usr/share"));
    // Preserve user directory order and duplicates except copies of our own prefix.
    dirs.extend(std::env::split_paths(existing).filter(|path| !seen.contains(path)));
    Some(std::env::join_paths(dirs).expect("split path components must be joinable"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsStr;
    use std::sync::atomic::{AtomicU64, Ordering};

    struct Fixture(PathBuf);

    impl Fixture {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let root = std::env::temp_dir().join(format!(
                "askhuman-schema-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            let schemas = root.join("glib-2.0/schemas");
            std::fs::create_dir_all(&schemas).unwrap();
            std::fs::write(schemas.join("gschemas.compiled"), b"fixture").unwrap();
            Self(root)
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            // Remove only files and directories created by this fixture.
            let schemas = self.0.join("glib-2.0/schemas");
            std::fs::remove_file(schemas.join("gschemas.compiled")).unwrap();
            std::fs::remove_dir(&schemas).unwrap();
            std::fs::remove_dir(self.0.join("glib-2.0")).unwrap();
            std::fs::remove_dir(&self.0).unwrap();
        }
    }

    #[test]
    fn absent_and_empty_metadata_are_optional() {
        assert_eq!(schema_data_dirs(None).unwrap(), "");
        assert_eq!(schema_data_dirs(Some(OsString::new())).unwrap(), "");
    }

    #[test]
    fn keeps_declared_order_and_deduplicates_roots() {
        let first = Fixture::new();
        let second = Fixture::new();
        let input = std::env::join_paths([&first.0, &second.0, &first.0]).unwrap();
        let expected = std::env::join_paths([&first.0, &second.0]).unwrap();
        assert_eq!(
            schema_data_dirs(Some(input)).unwrap(),
            expected.to_str().unwrap()
        );
    }

    #[test]
    fn rejects_relative_and_missing_roots() {
        assert!(schema_data_dirs(Some("relative".into()))
            .unwrap_err()
            .contains("absolute"));
        let fixture = Fixture::new();
        let missing = fixture.0.join("missing");
        assert!(schema_data_dirs(Some(missing.into_os_string()))
            .unwrap_err()
            .contains("missing"));
    }

    #[test]
    fn rejects_line_breaks_and_only_empty_segments() {
        assert!(schema_data_dirs(Some("/a\n/b".into()))
            .unwrap_err()
            .contains("line breaks"));
        assert!(schema_data_dirs(Some(":".into()))
            .unwrap_err()
            .contains("no data roots"));
    }

    #[cfg(unix)]
    #[test]
    fn rejects_non_utf8_metadata() {
        use std::os::unix::ffi::OsStringExt;
        assert!(schema_data_dirs(Some(OsString::from_vec(vec![0xff])))
            .unwrap_err()
            .contains("UTF-8"));
    }

    #[test]
    fn absent_compiled_metadata_does_not_change_environment() {
        assert_eq!(merged_data_dirs("", None), None);
        assert_eq!(merged_data_dirs("", Some(OsStr::new("/custom"))), None);
    }

    #[test]
    fn preserves_standard_defaults_for_unset_or_empty_xdg() {
        let expected = Some(OsString::from("/schema:/usr/local/share:/usr/share"));
        assert_eq!(merged_data_dirs("/schema", None), expected);
        assert_eq!(merged_data_dirs("/schema", Some(OsStr::new(""))), expected);
    }

    #[test]
    fn preserves_user_order_and_non_schema_duplicates() {
        assert_eq!(
            merged_data_dirs("/gtk:/desktop", Some(OsStr::new("/b:/a:/b"))),
            Some(OsString::from("/gtk:/desktop:/b:/a:/b"))
        );
    }

    #[test]
    fn self_spawn_and_reexec_do_not_accumulate_prefixes() {
        let first =
            merged_data_dirs("/gtk:/desktop:/gtk", Some(OsStr::new("/custom:/gtk"))).unwrap();
        let second = merged_data_dirs("/gtk:/desktop", Some(&first)).unwrap();
        assert_eq!(first, OsString::from("/gtk:/desktop:/custom"));
        assert_eq!(first, second);
    }

    #[test]
    fn a_new_binary_prefix_precedes_inherited_older_metadata() {
        assert_eq!(
            merged_data_dirs("/new", Some(OsStr::new("/old:/custom"))),
            Some(OsString::from("/new:/old:/custom"))
        );
    }
}
