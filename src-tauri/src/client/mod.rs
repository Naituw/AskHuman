//! CLI 作为 Daemon 客户端：连接 / 握手 / 按需自启 / 换新重连，以及 status·stop 辅助。
//!
//! Phase 0：仅提供连通性原语（ensure_running / request_status / request_stop）。
//! 任务提交（submit）将在 Phase 1 接入。

use crate::daemon::lifecycle;
use crate::daemon::spawn;
use crate::ipc::{
    self, transport, ClientHello, ClientMsg, DetectRequest, HelloStatus, ServerMsg, StatusInfo,
};
use std::io::{Error, ErrorKind};
use std::time::{Duration, Instant};
use tokio::io::BufReader;
use tokio::net::unix::{OwnedReadHalf, OwnedWriteHalf};

type Reader = BufReader<OwnedReadHalf>;

/// 本进程的握手信息。
fn hello() -> ClientHello {
    ClientHello {
        protocol_version: ipc::PROTOCOL_VERSION,
        client_version: env!("CARGO_PKG_VERSION").to_string(),
        binary_path: std::env::current_exe()
            .ok()
            .map(|p| p.display().to_string())
            .unwrap_or_default(),
        fingerprint: lifecycle::current_fingerprint(),
        pid: std::process::id(),
    }
}

/// 连接并拆分读写半。
async fn connect_split() -> std::io::Result<(Reader, OwnedWriteHalf)> {
    let stream = transport::connect().await?;
    let (r, w) = stream.into_split();
    Ok((BufReader::new(r), w))
}

/// 连一次并握手，返回握手状态（连不上返回 None）。
async fn hello_status() -> Option<HelloStatus> {
    let (mut reader, mut writer) = connect_split().await.ok()?;
    ipc::write_msg(&mut writer, &ClientMsg::Hello(hello())).await.ok()?;
    match ipc::read_msg::<_, ServerMsg>(&mut reader).await {
        Ok(Some(ServerMsg::HelloAck(ack))) => Some(ack.status),
        _ => None,
    }
}

