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
    pub platform_file_identity: FileIdentity,
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
            Ok(Self {
                path: self.path.join(component),
                file: unsafe { File::from_raw_fd(cvt(fd)?) },
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
            file.write_all(bytes)?;
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
            self.verify()?;
            let child = self.child(component)?;
            if &identity(&child.file.metadata()?) != expected {
                bail!("created directory identity drift");
            }
            let n = name(component)?;
            if unsafe { libc::unlinkat(self.file.as_raw_fd(), n.as_ptr(), libc::AT_REMOVEDIR) } != 0
            {
                return Err(std::io::Error::last_os_error().into());
            }
            self.sync()
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
            platform_file_identity: blob.identity.clone(),
        }
    }
    fn verify_owned(ctx: &RuleContext, blob: &Blob, record: &RuleOwnership) -> bool {
        record.path == ctx.target_path
            && record.sha256 == blob.sha256
            && record.platform_file_identity == blob.identity
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
        transaction_id: String,
        target: PathBuf,
        ledger: PathBuf,
        operation: String,
        phase: String,
        transaction_dir: String,
        transaction_directory_identity: FileIdentity,
        old_target: Option<Blob>,
        old_ledger: Option<Blob>,
        target_preimage: Option<Preimage>,
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
        if j.target != ctx.target_path
            || j.ledger != ctx.integration_state_file
            || j.transaction_dir != format!(".antigravity-rule-txn-{id}")
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
        if identity(&txn.file.metadata()?) != j.transaction_directory_identity {
            bail!("NeedsRepair: transaction directory identity drift");
        }
        Ok(())
    }
    fn cleanup(
        state: &Dir,
        target: Option<&Dir>,
        txn: &Dir,
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
                j.transaction_dir.as_ref(),
                &j.transaction_directory_identity,
            )?;
        if rollback {
            for d in j.created_dirs.iter().rev() {
                match optional_dir(&d.path)? {
                    None => {}
                    Some(_) => {
                        Dir::open(d.path.parent().context("created directory parent")?)?
                            .remove_empty(
                                d.path.file_name().context("created directory name")?,
                                &d.identity,
                            )?;
                    }
                }
            }
        }
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
            if let Some(txn) =
                optional_dir(&state.path.join(PRIVATE_STATE).join(&j.transaction_dir))?
            {
                require_transaction_directory(&txn, &j)?;
                return cleanup(state, target.as_ref(), &txn, &j, j.phase == "recovered");
            }
            // Transaction directory may already be gone, but only final journal removal remains.
            if j.phase == "recovered" {
                for d in j.created_dirs.iter().rev() {
                    if optional_dir(&d.path)?.is_some() {
                        Dir::open(d.path.parent().context("created parent")?)?.remove_empty(
                            d.path.file_name().context("created name")?,
                            &d.identity,
                        )?;
                    }
                }
            }
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
        let txn = journal_dir(state, false)?
            .context("missing private transaction state")?
            .child(j.transaction_dir.as_ref())?;
        require_transaction_directory(&txn, &j)?;
        let old_bytes = checked_preimage(&txn, j.target_preimage.as_ref(), j.old_target.as_ref())?;
        let old_ledger_bytes =
            checked_preimage(&txn, j.ledger_preimage.as_ref(), j.old_ledger.as_ref())?;
        let mut restored = match &old_ledger_bytes {
            Some(b) => serde_json::from_slice::<Value>(b)?,
            None => serde_json::json!({}),
        };
        let mut target_after = current.clone();
        if !same(current.as_ref(), j.old_target.as_ref())
            && !(j.recovery_target.is_some() && same(current.as_ref(), j.recovery_target.as_ref()))
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
                    None => dir.create(&j.recovery_target_stage, bytes)?,
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
        // Restoring bytes by rename gives a new inode. Reconcile that identity in the old ledger,
        // retaining every unrelated family's fields instead of restoring stale ownership.
        let mut reconciled = false;
        if let Some(mut old_record) = ownership(&restored)? {
            if let Some(blob) = target_after.as_ref() {
                if old_record.platform_file_identity != blob.identity {
                    old_record.platform_file_identity = blob.identity.clone();
                    old_record.sha256 = blob.sha256.clone();
                    set_ownership(&mut restored, Some(&old_record))?;
                    reconciled = true;
                }
            }
        }
        let restore_ledger = if reconciled {
            Some(serde_json::to_vec_pretty(&restored)?)
        } else {
            old_ledger_bytes
        };
        if let Some(bytes) = restore_ledger {
            if !same(current_ledger.as_ref(), j.recovery_ledger.as_ref())
                || j.recovery_ledger.is_none()
            {
                let staged = match state.read(&j.recovery_ledger_stage)? {
                    Some(b) => {
                        require_same(Some(&b), j.recovery_ledger.as_ref(), "restore ledger stage")?;
                        b
                    }
                    None => state.create(&j.recovery_ledger_stage, &bytes)?,
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
        cleanup(state, target.as_ref(), &txn, &j, true)
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
    fn checkpoint(point: &'static str) -> Result<()> {
        #[cfg(test)]
        if FAIL_AT.with(|v| v.get() == Some(point)) || FAIL_ALSO.with(|v| v.get() == Some(point)) {
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
        let transaction_dir = format!(".antigravity-rule-txn-{id}");
        let txn = journal_dir(&state, true)?
            .context("private state directory")?
            .create_dir(transaction_dir.as_ref())?;
        let mut j = Journal {
            transaction_id: id.clone(),
            target: ctx.target_path.clone(),
            ledger: ctx.integration_state_file.clone(),
            operation: if uninstall { "uninstall" } else { "install" }.into(),
            phase: "prepared".into(),
            transaction_dir,
            transaction_directory_identity: identity(&txn.file.metadata()?),
            target_preimage: backup(&txn, "target.preimage", old_target.as_ref())?,
            ledger_preimage: backup(&txn, "ledger.preimage", old_ledger.as_ref())?,
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
            let staged = dir.create(&j.target_stage, &ctx.rendered_bytes)?;
            j.new_target = Some(staged.clone());
            set_ownership(&mut value, Some(&record(ctx, &staged)))?;
        } else {
            set_ownership(&mut value, None)?;
        }
        j.new_ledger = Some(state.create(&j.ledger_stage, &serde_json::to_vec_pretty(&value)?)?);
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
        cleanup(&state, target.as_ref(), &txn, &j, false)
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use std::os::unix::fs::{symlink, PermissionsExt};
        fn context() -> (tempfile::TempDir, RuleContext) {
            let home = tempfile::tempdir().unwrap();
            let state = home.path().join(".askhuman");
            std::fs::create_dir(&state).unwrap();
            std::fs::set_permissions(&state, std::fs::Permissions::from_mode(0o700)).unwrap();
            let ctx = RuleContext {
                target_path: home
                    .path()
                    .join(".gemini/config/rules/askhuman-antigravity.md"),
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
        fn fail(point: &'static str) {
            FAIL_AT.with(|v| v.set(Some(point)));
        }
        fn clear() {
            FAIL_AT.with(|v| v.set(None));
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
        fn same_bytes_new_inode_and_content_drift_are_conflicts() {
            let (_home, ctx) = context();
            super::super::install_unlocked(&ctx).unwrap();
            let replacement = ctx.target_path.with_extension("replacement");
            std::fs::write(&replacement, &ctx.rendered_bytes).unwrap();
            std::fs::rename(replacement, &ctx.target_path).unwrap();
            assert_eq!(
                super::super::probe(&ctx).unwrap().state,
                RuleState::Conflict
            );
            assert!(super::super::uninstall_unlocked(&ctx).is_err());
            assert_eq!(std::fs::read(&ctx.target_path).unwrap(), ctx.rendered_bytes);
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
                let journal: Journal =
                    serde_json::from_slice(&read_journal(&state).unwrap().unwrap().bytes).unwrap();
                let drift_path = if drift_preimage {
                    state
                        .path
                        .join(PRIVATE_STATE)
                        .join(&journal.transaction_dir)
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
