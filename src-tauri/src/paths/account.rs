//! Account identity is independent of the invoking shell's file-expansion home.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

pub fn home() -> Result<PathBuf, String> {
    static ACCOUNT: OnceLock<Result<PathBuf, String>> = OnceLock::new();
    ACCOUNT.get_or_init(query).clone()
}

fn validate(path: PathBuf) -> Result<PathBuf, String> {
    if !path.is_absolute() || path.to_str().is_none() {
        return Err("the account database did not return a Unicode absolute home directory".into());
    }
    Ok(path)
}

#[cfg(unix)]
fn query() -> Result<PathBuf, String> {
    use std::ffi::CStr;
    use std::os::unix::ffi::OsStrExt;

    let uid = unsafe { libc::getuid() };
    let mut buffer = vec![0 as libc::c_char; 4096];
    loop {
        let mut entry: libc::passwd = unsafe { std::mem::zeroed() };
        let mut found = std::ptr::null_mut();
        // SAFETY: the output pointers and buffer remain valid until the owned path is copied.
        let status = unsafe {
            libc::getpwuid_r(
                uid,
                &mut entry,
                buffer.as_mut_ptr(),
                buffer.len(),
                &mut found,
            )
        };
        if status == libc::ERANGE && buffer.len() < 1024 * 1024 {
            buffer.resize(buffer.len() * 2, 0);
            continue;
        }
        let result = if status == 0 && !found.is_null() && !entry.pw_dir.is_null() {
            let bytes = unsafe { CStr::from_ptr(entry.pw_dir) }.to_bytes();
            validate(PathBuf::from(std::ffi::OsStr::from_bytes(bytes)))
        } else {
            Err(format!(
                "account lookup returned status {status} without a home directory"
            ))
        };
        return result.map_err(|error| format!(
            "cannot resolve account home for uid {uid}: {error}; repair the account record or select a private instance with ASKHUMAN_HOME=/absolute/path"
        ));
    }
}

#[cfg(windows)]
fn query() -> Result<PathBuf, String> {
    dirs::home_dir()
        .ok_or_else(|| "cannot resolve the account Profile Known Folder".to_string())
        .and_then(validate)
}

#[cfg(not(any(unix, windows)))]
fn query() -> Result<PathBuf, String> {
    Err("account home lookup is unsupported on this platform".into())
}

pub fn redirected(metadata: &std::fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        metadata.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    metadata.file_type().is_symlink()
}

pub fn validate_fixture(path: &Path) -> Result<PathBuf, String> {
    if !path.is_absolute() || path.to_str().is_none() {
        return Err("system fixture account home must be an absolute Unicode directory".into());
    }
    let path = path
        .canonicalize()
        .map_err(|e| format!("invalid system fixture: {e}"))?;
    let metadata = std::fs::metadata(&path).map_err(|e| e.to_string())?;
    if !metadata.is_dir() {
        return Err("system fixture account home must be an existing directory".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.uid() != unsafe { libc::getuid() } || metadata.mode() & 0o022 != 0 {
            return Err(
                "system fixture must be owned by this user and not writable by other users".into(),
            );
        }
    }
    let actual = home()?;
    let actual = actual.canonicalize().unwrap_or(actual);
    let production = actual.join(".askhuman");
    let production = production.canonicalize().unwrap_or(production);
    let root = path.join(".askhuman");
    if crate::development::state::same_path(&path, &actual)
        || actual.starts_with(&path)
        || path.starts_with(&production)
        || production.starts_with(&root)
    {
        return Err("system fixture conflicts with the real account or production data".into());
    }
    match std::fs::symlink_metadata(&root) {
        Ok(metadata) if redirected(&metadata) || !metadata.is_dir() => {
            return Err(
                "system fixture .askhuman must be a private directory without redirection".into(),
            );
        }
        Ok(_) => {
            let actual_root = root.canonicalize().map_err(|e| e.to_string())?;
            if actual_root != root
                || crate::development::state::same_path(&actual_root, &production)
            {
                return Err("system fixture data root escapes its account directory".into());
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.to_string()),
    }
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn account_lookup_returns_an_absolute_home() {
        assert!(home().unwrap().is_absolute());
    }

    #[test]
    fn a_fixture_cannot_alias_production_through_its_data_directory() {
        #[cfg(unix)]
        {
            let temp = tempfile::tempdir().unwrap();
            std::os::unix::fs::symlink(
                home().unwrap().join(".askhuman"),
                temp.path().join(".askhuman"),
            )
            .unwrap();
            assert!(validate_fixture(temp.path()).is_err());
        }
        assert!(validate_fixture(&home().unwrap()).is_err());
    }
}