/// 确保 Daemon 在运行且为当前二进制版本；必要时自启 / 等旧实例换新。
pub async fn ensure_running() -> std::io::Result<()> {
    // 1. 已在运行且 Ok → 直接用。
    match hello_status().await {
        Some(HelloStatus::Ok) => return Ok(()),
        Some(HelloStatus::Restarting) => {
            // 旧实例将自行退出；等它下线后再拉起新的。
            wait_until_down(Duration::from_secs(5)).await;
        }
        // 排空中：快速返回错误（本函数被设置进程 Detect 等复用，不能无限阻塞）；
        // 需要等待的调用方（run_ask / stop / restart）自行处理排空等待。
        Some(HelloStatus::Draining) => {
            return Err(Error::new(ErrorKind::WouldBlock, "daemon is draining"));
        }
        None => {}
    }

    // 2. 拉起并等待就绪（最多约 5 秒）。
    spawn::spawn_detached()?;
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(5) {
        if let Some(HelloStatus::Ok) = hello_status().await {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    Err(Error::new(
        ErrorKind::TimedOut,
        "daemon did not become ready in time",
    ))
}

/// 请求运行状态（未运行返回 None）。
pub async fn request_status() -> Option<StatusInfo> {
    let (mut reader, mut writer) = connect_split().await.ok()?;
    ipc::write_msg(&mut writer, &ClientMsg::Status).await.ok()?;
    match ipc::read_msg::<_, ServerMsg>(&mut reader).await {
        Ok(Some(ServerMsg::Status(info))) => Some(info),
        _ => None,
    }
}

/// 请求停止（force=false 为 graceful：有在途请求时 Daemon 排空后退出）；
/// 收到 Stopping 回应返回 true，未运行返回 false。
pub async fn request_stop(force: bool) -> bool {
    let Ok((mut reader, mut writer)) = connect_split().await else {
        return false;
    };
    if ipc::write_msg(&mut writer, &ClientMsg::Stop { force }).await.is_err() {
        return false;
    }
    matches!(
        ipc::read_msg::<_, ServerMsg>(&mut reader).await,
        Ok(Some(ServerMsg::Stopping))
    )
}

/// 轮询直到 Daemon 不可连（或超时）。
pub async fn wait_until_down(max: Duration) {
    let start = Instant::now();
    while start.elapsed() < max {
        if transport::connect().await.is_err() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// 排空等待：旧 Daemon 正在完结在途请求，无限等待其下线（首条提示立即输出，之后每 30s 一条，
/// 含剩余在途数与强制换新提示）。剩余数经 `Status` 查询获取（不带 Hello，不会误触发 stale 判定）。
async fn wait_for_drain() {
    let mut last_hint: Option<Instant> = None;
    loop {
        if transport::connect().await.is_err() {
            return; // 旧 Daemon 已下线，可拉起新的。
        }
        if last_hint.map_or(true, |t| t.elapsed() >= Duration::from_secs(30)) {
            match request_status().await {
                Some(info) => eprintln!(
                    "askhuman: daemon is draining ({} active request(s) left); waiting to submit… (run 'AskHuman daemon restart --force' to switch now, interrupting them)",
                    info.active_requests
                ),
                None => eprintln!(
                    "askhuman: daemon is draining; waiting to submit… (run 'AskHuman daemon restart --force' to switch now)"
                ),
            }
            last_hint = Some(Instant::now());
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

/// 请求 Daemon 执行「自动识别 userId/open_id」（Q6）。
///
/// 返回语义（供设置进程决定是否回退到进程内临时连接）：
/// - `None`：**无法接通 Daemon**（自启失败 / 握手失败）→ 调用方可回退进程内识别。
/// - `Some(Ok(id))`：识别成功。
/// - `Some(Err(msg))`：Daemon 已执行识别但失败（超时 / 断连）→ 调用方**不应**回退（避免再开冲突连接）。
pub async fn request_detect(req: DetectRequest) -> Option<Result<String, String>> {
    if ensure_running().await.is_err() {
        return None;
    }
    let (mut reader, mut writer) = connect_split().await.ok()?;
    if ipc::write_msg(&mut writer, &ClientMsg::Hello(hello())).await.is_err() {
        return None;
    }
    match ipc::read_msg::<_, ServerMsg>(&mut reader).await {
        Ok(Some(ServerMsg::HelloAck(ack))) if ack.status == HelloStatus::Ok => {}
        // 换新中或握手异常：视作暂不可接通，让调用方回退。
        _ => return None,
    }
    // 握手 OK 后发 Detect；此后的失败都视作「Daemon 已接管」的结果，不再回退。
    if ipc::write_msg(&mut writer, &ClientMsg::Detect(req)).await.is_err() {
        return Some(Err("failed to send detect request".to_string()));
    }
    loop {
        match ipc::read_msg::<_, ServerMsg>(&mut reader).await {
            Ok(Some(ServerMsg::Detected { id })) => return Some(Ok(id)),
            Ok(Some(ServerMsg::Error { message })) => return Some(Err(message)),
            Ok(Some(_)) => continue,
            Ok(None) | Err(_) => return Some(Err("daemon connection lost".to_string())),
        }
    }
}

/// 生命周期事件上报（reporter 用，spec D20）：确保 daemon 在跑（拿不到也尽力直连），
/// 发一条 `AgentEvent` 即走。全程 best-effort，任何失败静默——hook 不能因追踪而拖慢/报错。
pub fn report_agent_event(msg: ClientMsg) {
    let Ok(rt) = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    else {
        return;
    };
    rt.block_on(async {
        // 即便 daemon 在排空（WouldBlock）也继续尝试直连：control_loop 不依赖 Hello 即可处理 AgentEvent，
        // 排空中的 daemon 退出前会持久化、由新 daemon 重载。
        let _ = ensure_running().await;
        if let Ok((_, mut writer)) = connect_split().await {
            let _ = ipc::write_msg(&mut writer, &msg).await;
        }
    });
}

/// 打开一条到 daemon 的连接（订阅状态窗口用，spec D20）：确保在跑后连接并拆分读写半。
pub async fn open_for_subscribe() -> std::io::Result<(Reader, OwnedWriteHalf)> {
    ensure_running().await?;
    connect_split().await
}

/// 瘦客户端 ask 入口：确保 Daemon 在运行 → 握手 → 提交任务 → 流式取回结果 → 按退出码退出（不返回）。
pub fn run_ask(task: crate::ipc::TaskRequest) -> ! {
    let rt = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(_) => std::process::exit(3),
    };
    std::process::exit(rt.block_on(run_ask_async(task)));
}

async fn run_ask_async(task: crate::ipc::TaskRequest) -> i32 {
    use crate::ipc::ServerMsg;

    // 外层循环：撞上 Daemon 排空（draining）时无限等待其下线，然后重置重试预算重来。
    // 内层：提交前最多重试若干次，覆盖「自启就绪竞争」与「撞上 Daemon 换新」的瞬时失败。
    // 提交被受理后不再重试（避免重复弹窗）。
    'outer: loop {
        for _ in 0..3 {
            match ensure_running().await {
                Ok(()) => {}
                Err(e) if e.kind() == ErrorKind::WouldBlock => {
                    wait_for_drain().await;
                    continue 'outer;
                }
                Err(_) => {
                    eprintln!("askhuman: failed to start daemon");
                    return 3;
                }
            }
            let Ok((mut reader, mut writer)) = connect_split().await else {
                tokio::time::sleep(Duration::from_millis(50)).await;
                continue;
            };
            if ipc::write_msg(&mut writer, &ClientMsg::Hello(hello())).await.is_err() {
                continue;
            }
            match ipc::read_msg::<_, ServerMsg>(&mut reader).await {
                Ok(Some(ServerMsg::HelloAck(ack))) => match ack.status {
                    HelloStatus::Ok => {}
                    HelloStatus::Restarting => {
                        wait_until_down(Duration::from_secs(5)).await;
                        continue;
                    }
                    HelloStatus::Draining => {
                        wait_for_drain().await;
                        continue 'outer;
                    }
                },
                _ => continue,
            }
            // 提交任务。
            if ipc::write_msg(&mut writer, &ClientMsg::Submit(task.clone())).await.is_err() {
                continue;
            }
            // 流式取回：Warn → stderr；Final → stdout + 退出码；中途断连 → 退出码 3（P4）。
            loop {
                match ipc::read_msg::<_, ServerMsg>(&mut reader).await {
                    Ok(Some(ServerMsg::Accepted { .. })) => {}
                    // 排空闸门拒绝（只出现在 Accepted 之前）：等旧 Daemon 下线后重来。
                    Ok(Some(ServerMsg::Draining { .. })) => {
                        wait_for_drain().await;
                        continue 'outer;
                    }
                    Ok(Some(ServerMsg::Warn { text })) => eprintln!("{}", text),
                    Ok(Some(ServerMsg::Final { stdout, exit_code })) => {
                        if !stdout.is_empty() {
                            println!("{}", stdout);
                        }
                        return exit_code;
                    }
                    Ok(Some(_)) => {}
                    Ok(None) | Err(_) => {
                        eprintln!("askhuman: daemon connection lost");
                        return 3;
                    }
                }
            }
        }
        eprintln!("askhuman: could not reach daemon");
        return 3;
    }
}
