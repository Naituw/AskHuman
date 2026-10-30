//! Strict, exclusive ownership of the Antigravity shared rule.
//!
//! Callers hold IntegrationMutationLock across these unlocked operations. Unix operations use
//! pinned directory descriptors and reject links. The final check-to-rename/unlink window against
//! another process running as the same user is not an atomic compare-and-swap guarantee.

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub const MANAGED_FILE_MARK: &str =
    "<!-- AskHuman:antigravity-managed-file version=1 DO NOT EDIT -->";
const BLOCK_BEGIN: &str = "<!-- AskHuman:begin DO NOT EDIT (managed by AskHuman) -->";
const BLOCK_END: &str = "<!-- AskHuman:end -->";

#[derive(Clone, Debug)]
pub struct RuleContext {
    pub target_path: PathBuf,
    pub integration_state_file: PathBuf,
    pub generation_version: String,
    pub rendered_bytes: Vec<u8>,
    pub variant: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RuleState {
    Missing,
    OwnedCurrent,
    OwnedOutdated,
    Conflict,
    NeedsRepair,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileIdentity {
    pub device: u64,
    pub inode: u64,
    pub owner: u32,
    pub mode: u32,
    pub links: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuleOwnership {
    pub path: PathBuf,
    pub generation_version: String,
    pub variant: String,
    pub sha256: String,
    /// Owner recorded at install time. Ledgers written before ownership was content-based omit it.
    #[serde(default)]
    pub owner: Option<u32>,
    /// Full identity recorded by earlier prerelease builds. Read only to fall back to its uid;
    /// new records never write it back.
    #[serde(default, skip_serializing, alias = "platform_file_identity")]
    pub legacy_file_identity: Option<FileIdentity>,
}

#[derive(Clone, Debug)]
pub struct RuleSnapshot {
    pub state: RuleState,
    pub ownership: Option<RuleOwnership>,
    pub reason: Option<String>,
}

impl RuleSnapshot {
    fn plain(state: RuleState, ownership: Option<RuleOwnership>, reason: Option<String>) -> Self {
        Self {
            state,
            ownership,
            reason,
        }
    }
}

fn validate_context(ctx: &RuleContext) -> Result<()> {
    use std::path::Component;
    for path in [&ctx.target_path, &ctx.integration_state_file] {
        if !path.is_absolute()
            || path
                .components()
                .any(|c| matches!(c, Component::ParentDir | Component::CurDir))
        {
            bail!("integration paths must be absolute and normalized");
        }
    }
    if !ctx
        .target_path
        .ends_with(".gemini/config/rules/askhuman-antigravity.md")
        || ctx
            .integration_state_file
            .file_name()
            .and_then(|s| s.to_str())
            != Some("integration-state.json")
    {
        bail!("unexpected Antigravity rule or integration ledger path");
    }
    if ctx.generation_version.is_empty() || (ctx.variant != "cli" && ctx.variant != "mcp") {
        bail!("Antigravity supports only a versioned CLI or MCP rule");
    }
    validate_format(&ctx.rendered_bytes)
}

fn validate_format(bytes: &[u8]) -> Result<()> {
    let text = std::str::from_utf8(bytes).context("rule is not UTF-8")?;
    if bytes.len() >= 24000 || !text.starts_with("---\n") {
        bail!("invalid Antigravity rule size/frontmatter");
    }
    let frontmatter = text[4..]
        .split_once("\n---\n")
        .context("missing frontmatter end")?
        .0;
    if frontmatter
        .lines()
        .filter(|line| line.trim().starts_with("trigger:"))
        .count()
        != 1
        || frontmatter
            .lines()
            .filter(|line| line.trim().starts_with("description:"))
            .count()
            != 1
        || !frontmatter
            .lines()
            .any(|s| s.trim() == "trigger: always_on")
        || !frontmatter
            .lines()
            .any(|s| s.trim().starts_with("description:") && !s.trim()[12..].trim().is_empty())
        || text.matches(MANAGED_FILE_MARK).count() != 1
        || text.matches(BLOCK_BEGIN).count() != 1
        || text.matches(BLOCK_END).count() != 1
        || text.find(BLOCK_BEGIN) >= text.find(BLOCK_END)
    {
        bail!("invalid Antigravity managed rule format");
    }
    Ok(())
}

pub fn probe(ctx: &RuleContext) -> Result<RuleSnapshot> {
    validate_context(ctx)?;
    platform::probe(ctx)
}

pub fn install_unlocked(ctx: &RuleContext) -> Result<RuleSnapshot> {
    validate_context(ctx)?;
    platform::mutate(ctx, false)?;
    platform::probe(ctx)
}

pub fn uninstall_unlocked(ctx: &RuleContext) -> Result<()> {
    validate_context(ctx)?;
    platform::mutate(ctx, true)
}

/// Recover this rule's existing journal without starting a new mutation.
pub(crate) fn recover_unlocked(ctx: &RuleContext) -> Result<()> {
    validate_context(ctx)?;
    platform::recover_only(ctx)
}

#[cfg(all(test, unix))]
pub(crate) fn set_test_failures(first: Option<&'static str>, second: Option<&'static str>) {
    platform::set_test_failures(first, second);
}

#[cfg(not(unix))]
mod platform {
    use super::*;
    // VERIFY: implement and exercise pinned Windows reparse/identity checks and private ACLs on a
    // real Windows host before enabling mutations. Unix permission bits do not establish an ACL.
    pub fn probe(_: &RuleContext) -> Result<RuleSnapshot> {
        bail!("Antigravity strict rule ownership is unavailable: Windows identity/ACL verification is pending")
    }
    pub fn mutate(_: &RuleContext, _: bool) -> Result<()> {
        bail!("Antigravity strict rule mutation is unavailable: Windows identity/ACL verification is pending")
    }
    pub fn recover_only(_: &RuleContext) -> Result<()> {
        bail!("Antigravity strict rule recovery is unavailable on this platform")
    }
}

#[cfg(unix)]
mod platform {
    use super::*;
    use serde_json::Value;
    use sha2::{Digest, Sha256};
    use std::ffi::CString;
    use std::fs::{File, Metadata};
    use std::io::{Read, Write};
    use std::os::fd::{AsRawFd, FromRawFd};
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::MetadataExt;
    use std::path::{Component, Path};

    const JOURNAL: &str = ".antigravity-rule.pending.json";
    const PRIVATE_STATE: &str = ".antigravity-rule-state";

    #[derive(Debug)]
    struct Dir {
        path: PathBuf,
        file: File,
    }

    fn identity(meta: &Metadata) -> FileIdentity {
        FileIdentity {
            device: meta.dev(),
            inode: meta.ino(),
            owner: meta.uid(),
            mode: meta.mode(),
            links: meta.nlink(),
        }
    }
    fn same_directory_identity(actual: &FileIdentity, expected: &FileIdentity) -> bool {
        // Directory link counts can change as entries are added or removed. Regular-file
        // identity comparisons still include link counts to reject hard links.
        actual.device == expected.device
            && actual.inode == expected.inode
            && actual.owner == expected.owner
            && actual.mode == expected.mode
    }
    fn hash(bytes: &[u8]) -> String {
        format!("{:x}", Sha256::digest(bytes))
    }
    fn name(value: &std::ffi::OsStr) -> Result<CString> {
        if value.as_bytes().contains(&b'/') || value == "." || value == ".." {
            bail!("unsafe relative name");
        }
        CString::new(value.as_bytes()).context("NUL in path")
    }
    fn cvt(fd: i32) -> Result<i32> {
        if fd < 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        Ok(fd)
    }
    /// Name the directory that broke the walk. A bare `Not a directory (os error 20)` does not say
    /// which link was refused, and `doctor` and settings show the error as is. The error kind is
    /// preserved because callers treat `NotFound` as "absent".
    fn open_error(path: &Path, error: std::io::Error) -> anyhow::Error {
        let hint = match error.raw_os_error() {
            Some(libc::ENOTDIR | libc::ELOOP) => {
                "; links and non-directories are not accepted here"
            }
            _ => "",
        };
        std::io::Error::new(error.kind(), format!("{}: {error}{hint}", path.display())).into()
    }
    impl Dir {
        fn root() -> Result<Self> {
            let fd = unsafe {
                libc::open(
                    c"/".as_ptr(),
                    libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
                )
            };
            Ok(Self {
                path: PathBuf::from("/"),
                file: unsafe { File::from_raw_fd(cvt(fd)?) },
            })
        }
        fn child(&self, component: &std::ffi::OsStr) -> Result<Self> {
            let n = name(component)?;
            let fd = unsafe {
                libc::openat(
                    self.file.as_raw_fd(),
                    n.as_ptr(),
                    libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
                )
            };
            // Read errno before anything else can touch it.
            let failure = (fd < 0).then(std::io::Error::last_os_error);
            let path = self.path.join(component);
            if let Some(error) = failure {
                return Err(open_error(&path, error));
            }
            Ok(Self {
                path,
                file: unsafe { File::from_raw_fd(fd) },
            })
        }
        fn open(path: &Path) -> Result<Self> {
            let mut dir = Self::root()?;
            for component in path.components() {
                match component {
                    Component::RootDir => {}
                    Component::Normal(part) => {
                        dir = dir.child(part)?;
                    }
                    _ => bail!("non-normalized directory"),
                }
            }
            Ok(dir)
        }
        fn verify(&self) -> Result<()> {
            let fresh = Self::open(&self.path)?;
            if identity(&fresh.file.metadata()?) != identity(&self.file.metadata()?) {
                bail!("directory identity drift: {}", self.path.display());
            }
            Ok(())
        }
        fn sync(&self) -> Result<()> {
            self.file.sync_all().context("sync directory")
        }
        fn create_dir(&self, component: &std::ffi::OsStr) -> Result<Self> {
            self.verify()?;
            let n = name(component)?;
            let result = unsafe { libc::mkdirat(self.file.as_raw_fd(), n.as_ptr(), 0o700) };
            if result != 0 {
                return Err(std::io::Error::last_os_error().into());
            }
            self.sync()?;
            let child = self.child(component)?;
            if child.file.metadata()?.mode() & 0o777 != 0o700 {
                bail!("private directory mode is not 0700");
            }
            Ok(child)
        }
        fn read(&self, component: &str) -> Result<Option<Blob>> {
            let n = name(component.as_ref())?;
            let fd = unsafe {
                libc::openat(
                    self.file.as_raw_fd(),
                    n.as_ptr(),
                    libc::O_RDONLY | libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_NONBLOCK,
                )
            };
            if fd < 0 {
                let e = std::io::Error::last_os_error();
                if e.kind() == std::io::ErrorKind::NotFound {
                    return Ok(None);
                }
                return Err(e.into());
            }
            let mut file = unsafe { File::from_raw_fd(fd) };
            let meta = file.metadata()?;
            if !meta.is_file() || meta.uid() != unsafe { libc::geteuid() } || meta.nlink() != 1 {
                bail!(
                    "not a single-link user-owned regular file: {}",
                    self.path.join(component).display()
                );
            }
            // Bound reads of integration artifacts and transaction materials.
            if meta.len() > 2 * 1024 * 1024 {
                bail!("integration artifact exceeds 2 MiB");
            }
            let mut bytes = Vec::new();
            Read::by_ref(&mut file)
                .take(2 * 1024 * 1024 + 1)
                .read_to_end(&mut bytes)?;
            let after = file.metadata()?;
            if identity(&meta) != identity(&after)
                || meta.len() != after.len()
                || meta.mtime() != after.mtime()
                || meta.mtime_nsec() != after.mtime_nsec()
                || bytes.len() as u64 != meta.len()
            {
                bail!("file changed while reading");
            }
            Ok(Some(Blob {
                sha256: hash(&bytes),
                identity: identity(&meta),
                bytes,
            }))
        }
        fn create(&self, component: &str, bytes: &[u8]) -> Result<Blob> {
            self.create_with_role(component, bytes, None)
        }
        fn create_with_role(
            &self,
            component: &str,
            bytes: &[u8],
            role: Option<StageRole>,
        ) -> Result<Blob> {
            self.verify()?;
            let n = name(component.as_ref())?;
            let fd = unsafe {
                libc::openat(
                    self.file.as_raw_fd(),
                    n.as_ptr(),
                    libc::O_WRONLY
                        | libc::O_CREAT
                        | libc::O_EXCL
                        | libc::O_CLOEXEC
                        | libc::O_NOFOLLOW,
                    0o600,
                )
            };
            let mut file = unsafe { File::from_raw_fd(cvt(fd)?) };
            if let Some(role) = role {
                stage_checkpoint(role, "scratch-opened")?;
            }
            #[cfg(test)]
            let written = if let Some(role) = role {
                let count = bytes.len() / 2;
                file.write_all(&bytes[..count])?;
                file.sync_all()?;
                stage_checkpoint(role, "scratch-partial")?;
                count
            } else {
                0
            };
            #[cfg(not(test))]
            let written = 0;
            file.write_all(&bytes[written..])?;
            file.sync_all()?;
            let id = identity(&file.metadata()?);
            if id.mode & 0o777 != 0o600 {
                bail!("private file mode is not 0600");
            }
            self.sync()?;
            Ok(Blob {
                bytes: bytes.to_vec(),
                sha256: hash(bytes),
                identity: id,
            })
        }
        fn remove(&self, component: &str, expected: &Blob) -> Result<()> {
            if matches!(
                component,
                "askhuman-antigravity.md" | "integration-state.json"
            ) {
                crate::development::migration::before_write(&self.path.join(component), None)?;
            }
            self.verify()?;
            require_same(
                self.read(component)?.as_ref(),
                Some(expected),
                "remove preflight",
            )?;
            let n = name(component.as_ref())?;
            if unsafe { libc::unlinkat(self.file.as_raw_fd(), n.as_ptr(), 0) } != 0 {
                return Err(std::io::Error::last_os_error().into());
            }
            self.sync()
        }
        fn replace(
            &self,
            stage: &str,
            target: &str,
            expected: Option<&Blob>,
            staged: &Blob,
        ) -> Result<()> {
            self.verify()?;
            require_same(self.read(target)?.as_ref(), expected, "replace destination")?;
            require_same(self.read(stage)?.as_ref(), Some(staged), "replace stage")?;
            if matches!(target, "askhuman-antigravity.md" | "integration-state.json") {
                crate::development::migration::before_write(
                    &self.path.join(target),
                    Some(&staged.bytes),
                )?;
            }
            let a = name(stage.as_ref())?;
            let b = name(target.as_ref())?;
            let result = if expected.is_none() {
                #[cfg(target_os = "linux")]
                {
                    unsafe {
                        libc::renameat2(
                            self.file.as_raw_fd(),
                            a.as_ptr(),
                            self.file.as_raw_fd(),
                            b.as_ptr(),
                            libc::RENAME_NOREPLACE,
                        )
                    }
                }
                #[cfg(not(target_os = "linux"))]
                {
                    // A crash between link/unlink leaves two links and fails closed on recovery.
                    // VERIFY: exercise this exclusive-create interruption on a real macOS host.
                    let linked = unsafe {
                        libc::linkat(
                            self.file.as_raw_fd(),
                            a.as_ptr(),
                            self.file.as_raw_fd(),
                            b.as_ptr(),
                            0,
                        )
                    };
                    if linked == 0 {
                        unsafe { libc::unlinkat(self.file.as_raw_fd(), a.as_ptr(), 0) }
                    } else {
                        linked
                    }
                }
            } else {
                unsafe {
                    libc::renameat(
                        self.file.as_raw_fd(),
                        a.as_ptr(),
                        self.file.as_raw_fd(),
                        b.as_ptr(),
                    )
                }
            };
            if result != 0 {
                return Err(std::io::Error::last_os_error().into());
            }
            self.sync()?;
            require_same(self.read(target)?.as_ref(), Some(staged), "replace result")
        }
        fn remove_empty(&self, component: &std::ffi::OsStr, expected: &FileIdentity) -> Result<()> {
            self.remove_directory(component, expected, false)
                .map(|_| ())
        }
        fn remove_directory(
            &self,
            component: &std::ffi::OsStr,
            expected: &FileIdentity,
            retain_nonempty: bool,
        ) -> Result<bool> {
            self.verify()?;
            let child = self.child(component)?;
            if !same_directory_identity(&identity(&child.file.metadata()?), expected) {
                bail!("created directory identity drift");
            }
            let n = name(component)?;
            if unsafe { libc::unlinkat(self.file.as_raw_fd(), n.as_ptr(), libc::AT_REMOVEDIR) } != 0
            {
                let error = std::io::Error::last_os_error();
                if retain_nonempty
                    && matches!(error.raw_os_error(), Some(libc::ENOTEMPTY | libc::EEXIST))
                {
                    return Ok(false);
                }
                return Err(error.into());
            }
            self.sync()?;
            Ok(true)
        }
    }

    #[derive(Clone, Debug, Serialize, Deserialize)]
    struct Blob {
        bytes: Vec<u8>,
        sha256: String,
        identity: FileIdentity,
    }
    fn same(a: Option<&Blob>, b: Option<&Blob>) -> bool {
        match (a, b) {
            (None, None) => true,
            (Some(a), Some(b)) => a.sha256 == b.sha256 && a.identity == b.identity,
            _ => false,
        }
    }
    fn require_same(a: Option<&Blob>, b: Option<&Blob>, why: &str) -> Result<()> {
        if !same(a, b) {
            bail!("NeedsRepair: identity/hash drift ({why})");
        }
        Ok(())
    }
    fn optional_dir(path: &Path) -> Result<Option<Dir>> {
        match Dir::open(path) {
            Ok(dir) => Ok(Some(dir)),
            Err(e)
                if e.downcast_ref::<std::io::Error>()
                    .is_some_and(|e| e.kind() == std::io::ErrorKind::NotFound) =>
            {
                Ok(None)
            }
            Err(e) => Err(e),
        }
    }
    fn ledger(bytes: Option<&Blob>) -> Result<Value> {
        match bytes {
            Some(b) => {
                let value: Value =
                    serde_json::from_slice(&b.bytes).context("invalid integration ledger JSON")?;
                if !value.is_object() {
                    bail!("integration ledger is not an object");
                }
                if value.get("antigravity").is_some_and(|v| !v.is_object()) {
                    bail!("antigravity ledger entry is not an object");
                }
                Ok(value)
            }
            None => Ok(serde_json::json!({})),
        }
    }
    fn ownership(value: &Value) -> Result<Option<RuleOwnership>> {
        value
            .get("antigravity")
            .and_then(|v| v.get("rule"))
            .map(|v| serde_json::from_value(v.clone()).context("malformed Antigravity ownership"))
            .transpose()
    }
    fn set_ownership(value: &mut Value, record: Option<&RuleOwnership>) -> Result<Vec<u8>> {
        if value.get("antigravity").is_none() {
            value["antigravity"] = serde_json::json!({});
        }
        let object = value["antigravity"]
            .as_object_mut()
            .context("invalid Antigravity ledger")?;
        match record {
            Some(r) => {
                object.insert("rule".into(), serde_json::to_value(r)?);
            }
            None => {
                object.remove("rule");
            }
        }
        Ok(serde_json::to_vec_pretty(value)?)
    }
    fn record(ctx: &RuleContext, blob: &Blob) -> RuleOwnership {
        RuleOwnership {
            path: ctx.target_path.clone(),
            generation_version: ctx.generation_version.clone(),
            variant: ctx.variant.clone(),
            sha256: blob.sha256.clone(),
            owner: Some(blob.identity.owner),
            legacy_file_identity: None,
        }
    }
    fn recorded_owner(record: &RuleOwnership) -> Option<u32> {
        record
            .owner
            .or_else(|| record.legacy_file_identity.as_ref().map(|id| id.owner))
    }
    fn verify_owned(ctx: &RuleContext, blob: &Blob, record: &RuleOwnership) -> bool {
        // Content and owner prove ownership across backup restores, rewrites that only replace
        // the inode, chmod, and device-number drift. A user edit changes the digest and stays a
        // conflict; exact file identity is still enforced inside each transaction.
        record.path == ctx.target_path
            && record.sha256 == blob.sha256
            && recorded_owner(record).is_none_or(|owner| owner == blob.identity.owner)
            && validate_format(&blob.bytes).is_ok()
            && (record.variant == "cli" || record.variant == "mcp")
            && !record.generation_version.is_empty()
    }

    pub fn probe(ctx: &RuleContext) -> Result<RuleSnapshot> {
        let state = optional_dir(
            ctx.integration_state_file
                .parent()
                .context("ledger parent")?,
        )?;
        let Some(state) = state else {
            // Do not hide target path errors just because the state directory is missing.
            let target = optional_dir(ctx.target_path.parent().context("target parent")?)?;
            let exists = target
                .as_ref()
                .map(|d| d.read("askhuman-antigravity.md"))
                .transpose()?
                .flatten()
                .is_some();
            return Ok(RuleSnapshot::plain(
                if exists {
                    RuleState::Conflict
                } else {
                    RuleState::Missing
                },
                None,
                None,
            ));
        };
        if read_journal(&state)?.is_some() {
            return Ok(RuleSnapshot::plain(
                RuleState::NeedsRepair,
                None,
                Some("pending Antigravity transaction; run a mutation to recover".into()),
            ));
        }
        let value = ledger(state.read("integration-state.json")?.as_ref())?;
        let owned = ownership(&value)?;
        let dir = optional_dir(ctx.target_path.parent().context("target parent")?)?;
        let file = dir
            .as_ref()
            .map(|d| d.read("askhuman-antigravity.md"))
            .transpose()?
            .flatten();
        let status = match (file.as_ref(), owned.as_ref()) {
            (None, _) => RuleState::Missing,
            (Some(_), None) => RuleState::Conflict,
            (Some(b), Some(r)) if verify_owned(ctx, b, r) => {
                if r.generation_version == ctx.generation_version
                    && r.variant == ctx.variant
                    && b.bytes == ctx.rendered_bytes
                {
                    RuleState::OwnedCurrent
                } else {
                    RuleState::OwnedOutdated
                }
            }
            _ => RuleState::Conflict,
        };
        Ok(RuleSnapshot::plain(
            status,
            owned,
            if status == RuleState::Conflict {
                Some("unowned or changed rule; refusing to overwrite".into())
            } else {
                None
            },
        ))
    }

    #[derive(Clone, Debug, Serialize, Deserialize)]
    struct Preimage {
        file: String,
        sha256: String,
        identity: FileIdentity,
    }
    #[derive(Clone, Debug, Serialize, Deserialize)]
    struct CreatedDir {
        path: PathBuf,
        identity: FileIdentity,
    }
    #[derive(Clone, Debug, Serialize, Deserialize)]
    struct Journal {
        #[serde(default)]
        format_version: u32,
        transaction_id: String,
        target: PathBuf,
        ledger: PathBuf,
        operation: String,
        phase: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        transaction_dir: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        transaction_directory_identity: Option<FileIdentity>,
        old_target: Option<Blob>,
        old_ledger: Option<Blob>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        target_preimage: Option<Preimage>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        ledger_preimage: Option<Preimage>,
        new_target: Option<Blob>,
        new_ledger: Option<Blob>,
        recovery_target: Option<Blob>,
        recovery_ledger: Option<Blob>,
        target_stage: String,
        ledger_stage: String,
        recovery_target_stage: String,
        recovery_ledger_stage: String,
        created_dirs: Vec<CreatedDir>,
    }

    #[derive(Clone, Copy, Debug)]
    enum StageRole {
        Target,
        Ledger,
        RestoreTarget,
        RestoreLedger,
    }
    impl StageRole {
        fn name(self) -> &'static str {
            match self {
                Self::Target => "target",
                Self::Ledger => "ledger",
                Self::RestoreTarget => "restore-target",
                Self::RestoreLedger => "restore-ledger",
            }
        }
        fn stage(self, journal: &Journal) -> &str {
            match self {
                Self::Target => &journal.target_stage,
                Self::Ledger => &journal.ledger_stage,
                Self::RestoreTarget => &journal.recovery_target_stage,
                Self::RestoreLedger => &journal.recovery_ledger_stage,
            }
        }
        fn record(self, journal: &mut Journal, blob: Blob) {
            match self {
                Self::Target => journal.new_target = Some(blob),
                Self::Ledger => journal.new_ledger = Some(blob),
                Self::RestoreTarget => journal.recovery_target = Some(blob),
                Self::RestoreLedger => journal.recovery_ledger = Some(blob),
            }
            if matches!(self, Self::RestoreTarget | Self::RestoreLedger) {
                journal.phase = "recovering".into();
            }
        }
    }

    fn stage_checkpoint(role: StageRole, point: &str) -> Result<()> {
        #[cfg(test)]
        checkpoint(&format!("{}-{point}", role.name()))?;
        let _ = (role, point);
        Ok(())
    }

    fn prepare_stage(
        state: &Dir,
        dir: &Dir,
        journal: &mut Journal,
        role: StageRole,
        bytes: &[u8],
    ) -> Result<Blob> {
        // An interrupted write must never occupy a fixed recovery name without a durable Blob.
        // Unreferenced random scratches are preserved; their names alone do not prove ownership.
        let scratch = format!(
            ".antigravity-build-{}-{}-{}",
            role.name(),
            journal.transaction_id,
            uuid::Uuid::new_v4()
        );
        let blob = dir
            .create_with_role(&scratch, bytes, Some(role))
            .with_context(|| {
                format!(
                    "unpublished scratch preserved at {}",
                    dir.path.join(&scratch).display()
                )
            })?;
        require_same(
            dir.read(&scratch)?.as_ref(),
            Some(&blob),
            "scratch contents",
        )?;
        stage_checkpoint(role, "scratch-complete-before-record")?;
        role.record(journal, blob.clone());
        save_journal(state, journal)?;
        stage_checkpoint(role, "recorded-before-promotion")?;
        dir.replace(&scratch, role.stage(journal), None, &blob)?;
        stage_checkpoint(role, "stage-promoted")?;
        Ok(blob)
    }
    fn journal_dir(state: &Dir, create: bool) -> Result<Option<Dir>> {
        let directory = match state.child(PRIVATE_STATE.as_ref()) {
            Ok(dir) => Some(dir),
            Err(e)
                if e.downcast_ref::<std::io::Error>()
                    .is_some_and(|e| e.kind() == std::io::ErrorKind::NotFound) =>
            {
                if create {
                    Some(state.create_dir(PRIVATE_STATE.as_ref())?)
                } else {
                    None
                }
            }
            Err(e) => return Err(e),
        };
        if let Some(dir) = &directory {
            private(dir)?;
        }
        Ok(directory)
    }
    fn read_journal(state: &Dir) -> Result<Option<Blob>> {
        journal_dir(state, false)?
            .map(|dir| dir.read(JOURNAL))
            .transpose()
            .map(Option::flatten)
    }
    fn save_journal(state: &Dir, journal: &Journal) -> Result<()> {
        let private_state =
            journal_dir(state, true)?.context("private transaction state missing")?;
        let state = &private_state;
        let old = state.read(JOURNAL)?;
        let stage = format!(".antigravity-journal-{}", uuid::Uuid::new_v4());
        let blob = state.create(&stage, &serde_json::to_vec(journal)?)?;
        if let Err(error) = state.replace(&stage, JOURNAL, old.as_ref(), &blob) {
            // Preserve any unexpected file; remove only our unchanged staging artifact.
            let _ = state.remove(&stage, &blob);
            return Err(error);
        }
        Ok(())
    }
    #[cfg(test)]
    fn backup(txn: &Dir, filename: &str, blob: Option<&Blob>) -> Result<Option<Preimage>> {
        blob.map(|b| {
            let saved = txn.create(filename, &b.bytes)?;
            Ok(Preimage {
                file: filename.into(),
                sha256: saved.sha256,
                identity: saved.identity,
            })
        })
        .transpose()
    }
    fn checked_preimage(
        txn: &Dir,
        image: Option<&Preimage>,
        expected: Option<&Blob>,
    ) -> Result<Option<Vec<u8>>> {
        match (image, expected) {
            (None, None) => Ok(None),
            (Some(p), Some(e)) => {
                let b = txn.read(&p.file)?.context("missing durable preimage")?;
                if b.sha256 != p.sha256 || b.identity != p.identity || b.sha256 != e.sha256 {
                    bail!("NeedsRepair: preimage changed");
                }
                Ok(Some(b.bytes))
            }
            _ => bail!("NeedsRepair: inconsistent preimage metadata"),
        }
    }
    fn check_journal(ctx: &RuleContext, j: &Journal) -> Result<()> {
        let id = uuid::Uuid::parse_str(&j.transaction_id).context("invalid transaction id")?;
        if j.format_version > 1 {
            bail!("NeedsRepair: unsupported Antigravity journal format version");
        }
        if j.format_version == 0 {
            if j.transaction_dir.as_deref() != Some(format!(".antigravity-rule-txn-{id}").as_str())
                || j.transaction_directory_identity.is_none()
            {
                bail!("NeedsRepair: invalid legacy transaction directory");
            }
        } else if j.transaction_dir.is_some()
            || j.transaction_directory_identity.is_some()
            || j.target_preimage.is_some()
            || j.ledger_preimage.is_some()
        {
            bail!("NeedsRepair: unexpected legacy transaction materials");
        }
        for blob in [
            &j.old_target,
            &j.old_ledger,
            &j.new_target,
            &j.new_ledger,
            &j.recovery_target,
            &j.recovery_ledger,
        ]
        .into_iter()
        .flatten()
        {
            if hash(&blob.bytes) != blob.sha256 {
                bail!("NeedsRepair: journal blob digest mismatch");
            }
        }
        if j.target != ctx.target_path
            || j.ledger != ctx.integration_state_file
            || !matches!(j.operation.as_str(), "install" | "uninstall")
            || !matches!(
                j.phase.as_str(),
                "prepared"
                    | "file-applied"
                    | "ledger-applied"
                    | "committed"
                    | "recovering"
                    | "recovered"
            )
        {
            bail!("NeedsRepair: foreign or invalid Antigravity journal");
        }
        for (name, prefix) in [
            (&j.target_stage, ".antigravity-target-"),
            (&j.ledger_stage, ".antigravity-ledger-"),
            (&j.recovery_target_stage, ".antigravity-restore-target-"),
            (&j.recovery_ledger_stage, ".antigravity-restore-ledger-"),
        ] {
            if name != &format!("{prefix}{id}") {
                bail!("NeedsRepair: unsafe journal stage");
            }
        }
        for image in [&j.target_preimage, &j.ledger_preimage]
            .into_iter()
            .flatten()
        {
            if image.file != "target.preimage" && image.file != "ledger.preimage" {
                bail!("unsafe preimage filename");
            }
        }
        let home = ctx
            .target_path
            .parent()
            .and_then(Path::parent)
            .and_then(Path::parent)
            .and_then(Path::parent)
            .context("rule home")?;
        for d in &j.created_dirs {
            if ![
                home.join(".gemini"),
                home.join(".gemini/config"),
                home.join(".gemini/config/rules"),
            ]
            .contains(&d.path)
            {
                bail!("unsafe created directory journal");
            }
        }
        Ok(())
    }
    fn private(dir: &Dir) -> Result<()> {
        let m = dir.file.metadata()?;
        if m.uid() != unsafe { libc::geteuid() } || m.mode() & 0o077 != 0 {
            bail!("transaction directory is not private");
        }
        Ok(())
    }
    fn require_transaction_directory(txn: &Dir, j: &Journal) -> Result<()> {
        private(txn)?;
        let actual = identity(&txn.file.metadata()?);
        let expected = j
            .transaction_directory_identity
            .as_ref()
            .context("NeedsRepair: missing legacy transaction directory identity")?;
        if !same_directory_identity(&actual, expected) {
            bail!(
                "NeedsRepair: transaction directory identity drift: expected {:?}, found {:?}",
                j.transaction_directory_identity,
                actual
            );
        }
        Ok(())
    }
    fn cleanup(
        state: &Dir,
        target: Option<&Dir>,
        txn: Option<&Dir>,
        j: &Journal,
        rollback: bool,
    ) -> Result<()> {
        #[cfg(test)]
        checkpoint("cleanup")?;
        // Each artifact must still match the journal before it is removed.
        for (dir, name, expected) in [
            (target, &j.target_stage, j.new_target.as_ref()),
            (Some(state), &j.ledger_stage, j.new_ledger.as_ref()),
            (target, &j.recovery_target_stage, j.recovery_target.as_ref()),
            (
                Some(state),
                &j.recovery_ledger_stage,
                j.recovery_ledger.as_ref(),
            ),
        ] {
            if let Some(dir) = dir {
                if let Some(blob) = dir.read(name)? {
                    require_same(Some(&blob), expected, "transaction staging cleanup")?;
                    dir.remove(name, &blob)?;
                }
            }
        }
        if let Some(txn) = txn {
            for p in [&j.target_preimage, &j.ledger_preimage]
                .into_iter()
                .flatten()
            {
                if let Some(b) = txn.read(&p.file)? {
                    if b.identity != p.identity || b.sha256 != p.sha256 {
                        bail!("preimage cleanup drift");
                    }
                    txn.remove(&p.file, &b)?;
                }
            }
            require_transaction_directory(txn, j)?;
            journal_dir(state, false)?
                .context("missing private state")?
                .remove_empty(
                    j.transaction_dir
                        .as_deref()
                        .context("missing legacy transaction directory")?
                        .as_ref(),
                    j.transaction_directory_identity
                        .as_ref()
                        .context("missing legacy transaction identity")?,
                )?;
        }
        if rollback {
            cleanup_created_dirs(j)?;
        }
        verify_finished(state, j)?;
        let private_state =
            journal_dir(state, false)?.context("missing private transaction state")?;
        let blob = private_state
            .read(JOURNAL)?
            .context("journal disappeared")?;
        let saved: Journal = serde_json::from_slice(&blob.bytes)?;
        if saved.transaction_id != j.transaction_id {
            bail!("foreign journal during cleanup");
        }
        private_state.remove(JOURNAL, &blob)
    }

    fn cleanup_created_dirs(j: &Journal) -> Result<()> {
        for d in j.created_dirs.iter().rev() {
            if optional_dir(&d.path)?.is_some() {
                let removed = Dir::open(d.path.parent().context("created directory parent")?)?
                    .remove_directory(
                        d.path.file_name().context("created directory name")?,
                        &d.identity,
                        true,
                    )?;
                if !removed {
                    // A child mkdir or scratch write can precede its journal record. Never claim
                    // or delete those entries, and do not let them block logical rollback.
                    eprintln!(
                        "AskHuman: retaining nonempty Antigravity directory after rollback: {}",
                        d.path.display()
                    );
                    checkpoint("created-dir-retained")?;
                }
            }
        }
        Ok(())
    }

    fn verify_finished(state: &Dir, j: &Journal) -> Result<()> {
        let (expected_target, expected_ledger) = match j.phase.as_str() {
            "committed" => (j.new_target.as_ref(), j.new_ledger.as_ref()),
            "recovered" => (
                j.recovery_target.as_ref().or(j.old_target.as_ref()),
                j.recovery_ledger.as_ref(),
            ),
            _ => bail!("cannot clean up an unfinished Antigravity transaction"),
        };
        state.verify()?;
        let target = optional_dir(j.target.parent().context("rule parent")?)?;
        let current = target
            .as_ref()
            .map(|dir| dir.read("askhuman-antigravity.md"))
            .transpose()?
            .flatten();
        require_same(
            current.as_ref(),
            expected_target,
            "finished transaction target",
        )?;
        require_same(
            state.read("integration-state.json")?.as_ref(),
            expected_ledger,
            "finished transaction ledger",
        )
    }
    fn accepted(actual: Option<&Blob>, candidates: &[Option<&Blob>]) -> bool {
        candidates.iter().any(|c| same(actual, *c))
    }
    fn recover(ctx: &RuleContext, state: &Dir) -> Result<()> {
        let Some(saved) = read_journal(state)? else {
            return Ok(());
        };
        let mut j: Journal = serde_json::from_slice(&saved.bytes)
            .context("NeedsRepair: malformed transaction journal")?;
        check_journal(ctx, &j)?;
        // Cleanup can itself be interrupted after removal of backups. Committed/recovered phases
        // have no more rollback writes; validate the resulting artifacts before resuming cleanup.
        let target = optional_dir(ctx.target_path.parent().context("rule parent")?)?;
        let current = target
            .as_ref()
            .map(|d| d.read("askhuman-antigravity.md"))
            .transpose()?
            .flatten();
        let current_ledger = state.read("integration-state.json")?;
        if j.phase == "committed" || j.phase == "recovered" {
            let (expected_target, expected_ledger) = if j.phase == "committed" {
                (j.new_target.as_ref(), j.new_ledger.as_ref())
            } else {
                (
                    j.recovery_target.as_ref().or(j.old_target.as_ref()),
                    j.recovery_ledger.as_ref(),
                )
            };
            require_same(
                current.as_ref(),
                expected_target,
                "finished transaction target",
            )?;
            require_same(
                current_ledger.as_ref(),
                expected_ledger,
                "finished transaction ledger",
            )?;
            if j.format_version == 1 {
                return cleanup(state, target.as_ref(), None, &j, j.phase == "recovered");
            }
            if let Some(txn) = optional_dir(
                &state.path.join(PRIVATE_STATE).join(
                    j.transaction_dir
                        .as_deref()
                        .context("missing legacy transaction directory")?,
                ),
            )? {
                require_transaction_directory(&txn, &j)?;
                return cleanup(
                    state,
                    target.as_ref(),
                    Some(&txn),
                    &j,
                    j.phase == "recovered",
                );
            }
            // Transaction directory may already be gone, but only final journal removal remains.
            if j.phase == "recovered" {
                cleanup_created_dirs(&j)?;
            }
            verify_finished(state, &j)?;
            return journal_dir(state, false)?
                .context("missing private transaction state")?
                .remove(JOURNAL, &saved);
        }
        if !accepted(
            current.as_ref(),
            &[
                j.old_target.as_ref(),
                j.new_target.as_ref(),
                j.recovery_target.as_ref(),
            ],
        ) || !accepted(
            current_ledger.as_ref(),
            &[
                j.old_ledger.as_ref(),
                j.new_ledger.as_ref(),
                j.recovery_ledger.as_ref(),
            ],
        ) {
            bail!("NeedsRepair: external drift while transaction was pending");
        }
        let txn = if j.format_version == 0 {
            let txn = journal_dir(state, false)?
                .context("missing private transaction state")?
                .child(
                    j.transaction_dir
                        .as_deref()
                        .context("missing legacy transaction directory")?
                        .as_ref(),
                )?;
            require_transaction_directory(&txn, &j)?;
            Some(txn)
        } else {
            None
        };
        let (old_bytes, old_ledger_bytes) = if let Some(txn) = &txn {
            (
                checked_preimage(txn, j.target_preimage.as_ref(), j.old_target.as_ref())?,
                checked_preimage(txn, j.ledger_preimage.as_ref(), j.old_ledger.as_ref())?,
            )
        } else {
            (
                j.old_target.as_ref().map(|blob| blob.bytes.clone()),
                j.old_ledger.as_ref().map(|blob| blob.bytes.clone()),
            )
        };
        let mut target_after = current.clone();
        if !(same(current.as_ref(), j.old_target.as_ref())
            || (j.recovery_target.is_some() && same(current.as_ref(), j.recovery_target.as_ref())))
        {
            if let Some(bytes) = old_bytes.as_ref() {
                let dir = target
                    .as_ref()
                    .context("NeedsRepair: target directory disappeared")?;
                let staged = match dir.read(&j.recovery_target_stage)? {
                    Some(b) => {
                        require_same(Some(&b), j.recovery_target.as_ref(), "restore stage")?;
                        b
                    }
                    None => prepare_stage(state, dir, &mut j, StageRole::RestoreTarget, bytes)?,
                };
                j.recovery_target = Some(staged.clone());
                j.phase = "recovering".into();
                save_journal(state, &j)?;
                dir.replace(
                    &j.recovery_target_stage,
                    "askhuman-antigravity.md",
                    current.as_ref(),
                    &staged,
                )?;
                checkpoint("recovery-target-applied")?;
                target_after = Some(staged);
            } else if let Some(b) = current.as_ref() {
                j.phase = "recovering".into();
                save_journal(state, &j)?;
                target
                    .as_ref()
                    .context("target directory missing")?
                    .remove("askhuman-antigravity.md", b)?;
                checkpoint("recovery-target-applied")?;
                target_after = None;
            }
        }
        j.recovery_target = target_after.clone();
        // Ownership is content-based, so restoring the old bytes by rename keeps the old
        // ledger record valid even though the restored file has a new inode.
        let restore_ledger = old_ledger_bytes;
        if let Some(bytes) = restore_ledger {
            if !same(current_ledger.as_ref(), j.recovery_ledger.as_ref())
                || j.recovery_ledger.is_none()
            {
                let staged = match state.read(&j.recovery_ledger_stage)? {
                    Some(b) => {
                        require_same(Some(&b), j.recovery_ledger.as_ref(), "restore ledger stage")?;
                        b
                    }
                    None => prepare_stage(state, state, &mut j, StageRole::RestoreLedger, &bytes)?,
                };
                j.recovery_ledger = Some(staged.clone());
                j.phase = "recovering".into();
                save_journal(state, &j)?;
                state.replace(
                    &j.recovery_ledger_stage,
                    "integration-state.json",
                    current_ledger.as_ref(),
                    &staged,
                )?;
            }
        } else if let Some(b) = current_ledger.as_ref() {
            j.recovery_ledger = None;
            j.phase = "recovering".into();
            save_journal(state, &j)?;
            state.remove("integration-state.json", b)?;
        }
        checkpoint("recovery-ledger-applied")?;
        j.phase = "recovered".into();
        save_journal(state, &j)?;
        checkpoint("recovered")?;
        cleanup(state, target.as_ref(), txn.as_ref(), &j, true)
    }

    fn target_dirs(ctx: &RuleContext, state: &Dir, j: &mut Journal) -> Result<Dir> {
        let home = ctx
            .target_path
            .parent()
            .and_then(Path::parent)
            .and_then(Path::parent)
            .and_then(Path::parent)
            .context("rule home")?;
        let mut dir = Dir::open(home)?;
        for component in [".gemini", "config", "rules"] {
            match dir.child(component.as_ref()) {
                Ok(child) => {
                    dir = child;
                }
                Err(e)
                    if e.downcast_ref::<std::io::Error>()
                        .is_some_and(|e| e.kind() == std::io::ErrorKind::NotFound) =>
                {
                    let child = dir.create_dir(component.as_ref())?;
                    #[cfg(test)]
                    checkpoint(&format!("mkdir-{component}-before-record"))?;
                    j.created_dirs.push(CreatedDir {
                        path: child.path.clone(),
                        identity: identity(&child.file.metadata()?),
                    });
                    save_journal(state, j)?;
                    dir = child;
                }
                Err(e) => return Err(e),
            }
        }
        Ok(dir)
    }

    #[cfg(test)]
    thread_local! { static FAIL_AT: std::cell::Cell<Option<&'static str>> = const { std::cell::Cell::new(None) }; }
    #[cfg(test)]
    thread_local! { static FAIL_ALSO: std::cell::Cell<Option<&'static str>> = const { std::cell::Cell::new(None) }; }
    #[cfg(test)]
    pub fn set_test_failures(first: Option<&'static str>, second: Option<&'static str>) {
        FAIL_AT.with(|value| value.set(first));
        FAIL_ALSO.with(|value| value.set(second));
    }
    fn checkpoint(point: &str) -> Result<()> {
        #[cfg(test)]
        if FAIL_AT.with(|v| v.get().is_some_and(|failure| failure == point))
            || FAIL_ALSO.with(|v| v.get().is_some_and(|failure| failure == point))
        {
            bail!("injected failure at {point}");
        }
        let _ = point;
        Ok(())
    }

    pub fn recover_only(ctx: &RuleContext) -> Result<()> {
        if let Some(state) = optional_dir(
            ctx.integration_state_file
                .parent()
                .context("ledger parent")?,
        )? {
            let metadata = state.file.metadata()?;
            if metadata.uid() != unsafe { libc::geteuid() } || metadata.mode() & 0o022 != 0 {
                bail!("integration state parent must be owned and not writable by other users");
            }
            recover(ctx, &state)?;
        }
        Ok(())
    }

    pub fn mutate(ctx: &RuleContext, uninstall: bool) -> Result<()> {
        let state = Dir::open(
            ctx.integration_state_file
                .parent()
                .context("ledger parent")?,
        )?;
        let state_meta = state.file.metadata()?;
        if state_meta.uid() != unsafe { libc::geteuid() } || state_meta.mode() & 0o022 != 0 {
            bail!("integration state parent must be owned and not writable by other users");
        }
        recover(ctx, &state)?;
        let status = probe(ctx)?;
        if matches!(status.state, RuleState::Conflict | RuleState::NeedsRepair) {
            bail!("Antigravity rule conflict: {:?}", status.reason);
        }
        if !uninstall && status.state == RuleState::OwnedCurrent {
            return Ok(());
        }
        let old_ledger = state.read("integration-state.json")?;
        let mut value = ledger(old_ledger.as_ref())?;
        let old_dir = optional_dir(ctx.target_path.parent().context("target parent")?)?;
        let old_target = old_dir
            .as_ref()
            .map(|d| d.read("askhuman-antigravity.md"))
            .transpose()?
            .flatten();
        if uninstall && old_target.is_none() && ownership(&value)?.is_none() {
            return Ok(());
        }
        let id = uuid::Uuid::new_v4().to_string();
        let mut j = Journal {
            format_version: 1,
            transaction_id: id.clone(),
            target: ctx.target_path.clone(),
            ledger: ctx.integration_state_file.clone(),
            operation: if uninstall { "uninstall" } else { "install" }.into(),
            phase: "prepared".into(),
            transaction_dir: None,
            transaction_directory_identity: None,
            target_preimage: None,
            ledger_preimage: None,
            old_target: old_target.clone(),
            old_ledger: old_ledger.clone(),
            new_target: None,
            new_ledger: None,
            recovery_target: None,
            recovery_ledger: None,
            target_stage: format!(".antigravity-target-{id}"),
            ledger_stage: format!(".antigravity-ledger-{id}"),
            recovery_target_stage: format!(".antigravity-restore-target-{id}"),
            recovery_ledger_stage: format!(".antigravity-restore-ledger-{id}"),
            created_dirs: Vec::new(),
        };
        // No target/ledger changes occur until durable preimages and the journal exist.
        save_journal(&state, &j)?;
        checkpoint("prepared")?;
        let target = if uninstall {
            old_dir
        } else {
            Some(target_dirs(ctx, &state, &mut j)?)
        };
        if !uninstall {
            let dir = target.as_ref().context("target parent missing")?;
            let staged =
                prepare_stage(&state, dir, &mut j, StageRole::Target, &ctx.rendered_bytes)?;
            set_ownership(&mut value, Some(&record(ctx, &staged)))?;
        } else {
            set_ownership(&mut value, None)?;
        }
        prepare_stage(
            &state,
            &state,
            &mut j,
            StageRole::Ledger,
            &serde_json::to_vec_pretty(&value)?,
        )?;
        save_journal(&state, &j)?;
        checkpoint("staged")?;
        if let Some(dir) = target.as_ref() {
            if let Some(staged) = j.new_target.as_ref() {
                dir.replace(
                    &j.target_stage,
                    "askhuman-antigravity.md",
                    old_target.as_ref(),
                    staged,
                )?;
            } else if let Some(old) = old_target.as_ref() {
                dir.remove("askhuman-antigravity.md", old)?;
            }
        }
        checkpoint("target-applied-before-journal")?;
        j.phase = "file-applied".into();
        save_journal(&state, &j)?;
        checkpoint("file-applied")?;
        state.replace(
            &j.ledger_stage,
            "integration-state.json",
            old_ledger.as_ref(),
            j.new_ledger.as_ref().context("new ledger missing")?,
        )?;
        checkpoint("ledger-applied-before-journal")?;
        j.phase = "ledger-applied".into();
        save_journal(&state, &j)?;
        checkpoint("ledger-applied")?;
        require_same(
            target
                .as_ref()
                .map(|d| d.read("askhuman-antigravity.md"))
                .transpose()?
                .flatten()
                .as_ref(),
            j.new_target.as_ref(),
            "commit target",
        )?;
        require_same(
            state.read("integration-state.json")?.as_ref(),
            j.new_ledger.as_ref(),
            "commit ledger",
        )?;
        j.phase = "committed".into();
        save_journal(&state, &j)?;
        checkpoint("committed")?;
        cleanup(&state, target.as_ref(), None, &j, false)
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use std::os::unix::fs::{symlink, PermissionsExt};
        fn context() -> (tempfile::TempDir, RuleContext) {
            let home = tempfile::tempdir().unwrap();
            // macOS temporary roots can contain system symlinks; fixtures need physical paths.
            let home_path = home.path().canonicalize().unwrap();
            let state = home_path.join(".askhuman");
            std::fs::create_dir(&state).unwrap();
            std::fs::set_permissions(&state, std::fs::Permissions::from_mode(0o700)).unwrap();
            let ctx = RuleContext {
                target_path: home_path.join(".gemini/config/rules/askhuman-antigravity.md"),
                integration_state_file: state.join("integration-state.json"),
                generation_version: "1".into(),
                rendered_bytes: body("first"),
                variant: "cli".into(),
            };
            (home, ctx)
        }
        fn body(text: &str) -> Vec<u8> {
            format!("---\ntrigger: always_on\ndescription: AskHuman\n---\n{MANAGED_FILE_MARK}\n{BLOCK_BEGIN}\n{text}\n{BLOCK_END}\n").into_bytes()
        }
        fn put_ledger(ctx: &RuleContext) {
            std::fs::write(
                &ctx.integration_state_file,
                b"{\"codex\":{\"kept\":true},\"unknown\":17}",
            )
            .unwrap();
        }
        fn legacy_journal(state: &Dir) -> Journal {
            let mut j: Journal =
                serde_json::from_slice(&read_journal(state).unwrap().unwrap().bytes).unwrap();
            let transaction_dir = format!(".antigravity-rule-txn-{}", j.transaction_id);
            let txn = journal_dir(state, true)
                .unwrap()
                .unwrap()
                .create_dir(transaction_dir.as_ref())
                .unwrap();
            j.format_version = 0;
            j.transaction_dir = Some(transaction_dir);
            j.transaction_directory_identity = Some(super::identity(&txn.file.metadata().unwrap()));
            j.target_preimage = backup(&txn, "target.preimage", j.old_target.as_ref()).unwrap();
            j.ledger_preimage = backup(&txn, "ledger.preimage", j.old_ledger.as_ref()).unwrap();
            save_journal(state, &j).unwrap();
            // Reproduce the unversioned wire format emitted by earlier builds.
            let private = journal_dir(state, false).unwrap().unwrap();
            let saved = private.read(JOURNAL).unwrap().unwrap();
            let mut value = serde_json::to_value(&j).unwrap();
            value.as_object_mut().unwrap().remove("format_version");
            let stage = ".legacy-journal-fixture";
            let blob = private
                .create(stage, &serde_json::to_vec(&value).unwrap())
                .unwrap();
            private
                .replace(stage, JOURNAL, Some(&saved), &blob)
                .unwrap();
            j
        }
        fn fail(point: &'static str) {
            FAIL_AT.with(|v| v.set(Some(point)));
        }
        fn clear() {
            FAIL_AT.with(|v| v.set(None));
        }

        fn scratches(root: &Path) -> Vec<(PathBuf, Vec<u8>, FileIdentity)> {
            let mut found = Vec::new();
            for entry in std::fs::read_dir(root).unwrap() {
                let entry = entry.unwrap();
                if entry.file_type().unwrap().is_dir() {
                    found.extend(scratches(&entry.path()));
                } else if entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with(".antigravity-build-")
                {
                    found.push((
                        entry.path(),
                        std::fs::read(entry.path()).unwrap(),
                        super::identity(&entry.metadata().unwrap()),
                    ));
                }
            }
            found.sort_by(|a, b| a.0.cmp(&b.0));
            found
        }

        const FORWARD_STAGE_POINTS: [&str; 10] = [
            "target-scratch-opened",
            "target-scratch-partial",
            "target-scratch-complete-before-record",
            "target-recorded-before-promotion",
            "target-stage-promoted",
            "ledger-scratch-opened",
            "ledger-scratch-partial",
            "ledger-scratch-complete-before-record",
            "ledger-recorded-before-promotion",
            "ledger-stage-promoted",
        ];
        const RECOVERY_STAGE_POINTS: [&str; 10] = [
            "restore-target-scratch-opened",
            "restore-target-scratch-partial",
            "restore-target-scratch-complete-before-record",
            "restore-target-recorded-before-promotion",
            "restore-target-stage-promoted",
            "restore-ledger-scratch-opened",
            "restore-ledger-scratch-partial",
            "restore-ledger-scratch-complete-before-record",
            "restore-ledger-recorded-before-promotion",
            "restore-ledger-stage-promoted",
        ];

        #[test]
        fn scratch_interruptions_recover_without_claiming_unpublished_files() {
            for existing_rule in [false, true] {
                for point in FORWARD_STAGE_POINTS {
                    let (home, mut ctx) = context();
                    put_ledger(&ctx);
                    if existing_rule {
                        super::super::install_unlocked(&ctx).unwrap();
                    }
                    let old_target = std::fs::read(&ctx.target_path).ok();
                    let old_ledger = std::fs::read(&ctx.integration_state_file).unwrap();
                    ctx.generation_version = "2".into();
                    ctx.rendered_bytes = body("replacement");
                    fail(point);
                    assert!(mutate(&ctx, false).is_err(), "{point}");
                    clear();
                    let abandoned = scratches(home.path());
                    let state = Dir::open(ctx.integration_state_file.parent().unwrap()).unwrap();
                    recover(&ctx, &state)
                        .unwrap_or_else(|e| panic!("{existing_rule} {point}: {e:#}"));
                    recover(&ctx, &state).unwrap();
                    assert_eq!(std::fs::read(&ctx.target_path).ok(), old_target, "{point}");
                    assert_eq!(
                        std::fs::read(&ctx.integration_state_file).unwrap(),
                        old_ledger,
                        "{point}"
                    );
                    assert_eq!(scratches(home.path()), abandoned, "{point}");
                    assert!(read_journal(&state).unwrap().is_none());
                    super::super::install_unlocked(&ctx).unwrap();
                    assert_eq!(probe(&ctx).unwrap().state, RuleState::OwnedCurrent);
                }
            }
        }

        #[test]
        fn recovery_scratch_interruptions_work_with_both_journal_formats() {
            for legacy in [false, true] {
                for point in RECOVERY_STAGE_POINTS {
                    let (home, mut ctx) = context();
                    put_ledger(&ctx);
                    super::super::install_unlocked(&ctx).unwrap();
                    let old_target = std::fs::read(&ctx.target_path).unwrap();
                    let old_ledger = std::fs::read(&ctx.integration_state_file).unwrap();
                    ctx.generation_version = "2".into();
                    ctx.rendered_bytes = body("replacement");
                    fail("ledger-applied");
                    assert!(mutate(&ctx, false).is_err());
                    clear();
                    let state = Dir::open(ctx.integration_state_file.parent().unwrap()).unwrap();
                    if legacy {
                        legacy_journal(&state);
                    }
                    fail(point);
                    assert!(recover(&ctx, &state).is_err(), "{legacy} {point}");
                    clear();
                    let abandoned = scratches(home.path());
                    recover(&ctx, &state).unwrap_or_else(|e| panic!("{legacy} {point}: {e:#}"));
                    recover(&ctx, &state).unwrap();
                    assert_eq!(std::fs::read(&ctx.target_path).unwrap(), old_target);
                    assert_eq!(
                        std::fs::read(&ctx.integration_state_file).unwrap(),
                        old_ledger
                    );
                    assert_eq!(scratches(home.path()), abandoned);
                    assert!(read_journal(&state).unwrap().is_none());
                    super::super::install_unlocked(&ctx).unwrap();
                    assert_eq!(probe(&ctx).unwrap().state, RuleState::OwnedCurrent);
                }
            }
        }

        #[test]
        fn unrecorded_child_directories_do_not_block_logical_rollback() {
            for (point, relative) in [
                ("mkdir-.gemini-before-record", ".gemini"),
                ("mkdir-config-before-record", ".gemini/config"),
                ("mkdir-rules-before-record", ".gemini/config/rules"),
            ] {
                let (home, ctx) = context();
                put_ledger(&ctx);
                let old_ledger = std::fs::read(&ctx.integration_state_file).unwrap();
                fail(point);
                assert!(mutate(&ctx, false).is_err());
                clear();
                let state = Dir::open(ctx.integration_state_file.parent().unwrap()).unwrap();
                if relative != ".gemini" {
                    fail("created-dir-retained");
                    assert!(recover(&ctx, &state).is_err());
                    clear();
                }
                recover(&ctx, &state).unwrap();
                recover(&ctx, &state).unwrap();
                assert!(home.path().join(relative).is_dir());
                assert!(!ctx.target_path.exists());
                assert_eq!(
                    std::fs::read(&ctx.integration_state_file).unwrap(),
                    old_ledger
                );
                assert!(read_journal(&state).unwrap().is_none());
                super::super::install_unlocked(&ctx).unwrap();
            }
        }

        #[test]
        fn nonempty_created_directory_identity_drift_still_requires_repair() {
            let (home, ctx) = context();
            fail("mkdir-config-before-record");
            assert!(mutate(&ctx, false).is_err());
            clear();
            let gemini = home.path().join(".gemini");
            let saved = home.path().join("saved-gemini");
            std::fs::rename(&gemini, &saved).unwrap();
            std::fs::create_dir_all(gemini.join("foreign")).unwrap();
            std::fs::set_permissions(&gemini, std::fs::Permissions::from_mode(0o700)).unwrap();
            let state = Dir::open(ctx.integration_state_file.parent().unwrap()).unwrap();
            let error = recover(&ctx, &state).unwrap_err();
            assert!(
                error.to_string().contains("directory identity drift"),
                "{error:#}"
            );
            assert!(gemini.join("foreign").is_dir());
            assert!(saved.join("config").is_dir());
            assert!(read_journal(&state).unwrap().is_some());
        }

        #[test]
        fn unrecorded_fixed_stages_in_existing_journals_are_never_adopted() {
            for legacy in [false, true] {
                for role in [
                    StageRole::Target,
                    StageRole::Ledger,
                    StageRole::RestoreTarget,
                    StageRole::RestoreLedger,
                ] {
                    let complete = body("replacement");
                    for bytes in [b"".as_slice(), b"partial", complete.as_slice()] {
                        let (_home, mut ctx) = context();
                        super::super::install_unlocked(&ctx).unwrap();
                        ctx.rendered_bytes = body("replacement");
                        fail(if matches!(role, StageRole::Target | StageRole::Ledger) {
                            "prepared"
                        } else {
                            "file-applied"
                        });
                        assert!(mutate(&ctx, false).is_err());
                        clear();
                        let state =
                            Dir::open(ctx.integration_state_file.parent().unwrap()).unwrap();
                        let journal = if legacy {
                            legacy_journal(&state)
                        } else {
                            serde_json::from_slice(&read_journal(&state).unwrap().unwrap().bytes)
                                .unwrap()
                        };
                        let target = Dir::open(ctx.target_path.parent().unwrap()).unwrap();
                        let dir = if matches!(role, StageRole::Target | StageRole::RestoreTarget) {
                            &target
                        } else {
                            &state
                        };
                        let blob = dir.create(role.stage(&journal), bytes).unwrap();
                        assert!(recover(&ctx, &state).is_err(), "{legacy} {role:?}");
                        require_same(
                            dir.read(role.stage(&journal)).unwrap().as_ref(),
                            Some(&blob),
                            "unrecorded stage preserved",
                        )
                        .unwrap();
                        assert!(read_journal(&state).unwrap().is_some());
                    }
                }
            }
        }

        #[test]
        fn recorded_fixed_stage_drift_preserves_the_stage_and_pending_journal() {
            for drift in ["content", "identity", "symlink", "hardlink"] {
                let (home, mut ctx) = context();
                super::super::install_unlocked(&ctx).unwrap();
                ctx.rendered_bytes = body("replacement");
                fail("target-stage-promoted");
                assert!(mutate(&ctx, false).is_err());
                clear();
                let state = Dir::open(ctx.integration_state_file.parent().unwrap()).unwrap();
                let journal: Journal =
                    serde_json::from_slice(&read_journal(&state).unwrap().unwrap().bytes).unwrap();
                let stage = ctx
                    .target_path
                    .parent()
                    .unwrap()
                    .join(&journal.target_stage);
                let other = home.path().join("other-stage");
                match drift {
                    "content" => std::fs::write(&stage, b"user-edited stage").unwrap(),
                    "identity" => {
                        std::fs::rename(&stage, &other).unwrap();
                        std::fs::write(&stage, &ctx.rendered_bytes).unwrap();
                    }
                    "symlink" => {
                        std::fs::write(&other, b"external file").unwrap();
                        std::fs::remove_file(&stage).unwrap();
                        symlink(&other, &stage).unwrap();
                    }
                    _ => std::fs::hard_link(&stage, &other).unwrap(),
                }
                let identity_before = super::identity(&std::fs::symlink_metadata(&stage).unwrap());
                let bytes_before = std::fs::read(&stage).unwrap();
                assert!(recover(&ctx, &state).is_err(), "{drift}");
                assert_eq!(
                    super::identity(&std::fs::symlink_metadata(&stage).unwrap()),
                    identity_before
                );
                assert_eq!(std::fs::read(&stage).unwrap(), bytes_before);
                assert!(read_journal(&state).unwrap().is_some());
            }
        }
        fn identity(device: u64) -> FileIdentity {
            FileIdentity {
                device,
                inode: 11,
                owner: 12,
                mode: 0o100600,
                links: 1,
            }
        }
        fn blob(ctx: &RuleContext) -> Blob {
            Blob {
                bytes: ctx.rendered_bytes.clone(),
                sha256: hash(&ctx.rendered_bytes),
                identity: identity(42),
            }
        }
        #[test]
        fn persisted_ownership_ignores_file_identity_drift_but_checks_digest_and_owner() {
            let (_home, ctx) = context();
            let blob = blob(&ctx);
            let record = record(&ctx, &blob);
            assert!(verify_owned(&ctx, &blob, &record));

            let mut drifted = blob.clone();
            drifted.identity = FileIdentity {
                device: 7,
                inode: 99,
                owner: blob.identity.owner,
                mode: 0o100644,
                links: 1,
            };
            assert!(verify_owned(&ctx, &drifted, &record));

            let mut edited = drifted.clone();
            edited.bytes = body("edited");
            edited.sha256 = hash(&edited.bytes);
            assert!(!verify_owned(&ctx, &edited, &record));

            let mut foreign = drifted.clone();
            foreign.identity.owner += 1;
            assert!(!verify_owned(&ctx, &foreign, &record));

            let legacy = RuleOwnership {
                owner: None,
                legacy_file_identity: Some(blob.identity.clone()),
                ..record.clone()
            };
            assert!(verify_owned(&ctx, &blob, &legacy));
            let mut legacy_foreign = legacy;
            legacy_foreign.legacy_file_identity.as_mut().unwrap().owner += 1;
            assert!(!verify_owned(&ctx, &blob, &legacy_foreign));
        }
        #[test]
        fn transaction_blob_identity_comparison_remains_exact() {
            let expected = Blob {
                bytes: b"same".to_vec(),
                sha256: hash(b"same"),
                identity: identity(42),
            };
            let mut actual = expected.clone();
            actual.identity.device = 43;
            assert!(!same(Some(&actual), Some(&expected)));
        }
        #[test]
        fn lifecycle_preserves_other_families_and_directories() {
            let (_home, mut ctx) = context();
            put_ledger(&ctx);
            assert_eq!(super::super::probe(&ctx).unwrap().state, RuleState::Missing);
            let first = super::super::install_unlocked(&ctx).unwrap();
            assert_eq!(first.state, RuleState::OwnedCurrent);
            let again = super::super::install_unlocked(&ctx).unwrap();
            assert_eq!(first.ownership, again.ownership);
            ctx.generation_version = "2".into();
            ctx.rendered_bytes = body("second");
            assert_eq!(
                super::super::probe(&ctx).unwrap().state,
                RuleState::OwnedOutdated
            );
            super::super::install_unlocked(&ctx).unwrap();
            super::super::uninstall_unlocked(&ctx).unwrap();
            super::super::uninstall_unlocked(&ctx).unwrap();
            assert!(ctx.target_path.parent().unwrap().is_dir());
            let value: Value =
                serde_json::from_slice(&std::fs::read(&ctx.integration_state_file).unwrap())
                    .unwrap();
            assert_eq!(value["unknown"], 17);
            assert_eq!(value["codex"]["kept"], true);
        }
        #[test]
        fn same_bytes_with_a_new_inode_keep_ownership() {
            let (_home, ctx) = context();
            super::super::install_unlocked(&ctx).unwrap();
            let replacement = ctx.target_path.with_extension("replacement");
            std::fs::write(&replacement, &ctx.rendered_bytes).unwrap();
            std::fs::rename(replacement, &ctx.target_path).unwrap();
            assert_eq!(
                super::super::probe(&ctx).unwrap().state,
                RuleState::OwnedCurrent
            );
            assert_eq!(
                super::super::install_unlocked(&ctx).unwrap().state,
                RuleState::OwnedCurrent
            );
            super::super::uninstall_unlocked(&ctx).unwrap();
            assert!(!ctx.target_path.exists());
        }
        #[test]
        fn mode_drift_keeps_ownership() {
            let (_home, ctx) = context();
            super::super::install_unlocked(&ctx).unwrap();
            std::fs::set_permissions(&ctx.target_path, std::fs::Permissions::from_mode(0o644))
                .unwrap();
            assert_eq!(
                super::super::probe(&ctx).unwrap().state,
                RuleState::OwnedCurrent
            );
            super::super::uninstall_unlocked(&ctx).unwrap();
        }
        #[test]
        fn content_drift_is_a_conflict() {
            let (_home, ctx) = context();
            super::super::install_unlocked(&ctx).unwrap();
            std::fs::write(&ctx.target_path, body("user edit")).unwrap();
            assert_eq!(
                super::super::probe(&ctx).unwrap().state,
                RuleState::Conflict
            );
            assert!(super::super::uninstall_unlocked(&ctx).is_err());
            assert_eq!(std::fs::read(&ctx.target_path).unwrap(), body("user edit"));
        }
        #[test]
        fn legacy_ledger_identity_is_accepted_and_rewritten_on_update() {
            let (_home, mut ctx) = context();
            super::super::install_unlocked(&ctx).unwrap();
            let mut value: Value =
                serde_json::from_slice(&std::fs::read(&ctx.integration_state_file).unwrap())
                    .unwrap();
            let rule = value["antigravity"]["rule"].as_object_mut().unwrap();
            let owner = rule.remove("owner").unwrap();
            rule.insert(
                "platform_file_identity".into(),
                serde_json::json!({
                    "device": 1,
                    "inode": 2,
                    "owner": owner.clone(),
                    "mode": 0o100600u32,
                    "links": 1,
                }),
            );
            std::fs::write(
                &ctx.integration_state_file,
                serde_json::to_vec_pretty(&value).unwrap(),
            )
            .unwrap();
            assert_eq!(
                super::super::probe(&ctx).unwrap().state,
                RuleState::OwnedCurrent
            );

            ctx.generation_version = "2".into();
            ctx.rendered_bytes = body("second");
            super::super::install_unlocked(&ctx).unwrap();
            let value: Value =
                serde_json::from_slice(&std::fs::read(&ctx.integration_state_file).unwrap())
                    .unwrap();
            let rule = &value["antigravity"]["rule"];
            assert_eq!(rule["owner"], owner);
            assert!(rule.get("platform_file_identity").is_none());
        }
        #[test]
        fn rejects_unknown_file_malformed_ledger_and_symlink() {
            let (home, ctx) = context();
            std::fs::create_dir_all(ctx.target_path.parent().unwrap()).unwrap();
            std::fs::write(&ctx.target_path, &ctx.rendered_bytes).unwrap();
            assert_eq!(
                super::super::probe(&ctx).unwrap().state,
                RuleState::Conflict
            );
            assert!(super::super::install_unlocked(&ctx).is_err());
            std::fs::write(&ctx.integration_state_file, b"not json").unwrap();
            assert!(super::super::probe(&ctx).is_err());
            std::fs::remove_file(&ctx.integration_state_file).unwrap();
            std::fs::remove_file(&ctx.target_path).unwrap();
            symlink(home.path(), &ctx.target_path).unwrap();
            assert!(super::super::probe(&ctx).is_err());
        }
        #[test]
        fn rejects_link_ancestor_before_any_write() {
            let (home, ctx) = context();
            symlink(home.path(), home.path().join(".gemini")).unwrap();
            assert!(super::super::install_unlocked(&ctx).is_err());
            assert!(!ctx.integration_state_file.exists());
        }
        #[test]
        fn link_rejection_names_the_directory_and_keeps_absent_distinct() {
            let (home, ctx) = context();
            symlink(home.path(), home.path().join(".gemini")).unwrap();
            let linked = format!("{:#}", super::super::probe(&ctx).unwrap_err());
            assert!(linked.contains(".gemini"), "{linked}");
            assert!(linked.contains("links and non-directories"), "{linked}");

            // A missing directory keeps its `NotFound` kind: callers read it as "absent".
            let (_other, absent) = context();
            let missing = Dir::open(absent.target_path.parent().unwrap()).unwrap_err();
            assert_eq!(
                missing
                    .downcast_ref::<std::io::Error>()
                    .map(std::io::Error::kind),
                Some(std::io::ErrorKind::NotFound)
            );
        }
        #[test]
        fn interrupted_updates_and_uninstalls_restore_bytes_and_identity() {
            for uninstall in [false, true] {
                for point in [
                    "prepared",
                    "staged",
                    "target-applied-before-journal",
                    "file-applied",
                    "ledger-applied-before-journal",
                    "ledger-applied",
                ] {
                    let (_home, mut ctx) = context();
                    put_ledger(&ctx);
                    super::super::install_unlocked(&ctx).unwrap();
                    let original = ctx.rendered_bytes.clone();
                    ctx.generation_version = "2".into();
                    ctx.rendered_bytes = body("new");
                    fail(point);
                    assert!(mutate(&ctx, uninstall).is_err(), "{point}");
                    clear();
                    assert_eq!(
                        super::super::probe(&ctx).unwrap().state,
                        RuleState::NeedsRepair
                    );
                    let state = Dir::open(ctx.integration_state_file.parent().unwrap()).unwrap();
                    recover(&ctx, &state).unwrap_or_else(|e| panic!("{uninstall} {point}: {e:#}"));
                    recover(&ctx, &state).unwrap();
                    assert_eq!(std::fs::read(&ctx.target_path).unwrap(), original);
                    assert_eq!(
                        super::super::probe(&ctx).unwrap().state,
                        RuleState::OwnedOutdated
                    );
                    let v: Value = serde_json::from_slice(
                        &std::fs::read(&ctx.integration_state_file).unwrap(),
                    )
                    .unwrap();
                    assert_eq!(v["codex"]["kept"], true);
                    assert_eq!(v["unknown"], 17);
                }
            }
        }
        #[test]
        fn interrupted_new_install_rolls_back_only_created_empty_directories() {
            let (_home, ctx) = context();
            fail("file-applied");
            assert!(mutate(&ctx, false).is_err());
            clear();
            let state = Dir::open(ctx.integration_state_file.parent().unwrap()).unwrap();
            recover(&ctx, &state).unwrap();
            assert!(!ctx.target_path.exists());
            assert!(!ctx.target_path.ancestors().nth(3).unwrap().exists());
            assert!(!ctx.integration_state_file.exists());
        }
        #[test]
        fn interrupted_commit_finishes_cleanup_without_rollback() {
            let (_home, ctx) = context();
            fail("committed");
            assert!(mutate(&ctx, false).is_err());
            clear();
            let state = Dir::open(ctx.integration_state_file.parent().unwrap()).unwrap();
            recover(&ctx, &state).unwrap();
            assert_eq!(
                super::super::probe(&ctx).unwrap().state,
                RuleState::OwnedCurrent
            );
        }
        #[test]
        fn recovery_preserves_drift_and_preimages() {
            let (_home, mut ctx) = context();
            super::super::install_unlocked(&ctx).unwrap();
            ctx.generation_version = "2".into();
            ctx.rendered_bytes = body("new");
            fail("file-applied");
            assert!(mutate(&ctx, false).is_err());
            clear();
            std::fs::write(&ctx.target_path, b"user edit").unwrap();
            let state = Dir::open(ctx.integration_state_file.parent().unwrap()).unwrap();
            assert!(recover(&ctx, &state).is_err());
            assert_eq!(std::fs::read(&ctx.target_path).unwrap(), b"user edit");
            assert!(read_journal(&state).unwrap().is_some());
        }
        #[test]
        fn recovery_can_itself_be_interrupted_and_reentered() {
            for point in [
                "recovery-target-applied",
                "recovery-ledger-applied",
                "recovered",
            ] {
                let (_home, mut ctx) = context();
                put_ledger(&ctx);
                super::super::install_unlocked(&ctx).unwrap();
                let old = ctx.rendered_bytes.clone();
                ctx.generation_version = "2".into();
                ctx.rendered_bytes = body("replacement");
                fail("ledger-applied");
                assert!(mutate(&ctx, false).is_err());
                clear();
                let state = Dir::open(ctx.integration_state_file.parent().unwrap()).unwrap();
                fail(point);
                assert!(recover(&ctx, &state).is_err(), "{point}");
                clear();
                recover(&ctx, &state).unwrap_or_else(|e| panic!("{point}: {e:#}"));
                recover(&ctx, &state).unwrap();
                assert_eq!(std::fs::read(&ctx.target_path).unwrap(), old);
                assert_eq!(
                    super::super::probe(&ctx).unwrap().state,
                    RuleState::OwnedOutdated
                );
            }
        }
        #[test]
        fn pending_ledger_and_preimage_drift_are_preserved() {
            for drift_preimage in [false, true] {
                let (_home, mut ctx) = context();
                put_ledger(&ctx);
                super::super::install_unlocked(&ctx).unwrap();
                ctx.generation_version = "2".into();
                ctx.rendered_bytes = body("replacement");
                fail("file-applied");
                assert!(mutate(&ctx, false).is_err());
                clear();
                let state = Dir::open(ctx.integration_state_file.parent().unwrap()).unwrap();
                let journal = legacy_journal(&state);
                let drift_path = if drift_preimage {
                    state
                        .path
                        .join(PRIVATE_STATE)
                        .join(journal.transaction_dir.as_deref().unwrap())
                        .join("target.preimage")
                } else {
                    ctx.integration_state_file.clone()
                };
                std::fs::write(&drift_path, b"external edit").unwrap();
                assert!(recover(&ctx, &state).is_err());
                assert_eq!(std::fs::read(&drift_path).unwrap(), b"external edit");
                assert!(read_journal(&state).unwrap().is_some());
            }
        }
        #[test]
        fn prepared_recovery_retains_exact_ledger_bytes() {
            let (_home, mut ctx) = context();
            put_ledger(&ctx);
            super::super::install_unlocked(&ctx).unwrap();
            let before = std::fs::read(&ctx.integration_state_file).unwrap();
            ctx.generation_version = "2".into();
            ctx.rendered_bytes = body("replacement");
            fail("prepared");
            assert!(mutate(&ctx, false).is_err());
            clear();
            let state = Dir::open(ctx.integration_state_file.parent().unwrap()).unwrap();
            recover(&ctx, &state).unwrap();
            assert_eq!(std::fs::read(&ctx.integration_state_file).unwrap(), before);
        }
        #[test]
        fn recovery_accepts_directory_entry_count_changes_but_rejects_identity_drift() {
            for field in ["links", "device", "inode", "owner", "mode"] {
                let (_home, mut ctx) = context();
                put_ledger(&ctx);
                super::super::install_unlocked(&ctx).unwrap();
                let target = std::fs::read(&ctx.target_path).unwrap();
                let ledger = std::fs::read(&ctx.integration_state_file).unwrap();
                ctx.generation_version = "2".into();
                ctx.rendered_bytes = body("replacement");
                fail("prepared");
                assert!(mutate(&ctx, false).is_err());
                clear();
                let state = Dir::open(ctx.integration_state_file.parent().unwrap()).unwrap();
                let mut journal = legacy_journal(&state);
                // Model mutable directory entry counts independently of the host filesystem.
                let expected = journal.transaction_directory_identity.as_mut().unwrap();
                match field {
                    "links" => expected.links += 1,
                    "device" => expected.device ^= 1,
                    "inode" => expected.inode ^= 1,
                    "owner" => expected.owner ^= 1,
                    "mode" => expected.mode ^= 0o100,
                    _ => unreachable!(),
                }
                save_journal(&state, &journal).unwrap();
                let result = recover(&ctx, &state);
                if field == "links" {
                    result.unwrap();
                    assert!(read_journal(&state).unwrap().is_none());
                } else {
                    assert!(result
                        .unwrap_err()
                        .to_string()
                        .contains("directory identity drift"));
                    assert!(read_journal(&state).unwrap().is_some());
                }
                assert_eq!(std::fs::read(&ctx.target_path).unwrap(), target);
                assert_eq!(std::fs::read(&ctx.integration_state_file).unwrap(), ledger);
            }
        }

        #[test]
        fn recovery_preserves_unknown_transaction_directory_entries() {
            let (_home, ctx) = context();
            fail("prepared");
            assert!(mutate(&ctx, false).is_err());
            clear();
            let state = Dir::open(ctx.integration_state_file.parent().unwrap()).unwrap();
            let journal = legacy_journal(&state);
            let unknown = state
                .path
                .join(PRIVATE_STATE)
                .join(journal.transaction_dir.as_deref().unwrap())
                .join("unknown");
            std::fs::write(&unknown, b"preserve unknown bytes").unwrap();
            assert!(recover(&ctx, &state).is_err());
            assert_eq!(std::fs::read(&unknown).unwrap(), b"preserve unknown bytes");
            assert!(read_journal(&state).unwrap().is_some());
        }

        #[test]
        fn new_journal_embeds_preimages_without_duplicate_files() {
            let (_home, mut ctx) = context();
            put_ledger(&ctx);
            super::super::install_unlocked(&ctx).unwrap();
            let old_target = std::fs::read(&ctx.target_path).unwrap();
            let old_ledger = std::fs::read(&ctx.integration_state_file).unwrap();
            ctx.rendered_bytes = body("new");
            fail("prepared");
            assert!(mutate(&ctx, false).is_err());
            clear();
            let state = Dir::open(ctx.integration_state_file.parent().unwrap()).unwrap();
            let saved = read_journal(&state).unwrap().unwrap();
            let value: Value = serde_json::from_slice(&saved.bytes).unwrap();
            assert_eq!(value["format_version"], 1);
            for key in [
                "transaction_dir",
                "transaction_directory_identity",
                "target_preimage",
                "ledger_preimage",
            ] {
                assert!(value.get(key).is_none(), "{key}");
            }
            let journal: Journal = serde_json::from_slice(&saved.bytes).unwrap();
            assert_eq!(journal.old_target.unwrap().bytes, old_target);
            assert_eq!(journal.old_ledger.unwrap().bytes, old_ledger);
            let private = state.path.join(PRIVATE_STATE);
            let entries: Vec<_> = std::fs::read_dir(private)
                .unwrap()
                .map(|entry| entry.unwrap().file_name())
                .collect();
            assert_eq!(entries, vec![std::ffi::OsString::from(JOURNAL)]);
            recover(&ctx, &state).unwrap();
        }

        #[test]
        fn legacy_pending_phases_recover_with_original_material_checks() {
            for uninstall in [false, true] {
                for point in [
                    "prepared",
                    "staged",
                    "target-applied-before-journal",
                    "file-applied",
                    "ledger-applied-before-journal",
                    "ledger-applied",
                    "committed",
                    "recovery-target-applied",
                    "recovery-ledger-applied",
                    "recovered",
                ] {
                    let (_home, mut ctx) = context();
                    put_ledger(&ctx);
                    super::super::install_unlocked(&ctx).unwrap();
                    let original = ctx.rendered_bytes.clone();
                    ctx.rendered_bytes = body("new");
                    ctx.generation_version = "2".into();
                    let recovering = point.starts_with("recovery-") || point == "recovered";
                    fail(if recovering { "ledger-applied" } else { point });
                    assert!(mutate(&ctx, uninstall).is_err());
                    clear();
                    let state = Dir::open(ctx.integration_state_file.parent().unwrap()).unwrap();
                    let journal = legacy_journal(&state);
                    if recovering {
                        fail(point);
                        assert!(recover(&ctx, &state).is_err(), "{uninstall} {point}");
                        clear();
                    }
                    recover(&ctx, &state)
                        .unwrap_or_else(|error| panic!("{uninstall} {point}: {error:#}"));
                    recover(&ctx, &state).unwrap();
                    if point == "committed" {
                        if uninstall {
                            assert!(!ctx.target_path.exists());
                        } else {
                            assert_eq!(
                                std::fs::read(&ctx.target_path).unwrap(),
                                ctx.rendered_bytes
                            );
                        }
                    } else {
                        assert_eq!(std::fs::read(&ctx.target_path).unwrap(), original);
                        assert_eq!(
                            super::super::probe(&ctx).unwrap().state,
                            RuleState::OwnedOutdated
                        );
                    }
                    assert!(!state
                        .path
                        .join(PRIVATE_STATE)
                        .join(journal.transaction_dir.unwrap())
                        .exists());
                    assert!(read_journal(&state).unwrap().is_none());
                    let ledger: Value = serde_json::from_slice(
                        &std::fs::read(&ctx.integration_state_file).unwrap(),
                    )
                    .unwrap();
                    assert_eq!(ledger["unknown"], 17);
                    assert_eq!(ledger["codex"]["kept"], true);
                }
            }
        }

        #[test]
        fn journal_rejects_unknown_versions_and_inconsistent_embedded_blobs() {
            for field in [
                "format_version",
                "old_target",
                "old_ledger",
                "new_target",
                "new_ledger",
                "recovery_target",
                "recovery_ledger",
            ] {
                let (_home, mut ctx) = context();
                put_ledger(&ctx);
                super::super::install_unlocked(&ctx).unwrap();
                ctx.rendered_bytes = body("new");
                fail("ledger-applied");
                assert!(mutate(&ctx, false).is_err());
                clear();
                let state = Dir::open(ctx.integration_state_file.parent().unwrap()).unwrap();
                if field.starts_with("recovery_") {
                    fail("recovery-ledger-applied");
                    assert!(recover(&ctx, &state).is_err());
                    clear();
                }
                let saved = read_journal(&state).unwrap().unwrap();
                let mut j: Journal = serde_json::from_slice(&saved.bytes).unwrap();
                if field == "format_version" {
                    j.format_version = 2;
                } else {
                    let blob = match field {
                        "old_target" => &mut j.old_target,
                        "old_ledger" => &mut j.old_ledger,
                        "new_target" => &mut j.new_target,
                        "new_ledger" => &mut j.new_ledger,
                        "recovery_target" => &mut j.recovery_target,
                        "recovery_ledger" => &mut j.recovery_ledger,
                        _ => unreachable!(),
                    };
                    blob.as_mut().unwrap().bytes.push(b'!');
                }
                save_journal(&state, &j).unwrap();
                let target_before = std::fs::read(&ctx.target_path).unwrap();
                let ledger_before = std::fs::read(&ctx.integration_state_file).unwrap();
                let journal_before = read_journal(&state).unwrap().unwrap().bytes;
                assert!(recover(&ctx, &state).is_err(), "{field}");
                assert_eq!(std::fs::read(&ctx.target_path).unwrap(), target_before);
                assert_eq!(
                    std::fs::read(&ctx.integration_state_file).unwrap(),
                    ledger_before
                );
                assert_eq!(read_journal(&state).unwrap().unwrap().bytes, journal_before);
            }
        }

        #[test]
        fn existing_shared_state_parent_works_with_private_journal() {
            let (_home, ctx) = context();
            std::fs::set_permissions(
                ctx.integration_state_file.parent().unwrap(),
                std::fs::Permissions::from_mode(0o755),
            )
            .unwrap();
            super::super::install_unlocked(&ctx).unwrap();
            let state = Dir::open(ctx.integration_state_file.parent().unwrap()).unwrap();
            let private_state = journal_dir(&state, false).unwrap().unwrap();
            assert_eq!(private_state.file.metadata().unwrap().mode() & 0o777, 0o700);
            assert!(read_journal(&state).unwrap().is_none());
        }
        #[test]
        fn duplicate_yaml_keys_and_content_edits_are_rejected() {
            let (_home, ctx) = context();
            let text = String::from_utf8(ctx.rendered_bytes.clone())
                .unwrap()
                .replace("trigger: always_on", "trigger: always_on\ntrigger: manual");
            assert!(validate_format(text.as_bytes()).is_err());
            super::super::install_unlocked(&ctx).unwrap();
            std::fs::write(&ctx.target_path, body("external body")).unwrap();
            assert_eq!(
                super::super::probe(&ctx).unwrap().state,
                RuleState::Conflict
            );
            assert!(super::super::uninstall_unlocked(&ctx).is_err());
        }
    }
}
