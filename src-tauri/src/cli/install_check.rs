//! `AskHuman doctor install [--warn-only] [--json]`: which AskHuman binaries this machine has, which
//! one a bare `AskHuman` runs, and whether they are fighting over the daemon.
//!
//! Copies of AskHuman (the npm package, `scripts/install.sh`, a hand-made copy, another tool's bundled
//! CLI) share one `~/.askhuman`, one daemon socket and the Agent hook files. While their *contents*
//! differ the daemon gives way to each in turn (spec `docs/specs/daemon-graceful-drain.md` §6), and
//! the symptoms (an old CLI behind an MCP server, asks parked behind a drain) point away from the
//! cause. This module only looks: it finds the copies and says what it sees.
//!
//! Other binaries are probed with `--version` alone. Every release answers it without side effects;
//! an unknown subcommand on an old binary could end up asking the user a question.

use super::cfgio;
use crate::daemon::lifecycle::{self, Fingerprint};
use crate::daemon::restart_guard::{self, Trigger, Yield};
use crate::i18n::Lang;
use crate::ipc::{BinaryConflictInfo, StatusInfo};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// How long one `--version` probe may take before the copy is listed without a version.
const PROBE_TIMEOUT: Duration = Duration::from_secs(3);
/// Daemon replacements older than this are not listed.
const RECENT_MS: u64 = 60 * 60 * 1000;
/// At most this many replacements are listed (the newest).
const RECENT_SHOWN: usize = 5;

/// How a binary was installed, judged from where its file lives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// Inside an npm package (`node_modules/askhuman` shim or `node_modules/@humaninloop/*` binary).
    Npm,
    /// Anything else: `scripts/install.sh`, a hand-made copy, another tool's bundled binary.
    Standalone,
}

impl Source {
    fn label(self) -> &'static str {
        match self {
            Source::Npm => "npm",
            Source::Standalone => "standalone",
        }
    }
}

/// How a PATH entry reaches its binary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Link {
    /// The entry is the binary.
    Direct,
    /// The entry is a symlink to the binary.
    Symlink,
    /// The entry is npm's node shim; the binary sits in a platform package next to it.
    NpmShim,
}

impl Link {
    fn label(self) -> &'static str {
        match self {
            Link::Direct => "direct",
            Link::Symlink => "symlink",
            Link::NpmShim => "npmShim",
        }
    }
}

/// Where a binary was seen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Seen {
    /// An `AskHuman` entry in a PATH directory. `rank` 1 is the one a bare `AskHuman` runs.
    Path {
        rank: usize,
        path: PathBuf,
        link: Link,
    },
    /// The binary running this check.
    ThisProcess,
    /// The binary the daemon runs.
    Daemon,
}

/// One distinct binary file, with everywhere it was seen.
#[derive(Debug, Clone)]
pub struct Installed {
    pub native: PathBuf,
    pub source: Source,
    pub version: Option<String>,
    pub fingerprint: Fingerprint,
    pub seen: Vec<Seen>,
}

impl Installed {
    fn best_rank(&self) -> Option<usize> {
        self.seen
            .iter()
            .filter_map(|seen| match seen {
                Seen::Path { rank, .. } => Some(*rank),
                _ => None,
            })
            .min()
    }

    fn on_path(&self) -> bool {
        self.best_rank().is_some()
    }

    fn is_this_process(&self) -> bool {
        self.seen.contains(&Seen::ThisProcess)
    }

    fn is_daemon(&self) -> bool {
        self.seen.contains(&Seen::Daemon)
    }
}

/// A binary named in a finding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Brief {
    pub path: PathBuf,
    pub version: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Finding {
    /// The daemon refuses to give way to another binary that keeps connecting.
    Held(BinaryConflictInfo),
    /// The daemon runs a different build than the bare `AskHuman` does.
    DaemonElsewhere { daemon: Brief, first: Brief },
    /// A bare `AskHuman` runs a different build than the binary running this check.
    Shadowed { running: Brief, first: Brief },
    /// More than one build is reachable through PATH.
    MultipleBuilds { count: usize },
}

