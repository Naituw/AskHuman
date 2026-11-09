//! Resolve one process target before configuration, secrets, or a graphical runtime is loaded.

use super::state;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

pub const TARGET_ENV: &str = "ASKHUMAN_RESOLVED_TARGET_V1";
pub const FIXTURE_FLAG: &str = "--internal-system-fixture-v2";
pub const FIXTURE_ACK: &str = "ASKHUMAN_SYSTEM_FIXTURE_V2_ACTIVE";
static FIXTURE: OnceLock<Fixture> = OnceLock::new();

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Fixture {
    pub version: u32,
    pub account_home: PathBuf,
}

pub fn fixture() -> Result<Option<Fixture>, String> {
    if let Some(fixture) = FIXTURE.get() {
        return Ok(Some(fixture.clone()));
    }
    Ok(inherited()?.and_then(|target| target.fixture))
}

pub fn is_fixture() -> bool {
    fixture().ok().flatten().is_some()
}

pub fn legacy_system() -> bool {
    inherited().ok().flatten().is_some_and(|target| {
        target.kind == TargetKind::System
            && target.fixture.is_none()
            && crate::paths::account::home().map_or(true, |home| {
                !state::same_path(&target.home, &home.join(".askhuman"))
            })
    })
}

pub fn shared_writes_blocked() -> bool {
    is_fixture() || legacy_system()
}

pub fn background_account_home() -> Option<PathBuf> {
    inherited()
        .ok()
        .flatten()
        .filter(|target| target.kind == TargetKind::System)
        .and_then(|target| target.home.parent().map(Path::to_path_buf))
}

pub fn guard_fixture_program(program: &std::ffi::OsStr) -> std::io::Result<()> {
    if is_fixture() {
        check_fixture_capability(Path::new(program)).map_err(std::io::Error::other)?;
    }
    Ok(())
}

pub fn replacement_compatible() -> bool {
    if !is_fixture() {
        return true;
    }
    static CHECKED: std::sync::Mutex<Option<(crate::daemon::lifecycle::Fingerprint, bool)>> =
        std::sync::Mutex::new(None);
    let Ok(binary) = crate::executable::launch_path() else {
        return false;
    };
    let fingerprint = crate::daemon::lifecycle::fingerprint_at(&binary);
    let mut checked = CHECKED.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((previous, compatible)) = *checked {
        if previous == fingerprint {
            return compatible;
        }
    }
    let result = check_fixture_capability(&binary);
    if let Err(error) = &result {
        eprintln!("askhuman: retaining the System fixture service: {error}");
    }
    let compatible = result.is_ok();
    *checked = Some((fingerprint, compatible));
    compatible
}

pub fn require_config_writes() -> Result<(), String> {
    if legacy_system() {
        Err("this inherited legacy System home is read-only for configuration; finish its pending calls and use the normal account instance to change settings".into())
    } else {
        Ok(())
    }
}

fn contextualize(mut target: Target) -> Result<Target, String> {
    target.fixture = fixture()?;
    if target.fixture.is_some() {
        target.version = 2;
        target.no_keychain = true;
    }
    Ok(target)
}
/// Integration artifacts must retain the selected installation entry, even when the running
/// image is reached through a symlink or retained by the Linux publisher.
pub fn program_path() -> std::io::Result<PathBuf> {
    match inherited().map_err(std::io::Error::other)? {
        Some(target) => Ok(target.binary),
        None => crate::executable::launch_path(),
    }
}

