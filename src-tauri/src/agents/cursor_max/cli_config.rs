//! The Cursor CLI's own configuration (`cli-config.json`), as far as Max mode is concerned.
//!
//! Two jobs, both narrow on purpose (docs/specs/cursor-max-control.md §4):
//!
//! * [`read_state`] reads the few whitelisted keys that decide which model a launch will use and
//!   whether Max is switched on. The file also holds the account's credentials; they are never
//!   deserialized, kept, or logged.
//! * [`force_off`] turns the two persisted Max toggles off with a minimal textual edit: only the
//!   `true` literals change, every other byte stays, so the CLI's own formatting and any key this
//!   code does not know about survive. The result is re-parsed and compared with the original
//!   before it replaces the file.

use std::ffi::OsString;
use std::io::Write;
use std::ops::Range;
use std::path::{Path, PathBuf};

use serde_json::Value;

use super::catalog::Selection;

/// The CLI's config file is a few kilobytes; a much larger one is not touched at all.
const MAX_BYTES: u64 = 1024 * 1024;
const FILE_NAME: &str = "cli-config.json";

/// What the config says about the model a launch will use and about Max.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CliState {
    /// Top-level `maxMode`.
    pub max_mode: bool,
    /// `model.maxMode`.
    pub model_max_mode: bool,
    /// `selectedModel` (the model id and its parameters), if any.
    pub selected: Option<Selection>,
    /// `modelParameters`: the parameters last used per model id.
    pub saved: Vec<(String, Vec<(String, String)>)>,
}

impl CliState {
    pub fn persisted_max(&self) -> bool {
        self.max_mode || self.model_max_mode
    }

    /// The parameters to try for a model id when the config does not select it directly.
    pub fn saved_for(&self, model: &str) -> Option<&[(String, String)]> {
        self.saved
            .iter()
            .find(|(id, _)| id == model)
            .map(|(_, params)| params.as_slice())
    }
}

/// Where the CLI keeps its config: `CURSOR_CONFIG_DIR`, else `XDG_CONFIG_HOME/cursor`, else
/// `~/.cursor` (the same order the CLI uses).
pub fn config_dir(
    env: impl Fn(&str) -> Option<OsString>,
    home: Option<PathBuf>,
) -> Option<PathBuf> {
    let set = |name: &str| env(name).filter(|value| !value.to_string_lossy().trim().is_empty());
    if let Some(dir) = set("CURSOR_CONFIG_DIR") {
        return Some(PathBuf::from(dir));
    }
    if let Some(xdg) = set("XDG_CONFIG_HOME") {
        return Some(PathBuf::from(xdg).join("cursor"));
    }
    home.map(|home| home.join(".cursor"))
}

/// The config file of the current process' environment.
pub fn config_path() -> Option<PathBuf> {
    config_dir(|name| std::env::var_os(name), dirs::home_dir()).map(|dir| dir.join(FILE_NAME))
}