impl Finding {
    fn kind(&self) -> &'static str {
        match self {
            Finding::Held(_) => "held",
            Finding::DaemonElsewhere { .. } => "daemonElsewhere",
            Finding::Shadowed { .. } => "shadowed",
            Finding::MultipleBuilds { .. } => "multipleBuilds",
        }
    }

    fn message(&self, lang: Lang, now_ms: u64) -> String {
        match self {
            Finding::Held(conflict) => {
                let minutes = now_ms.saturating_sub(conflict.since_ms) / 60_000;
                let client = conflict.client_summary();
                cfgio::t(
                    lang,
                    &format!(
                        "Another AskHuman binary keeps connecting to this daemon: {client}. \
                         Automatic replacement has been paused for {minutes} min so requests are not interrupted. \
                         Keep one copy, then run `AskHuman daemon restart`."
                    ),
                    &format!(
                        "有另一份 AskHuman 二进制在反复连接这个 Daemon：{client}。\
                         为不打断提问，已暂停自动换新（{minutes} 分钟前开始）。\
                         请只保留一份，再运行 `AskHuman daemon restart`。"
                    ),
                )
            }
            Finding::DaemonElsewhere { daemon, first } => {
                let (daemon, first) = (daemon.describe(), first.describe());
                cfgio::t(
                    lang,
                    &format!(
                        "The running daemon was started from {daemon}, not from the `AskHuman` on PATH ({first}); \
                         whichever side connects next may replace the other's daemon."
                    ),
                    &format!(
                        "运行中的 Daemon 来自 {daemon}，而不是 PATH 里的 `AskHuman`（{first}）；\
                         下一个连接的一方可能把对方的 Daemon 顶掉。"
                    ),
                )
            }
            Finding::Shadowed { running, first } => {
                let (running, first) = (running.describe(), first.describe());
                cfgio::t(
                    lang,
                    &format!(
                        "`AskHuman` on PATH runs {first}, not {running} (the binary running this check); \
                         terminals and agents keep using the former until it is removed or PATH is reordered."
                    ),
                    &format!(
                        "PATH 里的 `AskHuman` 运行的是 {first}，而不是 {running}（当前运行本检查的二进制）；\
                         终端和 Agent 会继续用前者，除非删掉它或调整 PATH 顺序。"
                    ),
                )
            }
            Finding::MultipleBuilds { count } => cfgio::t(
                lang,
                &format!(
                    "{count} different builds of AskHuman are reachable through PATH. They share one daemon \
                     and take turns replacing it; keep one."
                ),
                &format!(
                    "PATH 上能找到 {count} 个内容不同的 AskHuman。它们共用同一个 Daemon、轮流把对方顶掉；请只保留一份。"
                ),
            ),
        }
    }
}

impl Brief {
    fn describe(&self) -> String {
        match &self.version {
            Some(version) => format!("{} (v{version})", self.path.display()),
            None => self.path.display().to_string(),
        }
    }
}

/// What the daemon says about its own binary.
#[derive(Debug, Clone)]
pub struct DaemonBinary {
    pub exe: PathBuf,
    /// Content of that binary when the daemon started (absent from daemons that predate the field).
    pub fingerprint: Option<Fingerprint>,
}

/// Everything the check reads from the machine, so tests can feed it a fake one.
pub struct Inputs {
    /// Directories of `$PATH`, in order.
    pub path_dirs: Vec<PathBuf>,
    pub current_exe: Option<PathBuf>,
    pub daemon: Option<DaemonBinary>,
    pub conflict: Option<BinaryConflictInfo>,
    pub yields: Vec<Yield>,
    pub now_ms: u64,
}

pub struct Report {
    /// Running inside a Dev Instance: its daemon is isolated and its binary is not meant to win
    /// the PATH, so the PATH and daemon-binary comparison is skipped. A hold and the replacement
    /// history belong to that instance's own daemon and are still reported.
    pub path_scan_skipped: bool,
    pub copies: Vec<Installed>,
    pub findings: Vec<Finding>,
    /// Daemon replacements from the last hour, oldest first.
    pub recent: Vec<Yield>,
    pub now_ms: u64,
}

/// Read the real machine and inspect it.
pub fn collect(status: Option<&StatusInfo>) -> Report {
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    let dev_instance = crate::dev_instance::is_dev_instance_context();
    let path_dirs = std::env::var_os("PATH")
        .filter(|_| !dev_instance)
        .map(|paths| {
            std::env::split_paths(&paths)
                .filter(|dir| !dir.as_os_str().is_empty())
                .collect()
        })
        .unwrap_or_default();
    let daemon = status
        .filter(|s| !dev_instance && !s.exe_path.is_empty())
        .map(|s| DaemonBinary {
            exe: PathBuf::from(&s.exe_path),
            fingerprint: s.fingerprint,
        });
    let inputs = Inputs {
        path_dirs,
        current_exe: std::env::current_exe().ok().filter(|_| !dev_instance),
        daemon,
        conflict: status.and_then(|s| s.binary_conflict.clone()),
        yields: restart_guard::load(),
        now_ms,
    };
    let mut report = inspect(&inputs, &|binary| probe_version(binary, PROBE_TIMEOUT));
    report.path_scan_skipped = dev_instance;
    report
}