fn formal_binary(fallback: &Path) -> Result<PathBuf, String> {
    let root = executable_root(fallback).or_else(|| {
        std::env::current_dir()
            .ok()
            .and_then(|cwd| super::workspace::find_root(&cwd))
    });
    let binary = match root.filter(|root| super::workspace::is_project(root)) {
        Some(root) => super::workspace::read(&root)?.system_binary,
        None => fallback.to_path_buf(),
    };
    if executable_root(&binary).is_some() {
        return Err("the formal entry cannot point into a Dev Instance".into());
    }
    if !binary.is_file() {
        return Err("the formal AskHuman entry is missing".into());
    }
    Ok(binary)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TargetKind {
    System,
    Dev,
    Isolated,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Target {
    pub version: u32,
    pub kind: TargetKind,
    pub binary: PathBuf,
    pub home: PathBuf,
    pub worktree: Option<PathBuf>,
    pub reason: String,
    #[serde(default)]
    pub no_keychain: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fixture: Option<Fixture>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Selection {
    System,
    Dev,
}

#[derive(Debug, PartialEq, Eq)]
struct Prefix {
    selection: Option<Selection>,
    worktree: Option<PathBuf>,
    argv: Vec<String>,
}

fn parse_prefix(argv: &[String]) -> Result<Prefix, String> {
    let mut parsed = Prefix {
        selection: None,
        worktree: None,
        argv: argv.first().cloned().into_iter().collect(),
    };
    let mut index = 1;
    while let Some(arg) = argv.get(index) {
        let (name, inline) = arg
            .split_once('=')
            .map_or((arg.as_str(), None), |(a, b)| (a, Some(b)));
        match name {
            "--instance" | "--worktree" => {
                let value = if let Some(value) = inline {
                    value
                } else {
                    index += 1;
                    argv.get(index)
                        .ok_or_else(|| format!("{name} requires a value"))?
                };
                if name == "--instance" {
                    if parsed.selection.is_some() {
                        return Err("--instance may only be specified once".into());
                    }
                    parsed.selection = Some(match value {
                        "system" => Selection::System,
                        "dev" => Selection::Dev,
                        _ => return Err("--instance must be system or dev".into()),
                    });
                } else if parsed.worktree.replace(PathBuf::from(value)).is_some() {
                    return Err("--worktree may only be specified once".into());
                }
            }
            "--caller-agent" if inline.is_none() => {
                parsed.argv.push(arg.clone());
                index += 1;
                parsed.argv.push(
                    argv.get(index)
                        .ok_or("--caller-agent requires a value")?
                        .clone(),
                );
            }
            _ => {
                parsed.argv.extend_from_slice(&argv[index..]);
                break;
            }
        }
        index += 1;
    }
    if parsed.worktree.is_some() && parsed.selection != Some(Selection::Dev) {
        return Err("--worktree requires --instance dev".into());
    }
    Ok(parsed)
}

pub fn inherited() -> Result<Option<Target>, String> {
    match std::env::var(TARGET_ENV) {
        Ok(raw) => Ok(Some(decode_target(&raw)?)),
        Err(std::env::VarError::NotPresent) => Ok(None),
        Err(_) => Err("inherited AskHuman target is not valid Unicode".into()),
    }
}

fn decode_target(raw: &str) -> Result<Target, String> {
    let value: serde_json::Value =
        serde_json::from_str(raw).map_err(|e| format!("invalid inherited AskHuman target: {e}"))?;
    if value["version"] == 1 && value.get("fixture").is_some() {
        return Err("an inherited v1 target cannot carry a fixture field".into());
    }
    let target: Target = serde_json::from_value(value)
        .map_err(|e| format!("invalid inherited AskHuman target: {e}"))?;
    let context_valid = match (&target.fixture, target.version) {
        (None, 1) => true,
        (Some(fixture), 2) => {
            fixture.version == 2 && fixture.account_home.is_absolute() && target.no_keychain
        }
        _ => false,
    };
    if !context_valid || !target.binary.is_absolute() || !target.home.is_absolute() {
        return Err("inherited AskHuman target has an unsupported version or relative path".into());
    }
    if target.kind == TargetKind::System
        && (target.worktree.is_some()
            || target.home.file_name() != Some(std::ffi::OsStr::new(".askhuman")))
    {
        return Err("inherited System home must name an account .askhuman directory; use ASKHUMAN_HOME for an explicit isolated path".into());
    }
    if target.kind == TargetKind::System
        && target.fixture.as_ref().is_some_and(|fixture| {
            !state::same_path(&target.home, &fixture.account_home.join(".askhuman"))
        })
    {
        return Err("inherited System home conflicts with its fixture account".into());
    }
    Ok(target)
}

fn executable_root(binary: &Path) -> Option<PathBuf> {
    let bin = binary.parent()?;
    let dev = bin.parent()?;
    (bin.file_name()? == "bin" && dev.file_name()? == crate::dev_instance::DEV_DIR)
        .then(|| dev.parent().map(Path::to_path_buf))
        .flatten()
}

pub fn dev_target(root: &Path) -> Result<Target, String> {
    let root = root
        .canonicalize()
        .map_err(|e| format!("invalid worktree {}: {e}", root.display()))?;
    if !super::workspace::is_enabled(&root) {
        return Err(format!("Dev Instance is not enabled at {}", root.display()));
    }
    if super::workspace::is_project(&root) {
        super::workspace::read(&root)?;
    }
    let binary = crate::dev_instance::instance_bin(&root);
    if !binary.is_file() {
        return Err(format!(
            "Dev binary is missing: {}; run this worktree's install script",
            binary.display()
        ));
    }
    let dev_directory = root
        .join(crate::dev_instance::DEV_DIR)
        .canonicalize()
        .map_err(|e| e.to_string())?;
    let home = crate::dev_instance::instance_home(&root)
        .canonicalize()
        .map_err(|e| format!("Dev home is missing: {e}"))?;
    let program = binary.canonicalize().map_err(|e| e.to_string())?;
    if !dev_directory.starts_with(&root)
        || !home.starts_with(&dev_directory)
        || !program.starts_with(&dev_directory)
        || state::same_path(&home, &state::system_home()?)
    {
        return Err("Dev program and home must remain inside this worktree's private .askhuman-dev directory".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if std::fs::metadata(&binary)
            .map_err(|e| e.to_string())?
            .permissions()
            .mode()
            & 0o111
            == 0
        {
            return Err(format!(
                "Dev binary is not executable: {}",
                binary.display()
            ));
        }
    }
    contextualize(Target {
        version: 1,
        kind: TargetKind::Dev,
        binary,
        home,
        worktree: Some(root),
        reason: "explicit Dev verification".into(),
        no_keychain: true,
        fixture: None,
    })
}

pub fn validate_home(target: &Target, external: Option<&Path>) -> Result<(), String> {
    if let Some(home) = external {
        if target.kind == TargetKind::System || !state::same_path(home, &target.home) {
            return Err(format!(
                "AskHuman target conflicts with ASKHUMAN_HOME={}; expected {} ({:?})",
                home.display(),
                target.home.display(),
                target.kind
            ));
        }
    }
    Ok(())
}

fn validate_fixture_target(target: &Target) -> Result<(), String> {
    if target.fixture.is_some() {
        let production = crate::paths::account::home()?.join(".askhuman");
        let production = production.canonicalize().unwrap_or(production);
        let ancestor = target
            .home
            .ancestors()
            .find(|path| path.exists())
            .ok_or("System fixture data root has no existing ancestor")?;
        if target
            .home
            .strip_prefix(ancestor)
            .map_err(|error| error.to_string())?
            .components()
            .any(|part| part == std::path::Component::ParentDir)
        {
            return Err("System fixture data root has an unresolved parent traversal".into());
        }
        let data = ancestor
            .canonicalize()
            .map_err(|error| error.to_string())?
            .join(
                target
                    .home
                    .strip_prefix(ancestor)
                    .map_err(|error| error.to_string())?,
            );
        if data.starts_with(&production) || production.starts_with(&data) {
            return Err("System fixture target conflicts with production data".into());
        }
    }
    Ok(())
}

fn acknowledge_fixture(target: &Target) -> Result<(), String> {
    validate_fixture_target(target)?;
    let fixture = target
        .fixture
        .as_ref()
        .ok_or("System fixture acknowledgement has no context")?;
    eprintln!(
        "{FIXTURE_ACK} {}",
        serde_json::json!({
            "version": 2, "accountHome": fixture.account_home,
            "kind": target.kind, "home": target.home,
        })
    );
    Ok(())
}

fn apply(target: &Target) -> Result<(), String> {
    validate_fixture_target(target)?;
    // Bootstrap runs before Tokio, GTK, WebView, or worker threads.
    std::env::set_var(
        TARGET_ENV,
        serde_json::to_string(target).map_err(|e| e.to_string())?,
    );
    if target.kind == TargetKind::System {
        std::env::remove_var(crate::dev_instance::ASKHUMAN_HOME_ENV);
        if target.no_keychain {
            std::env::set_var("ASKHUMAN_NO_KEYCHAIN", "1");
        } else {
            std::env::remove_var("ASKHUMAN_NO_KEYCHAIN");
        }
    } else {
        std::env::set_var(crate::dev_instance::ASKHUMAN_HOME_ENV, &target.home);
        std::env::set_var("ASKHUMAN_NO_KEYCHAIN", "1");
    }
    Ok(())
}

pub fn configure(command: &mut Command, target: &Target) -> Result<(), String> {
    validate_fixture_target(target)?;
    if target.fixture.is_some() {
        check_fixture_capability(&target.binary)?;
    }
    command.env(
        TARGET_ENV,
        serde_json::to_string(target).map_err(|e| e.to_string())?,
    );
    if target.kind == TargetKind::System {
        command.env_remove(crate::dev_instance::ASKHUMAN_HOME_ENV);
        command.env_remove(super::verification::VERIFICATION_ENV);
        command.env_remove(super::session::PROFILE_ENV);
        if target.no_keychain {
            command.env("ASKHUMAN_NO_KEYCHAIN", "1");
        } else {
            command.env_remove("ASKHUMAN_NO_KEYCHAIN");
        }
    } else {
        command
            .env(crate::dev_instance::ASKHUMAN_HOME_ENV, &target.home)
            .env("ASKHUMAN_NO_KEYCHAIN", "1");
    }
    Ok(())
}

pub fn pin_management(root: &Path) -> Result<(), String> {
    let root = root.canonicalize().map_err(|e| e.to_string())?;
    let directory = root
        .join(crate::dev_instance::DEV_DIR)
        .canonicalize()
        .map_err(|e| e.to_string())?;
    let home = crate::dev_instance::instance_home(&root);
    if !directory.starts_with(&root)
        || home
            .canonicalize()
            .is_ok_and(|home| !home.starts_with(&directory))
    {
        return Err("Dev data-management target escapes the private worktree directory".into());
    }
    let target = contextualize(Target {
        version: 1,
        kind: TargetKind::Isolated,
        binary: crate::executable::launch_path().map_err(|e| e.to_string())?,
        home,
        worktree: Some(root),
        reason: "explicit Dev data-management operation".into(),
        no_keychain: true,
        fixture: None,
    })?;
    apply(&target)?;
    // This is a short-lived CLI data operation, outside any test request or client profile.
    for key in [
        super::session::PROFILE_ENV,
        super::verification::VERIFICATION_ENV,
        super::workbench::WINDOW_ENV,
    ] {
        std::env::remove_var(key);
    }
    Ok(())
}

pub fn diagnostic() -> Result<Target, String> {
    if let Some(mut target) = inherited()? {
        if legacy_system() {
            target.reason = format!(
                "legacy-system-home: continuing {}; normal account data {}",
                target.home.display(),
                crate::paths::account::home()
                    .map(|home| home.join(".askhuman").display().to_string())
                    .unwrap_or_else(|error| format!("unavailable ({error})"))
            );
        }
        return Ok(target);
    }
    let current = crate::executable::launch_path().map_err(|e| e.to_string())?;
    let cwd = std::env::current_dir().map_err(|e| e.to_string())?;
    if let Some(root) = executable_root(&current) {
        return dev_target(&root);
    }
    if let Some(home) =
        std::env::var_os(crate::dev_instance::ASKHUMAN_HOME_ENV).filter(|v| !v.is_empty())
    {
        let home = PathBuf::from(home);
        return contextualize(Target {
            version: 1,
            kind: TargetKind::Isolated,
            binary: current,
            home: if home.is_absolute() {
                home
            } else {
                cwd.join(home)
            },
            worktree: None,
            reason: "explicit ASKHUMAN_HOME".into(),
            no_keychain: true,
            fixture: None,
        });
    }
    if let Some(root) = crate::dev_instance::find_dev_root(&cwd) {
        return dev_target(&root);
    }
    contextualize(Target {
        version: 1,
        kind: TargetKind::System,
        binary: current,
        home: state::system_home()?,
        worktree: None,
        reason: "everyday system entry".into(),
        no_keychain: false,
        fixture: None,
    })
}

fn reexec(target: &Target, argv: &[String]) -> Result<(), String> {
    check_capability(&target.binary)?;
    let mut command = Command::new(&target.binary);
    crate::executable::clear_launch_context(&mut command);
    configure(&mut command, target)?;
    command.args(&argv[1..]);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        Err(format!(
            "cannot enter selected AskHuman: {}",
            command.exec()
        ))
    }
    #[cfg(not(unix))]
    {
        let status = command.status().map_err(|e| e.to_string())?;
        std::process::exit(status.code().unwrap_or(1));
    }
}

pub fn check_capability(binary: &Path) -> Result<(), String> {
    let value = capabilities(binary)?;
    if value["developmentVerification"].as_u64() != Some(super::CAPABILITY_VERSION as u64) {
        return Err(format!(
            "{} does not support isolated Dev verification; install a compatible build",
            binary.display()
        ));
    }
    if is_fixture() && value["systemFixture"].as_u64() != Some(2) {
        return Err(format!(
            "{} does not support System fixture v2",
            binary.display()
        ));
    }
    Ok(())
}

pub fn check_fixture_capability(binary: &Path) -> Result<(), String> {
    if capabilities(binary)?["systemFixture"].as_u64() != Some(2) {
        return Err(format!(
            "{} does not support System fixture v2; the selected private target was retained",
            binary.display()
        ));
    }
    Ok(())
}

fn capabilities(binary: &Path) -> Result<serde_json::Value, String> {
    let mut child = Command::new(binary)
        .arg("--development-capabilities")
        .env_remove(TARGET_ENV)
        .env_remove(crate::dev_instance::ASKHUMAN_HOME_ENV)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("cannot probe {}: {e}", binary.display()))?;
    let until = Instant::now() + Duration::from_secs(3);
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if Instant::now() < until => std::thread::sleep(Duration::from_millis(10)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("AskHuman development capability probe timed out".into());
            }
        }
    }
    let output = child.wait_with_output().map_err(|e| e.to_string())?;
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap_or_default();
    if !output.status.success() {
        return Err(format!(
            "{} does not support isolated Dev verification; install a compatible build",
            binary.display()
        ));
    }
    Ok(value)
}

