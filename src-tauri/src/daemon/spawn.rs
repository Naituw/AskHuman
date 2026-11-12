//! 后台拉起 Daemon：macOS 优先交给当前用户的 GUI launchd domain 管理，其它 Unix 则 detach
//! （新会话）并把 stdio 重定向到 daemon.log；Windows 使用无控制台的新进程组并写同一日志。
//!
//! On macOS the daemon and its children need the current Aqua bootstrap namespace. A daemon
//! started between GUI sessions may still answer on its socket while AppKit and the login
//! keychain are unavailable. Retry through short login transitions and promote a genuinely
//! headless daemon once the GUI domain becomes available.

/// How to treat a daemon process that the platform service manager still reports as alive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpawnPolicy {
    /// Leave an alive instance alone and report success; the caller polls the socket anyway.
    /// This is the normal path: the alive instance is usually one a sibling client just started.
    ReuseAlive,
    /// Tear the existing job down and start a fresh one even if its process is alive. Only for
    /// recovery from an instance that stays unresponsive.
    Replace,
}

pub async fn spawn_detached(policy: SpawnPolicy) -> std::io::Result<()> {
    crate::development::routing::guard_fixture_program(
        crate::executable::launch_path()?.as_os_str(),
    )?;
    #[cfg(target_os = "macos")]
    {
        tokio::task::spawn_blocking(move || start_macos_daemon(policy))
            .await
            .map_err(std::io::Error::other)?
    }
    #[cfg(not(target_os = "macos"))]
    let _ = policy; // Other platforms have no service manager to consult; a duplicate daemon
                    // loses the single-instance `daemon.lock` and exits on its own.
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        spawn_plain_detached()
    }
    #[cfg(windows)]
    {
        spawn_windows_detached()
    }
    #[cfg(not(any(unix, windows)))]
    {
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "daemon spawning is unsupported on this platform",
        ))
    }
}

#[cfg(windows)]
fn spawn_windows_detached() -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{
        CreateProcessW, CREATE_NEW_PROCESS_GROUP, CREATE_NO_WINDOW, CREATE_UNICODE_ENVIRONMENT,
        PROCESS_INFORMATION, STARTUPINFOW,
    };

    let exe = crate::executable::launch_path()?;
    let mut application: Vec<u16> = exe.as_os_str().encode_wide().collect();
    application.push(0);

    // `std::process::Command` must enable handle inheritance when it wires redirected stdio.
    // On Windows that can retain an unrelated caller pipeline in the daemon and its GUI children,
    // so a script capturing `AskHuman daemon start` never observes EOF. Create the background role
    // with inheritance explicitly disabled; `--background` makes the child append logs itself.
    let mut command_line = Vec::new();
    command_line.push(b'"' as u16);
    command_line.extend(exe.as_os_str().encode_wide());
    command_line.push(b'"' as u16);
    command_line.extend(" daemon run --background".encode_utf16());
    command_line.push(0);

    let startup = STARTUPINFOW {
        cb: std::mem::size_of::<STARTUPINFOW>() as u32,
        ..Default::default()
    };
    let mut process = PROCESS_INFORMATION::default();
    let environment = background_environment()?;
    let created = unsafe {
        CreateProcessW(
            application.as_ptr(),
            command_line.as_mut_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            0,
            CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW | CREATE_UNICODE_ENVIRONMENT,
            environment.as_ptr().cast(),
            std::ptr::null(),
            &startup,
            &mut process,
        )
    };
    if created == 0 {
        return Err(std::io::Error::last_os_error());
    }
    unsafe {
        CloseHandle(process.hThread);
        CloseHandle(process.hProcess);
    }
    Ok(())
}

#[cfg(windows)]
fn background_environment() -> std::io::Result<Vec<u16>> {
    use windows_sys::Win32::System::Environment::{
        FreeEnvironmentStringsW, GetEnvironmentStringsW,
    };
    // Preserve Windows' complete sorted block, including hidden drive-current-directory entries.
    unsafe {
        let block = GetEnvironmentStringsW();
        if block.is_null() {
            return Err(std::io::Error::last_os_error());
        }
        let mut pointer = block;
        let mut result = Vec::new();
        while *pointer != 0 {
            let mut length = 0;
            while *pointer.add(length) != 0 {
                length += 1;
            }
            let entry = std::slice::from_raw_parts(pointer, length);
            let name_end = entry
                .iter()
                .position(|ch| *ch == b'=' as u16)
                .unwrap_or(length);
            let name = String::from_utf16_lossy(&entry[..name_end]);
            if !crate::development::client_environment_key(&name) {
                result.extend_from_slice(entry);
                result.push(0);
            }
            pointer = pointer.add(length + 1);
        }
        FreeEnvironmentStringsW(block);
        result.push(0);
        if result.len() == 1 {
            result.push(0);
        }
        Ok(result)
    }
}