/// Find the binaries and judge them. `probe` reports a binary's version (`None` when it cannot).
pub fn inspect(inputs: &Inputs, probe: &dyn Fn(&Path) -> Option<String>) -> Report {
    let mut copies: Vec<Installed> = Vec::new();

    let mut seen_dirs: Vec<&Path> = Vec::new();
    let mut rank = 0;
    for dir in &inputs.path_dirs {
        if seen_dirs.contains(&dir.as_path()) {
            continue;
        }
        seen_dirs.push(dir.as_path());
        let entry = dir.join(entry_name());
        if !is_runnable(&entry) {
            continue;
        }
        let Some(resolved) = resolve(&entry) else {
            continue;
        };
        rank += 1;
        note(
            &mut copies,
            resolved.native,
            resolved.source,
            Seen::Path {
                rank,
                path: entry,
                link: resolved.link,
            },
            probe,
        );
    }
    if let Some(resolved) = inputs.current_exe.as_deref().and_then(resolve) {
        note(
            &mut copies,
            resolved.native,
            resolved.source,
            Seen::ThisProcess,
            probe,
        );
    }
    if let Some(resolved) = inputs
        .daemon
        .as_ref()
        .and_then(|daemon| resolve(&daemon.exe))
    {
        note(
            &mut copies,
            resolved.native,
            resolved.source,
            Seen::Daemon,
            probe,
        );
    }
    // What a bare `AskHuman` runs first, then the rest by PATH order; copies off PATH last.
    copies.sort_by(|a, b| {
        let key = |c: &Installed| (c.best_rank().unwrap_or(usize::MAX), c.native.clone());
        key(a).cmp(&key(b))
    });

    let findings = judge(&copies, inputs);
    let recent = inputs
        .yields
        .iter()
        .filter(|y| inputs.now_ms.saturating_sub(y.at_ms) <= RECENT_MS)
        .cloned()
        .collect();
    Report {
        path_scan_skipped: false,
        copies,
        findings,
        recent,
        now_ms: inputs.now_ms,
    }
}

fn judge(copies: &[Installed], inputs: &Inputs) -> Vec<Finding> {
    let brief = |copy: &Installed| Brief {
        path: copy.native.clone(),
        version: copy.version.clone(),
    };
    let first = copies.iter().find(|c| c.best_rank() == Some(1));
    let this = copies.iter().find(|c| c.is_this_process());
    let daemon = copies.iter().find(|c| c.is_daemon());
    let mut findings = Vec::new();

    if let Some(conflict) = &inputs.conflict {
        findings.push(Finding::Held(conflict.clone()));
    }
    if let (Some(daemon), Some(first)) = (daemon, first) {
        // Same file, different content means an update the daemon has not picked up yet: it does
        // that by itself within seconds, so only a different *file* counts.
        let daemon_fp = inputs
            .daemon
            .as_ref()
            .and_then(|d| d.fingerprint)
            .unwrap_or(daemon.fingerprint);
        if daemon.native != first.native && daemon_fp != first.fingerprint {
            findings.push(Finding::DaemonElsewhere {
                daemon: brief(daemon),
                first: brief(first),
            });
        }
    }
    if let (Some(this), Some(first)) = (this, first) {
        if this.native != first.native && this.fingerprint != first.fingerprint {
            findings.push(Finding::Shadowed {
                running: brief(this),
                first: brief(first),
            });
        }
    }
    let mut builds: Vec<Fingerprint> = Vec::new();
    for copy in copies.iter().filter(|c| c.on_path()) {
        if !builds.contains(&copy.fingerprint) {
            builds.push(copy.fingerprint);
        }
    }
    if builds.len() >= 2 {
        findings.push(Finding::MultipleBuilds {
            count: builds.len(),
        });
    }
    findings
}

/// Record that `native` was seen somewhere, creating its entry (version probe + fingerprint) once.
fn note(
    copies: &mut Vec<Installed>,
    native: PathBuf,
    source: Source,
    seen: Seen,
    probe: &dyn Fn(&Path) -> Option<String>,
) {
    if let Some(existing) = copies.iter_mut().find(|c| c.native == native) {
        existing.seen.push(seen);
        return;
    }
    let fingerprint = lifecycle::fingerprint_uncached(&native);
    let version = probe(&native);
    copies.push(Installed {
        native,
        source,
        version,
        fingerprint,
        seen: vec![seen],
    });
}

fn entry_name() -> &'static str {
    if cfg!(windows) {
        "AskHuman.exe"
    } else {
        "AskHuman"
    }
}

