//! 后台拉起 Daemon：detach（新会话）+ stdio 重定向到 daemon.log，使其脱离 CLI 终端独立存活。
//!
//! macOS 会话修正：若 CLI 本身处于「非 GUI(非 Aqua) 会话」（如 SSH、Codex app-server 等后台上下文，
//! `launchctl managername != "Aqua"`），直接 `setsid` 拉起的 daemon 会继承这个非 Aqua 安全会话，
//! **运行时读不到登录钥匙串** → 各渠道密钥取空 → 渠道被判「未配置」→ 长连接不建（现象：飞书 `/status`
//! 无回复、`daemon status` 显 `im conns none`）。此时改经用户 GUI launchd 域（`gui/<uid>`）bootstrap 一个
//! 跑 `daemon run` 的任务，让 daemon 落在 Aqua 会话、能静默读钥匙串（复刻 install.sh 的
//! `sign_via_gui_launchd` 手法）。GUI 会话、或 GUI 域不可用（纯 headless 无图形登录）时回退原 setsid 拉起。

#[cfg(unix)]
pub fn spawn_detached() -> std::io::Result<()> {
    #[cfg(target_os = "macos")]
    {
        // 仅「非 Aqua 会话」才绕道 GUI 域；Aqua 会话保持原样，
        // 不影响 perf/隔离等以 HOME/env 隔离的前台调用方。
        if !in_aqua_session() {
            if spawn_via_gui_launchd().is_ok() {
                return Ok(());
            }
            // GUI 域不可用（纯 headless）→ 回退原 setsid 拉起。
        }
    }
    spawn_plain_detached()
}

/// 原始拉起方式：`setsid` 新建会话 + stdio 重定向到 daemon.log，直接继承当前会话上下文。
#[cfg(unix)]
fn spawn_plain_detached() -> std::io::Result<()> {
    use super::lifecycle;
    use std::os::unix::process::CommandExt;
    use std::process::{Command, Stdio};

    let exe = std::env::current_exe()?;
    if let Some(dir) = lifecycle::log_path().parent() {
        std::fs::create_dir_all(dir)?;
    }
    let log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(lifecycle::log_path())?;
    let log_err = log.try_clone()?;

    let mut cmd = Command::new(exe);
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

// ===== macOS：非 Aqua 会话时经 GUI launchd 域拉起 =====

/// 当前进程是否处于用户 GUI(Aqua) 安全会话。
///
/// `launchctl managername` 打印当前 launchd 域管理者名：GUI 登录会话为 `Aqua`，SSH / 后台
/// 上下文为 `Background` / `System` / `StandardIO` 等。非 Aqua 即拿不到登录钥匙串的静默访问权。
/// 查询失败时保守当作 Aqua（不改变原有行为，避免误绕道）。
#[cfg(target_os = "macos")]
fn in_aqua_session() -> bool {
    use std::process::Command;
    match Command::new("/bin/launchctl").arg("managername").output() {
        Ok(out) if out.status.success() => {
            String::from_utf8_lossy(&out.stdout).trim() == "Aqua"
        }
        _ => true,
    }
}

/// 经 `gui/<uid>` launchd 域 bootstrap 一个跑 `daemon run` 的任务，使 daemon 落在 Aqua 会话。
///
/// 透传 HOME / TMPDIR / PATH 及全部 `ASKHUMAN_*` 环境变量，保住 perf/隔离调用方（隔离 HOME、
/// `ASKHUMAN_NO_KEYCHAIN`、mock API base 等）的语义。成功返回 `Ok(())`，否则 `Err`（调用方回退）。
#[cfg(target_os = "macos")]
fn spawn_via_gui_launchd() -> std::io::Result<()> {
    use super::lifecycle;
    use std::process::{Command, Stdio};

    const LABEL: &str = "com.naituw.humaninloop.daemon";

    let exe = std::env::current_exe()?;
    let log = lifecycle::log_path();
    if let Some(dir) = log.parent() {
        std::fs::create_dir_all(dir)?;
    }

    let uid = unsafe { libc::getuid() };
    let domain = format!("gui/{uid}");
    let plist_path = crate::paths::config_dir().join("daemon-launchd.plist");

    // 透传隔离/配置相关 env：HOME/TMPDIR/PATH + 全部 ASKHUMAN_*。
    let mut env_xml = String::new();
    for key in ["HOME", "TMPDIR", "PATH"] {
        if let Ok(v) = std::env::var(key) {
            env_xml.push_str(&plist_env_entry(key, &v));
        }
    }
    for (k, v) in std::env::vars() {
        if k.starts_with("ASKHUMAN_") {
            env_xml.push_str(&plist_env_entry(&k, &v));
        }
    }

    let plist = format!(
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
        <string>run</string>
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
        label = LABEL,
        exe = xml_escape(&exe.display().to_string()),
        log = xml_escape(&log.display().to_string()),
    );
    std::fs::write(&plist_path, plist)?;

    // 自清理：先 bootout 上次残留的（已退出）任务，再 bootstrap 新的（RunAtLoad 立即启动）。
    let plist_str = plist_path.display().to_string();
    let _ = Command::new("/bin/launchctl")
        .args(["bootout", &domain, &plist_str])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    let status = Command::new("/bin/launchctl")
        .args(["bootstrap", &domain, &plist_str])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()?;
    if status.success() {
        Ok(())
    } else {
        Err(std::io::Error::new(
            std::io::ErrorKind::Other,
            "launchctl bootstrap gui domain failed",
        ))
    }
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