pub fn prepare(mut argv: Vec<String>) -> Result<Vec<String>, String> {
    // The known capability flag makes older binaries exit before loading any configuration.
    let explicit_fixture = if argv
        .get(1)
        .is_some_and(|s| s == "--development-capabilities")
    {
        if argv.len() == 2 {
            return Ok(argv);
        }
        if argv.get(2).is_none_or(|s| s != FIXTURE_FLAG)
            || argv.get(4).is_none_or(|s| s != "--")
            || argv.len() < 6
        {
            return Err("invalid protected System fixture invocation".into());
        }
        let fixture = Fixture {
            version: 2,
            account_home: crate::paths::account::validate_fixture(Path::new(&argv[3]))?,
        };
        argv.drain(1..5);
        Some(fixture)
    } else {
        if argv.get(1).is_some_and(|s| s == FIXTURE_FLAG) {
            return Err(
                "System fixtures require the protected --development-capabilities prefix".into(),
            );
        }
        if argv.get(1).is_some_and(|s| s == "__dev-capabilities") && argv.len() == 2 {
            return Ok(argv);
        }
        None
    };
    let previous = inherited()?;
    let inherited_fixture = previous.as_ref().and_then(|target| target.fixture.clone());
    if let (Some(explicit), Some(inherited)) = (&explicit_fixture, &inherited_fixture) {
        if !state::same_path(&explicit.account_home, &inherited.account_home) {
            return Err("explicit System fixture conflicts with inherited context".into());
        }
    }
    if explicit_fixture.is_some() && previous.is_some() && inherited_fixture.is_none() {
        return Err("System fixture conflicts with an inherited v1 target".into());
    }
    let protected_entry = explicit_fixture.is_some();
    if let Some(mut fixture) = explicit_fixture.or(inherited_fixture) {
        fixture.account_home = crate::paths::account::validate_fixture(&fixture.account_home)?;
        FIXTURE
            .set(fixture)
            .map_err(|_| "System fixture was already initialized")?;
    }
    let parsed = parse_prefix(&argv)?;
    if helper_tick(&parsed.argv) && previous.is_none() && !is_fixture() {
        if let Some(home) = std::env::var_os("HOME") {
            let account = crate::paths::account::home()?;
            if !state::same_path(Path::new(&home), &account) {
                return Err("an old helper with a temporary HOME cannot select the account instance; stop that private helper and restart it with the guarded System fixture entry".into());
            }
        }
    }
    let current = crate::executable::launch_path().map_err(|e| e.to_string())?;
    let cwd = std::env::current_dir().map_err(|e| e.to_string())?;
    let home = std::env::var_os(crate::dev_instance::ASKHUMAN_HOME_ENV)
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .map(|p| if p.is_absolute() { p } else { cwd.join(p) });
    if parsed.selection.is_none() {
        if let Some(target) = previous {
            validate_helper_directory(&parsed.argv, &target)?;
            if target.kind == TargetKind::Dev {
                let root = target
                    .worktree
                    .as_deref()
                    .ok_or("inherited Dev target has no worktree")?;
                if super::workspace::is_project(root) || executable_root(&target.binary).is_some() {
                    let expected = dev_target(root)?;
                    if !state::same_path(&target.home, &expected.home)
                        || !state::same_path(&target.binary, &expected.binary)
                    {
                        return Err("inherited Dev target conflicts with its private worktree program or home".into());
                    }
                }
            }
            let worker = parsed.argv.get(1).is_some_and(|role| {
                matches!(role.as_str(), "__update-worker" | "__npm-update-worker")
            });
            if executable_root(&current).is_some() && target.kind == TargetKind::System {
                return Err("a Dev executable cannot inherit the system instance; use the configured formal entry".into());
            }
            if !worker && !state::same_path(&current, &target.binary) {
                if protected_entry {
                    acknowledge_fixture(&target)?;
                }
                return reexec(&target, &parsed.argv).map(|_| parsed.argv);
            }
            validate_home(&target, home.as_deref())?;
            apply(&target)?;
            configure_role_home(&parsed.argv)?;
            if protected_entry {
                acknowledge_fixture(&target)?;
            }
            return Ok(parsed.argv);
        }
        // Management commands select their own tree, and must work before its binary exists.
        if parsed.argv.get(1).is_some_and(|s| s == "dev") {
            if is_fixture() {
                let target = if parsed.argv.get(2).is_some_and(|role| role == "target") {
                    diagnostic()?
                } else {
                    contextualize(Target {
                        version: 1,
                        kind: if home.is_some() {
                            TargetKind::Isolated
                        } else {
                            TargetKind::System
                        },
                        binary: current,
                        home: home
                            .clone()
                            .unwrap_or(crate::paths::system_account_home()?.join(".askhuman")),
                        worktree: None,
                        reason: "fixture Dev management bootstrap".into(),
                        no_keychain: true,
                        fixture: None,
                    })?
                };
                validate_home(&target, home.as_deref())?;
                apply(&target)?;
                if protected_entry {
                    acknowledge_fixture(&target)?;
                }
                return Ok(parsed.argv);
            }
            if home.is_none() {
                crate::paths::system_account_home()?;
            }
            return Ok(parsed.argv);
        }
    }
    let target = if parsed.selection == Some(Selection::Dev) {
        let root = parsed
            .worktree
            .or_else(|| super::workspace::find_root(&cwd))
            .ok_or("no enabled Dev worktree; specify --worktree")?;
        dev_target(&root)?
    } else if parsed.selection == Some(Selection::System) {
        let binary = formal_binary(&current)?;
        Target {
            version: 1,
            kind: TargetKind::System,
            binary,
            home: crate::paths::system_account_home()?.join(".askhuman"),
            worktree: None,
            reason: "explicit system entry".into(),
            no_keychain: std::env::var("ASKHUMAN_NO_KEYCHAIN")
                .is_ok_and(|v| !v.is_empty() && v != "0"),
            fixture: None,
        }
    } else if let Some(root) = executable_root(&current) {
        dev_target(&root)?
    } else if let Some(home) = home.clone() {
        Target {
            version: 1,
            kind: TargetKind::Isolated,
            binary: current.clone(),
            home,
            worktree: None,
            reason: "explicit ASKHUMAN_HOME".into(),
            no_keychain: true,
            fixture: None,
        }
    } else {
        {
            if let Some(root) = crate::dev_instance::find_dev_root(&cwd).filter(|_| {
                crate::dev_instance::classify_command(&parsed.argv)
                    != crate::dev_instance::CommandClass::Skip
            }) {
                let class = crate::dev_instance::classify_command(&parsed.argv);
                let binary = crate::dev_instance::instance_bin(&root);
                if class == crate::dev_instance::CommandClass::Runtime && !binary.is_file() {
                    return Err(format!("Dev Instance enabled at {} but binary is missing: {}; run this worktree's install script", root.display(), binary.display()));
                }
                // Meta/config commands retain the legacy missing-binary fallback.
                let target = contextualize(Target {
                    version: 1,
                    kind: TargetKind::Dev,
                    binary: if binary.is_file() {
                        binary
                    } else {
                        current.clone()
                    },
                    home: crate::dev_instance::instance_home(&root),
                    worktree: Some(root),
                    reason: "legacy worktree routing".into(),
                    no_keychain: true,
                    fixture: None,
                })?;
                validate_helper_directory(&parsed.argv, &target)?;
                validate_home(&target, home.as_deref())?;
                if !state::same_path(&target.binary, &current) {
                    if protected_entry {
                        acknowledge_fixture(&target)?;
                    }
                    return reexec(&target, &parsed.argv).map(|_| parsed.argv);
                }
                apply(&target)?;
                configure_role_home(&parsed.argv)?;
                if protected_entry {
                    acknowledge_fixture(&target)?;
                }
                return Ok(parsed.argv);
            }
        }
        let binary = current.clone();
        Target {
            version: 1,
            kind: TargetKind::System,
            binary,
            home: crate::paths::system_account_home()?.join(".askhuman"),
            worktree: None,
            reason: "everyday system entry".into(),
            no_keychain: std::env::var("ASKHUMAN_NO_KEYCHAIN")
                .is_ok_and(|v| !v.is_empty() && v != "0"),
            fixture: None,
        }
    };
    let target = contextualize(target)?;
    validate_helper_directory(&parsed.argv, &target)?;
    if let Some(previous) = previous {
        if previous.kind != target.kind || !state::same_path(&previous.home, &target.home) {
            return Err("explicit target conflicts with the inherited resolved target".into());
        }
    }
    validate_home(&target, home.as_deref())?;
    if !state::same_path(&target.binary, &current) {
        if protected_entry {
            acknowledge_fixture(&target)?;
        }
        return reexec(&target, &parsed.argv).map(|_| parsed.argv);
    }
    apply(&target)?;
    configure_role_home(&parsed.argv)?;
    if protected_entry {
        acknowledge_fixture(&target)?;
    }
    Ok(parsed.argv)
}