/// A file a shell would run: it exists (links followed) and, on Unix, has an execute bit.
fn is_runnable(path: &Path) -> bool {
    let Ok(meta) = std::fs::metadata(path) else {
        return false;
    };
    if !meta.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        meta.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

struct Resolved {
    native: PathBuf,
    source: Source,
    link: Link,
}

/// Follow `entry` to the native executable it ends up running. `None` when it cannot be found
/// (a dangling link, or an npm shim whose platform package is missing: that `AskHuman` cannot run).
fn resolve(entry: &Path) -> Option<Resolved> {
    let canonical = std::fs::canonicalize(entry).ok()?;
    if let Some(package) = npm_shim_package(&canonical) {
        return Some(Resolved {
            native: npm_platform_binary(&package, platform_key()?)?,
            source: Source::Npm,
            link: Link::NpmShim,
        });
    }
    let symlink = std::fs::symlink_metadata(entry)
        .map(|meta| meta.file_type().is_symlink())
        .unwrap_or(false);
    Some(Resolved {
        source: if in_npm_package(&canonical) {
            Source::Npm
        } else {
            Source::Standalone
        },
        native: canonical,
        link: if symlink { Link::Symlink } else { Link::Direct },
    })
}

/// The package directory when `path` is npm's node shim (`.../node_modules/askhuman/bin/cli.js`).
fn npm_shim_package(path: &Path) -> Option<PathBuf> {
    let names: Vec<String> = path
        .components()
        .rev()
        .take(4)
        .map(|part| part.as_os_str().to_string_lossy().into_owned())
        .collect();
    (names == ["cli.js", "bin", "askhuman", "node_modules"])
        .then(|| path.parent()?.parent().map(Path::to_path_buf))
        .flatten()
}

/// Whether `path` lies inside an npm package of ours (`node_modules/askhuman` or `node_modules/@humaninloop`).
fn in_npm_package(path: &Path) -> bool {
    let parts: Vec<String> = path
        .components()
        .map(|part| part.as_os_str().to_string_lossy().into_owned())
        .collect();
    parts.windows(2).any(|pair| {
        pair[0] == "node_modules" && (pair[1] == "askhuman" || pair[1] == "@humaninloop")
    })
}

/// The platform package directory name npm uses (`packaging/npm/humaninloop/index.js`).
fn platform_key() -> Option<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => Some("darwin-arm64"),
        ("macos", "x86_64") => Some("darwin-x64"),
        ("linux", "x86_64") => Some("linux-x64"),
        ("windows", "x86_64") => Some("win32-x64"),
        _ => None,
    }
}

/// The native binary the shim of `package` would spawn: nested in the package's own
/// `node_modules`, or hoisted next to it.
fn npm_platform_binary(package: &Path, key: &str) -> Option<PathBuf> {
    let file = entry_name();
    let nested = package
        .join("node_modules")
        .join("@humaninloop")
        .join(key)
        .join("bin")
        .join(file);
    let hoisted = package
        .parent()?
        .join("@humaninloop")
        .join(key)
        .join("bin")
        .join(file);
    [nested, hoisted]
        .into_iter()
        .find(|path| path.is_file())
        .and_then(|path| std::fs::canonicalize(path).ok())
}

/// `AskHuman --version` of another binary, killed after `timeout`.
fn probe_version(binary: &Path, timeout: Duration) -> Option<String> {
    let mut command = Command::new(binary);
    // Run from a neutral directory: a binary started inside a Dev Instance tree would join it.
    command
        .arg("--version")
        .current_dir(std::env::temp_dir())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let mut child = command.spawn().ok()?;
    let out = child.stdout.take()?;
    let reader = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        out.take(4096).read_to_end(&mut bytes).ok()?;
        Some(bytes)
    });
    let began = Instant::now();
    let succeeded = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status.success(),
            Err(_) => break false,
            Ok(None) => {}
        }
        if began.elapsed() > timeout {
            let _ = child.kill();
            let _ = child.wait();
            break false;
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    let bytes = reader.join().ok()??;
    if !succeeded {
        return None;
    }
    parse_version(&String::from_utf8_lossy(&bytes))
}

/// `AskHuman v0.15.0` → `0.15.0`.
fn parse_version(output: &str) -> Option<String> {
    output
        .lines()
        .next()?
        .trim()
        .strip_prefix("AskHuman v")
        .map(str::to_string)
}

// —— Output ——

fn version_label(version: &Option<String>) -> String {
    match version {
        Some(version) => format!("v{version}"),
        None => "v?".to_string(),
    }
}

fn clock(at_ms: u64) -> String {
    chrono::DateTime::from_timestamp_millis(at_ms as i64)
        .map(|utc| {
            utc.with_timezone(&chrono::Local)
                .format("%H:%M:%S")
                .to_string()
        })
        .unwrap_or_else(|| "--:--:--".to_string())
}

fn trigger_label(trigger: Trigger, lang: Lang) -> String {
    match trigger {
        Trigger::Client => cfgio::t(lang, "client binary differs", "客户端二进制不同"),
        Trigger::Disk => cfgio::t(lang, "own binary changed on disk", "自身二进制被替换"),
        Trigger::Protocol => cfgio::t(lang, "protocol version differs", "协议号不同"),
    }
}

