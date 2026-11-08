//! The setup wizard's silent environment check (spec D12 / §3.4): is `AskHuman` reachable from a
//! shell, does the running binary sit somewhere that will not survive, and are there several
//! builds fighting over one daemon? It stays quiet when everything is fine; the wizard only shows
//! red cards for what is found.

use crate::cli::{cfgio, install_check};
use crate::i18n::Lang;
use crate::integrations::agent_launch::{self, EnvLookup};
use serde::Serialize;
use std::path::{Path, PathBuf};

/// The command a bare `AskHuman` is looked up by.
const COMMAND: &str = "AskHuman";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Os {
    Unix,
    Windows,
}

impl Os {
    pub fn current() -> Self {
        if cfg!(windows) {
            Os::Windows
        } else {
            Os::Unix
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransientKind {
    /// A temporary directory.
    Temp,
    /// The user's Downloads folder.
    Downloads,
    /// npm's `npx` cache.
    Npx,
}

impl TransientKind {
    fn as_str(self) -> &'static str {
        match self {
            TransientKind::Temp => "temp",
            TransientKind::Downloads => "downloads",
            TransientKind::Npx => "npx",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PathFix {
    /// A one-line shell command that makes `AskHuman` reachable.
    pub command: String,
    /// The directory the command links into (or appends to PATH on Windows).
    pub dir: String,
    /// The directory is not on PATH yet: the user also has to add it in their shell profile.
    pub needs_path_edit: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Transient {
    pub path: String,
    /// `temp` | `downloads` | `npx`.
    pub kind: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Installs {
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EnvReport {
    /// Whether the login shell finds `AskHuman`: `found` | `missing` | `unknown`.
    pub askhuman_on_path: String,
    /// Present when `AskHuman` is missing from PATH and the running binary is somewhere stable.
    pub path_fix: Option<PathFix>,
    /// Present when the running binary sits in a temporary location.
    pub transient: Option<Transient>,
    /// Present when `doctor install` has something to say about several builds.
    pub installs: Option<Installs>,
}

impl EnvReport {
    /// What a Dev Instance reports: nothing to say.
    pub fn empty() -> Self {
        Self {
            askhuman_on_path: "unknown".to_string(),
            path_fix: None,
            transient: None,
            installs: None,
        }
    }
}

/// Whether `path` lies in a place that will not stay: a temp directory, Downloads or npm's
/// `npx` cache. Pure: the real directories are passed in.
pub fn is_transient_location(path: &Path, home: &Path, temp_dir: &Path) -> Option<TransientKind> {
    if path.components().any(|part| part.as_os_str() == "_npx") {
        return Some(TransientKind::Npx);
    }
    let mut temp_roots: Vec<PathBuf> = vec![temp_dir.to_path_buf()];
    if cfg!(unix) {
        for fixed in [
            "/tmp",
            "/private/tmp",
            "/var/folders",
            "/private/var/folders",
        ] {
            temp_roots.push(PathBuf::from(fixed));
        }
    }
    if temp_roots
        .iter()
        .any(|root| !root.as_os_str().is_empty() && path.starts_with(root))
    {
        return Some(TransientKind::Temp);
    }
    if path.starts_with(home.join("Downloads")) {
        return Some(TransientKind::Downloads);
    }
    None
}

/// Quote for a POSIX shell: wrap in single quotes, escaping embedded ones.
fn sh_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

/// Quote for a PowerShell single-quoted string literal.
fn ps_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

/// The one-line fix that makes `AskHuman` reachable from a shell.
///
/// Unix: link the binary into a directory that is already on the login shell's PATH, preferring
/// one under the user's home and writable, then any writable one; with none, link into
/// `~/.local/bin` and tell the user to put it on PATH. Windows: append the binary's directory to
/// the user-level `Path`.
pub fn fix_command(
    exe: &Path,
    login_path_dirs: &[PathBuf],
    home: &Path,
    is_writable: &dyn Fn(&Path) -> bool,
    os: Os,
) -> PathFix {
    let exe_text = exe.display().to_string();
    match os {
        Os::Windows => {
            let dir = exe
                .parent()
                .map(|p| p.display().to_string())
                .unwrap_or_default();
            PathFix {
                command: format!(
                    "[Environment]::SetEnvironmentVariable('Path', \
                     [Environment]::GetEnvironmentVariable('Path', 'User') + ';' + {}, 'User')",
                    ps_quote(&dir)
                ),
                dir,
                needs_path_edit: false,
            }
        }
        Os::Unix => {
            let usable = |dir: &&PathBuf| !dir.as_os_str().is_empty() && is_writable(dir);
            let chosen = login_path_dirs
                .iter()
                .filter(usable)
                .find(|dir| dir.starts_with(home))
                .or_else(|| login_path_dirs.iter().find(usable));
            match chosen {
                Some(dir) => PathFix {
                    command: format!(
                        "ln -sf {} {}",
                        sh_quote(&exe_text),
                        sh_quote(&dir.join(COMMAND).display().to_string())
                    ),
                    dir: dir.display().to_string(),
                    needs_path_edit: false,
                },
                None => PathFix {
                    command: format!(
                        "mkdir -p ~/.local/bin && ln -sf {} ~/.local/bin/{COMMAND}",
                        sh_quote(&exe_text)
                    ),
                    dir: "~/.local/bin".to_string(),
                    needs_path_edit: true,
                },
            }
        }
    }
}

/// Everything the report is assembled from.
pub struct Inputs<'a> {
    pub lookup: EnvLookup,
    pub login_path_dirs: Vec<PathBuf>,
    pub exe: Option<&'a Path>,
    pub home: &'a Path,
    pub temp_dir: &'a Path,
    pub is_writable: &'a dyn Fn(&Path) -> bool,
    pub os: Os,
    /// The finished "several builds" text, when there is one.
    pub installs_message: Option<String>,
}

/// Assemble the report from what was read. Pure.
pub fn build_report(inputs: Inputs<'_>) -> EnvReport {
    let askhuman_on_path = match &inputs.lookup {
        EnvLookup::Found(_) => "found",
        EnvLookup::Missing => "missing",
        EnvLookup::Unknown(_) => "unknown",
    };
    let transient_kind = inputs
        .exe
        .and_then(|exe| is_transient_location(exe, inputs.home, inputs.temp_dir));
    let transient = inputs.exe.zip(transient_kind).map(|(exe, kind)| Transient {
        path: exe.display().to_string(),
        kind: kind.as_str().to_string(),
    });
    // Never offer to link a temporary location into PATH.
    let path_fix = match (&inputs.lookup, inputs.exe, transient_kind) {
        (EnvLookup::Missing, Some(exe), None) => Some(fix_command(
            exe,
            &inputs.login_path_dirs,
            inputs.home,
            inputs.is_writable,
            inputs.os,
        )),
        _ => None,
    };
    EnvReport {
        askhuman_on_path: askhuman_on_path.to_string(),
        path_fix,
        transient,
        installs: inputs.installs_message.map(|message| Installs { message }),
    }
}

#[cfg(unix)]
fn dir_is_writable(dir: &Path) -> bool {
    use std::os::unix::ffi::OsStrExt;
    let Ok(path) = std::ffi::CString::new(dir.as_os_str().as_bytes()) else {
        return false;
    };
    dir.is_dir() && unsafe { libc::access(path.as_ptr(), libc::W_OK | libc::X_OK) == 0 }
}

#[cfg(not(unix))]
fn dir_is_writable(dir: &Path) -> bool {
    std::fs::metadata(dir).is_ok_and(|meta| meta.is_dir() && !meta.permissions().readonly())
}

/// The "several builds" text: the first finding of `doctor install`, a pointer to the command, and
/// how many more there are.
fn installs_message(report: &install_check::Report, lang: Lang) -> Option<String> {
    let first = report.findings.first()?;
    let mut message = first.message(lang, report.now_ms);
    message.push(' ');
    message.push_str(&cfgio::t(
        lang,
        "Details: AskHuman doctor install",
        "详见 AskHuman doctor install",
    ));
    let more = report.findings.len() - 1;
    if more > 0 {
        message.push_str(&cfgio::t(
            lang,
            &format!(" ({more} more)"),
            &format!("（另有 {more} 项）"),
        ));
    }
    Some(message)
}

/// Run the real check. A Dev Instance reports nothing: its binary is not meant to win the PATH and
/// its daemon is isolated.
pub fn check(lang: Lang) -> EnvReport {
    if crate::dev_instance::is_dev_instance_context() {
        return EnvReport::empty();
    }
    let (lookup, login_path) = agent_launch::lookup_with_login_path(COMMAND);
    let login_path_dirs: Vec<PathBuf> = std::env::split_paths(&login_path)
        .filter(|dir| !dir.as_os_str().is_empty())
        .collect();
    // Scan the login shell's PATH when it is known: a GUI process inherits a sparse one.
    let scan_dirs = (!login_path_dirs.is_empty()).then(|| login_path_dirs.clone());
    let status = cfgio::daemon_status_result();
    let report =
        install_check::collect_with_path(status.as_ref().ok().and_then(Option::as_ref), scan_dirs);
    let exe = std::env::current_exe().ok();
    let home = crate::paths::home();
    let temp_dir = std::env::temp_dir();
    build_report(Inputs {
        lookup,
        login_path_dirs,
        exe: exe.as_deref(),
        home: &home,
        temp_dir: &temp_dir,
        is_writable: &dir_is_writable,
        os: Os::current(),
        installs_message: installs_message(&report, lang),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(text: &str) -> PathBuf {
        PathBuf::from(text)
    }

    const HOME: &str = "/Users/me";

    #[cfg(unix)]
    #[test]
    fn transient_locations_are_recognized() {
        let home = p(HOME);
        let temp = p("/var/folders/ab/xyz/T");
        let kind = |path: &str| is_transient_location(&p(path), &home, &temp);
        assert_eq!(kind("/tmp/AskHuman"), Some(TransientKind::Temp));
        assert_eq!(kind("/private/tmp/x/AskHuman"), Some(TransientKind::Temp));
        assert_eq!(
            kind("/private/var/folders/ab/xyz/T/AskHuman"),
            Some(TransientKind::Temp)
        );
        assert_eq!(
            kind("/var/folders/ab/xyz/T/a/AskHuman"),
            Some(TransientKind::Temp)
        );
        assert_eq!(
            kind("/Users/me/Downloads/AskHuman-macos/AskHuman"),
            Some(TransientKind::Downloads)
        );
        assert_eq!(
            kind("/Users/me/.npm/_npx/1a2b/node_modules/@humaninloop/darwin/AskHuman"),
            Some(TransientKind::Npx)
        );
    }

    #[cfg(unix)]
    #[test]
    fn stable_locations_are_not_transient() {
        let home = p(HOME);
        let temp = p("/var/folders/ab/xyz/T");
        for path in [
            "/usr/local/bin/AskHuman",
            "/Users/me/.local/bin/AskHuman",
            "/Users/me/Developer/HumanInLoop/src-tauri/target/release/AskHuman",
            "/opt/homebrew/lib/node_modules/@humaninloop/darwin-arm64/AskHuman",
            "/Users/me/DownloadsArchive/AskHuman",
        ] {
            assert_eq!(
                is_transient_location(&p(path), &home, &temp),
                None,
                "{path}"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn fix_prefers_a_writable_dir_under_home() {
        let dirs = vec![
            p("/usr/local/bin"),
            p("/Users/me/bin"),
            p("/Users/me/.local/bin"),
        ];
        let writable = |dir: &Path| dir != Path::new("/Users/me/.local/bin");
        let fix = fix_command(
            &p("/Applications/AskHuman/AskHuman"),
            &dirs,
            &p(HOME),
            &writable,
            Os::Unix,
        );
        assert_eq!(fix.dir, "/Users/me/bin");
        assert_eq!(
            fix.command,
            "ln -sf '/Applications/AskHuman/AskHuman' '/Users/me/bin/AskHuman'"
        );
        assert!(!fix.needs_path_edit);
    }

    #[test]
    fn fix_falls_back_to_any_writable_dir() {
        let dirs = vec![p("/usr/bin"), p("/usr/local/bin")];
        let writable = |dir: &Path| dir == Path::new("/usr/local/bin");
        let fix = fix_command(&p("/opt/AskHuman"), &dirs, &p(HOME), &writable, Os::Unix);
        assert_eq!(fix.dir, "/usr/local/bin");
        assert!(!fix.needs_path_edit);
    }

    #[test]
    fn fix_without_any_writable_dir_links_into_local_bin_and_asks_for_a_path_edit() {
        let writable = |_: &Path| false;
        let fix = fix_command(
            &p("/opt/AskHuman"),
            &[p("/usr/bin")],
            &p(HOME),
            &writable,
            Os::Unix,
        );
        assert_eq!(
            fix.command,
            "mkdir -p ~/.local/bin && ln -sf '/opt/AskHuman' ~/.local/bin/AskHuman"
        );
        assert_eq!(fix.dir, "~/.local/bin");
        assert!(fix.needs_path_edit);
        // An empty PATH behaves the same.
        let fix = fix_command(&p("/opt/AskHuman"), &[], &p(HOME), &writable, Os::Unix);
        assert!(fix.needs_path_edit);
    }

    #[cfg(unix)]
    #[test]
    fn fix_quotes_awkward_paths() {
        let writable = |_: &Path| true;
        let fix = fix_command(
            &p("/Users/me/it's here/AskHuman"),
            &[p("/Users/me/bin")],
            &p(HOME),
            &writable,
            Os::Unix,
        );
        assert_eq!(
            fix.command,
            "ln -sf '/Users/me/it'\\''s here/AskHuman' '/Users/me/bin/AskHuman'"
        );
    }

    #[test]
    fn windows_fix_appends_the_binary_directory_to_the_user_path() {
        let writable = |_: &Path| true;
        let fix = fix_command(
            &p("C:/Tools/it's/AskHuman.exe"),
            &[],
            &p("C:/Users/me"),
            &writable,
            Os::Windows,
        );
        assert!(fix
            .command
            .starts_with("[Environment]::SetEnvironmentVariable('Path'"));
        assert!(fix.command.contains("'C:/Tools/it''s'"), "{}", fix.command);
        assert!(!fix.needs_path_edit);
    }

    fn inputs<'a>(
        lookup: EnvLookup,
        exe: Option<&'a Path>,
        home: &'a Path,
        temp: &'a Path,
        writable: &'a dyn Fn(&Path) -> bool,
        installs: Option<&str>,
    ) -> Inputs<'a> {
        Inputs {
            lookup,
            login_path_dirs: vec![p("/Users/me/bin")],
            exe,
            home,
            temp_dir: temp,
            is_writable: writable,
            os: Os::Unix,
            installs_message: installs.map(str::to_string),
        }
    }

    #[test]
    fn a_healthy_machine_reports_nothing() {
        let (home, temp) = (p(HOME), p("/tmp"));
        let writable = |_: &Path| true;
        let exe = p("/usr/local/bin/AskHuman");
        let report = build_report(inputs(
            EnvLookup::Found("/usr/local/bin/AskHuman".into()),
            Some(&exe),
            &home,
            &temp,
            &writable,
            None,
        ));
        assert_eq!(report.askhuman_on_path, "found");
        assert!(
            report.path_fix.is_none() && report.transient.is_none() && report.installs.is_none()
        );
    }

    #[test]
    fn a_missing_command_in_a_stable_place_gets_a_fix() {
        let (home, temp) = (p(HOME), p("/tmp"));
        let writable = |_: &Path| true;
        let exe = p("/Applications/AskHuman/AskHuman");
        let report = build_report(inputs(
            EnvLookup::Missing,
            Some(&exe),
            &home,
            &temp,
            &writable,
            None,
        ));
        assert_eq!(report.askhuman_on_path, "missing");
        assert!(report.path_fix.is_some());
        assert!(report.transient.is_none());
    }

    #[cfg(unix)]
    #[test]
    fn a_transient_binary_gets_the_location_card_and_never_a_path_fix() {
        let (home, temp) = (p(HOME), p("/tmp"));
        let writable = |_: &Path| true;
        let exe = p("/Users/me/Downloads/AskHuman");
        let report = build_report(inputs(
            EnvLookup::Missing,
            Some(&exe),
            &home,
            &temp,
            &writable,
            None,
        ));
        assert!(report.path_fix.is_none());
        let transient = report.transient.expect("transient card");
        assert_eq!(transient.kind, "downloads");
        assert_eq!(transient.path, "/Users/me/Downloads/AskHuman");
    }

    #[test]
    fn an_unknown_lookup_never_produces_a_fix() {
        let (home, temp) = (p(HOME), p("/tmp"));
        let writable = |_: &Path| true;
        let exe = p("/Applications/AskHuman/AskHuman");
        let report = build_report(inputs(
            EnvLookup::Unknown("timeout".into()),
            Some(&exe),
            &home,
            &temp,
            &writable,
            None,
        ));
        assert_eq!(report.askhuman_on_path, "unknown");
        assert!(report.path_fix.is_none());
    }

    #[test]
    fn several_builds_are_reported_independently_of_the_path_lookup() {
        let (home, temp) = (p(HOME), p("/tmp"));
        let writable = |_: &Path| true;
        let exe = p("/Applications/AskHuman/AskHuman");
        let report = build_report(inputs(
            EnvLookup::Found("/usr/local/bin/AskHuman".into()),
            Some(&exe),
            &home,
            &temp,
            &writable,
            Some("2 different builds"),
        ));
        assert_eq!(
            report.installs,
            Some(Installs {
                message: "2 different builds".to_string()
            })
        );
    }

    #[test]
    fn the_report_serializes_in_camel_case() {
        let json = serde_json::to_value(EnvReport::empty()).unwrap();
        assert_eq!(json["askhumanOnPath"], "unknown");
        assert!(json["pathFix"].is_null());
        assert!(json["transient"].is_null());
        assert!(json["installs"].is_null());
        let fix = serde_json::to_value(PathFix {
            command: "c".into(),
            dir: "d".into(),
            needs_path_edit: true,
        })
        .unwrap();
        assert_eq!(fix["needsPathEdit"], true);
    }
}
