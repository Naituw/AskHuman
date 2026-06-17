//! 常驻 Daemon：子命令分发（run/start/stop/restart/status/logs）+ Phase 0 的空 Daemon 服务。
//!
//! Phase 0：起一个不承载任何渠道的空 Daemon，提供握手（含二进制指纹换新）、status、stop、
//! 单实例（flock）、自启、空闲退出。渠道 / 弹窗 / 提交将在后续 Phase 接入。

#[cfg(unix)]
pub mod config_watch;
pub mod lifecycle;
pub mod request;
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
    use crate::agents::registry::AgentRegistry;
    use crate::agents::{AgentKind, LifecycleEvent};
    use crate::channels::dingding::DingTalkChannel;
    use crate::channels::feishu::FeishuChannel;
    use crate::channels::slack::SlackChannel;
    use crate::channels::telegram::TelegramChannel;
    use crate::channels::Channel;
    use crate::client;
    use crate::config::AppConfig;
    use crate::dingtalk::router::DdRouter;
    use crate::feishu::router::FsRouter;
    use crate::i18n::Lang;
    use crate::ipc::{
        self, transport, ClientMsg, DetectRequest, HelloAck, HelloStatus, ServerMsg, StatusInfo,
        TaskRequest,
    };
    use crate::models::{ChannelAction, ChannelResult};
    use crate::slack::router::SlRouter;
    use crate::telegram::router::TgRouter;
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
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
        let force = args.iter().skip(1).any(|a| a == "--force");
        // 子命令后仅允许 `--force`（且只对 stop/restart 有意义），其余一律报错。
        if let Some(extra) = args.iter().skip(1).find(|a| a.as_str() != "--force") {
            eprintln!("unknown daemon argument: {}", extra);
            return 1;
        }
        match args.first().map(|s| s.as_str()).unwrap_or("") {
            "run" => run_cmd(),
            "start" => start_cmd(),
            "stop" => stop_cmd(force),
            "restart" => restart_cmd(force),
            "status" => status_cmd(),
            "logs" => logs_cmd(),
            "" => {
                eprintln!("usage: AskHuman daemon <run|start|stop [--force]|restart [--force]|status|logs>");
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
        /// 排空状态（graceful drain）：true 时拒绝新 Submit/Detect，在途请求服务到完结后退出。
        draining: AtomicBool,
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
        /// 版本自更新快照（后台检查 / 指纹监听维护，握手与广播据此告知弹窗）。
        update: Mutex<UpdateSnapshot>,
        /// Agent 生命周期注册表（实验性功能，spec D3）。
        agents: Arc<AgentRegistry>,
        /// 状态窗口订阅者的发送端列表（变化 / 心跳时推 `AgentsState`）。
        agent_subs: Mutex<Vec<tokio::sync::mpsc::UnboundedSender<ServerMsg>>>,
        /// 菜单栏宿主订阅者的发送端列表（变化 / 心跳时推 `TrayState`）。
        /// **非保活**：该列表不参与空闲退出判定（见 `handle_tray_sub`），图标不得续命 daemon。
        tray_subs: Mutex<Vec<tokio::sync::mpsc::UnboundedSender<ServerMsg>>>,
        /// 「IM 会话期自动激活」当前活跃槽（持久化、跨重启保留，仅由入站消息改变）。
        active_channel: Mutex<Option<String>>,
        /// 已启动入站监听器的注册表（防重复 spawn + 改配置时主动停旧监听并重建）。
        inbound_listeners: InboundRegistry,
    }

    /// 入站监听注册表：渠道 id → 该监听任务的停止信号（`Notify`）。
    ///
    /// 既防重复 spawn（已认领的渠道不再起第二个监听），又支持「改配置时主动停掉旧监听」：
    /// `take` 出 stop 后 `notify` 即让旧任务退出、同时立刻释放认领，便于按新连接重建。
    /// **释放按身份（`Arc::ptr_eq`）判定**：旧任务退出时只移除「自己那次」的认领，绝不误删
    /// 配置变更后新建监听的认领（否则会出现两个监听并存）。
    #[derive(Default)]
    struct InboundRegistry {
        inner: Mutex<HashMap<String, Arc<tokio::sync::Notify>>>,
    }

    impl InboundRegistry {
        /// 认领某渠道的监听位：未被认领则插入新 stop 信号并返回；已被认领返回 None。
        fn claim(&self, id: &str) -> Option<Arc<tokio::sync::Notify>> {
            let mut map = self.inner.lock().unwrap();
            if map.contains_key(id) {
                return None;
            }
            let stop = Arc::new(tokio::sync::Notify::new());
            map.insert(id.to_string(), stop.clone());
            Some(stop)
        }

        /// 取出并移除某渠道的 stop 信号（改配置时用：拿到后 notify 停旧监听、同时释放认领）。
        fn take(&self, id: &str) -> Option<Arc<tokio::sync::Notify>> {
            self.inner.lock().unwrap().remove(id)
        }

        /// 身份安全释放：仅当当前登记的 stop 与 `token` 是同一实例（`Arc::ptr_eq`）时才移除。
        /// 监听任务退出时调——避免「改配置后旧任务迟到释放」误删新监听的认领。
        fn release(&self, id: &str, token: &Arc<tokio::sync::Notify>) {
            let mut map = self.inner.lock().unwrap();
            if map.get(id).map(|s| Arc::ptr_eq(s, token)).unwrap_or(false) {
                map.remove(id);
            }
        }
    }

    /// Daemon 维护的「自更新」当前态（广播给弹窗用）。
    #[derive(Clone, Default)]
    struct UpdateSnapshot {
        /// 远端有更新且未被忽略。
        available: bool,
        /// 最新正式版版本号。
        latest_version: String,
        /// 新二进制已落盘、待 drain 换新生效。
        pending: bool,
    }

    /// 启动时从 `update.json` 还原快照：available 据本地版本重算；
    /// pending 一律清零——刚启动的 daemon 运行的就是盘上二进制，不存在「更新的二进制在等生效」，
    /// 若残留 pending（上次换新前由旧 daemon 落盘）会让弹窗错误地常驻「待生效」横条。
    fn init_update_snapshot() -> UpdateSnapshot {
        let st = crate::update::state::load();
        let available = !st.latest_version.is_empty()
            && crate::update::compare_versions(
                &st.latest_version,
                &crate::update::current_version(),
            ) > 0
            && !st
                .dismissed_versions
                .iter()
                .any(|v| v == &st.latest_version);
        if st.pending {
            crate::update::state::set_pending(false);
        }
        UpdateSnapshot {
            available,
            latest_version: st.latest_version,
            pending: false,
        }
    }

    /// 由当前快照构造广播消息。
    fn update_state_msg(state: &ServerState) -> ServerMsg {
        let u = state.update.lock().unwrap().clone();
        ServerMsg::UpdateState {
            available: u.available,
            latest_version: u.latest_version,
            pending: u.pending,
        }
    }

    /// 后台检查一次更新：查远端 → 落 `update.json` → 更新快照 → 有变化则广播。失败静默。
    async fn check_for_update(state: &Arc<ServerState>) {
        match crate::update::check().await {
            Ok(info) => {
                crate::update::state::record_check(&info.latest_version, &info.release_notes);
                let dismissed = crate::update::state::is_dismissed(&info.latest_version);
                let available = info.available && !dismissed;
                let changed = {
                    let mut u = state.update.lock().unwrap();
                    let changed =
                        u.available != available || u.latest_version != info.latest_version;
                    u.available = available;
                    u.latest_version = info.latest_version.clone();
                    changed
                };
                if changed {
                    state.registry.broadcast_to_guis(update_state_msg(state));
                    broadcast_tray_state(state);
                }
            }
            Err(e) => log(&format!("update check failed: {}", e)),
        }
    }

    /// 检测盘上二进制是否已被换新（应用内更新或外部 npm 更新）。变化则标记 pending + 广播。
    fn check_pending_update(state: &Arc<ServerState>) {
        let now_fp = lifecycle::current_fingerprint();
        if now_fp == state.startup_fp {
            return;
        }
        let changed = {
            let mut u = state.update.lock().unwrap();
            let was = u.pending;
            u.pending = true;
            !was
        };
        if changed {
            crate::update::state::set_pending(true);
            log("binary on disk changed; marking update pending");
            state.registry.broadcast_to_guis(update_state_msg(state));
            broadcast_tray_state(state);
            // 主动换新（与 Hello 分支同一套语义，spec self-update）：盘上二进制已变即触发排空换新，
            // 不再被动等下一次握手。长连接（状态窗口订阅 / 工作中 agent）只保活、自身不再发 Hello，
            // 若无人握手旧 daemon 会一直停在旧二进制——这里主动 begin_drain 补上：有在途 ASK 则排空到
            // 完结再退（不打断），无在途立即退；退出前 persist 的 agent 注册表由新 daemon load 复核恢复。
            // 受 ASKHUMAN_DAEMON_AUTORESTART 开关控制（默认开），与 Hello 分支保持一致。
            let auto_restart = std::env::var("ASKHUMAN_DAEMON_AUTORESTART")
                .map(|v| v != "0")
                .unwrap_or(true);
            if auto_restart {
                log("binary on disk changed; draining for restart");
                begin_drain(state);
            }
        }
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

        // Ensure the user hooks directory exists and includes a sample script.
        crate::hooks::ensure_sample();

        let state = Arc::new(ServerState {
            startup_fp,
            started_at,
            active: AtomicUsize::new(0),
            last_active: Mutex::new(Instant::now()),
            shutdown: tokio::sync::Notify::new(),
            draining: AtomicBool::new(false),
            registry: RequestRegistry::new(),
            dd_router: tokio::sync::Mutex::new(None),
            fs_router: tokio::sync::Mutex::new(None),
            tg_router: tokio::sync::Mutex::new(None),
            sl_router: tokio::sync::Mutex::new(None),
            config: Mutex::new(AppConfig::load()),
            update: Mutex::new(init_update_snapshot()),
            agents: Arc::new(AgentRegistry::load()),
            agent_subs: Mutex::new(Vec::new()),
            tray_subs: Mutex::new(Vec::new()),
            active_channel: Mutex::new(crate::autochannel::load_active()),
            inbound_listeners: InboundRegistry::default(),
        });

        // 空闲退出检查。
        {
            let state = state.clone();
            tokio::spawn(async move {
                let timeout = idle_timeout();
                loop {
                    tokio::time::sleep(Duration::from_secs(30)).await;
                    // 空闲退出守卫（spec D18）：仅当无在途请求、无状态窗口订阅、**且**无「工作中」
                    // agent 时才计空闲。空闲 agent 不保活；版本更新 drain 由 begin_drain 独立处理、不受此影响。
                    if state.active.load(Ordering::SeqCst) == 0
                        && state.agents.working_count() == 0
                        && !has_agent_subs(&state)
                    {
                        let idle = state
                            .last_active
                            .lock()
                            .map(|t| t.elapsed())
                            .unwrap_or_default();
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

        // 版本自更新：后台定期检查远端最新版（启动稍延迟一次，之后每 24h），有变化广播给弹窗。
        {
            let state = state.clone();
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_secs(20)).await;
                loop {
                    check_for_update(&state).await;
                    tokio::time::sleep(Duration::from_secs(24 * 3600)).await;
                }
            });
        }

        // 版本自更新：周期监听盘上二进制指纹（应用内更新 / 外部 npm 更新）→ 标记待生效并广播。
        {
            let state = state.clone();
            tokio::spawn(async move {
                loop {
                    tokio::time::sleep(Duration::from_secs(15)).await;
                    check_pending_update(&state);
                }
            });
        }

        // Agent 生命周期：进程存活轮询 + TTL 兜底（spec D5/D12）。变化即持久化 + 推快照；
        // 有订阅窗口时每个 tick 都推（窗口的相对时间由前端自身 ticker 每秒刷新，这里只为状态变化兜底）。
        // 间隔取 15s：正常退出走事件驱动即时反映；只有 kill/崩溃这类漏事件才靠轮询兜底，
        // 15s 的判定延迟可接受，又能显著降低 daemon 空转唤醒。
        {
            let state = state.clone();
            tokio::spawn(async move {
                loop {
                    tokio::time::sleep(Duration::from_secs(15)).await;
                    let changed = state.agents.poll_liveness() | state.agents.ttl_sweep();
                    if changed {
                        state.agents.persist();
                    }
                    if changed || has_agent_subs(&state) {
                        broadcast_agents_state(&state);
                    }
                    // 菜单栏宿主在线时周期刷新（uptime / IM 连接等随时间变化的字段）。
                    if has_tray_subs(&state) {
                        broadcast_tray_state(&state);
                    }
                }
            });
        }

        // 菜单栏图标已开启则兜底拉起 GUI 宿主（单实例去重；always 主要靠登录项）。
        maybe_spawn_gui_host(&state.config.lock().unwrap().clone());

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

        // 退出前持久化 agent 注册表（spec D18：换新 / 闲退后由新 daemon 重载复核）。
        state.agents.persist();

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
        /// 状态窗口订阅：接管连接，持续推送 agent 快照。
        AgentsSub,
        /// 菜单栏宿主订阅：接管连接，持续推送 `TrayState`（非保活）。
        TraySub,
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
            Control::AgentsSub => handle_agents_sub(reader, w, &state).await,
            Control::TraySub => handle_tray_sub(reader, w, &state).await,
            Control::Closed => {}
        }

        if let Ok(mut t) = state.last_active.lock() {
            *t = Instant::now();
        }
        state.active.fetch_sub(1, Ordering::SeqCst);
    }

    /// 控制阶段：处理 Hello / Status / Stop（即时应答）；遇到 Submit / GuiHello 返回以便接管连接。
    async fn control_loop(
        reader: &mut Reader,
        w: &mut OwnedWriteHalf,
        state: &Arc<ServerState>,
    ) -> Control {
        loop {
            let msg: Option<ClientMsg> = match ipc::read_msg(reader).await {
                Ok(m) => m,
                Err(e) => {
                    log(&format!("read error: {}", e));
                    return Control::Closed;
                }
            };
            let Some(msg) = msg else {
                return Control::Closed;
            }; // EOF / 对端关闭

            match msg {
                ClientMsg::Hello(hello) => {
                    // 已在排空：一律回 Draining（客户端等下线后用新二进制拉起重试）。
                    if state.draining.load(Ordering::SeqCst) {
                        let ack = HelloAck {
                            protocol_version: ipc::PROTOCOL_VERSION,
                            daemon_version: version(),
                            status: HelloStatus::Draining,
                            reason: Some("draining: waiting for active requests".to_string()),
                        };
                        let _ = ipc::write_msg(w, &ServerMsg::HelloAck(ack)).await;
                        continue;
                    }
                    let now_fp = lifecycle::current_fingerprint();
                    // 指纹按「内容哈希」比对（与路径/mtime 无关）：
                    // 自身盘上二进制内容被换 / 客户端二进制内容不一致 / 协议不一致 → 过时，让位换新。
                    let stale = now_fp != state.startup_fp
                        || hello.fingerprint != state.startup_fp
                        || hello.protocol_version != ipc::PROTOCOL_VERSION;
                    let auto_restart = std::env::var("ASKHUMAN_DAEMON_AUTORESTART")
                        .map(|v| v != "0")
                        .unwrap_or(true);
                    // 过时且有在途请求 → 进入排空（不打断在途）；无在途 → 立即换新（零延迟）。
                    let draining = stale && auto_restart && state.registry.active_count() > 0;
                    let restarting = stale && auto_restart && !draining;
                    let ack = HelloAck {
                        protocol_version: ipc::PROTOCOL_VERSION,
                        daemon_version: version(),
                        status: if draining {
                            HelloStatus::Draining
                        } else if restarting {
                            HelloStatus::Restarting
                        } else {
                            HelloStatus::Ok
                        },
                        reason: if stale && auto_restart {
                            Some("binary or protocol changed".to_string())
                        } else {
                            None
                        },
                    };
                    let _ = ipc::write_msg(w, &ServerMsg::HelloAck(ack)).await;
                    if draining {
                        log("stale binary/protocol detected with active requests; draining");
                        begin_drain(state);
                        continue;
                    }
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
                        draining: state.draining.load(Ordering::SeqCst),
                    };
                    let _ = ipc::write_msg(w, &ServerMsg::Status(info)).await;
                }
                ClientMsg::Stop { force } => {
                    let _ = ipc::write_msg(w, &ServerMsg::Stopping).await;
                    // 默认 graceful：有在途请求时排空后退出；`--force` 或无在途 → 立即退出。
                    if !force && state.registry.active_count() > 0 {
                        log("graceful stop requested; draining");
                        begin_drain(state);
                        return Control::Closed;
                    }
                    log("stop requested");
                    state.shutdown.notify_one();
                    return Control::Closed;
                }
                ClientMsg::Submit(task) => return Control::Submit(task),
                ClientMsg::GuiHello { token } => return Control::Gui(token),
                // Agent 生命周期事件上报（即发即走）：更新注册表，变化则持久化 + 推订阅窗口。
                ClientMsg::AgentEvent {
                    agent,
                    event,
                    session_id,
                    pid,
                    cwd,
                    ts,
                } => {
                    if let (Some(kind), Some(ev)) =
                        (AgentKind::parse(&agent), LifecycleEvent::parse(&event))
                    {
                        let changed = state
                            .agents
                            .apply_event(kind, ev, &session_id, pid, cwd, ts);
                        if changed {
                            state.agents.persist();
                            broadcast_agents_state(state);
                        }
                        // turn-start 经此即时上线 IM 入站消费（与开关无关，使 /here、/status 在 agent
                        // 工作期间随时可用）；幂等。连接随守护进程退出而释放，无需在 turn-end 主动断。
                        ensure_inbound_listeners(state).await;
                    }
                }
                // 状态窗口订阅：接管连接持续推送。
                ClientMsg::AgentsSubscribe => return Control::AgentsSub,
                // 菜单栏宿主订阅：接管连接持续推送 TrayState（非保活）。
                ClientMsg::TraySubscribe => return Control::TraySub,
                // 自动识别（Q6）：就地处理（可能阻塞至多 120s 等用户发码），完成后回结果继续循环。
                // 排空期拒绝（兜底；正常情况下客户端在 Hello 即被挡住而回退进程内识别）。
                ClientMsg::Detect(req) => {
                    if state.draining.load(Ordering::SeqCst) {
                        let _ = ipc::write_msg(
                            w,
                            &ServerMsg::Error {
                                message: "daemon is draining".to_string(),
                            },
                        )
                        .await;
                    } else {
                        handle_detect(&req, state, reader, w).await
                    }
                }
                // Answer 只应在 GUI 接管阶段出现；控制阶段收到即忽略。
                ClientMsg::Answer { .. } => {}
            }
        }
    }

    /// 进入排空状态（幂等）：首次置位时 spawn 看门狗，在途请求全部完结后触发退出。
    fn begin_drain(state: &Arc<ServerState>) {
        if state.draining.swap(true, Ordering::SeqCst) {
            return; // 已在排空。
        }
        broadcast_tray_state(state);
        let state = state.clone();
        tokio::spawn(async move {
            loop {
                if state.registry.active_count() == 0 {
                    log("drain complete; shutting down");
                    state.shutdown.notify_one();
                    return;
                }
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
        });
    }

    /// CLI 提交一次任务：建请求、spawn GUI Helper、流式回结果；CLI 断开则取消。
    async fn handle_submit(
        task: TaskRequest,
        mut reader: Reader,
        mut w: OwnedWriteHalf,
        state: &Arc<ServerState>,
    ) {
        // 排空闸门（兜底，覆盖「Hello Ok → Submit 间隙开始排空」的竞态）：拒绝并断开，
        // 客户端等本 Daemon 下线后用新二进制重新提交。
        if state.draining.load(Ordering::SeqCst) {
            let _ = ipc::write_msg(
                &mut w,
                &ServerMsg::Draining {
                    active: state.registry.active_count(),
                },
            )
            .await;
            return;
        }
        // 「自动激活」开启时：每次提问按「工作中」兜底登记会话（无 turn hook 也能驱动入站监听 / 切槽）；
        // 开关关时保持旧行为（仅刷新已追踪会话的活动，尊重「未装 hook = 不追踪」，不污染注册表）。
        let auto = AppConfig::load_without_secrets().channels.auto_activation;
        let changed = match (
            task.agent_kind.as_deref().and_then(AgentKind::parse),
            task.agent_session_id.clone(),
            task.agent_pid,
        ) {
            // 有 session_id（shell 工具子进程能从 env 拿到）：按 session 刷新。
            (Some(kind), Some(sid), pid) => {
                let cwd = Some(task.project.clone()).filter(|s| !s.trim().is_empty());
                // MCP 模式（`from_mcp`）下 `agent_session_id` 取自长驻 MCP server 的启动 env，可能过期；
                // 故即便「自动激活」开启也**只刷新已存在的 session、绝不新建**，避免造出幽灵会话。
                if auto && !task.from_mcp {
                    state.agents.upsert_working(kind, &sid, pid, cwd)
                } else {
                    state.agents.touch_activity(kind, &sid, pid)
                }
            }
            // 无 session_id 但有 pid（典型 **MCP 模式**：agent 把 MCP server 的 env 清空，本次仅靠进程树
            // walk 拿到 `(kind, pid)`）：按 pid 匹配已存在 session 刷新（只更新、绝不新建）。
            (Some(kind), None, Some(pid)) => state.agents.touch_activity_by_pid(kind, pid),
            _ => false,
        };
        if changed {
            state.agents.persist();
            broadcast_agents_state(state);
        }
        // 确保入站消费在线（自身按「有工作中 agent」自门控；与开关无关，使 /status 等命令独立可用）。
        ensure_inbound_listeners(state).await;
        let lang = Lang::resolve(&task.lang);
        let (entry, mut final_rx) = state.registry.create(task);
        let request_id = entry.request_id.clone();
        log(&format!("request {} accepted", request_id));

        // Fire `ask-received` once per accepted request, independent of popup state.
        crate::hooks::fire_ask_received(
            &request_id,
            &entry.show.source,
            &entry.show.project,
            &entry.show.request,
        );

        if ipc::write_msg(
            &mut w,
            &ServerMsg::Accepted {
                request_id: request_id.clone(),
            },
        )
        .await
        .is_err()
        {
            state.registry.remove(&request_id);
            return;
        }
        // 在途请求数 +1：刷新菜单栏状态（待答数 / 图标圆点）。
        broadcast_tray_state(state);

        // 挂接可用的 IM 渠道（钉钉/…）到本请求的协调器，与弹窗并行抢答。
        let im_attached = attach_im_channels(&entry, state, &mut w, lang).await;
        // IM 长连接可能在此刚建立，刷新菜单栏「已连 IM」。
        broadcast_tray_state(state);

        // spawn GUI Helper（独立短命进程，带一次性 token）。
        let popup_ok = match spawn_gui_helper(&entry.token) {
            Ok(()) => true,
            Err(e) => {
                log(&format!("failed to spawn GUI helper: {}", e));
                let _ = ipc::write_msg(
                    &mut w,
                    &ServerMsg::Warn {
                        text: format!(
                            "{}failed to spawn popup: {}",
                            crate::i18n::err_prefix(lang),
                            e
                        ),
                    },
                )
                .await;
                false
            }
        };

        // 既无弹窗也无 IM 渠道 → 无可用渠道，按错误收尾。
        if !popup_ok && !im_attached {
            let _ = ipc::write_msg(
                &mut w,
                &ServerMsg::Final {
                    stdout: String::new(),
                    exit_code: request::EXIT_NO_CHANNEL,
                },
            )
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
                let _ = ipc::write_msg(
                    &mut w,
                    &ServerMsg::Final {
                        stdout: o.stdout,
                        exit_code: o.exit_code,
                    },
                )
                .await;
            }
            None => {
                // 渲染通道意外关闭：判异常退出码 3。
                let _ = ipc::write_msg(
                    &mut w,
                    &ServerMsg::Final {
                        stdout: String::new(),
                        exit_code: 3,
                    },
                )
                .await;
            }
        }
        // 「在哪个渠道作答就用哪个」：把活跃槽更新为本次作答渠道（弹窗作答 → "popup"，即不再发 IM）。
        // 若由此从某 IM 切走，旧 IM 在 set_active_channel 内收反激活提示。仅自动激活开时生效。
        if auto {
            if let Some(winner) = entry.coordinator.winner_channel_id() {
                set_active_channel(state, &winner).await;
            }
        }
        entry.cancel.notify_waiters();
        state.registry.remove(&request_id);
        // 在途请求数 -1：刷新菜单栏状态。
        broadcast_tray_state(state);
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

        // 握手即带上当前自更新态（有更新 / 待生效），使弹窗一打开就知道，无需等下次广播。
        {
            let u = state.update.lock().unwrap();
            if u.available || u.pending {
                let _ = gui_tx.send(ServerMsg::UpdateState {
                    available: u.available,
                    latest_version: u.latest_version.clone(),
                    pending: u.pending,
                });
            }
        }

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

    /// 是否有状态窗口在订阅。
    fn has_agent_subs(state: &Arc<ServerState>) -> bool {
        state
            .agent_subs
            .lock()
            .map(|s| !s.is_empty())
            .unwrap_or(false)
    }

    /// 向所有状态窗口推送一次 agent 全量快照（顺带剔除已断开的发送端）。
    /// agent 忙闲变化也影响菜单栏状态，故顺带刷新 TrayState。
    fn broadcast_agents_state(state: &Arc<ServerState>) {
        let msg = ServerMsg::AgentsState {
            agents: state.agents.snapshot(),
        };
        if let Ok(mut subs) = state.agent_subs.lock() {
            subs.retain(|tx| tx.send(msg.clone()).is_ok());
        }
        broadcast_tray_state(state);
    }

    /// 状态窗口订阅连接：注册发送端、立即推一次快照，随后专用写任务持续推送；读端用于探测断开。
    /// 该连接保持期间计入 `active`（连同「工作中」agent 一起阻止 daemon 闲退，spec D18）。
    async fn handle_agents_sub(mut reader: Reader, w: OwnedWriteHalf, state: &Arc<ServerState>) {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<ServerMsg>();
        let mut w = w;
        let writer = tokio::spawn(async move {
            while let Some(msg) = rx.recv().await {
                if ipc::write_msg(&mut w, &msg).await.is_err() {
                    break;
                }
            }
        });
        // 注册订阅端并立即推一次当前快照。
        if let Ok(mut subs) = state.agent_subs.lock() {
            subs.push(tx.clone());
        }
        let _ = tx.send(ServerMsg::AgentsState {
            agents: state.agents.snapshot(),
        });

        // 读端仅用于探测断开；窗口正常不发消息。
        wait_cli_eof(&mut reader).await;

        // 收尾：从订阅表移除本端（按指针标识），结束写任务。
        if let Ok(mut subs) = state.agent_subs.lock() {
            subs.retain(|s| !s.same_channel(&tx));
        }
        drop(tx);
        let _ = writer.await;
    }

    /// 是否有菜单栏宿主在订阅 TrayState。
    fn has_tray_subs(state: &Arc<ServerState>) -> bool {
        state
            .tray_subs
            .lock()
            .map(|s| !s.is_empty())
            .unwrap_or(false)
    }

    /// 构造一帧整合的 `TrayState`（含 IM 连接、agent 忙闲、更新态）。需 await（读 tokio Mutex）。
    async fn build_tray_state(state: &Arc<ServerState>) -> ServerMsg {
        let u = { state.update.lock().unwrap().clone() };
        ServerMsg::TrayState {
            running: true,
            version: version(),
            uptime_secs: now_secs().saturating_sub(state.started_at),
            active_requests: state.registry.active_count(),
            im_connections: active_im_connections(state).await,
            draining: state.draining.load(Ordering::SeqCst),
            agents_working: state.agents.working_count(),
            agents_idle: state.agents.idle_count(),
            update_available: u.available,
            update_latest: u.latest_version,
            pending: u.pending,
        }
    }

    /// 向所有菜单栏宿主推送一帧 `TrayState`（顺带剔除已断开的发送端）。
    /// 因 `build_tray_state` 需 await 而本函数被大量同步调用点引用，故 spawn 一个任务异步构造并发送；
    /// 无订阅者时早退（廉价）。
    fn broadcast_tray_state(state: &Arc<ServerState>) {
        if !has_tray_subs(state) {
            return;
        }
        let state = state.clone();
        tokio::spawn(async move {
            let msg = build_tray_state(&state).await;
            if let Ok(mut subs) = state.tray_subs.lock() {
                subs.retain(|tx| tx.send(msg.clone()).is_ok());
            }
        });
    }

    /// 菜单栏宿主订阅连接（spec D10/D13）：注册发送端、立即推一帧，随后持续推送；读端探测断开。
    ///
    /// **关键：非保活。** `handle_conn` 在连接建立时对 `active` 自增了 1；这里立即 `fetch_sub(1)`
    /// 抵消，连接存续期间净占用为 0，再于退出前 `fetch_add(1)` 让 `handle_conn` 末尾的 `fetch_sub(1)`
    /// 归零。配合空闲判定不引用 `tray_subs`，从而图标订阅**不会**把 daemon 续命（spec D5 核心）。
    async fn handle_tray_sub(mut reader: Reader, w: OwnedWriteHalf, state: &Arc<ServerState>) {
        state.active.fetch_sub(1, Ordering::SeqCst);

        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<ServerMsg>();
        let mut w = w;
        let writer = tokio::spawn(async move {
            while let Some(msg) = rx.recv().await {
                if ipc::write_msg(&mut w, &msg).await.is_err() {
                    break;
                }
            }
        });
        // 注册订阅端并立即推一帧当前状态。
        if let Ok(mut subs) = state.tray_subs.lock() {
            subs.push(tx.clone());
        }
        let _ = tx.send(build_tray_state(state).await);

        // 读端仅用于探测断开；宿主正常不发消息。
        wait_cli_eof(&mut reader).await;

        // 收尾：从订阅表移除本端，结束写任务，恢复 active 计数。
        if let Ok(mut subs) = state.tray_subs.lock() {
            subs.retain(|s| !s.same_channel(&tx));
        }
        drop(tx);
        let _ = writer.await;
        state.active.fetch_add(1, Ordering::SeqCst);
    }

    /// 菜单栏图标是否在当前平台被支持（用于 daemon 决定是否兜底拉起宿主）。
    /// macOS 恒真；Linux 仅在存在图形会话时（保守门控，headless 不拉宿主）。
    fn tray_supported() -> bool {
        #[cfg(target_os = "macos")]
        {
            true
        }
        #[cfg(all(unix, not(target_os = "macos")))]
        {
            std::env::var_os("DISPLAY").is_some() || std::env::var_os("WAYLAND_DISPLAY").is_some()
        }
    }

    /// 按配置兜底拉起 GUI 宿主（spec D14）：`menu_bar_icon != off` 且托盘可用时尝试 spawn。
    /// 单实例由宿主自身 flock 去重；always 主要靠登录项，此处为兜底。失败静默。
    fn maybe_spawn_gui_host(config: &AppConfig) {
        use crate::config::MenuBarIconMode;
        if config.general.menu_bar_icon == MenuBarIconMode::Off {
            return;
        }
        if !tray_supported() {
            return;
        }
        if let Err(e) = crate::gui_host::spawn_detached() {
            log(&format!("failed to spawn gui-host: {}", e));
        }
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

        // 「IM 会话期自动激活」：开关开时，仅当前活跃槽对应的 IM 发卡片（其余 IM 由入站监听器保持连接、
        // 只监听 here，不发卡片）。开关关时维持旧「全发」行为。
        let auto = config.channels.auto_activation;
        let active = state.active_channel.lock().unwrap().clone();
        let want = |id: &str| -> bool { !auto || active.as_deref() == Some(id) };

        if want("dingding") && crate::app::is_dingding_active(&config) {
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
                    let _ = ipc::write_msg(
                        w,
                        &ServerMsg::Warn {
                            text: format!(
                                "{}{}",
                                crate::i18n::warn_prefix(lang),
                                crate::i18n::tr(lang, "channel.ddConfigInvalidSkip")
                                    .replace("{e}", "Stream connection failed"),
                            ),
                        },
                    )
                    .await;
                }
            }
        }

        if want("feishu") && crate::app::is_feishu_active(&config) {
            let fs = &config.channels.feishu;
            match ensure_fs_router(state, fs).await {
                Some(router) => {
                    let ch: Arc<dyn Channel> = Arc::new(FeishuChannel::shared(fs.clone(), router));
                    entry.coordinator.register(ch.clone());
                    ch.start(&request, sink.clone());
                    attached = true;
                }
                None => {
                    let _ = ipc::write_msg(
                        w,
                        &ServerMsg::Warn {
                            text: format!(
                                "{}{}",
                                crate::i18n::warn_prefix(lang),
                                crate::i18n::tr(lang, "channel.fsConfigInvalidSkip")
                                    .replace("{e}", "WebSocket connection failed"),
                            ),
                        },
                    )
                    .await;
                }
            }
        }

        if want("telegram") && crate::app::is_telegram_active(&config) {
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
                    let _ = ipc::write_msg(
                        w,
                        &ServerMsg::Warn {
                            text: format!(
                                "{}{}",
                                crate::i18n::warn_prefix(lang),
                                crate::i18n::tr(lang, "channel.tgConfigInvalidSkip")
                                    .replace("{e}", "poller start failed"),
                            ),
                        },
                    )
                    .await;
                }
            }
        }

        if want("slack") && crate::app::is_slack_active(&config) {
            let sl = &config.channels.slack;
            match ensure_sl_router(state, sl).await {
                Some(router) => {
                    let ch: Arc<dyn Channel> = Arc::new(SlackChannel::shared(sl.clone(), router));
                    entry.coordinator.register(ch.clone());
                    ch.start(&request, sink.clone());
                    attached = true;
                }
                None => {
                    let _ = ipc::write_msg(
                        w,
                        &ServerMsg::Warn {
                            text: format!(
                                "{}{}",
                                crate::i18n::warn_prefix(lang),
                                crate::i18n::tr(lang, "channel.slConfigInvalidSkip")
                                    .replace("{e}", "Socket Mode connection failed"),
                            ),
                        },
                    )
                    .await;
                }
            }
        }

        attached
    }

    // ===== IM 入站消费（命令 /here、/status…）/ 活跃槽 / 补推在途 =====

    /// 确保各已启用 IM 的入站消费任务在线，使守护进程在世期间能收到入站命令（`/here`、`/status`…）。
    /// 触发条件 = **存在「工作中」agent**：守护进程存活本就由工作 agent 生命周期约束（D18），连接随其
    /// 退出而释放（serve 收尾丢弃 Router → Drop 关长连接），故无需「反激活式」主动断连。
    /// **与「自动激活」开关无关**——`/status` 等命令是与开关独立的功能（开关只决定 §3.4 的切槽/发卡行为）。
    /// 各渠道只提供「连接 Router + 取原始消息观察者 + 抽取 (发送者, 文本) + 期望发送者」这几样传输原语；
    /// 通用循环与命令分派（`spawn_listener` / `handle_inbound`）一份实现，各渠道复用。幂等：可反复调用。
    async fn ensure_inbound_listeners(state: &Arc<ServerState>) {
        if state.agents.working_count() == 0 {
            return;
        }
        let config = AppConfig::load();

        if crate::app::is_feishu_active(&config) {
            if let Some(stop) = state.inbound_listeners.claim("feishu") {
                match ensure_fs_router(state, &config.channels.feishu).await {
                    Some(r) => spawn_listener(
                        state,
                        "feishu",
                        r.observe_message(),
                        extract_feishu,
                        config.channels.feishu.open_id.trim().to_string(),
                        stop,
                    ),
                    None => state.inbound_listeners.release("feishu", &stop),
                }
            }
        }

        if crate::app::is_dingding_active(&config) {
            if let Some(stop) = state.inbound_listeners.claim("dingding") {
                let dd = &config.channels.dingding;
                match ensure_dd_router(state, dd.client_id.trim(), dd.client_secret.trim()).await {
                    Some(r) => spawn_listener(
                        state,
                        "dingding",
                        r.observe_bot(),
                        extract_dingtalk,
                        dd.user_id.trim().to_string(),
                        stop,
                    ),
                    None => state.inbound_listeners.release("dingding", &stop),
                }
            }
        }

        if crate::app::is_slack_active(&config) {
            if let Some(stop) = state.inbound_listeners.claim("slack") {
                match ensure_sl_router(state, &config.channels.slack).await {
                    Some(r) => spawn_listener(
                        state,
                        "slack",
                        r.observe_message(),
                        extract_slack,
                        config.channels.slack.user_id.trim().to_string(),
                        stop,
                    ),
                    None => state.inbound_listeners.release("slack", &stop),
                }
            }
        }

        if crate::app::is_telegram_active(&config) {
            if let Some(stop) = state.inbound_listeners.claim("telegram") {
                match ensure_tg_router(state, &config.channels.telegram).await {
                    Some(r) => spawn_listener(
                        state,
                        "telegram",
                        r.observe_message(),
                        extract_telegram,
                        config.channels.telegram.chat_id.trim().to_string(),
                        stop,
                    ),
                    None => state.inbound_listeners.release("telegram", &stop),
                }
            }
        }
    }

    /// 通用入站监听循环（与渠道无关）：从原始消息流抽取 (发送者, 文本)，按期望发送者过滤后交 `handle_inbound`。
    /// 收到 `stop`（改配置时由 `invalidate_changed_routers` 触发）或流结束（连接断开）即退出，
    /// 退出时**按身份**释放监听位（不误删改配置后新建监听的认领），下次提问 / 配置变更可重建。
    fn spawn_listener(
        state: &Arc<ServerState>,
        channel_id: &'static str,
        mut rx: tokio::sync::mpsc::UnboundedReceiver<serde_json::Value>,
        extract: fn(&serde_json::Value) -> Option<(String, String)>,
        expected_sender: String,
        stop: Arc<tokio::sync::Notify>,
    ) {
        let state = state.clone();
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = stop.notified() => break,
                    ev = rx.recv() => match ev {
                        Some(ev) => {
                            if let Some((sender, text)) = extract(&ev) {
                                // 单聊机器人：仅处理期望发送者发来的消息（期望为空则不过滤）；过滤掉机器人自身回声。
                                if !expected_sender.is_empty() && sender != expected_sender {
                                    continue;
                                }
                                handle_inbound(&state, channel_id, &text).await;
                            }
                        }
                        None => break,
                    },
                }
            }
            state.inbound_listeners.release(channel_id, &stop);
        });
    }

    /// 飞书原始消息 → (发送者 open_id, 文本)；非文本返回 None。
    fn extract_feishu(ev: &serde_json::Value) -> Option<(String, String)> {
        fs_text_and_sender(ev)
    }

    /// 钉钉原始 bot 消息 → (senderStaffId, 文本)；非文本返回 None。
    fn extract_dingtalk(ev: &serde_json::Value) -> Option<(String, String)> {
        let sender = ev
            .get("senderStaffId")
            .and_then(|v| v.as_str())?
            .to_string();
        let text = ev
            .get("text")
            .and_then(|t| t.get("content"))
            .and_then(|c| c.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())?
            .to_string();
        Some((sender, text))
    }

    /// Slack 原始消息事件 → (user, 文本)；非文本 / 机器人自身消息返回 None。
    fn extract_slack(ev: &serde_json::Value) -> Option<(String, String)> {
        let user = ev.get("user").and_then(|v| v.as_str())?.to_string();
        let text = ev
            .get("text")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())?
            .to_string();
        Some((user, text))
    }

    /// Telegram 原始 `message` 对象 → (chat id, 文本)；非文本返回 None。
    fn extract_telegram(ev: &serde_json::Value) -> Option<(String, String)> {
        let chat = ev
            .get("chat")
            .and_then(|c| c.get("id"))
            .and_then(|v| v.as_i64())?
            .to_string();
        let text = ev
            .get("text")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())?
            .to_string();
        Some((chat, text))
    }

    /// 统一入站分派（与渠道无关）：`/` 开头按内置命令处理，否则按普通消息。
    /// - `/status`：**与「自动激活」开关无关**，始终回状态文本（开关开且因此切槽时附激活回执）。
    /// - `/here` 与「普通消息切槽」：仅「自动激活」开时生效；关时 `/here` 静默忽略、普通文字
    ///   交由卡片作答（观察者不处理）。切槽细则见设计 §3.4。补推在途已下沉到 `set_active_channel`。
    async fn handle_inbound(state: &Arc<ServerState>, channel_id: &str, text: &str) {
        let lang = Lang::current();
        let config = AppConfig::load();
        let auto = config.channels.auto_activation;
        match crate::autochannel::parse_command(text) {
            Some(crate::autochannel::Command::Status) => {
                // 状态查询是独立功能：始终响应。仅当开关开、且本次因 /status 切了活跃槽时附激活回执。
                let (switched, n) = if auto {
                    set_active_channel(state, channel_id).await
                } else {
                    (false, 0)
                };
                let mut body = String::new();
                if switched {
                    body.push_str(&crate::autochannel::activated_receipt(n, lang));
                    body.push_str("\n\n");
                }
                body.push_str(&crate::autochannel::status_text(
                    &state.agents.snapshot(),
                    lang,
                ));
                let _ = reply_channel_text(channel_id, &config, &body).await;
            }
            Some(crate::autochannel::Command::Here) => {
                if !auto {
                    return; // 关态下「活跃槽」概念不存在：静默忽略。
                }
                // 激活 + 补推（在 set_active_channel 内完成）；/here 始终回执（即便已是当前槽，n=0）。
                let (_switched, n) = set_active_channel(state, channel_id).await;
                let _ = reply_channel_text(
                    channel_id,
                    &config,
                    &crate::autochannel::activated_receipt(n, lang),
                )
                .await;
            }
            None => {
                if !auto {
                    return; // 关态：普通文字交卡片作答，观察者不切槽、不补推。
                }
                // 普通消息：切槽 + 补推（set_active_channel 内）+（仅当发生切换时）回执；文本本身不当答案。
                let (switched, n) = set_active_channel(state, channel_id).await;
                if switched {
                    let _ = reply_channel_text(
                        channel_id,
                        &config,
                        &crate::autochannel::activated_receipt(n, lang),
                    )
                    .await;
                }
            }
        }
    }

    /// 把活跃槽切到 `new_id`（IM id 或 "popup"）。统一入口：「在哪个渠道说话 / 作答就用哪个」。
    /// 切换时：持久化 → 给**旧**渠道（若为 IM）发反激活提示 → 把**在途未答**问题补推给**新**渠道
    /// （若为 IM）。补推是「渠道激活」的固有行为，与触发方式无关（`/here`、普通消息、`/status`、作答切槽均同）。
    /// 返回 `(是否切换, 补推条数)`；新渠道激活回执文案由调用方按场景发送（弹窗无需）。
    async fn set_active_channel(state: &Arc<ServerState>, new_id: &str) -> (bool, usize) {
        let prev = {
            let mut guard = state.active_channel.lock().unwrap();
            if guard.as_deref() == Some(new_id) {
                return (false, 0);
            }
            let prev = guard.take();
            *guard = Some(new_id.to_string());
            prev
        };
        crate::autochannel::save_active(Some(new_id));
        log(&format!("auto-channel: active slot -> {}", new_id));
        let cfg = AppConfig::load();
        // 旧渠道反激活提示（仅真实 IM；"popup" / None 无收件端，跳过）。
        if let Some(old) = prev {
            if old != "popup" && old != new_id {
                let _ = reply_channel_text(
                    &old,
                    &cfg,
                    &crate::autochannel::deactivated_receipt(new_id, Lang::current()),
                )
                .await;
            }
        }
        // 激活即补推在途（仅真实 IM；弹窗无卡片概念）。
        let backfilled = if new_id != "popup" {
            backfill_inflight(state, new_id, &cfg).await
        } else {
            0
        };
        (true, backfilled)
    }

    /// 把所有「在途未答」问题补推为 `channel_id` 的卡片（已挂接该渠道的请求跳过，避免重发）。返回补推数。
    async fn backfill_inflight(
        state: &Arc<ServerState>,
        channel_id: &str,
        config: &AppConfig,
    ) -> usize {
        let mut n = 0;
        for entry in state.registry.in_flight_entries() {
            if entry.coordinator.has_channel(channel_id) {
                continue;
            }
            if let Some(ch) = build_im_channel(channel_id, config, state).await {
                entry.coordinator.register(ch.clone());
                ch.start(&entry.show.request, entry.coordinator.clone());
                n += 1;
            }
        }
        n
    }

    /// 为补推构造一个挂共享 Router 的渠道实例（各渠道仅此处差异：取对应 Router + 构造对应 Channel）。
    async fn build_im_channel(
        channel_id: &str,
        config: &AppConfig,
        state: &Arc<ServerState>,
    ) -> Option<Arc<dyn Channel>> {
        let ch: Arc<dyn Channel> = match channel_id {
            "feishu" => {
                let router = ensure_fs_router(state, &config.channels.feishu).await?;
                Arc::new(FeishuChannel::shared(
                    config.channels.feishu.clone(),
                    router,
                ))
            }
            "dingding" => {
                let dd = &config.channels.dingding;
                let router =
                    ensure_dd_router(state, dd.client_id.trim(), dd.client_secret.trim()).await?;
                Arc::new(DingTalkChannel::shared(dd.clone(), router))
            }
            "slack" => {
                let router = ensure_sl_router(state, &config.channels.slack).await?;
                Arc::new(SlackChannel::shared(config.channels.slack.clone(), router))
            }
            "telegram" => {
                let router = ensure_tg_router(state, &config.channels.telegram).await?;
                Arc::new(TelegramChannel::shared(
                    config.channels.telegram.clone(),
                    router,
                ))
            }
            _ => return None,
        };
        Some(ch)
    }

    /// 向某渠道回一条纯文本（回执 / 状态）。各渠道仅此处差异：用对应 OpenAPI client 发文本。
    async fn reply_channel_text(
        channel_id: &str,
        config: &AppConfig,
        text: &str,
    ) -> Result<(), String> {
        match channel_id {
            "feishu" => {
                let client = crate::feishu::client::FeishuClient::new(&config.channels.feishu)
                    .map_err(|e| e.to_string())?;
                client
                    .send_text(text)
                    .await
                    .map(|_| ())
                    .map_err(|e| e.to_string())
            }
            "dingding" => {
                let client =
                    crate::dingtalk::client::DingTalkClient::new(&config.channels.dingding)
                        .map_err(|e| e.to_string())?;
                client.send_oto_text(text).await.map_err(|e| e.to_string())
            }
            "slack" => {
                let client = crate::slack::client::SlackClient::new(&config.channels.slack)
                    .map_err(|e| e.to_string())?;
                let channel = client.open_dm().await.map_err(|e| e.to_string())?;
                client
                    .post_text(&channel, text)
                    .await
                    .map(|_| ())
                    .map_err(|e| e.to_string())
            }
            "telegram" => {
                let tg = &config.channels.telegram;
                let client = crate::telegram::TelegramClient::new(
                    tg.bot_token.clone(),
                    tg.chat_id.clone(),
                    tg.api_base_url.clone(),
                )
                .map_err(|e| e.to_string())?;
                client
                    .send_message(text, None, None)
                    .await
                    .map(|_| ())
                    .map_err(|e| e.to_string())
            }
            _ => Err(format!("reply unsupported for channel: {}", channel_id)),
        }
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
    async fn handle_detect(
        req: &DetectRequest,
        state: &Arc<ServerState>,
        reader: &mut Reader,
        w: &mut OwnedWriteHalf,
    ) {
        let lang = Lang::resolve(&req.lang);
        let work = async {
            match req.kind.as_str() {
                "dingtalk" => detect_dingtalk(state, req, lang).await,
                "feishu" => detect_feishu(state, req, lang).await,
                "slack" => detect_slack(state, req, lang).await,
                other => Err(format!("unknown detect kind: {}", other)),
            }
        };
        // 识别可能阻塞至多 120s。其间同时监听控制连接：设置进程点「取消」会丢弃 wait 命令的
        // future 并关闭这条连接，`wait_conn_closed` 即返回 → 丢弃 `work`（连带 drop 掉临时长连接，
        // 不残留），不再回包。正常完成则回 `Detected`/`Error`。
        tokio::select! {
            result = work => {
                let msg = match result {
                    Ok(id) => ServerMsg::Detected { id },
                    Err(message) => ServerMsg::Error { message },
                };
                let _ = ipc::write_msg(w, &msg).await;
            }
            _ = wait_conn_closed(reader) => {
                log("detect cancelled by client (connection closed)");
            }
        }
    }

    /// 等到该控制连接关闭/出错（或对端发来任何消息）即返回——用于在 detect 等待期间感知客户端取消。
    async fn wait_conn_closed(reader: &mut Reader) {
        let _ = ipc::read_msg::<_, ClientMsg>(reader).await;
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
        let content_str = message
            .get("content")
            .and_then(|v| v.as_str())
            .unwrap_or("{}");
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
        // 凭据/收件人变更已停掉旧入站监听（见 invalidate_changed_routers）；这里立即按新配置重建，
        // 使 `/here`、`/status`、普通消息切槽无需等在途请求结束或 daemon 重启即恢复（有工作中 agent 时）。
        ensure_inbound_listeners(state).await;
        let general = serde_json::to_value(&new.general).unwrap_or(serde_json::Value::Null);
        state
            .registry
            .broadcast_to_guis(ServerMsg::ConfigChanged { general });
        // 配置变更可能刚开启菜单栏图标 → 兜底拉起宿主（宿主自身也监听配置，二者均幂等）。
        maybe_spawn_gui_host(&new);
        log("config reloaded");
    }

    /// 比对新旧配置：凭据变更或渠道被禁用 → 丢弃对应缓存 Router（惰性失效，Q1）+ 停掉旧入站监听。
    ///
    /// 进行中的请求仍持有自己的 Router `Arc` 克隆，故其连接保留到该请求结束；
    /// 下一个请求会经 `ensure_*_router` 用新配置重连。注意：若仅改了同 client_id 的 secret，
    /// 且旧请求未结束时新请求又到达，可能短暂出现两条同 client_id 连接（平台会踢掉旧的）——
    /// 属配置在「问题进行中」被改动的少见边角，可接受。
    ///
    /// 入站监听单独处理：除凭据变更（连带 Router）外，**收件人 id 变更**（feishu open_id /
    /// dingding·slack user_id / telegram chat_id，即监听的 expected_sender）也需停旧监听——否则监听
    /// 仍用旧过滤条件绑在旧连接上，`on_config_changed` 随后会按新配置重建（`stop_listener` 释放认领）。
    async fn invalidate_changed_routers(
        state: &Arc<ServerState>,
        old: &AppConfig,
        new: &AppConfig,
    ) {
        let dd_router_changed = !crate::app::is_dingding_active(new)
            || old.channels.dingding.client_id != new.channels.dingding.client_id
            || old.channels.dingding.client_secret != new.channels.dingding.client_secret;
        if dd_router_changed {
            *state.dd_router.lock().await = None;
        }
        if dd_router_changed || old.channels.dingding.user_id != new.channels.dingding.user_id {
            stop_listener(state, "dingding");
        }

        let fs_router_changed = !crate::app::is_feishu_active(new)
            || old.channels.feishu.app_id != new.channels.feishu.app_id
            || old.channels.feishu.app_secret != new.channels.feishu.app_secret
            || old.channels.feishu.base_url != new.channels.feishu.base_url;
        if fs_router_changed {
            *state.fs_router.lock().await = None;
        }
        if fs_router_changed || old.channels.feishu.open_id != new.channels.feishu.open_id {
            stop_listener(state, "feishu");
        }

        let tg_router_changed = !crate::app::is_telegram_active(new)
            || old.channels.telegram.bot_token != new.channels.telegram.bot_token
            || old.channels.telegram.api_base_url != new.channels.telegram.api_base_url;
        if tg_router_changed {
            *state.tg_router.lock().await = None;
        }
        if tg_router_changed || old.channels.telegram.chat_id != new.channels.telegram.chat_id {
            stop_listener(state, "telegram");
        }

        let sl_router_changed = !crate::app::is_slack_active(new)
            || old.channels.slack.bot_token != new.channels.slack.bot_token
            || old.channels.slack.app_token != new.channels.slack.app_token;
        if sl_router_changed {
            *state.sl_router.lock().await = None;
        }
        if sl_router_changed || old.channels.slack.user_id != new.channels.slack.user_id {
            stop_listener(state, "slack");
        }
    }

    /// 停掉某渠道的入站监听（改配置时）：take 出其 stop 并 notify——旧任务随即退出、认领立即释放，
    /// 供 `ensure_inbound_listeners` 按新配置重建。无监听时为 no-op。
    fn stop_listener(state: &Arc<ServerState>, id: &str) {
        if let Some(stop) = state.inbound_listeners.take(id) {
            stop.notify_one();
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

    fn stop_cmd(force: bool) -> i32 {
        block_on(async {
            if !client::request_stop(force).await {
                println!("askhuman daemon: not running");
                return 0;
            }
            println!("askhuman daemon: stopping");
            wait_stopped(force).await;
            println!("askhuman daemon: stopped");
            0
        })
    }

    fn restart_cmd(force: bool) -> i32 {
        block_on(async {
            if client::request_stop(force).await {
                wait_stopped(force).await;
            }
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

    /// 等 Daemon 下线。force：限时即可；graceful：可能在排空（等在途请求完结），
    /// 无限等待并周期性打印进度与强制提示。
    async fn wait_stopped(force: bool) {
        if force {
            client::wait_until_down(Duration::from_secs(5)).await;
            return;
        }
        let mut last_hint: Option<Instant> = None;
        loop {
            let Some(info) = client::request_status().await else {
                return; // 已下线。
            };
            if info.draining && last_hint.map_or(true, |t| t.elapsed() >= Duration::from_secs(30)) {
                eprintln!(
                    "askhuman daemon: draining ({} active request(s) left); waiting… (use --force to terminate now)",
                    info.active_requests
                );
                last_hint = Some(Instant::now());
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
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
        println!(
            "  requests   {} active{}",
            info.active_requests,
            if info.draining { " (draining)" } else { "" }
        );
        let im = if info.im_connections.is_empty() {
            "none".to_string()
        } else {
            info.im_connections.join(", ")
        };
        println!("  im conns   {}", im);
    }

    #[cfg(test)]
    mod tests {
        use super::InboundRegistry;
        use std::sync::Arc;

        #[test]
        fn inbound_claim_is_exclusive() {
            let reg = InboundRegistry::default();
            let a = reg.claim("feishu").expect("first claim succeeds");
            assert!(
                reg.claim("feishu").is_none(),
                "a second claim is blocked while the first is held"
            );
            reg.release("feishu", &a);
            assert!(
                reg.claim("feishu").is_some(),
                "the channel can be re-claimed once the owner releases it"
            );
        }

        #[test]
        fn inbound_release_is_identity_safe() {
            // 配置变更场景：take 出旧 stop 释放认领 → 新监听重新认领 → 旧任务**之后**才退出并尝试
            // 释放。释放必须按身份判定，绝不能把「新监听」的认领误删（否则会重复 spawn）。
            let reg = InboundRegistry::default();
            let old = reg.claim("feishu").expect("old listener claims");
            let taken = reg.take("feishu").expect("config change takes current stop");
            assert!(Arc::ptr_eq(&old, &taken));
            let new = reg.claim("feishu").expect("new listener re-claims after take");
            reg.release("feishu", &old); // 旧任务迟到的释放
            assert!(
                reg.claim("feishu").is_none(),
                "the new listener's claim must survive a stale release from the old task"
            );
            reg.release("feishu", &new);
            assert!(reg.claim("feishu").is_some());
        }
    }
}
