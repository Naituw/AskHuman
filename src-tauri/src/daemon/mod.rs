//! 常驻 Daemon：子命令分发（run/start/stop/restart/status/logs）+ Phase 0 的空 Daemon 服务。
//!
//! Phase 0：起一个不承载任何渠道的空 Daemon，提供握手（含二进制指纹换新）、status、stop、
//! 单实例（flock）、自启、空闲退出。渠道 / 弹窗 / 提交将在后续 Phase 接入。

pub mod lifecycle;
pub mod request;
#[cfg(unix)]
pub mod config_watch;
#[cfg(unix)]
pub mod spawn;

/// `AskHuman daemon <sub>` 入口。永不返回（自行退出进程）。
pub fn dispatch(args: &[String]) -> ! {
    #[cfg(unix)]
    {
        std::process::exit(unix_impl::dispatch(args));
    }
    #[cfg(not(unix))]
    {
        let _ = args;
        eprintln!("AskHuman daemon is not supported on this platform yet.");
        std::process::exit(1);
    }
}

#[cfg(unix)]
mod unix_impl {
    use super::config_watch;
    use super::lifecycle::{self, DaemonMeta, LockGuard};
    use super::request::{self, RequestEntry, RequestRegistry};
    use crate::channels::dingding::DingTalkChannel;
    use crate::channels::feishu::FeishuChannel;
    use crate::channels::slack::SlackChannel;
    use crate::channels::telegram::TelegramChannel;
    use crate::channels::Channel;
    use crate::client;
    use crate::config::AppConfig;
    use crate::dingtalk::router::DdRouter;
    use crate::feishu::router::FsRouter;
    use crate::slack::router::SlRouter;
    use crate::telegram::router::TgRouter;
    use crate::i18n::Lang;
    use crate::ipc::{
        self, transport, ClientMsg, DetectRequest, HelloAck, HelloStatus, ServerMsg, StatusInfo,
        TaskRequest,
    };
    use crate::models::{ChannelResult, ChannelAction};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
    use tokio::io::BufReader;
    use tokio::net::unix::{OwnedReadHalf, OwnedWriteHalf};
    use tokio::net::UnixStream;

    type Reader = BufReader<OwnedReadHalf>;

    /// 无活动且无连接持续此时长后自动退出。可用 `ASKHUMAN_DAEMON_IDLE_SECS` 覆盖（便于测试）。
    const DEFAULT_IDLE_SECS: u64 = 300;

    fn idle_timeout() -> Duration {
        let secs = std::env::var("ASKHUMAN_DAEMON_IDLE_SECS")
            .ok()
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(DEFAULT_IDLE_SECS);
        Duration::from_secs(secs)
    }

    fn version() -> String {
        env!("CARGO_PKG_VERSION").to_string()
    }

