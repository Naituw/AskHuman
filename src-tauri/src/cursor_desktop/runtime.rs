//! Current-user process discovery, authenticated bootstrap, and daemon-owned reconciliation.

use super::{now_ms, status_path, Status, WindowStatus};
use anyhow::{bail, Context, Result};
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet},
    io::Read,
    os::unix::fs::{MetadataExt, PermissionsExt},
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
    time::{Duration, Instant},
};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::UnixStream,
    process::Command,
    time::timeout,
};
use tokio_tungstenite::{connect_async, tungstenite::Message};

const RENDERER: &str = include_str!("renderer.js");
const PREFLIGHT: &str = include_str!("preflight.js");
const BRIDGE: &str = include_str!("bridge.js");
const PORT: u16 = 9229;

pub(super) fn adapter_revision() -> String {
    static REVISION: OnceLock<String> = OnceLock::new();
    REVISION
        .get_or_init(|| {
            let mut hash = Sha256::new();
            hash.update(BRIDGE);
            hash.update(RENDERER);
            hash.update(PREFLIGHT);
            format!("{:x}", hash.finalize())[..16].into()
        })
        .clone()
}

fn scan_inspector_fuse(mut reader: impl Read) -> Result<bool> {
    const SENTINEL: &[u8] = b"dL7pKGdnNz796PbbjQWNKmHXBZaB9tsX";
    let mut found = false;
    let mut tail = Vec::new();
    let mut chunk = [0_u8; 64 * 1024];
    let mut total = 0_usize;
    loop {
        let length = reader.read(&mut chunk)?;
        total += length;
        if total > 512 * 1024 * 1024 {
            bail!("Cursor Electron framework exceeds the inspection limit");
        }
        tail.extend_from_slice(&chunk[..length]);
        for (index, bytes) in tail.windows(SENTINEL.len()).enumerate() {
            if bytes != SENTINEL {
                continue;
            }
            let header = index + SENTINEL.len();
            if header + 6 > tail.len() {
                if length == 0 {
                    return Ok(false);
                }
                continue;
            }
            if tail[header] != 1
                || tail[header + 1] < 4
                || tail[header + 1] > 64
                || tail[header + 2 + 3] != b'1'
            {
                return Ok(false);
            }
            found = true;
        }
        if length == 0 {
            return Ok(found);
        }
        let keep = tail.len().saturating_sub(SENTINEL.len() + 6);
        tail.drain(..keep);
    }
}

