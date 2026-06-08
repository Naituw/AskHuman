//! Daemon 生命周期支撑：二进制指纹、运行元信息（daemon.json）、单实例锁（flock）。

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

/// 可执行文件指纹：用 size + 内容哈希判定「盘上二进制内容是否变化」。
///
/// 刻意**不取 mtime**：同一份字节复制到不同位置、或重装同版本后 mtime 会变，但内容相同，
/// 应视为同一实例。若把 mtime 计入指纹，多处安装会互相误判为「二进制换了」从而反复重启
/// daemon（ping-pong）。改用内容哈希后：同内容→同指纹（与路径/mtime 无关）；内容变化
/// （dev 改码重编）→哈希变→仍会自动换新。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Fingerprint {
    pub size: u64,
    pub hash: u64,
}

/// 计算当前可执行文件的指纹（解析失败回退到全 0）。
///
/// 哈希基于文件内容，因此每次调用都要读盘；为避免每次 CLI 调用都重哈希几 MB 的二进制，
/// 按 (路径, mtime, size) 做持久缓存（`~/.askhuman/binhash.json`）：命中即复用已算哈希，
/// 把稳态开销降到一次。缓存仅为加速，任何缺失/损坏都会回退到重新计算。
pub fn current_fingerprint() -> Fingerprint {
    let zero = Fingerprint { size: 0, hash: 0 };
    let Ok(path) = std::env::current_exe() else {
        return zero;
    };
    let Ok(meta) = std::fs::metadata(&path) else {
        return zero;
    };
    let size = meta.len();
    let mtime_ms = mtime_ms_of(&meta);
    if let Some(hash) = cached_hash(&path, size, mtime_ms) {
        return Fingerprint { size, hash };
    }
    let hash = hash_file(&path).unwrap_or(0);
    store_cached_hash(&path, size, mtime_ms, hash);
    Fingerprint { size, hash }
}

/// 文件 mtime（毫秒）；解析失败回退 0。仅用作内容哈希缓存的键。
fn mtime_ms_of(meta: &std::fs::Metadata) -> u64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// 流式读取文件并计算内容哈希。
///
/// 用标准库 `DefaultHasher`（SipHash，固定 key），跨进程/跨次运行结果一致——这点很关键，
/// 因为 client 与 daemon 必须对同一份字节得到相同哈希。逐块 `write` 与一次性 write 等价
/// （不带长度前缀），分块大小不影响结果。
fn hash_file(path: &Path) -> std::io::Result<u64> {
    use std::hash::Hasher;
    use std::io::Read;
    let mut f = std::fs::File::open(path)?;
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.write(&buf[..n]);
    }
    Ok(hasher.finish())
}

/// 内容哈希缓存文件 `~/.askhuman/binhash.json`（路径 → 该路径上次算过的内容哈希）。
fn hash_cache_path() -> PathBuf {
    crate::paths::config_dir().join("binhash.json")
}

/// 单条缓存：某路径在给定 (mtime, size) 下的内容哈希。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct HashCacheEntry {
    mtime_ms: u64,
    size: u64,
    hash: u64,
}

type HashCache = HashMap<String, HashCacheEntry>;

fn read_hash_cache() -> HashCache {
    std::fs::read(hash_cache_path())
        .ok()
        .and_then(|d| serde_json::from_slice(&d).ok())
        .unwrap_or_default()
}

/// 缓存命中（同路径、mtime+size 未变）则返回已算哈希；否则 `None`（需重算）。
fn cached_hash(path: &Path, size: u64, mtime_ms: u64) -> Option<u64> {
    let map = read_hash_cache();
    let e = map.get(path.to_string_lossy().as_ref())?;
    (e.size == size && e.mtime_ms == mtime_ms).then_some(e.hash)
}

/// 写回缓存（best-effort）：原子落盘；不存在配置目录则跳过（不为缓存而创建目录，避免在
/// 测试 / 首次运行时污染用户目录）。无变化则不写。
fn store_cached_hash(path: &Path, size: u64, mtime_ms: u64, hash: u64) {
    let cache_path = hash_cache_path();
    let Some(dir) = cache_path.parent() else {
        return;
    };
    if !dir.exists() {
        return;
    }
    let key = path.to_string_lossy().into_owned();
    let mut map = read_hash_cache();
    if let Some(e) = map.get(&key) {
        if e.size == size && e.mtime_ms == mtime_ms && e.hash == hash {
            return;
        }
    }
    map.insert(key, HashCacheEntry { mtime_ms, size, hash });
    let Ok(data) = serde_json::to_vec(&map) else {
        return;
    };
    let tmp = cache_path.with_extension("json.tmp");
    if std::fs::write(&tmp, &data).is_ok() {
        let _ = std::fs::rename(&tmp, &cache_path);
    }
}

