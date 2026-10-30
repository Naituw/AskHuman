//! Verify the installed artifact and source snapshot, including uncommitted build inputs.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::{Path, PathBuf};

const DIRECTORIES: &[&str] = &[
    "src",
    "public",
    "scripts",
    "src-tauri/src",
    "src-tauri/resources",
    "src-tauri/swift",
    "src-tauri/icons",
    "src-tauri/capabilities",
    "src-tauri/vendor",
];
const FILES: &[&str] = &[
    "package.json",
    "pnpm-lock.yaml",
    "vite.config.ts",
    "vite.attachment.config.ts",
    "tsconfig.json",
    "src-tauri/Cargo.toml",
    "src-tauri/Cargo.lock",
    "src-tauri/tauri.conf.json",
    "src-tauri/build.rs",
];

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BuildInfo {
    pub version: u32,
    pub worktree: PathBuf,
    pub binary: PathBuf,
    pub source_fingerprint: String,
    pub binary_fingerprint: String,
    pub git_commit: Option<String>,
    pub created_at_ms: u64,
}

pub fn file_fingerprint(path: &Path) -> Result<String, String> {
    let mut file = std::fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut digest = Sha256::new();
    let mut bytes = [0u8; 64 * 1024];
    loop {
        let count = file.read(&mut bytes).map_err(|e| e.to_string())?;
        if count == 0 {
            break;
        }
        digest.update(&bytes[..count]);
    }
    Ok(format!("{:x}", digest.finalize()))
}

pub fn source_fingerprint(root: &Path) -> Result<String, String> {
    fn visit(
        root: &Path,
        relative: &str,
        inputs: &mut Vec<(String, String)>,
    ) -> Result<(), String> {
        let path = root.join(relative);
        let metadata = match path.symlink_metadata() {
            Ok(metadata) => metadata,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(e.to_string()),
        };
        if metadata.file_type().is_symlink() {
            let link = std::fs::read_link(&path).map_err(|e| e.to_string())?;
            let value = format!(
                "link:{}",
                link.to_str().ok_or("non-Unicode source symlink")?
            );
            inputs.push((
                relative.to_string(),
                format!("{:x}", Sha256::digest(value.as_bytes())),
            ));
        } else if metadata.is_file() {
            inputs.push((relative.to_string(), file_fingerprint(&path)?));
        } else if metadata.is_dir() {
            for entry in std::fs::read_dir(&path).map_err(|e| e.to_string())? {
                let entry = entry.map_err(|e| e.to_string())?;
                let name = entry
                    .file_name()
                    .into_string()
                    .map_err(|_| "non-Unicode build input")?;
                if [".git", "node_modules", "target", ".DS_Store"].contains(&name.as_str()) {
                    continue;
                }
                visit(root, &format!("{relative}/{name}"), inputs)?;
            }
        }
        Ok(())
    }
    let mut inputs = Vec::new();
    for relative in DIRECTORIES.iter().chain(FILES) {
        visit(root, relative, &mut inputs)?;
    }
    inputs.sort_by(|a, b| a.0.cmp(&b.0));
    let mut hash = Sha256::new();
    for (relative, fingerprint) in inputs {
        hash.update(relative.as_bytes());
        hash.update(b"\0");
        hash.update(fingerprint.as_bytes());
        hash.update(b"\0");
    }
    Ok(format!("{:x}", hash.finalize()))
}

pub fn verify(root: &Path) -> Result<BuildInfo, String> {
    let target = super::routing::dev_target(root)?;
    let root = target.worktree.as_ref().ok_or("missing Dev worktree")?;
    let path = root.join(crate::dev_instance::DEV_DIR).join("build.json");
    let mut bytes = Vec::new();
    std::fs::File::open(&path)
        .map_err(|_| "Dev build record is missing; run the worktree install script".to_string())?
        .take(1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > 1024 * 1024 {
        return Err("Dev build record is too large".into());
    }
    let info: BuildInfo =
        serde_json::from_slice(&bytes).map_err(|e| format!("invalid Dev build record: {e}"))?;
    if info.version != 1
        || !super::state::same_path(root, &info.worktree)
        || !super::state::same_path(&target.binary, &info.binary)
    {
        return Err("Dev build record does not identify the selected worktree and program".into());
    }
    if file_fingerprint(&target.binary)? != info.binary_fingerprint {
        return Err(
            "Dev program differs from its installed build record; rebuild before verification"
                .into(),
        );
    }
    if source_fingerprint(root)? != info.source_fingerprint {
        return Err(
            "Dev source has unbuilt changes; run the worktree install script before verification"
                .into(),
        );
    }
    super::routing::check_capability(&target.binary)?;
    Ok(info)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_snapshot_covers_uncommitted_inputs_but_excludes_runtime_and_docs() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(temp.path().join("src")).unwrap();
        std::fs::write(temp.path().join("src/App.vue"), "before").unwrap();
        let before = source_fingerprint(temp.path()).unwrap();
        std::fs::create_dir_all(temp.path().join(".askhuman-dev")).unwrap();
        std::fs::write(temp.path().join(".askhuman-dev/build.json"), "metadata").unwrap();
        std::fs::create_dir_all(temp.path().join("docs")).unwrap();
        std::fs::write(temp.path().join("docs/PROGRESS.md"), "progress").unwrap();
        assert_eq!(before, source_fingerprint(temp.path()).unwrap());
        std::fs::write(temp.path().join("src/App.vue"), "after").unwrap();
        assert_ne!(before, source_fingerprint(temp.path()).unwrap());
    }

    #[test]
    fn rust_and_installer_use_identical_snapshot_format() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(temp.path().join("src")).unwrap();
        std::fs::write(temp.path().join("src/测试.vue"), "changed source").unwrap();
        std::fs::write(temp.path().join("package.json"), "{}").unwrap();
        let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../scripts/dev-build-record.mjs");
        let output = std::process::Command::new("node")
            .arg(script)
            .arg("snapshot")
            .arg(temp.path())
            .output()
            .unwrap();
        assert!(output.status.success());
        assert_eq!(
            source_fingerprint(temp.path()).unwrap(),
            String::from_utf8(output.stdout).unwrap().trim()
        );
    }
}