fn helper_tick(argv: &[String]) -> bool {
    argv.get(1).is_some_and(|role| role == "todo-sync")
        && std::env::var("ASKHUMAN_TODO_SYNC_CALLER").is_ok_and(|caller| caller == "helper")
}

fn validate_helper_directory(argv: &[String], target: &Target) -> Result<(), String> {
    if helper_tick(argv) {
        if let Some(raw) = std::env::var_os("ASKHUMAN_TODO_SYNC_DIR").filter(|raw| !raw.is_empty())
        {
            let directory = Path::new(&raw);
            if !directory.is_absolute()
                || !state::same_path(directory, &target.home.join("state/todo-sync"))
            {
                return Err("sync helper sidecar conflicts with the selected AskHuman target; restart the helper with matching explicit System fixture context".into());
            }
        }
    }
    Ok(())
}

fn configure_role_home(argv: &[String]) -> Result<(), String> {
    let background = argv.get(1).is_some_and(|role| {
        matches!(
            role.as_str(),
            "--gui-host"
                | "--popup"
                | "--popup-host"
                | "__update-worker"
                | "__npm-update-worker"
                | "__dev-relay-preflight"
                | "__dev-relay"
                | "__dev-channel-identities"
                | "todo-sync"
        ) || (role == "daemon" && argv.get(2).is_some_and(|s| s == "run"))
    });
    if background {
        if let Some(target) = inherited()?.filter(|target| target.kind == TargetKind::System) {
            std::env::set_var(
                "HOME",
                target
                    .home
                    .parent()
                    .ok_or("System target has no account home")?,
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn argv(values: &[&str]) -> Vec<String> {
        values.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn selectors_precede_caller_hint_and_do_not_consume_question_options() {
        let parsed = parse_prefix(&argv(&[
            "AskHuman",
            "--caller-agent",
            "antigravity",
            "--instance=dev",
            "--worktree",
            "/work",
            "Question",
            "--instance",
            "system",
        ]))
        .unwrap();
        assert_eq!(parsed.selection, Some(Selection::Dev));
        assert_eq!(parsed.worktree, Some(PathBuf::from("/work")));
        assert_eq!(
            parsed.argv,
            argv(&[
                "AskHuman",
                "--caller-agent",
                "antigravity",
                "Question",
                "--instance",
                "system"
            ])
        );
    }

    #[test]
    fn ambiguous_or_incomplete_targets_fail_before_configuration() {
        for input in [
            argv(&["AskHuman", "--instance"]),
            argv(&["AskHuman", "--instance", "invalid"]),
            argv(&["AskHuman", "--instance", "dev", "--instance", "system"]),
            argv(&["AskHuman", "--worktree", "/work"]),
        ] {
            assert!(parse_prefix(&input).is_err(), "{input:?}");
        }
    }

    #[test]
    fn fixture_envelopes_cannot_be_downgraded_or_detached_from_their_account() {
        let root = tempfile::tempdir().unwrap();
        let account = root.path().canonicalize().unwrap();
        let target = serde_json::json!({
            "version":2, "kind":"system", "binary":account.join("AskHuman"),
            "home":account.join(".askhuman"), "worktree":null, "reason":"test",
            "noKeychain":true, "fixture":{"version":2,"accountHome":account}
        });
        assert!(decode_target(&target.to_string()).is_ok());
        let mut disguised = target.clone();
        disguised["version"] = serde_json::json!(1);
        disguised["fixture"] = serde_json::Value::Null;
        assert!(decode_target(&disguised.to_string()).is_err());
        for (field, value) in [
            ("version", serde_json::json!(1)),
            ("fixture", serde_json::Value::Null),
            ("noKeychain", serde_json::json!(false)),
            ("home", serde_json::json!(account.join("other/.askhuman"))),
        ] {
            let mut invalid = target.clone();
            invalid[field] = value;
            assert!(decode_target(&invalid.to_string()).is_err(), "{field}");
        }
    }

    #[test]
    fn legacy_system_continuation_requires_the_original_account_shape() {
        let root = tempfile::tempdir().unwrap();
        let root = root.path().canonicalize().unwrap();
        let mut target = serde_json::json!({"version":1,"kind":"system","binary":root.join("AskHuman"),
            "home":root.join(".askhuman"),"worktree":null,"reason":"old System","noKeychain":false});
        assert!(decode_target(&target.to_string()).is_ok());
        target["home"] = serde_json::json!(root.join("arbitrary"));
        assert!(decode_target(&target.to_string()).is_err());
    }

    #[test]
    fn explicit_system_never_accepts_an_external_home_even_if_it_names_system_data() {
        let target = Target {
            version: 1,
            kind: TargetKind::System,
            binary: PathBuf::from("/bin/AskHuman"),
            home: PathBuf::from("/home/user/.askhuman"),
            worktree: None,
            reason: String::new(),
            no_keychain: false,
            fixture: None,
        };
        assert!(validate_home(&target, Some(&target.home)).is_err());
        assert!(validate_home(&target, None).is_ok());
    }
}