/// 单实例锁文件 `~/.askhuman/daemon.lock`。
pub fn lock_path() -> PathBuf {
    crate::paths::config_dir().join("daemon.lock")
}

/// 运行元信息文件 `~/.askhuman/daemon.json`。
pub fn meta_path() -> PathBuf {
    crate::paths::config_dir().join("daemon.json")
}

/// 运行日志 `~/.askhuman/daemon.log`。
pub fn log_path() -> PathBuf {
    crate::paths::config_dir().join("daemon.log")
}

/// Daemon 运行元信息（落 daemon.json，供调试/排查）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DaemonMeta {
    pub pid: u32,
    pub version: String,
    pub protocol_version: u32,
    pub started_at: u64,
    pub socket: String,
    pub fingerprint: Fingerprint,
}

pub fn write_meta(meta: &DaemonMeta) -> std::io::Result<()> {
    if let Some(dir) = meta_path().parent() {
        std::fs::create_dir_all(dir)?;
    }
    let data = serde_json::to_vec_pretty(meta)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    std::fs::write(meta_path(), data)
}

/// 持有期间代表「本进程为唯一 Daemon」。Drop（文件关闭）时锁自动释放。
#[cfg(unix)]
pub struct LockGuard {
    _file: std::fs::File,
}

/// 尝试获取单实例锁（非阻塞）。
/// - `Ok(Some(guard))`：成功，本进程是唯一 Daemon。
/// - `Ok(None)`：已有其它 Daemon 持锁。
/// - `Err`：其它 IO 错误。
#[cfg(unix)]
pub fn acquire_lock() -> std::io::Result<Option<LockGuard>> {
    use std::os::unix::io::AsRawFd;
    if let Some(dir) = lock_path().parent() {
        std::fs::create_dir_all(dir)?;
    }
    let file = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(false)
        .open(lock_path())?;
    let rc = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    if rc != 0 {
        let err = std::io::Error::last_os_error();
        // 已被其它进程持有（EWOULDBLOCK 与 EAGAIN 在各 Unix 上同值）。
        if err.raw_os_error() == Some(libc::EWOULDBLOCK) {
            return Ok(None);
        }
        return Err(err);
    }
    Ok(Some(LockGuard { _file: file }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fingerprint_reflects_current_exe() {
        // 测试可执行文件存在 → size 非 0，且两次取值一致（盘上未变）。
        let a = current_fingerprint();
        let b = current_fingerprint();
        assert!(a.size > 0);
        assert_eq!(a, b);
    }

    #[test]
    fn hash_is_content_identity_not_path_or_mtime() {
        use std::io::Write;
        let dir = std::env::temp_dir().join(format!("ah-fp-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let bytes = b"AskHuman binary content sample";

        // 同内容、不同路径 → 同哈希（与路径无关）。
        let p1 = dir.join("a/AskHuman");
        let p2 = dir.join("b/AskHuman");
        std::fs::create_dir_all(p1.parent().unwrap()).unwrap();
        std::fs::create_dir_all(p2.parent().unwrap()).unwrap();
        std::fs::File::create(&p1).unwrap().write_all(bytes).unwrap();
        std::fs::File::create(&p2).unwrap().write_all(bytes).unwrap();
        assert_eq!(hash_file(&p1).unwrap(), hash_file(&p2).unwrap());

        // 内容不同 → 哈希不同。
        let p3 = dir.join("c/AskHuman");
        std::fs::create_dir_all(p3.parent().unwrap()).unwrap();
        std::fs::File::create(&p3).unwrap().write_all(b"different").unwrap();
        assert_ne!(hash_file(&p1).unwrap(), hash_file(&p3).unwrap());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn meta_round_trip() {
        let meta = DaemonMeta {
            pid: 1,
            version: "9.9.9".into(),
            protocol_version: 1,
            started_at: 100,
            socket: "/tmp/x.sock".into(),
            fingerprint: Fingerprint { size: 6, hash: 5 },
        };
        let json = serde_json::to_string(&meta).unwrap();
        let back: DaemonMeta = serde_json::from_str(&json).unwrap();
        assert_eq!(back.pid, 1);
        assert_eq!(back.version, "9.9.9");
        assert_eq!(back.fingerprint.size, 6);
        assert_eq!(back.fingerprint.hash, 5);
    }
}