/// The text of the check. With `warn_only` nothing is returned unless a finding exists, so a script
/// can call it unconditionally.
pub fn render_text(report: &Report, lang: Lang, warn_only: bool) -> String {
    if warn_only && report.findings.is_empty() {
        return String::new();
    }
    let mut out = String::new();
    out.push_str(&cfgio::t(lang, "Install\n", "安装\n"));
    if report.findings.is_empty() && !report.path_scan_skipped {
        out.push_str(&cfgio::t(
            lang,
            "  ok: no conflicting AskHuman binaries found\n",
            "  正常：没有发现互相冲突的 AskHuman 二进制\n",
        ));
    }
    for finding in &report.findings {
        out.push_str(&format!(
            "  {}: {}\n",
            cfgio::t(lang, "warning", "警告"),
            finding.message(lang, report.now_ms)
        ));
    }
    if report.path_scan_skipped {
        out.push_str(&cfgio::t(
            lang,
            "  PATH comparison skipped: this is a Dev Instance with its own daemon\n",
            "  已跳过 PATH 比对：当前是 Dev Instance，有独立的 Daemon\n",
        ));
    } else {
        out.push_str(&cfgio::t(
            lang,
            "  AskHuman binaries (the first PATH entry is what a bare `AskHuman` runs):\n",
            "  AskHuman 二进制（PATH 里排第一的就是直接敲 `AskHuman` 运行的那份）：\n",
        ));
    }
    for copy in &report.copies {
        out.push_str(&format!(
            "    {}  fp {}  {}  [{}]\n",
            version_label(&copy.version),
            copy.fingerprint.short(),
            copy.native.display(),
            copy.source.label(),
        ));
        for seen in &copy.seen {
            match seen {
                Seen::Path { rank, path, link } => out.push_str(&format!(
                    "        PATH #{rank}  {}{}\n",
                    path.display(),
                    match link {
                        Link::Direct => String::new(),
                        Link::Symlink => cfgio::t(lang, "  (symlink)", "  （符号链接）"),
                        Link::NpmShim => cfgio::t(lang, "  (npm shim)", "  （npm 入口脚本）"),
                    }
                )),
                Seen::ThisProcess => out.push_str(&format!(
                    "        {}\n",
                    cfgio::t(lang, "running this check", "当前运行本检查的二进制")
                )),
                Seen::Daemon => out.push_str(&format!(
                    "        {}\n",
                    cfgio::t(lang, "run by the daemon", "Daemon 正在运行的二进制")
                )),
            }
        }
    }
    if report.recent.is_empty() {
        return out.trim_end().to_string();
    }
    out.push_str(&format!(
        "  {} ({}):\n",
        cfgio::t(
            lang,
            "Daemon replaced in the last hour",
            "最近一小时 Daemon 被换新"
        ),
        report.recent.len()
    ));
    let skip = report.recent.len().saturating_sub(RECENT_SHOWN);
    for entry in report.recent.iter().skip(skip) {
        let who = match &entry.client {
            Some(peer) => format!("v{} (pid {}) {}", peer.version, peer.pid, peer.path),
            None => String::new(),
        };
        out.push_str(&format!(
            "    {}  {}  {}\n",
            clock(entry.at_ms),
            trigger_label(entry.trigger, lang),
            who
        ));
    }
    out.trim_end().to_string()
}

pub fn render_json_value(report: &Report, lang: Lang) -> serde_json::Value {
    let copies: Vec<serde_json::Value> = report
        .copies
        .iter()
        .map(|copy| {
            let seen: Vec<serde_json::Value> = copy
                .seen
                .iter()
                .map(|seen| match seen {
                    Seen::Path { rank, path, link } => serde_json::json!({
                        "kind": "path",
                        "rank": rank,
                        "path": path.display().to_string(),
                        "link": link.label(),
                    }),
                    Seen::ThisProcess => serde_json::json!({ "kind": "thisProcess" }),
                    Seen::Daemon => serde_json::json!({ "kind": "daemon" }),
                })
                .collect();
            serde_json::json!({
                "native": copy.native.display().to_string(),
                "source": copy.source.label(),
                "version": copy.version,
                "fingerprint": copy.fingerprint.short(),
                "seen": seen,
            })
        })
        .collect();
    let findings: Vec<serde_json::Value> = report
        .findings
        .iter()
        .map(|finding| {
            serde_json::json!({
                "kind": finding.kind(),
                "message": finding.message(lang, report.now_ms),
            })
        })
        .collect();
    serde_json::json!({
        "pathScanSkipped": report.path_scan_skipped,
        "copies": copies,
        "findings": findings,
        "recentReplacements": report.recent,
    })
}

const HELP_EN: &str = "AskHuman doctor install [--warn-only] [--json] — list the AskHuman binaries on this machine and say which one wins\n  --warn-only  print only when something conflicts (for scripts)";
const HELP_ZH: &str = "AskHuman doctor install [--warn-only] [--json] —— 列出本机的 AskHuman 二进制，指出实际生效的是哪一份\n  --warn-only  仅在发现冲突时输出（供脚本调用）";