fn params(value: Option<&Value>) -> Vec<(String, String)> {
    value
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .take(16)
                .filter_map(|item| {
                    let id = item.get("id")?.as_str()?.trim();
                    let value = item.get("value")?.as_str()?.trim();
                    (!id.is_empty() && id.len() <= 160 && value.len() <= 160)
                        .then(|| (id.to_string(), value.to_string()))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Reads the whitelisted keys. `None` when the text is not a JSON object.
pub fn read_state(text: &str) -> Option<CliState> {
    #[derive(serde::Deserialize, Default)]
    #[serde(default)]
    struct Raw {
        #[serde(rename = "maxMode")]
        max_mode: Option<Value>,
        model: Option<Value>,
        #[serde(rename = "selectedModel")]
        selected_model: Option<Value>,
        #[serde(rename = "modelParameters")]
        model_parameters: Option<Value>,
    }
    // Typed fields only: everything else in the file, credentials included, is skipped.
    let raw: Raw = serde_json::from_str(text).ok()?;
    let truthy = |value: Option<&Value>| value.and_then(Value::as_bool).unwrap_or(false);
    let max_mode = truthy(raw.max_mode.as_ref());
    let model_max_mode = truthy(raw.model.as_ref().and_then(|model| model.get("maxMode")));
    let saved: Vec<(String, Vec<(String, String)>)> = raw
        .model_parameters
        .as_ref()
        .and_then(Value::as_object)
        .map(|map| {
            map.iter()
                .take(400)
                .map(|(id, list)| (id.clone(), params(Some(list))))
                .collect()
        })
        .unwrap_or_default();
    let model_id = |value: Option<&Value>| {
        let id = value?.get("modelId")?.as_str()?.trim();
        (!id.is_empty() && id.len() <= 160).then(|| id.to_string())
    };
    // `selectedModel` is what the CLI sends; the older `model` section only names the model, and
    // the parameters it last used are in `modelParameters`.
    let selected = match model_id(raw.selected_model.as_ref()) {
        Some(model) => Some(Selection {
            params: params(
                raw.selected_model
                    .as_ref()
                    .and_then(|s| s.get("parameters")),
            ),
            model,
            max_mode,
        }),
        None => model_id(raw.model.as_ref()).map(|model| Selection {
            params: saved
                .iter()
                .find(|(id, _)| *id == model)
                .map(|(_, params)| params.clone())
                .unwrap_or_default(),
            model,
            max_mode,
        }),
    };
    Some(CliState {
        max_mode,
        model_max_mode,
        selected,
        saved,
    })
}

/// Reads and parses the config at `path` within the size cap.
pub fn read_state_at(path: &Path) -> Option<CliState> {
    let text = read_capped(path).ok()?;
    read_state(&text)
}

fn read_capped(path: &Path) -> std::io::Result<String> {
    let meta = std::fs::metadata(path)?;
    if meta.len() > MAX_BYTES {
        return Err(std::io::Error::other("config file too large"));
    }
    std::fs::read_to_string(path)
}

/// Why the config could not be rewritten.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RewriteError {
    NotJson,
    NotObject,
    /// The text has a shape this edit does not dare to touch (duplicate keys, unexpected nesting).
    Ambiguous,
    /// The edited text did not parse back to the original with only the toggles changed.
    Unverified,
}

/// Turns the top-level `maxMode` and `model.maxMode` from `true` to `false` in place.
/// `Ok(None)` when neither is on.
pub fn rewrite_off(text: &str) -> Result<Option<String>, RewriteError> {
    let value: Value = serde_json::from_str(text).map_err(|_| RewriteError::NotJson)?;
    let root = value.as_object().ok_or(RewriteError::NotObject)?;
    let on = |value: Option<&Value>| value == Some(&Value::Bool(true));
    let top = on(root.get("maxMode"));
    let nested = on(root.get("model").and_then(|model| model.get("maxMode")));
    if !top && !nested {
        return Ok(None);
    }

    let start = text
        .find(|c: char| !c.is_whitespace())
        .ok_or(RewriteError::NotObject)?;
    let members = object_members(text, start).ok_or(RewriteError::Ambiguous)?;
    let mut edits: Vec<Range<usize>> = Vec::new();
    if top {
        edits.push(unique_true(text, &members, "maxMode")?);
    }
    if nested {
        let model = unique(&members, "model")?;
        let inner = object_members(text, model.start).ok_or(RewriteError::Ambiguous)?;
        edits.push(unique_true(text, &inner, "maxMode")?);
    }
    edits.sort_by_key(|range| std::cmp::Reverse(range.start));
    let mut out = text.to_string();
    for range in edits {
        out.replace_range(range, "false");
    }

    let mut expected = value.clone();
    if top {
        expected["maxMode"] = Value::Bool(false);
    }
    if nested {
        expected["model"]["maxMode"] = Value::Bool(false);
    }
    let reparsed: Value = serde_json::from_str(&out).map_err(|_| RewriteError::Unverified)?;
    if reparsed != expected {
        return Err(RewriteError::Unverified);
    }
    Ok(Some(out))
}

type Members = Vec<(String, Range<usize>)>;

fn unique(members: &Members, key: &str) -> Result<Range<usize>, RewriteError> {
    let mut found = members.iter().filter(|(name, _)| name == key);
    match (found.next(), found.next()) {
        (Some((_, range)), None) => Ok(range.clone()),
        _ => Err(RewriteError::Ambiguous),
    }
}

fn unique_true(text: &str, members: &Members, key: &str) -> Result<Range<usize>, RewriteError> {
    let range = unique(members, key)?;
    if &text[range.clone()] == "true" {
        Ok(range)
    } else {
        Err(RewriteError::Ambiguous)
    }
}

fn skip_ws(bytes: &[u8], mut at: usize) -> usize {
    while at < bytes.len() && bytes[at].is_ascii_whitespace() {
        at += 1;
    }
    at
}

/// The index just past the string that opens at `start`.
fn string_end(bytes: &[u8], start: usize) -> Option<usize> {
    if bytes.get(start) != Some(&b'"') {
        return None;
    }
    let mut at = start + 1;
    while at < bytes.len() {
        match bytes[at] {
            b'\\' => at += 2,
            b'"' => return Some(at + 1),
            _ => at += 1,
        }
    }
    None
}

/// The index just past the JSON value that starts at `start`.
fn value_end(bytes: &[u8], start: usize) -> Option<usize> {
    match *bytes.get(start)? {
        b'"' => string_end(bytes, start),
        b'{' | b'[' => {
            let mut depth = 0usize;
            let mut at = start;
            while at < bytes.len() {
                match bytes[at] {
                    b'"' => {
                        at = string_end(bytes, at)?;
                        continue;
                    }
                    b'{' | b'[' => depth += 1,
                    b'}' | b']' => {
                        depth = depth.checked_sub(1)?;
                        if depth == 0 {
                            return Some(at + 1);
                        }
                    }
                    _ => {}
                }
                at += 1;
            }
            None
        }
        _ => {
            let mut at = start;
            while at < bytes.len()
                && !matches!(bytes[at], b',' | b'}' | b']')
                && !bytes[at].is_ascii_whitespace()
            {
                at += 1;
            }
            (at > start).then_some(at)
        }
    }
}

/// The members of the object that opens at `start`, with the byte range of each value.
fn object_members(text: &str, start: usize) -> Option<Members> {
    let bytes = text.as_bytes();
    if bytes.get(start) != Some(&b'{') {
        return None;
    }
    let mut members = Vec::new();
    let mut at = skip_ws(bytes, start + 1);
    if bytes.get(at) == Some(&b'}') {
        return Some(members);
    }
    loop {
        let key_end = string_end(bytes, at)?;
        let key: String = serde_json::from_str(&text[at..key_end]).ok()?;
        at = skip_ws(bytes, key_end);
        if bytes.get(at) != Some(&b':') {
            return None;
        }
        at = skip_ws(bytes, at + 1);
        let end = value_end(bytes, at)?;
        members.push((key, at..end));
        at = skip_ws(bytes, end);
        match bytes.get(at)? {
            b',' => at = skip_ws(bytes, at + 1),
            b'}' => return Some(members),
            _ => return None,
        }
    }
}

/// How a [`force_off`] ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ForceOff {
    /// Neither toggle was on.
    Unchanged,
    /// At least one toggle was on and has been turned off.
    Turned,
}

/// Why [`force_off`] gave up. Nothing was written in any of these cases.
#[derive(Debug)]
pub enum ForceOffError {
    Io(std::io::Error),
    Rewrite(RewriteError),
    /// The file changed between reading and replacing it, twice in a row.
    Busy,
}

impl std::fmt::Display for ForceOffError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(error) => write!(f, "{error}"),
            Self::Rewrite(error) => write!(f, "the config has an unexpected shape ({error:?})"),
            Self::Busy => write!(f, "the config kept changing while it was being updated"),
        }
    }
}