/// Prevent background roles from opening a second console while keeping ordinary CLI output.
pub fn configure_background(command: &mut std::process::Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW);
    }
    #[cfg(not(windows))]
    let _ = command;
}

/// Spawn a GUI child without blocking the caller, and reap it when it exits on Unix.
// VERIFY: Exercise popup, warm popup, and GUI Host launch/exit on native macOS and Windows.
pub fn spawn_and_reap(command: &mut std::process::Command) -> std::io::Result<u32> {
    #[cfg(unix)]
    {
        let (sender, receiver) = std::sync::mpsc::channel::<std::process::Child>();
        // Allocate the waiter before spawning: a thread creation failure must not orphan a child.
        std::thread::Builder::new()
            .name("gui-child-reaper".into())
            .spawn(move || {
                if let Ok(mut child) = receiver.recv() {
                    let _ = child.wait();
                }
            })?;
        let child = command.spawn()?;
        let pid = child.id();
        if let Err(error) = sender.send(child) {
            // Recover ownership if the waiter unexpectedly disappears before receiving the child.
            let mut child = error.0;
            let _ = child.kill();
            let _ = child.wait();
            return Err(std::io::Error::other("GUI child reaper disconnected"));
        }
        Ok(pid)
    }
    #[cfg(not(unix))]
    {
        command.spawn().map(|child| child.id())
    }
}

/// 原始拉起方式：`setsid` 新建会话 + stdio 重定向到 daemon.log，直接继承当前会话上下文。
#[cfg(unix)]
fn spawn_plain_detached() -> std::io::Result<()> {
    use super::lifecycle;
    use std::os::unix::process::CommandExt;
    use std::process::{Command, Stdio};

    let exe = crate::executable::launch_path()?;
    if let Some(dir) = lifecycle::log_path().parent() {
        std::fs::create_dir_all(dir)?;
    }
    let log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(lifecycle::log_path())?;
    let log_err = log.try_clone()?;

    let mut cmd = Command::new(exe);
    if let Some(home) = crate::development::routing::background_account_home() {
        cmd.env("HOME", home);
    }
    crate::development::clear_client_context(&mut cmd);
    cmd.arg("daemon")
        .arg("run")
        .stdin(Stdio::null())
        .stdout(Stdio::from(log))
        .stderr(Stdio::from(log_err));
    // 新建会话，彻底脱离调用方的控制终端 / 进程组（终端关闭不会带走 daemon）。
    unsafe {
        cmd.pre_exec(|| {
            libc::setsid();
            Ok(())
        });
    }
    cmd.spawn()?;
    Ok(())
}

// ===== macOS：经 GUI launchd 域拉起 =====

#[cfg(target_os = "macos")]
const DAEMON_LAUNCHD_LABEL: &str = "com.naituw.humaninloop.daemon";

#[cfg(target_os = "macos")]
pub(super) fn is_macos_headless() -> bool {
    static HEADLESS: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *HEADLESS.get_or_init(|| {
        // The fallback calls setsid and disconnects stdin. Check the running process so a
        // surviving older CLI or MCP can start this binary without passing a new flag.
        let pid = std::process::id();
        if unsafe { libc::getsid(0) } != pid as libc::pid_t
            || unsafe { libc::isatty(libc::STDIN_FILENO) } == 1
        {
            return false;
        }
        let label = instance_launchd_label(&crate::paths::config_dir());
        launchd_job(&gui_domain(), &label)
            .as_deref()
            .and_then(launchd_print_pid)
            != Some(pid)
    })
}

#[cfg(target_os = "macos")]
fn gui_domain() -> String {
    format!("gui/{}", unsafe { libc::getuid() })
}

#[cfg(target_os = "macos")]
pub(super) fn macos_gui_available() -> std::io::Result<bool> {
    gui_domain_exists(&gui_domain())
}