/// `AskHuman doctor install ...`
pub fn dispatch(args: &[String], lang: Lang) {
    if args
        .iter()
        .any(|a| a == "help" || a == "-h" || a == "--help")
    {
        super::print_line(&cfgio::t(lang, HELP_EN, HELP_ZH));
        return;
    }
    let status = cfgio::daemon_status();
    let report = collect(status.as_ref());
    if args.iter().any(|a| a == "--json") {
        let value = render_json_value(&report, lang);
        super::print_line(&serde_json::to_string_pretty(&value).unwrap_or_default());
        return;
    }
    let text = render_text(&report, lang, args.iter().any(|a| a == "--warn-only"));
    if !text.is_empty() {
        super::print_line(&text);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    fn write_binary(path: &Path, content: &str) {
        use std::os::unix::fs::PermissionsExt;
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, content).unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    /// The version of a fake binary is its file content (`1.0.0`), no process involved.
    fn read_version(binary: &Path) -> Option<String> {
        std::fs::read_to_string(binary).ok()
    }

    fn inputs(path_dirs: Vec<PathBuf>) -> Inputs {
        Inputs {
            path_dirs,
            current_exe: None,
            daemon: None,
            conflict: None,
            yields: Vec::new(),
            now_ms: 10_000_000,
        }
    }

    #[test]
    fn parses_the_version_line() {
        assert_eq!(
            parse_version("AskHuman v0.15.0\n").as_deref(),
            Some("0.15.0")
        );
        assert_eq!(
            parse_version("AskHuman v0.15.0-rc.1").as_deref(),
            Some("0.15.0-rc.1")
        );
        assert_eq!(parse_version("usage: something else"), None);
        assert_eq!(parse_version(""), None);
    }

    #[cfg(unix)]
    #[test]
    fn probes_a_real_process_and_gives_up_on_a_hung_one() {
        let dir = tempfile::tempdir().unwrap();
        let good = dir.path().join("good");
        write_binary(&good, "#!/bin/sh\necho 'AskHuman v9.9.9'\n");
        assert_eq!(
            probe_version(&good, Duration::from_secs(5)).as_deref(),
            Some("9.9.9")
        );

        let hung = dir.path().join("hung");
        write_binary(&hung, "#!/bin/sh\nexec sleep 30\n");
        let began = Instant::now();
        assert_eq!(probe_version(&hung, Duration::from_millis(300)), None);
        assert!(began.elapsed() < Duration::from_secs(10));

        let failing = dir.path().join("failing");
        write_binary(&failing, "#!/bin/sh\necho 'AskHuman v1.0.0'\nexit 1\n");
        assert_eq!(probe_version(&failing, Duration::from_secs(5)), None);
        assert_eq!(
            probe_version(&dir.path().join("missing"), Duration::from_secs(1)),
            None
        );
    }

    #[cfg(unix)]
    #[test]
    fn lists_each_distinct_binary_once_with_everywhere_it_was_seen() {
        let dir = tempfile::tempdir().unwrap();
        let (a, c) = (dir.path().join("a"), dir.path().join("c"));
        write_binary(&a.join("AskHuman"), "1.0.0");
        write_binary(&c.join("AskHuman"), "2.0.0");
        let b = dir.path().join("b");
        std::fs::create_dir_all(&b).unwrap();
        std::os::unix::fs::symlink(a.join("AskHuman"), b.join("AskHuman")).unwrap();

        let report = inspect(
            &inputs(vec![
                a.clone(),
                b.clone(),
                c.clone(),
                dir.path().join("nowhere"),
                a.clone(),
            ]),
            &read_version,
        );
        assert_eq!(report.copies.len(), 2);
        let first = &report.copies[0];
        assert_eq!(first.version.as_deref(), Some("1.0.0"));
        assert_eq!(first.source, Source::Standalone);
        assert_eq!(first.seen.len(), 2, "a repeated PATH directory counts once");
        assert!(matches!(
            first.seen[0],
            Seen::Path {
                rank: 1,
                link: Link::Direct,
                ..
            }
        ));
        assert!(matches!(
            first.seen[1],
            Seen::Path {
                rank: 2,
                link: Link::Symlink,
                ..
            }
        ));
        assert_eq!(report.copies[1].version.as_deref(), Some("2.0.0"));
        assert!(matches!(
            report.copies[1].seen[0],
            Seen::Path { rank: 3, .. }
        ));
        assert_eq!(report.findings, vec![Finding::MultipleBuilds { count: 2 }]);
    }

    #[cfg(unix)]
    #[test]
    fn identical_content_at_two_places_is_not_a_conflict() {
        let dir = tempfile::tempdir().unwrap();
        let (a, b) = (dir.path().join("a"), dir.path().join("b"));
        write_binary(&a.join("AskHuman"), "1.0.0");
        write_binary(&b.join("AskHuman"), "1.0.0");
        let mut input = inputs(vec![a.clone(), b.clone()]);
        input.current_exe = Some(b.join("AskHuman"));
        let report = inspect(&input, &read_version);
        assert_eq!(report.copies.len(), 2);
        assert!(report.findings.is_empty(), "{:?}", report.findings);
    }

    #[cfg(unix)]
    #[test]
    fn skips_entries_a_shell_would_not_run() {
        let dir = tempfile::tempdir().unwrap();
        let plain = dir.path().join("plain");
        std::fs::create_dir_all(&plain).unwrap();
        std::fs::write(plain.join("AskHuman"), "not executable").unwrap();
        let dangling = dir.path().join("dangling");
        std::fs::create_dir_all(&dangling).unwrap();
        std::os::unix::fs::symlink(dir.path().join("gone"), dangling.join("AskHuman")).unwrap();
        let report = inspect(&inputs(vec![plain, dangling]), &read_version);
        assert!(report.copies.is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn this_binary_losing_to_another_copy_is_shadowed() {
        let dir = tempfile::tempdir().unwrap();
        let (nvm, local) = (dir.path().join("nvm"), dir.path().join("local"));
        write_binary(&nvm.join("AskHuman"), "0.14.5");
        write_binary(&local.join("AskHuman"), "0.15.0");
        let mut input = inputs(vec![nvm.clone(), local.clone()]);
        input.current_exe = Some(local.join("AskHuman"));
        let report = inspect(&input, &read_version);
        let canon = |p: &Path| std::fs::canonicalize(p).unwrap();
        assert_eq!(
            report.findings,
            vec![
                Finding::Shadowed {
                    running: Brief {
                        path: canon(&local.join("AskHuman")),
                        version: Some("0.15.0".into())
                    },
                    first: Brief {
                        path: canon(&nvm.join("AskHuman")),
                        version: Some("0.14.5".into())
                    },
                },
                Finding::MultipleBuilds { count: 2 },
            ]
        );
        assert!(report.copies[1].is_this_process());

        // Running the first copy instead: no shadowing, only the two builds.
        input.current_exe = Some(nvm.join("AskHuman"));
        let report = inspect(&input, &read_version);
        assert_eq!(report.findings, vec![Finding::MultipleBuilds { count: 2 }]);
    }

    #[cfg(unix)]
    #[test]
    fn a_daemon_started_from_another_file_is_reported_but_a_pending_update_is_not() {
        let dir = tempfile::tempdir().unwrap();
        let (a, b) = (dir.path().join("a"), dir.path().join("b"));
        write_binary(&a.join("AskHuman"), "0.15.0");
        write_binary(&b.join("AskHuman"), "0.14.5");
        let mut input = inputs(vec![a.clone()]);
        input.daemon = Some(DaemonBinary {
            exe: b.join("AskHuman"),
            fingerprint: Some(lifecycle::fingerprint_uncached(&b.join("AskHuman"))),
        });
        let report = inspect(&input, &read_version);
        assert!(matches!(
            report.findings.as_slice(),
            [Finding::DaemonElsewhere { .. }]
        ));
        assert!(report.copies.iter().any(|c| c.is_daemon() && !c.on_path()));

        // The daemon runs the first PATH file but started from older content: it updates itself.
        input.daemon = Some(DaemonBinary {
            exe: a.join("AskHuman"),
            fingerprint: Some(Fingerprint { size: 1, hash: 1 }),
        });
        assert!(inspect(&input, &read_version).findings.is_empty());
    }

    #[test]
    fn an_active_hold_is_the_first_finding() {
        let mut input = inputs(Vec::new());
        input.conflict = Some(BinaryConflictInfo {
            since_ms: input.now_ms - 7 * 60_000,
            last_ms: input.now_ms,
            shared: 4,
            client_version: "0.14.5".into(),
            client_protocol_version: 3,
            client_path: "/old/AskHuman".into(),
            client_pid: 4242,
        });
        let report = inspect(&input, &read_version);
        assert!(matches!(report.findings.as_slice(), [Finding::Held(_)]));
        let text = render_text(&report, Lang::En, true);
        assert!(text.contains("v0.14.5 (pid 4242) /old/AskHuman"), "{text}");
        assert!(text.contains("7 min"), "{text}");
        let zh = render_text(&report, Lang::Zh, true);
        assert!(zh.contains("警告") && zh.contains("7 分钟前"), "{zh}");
    }

    #[cfg(unix)]
    #[test]
    fn npm_shim_resolves_to_the_platform_binary_and_a_symlink_into_it_is_the_same_copy() {
        let Some(key) = platform_key() else {
            return; // no npm platform package for this host
        };
        let dir = tempfile::tempdir().unwrap();
        let prefix = dir.path().join("prefix");
        let package = prefix.join("lib/node_modules/askhuman");
        write_binary(&package.join("bin/cli.js"), "#!/usr/bin/env node\n");
        let native = package
            .join("node_modules/@humaninloop")
            .join(key)
            .join("bin")
            .join(entry_name());
        write_binary(&native, "0.15.0");
        std::fs::create_dir_all(prefix.join("bin")).unwrap();
        std::os::unix::fs::symlink(
            package.join("bin/cli.js"),
            prefix.join("bin").join(entry_name()),
        )
        .unwrap();
        // `~/.local/bin/AskHuman` pointing straight at npm's binary.
        let local = dir.path().join("local");
        std::fs::create_dir_all(&local).unwrap();
        std::os::unix::fs::symlink(&native, local.join(entry_name())).unwrap();

        let resolved = resolve(&prefix.join("bin").join(entry_name())).unwrap();
        assert_eq!(resolved.native, std::fs::canonicalize(&native).unwrap());
        assert_eq!(resolved.source, Source::Npm);
        assert_eq!(resolved.link, Link::NpmShim);

        let report = inspect(
            &inputs(vec![prefix.join("bin"), local.clone()]),
            &read_version,
        );
        assert_eq!(report.copies.len(), 1, "both entries run the same file");
        assert_eq!(report.copies[0].seen.len(), 2);
        // One file under two names agrees with itself, so there is nothing to warn about: installing
        // through the link keeps every name on the same build.
        assert!(report.findings.is_empty(), "{:?}", report.findings);

        // npm may hoist the platform package next to `askhuman` instead of nesting it.
        let hoisted = dir.path().join("hoisted");
        let package = hoisted.join("lib/node_modules/askhuman");
        write_binary(&package.join("bin/cli.js"), "#!/usr/bin/env node\n");
        let native = hoisted
            .join("lib/node_modules/@humaninloop")
            .join(key)
            .join("bin")
            .join(entry_name());
        write_binary(&native, "0.15.0");
        let resolved = resolve(&package.join("bin/cli.js")).unwrap();
        assert_eq!(resolved.native, std::fs::canonicalize(&native).unwrap());

        // A shim without its platform package cannot run: it is not a binary to list.
        let broken = dir.path().join("broken/lib/node_modules/askhuman");
        write_binary(&broken.join("bin/cli.js"), "#!/usr/bin/env node\n");
        assert!(resolve(&broken.join("bin/cli.js")).is_none());
    }

    #[cfg(unix)]
    #[test]
    fn warn_only_is_silent_without_findings_and_the_full_report_lists_the_binaries() {
        let dir = tempfile::tempdir().unwrap();
        let only = dir.path().join("only");
        write_binary(&only.join("AskHuman"), "0.15.0");
        let mut input = inputs(vec![only.clone()]);
        input.yields = vec![Yield {
            at_ms: input.now_ms - 60_000,
            trigger: Trigger::Client,
            drained: false,
            daemon_pid: 1,
            daemon_version: "0.15.0".into(),
            from: Fingerprint { size: 1, hash: 1 },
            to: Fingerprint { size: 2, hash: 2 },
            client: Some(restart_guard::Peer {
                version: "0.14.5".into(),
                protocol_version: 3,
                path: "/old/AskHuman".into(),
                pid: 4242,
                fingerprint: Fingerprint { size: 2, hash: 2 },
            }),
        }];
        let report = inspect(&input, &read_version);
        assert!(report.findings.is_empty());
        assert_eq!(render_text(&report, Lang::En, true), "");

        let text = render_text(&report, Lang::En, false);
        assert!(text.contains("ok: no conflicting"), "{text}");
        assert!(text.contains("PATH #1"), "{text}");
        assert!(text.contains("v0.15.0"), "{text}");
        assert!(text.contains("client binary differs"), "{text}");
        assert!(text.contains("v0.14.5 (pid 4242) /old/AskHuman"), "{text}");

        let json = render_json_value(&report, Lang::En);
        assert_eq!(json["copies"][0]["seen"][0]["rank"], 1);
        assert_eq!(json["recentReplacements"][0]["trigger"], "client");
        assert_eq!(json["findings"], serde_json::json!([]));
    }

    #[test]
    fn only_the_last_hour_of_replacements_is_listed() {
        let mut input = inputs(Vec::new());
        let old = Yield {
            at_ms: input.now_ms - RECENT_MS - 1,
            trigger: Trigger::Disk,
            drained: true,
            daemon_pid: 1,
            daemon_version: "0.15.0".into(),
            from: Fingerprint { size: 1, hash: 1 },
            to: Fingerprint { size: 2, hash: 2 },
            client: None,
        };
        let recent = Yield {
            at_ms: input.now_ms - 1000,
            ..old.clone()
        };
        input.yields = vec![old, recent];
        let report = inspect(&input, &read_version);
        assert_eq!(report.recent.len(), 1);
    }

    #[test]
    fn a_dev_instance_skips_the_path_comparison_but_still_reports_a_hold() {
        let mut report = Report {
            path_scan_skipped: true,
            copies: Vec::new(),
            findings: Vec::new(),
            recent: Vec::new(),
            now_ms: 10_000_000,
        };
        assert_eq!(render_text(&report, Lang::En, true), "");
        let full = render_text(&report, Lang::En, false);
        assert!(full.contains("Dev Instance"), "{full}");
        assert!(!full.contains("ok:"), "nothing was compared: {full}");
        assert_eq!(
            render_json_value(&report, Lang::En)["pathScanSkipped"],
            true
        );

        report.findings = vec![Finding::Held(BinaryConflictInfo {
            since_ms: 9_000_000,
            last_ms: 10_000_000,
            shared: 2,
            client_version: "0.15.0".into(),
            client_protocol_version: 3,
            client_path: "/other/AskHuman".into(),
            client_pid: 9,
        })];
        let warned = render_text(&report, Lang::En, true);
        assert!(warned.contains("/other/AskHuman"), "{warned}");
        assert_eq!(
            render_json_value(&report, Lang::En)["findings"][0]["kind"],
            "held"
        );
    }
}
