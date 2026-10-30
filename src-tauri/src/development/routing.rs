//! Resolve one process target before configuration, secrets, or a graphical runtime is loaded.

use super::state::{self, RoutingMode};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

pub const TARGET_ENV: &str = "ASKHUMAN_RESOLVED_TARGET_V1";
pub const SYSTEM_BINDINGS_ENV: &str = "ASKHUMAN_WRITE_SYSTEM_BINDINGS";

pub fn system_bindings() -> bool {
    inherited()
        .ok()
        .flatten()
        .is_some_and(|target| target.kind == TargetKind::System)
        && std::env::var_os(super::session::PROFILE_ENV).is_none()
        && (std::env::var(SYSTEM_BINDINGS_ENV).is_ok_and(|value| value == "1")
            || state::read().is_ok_and(|state| state.mode == RoutingMode::Explicit))
}

pub fn integration_prefix(executable: &str) -> String {
    if system_bindings() {
        format!(
            "'{}' --instance system",
            executable.replace('\'', "'\"'\"'")
        )
    } else {
        format!("\"{executable}\"")
    }
}

pub fn integration_args(args: &[&str]) -> Vec<String> {
    let mut result = Vec::new();
    if system_bindings() {
        result.extend(["--instance".into(), "system".into()]);
    }
    result.extend(args.iter().map(|value| value.to_string()));
    result
}

/// Integration artifacts must retain the selected installation entry, even when the running
/// image is reached through a symlink or retained by the Linux publisher.
pub fn program_path() -> std::io::Result<PathBuf> {
    match inherited().map_err(std::io::Error::other)? {
        Some(target) => Ok(target.binary),
        None => crate::executable::launch_path(),
    }
}

fn registered_system_binary(fallback: &Path) -> Result<PathBuf, String> {
    let binary = state::read()?
        .system_binary
        .unwrap_or_else(|| fallback.to_path_buf());
    if executable_root(&binary).is_some() {
        return Err("registered system entry points into a Dev Instance".into());
    }
    if !binary.is_file() {
        return Err("registered system entry is missing; reinstall the system AskHuman".into());
    }
    Ok(binary)
}