#[cfg(target_os = "macos")]
fn gui_domain_exists(domain: &str) -> std::io::Result<bool> {
    let output = std::process::Command::new("/bin/launchctl")
        .args(["print", domain])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .output()?;
    match output.status.code() {
        Some(0) => Ok(true),
        // launchctl's "Could not find specified domain" status.
        Some(112) => Ok(false),
        _ => Err(launchctl_error("print GUI domain", &output)),
    }
}

#[cfg(target_os = "macos")]
fn launchctl_error(action: &str, output: &std::process::Output) -> std::io::Error {
    let stderr = String::from_utf8_lossy(&output.stderr[..output.stderr.len().min(1024)]);
    std::io::Error::other(format!(
        "launchctl {action} failed ({}): {}",
        output.status,
        stderr.trim().replace('\n', " ")
    ))
}

#[cfg(target_os = "macos")]
fn start_macos_daemon(policy: SpawnPolicy) -> std::io::Result<()> {
    use std::time::{Duration, Instant};

    let domain = gui_domain();
    let deadline = Instant::now() + Duration::from_secs(6);
    let started = retry_gui_launch(
        || {
            if !gui_domain_exists(&domain)? {
                return Ok(false);
            }
            spawn_via_gui_launchd(&domain, policy, false).map(|()| true)
        },
        || {
            if Instant::now() >= deadline {
                return false;
            }
            std::thread::sleep(Duration::from_millis(250));
            true
        },
    )?;
    if started {
        Ok(())
    } else {
        spawn_plain_detached()
    }
}

#[cfg(target_os = "macos")]
fn retry_gui_launch(
    mut attempt: impl FnMut() -> std::io::Result<bool>,
    mut retry: impl FnMut() -> bool,
) -> std::io::Result<bool> {
    loop {
        let result = attempt();
        if matches!(result, Ok(true)) || !retry() {
            return result;
        }
    }
}

#[cfg(target_os = "macos")]
pub(super) fn start_macos_gui_successor() -> std::io::Result<()> {
    crate::development::routing::guard_fixture_program(
        crate::executable::launch_path()?.as_os_str(),
    )?;
    spawn_via_gui_launchd(&gui_domain(), SpawnPolicy::Replace, true)
}

/// The account's home directory from the user database. Unlike `$HOME` it cannot be overridden
/// by a caller that wants a throwaway profile.
#[cfg(target_os = "macos")]
fn account_home() -> Option<std::path::PathBuf> {
    crate::paths::account::home().ok()
}

/// Whether two directories are the same place, looking through symlinks when both exist.
#[cfg(target_os = "macos")]
fn same_dir(a: &std::path::Path, b: &std::path::Path) -> bool {
    a == b
        || matches!(
            (std::fs::canonicalize(a), std::fs::canonicalize(b)),
            (Ok(a), Ok(b)) if a == b
        )
}

/// Whether the daemon of `config_dir` must not use the production launchd label.
///
/// That is every instance (`ASKHUMAN_HOME`), and also any process whose config directory is not
/// the account's own `~/.askhuman`, e.g. a sandbox that only overrides `$HOME`. The label is
/// global to the `gui/<uid>` domain, so sharing it would let such a daemon `bootout` the user's
/// real one. When the account's home is unknown the instance flag alone decides, as before.
#[cfg(target_os = "macos")]
fn needs_isolated_label(
    dev_instance: bool,
    config_dir: &std::path::Path,
    account_home: Option<&std::path::Path>,
) -> bool {
    if dev_instance {
        return true;
    }
    match account_home {
        Some(home) => !same_dir(config_dir, &home.join(".askhuman")),
        None => false,
    }
}

/// Production 沿用登录项的固定 label；Dev Instance 以及配置目录不是账户自己的 `~/.askhuman`
/// 的进程（例如只覆盖了 `$HOME` 的沙箱）按配置目录生成稳定后缀，避免多个工作树、沙箱以及
/// production daemon 在同一个 `gui/<uid>` domain 内互相 bootout。
#[cfg(target_os = "macos")]
fn launchd_label(config_dir: &std::path::Path, isolated: bool) -> String {
    if !isolated {
        return DAEMON_LAUNCHD_LABEL.to_string();
    }

    use sha2::{Digest, Sha256};
    use std::fmt::Write as _;

    let digest = Sha256::digest(config_dir.to_string_lossy().as_bytes());
    let mut suffix = String::with_capacity(16);
    for byte in &digest[..8] {
        write!(&mut suffix, "{byte:02x}").expect("writing to String cannot fail");
    }
    format!("{DAEMON_LAUNCHD_LABEL}.instance.{suffix}")
}