fn inspector_fuse_enabled(package: &Path) -> Result<bool> {
    type CachedFuse = (u64, u64, std::time::SystemTime, bool);
    static CACHE: OnceLock<Mutex<HashMap<PathBuf, CachedFuse>>> = OnceLock::new();
    let app = package
        .ancestors()
        .nth(4)
        .context("Invalid Cursor package path")?;
    let framework =
        app.join("Contents/Frameworks/Electron Framework.framework/Versions/A/Electron Framework");
    let metadata = std::fs::metadata(&framework)?;
    let fingerprint = (metadata.ino(), metadata.len(), metadata.modified()?);
    let cache = CACHE.get_or_init(Mutex::default);
    if let Some(&(inode, length, modified, enabled)) = cache.lock().unwrap().get(&framework) {
        if (inode, length, modified) == fingerprint {
            return Ok(enabled);
        }
    }
    let enabled = scan_inspector_fuse(std::fs::File::open(&framework)?)?;
    cache.lock().unwrap().insert(
        framework,
        (fingerprint.0, fingerprint.1, fingerprint.2, enabled),
    );
    Ok(enabled)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Process {
    pid: u32,
    started: String,
    executable: PathBuf,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Manifest {
    process: Process,
    socket: PathBuf,
    token: String,
    owner: String,
}

#[derive(Default)]
struct Retry {
    attempts: u32,
    next: Option<Instant>,
    error: Option<String>,
}

impl Retry {
    fn fail(&mut self, error: &anyhow::Error) {
        self.attempts = self.attempts.saturating_add(1);
        self.next = Some(
            Instant::now()
                + Duration::from_secs(2_u64.pow(self.attempts.saturating_sub(1).min(5)).min(30)),
        );
        self.error = Some(error.to_string().chars().take(240).collect());
    }

    fn ready(&self) -> bool {
        self.next.is_none_or(|at| Instant::now() >= at)
    }
}

fn owner() -> String {
    let digest = Sha256::digest(crate::paths::config_dir().to_string_lossy().as_bytes());
    format!("{:x}", digest)[..12].into()
}

fn manifests_dir() -> PathBuf {
    crate::paths::state_dir().join("cursor-desktop")
}

fn manifest_path(pid: u32) -> PathBuf {
    manifests_dir().join(format!("{pid}.json"))
}

fn socket_dir() -> PathBuf {
    PathBuf::from("/tmp")
        .join(format!("ah-cursor-{}", unsafe { libc::geteuid() }))
        .join(owner())
}

fn private_dir(path: &Path) -> Result<()> {
    if !path.exists() {
        let mut builder = std::fs::DirBuilder::new();
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700).create(path)?;
    }
    let meta = std::fs::symlink_metadata(path)?;
    if !meta.is_dir() || meta.uid() != unsafe { libc::geteuid() } {
        bail!("Cursor bridge directory has an unexpected owner or type");
    }
    if meta.mode() & 0o777 != 0o700 {
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

fn prepare_dirs() -> Result<()> {
    // config/state are managed by AskHuman; the socket's /tmp parents are checked individually.
    std::fs::create_dir_all(crate::paths::state_dir())?;
    private_dir(&manifests_dir())?;
    let sockets = socket_dir();
    private_dir(sockets.parent().context("Missing socket parent")?)?;
    private_dir(&sockets)?;
    Ok(())
}

fn write_private(path: &Path, value: &impl Serialize) -> Result<()> {
    let parent = path.parent().context("Missing state directory")?;
    private_dir(parent)?;
    let mut file = tempfile::NamedTempFile::new_in(parent)?;
    use std::io::Write;
    file.as_file()
        .set_permissions(std::fs::Permissions::from_mode(0o600))?;
    serde_json::to_writer(&mut file, value)?;
    file.flush()?;
    file.persist(path).map_err(|e| e.error)?;
    Ok(())
}

async fn output(program: &str, args: &[&str]) -> Result<String> {
    let output = timeout(
        Duration::from_secs(3),
        Command::new(program).args(args).kill_on_drop(true).output(),
    )
    .await
    .context("Process inspection timed out")??;
    if !(output.status.success()
        || (program == "/usr/sbin/lsof"
            && output.status.code() == Some(1)
            && output.stderr.is_empty()))
    {
        bail!("Process inspection failed");
    }
    if output.stdout.len() > 4 * 1024 * 1024 {
        bail!("Process inspection output exceeds its limit");
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn parse_processes(text: &str, uid: u32) -> Vec<Process> {
    text.lines()
        .filter_map(|line| {
            let parts: Vec<_> = line.split_whitespace().collect();
            if parts.len() < 8 || parts[1].parse::<u32>().ok()? != uid {
                return None;
            }
            let executable = PathBuf::from(parts[7..].join(" "));
            if !executable.is_absolute()
                || !executable.ends_with("Cursor.app/Contents/MacOS/Cursor")
            {
                return None;
            }
            Some(Process {
                pid: parts[0].parse().ok()?,
                started: parts[2..7].join(" "),
                executable,
            })
        })
        .collect()
}

async fn discover() -> Result<Vec<Process>> {
    let text = output("/bin/ps", &["-axo", "pid=,uid=,lstart=,comm="]).await?;
    Ok(parse_processes(&text, unsafe { libc::geteuid() }))
}

async fn inspector_owners() -> Result<HashSet<u32>> {
    let text = output(
        "/usr/sbin/lsof",
        &["-nP", "-t", "-iTCP:9229", "-sTCP:LISTEN"],
    )
    .await?;
    Ok(text.lines().filter_map(|line| line.parse().ok()).collect())
}

async fn inspect_process(process: &Process) -> Result<PathBuf> {
    if !discover().await?.contains(process) {
        bail!("Cursor process identity changed");
    }
    let app = process
        .executable
        .parent()
        .and_then(Path::parent)
        .and_then(Path::parent)
        .context("Invalid Cursor bundle path")?;
    let info = app.join("Contents/Info.plist");
    let id = output(
        "/usr/libexec/PlistBuddy",
        &["-c", "Print :CFBundleIdentifier", &info.to_string_lossy()],
    )
    .await?;
    if id.trim() != "com.todesktop.230313mzl4w4u92" {
        bail!("Process is not the supported Cursor Desktop application");
    }
    let package = app.join("Contents/Resources/app/package.json");
    if !package.is_file() {
        bail!("Cursor Electron runtime is missing");
    }
    Ok(package)
}

async fn bridge_request(manifest: &Manifest, op: &str) -> Result<Value> {
    if manifest.owner != owner()
        || manifest.socket != socket_dir().join(format!("p{}.sock", manifest.process.pid))
        || manifest.token.len() != 64
    {
        bail!("Cursor bridge manifest identity is invalid");
    }
    let meta = std::fs::symlink_metadata(&manifest.socket)?;
    use std::os::unix::fs::FileTypeExt;
    if !meta.file_type().is_socket()
        || meta.uid() != unsafe { libc::geteuid() }
        || meta.mode() & 0o777 != 0o600
    {
        bail!("Cursor bridge endpoint permissions are invalid");
    }
    timeout(Duration::from_secs(8), async {
        let mut socket = UnixStream::connect(&manifest.socket).await?;
        let mut request = serde_json::to_vec(&json!({"token": manifest.token, "op": op}))?;
        request.push(b'\n');
        socket.write_all(&request).await?;
        let mut reader = BufReader::new(socket);
        let mut bytes = Vec::new();
        // A capped reader prevents a corrupted/foreign endpoint from allocating an arbitrary line.
        tokio::io::AsyncReadExt::take(&mut reader, 128 * 1024)
            .read_until(b'\n', &mut bytes)
            .await?;
        if bytes.last() != Some(&b'\n') {
            bail!("Cursor bridge response exceeds its limit");
        }
        let response: Value = serde_json::from_slice(&bytes)?;
        if response["ok"] != true {
            bail!("Cursor bridge rejected the operation");
        }
        Ok(response["result"].clone())
    })
    .await
    .context("Cursor bridge request timed out")?
}

fn bootstrap_source(manifest: &Manifest, package: &Path, close_inspector: bool) -> String {
    let config = json!({
        "socket": manifest.socket, "owner": manifest.owner,
        "token": manifest.token, "appPackage": package,
        "revision": adapter_revision(),
    });
    let renderer = RENDERER
        .replacen("__ASKHUMAN_PREFLIGHT__", PREFLIGHT, 1)
        .replace("__ASKHUMAN_OWNER__", &json!(manifest.owner).to_string())
        .replace(
            "__ASKHUMAN_REVISION__",
            &json!(adapter_revision()).to_string(),
        );
    let bridge = BRIDGE
        .replacen("__ASKHUMAN_RENDERER__", &json!(renderer).to_string(), 1)
        .replacen("__ASKHUMAN_CONFIG__", &config.to_string(), 1);
    format!(
        "(async()=>{{try{{return await ({bridge});}}finally{{if({close_inspector})setTimeout(()=>process.getBuiltinModule('inspector').close(),100);}}}})()"
    )
}

async fn inspector_url() -> Result<String> {
    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(2))
        .build()?;
    let mut response = client
        .get(format!("http://127.0.0.1:{PORT}/json/list"))
        .send()
        .await?
        .error_for_status()?;
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        if bytes.len() + chunk.len() > 128 * 1024 {
            bail!("Inspector target list exceeds its limit");
        }
        bytes.extend_from_slice(&chunk);
    }
    let targets: Vec<Value> = serde_json::from_slice(&bytes)?;
    let url = targets
        .iter()
        .find(|t| t["type"] == "node")
        .and_then(|t| t["webSocketDebuggerUrl"].as_str())
        .context("Cursor Node inspector target is unavailable")?;
    let parsed = reqwest::Url::parse(url)?;
    if parsed.scheme() != "ws"
        || parsed.host_str() != Some("127.0.0.1")
        || parsed.port() != Some(PORT)
        || parsed.username() != ""
        || parsed.password().is_some()
    {
        bail!("Inspector target is not the expected local endpoint");
    }
    Ok(url.into())
}

type Inspector =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn evaluate(socket: &mut Inspector, id: u32, expression: &str) -> Result<Value> {
    timeout(Duration::from_secs(10), async {
        socket
            .send(Message::Text(
                json!({"id": id, "method": "Runtime.evaluate", "params": {
                    "expression": expression, "awaitPromise": true, "returnByValue": true,
                }})
                .to_string()
                .into(),
            ))
            .await?;
        while let Some(message) = socket.next().await {
            let message = message?;
            if message.len() > 256 * 1024 {
                bail!("Inspector response exceeds its limit");
            }
            let Message::Text(text) = message else {
                continue;
            };
            let response: Value = serde_json::from_str(&text)?;
            if response["id"] != id {
                continue;
            }
            if response.get("error").is_some()
                || response["result"].get("exceptionDetails").is_some()
            {
                bail!("Cursor rejected the adapter bootstrap");
            }
            return Ok(response["result"]["result"]["value"].clone());
        }
        bail!("Cursor inspector disconnected");
    })
    .await
    .context("Cursor inspector command timed out")?
}

async fn bootstrap(manifest: &Manifest) -> Result<Value> {
    let package = inspect_process(&manifest.process).await?;
    let owners = inspector_owners().await?;
    if owners.iter().any(|pid| *pid != manifest.process.pid) {
        bail!("Local inspector port 9229 is already in use by another process");
    }
    let opened = owners.is_empty();
    let result = async {
        if opened {
            if !inspector_fuse_enabled(&package)? {
                bail!("Cursor does not allow temporary Inspector attachment");
            }
            // Recheck immediately before signaling; never send this signal to a helper or CLI.
            inspect_process(&manifest.process).await?;
            if unsafe { libc::kill(manifest.process.pid as i32, libc::SIGUSR1) } != 0 {
                bail!("Could not open Cursor's temporary inspector");
            }
            for _ in 0..20 {
                if inspector_owners().await?.contains(&manifest.process.pid) {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }
        if inspector_owners().await? != HashSet::from([manifest.process.pid]) {
            bail!("Cursor's temporary inspector did not become available");
        }
        bootstrap_connected(manifest, &package, opened).await
    }
    .await;
    if opened {
        if let Err(error) = close_owned_inspector(manifest.process.pid).await {
            return Err(error.context("Could not close Cursor's temporary inspector"));
        }
    }
    result
}

async fn bootstrap_connected(manifest: &Manifest, package: &Path, opened: bool) -> Result<Value> {
    let (mut socket, _) = timeout(
        Duration::from_secs(3),
        connect_async(inspector_url().await?),
    )
    .await
    .context("Cursor inspector connection timed out")??;
    let mut verified = false;
    let result = async {
        if evaluate(&mut socket, 1, "process.pid").await? != manifest.process.pid {
            bail!("Inspector process identity does not match Cursor");
        }
        verified = true;
        evaluate(&mut socket, 2, &bootstrap_source(manifest, package, opened)).await
    }
    .await;
    if opened && verified {
        // Also try cleanup if evaluating the wrapper failed before its finally block ran.
        let _ = evaluate(
            &mut socket,
            3,
            "setTimeout(()=>process.getBuiltinModule('inspector').close(),100);true",
        )
        .await;
    }
    let _ = socket.close(None).await;
    result
}

async fn close_owned_inspector(pid: u32) -> Result<()> {
    // The successful bootstrap schedules closure itself. Give that a bounded chance before
    // opening a recovery connection for failures before the script could enter its finally.
    for _ in 0..5 {
        let owners = inspector_owners().await?;
        if owners.is_empty() {
            return Ok(());
        }
        if owners != HashSet::from([pid]) {
            bail!("Inspector ownership changed during cleanup");
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let (mut socket, _) = timeout(
        Duration::from_secs(3),
        connect_async(inspector_url().await?),
    )
    .await
    .context("Inspector cleanup connection timed out")??;
    let result = async {
        if evaluate(&mut socket, 1, "process.pid").await? != pid {
            bail!("Inspector cleanup process identity changed");
        }
        evaluate(
            &mut socket,
            2,
            "setTimeout(()=>process.getBuiltinModule('inspector').close(),100);true",
        )
        .await?;
        Ok(())
    }
    .await;
    let _ = socket.close(None).await;
    result?;
    for _ in 0..10 {
        if !inspector_owners().await?.contains(&pid) {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    bail!("Temporary inspector remained open");
}

fn load_manifests() -> HashMap<u32, Manifest> {
    std::fs::read_dir(manifests_dir())
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            let file_pid = path
                .file_name()?
                .to_str()?
                .strip_suffix(".json")?
                .parse::<u32>()
                .ok()?;
            let meta = std::fs::symlink_metadata(&path).ok()?;
            if !meta.is_file() || meta.len() > 8192 || meta.uid() != unsafe { libc::geteuid() } {
                return None;
            }
            let manifest: Manifest = serde_json::from_slice(&std::fs::read(path).ok()?).ok()?;
            (manifest.owner == owner() && manifest.process.pid == file_pid)
                .then_some((manifest.process.pid, manifest))
        })
        .collect()
}

pub(super) fn has_managed_bridges() -> bool {
    !load_manifests().is_empty()
}

fn new_manifest(process: Process) -> Manifest {
    Manifest {
        socket: socket_dir().join(format!("p{}.sock", process.pid)),
        process,
        token: format!(
            "{}{}",
            uuid::Uuid::new_v4().simple(),
            uuid::Uuid::new_v4().simple()
        ),
        owner: owner(),
    }
}

fn remove_manifest(manifest: &Manifest, remove_socket: bool) {
    let _ = std::fs::remove_file(manifest_path(manifest.process.pid));
    if remove_socket
        && manifest.socket == socket_dir().join(format!("p{}.sock", manifest.process.pid))
    {
        let _ = std::fs::remove_file(&manifest.socket);
    }
}

pub(super) async fn run() {
    let mut manifests = load_manifests();
    let mut retries = HashMap::<u32, Retry>::new();
    let mut last_error = None;
    loop {
        let config = crate::config::AppConfig::load_without_secrets();
        let enabled = super::active(&config.cursor_desktop);
        let mut status = Status {
            supported: true,
            adapter_version: env!("CARGO_PKG_VERSION").into(),
            adapter_revision: adapter_revision(),
            preference_enabled: config
                .cursor_desktop
                .suppress_background_completion_follow_up,
            enabled,
            phase: if enabled { "waiting" } else { "disabled" }.into(),
            daemon_pid: std::process::id(),
            updated_at: now_ms(),
            ..Status::default()
        };
        let result = reconcile(enabled, &mut manifests, &mut retries, &mut status).await;
        if let Err(error) = result {
            status.error = Some(error.to_string().chars().take(240).collect());
            status.phase = "error".into();
        }
        if status.error != last_error {
            if let Some(error) = &status.error {
                eprintln!("[askhuman-daemon] Cursor Desktop attachment: {error}");
            }
            last_error = status.error.clone();
        }
        status.updated_at = now_ms();
        if enabled || status_path().exists() {
            let _ = write_private(&status_path(), &status);
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
}

async fn reconcile(
    enabled: bool,
    manifests: &mut HashMap<u32, Manifest>,
    retries: &mut HashMap<u32, Retry>,
    status: &mut Status,
) -> Result<()> {
    if !enabled && manifests.is_empty() {
        return Ok(());
    }
    prepare_dirs()?;
    let processes = discover().await?;
    let identities: HashMap<_, _> = processes.iter().map(|p| (p.pid, p)).collect();
    for pid in manifests.keys().copied().collect::<Vec<_>>() {
        let manifest = &manifests[&pid];
        if identities.get(&pid).copied() != Some(&manifest.process) {
            // A PID reused by a new live process must not have its endpoint unlinked.
            let process_alive = unsafe { libc::kill(pid as i32, 0) } == 0;
            remove_manifest(manifest, !process_alive);
            manifests.remove(&pid);
            retries.remove(&pid);
        } else if !enabled {
            let result = bridge_request(manifest, "restore").await;
            if result.as_ref().is_ok_and(|value| value["restored"] == true)
                || !manifest.socket.exists()
            {
                remove_manifest(manifest, false);
                manifests.remove(&pid);
                retries.remove(&pid);
            } else {
                status.error = Some("Could not restore a Cursor completion adapter; reload the affected window or close Cursor".into());
                status.phase = "error".into();
            }
        }
    }
    if !enabled {
        return Ok(());
    }
    status.processes = processes.len();
    let mut all_active = !processes.is_empty();
    for process in processes {
        let manifest = manifests
            .entry(process.pid)
            .or_insert_with(|| new_manifest(process));
        let retry = retries.entry(manifest.process.pid).or_default();
        if !retry.ready() {
            all_active = false;
            status.error = retry.error.clone();
            continue;
        }
        let attached = async {
            let value = match bridge_request(manifest, "status").await {
                Ok(value)
                    if value["adapterRevision"] == adapter_revision()
                        && value["enabled"] == true =>
                {
                    Ok(value)
                }
                Ok(value) => {
                    if value["pid"] != manifest.process.pid || value["protocol"] != 1 {
                        bail!("Cursor bridge returned an unexpected process identity");
                    }
                    let restored = bridge_request(manifest, "restore").await?;
                    if restored["restored"] != true {
                        bail!(
                        "Could not replace the previous Cursor adapter; reload the affected window"
                    );
                    }
                    for _ in 0..20 {
                        if !manifest.socket.exists() {
                            break;
                        }
                        tokio::time::sleep(Duration::from_millis(50)).await;
                    }
                    if manifest.socket.exists() {
                        bail!("Previous Cursor bridge is still closing");
                    }
                    bootstrap(manifest).await
                }
                Err(_) => {
                    // Persist before bootstrap so an unknown outcome can be recovered with the same token.
                    write_private(&manifest_path(manifest.process.pid), manifest)?;
                    bootstrap(manifest).await
                }
            }?;
            if value["pid"] != manifest.process.pid
                || value["protocol"] != 1
                || value["enabled"] != true
                || value["adapterRevision"] != adapter_revision()
            {
                bail!("Cursor bridge returned an unexpected identity or adapter version");
            }
            let windows: Vec<WindowStatus> = serde_json::from_value(value["windows"].clone())?;
            Ok::<_, anyhow::Error>((value, windows))
        }
        .await;
        match attached {
            Ok((value, windows)) => {
                *retry = Retry::default();
                if let Some(version) = value["version"].as_str() {
                    if !status.cursor_versions.iter().any(|known| known == version) {
                        status.cursor_versions.push(version.into());
                    }
                }
                all_active &= super::windows_verified(&windows);
                for mut window in windows {
                    window.pid = manifest.process.pid;
                    status.windows.push(window);
                }
            }
            Err(error) => {
                all_active = false;
                retry.fail(&error);
                status.error = retry.error.clone();
            }
        }
    }
    status.active = all_active;
    status.phase = if status.error.is_some() || status.windows.iter().any(|w| w.phase == "error") {
        "error"
    } else if all_active {
        "active"
    } else if status.processes == 0 || status.windows.is_empty() {
        "waiting"
    } else {
        "attaching"
    }
    .into();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signaling_requires_a_known_enabled_inspector_fuse() {
        let wire = b"dL7pKGdnNz796PbbjQWNKmHXBZaB9tsX\x01\x09101100011";
        assert!(scan_inspector_fuse(&wire[..]).unwrap());
        let mut disabled = wire.to_vec();
        disabled[32 + 2 + 3] = b'0';
        assert!(!scan_inspector_fuse(&disabled[..]).unwrap());
        let mut mixed = wire.to_vec();
        mixed.extend_from_slice(&disabled);
        assert!(!scan_inspector_fuse(&mixed[..]).unwrap());
        assert!(!scan_inspector_fuse(&wire[..33]).unwrap());
        assert!(!scan_inspector_fuse(&b"unknown runtime"[..]).unwrap());
        let mut boundary = vec![0_u8; 65520];
        boundary.extend_from_slice(wire);
        assert!(scan_inspector_fuse(&boundary[..]).unwrap());
    }

    #[test]
    fn discovery_accepts_only_current_user_desktop_main_processes() {
        let processes = parse_processes(
            "12 501 Tue Oct 13 08:00:00 2026 /Applications/Cursor.app/Contents/MacOS/Cursor\n\
             13 502 Tue Oct 13 08:00:00 2026 /Applications/Cursor.app/Contents/MacOS/Cursor\n\
             14 501 Tue Oct 13 08:00:00 2026 /Applications/Cursor.app/Contents/MacOS/Cursor Helper\n\
             15 501 Tue Oct 13 08:00:00 2026 /Users/me/.local/bin/cursor-agent\n\
             16 501 Tue Oct 13 08:00:00 2026 /Users/me/My Apps/Cursor.app/Contents/MacOS/Cursor",
            501,
        );
        assert_eq!(
            processes.iter().map(|p| p.pid).collect::<Vec<_>>(),
            [12, 16]
        );
        assert_eq!(processes[0].started, "Tue Oct 13 08:00:00 2026");
    }

    #[test]
    fn bootstrap_quotes_paths_and_keeps_the_renderer_owner_exact() {
        let manifest = new_manifest(Process {
            pid: 42,
            started: "a".into(),
            executable: "/Applications/Cursor.app/Contents/MacOS/Cursor".into(),
        });
        let source = bootstrap_source(&manifest, Path::new("/tmp/a'\"b/package.json"), true);
        assert!(!source.contains("__ASKHUMAN_CONFIG__"));
        assert!(!source.contains("__ASKHUMAN_RENDERER__"));
        assert!(!source.contains("__ASKHUMAN_OWNER__"));
        assert!(!source.contains("__ASKHUMAN_PREFLIGHT__"));
        assert!(source.contains("finally"));
        assert!(manifest.socket.as_os_str().len() < 100);
        assert_eq!(manifest.token.len(), 64);
    }

    #[tokio::test]
    async fn a_foreign_manifest_is_rejected_before_connecting() {
        let mut manifest = new_manifest(Process {
            pid: 42,
            started: "a".into(),
            executable: "/Applications/Cursor.app/Contents/MacOS/Cursor".into(),
        });
        manifest.socket = "/tmp/foreign.sock".into();
        assert!(bridge_request(&manifest, "restore").await.is_err());
    }

    #[tokio::test]
    #[ignore = "requires a user-authorized isolated Cursor Desktop PID; never starts a model"]
    async fn real_cursor_bridge_contract_when_pid_is_provided() {
        let pid: u32 = std::env::var("ASKHUMAN_CURSOR_TEST_PID")
            .unwrap()
            .parse()
            .unwrap();
        let process = discover()
            .await
            .unwrap()
            .into_iter()
            .find(|p| p.pid == pid)
            .unwrap();
        assert!(
            inspector_owners().await.unwrap().is_empty(),
            "Test requires a free inspector port"
        );
        prepare_dirs().unwrap();
        let manifest = new_manifest(process);
        write_private(&manifest_path(pid), &manifest).unwrap();
        let result = async {
            bootstrap(&manifest).await?;
            let deadline = Instant::now() + Duration::from_secs(20);
            loop {
                let status = bridge_request(&manifest, "status").await?;
                let windows = status["windows"].as_array().context("Missing windows")?;
                if !windows.is_empty()
                    && windows.iter().all(|w| {
                        w["phase"] == "active"
                            && w["verified"] == true
                            && w["checks"].as_u64().unwrap_or(0) >= 6
                    })
                {
                    println!(
                        "Attached native Cursor windows: {}",
                        serde_json::to_string(windows)?
                    );
                    break;
                }
                if Instant::now() >= deadline {
                    bail!(
                        "Native Cursor attachment failed: {}",
                        serde_json::to_string(windows)?
                    );
                }
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
            let mut wrong = manifest.clone();
            wrong.token = "0".repeat(64);
            anyhow::ensure!(bridge_request(&wrong, "restore").await.is_err());
            tokio::time::sleep(Duration::from_millis(300)).await;
            anyhow::ensure!(!inspector_owners().await?.contains(&pid));
            // An external owned-process harness may reload the window during this bounded hold.
            let hold_ms = std::env::var("ASKHUMAN_CURSOR_TEST_HOLD_MS")
                .ok()
                .and_then(|value| value.parse::<u64>().ok())
                .unwrap_or(0)
                .min(15_000);
            if hold_ms > 0 {
                println!("Ready for external reload");
                tokio::time::sleep(Duration::from_millis(hold_ms)).await;
                let status = bridge_request(&manifest, "status").await?;
                let windows = status["windows"].as_array().context("Missing windows")?;
                anyhow::ensure!(!windows.is_empty());
                anyhow::ensure!(windows
                    .iter()
                    .all(|window| { window["phase"] == "active" && window["verified"] == true }));
                println!(
                    "Verified after external reload: {}",
                    serde_json::to_string(windows)?
                );
            }
            Ok::<_, anyhow::Error>(())
        }
        .await;
        let restored = bridge_request(&manifest, "restore").await;
        if restored
            .as_ref()
            .is_ok_and(|value| value["restored"] == true)
        {
            remove_manifest(&manifest, false);
        }
        result.unwrap();
        assert_eq!(restored.unwrap()["restored"], true);
    }
}