fn system_entry(current: &Path, configured: &state::State) -> Result<PathBuf, String> {
    if configured.mode == RoutingMode::Explicit {
        return registered_system_binary(current);
    }
    // Preserve legacy binary choice while retaining a registered alias for the same image.
    Ok(configured
        .system_binary
        .as_ref()
        .filter(|entry| state::same_path(entry, current))
        .cloned()
        .unwrap_or_else(|| current.to_path_buf()))
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
        Ok(raw) => {
            let target: Target = serde_json::from_str(&raw)
                .map_err(|e| format!("invalid inherited AskHuman target: {e}"))?;
            if target.version != 1 || !target.binary.is_absolute() || !target.home.is_absolute() {
                return Err(
                    "inherited AskHuman target has an unsupported version or relative path".into(),
                );
            }
            Ok(Some(target))
        }
        Err(std::env::VarError::NotPresent) => Ok(None),
        Err(_) => Err("inherited AskHuman target is not valid Unicode".into()),
    }
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
    if !root
        .join(crate::dev_instance::DEV_DIR)
        .join(crate::dev_instance::ENABLED_MARKER)
        .is_file()
    {
        return Err(format!("Dev Instance is not enabled at {}", root.display()));
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
        || state::same_path(&home, &state::system_home())
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
    Ok(Target {
        version: 1,
        kind: TargetKind::Dev,
        binary,
        home,
        worktree: Some(root),
        reason: "explicit Dev verification".into(),
        no_keychain: true,
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

fn apply(target: &Target) -> Result<(), String> {
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
    let target = Target {
        version: 1,
        kind: TargetKind::Isolated,
        binary: crate::executable::launch_path().map_err(|e| e.to_string())?,
        home,
        worktree: Some(root),
        reason: "explicit Dev data-management operation".into(),
        no_keychain: true,
    };
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
    if let Some(target) = inherited()? {
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
        return Ok(Target {
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
        });
    }
    if state::read()?.mode == RoutingMode::Legacy {
        if let Some(root) = crate::dev_instance::find_dev_root(&cwd) {
            return dev_target(&root);
        }
    }
    Ok(Target {
        version: 1,
        kind: TargetKind::System,
        binary: system_entry(&current, &state::read()?)?,
        home: state::system_home(),
        worktree: None,
        reason: "everyday system entry".into(),
        no_keychain: false,
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
    let mut child = Command::new(binary)
        .arg("__dev-capabilities")
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
    if !output.status.success() || value["developmentVerification"].as_u64() != Some(1) {
        return Err(format!(
            "{} does not support isolated Dev verification; install a compatible build",
            binary.display()
        ));
    }
    Ok(())
}

pub fn prepare(argv: Vec<String>) -> Result<Vec<String>, String> {
    // Probing an older executable must not recurse through any worktree dispatcher.
    if argv.get(1).is_some_and(|s| s == "__dev-capabilities") {
        return Ok(argv);
    }
    let parsed = parse_prefix(&argv)?;
    let current = crate::executable::launch_path().map_err(|e| e.to_string())?;
    let cwd = std::env::current_dir().map_err(|e| e.to_string())?;
    let home = std::env::var_os(crate::dev_instance::ASKHUMAN_HOME_ENV)
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .map(|p| if p.is_absolute() { p } else { cwd.join(p) });
    let previous = inherited()?;
    if parsed.selection.is_none() {
        if let Some(target) = previous {
            let worker = parsed.argv.get(1).is_some_and(|role| {
                matches!(role.as_str(), "__update-worker" | "__npm-update-worker")
            });
            if executable_root(&current).is_some() && target.kind == TargetKind::System {
                return Err("a Dev executable cannot inherit the system instance; use the registered system entry".into());
            }
            if !worker && !state::same_path(&current, &target.binary) {
                return reexec(&target, &parsed.argv).map(|_| parsed.argv);
            }
            validate_home(&target, home.as_deref())?;
            apply(&target)?;
            return Ok(parsed.argv);
        }
        // Management commands select their own tree, and must work before its binary exists.
        if parsed.argv.get(1).is_some_and(|s| s == "dev") {
            return Ok(parsed.argv);
        }
    }
    let target = if parsed.selection == Some(Selection::Dev) {
        let root = parsed
            .worktree
            .or_else(|| crate::dev_instance::find_dev_root(&cwd))
            .ok_or("no enabled Dev worktree; specify --worktree")?;
        dev_target(&root)?
    } else if parsed.selection == Some(Selection::System) {
        let binary = registered_system_binary(&current)?;
        Target {
            version: 1,
            kind: TargetKind::System,
            binary,
            home: state::system_home(),
            worktree: None,
            reason: "explicit system entry".into(),
            no_keychain: std::env::var("ASKHUMAN_NO_KEYCHAIN")
                .is_ok_and(|v| !v.is_empty() && v != "0"),
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
        }
    } else {
        if state::read()?.mode == RoutingMode::Legacy {
            crate::dev_instance::maybe_enter_dev_instance();
            if let Some(root) = crate::dev_instance::find_dev_root(&cwd).filter(|_| {
                crate::dev_instance::classify_command(&parsed.argv)
                    != crate::dev_instance::CommandClass::Skip
            }) {
                // Meta/config commands retain the legacy missing-binary fallback.
                let target = Target {
                    version: 1,
                    kind: TargetKind::Dev,
                    binary: current.clone(),
                    home: crate::dev_instance::instance_home(&root),
                    worktree: Some(root),
                    reason: "legacy worktree routing".into(),
                    no_keychain: true,
                };
                apply(&target)?;
                return Ok(parsed.argv);
            }
        }
        let binary = system_entry(&current, &state::read()?)?;
        Target {
            version: 1,
            kind: TargetKind::System,
            binary,
            home: state::system_home(),
            worktree: None,
            reason: "everyday system entry".into(),
            no_keychain: std::env::var("ASKHUMAN_NO_KEYCHAIN")
                .is_ok_and(|v| !v.is_empty() && v != "0"),
        }
    };
    if let Some(previous) = previous {
        if previous.kind != target.kind || !state::same_path(&previous.home, &target.home) {
            return Err("explicit target conflicts with the inherited resolved target".into());
        }
    }
    validate_home(&target, home.as_deref())?;
    if !state::same_path(&target.binary, &current) {
        return reexec(&target, &parsed.argv).map(|_| parsed.argv);
    }
    apply(&target)?;
    Ok(parsed.argv)
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
    fn explicit_system_never_accepts_an_external_home_even_if_it_names_system_data() {
        let target = Target {
            version: 1,
            kind: TargetKind::System,
            binary: PathBuf::from("/bin/AskHuman"),
            home: PathBuf::from("/home/user/.askhuman"),
            worktree: None,
            reason: String::new(),
            no_keychain: false,
        };
        assert!(validate_home(&target, Some(&target.home)).is_err());
        assert!(validate_home(&target, None).is_ok());
    }
}