#[cfg(target_os = "macos")]
fn instance_launchd_label(config_dir: &std::path::Path) -> String {
    let isolated = needs_isolated_label(
        crate::dev_instance::is_dev_instance()
            || crate::development::routing::shared_writes_blocked(),
        config_dir,
        account_home().as_deref(),
    );
    launchd_label(config_dir, isolated)
}

/// 经 `gui/<uid>` launchd 域 bootstrap 一个跑 `daemon run` 的任务，使 daemon 落在 Aqua 会话。
///
/// 透传 HOME / TMPDIR / PATH 及全部 `ASKHUMAN_*` 环境变量，保住 perf/隔离调用方（隔离 HOME、
/// `ASKHUMAN_NO_KEYCHAIN`、mock API base 等）的语义。成功返回 `Ok(())`，否则 `Err`（调用方回退）。
#[cfg(target_os = "macos")]
fn spawn_via_gui_launchd(
    domain: &str,
    policy: SpawnPolicy,
    wait_for_handoff: bool,
) -> std::io::Result<()> {
    use super::lifecycle;
    use std::process::{Command, Stdio};

    let exe = crate::executable::launch_path()?;
    let log = lifecycle::log_path();
    if let Some(dir) = log.parent() {
        std::fs::create_dir_all(dir)?;
    }

    let config_dir = crate::paths::config_dir();
    let label = instance_launchd_label(&config_dir);
    let plist_path = config_dir.join("daemon-launchd.plist");

    // `bootout` below is a SIGTERM (then SIGKILL after the job's 5s exit timeout) for a job whose
    // process is still alive, and the daemon it would hit is almost always one a sibling client
    // started a moment ago. Never tear an alive job down on the reuse path; the caller keeps
    // polling the socket and picks that instance up as it becomes ready.
    if policy == SpawnPolicy::ReuseAlive && launchd_job_alive(domain, &label) {
        return Ok(());
    }

    // 透传隔离/配置相关 env：HOME/TMPDIR/PATH + 全部 ASKHUMAN_*。
    let mut env_xml = String::new();
    for key in ["HOME", "TMPDIR", "PATH"] {
        if key == "HOME" {
            if let Some(home) = crate::development::routing::background_account_home() {
                env_xml.push_str(&plist_env_entry(key, &home.to_string_lossy()));
                continue;
            }
        }
        if let Ok(v) = std::env::var(key) {
            env_xml.push_str(&plist_env_entry(key, &v));
        }
    }
    env_xml.push_str(&askhuman_env_xml(std::env::vars_os()));

    let plist = launchd_plist_contents(
        &exe.display().to_string(),
        &log.display().to_string(),
        &label,
        &env_xml,
        wait_for_handoff,
    );
    std::fs::write(&plist_path, plist)?;

    // 自清理：先 bootout 上次残留的任务（正常已退出；`Replace` 时可能仍在跑，SIGTERM 由 daemon 的
    // 信号处理走 graceful 退出），再 bootstrap 新的（RunAtLoad 立即启动）。
    let _ = Command::new("/bin/launchctl")
        .args(["bootout", &format!("{domain}/{label}")])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    let output = Command::new("/bin/launchctl")
        .args(["bootstrap", domain])
        .arg(&plist_path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .output()?;
    if output.status.success()
        || (policy == SpawnPolicy::ReuseAlive && launchd_job_alive(domain, &label))
    {
        Ok(())
    } else {
        Err(launchctl_error("bootstrap GUI domain", &output))
    }
}

#[cfg(target_os = "macos")]
fn launchd_plist_contents(
    exe: &str,
    log: &str,
    label: &str,
    env_xml: &str,
    wait_for_handoff: bool,
) -> String {
    let handoff_arg = if wait_for_handoff {
        "\n        <string>--wait-for-gui-handoff</string>"
    } else {
        ""
    };
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>{label}</string>
    <key>ProgramArguments</key>
    <array>
        <string>{exe}</string>
        <string>daemon</string>
        <string>run</string>{handoff_arg}
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>ProcessType</key>
    <string>Interactive</string>
    <key>StandardOutPath</key>
    <string>{log}</string>
    <key>StandardErrorPath</key>
    <string>{log}</string>
    <key>EnvironmentVariables</key>
    <dict>
{env_xml}    </dict>
</dict>
</plist>
"#,
        label = xml_escape(label),
        exe = xml_escape(exe),
        log = xml_escape(log),
    )
}

