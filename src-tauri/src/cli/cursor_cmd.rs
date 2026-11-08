use anyhow::{bail, Result};
use std::ffi::OsString;

const USAGE: &str = "Usage: AskHuman cursor [-- <Cursor arguments>]\n       AskHuman cursor shortcut <name>\n\nLaunch the macOS Cursor CLI with experimental session protection.\nThe shortcut is written to ~/.local/bin and forwards all arguments.";

pub fn run(args: Vec<OsString>) -> Result<i32> {
    if matches!(
        args.first().and_then(|arg| arg.to_str()),
        Some("--help" | "-h")
    ) {
        println!("{USAGE}");
        return Ok(0);
    }
    #[cfg(target_os = "macos")]
    return macos::run(args);
    #[cfg(not(target_os = "macos"))]
    bail!("Cursor CLI session protection is currently available on macOS");
}

#[cfg(target_os = "macos")]
mod macos {
    use super::*;
    use anyhow::Context;
    use sha2::{Digest, Sha256};
    use std::{
        env, fs,
        io::{self, IsTerminal, Write},
        os::unix::{
            fs::{DirBuilderExt, MetadataExt, PermissionsExt},
            process::CommandExt,
        },
        path::{Path, PathBuf},
        process::Command,
    };

    const PRELOAD: &[u8] = include_bytes!("cursor_preload.cjs");
    const BOOT_ENV: &str = "ASKHUMAN_CURSOR_CLI_BOOT";

    pub(super) fn run(mut args: Vec<OsString>) -> Result<i32> {
        if args.first().is_some_and(|arg| arg == "shortcut") {
            let name = match args.as_slice() {
                [_, name] => name.to_str().context("shortcut name must be UTF-8")?,
                _ => bail!("Usage: AskHuman cursor shortcut <name>"),
            };
            let home = dirs::home_dir().context("cannot locate the home directory")?;
            let executable = crate::executable::launch_path()?;
            let path = install_shortcut(
                name,
                &home.join(".local/bin"),
                &executable,
                env::var_os("PATH"),
            )?;
            println!("{}", path.display());
            return Ok(0);
        }
        if args.first().is_some_and(|arg| arg == "--") {
            args.remove(0);
        }

        use crate::integrations::agent_launch::{lookup_with_login_path, EnvLookup};
        let (found, _) = lookup_with_login_path("cursor-agent");
        let executable = match found {
            EnvLookup::Found(path) => PathBuf::from(path),
            EnvLookup::Missing => bail!("cursor-agent was not found in the login shell's PATH"),
            EnvLookup::Unknown(reason) => bail!("could not find cursor-agent: {reason}"),
        };
        let mut command = Command::new(&executable);
        command.args(&args).env_remove(BOOT_ENV);
        if io::stdin().is_terminal() && io::stdout().is_terminal() && interactive(&args) {
            protect(&mut command, &executable)
                .context("Cursor CLI Max resume protection stopped Cursor")?;
        }
        Err(command.exec()).context("could not start cursor-agent")
    }

    fn interactive(args: &[OsString]) -> bool {
        let mut consumes_value = false;
        let mut first_positional = true;
        for arg in args {
            let Some(arg) = arg.to_str() else {
                return false;
            };
            if consumes_value {
                consumes_value = false;
                continue;
            }
            if arg == "--" {
                break;
            }
            if arg.starts_with('-') {
                let (option, inline_value) = arg.split_once('=').unwrap_or((arg, ""));
                match option {
                    "--model" | "-m" | "--workspace" | "--directory" | "--mode" | "--api-key"
                    | "--add-dir" | "--sandbox" | "--header" | "-H" | "--plugin-dir"
                    | "--worktree-base" => consumes_value = inline_value.is_empty(),
                    "--print" | "-p" | "--acp" | "--version" | "-v" | "--help" | "-h"
                    | "--list-models" => return false,
                    _ => {}
                }
            } else if first_positional {
                first_positional = false;
                if matches!(
                    arg,
                    "acp"
                        | "about"
                        | "bedrock"
                        | "create-chat"
                        | "generate-rule"
                        | "help"
                        | "install-shell-integration"
                        | "login"
                        | "logout"
                        | "ls"
                        | "mcp"
                        | "models"
                        | "persist"
                        | "plugin"
                        | "rule"
                        | "status"
                        | "team"
                        | "uninstall-shell-integration"
                        | "update"
                        | "whoami"
                        | "worker"
                ) {
                    return false;
                }
            }
        }
        !consumes_value
    }

    fn protect(command: &mut Command, executable: &Path) -> Result<()> {
        let package = executable.parent().context("invalid Cursor CLI path")?;
        if executable
            .file_name()
            .is_none_or(|name| name != "cursor-agent")
            || package
                .parent()
                .and_then(Path::file_name)
                .is_none_or(|name| name != "versions")
            || fs::metadata(executable)?.len() > 16384
            || !uses_bundled_node(&fs::read_to_string(executable)?)
        {
            bail!("Cursor launcher does not use the supported bundled Node entry point");
        }
        let preload = install_preload()?;
        let preload = preload.to_str().context("preload path must be UTF-8")?;
        if preload.contains(['"', '\\', '\r', '\n']) {
            bail!("preload path cannot be represented safely in NODE_OPTIONS");
        }
        let original_options = env::var_os("NODE_OPTIONS")
            .map(|options| {
                options
                    .into_string()
                    .map_err(|_| anyhow::anyhow!("NODE_OPTIONS must be UTF-8"))
            })
            .transpose()?;
        let options = format!(
            "{} --require \"{preload}\"",
            original_options.as_deref().unwrap_or("")
        );
        let boot = serde_json::json!({ "package": package, "originalOptions": original_options });
        command
            .env("NODE_OPTIONS", options)
            .env(BOOT_ENV, boot.to_string());
        Ok(())
    }