impl std::error::Error for ForceOffError {}

impl From<std::io::Error> for ForceOffError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

/// Turns the persisted Max toggles in the config at `path` off. Serialized between AskHuman
/// processes by `lock`; written through a temporary file next to the real target and renamed over
/// it, keeping the original's permissions. A file the CLI rewrites in between is read again once.
pub fn force_off(path: &Path, lock: &Path) -> Result<ForceOff, ForceOffError> {
    let _guard = crate::file_lock::FileLock::exclusive(lock)?;
    // A symlinked config (dotfiles managers do that) is edited where it really lives.
    let target = std::fs::canonicalize(path)?;
    for _ in 0..2 {
        let before = std::fs::metadata(&target)?;
        let text = read_capped(&target)?;
        let Some(updated) = rewrite_off(&text).map_err(ForceOffError::Rewrite)? else {
            return Ok(ForceOff::Unchanged);
        };
        let dir = target
            .parent()
            .ok_or_else(|| std::io::Error::other("no parent directory"))?;
        let mut temp = tempfile::Builder::new()
            .prefix(".cli-config.askhuman-")
            .suffix(".tmp")
            .tempfile_in(dir)?;
        temp.write_all(updated.as_bytes())?;
        temp.as_file().sync_all()?;
        std::fs::set_permissions(temp.path(), before.permissions())?;
        let after = std::fs::metadata(&target)?;
        if after.len() != before.len() || after.modified()? != before.modified()? {
            continue;
        }
        temp.persist(&target).map_err(|error| error.error)?;
        return Ok(ForceOff::Turned);
    }
    Err(ForceOffError::Busy)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{
  "version": 1,
  "authInfo": {"accessToken": "SECRET-TOKEN", "email": "a@b.c"},
  "maxMode": true,
  "model": {"modelId": "grok-4.7", "maxMode": true, "displayName": "Grok \"4.7\" { [x] }"},
  "selectedModel": {"modelId": "grok-4.7", "parameters": [{"id": "context", "value": "256k"}, {"id": "effort", "value": "high"}]},
  "modelParameters": {"grok-4.7": [{"id": "fast", "value": "true"}]},
  "other": [true, {"maxMode": true}]
}
"#;

    fn dir_of(files: &[(&str, &str)]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        for (name, body) in files {
            std::fs::write(dir.path().join(name), body).unwrap();
        }
        dir
    }

    #[test]
    fn the_config_dir_follows_the_cli_order() {
        let home = Some(PathBuf::from("/home/u"));
        let env = |pairs: &'static [(&'static str, &'static str)]| {
            move |name: &str| {
                pairs
                    .iter()
                    .find(|(key, _)| *key == name)
                    .map(|(_, value)| OsString::from(*value))
            }
        };
        assert_eq!(
            config_dir(env(&[]), home.clone()),
            Some("/home/u/.cursor".into())
        );
        assert_eq!(
            config_dir(env(&[("XDG_CONFIG_HOME", "/x")]), home.clone()),
            Some("/x/cursor".into())
        );
        assert_eq!(
            config_dir(
                env(&[("XDG_CONFIG_HOME", "/x"), ("CURSOR_CONFIG_DIR", "/c")]),
                home.clone()
            ),
            Some("/c".into())
        );
        assert_eq!(
            config_dir(env(&[("CURSOR_CONFIG_DIR", "  ")]), home.clone()),
            Some("/home/u/.cursor".into()),
            "a blank value is ignored"
        );
        assert_eq!(config_dir(env(&[]), None), None);
    }

    #[test]
    fn the_state_has_only_the_whitelisted_keys() {
        let state = read_state(SAMPLE).unwrap();
        assert!(state.max_mode && state.model_max_mode && state.persisted_max());
        let selected = state.selected.clone().unwrap();
        assert_eq!(selected.model, "grok-4.7");
        assert_eq!(selected.params.len(), 2);
        assert_eq!(state.saved_for("grok-4.7").unwrap().len(), 1);
        assert!(!format!("{state:?}").contains("SECRET-TOKEN"));
        assert!(read_state("not json").is_none());
        assert_eq!(read_state("{}").unwrap(), CliState::default());
        assert!(!read_state(r#"{"maxMode":"yes"}"#).unwrap().max_mode);
    }

    #[test]
    fn only_the_two_toggles_change_and_every_other_byte_stays() {
        let out = rewrite_off(SAMPLE).unwrap().unwrap();
        assert_eq!(
            out,
            SAMPLE.replacen("\"maxMode\": true", "\"maxMode\": false", 2)
        );
        // The look-alike inside `other` and the credentials are untouched.
        assert!(out.contains(r#""other": [true, {"maxMode": true}]"#));
        assert!(out.contains("SECRET-TOKEN"));
        assert!(
            rewrite_off(&out).unwrap().is_none(),
            "a second pass has nothing to do"
        );
    }

    #[test]
    fn one_toggle_alone_and_other_layouts() {
        let top_only = r#"{"model":{"maxMode":false},"maxMode":true}"#;
        assert_eq!(
            rewrite_off(top_only).unwrap().unwrap(),
            r#"{"model":{"maxMode":false},"maxMode":false}"#
        );
        let nested_only = r#"{ "model" : { "x": "}", "maxMode" : true } , "maxMode":false}"#;
        assert_eq!(
            rewrite_off(nested_only).unwrap().unwrap(),
            r#"{ "model" : { "x": "}", "maxMode" : false } , "maxMode":false}"#
        );
        assert!(rewrite_off(r#"{"maxMode":false}"#).unwrap().is_none());
        assert!(rewrite_off("{}").unwrap().is_none());
    }

    #[test]
    fn shapes_it_does_not_dare_to_touch_are_refused() {
        assert_eq!(rewrite_off("nope"), Err(RewriteError::NotJson));
        assert_eq!(rewrite_off("[true]"), Err(RewriteError::NotObject));
        // Duplicate keys: serde keeps the last one, a textual edit could hit the other.
        assert_eq!(
            rewrite_off(r#"{"maxMode":true,"maxMode":true}"#),
            Err(RewriteError::Ambiguous)
        );
        assert_eq!(
            rewrite_off(r#"{"model":{},"model":{"maxMode":true}}"#),
            Err(RewriteError::Ambiguous)
        );
        // The last duplicate wins when parsed, and it has nothing to turn off.
        assert_eq!(
            rewrite_off(r#"{"model":{"maxMode":true},"model":{}}"#),
            Ok(None)
        );
    }

    #[test]
    fn force_off_rewrites_in_place_keeping_permissions_and_symlinks() {
        let dir = dir_of(&[("cli-config.json", SAMPLE)]);
        let path = dir.path().join("cli-config.json");
        let lock = dir.path().join("lock");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        }
        assert_eq!(force_off(&path, &lock).unwrap(), ForceOff::Turned);
        let state = read_state_at(&path).unwrap();
        assert!(!state.persisted_max());
        assert_eq!(state.selected.unwrap().model, "grok-4.7");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }
        assert_eq!(force_off(&path, &lock).unwrap(), ForceOff::Unchanged);
        // No temporary file is left behind.
        let leftovers: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().ends_with(".tmp"))
            .collect();
        assert!(leftovers.is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn a_symlinked_config_is_edited_where_it_lives() {
        let dir = dir_of(&[("real.json", SAMPLE)]);
        let real = dir.path().join("real.json");
        let link = dir.path().join("cli-config.json");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        force_off(&link, &dir.path().join("lock")).unwrap();
        assert!(std::fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink());
        assert!(!read_state_at(&real).unwrap().persisted_max());
    }

    #[test]
    fn an_unsafe_file_is_left_alone() {
        let dir = dir_of(&[("cli-config.json", r#"{"maxMode":true,"maxMode":true}"#)]);
        let path = dir.path().join("cli-config.json");
        let before = std::fs::read_to_string(&path).unwrap();
        assert!(matches!(
            force_off(&path, &dir.path().join("lock")),
            Err(ForceOffError::Rewrite(RewriteError::Ambiguous))
        ));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), before);
        assert!(matches!(
            force_off(&dir.path().join("nope.json"), &dir.path().join("lock")),
            Err(ForceOffError::Io(_))
        ));
    }

    #[test]
    fn a_huge_config_is_not_read() {
        let dir = dir_of(&[]);
        let path = dir.path().join("cli-config.json");
        std::fs::write(&path, " ".repeat(MAX_BYTES as usize + 1)).unwrap();
        assert!(read_state_at(&path).is_none());
        assert!(matches!(
            force_off(&path, &dir.path().join("lock")),
            Err(ForceOffError::Io(_))
        ));
    }
}