/// Every `ASKHUMAN_*` entry of `vars` as plist entries, in the given order. Non-Unicode entries
/// are skipped: `std::env::vars()` would panic on one (release builds abort), whichever variable
/// it belongs to, and a plist string cannot carry it.
#[cfg(target_os = "macos")]
fn askhuman_env_xml(
    vars: impl Iterator<Item = (std::ffi::OsString, std::ffi::OsString)>,
) -> String {
    let mut xml = String::new();
    for (key, value) in vars {
        let (Some(key), Some(value)) = (key.to_str(), value.to_str()) else {
            continue;
        };
        if key.starts_with("ASKHUMAN_") && !crate::development::client_environment_key(key) {
            xml.push_str(&plist_env_entry(key, value));
        }
    }
    xml
}

/// 生成一条 plist `EnvironmentVariables` 子项（key/value 均做 XML 转义）。
#[cfg(target_os = "macos")]
fn plist_env_entry(key: &str, value: &str) -> String {
    format!(
        "        <key>{}</key>\n        <string>{}</string>\n",
        xml_escape(key),
        xml_escape(value)
    )
}

/// 最小 XML 转义（路径/值理论上可能含 & < >）。
#[cfg(target_os = "macos")]
fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Ask launchd whether the daemon job's process is currently alive. A job that is not loaded or
/// has exited (`state = not running`, no `pid = …`) is not alive; a failing `launchctl` is
/// treated as "not alive" so the caller falls through to the regular bootstrap path.
#[cfg(target_os = "macos")]
fn launchd_job_alive(domain: &str, label: &str) -> bool {
    launchd_job(domain, label)
        .as_deref()
        .is_some_and(launchd_print_reports_alive)
}