    fn uses_bundled_node(launcher: &str) -> bool {
        let mut lines = launcher.lines().map(str::trim);
        if lines.next() != Some("#!/usr/bin/env bash") {
            return false;
        }
        let mut bundled_node = false;
        let mut starts_cli = false;
        for line in lines {
            match line {
                r#"NODE_BIN="$SCRIPT_DIR/node""# => bundled_node = true,
                r#"exec -a "$0" "$NODE_BIN" --use-system-ca "$SCRIPT_DIR/index.js" "$@""#
                | r#"exec -a "$0" "$NODE_BIN" "$SCRIPT_DIR/index.js" "$@""# => starts_cli = true,
                line if line.starts_with("exec ") || line.contains("NODE_OPTIONS") => return false,
                _ => {}
            }
        }
        bundled_node && starts_cli
    }

    fn install_preload() -> Result<PathBuf> {
        let directory = crate::paths::state_dir().join("cursor-cli");
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&directory)?;
        let metadata = fs::symlink_metadata(&directory)?;
        let uid = unsafe { libc::geteuid() };
        if !metadata.is_dir() || metadata.uid() != uid || metadata.mode() & 0o077 != 0 {
            bail!("Cursor preload directory must be private and owned by the current user");
        }
        let path = directory.join(format!("preload-{:x}.cjs", Sha256::digest(PRELOAD)));
        if !path.exists() {
            match publish(&path, PRELOAD, 0o600) {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(error.into()),
            }
        }
        let metadata = fs::symlink_metadata(&path)?;
        if !metadata.is_file()
            || metadata.uid() != uid
            || metadata.mode() & 0o077 != 0
            || metadata.len() != PRELOAD.len() as u64
            || fs::read(&path)? != PRELOAD
        {
            bail!("Cursor preload has unexpected ownership or content");
        }
        Ok(path)
    }

    fn install_shortcut(
        name: &str,
        bin: &Path,
        executable: &Path,
        path_env: Option<OsString>,
    ) -> Result<PathBuf> {
        if name.is_empty()
            || name.len() > 64
            || !name.as_bytes()[0].is_ascii_alphanumeric()
            || !name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte))
            || ["agent", "cursor-agent", "askhuman"]
                .iter()
                .any(|reserved| name.eq_ignore_ascii_case(reserved))
        {
            bail!("choose a new command name using letters, digits, dots, dashes or underscores");
        }
        let destination = bin.join(name);
        if let Some(path) = path_env {
            for directory in env::split_paths(&path) {
                let candidate = directory.join(name);
                if fs::symlink_metadata(&candidate).is_ok() {
                    bail!("shortcut name is already used by {}", candidate.display());
                }
            }
        }
        let target = executable
            .to_str()
            .context("AskHuman executable path must be UTF-8")?;
        let target = target.replace('\'', "'\\''");
        let script = format!("#!/bin/sh\nexec '{target}' cursor -- \"$@\"\n");
        fs::create_dir_all(bin)?;
        publish(&destination, script.as_bytes(), 0o700)
            .with_context(|| format!("could not create {}", destination.display()))?;
        Ok(destination)
    }

    fn publish(path: &Path, contents: &[u8], mode: u32) -> io::Result<()> {
        let mut file =
            tempfile::NamedTempFile::new_in(path.parent().expect("output file has a parent"))?;
        file.write_all(contents)?;
        file.as_file()
            .set_permissions(fs::Permissions::from_mode(mode))?;
        file.persist_noclobber(path).map_err(|error| error.error)?;
        Ok(())
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn shortcut_preserves_arguments_and_refuses_to_overwrite() {
            let root = tempfile::tempdir().unwrap();
            let executable = root.path().join("Ask Human's binary");
            publish(
                &executable,
                b"#!/bin/sh\nprintf '<%s>\\n' \"$@\"\nexit 17\n",
                0o700,
            )
            .unwrap();
            let shortcut = install_shortcut("cursor2", root.path(), &executable, None).unwrap();
            let output = Command::new(&shortcut)
                .args(["--resume", "hello world", "$(false)", ""])
                .output()
                .unwrap();
            assert_eq!(output.status.code(), Some(17));
            assert_eq!(
                String::from_utf8(output.stdout).unwrap(),
                "<cursor>\n<-->\n<--resume>\n<hello world>\n<$(false)>\n<>\n"
            );
            assert!(install_shortcut("cursor2", root.path(), &executable, None).is_err());
            assert!(install_shortcut("../cursor2", root.path(), &executable, None).is_err());
            assert!(install_shortcut("Cursor-Agent", root.path(), &executable, None).is_err());
        }

        #[test]
        fn native_modes_are_outside_protection() {
            let classify =
                |args: &[&str]| interactive(&args.iter().map(OsString::from).collect::<Vec<_>>());
            assert!(classify(&[]));
            assert!(classify(&["--resume", "chat-id", "Continue"]));
            assert!(classify(&["--model", "example", "Use the --print option"]));
            assert!(classify(&["--resume", "id", "--unknown"]));
            for args in [
                &["--print"][..],
                &["--model", "m", "--acp"],
                &["--resume", "id", "--help"],
                &["update"],
                &["worker"],
            ] {
                assert!(!classify(args));
            }
        }
    }
}