    fn now_secs() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0)
    }

    fn log(msg: &str) {
        // `daemon run` 经 spawn 时 stderr 已重定向到 daemon.log；前台运行则打到终端。
        eprintln!("[askhuman-daemon {}] {}", now_secs(), msg);
    }

    fn block_on<F: std::future::Future>(f: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("build current-thread runtime")
            .block_on(f)
    }

    pub fn dispatch(args: &[String]) -> i32 {
        match args.first().map(|s| s.as_str()).unwrap_or("") {
            "run" => run_cmd(),
            "start" => start_cmd(),
            "stop" => stop_cmd(),
            "restart" => restart_cmd(),
            "status" => status_cmd(),
            "logs" => logs_cmd(),
            "" => {
                eprintln!("usage: AskHuman daemon <run|start|stop|restart|status|logs>");
                1
            }
            other => {
                eprintln!("unknown daemon subcommand: {}", other);
                eprintln!("usage: AskHuman daemon <run|start|stop|restart|status|logs>");
                1
            }
        }
    }

    // —— run：前台运行 Daemon 主体 ——

    fn run_cmd() -> i32 {
        let lock = match lifecycle::acquire_lock() {
            Ok(Some(l)) => l,
            Ok(None) => {
                log("another daemon is already running; exiting");
                return 0;
            }
            Err(e) => {
                log(&format!("failed to acquire lock: {}", e));
                return 1;
            }
        };
        // 复用 tauri 的全局（多线程）tokio 运行时，使各 Channel 的 `tauri::async_runtime::spawn`
        // 与 Daemon 自身的任务跑在同一运行时上（不初始化任何 GUI / AppKit）。
        tauri::async_runtime::block_on(serve(lock))
    }

    struct ServerState {
        startup_fp: lifecycle::Fingerprint,
        started_at: u64,
        /// 当前打开的连接数（CLI 提交连接 / GUI 连接 / 控制连接）。用于空闲退出判定：
        /// 提交连接在请求存续期间保持打开，故人在环里的长等待算「活动」，不会被空闲计时杀掉。
        active: AtomicUsize,
        last_active: Mutex<Instant>,
        shutdown: tokio::sync::Notify,
        /// 活动请求登记表。
        registry: Arc<RequestRegistry>,
        /// 钉钉长连接 Router（惰性建连、常热复用；连接死亡后按需重连）。
        dd_router: tokio::sync::Mutex<Option<Arc<DdRouter>>>,
        /// 飞书长连接 Router（惰性建连、常热复用；连接死亡后按需重连）。
        fs_router: tokio::sync::Mutex<Option<Arc<FsRouter>>>,
        /// Telegram 长轮询 Router（惰性建连、常热复用；单一 offset）。
        tg_router: tokio::sync::Mutex<Option<Arc<TgRouter>>>,
        /// Slack Socket Mode Router（惰性建连、常热复用；连接死亡后按需重连）。
        sl_router: tokio::sync::Mutex<Option<Arc<SlRouter>>>,
        /// 最近一次已知配置快照（config watch 据此比对差异，决定哪些 Router 需失效，A12）。
        config: Mutex<AppConfig>,
    }

    async fn serve(_lock: LockGuard) -> i32 {
        let listener = match transport::bind() {
            Ok(l) => l,
            Err(e) => {
                log(&format!("failed to bind socket: {}", e));
                return 1;
            }
        };
        let startup_fp = lifecycle::current_fingerprint();
        let started_at = now_secs();
        let meta = DaemonMeta {
            pid: std::process::id(),
            version: version(),
            protocol_version: ipc::PROTOCOL_VERSION,
            started_at,
            socket: transport::socket_path().display().to_string(),
            fingerprint: startup_fp,
        };
        if let Err(e) = lifecycle::write_meta(&meta) {
            log(&format!("failed to write daemon.json: {}", e));
        }
        log(&format!(
            "started pid={} version={} protocol={}",
            meta.pid, meta.version, meta.protocol_version
        ));

        let state = Arc::new(ServerState {
            startup_fp,
            started_at,
            active: AtomicUsize::new(0),
            last_active: Mutex::new(Instant::now()),
            shutdown: tokio::sync::Notify::new(),
            registry: RequestRegistry::new(),
            dd_router: tokio::sync::Mutex::new(None),
            fs_router: tokio::sync::Mutex::new(None),
            tg_router: tokio::sync::Mutex::new(None),
            sl_router: tokio::sync::Mutex::new(None),
            config: Mutex::new(AppConfig::load()),
        });

        // 空闲退出检查。
        {
            let state = state.clone();
            tokio::spawn(async move {
                let timeout = idle_timeout();
                loop {
                    tokio::time::sleep(Duration::from_secs(30)).await;
                    if state.active.load(Ordering::SeqCst) == 0 {
                        let idle = state.last_active.lock().map(|t| t.elapsed()).unwrap_or_default();
                        if idle >= timeout {
                            log("idle timeout reached; shutting down");
                            state.shutdown.notify_one();
                            break;
                        }
                    }
                }
            });
        }

        // 配置实时生效（A12）：监听 config.json 变更 → 重载 + 失效改动的 Router + 通知活动 GUI。
        {
            let state = state.clone();
            let mut rx = config_watch::spawn();
            tokio::spawn(async move {
                while rx.recv().await.is_some() {
                    on_config_changed(&state).await;
                }
            });
        }

        // 临时目录清理（A10）：启动即清一次，之后每小时清理过期 temp/askhuman/<id>/。
        tokio::spawn(async move {
            loop {
                cleanup_temp_dirs();
                tokio::time::sleep(Duration::from_secs(3600)).await;
            }
        });

        loop {
            tokio::select! {
                _ = state.shutdown.notified() => break,
                accepted = listener.accept() => match accepted {
                    Ok((stream, _addr)) => {
                        let st = state.clone();
                        tokio::spawn(async move { handle_conn(stream, st).await; });
                    }
                    Err(e) => log(&format!("accept error: {}", e)),
                }
            }
        }

        // Shutting down: cancel any in-flight requests so their IM cards finalize to "Cancelled".
        // Unlike CLI-disconnect, the runtime is about to exit, so we give the finalize HTTP calls a
        // brief bounded window to land (sessions may take up to ~1s to notice the cancel + ~0.3s HTTP).
        if state.registry.cancel_all_requests() > 0 {
            tokio::time::sleep(Duration::from_millis(2000)).await;
        }

        // 收尾：主动丢弃常热 Router（其 Drop 中止 reader 任务、关闭 IM 长连接），再清理 socket/meta。
        // 进行中的请求各自持有 Router Arc 克隆，故仅在无人持有时才真正断连。
        *state.dd_router.lock().await = None;
        *state.fs_router.lock().await = None;
        *state.tg_router.lock().await = None;
        *state.sl_router.lock().await = None;
        cleanup();
        log("stopped");
        0
    }

    /// 控制阶段的产物：收到接管型消息（提交 / GUI 握手）或连接关闭。
    enum Control {
        Submit(TaskRequest),
        Gui(String),
        Closed,
    }

    async fn handle_conn(stream: UnixStream, state: Arc<ServerState>) {
        state.active.fetch_add(1, Ordering::SeqCst);
        let (r, w) = stream.into_split();
        let mut reader = BufReader::new(r);
        let mut w = w;

        match control_loop(&mut reader, &mut w, &state).await {
            Control::Submit(task) => handle_submit(task, reader, w, &state).await,
            Control::Gui(token) => handle_gui(token, reader, w, &state).await,
            Control::Closed => {}
        }

        if let Ok(mut t) = state.last_active.lock() {
            *t = Instant::now();
        }
        state.active.fetch_sub(1, Ordering::SeqCst);
    }

    /// 控制阶段：处理 Hello / Status / Stop（即时应答）；遇到 Submit / GuiHello 返回以便接管连接。
    async fn control_loop(reader: &mut Reader, w: &mut OwnedWriteHalf, state: &Arc<ServerState>) -> Control {
        loop {
            let msg: Option<ClientMsg> = match ipc::read_msg(reader).await {
                Ok(m) => m,
                Err(e) => {
                    log(&format!("read error: {}", e));
                    return Control::Closed;
                }
            };
            let Some(msg) = msg else { return Control::Closed }; // EOF / 对端关闭

            match msg {
                ClientMsg::Hello(hello) => {
                    let now_fp = lifecycle::current_fingerprint();
                    // 指纹按「内容哈希」比对（与路径/mtime 无关）：
                    // 自身盘上二进制内容被换 / 客户端二进制内容不一致 / 协议不一致 → 过时，让位换新。
                    let stale = now_fp != state.startup_fp
                        || hello.fingerprint != state.startup_fp
                        || hello.protocol_version != ipc::PROTOCOL_VERSION;
                    let auto_restart = std::env::var("ASKHUMAN_DAEMON_AUTORESTART")
                        .map(|v| v != "0")
                        .unwrap_or(true);
                    let restarting = stale && auto_restart;
                    let ack = HelloAck {
                        protocol_version: ipc::PROTOCOL_VERSION,
                        daemon_version: version(),
                        status: if restarting {
                            HelloStatus::Restarting
                        } else {
                            HelloStatus::Ok
                        },
                        reason: if restarting {
                            Some("binary or protocol changed".to_string())
                        } else {
                            None
                        },
                    };
                    let _ = ipc::write_msg(w, &ServerMsg::HelloAck(ack)).await;
                    if restarting {
                        log("stale binary/protocol detected; shutting down for restart");
                        state.shutdown.notify_one();
                        return Control::Closed;
                    }
                }
                ClientMsg::Status => {
                    let info = StatusInfo {
                        pid: std::process::id(),
                        version: version(),
                        protocol_version: ipc::PROTOCOL_VERSION,
                        uptime_secs: now_secs().saturating_sub(state.started_at),
                        socket: transport::socket_path().display().to_string(),
                        active_requests: state.registry.active_count(),
                        im_connections: active_im_connections(state).await,
                    };
                    let _ = ipc::write_msg(w, &ServerMsg::Status(info)).await;
                }
                ClientMsg::Stop => {
                    let _ = ipc::write_msg(w, &ServerMsg::Stopping).await;
                    log("stop requested");
                    state.shutdown.notify_one();
                    return Control::Closed;
                }
                ClientMsg::Submit(task) => return Control::Submit(task),
                ClientMsg::GuiHello { token } => return Control::Gui(token),
                // 自动识别（Q6）：就地处理（可能阻塞至多 120s 等用户发码），完成后回结果继续循环。
                ClientMsg::Detect(req) => handle_detect(&req, state, w).await,
                // Answer 只应在 GUI 接管阶段出现；控制阶段收到即忽略。
                ClientMsg::Answer { .. } => {}
            }
        }
    }

    /// CLI 提交一次任务：建请求、spawn GUI Helper、流式回结果；CLI 断开则取消。
    async fn handle_submit(
        task: TaskRequest,
        mut reader: Reader,
        mut w: OwnedWriteHalf,
        state: &Arc<ServerState>,
    ) {
        let lang = Lang::resolve(&task.lang);
        let (entry, mut final_rx) = state.registry.create(task);
        let request_id = entry.request_id.clone();
        log(&format!("request {} accepted", request_id));

        if ipc::write_msg(&mut w, &ServerMsg::Accepted { request_id: request_id.clone() })
            .await
            .is_err()
        {
            state.registry.remove(&request_id);
            return;
        }

        // 挂接可用的 IM 渠道（钉钉/…）到本请求的协调器，与弹窗并行抢答。
        let im_attached = attach_im_channels(&entry, state, &mut w, lang).await;

        // spawn GUI Helper（独立短命进程，带一次性 token）。
        let popup_ok = match spawn_gui_helper(&entry.token) {
            Ok(()) => true,
            Err(e) => {
                log(&format!("failed to spawn GUI helper: {}", e));
                let _ = ipc::write_msg(&mut w, &ServerMsg::Warn {
                    text: format!("{}failed to spawn popup: {}", crate::i18n::err_prefix(lang), e),
                })
                .await;
                false
            }
        };

        // 既无弹窗也无 IM 渠道 → 无可用渠道，按错误收尾。
        if !popup_ok && !im_attached {
            let _ = ipc::write_msg(&mut w, &ServerMsg::Final {
                stdout: String::new(),
                exit_code: request::EXIT_NO_CHANNEL,
            })
            .await;
            state.registry.remove(&request_id);
            return;
        }

        // 看门狗：弹窗已拉起但限定时间内未连上 → 判失败；但若已挂了 IM 渠道则不致命（让 IM 继续等答）。
        if popup_ok {
            spawn_gui_watchdog(entry.clone(), lang, im_attached);
        }

        // 等待结果或 CLI 断开。
        let outcome = tokio::select! {
            o = final_rx.recv() => o,
            _ = wait_cli_eof(&mut reader) => {
                log(&format!("request {} cli disconnected; cancelling", request_id));
                // Cancel the whole request: IM cards finalize to "Cancelled by caller", popup closes.
                // The IM finalize runs in the channels' own tasks (which outlive this entry), so the
                // daemon need not wait here — it stays alive.
                let caller = crate::i18n::tr(lang, "channel.sourceCaller").to_string();
                entry.coordinator.cancel_request(caller);
                entry.cancel.notify_waiters();
                state.registry.remove(&request_id);
                return;
            }
        };

        match outcome {
            Some(o) => {
                if let Some(err) = &o.stderr {
                    let _ = ipc::write_msg(&mut w, &ServerMsg::Warn { text: err.clone() }).await;
                }
                let _ = ipc::write_msg(&mut w, &ServerMsg::Final {
                    stdout: o.stdout,
                    exit_code: o.exit_code,
                })
                .await;
            }
            None => {
                // 渲染通道意外关闭：判异常退出码 3。
                let _ = ipc::write_msg(&mut w, &ServerMsg::Final {
                    stdout: String::new(),
                    exit_code: 3,
                })
                .await;
            }
        }
        entry.cancel.notify_waiters();
        state.registry.remove(&request_id);
        log(&format!("request {} done", request_id));
    }

    /// GUI Helper 连接：凭 token 关联请求、下发 show、收 answer 投递协调器。
    async fn handle_gui(
        token: String,
        mut reader: Reader,
        w: OwnedWriteHalf,
        state: &Arc<ServerState>,
    ) {
        let Some(entry) = state.registry.attach_gui(&token) else {
            log("gui hello with unknown token; closing");
            return;
        };
        entry.gui_connected.store(true, Ordering::SeqCst);

        // GUI 发送端：专用写任务串行写出 ServerMsg，供 show 下发与 adapter cancel 共用。
        let (gui_tx, mut gui_rx) = tokio::sync::mpsc::unbounded_channel::<ServerMsg>();
        let mut w = w;
        let writer = tokio::spawn(async move {
            while let Some(msg) = gui_rx.recv().await {
                if ipc::write_msg(&mut w, &msg).await.is_err() {
                    break;
                }
            }
        });
        if let Ok(mut slot) = entry.gui.lock() {
            *slot = Some(gui_tx.clone());
        }

        // 下发题目。
        let _ = gui_tx.send(request::show_msg(&entry));

        // 读 GUI 答复 / 收到取消通知。
        loop {
            tokio::select! {
                msg = ipc::read_msg::<_, ClientMsg>(&mut reader) => {
                    match msg {
                        Ok(Some(ClientMsg::Answer { action, answers, .. })) => {
                            let result = match action {
                                ChannelAction::Send => ChannelResult {
                                    action: ChannelAction::Send,
                                    answers,
                                    source_channel_id: "popup".to_string(),
                                },
                                ChannelAction::Cancel => ChannelResult::cancel("popup"),
                            };
                            entry.coordinator.submit(result);
                            break;
                        }
                        Ok(Some(_)) => {}
                        Ok(None) | Err(_) => {
                            // Helper 断开且未作答：视为取消（已完成则为 no-op）。
                            entry.coordinator.submit(ChannelResult::cancel("popup"));
                            break;
                        }
                    }
                }
                _ = entry.cancel.notified() => break,
            }
        }

        // 收尾：清空发送端槽位并丢弃发送端 → 写任务结束 → GUI 连接关闭 → Helper 收到 EOF 退出。
        if let Ok(mut slot) = entry.gui.lock() {
            *slot = None;
        }
        drop(gui_tx);
        let _ = writer.await;
    }

    /// 等待 CLI 提交连接断开（提交后 CLI 不再发消息；任何 EOF/错误即视为断开）。
    async fn wait_cli_eof(reader: &mut Reader) {
        loop {
            match ipc::read_msg::<_, ClientMsg>(reader).await {
                Ok(Some(_)) => continue,
                Ok(None) | Err(_) => return,
            }
        }
    }

    /// 看门狗：限定时间内 GUI 未连上 → 经渲染通道送「弹窗拉起失败」结果（退出码 3）。
    /// `im_attached` 为真时弹窗未连上不判失败——IM 渠道仍可作答。
    fn spawn_gui_watchdog(entry: Arc<RequestEntry>, lang: Lang, im_attached: bool) {
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(request::GUI_CONNECT_TIMEOUT_SECS)).await;
            if !entry.gui_connected.load(Ordering::SeqCst) && !im_attached {
                let _ = entry.final_tx.send(request::popup_failed_outcome(lang));
            }
        });
    }

    /// 取得（必要时惰性建连）钉钉 Router；连接已死则重连。失败返回 None。
    async fn ensure_dd_router(
        state: &Arc<ServerState>,
        client_id: &str,
        client_secret: &str,
    ) -> Option<Arc<DdRouter>> {
        let mut guard = state.dd_router.lock().await;
        if let Some(r) = guard.as_ref() {
            if r.is_alive() {
                return Some(r.clone());
            }
        }
        match DdRouter::connect(client_id, client_secret).await {
            Ok(r) => {
                log("dingtalk router connected");
                *guard = Some(r.clone());
                Some(r)
            }
            Err(e) => {
                log(&format!("dingtalk router connect failed: {}", e));
                *guard = None;
                None
            }
        }
    }

    /// 取得（必要时惰性建连）飞书 Router；连接已死则重连。失败返回 None。
    async fn ensure_fs_router(
        state: &Arc<ServerState>,
        cfg: &crate::config::FeishuChannelConfig,
    ) -> Option<Arc<FsRouter>> {
        let mut guard = state.fs_router.lock().await;
        if let Some(r) = guard.as_ref() {
            if r.is_alive() {
                return Some(r.clone());
            }
        }
        match FsRouter::connect(cfg).await {
            Ok(r) => {
                log("feishu router connected");
                *guard = Some(r.clone());
                Some(r)
            }
            Err(e) => {
                log(&format!("feishu router connect failed: {}", e));
                *guard = None;
                None
            }
        }
    }

    /// 取得（必要时惰性建连）Telegram Router；轮询器已停则重建。失败返回 None。
    async fn ensure_tg_router(
        state: &Arc<ServerState>,
        cfg: &crate::config::TelegramChannelConfig,
    ) -> Option<Arc<TgRouter>> {
        let mut guard = state.tg_router.lock().await;
        if let Some(r) = guard.as_ref() {
            if r.is_alive() {
                return Some(r.clone());
            }
        }
        match TgRouter::connect(cfg).await {
            Ok(r) => {
                log("telegram router connected");
                *guard = Some(r.clone());
                Some(r)
            }
            Err(e) => {
                log(&format!("telegram router connect failed: {}", e));
                *guard = None;
                None
            }
        }
    }

    /// 取得（必要时惰性建连）Slack Router；连接已死则重连。失败返回 None。
    async fn ensure_sl_router(
        state: &Arc<ServerState>,
        cfg: &crate::config::SlackChannelConfig,
    ) -> Option<Arc<SlRouter>> {
        let mut guard = state.sl_router.lock().await;
        if let Some(r) = guard.as_ref() {
            if r.is_alive() {
                return Some(r.clone());
            }
        }
        match SlRouter::connect(cfg).await {
            Ok(r) => {
                log("slack router connected");
                *guard = Some(r.clone());
                Some(r)
            }
            Err(e) => {
                log(&format!("slack router connect failed: {}", e));
                *guard = None;
                None
            }
        }
    }

    /// 按当前配置把可用 IM 渠道挂到请求协调器上（与弹窗并行抢答）。返回是否至少挂上一个。
    /// 失败的渠道经 `Warn` 流给 CLI stderr，并记 daemon.log。
    async fn attach_im_channels(
        entry: &Arc<RequestEntry>,
        state: &Arc<ServerState>,
        w: &mut OwnedWriteHalf,
        lang: Lang,
    ) -> bool {
        let config = AppConfig::load();
        let request = entry.show.request.clone();
        let sink = entry.coordinator.clone();
        let mut attached = false;

        if crate::app::is_dingding_active(&config) {
            let dd = &config.channels.dingding;
            match ensure_dd_router(state, dd.client_id.trim(), dd.client_secret.trim()).await {
                Some(router) => {
                    let ch: Arc<dyn Channel> =
                        Arc::new(DingTalkChannel::shared(dd.clone(), router));
                    entry.coordinator.register(ch.clone());
                    ch.start(&request, sink.clone());
                    attached = true;
                }
                None => {
                    let _ = ipc::write_msg(w, &ServerMsg::Warn {
                        text: format!(
                            "{}{}",
                            crate::i18n::warn_prefix(lang),
                            crate::i18n::tr(lang, "channel.ddConfigInvalidSkip")
                                .replace("{e}", "Stream connection failed"),
                        ),
                    })
                    .await;
                }
            }
        }

        if crate::app::is_feishu_active(&config) {
            let fs = &config.channels.feishu;
            match ensure_fs_router(state, fs).await {
                Some(router) => {
                    let ch: Arc<dyn Channel> = Arc::new(FeishuChannel::shared(fs.clone(), router));
                    entry.coordinator.register(ch.clone());
                    ch.start(&request, sink.clone());
                    attached = true;
                }
                None => {
                    let _ = ipc::write_msg(w, &ServerMsg::Warn {
                        text: format!(
                            "{}{}",
                            crate::i18n::warn_prefix(lang),
                            crate::i18n::tr(lang, "channel.fsConfigInvalidSkip")
                                .replace("{e}", "WebSocket connection failed"),
                        ),
                    })
                    .await;
                }
            }
        }

        if crate::app::is_telegram_active(&config) {
            let tg = &config.channels.telegram;
            match ensure_tg_router(state, tg).await {
                Some(router) => {
                    let ch: Arc<dyn Channel> =
                        Arc::new(TelegramChannel::shared(tg.clone(), router));
                    entry.coordinator.register(ch.clone());
                    ch.start(&request, sink.clone());
                    attached = true;
                }
                None => {
                    let _ = ipc::write_msg(w, &ServerMsg::Warn {
                        text: format!(
                            "{}{}",
                            crate::i18n::warn_prefix(lang),
                            crate::i18n::tr(lang, "channel.tgConfigInvalidSkip")
                                .replace("{e}", "poller start failed"),
                        ),
                    })
                    .await;
                }
            }
        }

        if crate::app::is_slack_active(&config) {
            let sl = &config.channels.slack;
            match ensure_sl_router(state, sl).await {
                Some(router) => {
                    let ch: Arc<dyn Channel> = Arc::new(SlackChannel::shared(sl.clone(), router));
                    entry.coordinator.register(ch.clone());
                    ch.start(&request, sink.clone());
                    attached = true;
                }
                None => {
                    let _ = ipc::write_msg(w, &ServerMsg::Warn {
                        text: format!(
                            "{}{}",
                            crate::i18n::warn_prefix(lang),
                            crate::i18n::tr(lang, "channel.slConfigInvalidSkip")
                                .replace("{e}", "Socket Mode connection failed"),
                        ),
                    })
                    .await;
                }
            }
        }

        attached
    }

    /// 当前已建连且存活的 IM 长连接名（供 `daemon status` 展示）。
    async fn active_im_connections(state: &Arc<ServerState>) -> Vec<String> {
        let mut v = Vec::new();
        if state
            .dd_router
            .lock()
            .await
            .as_ref()
            .map(|r| r.is_alive())
            .unwrap_or(false)
        {
            v.push("dingtalk".to_string());
        }
        if state
            .fs_router
            .lock()
            .await
            .as_ref()
            .map(|r| r.is_alive())
            .unwrap_or(false)
        {
            v.push("feishu".to_string());
        }
        if state
            .tg_router
            .lock()
            .await
            .as_ref()
            .map(|r| r.is_alive())
            .unwrap_or(false)
        {
            v.push("telegram".to_string());
        }
        if state
            .sl_router
            .lock()
            .await
            .as_ref()
            .map(|r| r.is_alive())
            .unwrap_or(false)
        {
            v.push("slack".to_string());
        }
        v
    }

    /// 处理「自动识别 userId/open_id」（Q6）：观察现有同 app_key 的长连接，否则临时开连完成识别。
    /// 结果经 `Detected`（成功）/ `Error`（失败，已本地化）回设置进程。
    async fn handle_detect(req: &DetectRequest, state: &Arc<ServerState>, w: &mut OwnedWriteHalf) {
        let lang = Lang::resolve(&req.lang);
        let result = match req.kind.as_str() {
            "dingtalk" => detect_dingtalk(state, req, lang).await,
            "feishu" => detect_feishu(state, req, lang).await,
            "slack" => detect_slack(state, req, lang).await,
            other => Err(format!("unknown detect kind: {}", other)),
        };
        let msg = match result {
            Ok(id) => ServerMsg::Detected { id },
            Err(message) => ServerMsg::Error { message },
        };
        let _ = ipc::write_msg(w, &msg).await;
    }

    /// 钉钉识别：优先观察现有同 client_id 的活动连接（零冲突），否则临时开连。
    async fn detect_dingtalk(
        state: &Arc<ServerState>,
        req: &DetectRequest,
        lang: Lang,
    ) -> Result<String, String> {
        let code = req.code.trim().to_string();
        if code.is_empty() {
            return Err(crate::i18n::tr(lang, "cmd.detectCodeInvalid").to_string());
        }
        // 复用：已有同 client_id 的活动 Router → 观察现有连接（忽略表单 secret）。
        let existing = {
            let guard = state.dd_router.lock().await;
            match guard.as_ref() {
                Some(r) if r.is_alive() && r.client_id() == req.app_key.trim() => {
                    Some(r.observe_bot())
                }
                _ => None,
            }
        };
        if let Some(mut rx) = existing {
            return wait_dd_code(&mut rx, &code, lang).await;
        }
        // 否则 daemon 自行临时开连；完成后 drop（Drop 中止 reader、关闭连接，零泄漏）。
        let router = DdRouter::connect(req.app_key.trim(), req.app_secret.trim()).await?;
        let mut rx = router.observe_bot();
        let out = wait_dd_code(&mut rx, &code, lang).await;
        drop(rx);
        drop(router);
        out
    }

    /// 等钉钉单聊文本内容等于识别码的消息，返回 senderStaffId；120s 超时。
    async fn wait_dd_code(
        rx: &mut tokio::sync::mpsc::UnboundedReceiver<serde_json::Value>,
        code: &str,
        lang: Lang,
    ) -> Result<String, String> {
        let deadline = Instant::now() + Duration::from_secs(120);
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(crate::i18n::tr(lang, "cmd.detectTimeout").to_string());
            }
            match tokio::time::timeout(remaining, rx.recv()).await {
                Ok(Some(data)) => {
                    let content = data
                        .get("text")
                        .and_then(|t| t.get("content"))
                        .and_then(|c| c.as_str())
                        .unwrap_or("")
                        .trim();
                    if content == code {
                        if let Some(sender) = data.get("senderStaffId").and_then(|v| v.as_str()) {
                            return Ok(sender.to_string());
                        }
                    }
                }
                Ok(None) => return Err(crate::i18n::tr(lang, "cmd.streamDisconnected").to_string()),
                Err(_) => return Err(crate::i18n::tr(lang, "cmd.detectTimeout").to_string()),
            }
        }
    }

    /// 飞书识别：优先观察现有同 app_id 的活动连接（零冲突），否则临时开连。
    async fn detect_feishu(
        state: &Arc<ServerState>,
        req: &DetectRequest,
        lang: Lang,
    ) -> Result<String, String> {
        let code = req.code.trim().to_string();
        if code.is_empty() {
            return Err(crate::i18n::tr(lang, "cmd.detectCodeInvalid").to_string());
        }
        let existing = {
            let guard = state.fs_router.lock().await;
            match guard.as_ref() {
                Some(r) if r.is_alive() && r.app_id() == req.app_key.trim() => {
                    Some(r.observe_message())
                }
                _ => None,
            }
        };
        if let Some(mut rx) = existing {
            return wait_fs_code(&mut rx, &code, lang).await;
        }
        let cfg = crate::config::FeishuChannelConfig {
            enabled: true,
            app_id: req.app_key.trim().to_string(),
            app_secret: req.app_secret.trim().to_string(),
            open_id: String::new(),
            base_url: req.base_url.trim().to_string(),
        };
        let router = FsRouter::connect(&cfg).await?;
        let mut rx = router.observe_message();
        let out = wait_fs_code(&mut rx, &code, lang).await;
        drop(rx);
        drop(router);
        out
    }

    /// 等飞书单聊文本内容等于识别码的消息，返回发送者 open_id；120s 超时。
    async fn wait_fs_code(
        rx: &mut tokio::sync::mpsc::UnboundedReceiver<serde_json::Value>,
        code: &str,
        lang: Lang,
    ) -> Result<String, String> {
        let deadline = Instant::now() + Duration::from_secs(120);
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(crate::i18n::tr(lang, "cmd.detectTimeout").to_string());
            }
            match tokio::time::timeout(remaining, rx.recv()).await {
                Ok(Some(event)) => {
                    if let Some((open_id, text)) = fs_text_and_sender(&event) {
                        if text.trim() == code {
                            return Ok(open_id);
                        }
                    }
                }
                Ok(None) => return Err(crate::i18n::tr(lang, "cmd.streamDisconnected").to_string()),
                Err(_) => return Err(crate::i18n::tr(lang, "cmd.detectTimeout").to_string()),
            }
        }
    }

    /// Slack 识别：优先观察现有同 app_token 的活动连接（零冲突），否则临时开连。
    /// app_key = App Token（Socket 复用键），app_secret = Bot Token（建连校验齐全）。
    async fn detect_slack(
        state: &Arc<ServerState>,
        req: &DetectRequest,
        lang: Lang,
    ) -> Result<String, String> {
        let code = req.code.trim().to_string();
        if code.is_empty() {
            return Err(crate::i18n::tr(lang, "cmd.detectCodeInvalid").to_string());
        }
        let existing = {
            let guard = state.sl_router.lock().await;
            match guard.as_ref() {
                Some(r) if r.is_alive() && r.app_token() == req.app_key.trim() => {
                    Some(r.observe_message())
                }
                _ => None,
            }
        };
        if let Some(mut rx) = existing {
            return wait_sl_code(&mut rx, &code, lang).await;
        }
        let cfg = crate::config::SlackChannelConfig {
            enabled: true,
            bot_token: req.app_secret.trim().to_string(),
            app_token: req.app_key.trim().to_string(),
            user_id: String::new(),
        };
        let router = SlRouter::connect(&cfg).await?;
        let mut rx = router.observe_message();
        let out = wait_sl_code(&mut rx, &code, lang).await;
        drop(rx);
        drop(router);
        out
    }

    /// 等 Slack 单聊文本内容等于识别码的消息，返回发送者 user id；120s 超时。
    async fn wait_sl_code(
        rx: &mut tokio::sync::mpsc::UnboundedReceiver<serde_json::Value>,
        code: &str,
        lang: Lang,
    ) -> Result<String, String> {
        let deadline = Instant::now() + Duration::from_secs(120);
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(crate::i18n::tr(lang, "cmd.detectTimeout").to_string());
            }
            match tokio::time::timeout(remaining, rx.recv()).await {
                Ok(Some(event)) => {
                    let user = event.get("user").and_then(|v| v.as_str()).unwrap_or("");
                    let text = event.get("text").and_then(|v| v.as_str()).unwrap_or("");
                    if !user.is_empty() && text.trim() == code {
                        return Ok(user.to_string());
                    }
                }
                Ok(None) => return Err(crate::i18n::tr(lang, "cmd.streamDisconnected").to_string()),
                Err(_) => return Err(crate::i18n::tr(lang, "cmd.detectTimeout").to_string()),
            }
        }
    }

    /// 从飞书 im.message.receive_v1 的 event 取 (发送者 open_id, 文本)。非文本消息返回 None。
    fn fs_text_and_sender(event: &serde_json::Value) -> Option<(String, String)> {
        let open_id = event
            .get("sender")
            .and_then(|s| s.get("sender_id"))
            .and_then(|i| i.get("open_id"))
            .and_then(|v| v.as_str())?
            .to_string();
        let message = event.get("message")?;
        if message.get("message_type").and_then(|v| v.as_str()) != Some("text") {
            return None;
        }
        let content_str = message.get("content").and_then(|v| v.as_str()).unwrap_or("{}");
        let content: serde_json::Value = serde_json::from_str(content_str).ok()?;
        let text = content.get("text").and_then(|v| v.as_str())?.to_string();
        Some((open_id, text))
    }

    /// spawn 一个 GUI Helper 进程（`AskHuman --popup --endpoint <sock> --token <tok>`）。
    fn spawn_gui_helper(token: &str) -> std::io::Result<()> {
        use std::process::{Command, Stdio};
        let exe = std::env::current_exe()?;
        Command::new(exe)
            .arg("--popup")
            .arg("--endpoint")
            .arg(transport::socket_path())
            .arg("--token")
            .arg(token)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map(|_| ())
    }

    fn cleanup() {
        let _ = std::fs::remove_file(transport::socket_path());
        let _ = std::fs::remove_file(lifecycle::meta_path());
    }

    /// 临时产物保留时长：消费者(AI)在 CLI 退出后才读图片路径，留足窗口避免误删刚产出的文件。
    const TEMP_MAX_AGE: Duration = Duration::from_secs(24 * 3600);

    /// 清理 `temp/askhuman/<id>/` 中超过 `TEMP_MAX_AGE` 未改动的目录（A10，启动 + 每小时）。
    fn cleanup_temp_dirs() {
        let base = std::env::temp_dir().join("askhuman");
        let Ok(entries) = std::fs::read_dir(&base) else {
            return;
        };
        let now = SystemTime::now();
        for e in entries.flatten() {
            let p = e.path();
            if !p.is_dir() {
                continue;
            }
            let age = e
                .metadata()
                .ok()
                .and_then(|m| m.modified().ok())
                .and_then(|t| now.duration_since(t).ok());
            if matches!(age, Some(a) if a >= TEMP_MAX_AGE) {
                let _ = std::fs::remove_dir_all(&p);
            }
        }
    }

    /// config.json 变更（已去抖）：重载配置 → 失效凭据改动/被禁用渠道的缓存 Router → 通知活动 GUI。
    async fn on_config_changed(state: &Arc<ServerState>) {
        let new = AppConfig::load();
        let old = { state.config.lock().unwrap().clone() };
        invalidate_changed_routers(state, &old, &new).await;
        *state.config.lock().unwrap() = new.clone();
        let general = serde_json::to_value(&new.general).unwrap_or(serde_json::Value::Null);
        state
            .registry
            .broadcast_to_guis(ServerMsg::ConfigChanged { general });
        log("config reloaded");
    }

    /// 比对新旧配置：凭据变更或渠道被禁用 → 丢弃对应缓存 Router（惰性失效，Q1）。
    ///
    /// 进行中的请求仍持有自己的 Router `Arc` 克隆，故其连接保留到该请求结束；
    /// 下一个请求会经 `ensure_*_router` 用新配置重连。注意：若仅改了同 client_id 的 secret，
    /// 且旧请求未结束时新请求又到达，可能短暂出现两条同 client_id 连接（平台会踢掉旧的）——
    /// 属配置在「问题进行中」被改动的少见边角，可接受。
    async fn invalidate_changed_routers(state: &Arc<ServerState>, old: &AppConfig, new: &AppConfig) {
        let dd_changed = !crate::app::is_dingding_active(new)
            || old.channels.dingding.client_id != new.channels.dingding.client_id
            || old.channels.dingding.client_secret != new.channels.dingding.client_secret;
        if dd_changed {
            *state.dd_router.lock().await = None;
        }

        let fs_changed = !crate::app::is_feishu_active(new)
            || old.channels.feishu.app_id != new.channels.feishu.app_id
            || old.channels.feishu.app_secret != new.channels.feishu.app_secret
            || old.channels.feishu.base_url != new.channels.feishu.base_url;
        if fs_changed {
            *state.fs_router.lock().await = None;
        }

        let tg_changed = !crate::app::is_telegram_active(new)
            || old.channels.telegram.bot_token != new.channels.telegram.bot_token
            || old.channels.telegram.chat_id != new.channels.telegram.chat_id
            || old.channels.telegram.api_base_url != new.channels.telegram.api_base_url;
        if tg_changed {
            *state.tg_router.lock().await = None;
        }

        let sl_changed = !crate::app::is_slack_active(new)
            || old.channels.slack.bot_token != new.channels.slack.bot_token
            || old.channels.slack.app_token != new.channels.slack.app_token;
        if sl_changed {
            *state.sl_router.lock().await = None;
        }
    }

    // —— start / stop / restart / status / logs：作为客户端操作 Daemon ——

    fn start_cmd() -> i32 {
        block_on(async {
            match client::ensure_running().await {
                Ok(()) => {
                    match client::request_status().await {
                        Some(info) => print_status(&info),
                        None => println!("askhuman daemon: running"),
                    }
                    0
                }
                Err(e) => {
                    eprintln!("failed to start daemon: {}", e);
                    1
                }
            }
        })
    }

    fn stop_cmd() -> i32 {
        block_on(async {
            if client::request_stop().await {
                println!("askhuman daemon: stopping");
            } else {
                println!("askhuman daemon: not running");
            }
            0
        })
    }

    fn restart_cmd() -> i32 {
        block_on(async {
            let _ = client::request_stop().await;
            client::wait_until_down(Duration::from_secs(5)).await;
            match client::ensure_running().await {
                Ok(()) => {
                    println!("askhuman daemon: restarted");
                    0
                }
                Err(e) => {
                    eprintln!("failed to restart daemon: {}", e);
                    1
                }
            }
        })
    }

    fn status_cmd() -> i32 {
        block_on(async {
            match client::request_status().await {
                Some(info) => {
                    print_status(&info);
                    0
                }
                None => {
                    println!("askhuman daemon: not running");
                    1
                }
            }
        })
    }

    fn logs_cmd() -> i32 {
        let p = lifecycle::log_path();
        println!("daemon log: {}", p.display());
        println!("(tip: tail -f {})", p.display());
        0
    }

    fn print_status(info: &StatusInfo) {
        println!("askhuman daemon: running");
        println!("  pid        {}", info.pid);
        println!(
            "  version    {} (protocol {})",
            info.version, info.protocol_version
        );
        println!("  uptime     {}s", info.uptime_secs);
        println!("  socket     {}", info.socket);
        println!("  requests   {} active", info.active_requests);
        let im = if info.im_connections.is_empty() {
            "none".to_string()
        } else {
            info.im_connections.join(", ")
        };
        println!("  im conns   {}", im);
    }
}