#[cfg(target_os = "macos")]
fn launchd_job(domain: &str, label: &str) -> Option<String> {
    use std::process::{Command, Stdio};

    let out = Command::new("/bin/launchctl")
        .args(["print", &format!("{domain}/{label}")])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

#[cfg(target_os = "macos")]
fn launchd_print_pid(text: &str) -> Option<u32> {
    text.lines()
        .find_map(|line| line.strip_prefix("\tpid = ")?.trim().parse().ok())
}

/// Parse `launchctl print <service>` output. Only the service's own top-level `state = …` line
/// counts (endpoint sub-blocks also print `state = active`); an explicit `pid = …` line is the
/// strongest signal and appears only while the process exists.
#[cfg(target_os = "macos")]
fn launchd_print_reports_alive(text: &str) -> bool {
    if launchd_print_pid(text).is_some() {
        return true;
    }
    let mut top_level_state: Option<&str> = None;
    for line in text.lines() {
        let trimmed = line.trim_start();
        // Top-level keys are indented by exactly one tab; nested blocks use two or more.
        if top_level_state.is_none() && line.starts_with('\t') && !line.starts_with("\t\t") {
            if let Some(value) = trimmed.strip_prefix("state = ") {
                top_level_state = Some(value.trim());
            }
        }
    }
    matches!(top_level_state, Some(state) if state != "not running")
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn production_and_isolated_launchd_labels_do_not_collide() {
        let production = launchd_label(Path::new("/Users/test/.askhuman"), false);
        let dev_a = launchd_label(Path::new("/work/a/.askhuman-dev/home"), true);
        let dev_a_again = launchd_label(Path::new("/work/a/.askhuman-dev/home"), true);
        let dev_b = launchd_label(Path::new("/work/b/.askhuman-dev/home"), true);

        assert_eq!(production, DAEMON_LAUNCHD_LABEL);
        assert_eq!(dev_a, dev_a_again);
        assert_ne!(dev_a, production);
        assert_ne!(dev_a, dev_b);
        assert!(dev_a.starts_with(&format!("{DAEMON_LAUNCHD_LABEL}.instance.")));
    }

    #[test]
    fn the_account_own_config_dir_keeps_the_production_label() {
        let home = Path::new("/Users/test");
        let config = home.join(".askhuman");
        assert!(!needs_isolated_label(false, &config, Some(home)));
        // A trailing slash or a `.` segment is the same place.
        assert!(!needs_isolated_label(
            false,
            Path::new("/Users/test/./.askhuman/"),
            Some(home)
        ));
    }

    #[test]
    fn a_home_only_sandbox_gets_its_own_label() {
        // `HOME=/tmp/sandbox` moves the config dir without setting `ASKHUMAN_HOME`.
        let account = Path::new("/Users/test");
        let sandbox = Path::new("/tmp/sandbox/.askhuman");
        assert!(needs_isolated_label(false, sandbox, Some(account)));
        let production = launchd_label(&account.join(".askhuman"), false);
        let isolated = launchd_label(sandbox, true);
        assert_ne!(isolated, production);
        assert_ne!(
            isolated,
            launchd_label(Path::new("/tmp/other/.askhuman"), true)
        );
    }

    #[test]
    fn forwarded_askhuman_env_skips_non_unicode_entries() {
        use std::ffi::OsString;
        use std::os::unix::ffi::OsStringExt;
        let vars = vec![
            (OsString::from("ASKHUMAN_HOME"), OsString::from("/tmp/a&b")),
            (OsString::from("UNRELATED"), OsString::from("x")),
            // A non-Unicode entry makes `std::env::vars()` panic, whichever variable holds it.
            (
                OsString::from("OTHER"),
                OsString::from_vec(b"caf\xff".to_vec()),
            ),
            (
                OsString::from("ASKHUMAN_BAD"),
                OsString::from_vec(b"caf\xff".to_vec()),
            ),
            (
                OsString::from_vec(b"ASKHUMAN_\xff".to_vec()),
                OsString::from("v"),
            ),
            (OsString::from("ASKHUMAN_MOCK"), OsString::from("1")),
        ];
        assert_eq!(
            askhuman_env_xml(vars.into_iter()),
            format!(
                "{}{}",
                plist_env_entry("ASKHUMAN_HOME", "/tmp/a&b"),
                plist_env_entry("ASKHUMAN_MOCK", "1")
            )
        );
    }

    #[test]
    fn an_instance_is_isolated_even_in_the_default_directory() {
        let home = Path::new("/Users/test");
        assert!(needs_isolated_label(
            true,
            &home.join(".askhuman"),
            Some(home)
        ));
    }

    #[test]
    fn an_unknown_account_home_falls_back_to_the_instance_flag() {
        let config = Path::new("/tmp/sandbox/.askhuman");
        assert!(!needs_isolated_label(false, config, None));
        assert!(needs_isolated_label(true, config, None));
    }

    #[test]
    fn symlinked_spellings_of_the_default_dir_are_the_same_place() {
        let root = std::env::temp_dir().join(format!("ah-label-{}", uuid::Uuid::new_v4()));
        let real = root.join("real-home");
        std::fs::create_dir_all(real.join(".askhuman")).unwrap();
        let link = root.join("link-home");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        assert!(!needs_isolated_label(
            false,
            &link.join(".askhuman"),
            Some(&real)
        ));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_account_home_is_read_from_the_user_database() {
        let home = account_home().expect("the test user has a passwd entry");
        assert!(home.is_absolute());
        assert!(home.is_dir());
    }

    #[test]
    fn launchd_plist_binds_daemon_to_interactive_gui_job() {
        let plist = launchd_plist_contents(
            "/Applications/A&B/AskHuman",
            "/Users/test/log<1>",
            DAEMON_LAUNCHD_LABEL,
            "        <key>ASKHUMAN_HOME</key>\n        <string>/tmp/dev</string>\n",
            false,
        );

        assert!(plist.contains(&format!("<string>{DAEMON_LAUNCHD_LABEL}</string>")));
        assert!(plist.contains("<string>daemon</string>"));
        assert!(plist.contains("<string>run</string>"));
        assert!(plist.contains("<key>RunAtLoad</key>"));
        assert!(plist.contains("<string>Interactive</string>"));
        assert!(plist.contains("/Applications/A&amp;B/AskHuman"));
        assert!(plist.contains("/Users/test/log&lt;1&gt;"));
        assert!(plist.contains("<key>ASKHUMAN_HOME</key>"));
        assert!(!plist.contains("<key>KeepAlive</key>"));
        assert!(!plist.contains("--wait-for-gui-handoff"));
        let successor = launchd_plist_contents("/test/AskHuman", "/test/log", "test", "", true);
        assert!(successor.contains("<string>--wait-for-gui-handoff</string>"));
    }

    #[test]
    fn gui_launch_waits_through_the_login_gap() {
        let mut results = [
            Ok(false),
            Err(std::io::Error::other("login bootstrap in progress")),
            Ok(true),
        ]
        .into_iter();
        let result = retry_gui_launch(|| results.next().unwrap(), || true);
        assert!(result.unwrap());
        assert!(results.next().is_none());
    }

    #[test]
    fn gui_launch_only_allows_headless_when_the_domain_is_missing() {
        let mut retries = 2;
        let result = retry_gui_launch(
            || Err(std::io::Error::other("launchd refused the GUI job")),
            || {
                retries -= 1;
                retries > 0
            },
        );
        assert_eq!(
            result.unwrap_err().to_string(),
            "launchd refused the GUI job"
        );
        assert_eq!(retries, 0);
        assert!(!retry_gui_launch(|| Ok(false), || false).unwrap());
    }

    #[test]
    fn launchd_print_running_job_is_alive() {
        let text = "com.naituw.humaninloop.daemon = {\n\tactive count = 1\n\tpath = /x.plist\n\tstate = running\n\n\tprogram = /x/AskHuman\n\tpid = 81380\n\tlast exit code = (never exited)\n\tendpoints = {\n\t\t\"a\" = {\n\t\t\tstate = active\n\t\t}\n\t}\n}\n";
        assert!(launchd_print_reports_alive(text));
        assert_eq!(launchd_print_pid(text), Some(81380));
    }

    #[test]
    fn launchd_print_exited_job_is_not_alive() {
        // Loaded but exited: no `pid =` line; nested endpoint `state = active` must not count.
        let text = "com.naituw.humaninloop.daemon = {\n\tactive count = 0\n\tstate = not running\n\n\tprogram = /x/AskHuman\n\tlast exit code = 0\n\tendpoints = {\n\t\t\"a\" = {\n\t\t\tstate = active\n\t\t}\n\t}\n}\n";
        assert!(!launchd_print_reports_alive(text));
        assert!(!launchd_print_reports_alive(""));
    }

    #[test]
    fn launchd_print_scheduled_job_counts_as_alive() {
        // Just bootstrapped by a sibling: process not yet forked, but tearing it down would race.
        let text = "com.naituw.humaninloop.daemon = {\n\tstate = spawn scheduled\n\tprogram = /x/AskHuman\n}\n";
        assert!(launchd_print_reports_alive(text));
    }
}

#[cfg(all(test, unix))]
mod reaper_tests {
    use super::spawn_and_reap;
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};

    fn assert_reaped(pid: u32) {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let result = unsafe { libc::kill(pid as libc::pid_t, 0) };
            if result == -1 {
                assert_eq!(
                    std::io::Error::last_os_error().raw_os_error(),
                    Some(libc::ESRCH)
                );
                #[cfg(target_os = "linux")]
                assert!(!std::path::Path::new(&format!("/proc/{pid}")).exists());
                return;
            }
            assert!(Instant::now() < deadline, "child {pid} was not reaped");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn spawn_error_is_returned() {
        let error = spawn_and_reap(&mut Command::new("/nonexistent/askhuman-reaper-test"))
            .expect_err("missing executable must fail");
        assert_eq!(error.kind(), std::io::ErrorKind::NotFound);
    }

    #[test]
    fn immediately_exiting_child_is_reaped() {
        let pid =
            spawn_and_reap(Command::new("/bin/sh").args(["-c", "exit 0"])).expect("spawn shell");
        assert_reaped(pid);
    }

    #[test]
    fn running_child_does_not_block_and_is_eventually_reaped() {
        // The fixture exits on its own even if an assertion fails.
        let pid = spawn_and_reap(
            Command::new("/bin/sh")
                .args(["-c", "exec sleep 1"])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null()),
        )
        .expect("spawn sleeping child");
        assert_eq!(unsafe { libc::kill(pid as libc::pid_t, 0) }, 0);
        assert_reaped(pid);
    }
}
