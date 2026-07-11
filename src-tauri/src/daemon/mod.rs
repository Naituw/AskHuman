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
    use super::request::{self, InteractionEntry, RequestEntry, RequestRegistry};
    use crate::agents::registry::AgentRegistry;
    use crate::agents::{AgentKind, LifecycleEvent};
    use crate::app::confirm_coordinator::ConfirmOutcome;
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
        self, transport, ClientMsg, ConfirmTask, DetectRequest, HelloAck, HelloStatus, ServerMsg,
        StatusInfo, TaskRequest,
    };
    use crate::models::{ChannelAction, ChannelResult, ConfirmFallbackReason};
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

    /// 「工作中」兜底超时秒数。默认 `WORKING_BACKSTOP_SECS`（30min），
    /// 可用 `ASKHUMAN_WORKING_BACKSTOP_SECS` 覆盖（便于实测自愈，无需真等 30 分钟）。
    fn working_backstop_secs() -> u64 {
        std::env::var("ASKHUMAN_WORKING_BACKSTOP_SECS")
            .ok()
            .and_then(|s| s.parse::<u64>().ok())
            .filter(|&s| s > 0)
            .unwrap_or(crate::agents::registry::WORKING_BACKSTOP_SECS)
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

    fn now_ms() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
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
        /// Agent 插话队列（spec agent-interject）：内存常驻，变更时落盘 `interject.json`。
        interject: crate::agents::interject::InterjectStore,
        /// 状态窗口订阅者的发送端列表（变化 / 心跳时推 `AgentsState`）。
        agent_subs: Mutex<Vec<tokio::sync::mpsc::UnboundedSender<ServerMsg>>>,
        /// 菜单栏宿主订阅者的发送端列表（变化 / 心跳时推 `TrayState`）。
        /// **非保活**：该列表不参与空闲退出判定（见 `handle_tray_sub`），图标不得续命 daemon。
        tray_subs: Mutex<Vec<tokio::sync::mpsc::UnboundedSender<ServerMsg>>>,
        /// 「IM 会话期自动激活」当前活跃槽（持久化、跨重启保留，仅由入站消息改变）。
        active_channel: Mutex<Option<String>>,
        /// 已启动入站监听器的注册表（防重复 spawn + 改配置时主动停旧监听并重建）。
        inbound_listeners: InboundRegistry,
        /// 方案6 弹窗预热「热池」：最多 1 个已挂载、隐藏待命的热实例连接。来请求时 `dispatch_popup` 取出
        /// 并把请求 entry 交给其 holder 任务领用上屏，省掉冷 spawn + WebView 初始化。**非保活**（不计入
        /// `active`），daemon 仍可正常空闲退出（在途请求由 CLI submit 连接保活）。
        warm_pool: Mutex<Option<WarmSlot>>,
        /// 正在补热中（去重，避免并发 spawn 多个热实例）。
        warm_spawning: AtomicBool,
        /// `/watch` 实时关注子系统（spec docs/specs/im-watch.md；飞书/Telegram/Slack）。
        watch: WatchState,
        /// 通用「单选卡」子系统（spec docs/specs/im-select-card.md；MVP 仅飞书）。
        select: SelectState,
    }

    /// 「跟底」重发节流：同一订阅两次跟底之间的最短间隔（用户定案 30s）。
    const WATCH_MOVE_THROTTLE_MS: u64 = 30_000;

    /// rewatchable entry 保留时限（秒）：超时后自动清理（路由失效、按钮不再可用）。
    const REWATCHABLE_TTL_SECS: u64 = 600;

    /// `/watch` 实时关注子系统的 daemon 侧状态。
    #[derive(Default)]
    struct WatchState {
        /// 活动订阅（agent 结束 / 用户取消即移除；每渠道上限 `watch::MAX_WATCHES`）。
        subs: Mutex<Vec<WatchEntry>>,
        /// 引擎唤醒信号（AgentEvent / 提问创建、完结 / 订阅变化）。
        notify: tokio::sync::Notify,
        /// 渠道 id → 卡片按钮回调路由任务句柄（随 Router 生命周期 / 订阅集合变化整体重建）。
        routes: Mutex<HashMap<String, WatchRouteHandle>>,
        /// 渠道 id → 「最后一条非 watch 消息」时刻（Unix 毫秒）——跟底判定的**淹没信号**。
        /// 只有非 watch 消息才算淹没：watch 卡之间互不影响（用户定案）。
        disturb: Mutex<HashMap<String, u64>>,
    }

    /// 一条活动的 watch 订阅（引擎工作台账；持久化时压成 `watch::PersistedWatch`）。
    #[derive(Clone)]
    struct WatchEntry {
        /// 卡片所在渠道 id（feishu / telegram / slack）。
        channel: String,
        /// 被关注 agent 的 session_id（身份键）。
        session_id: String,
        /// 实时状态卡的消息 id（编辑目标；跟底重发后换新）。渠道各异：飞书 open_message_id、
        /// Telegram message_id 十进制串、Slack 消息 ts、钉钉 outTrackId（自铸 uuid）。
        message_id: String,
        /// 展示编号（同 `/status`；daemon 生命周期内稳定，重启恢复时按 session 重解析）。
        seq: u64,
        created_at: u64,
        /// 上一帧签名：内容不变不编辑（防无谓编辑请求）。
        last_sig: String,
        /// 上次成功编辑时刻（Unix 毫秒）：每卡最短编辑间隔按渠道（`WatchClient::min_edit_interval_ms`）。
        last_edit_ms: u64,
        /// 连续编辑失败次数（≥5 自动退订：超时不可改 / 卡被删等）。
        fails: u32,
        /// 上一帧是否「工作中」（引擎自适应 tick：有工作中 2s，否则 10s）。
        working: bool,
        /// 当前卡发出时刻（Unix 毫秒）：与渠道 disturb 水位比较判定卡是否已被淹没。
        sent_at_ms: u64,
        /// 上次跟底重发时刻（Unix 毫秒）：30s 节流；提问答复完结时清零（下次更新立即跟底）。
        last_move_ms: u64,
        /// 终态已定格但保留路由供重新关注（仅 AutoStopped；引擎/上限/空闲退出跳过此类 entry）。
        rewatchable: bool,
    }

    /// watch 卡片回调路由任务的句柄：绑定特定 Router 实例与注册的卡片集合，
    /// 任一变化（Router 重建 / 订阅增减）都停旧任务整体重建。
    struct WatchRouteHandle {
        stop: Arc<tokio::sync::Notify>,
        router: WatchRouterRef,
        /// 任务注册路由时的卡片 message_id 集合（已排序，用于变更比较）。
        mids: Vec<String>,
    }

    /// 路由任务绑定的 Router 弱引用（按渠道类型分列，用于「同一存活 Router」比对）。
    enum WatchRouterRef {
        Feishu(std::sync::Weak<FsRouter>),
        Telegram(std::sync::Weak<TgRouter>),
        Slack(std::sync::Weak<SlRouter>),
        DingTalk(std::sync::Weak<DdRouter>),
    }

    impl WatchRouterRef {
        /// 是否仍绑定同一个存活的 Router 实例。
        fn is_same_alive(&self, ch: &WatchChannelRouter) -> bool {
            match (self, ch) {
                (WatchRouterRef::Feishu(w), WatchChannelRouter::Feishu(r)) => w
                    .upgrade()
                    .map(|x| Arc::ptr_eq(&x, r) && x.is_alive())
                    .unwrap_or(false),
                (WatchRouterRef::Telegram(w), WatchChannelRouter::Telegram(r)) => w
                    .upgrade()
                    .map(|x| Arc::ptr_eq(&x, r) && x.is_alive())
                    .unwrap_or(false),
                (WatchRouterRef::Slack(w), WatchChannelRouter::Slack(r)) => w
                    .upgrade()
                    .map(|x| Arc::ptr_eq(&x, r) && x.is_alive())
                    .unwrap_or(false),
                (WatchRouterRef::DingTalk(w), WatchChannelRouter::DingTalk(r)) => w
                    .upgrade()
                    .map(|x| Arc::ptr_eq(&x, r) && x.is_alive())
                    .unwrap_or(false),
                _ => false,
            }
        }
    }

    /// 某渠道当前的共享 Router（强引用，路由重建时用）。
    enum WatchChannelRouter {
        Feishu(Arc<FsRouter>),
        Telegram(Arc<TgRouter>),
        Slack(Arc<SlRouter>),
        DingTalk(Arc<DdRouter>),
    }

    /// 通用「单选卡」子系统的 daemon 侧状态（spec docs/specs/im-select-card.md）。
    /// picker 是一次性选择器、**不持久化**（daemon 重启后旧卡点击静默无效，D7）。
    /// 同台账挂 `/stage` Confirm 卡（spec im-diff-stage-transcript；不持久化，TTL 同 picker）。
    struct SelectState {
        /// 活动的单选卡台账（被消费即移除；软上限 + TTL 兜底清理，见 `register_picker`）。
        pickers: Mutex<Vec<PickerEntry>>,
        /// `/stage` 确认卡台账（与 pickers 共享路由重建，message_id 并入 ensure_select_routes）。
        confirms: Mutex<Vec<ConfirmEntry>>,
        /// 渠道 id → 卡片按钮回调路由任务句柄（复用 watch 的 `WatchRouteHandle`）。
        routes: Mutex<HashMap<String, WatchRouteHandle>>,
        /// 异步请求重建 select/confirm 路由（打破 ensure_select_route_for ↔ card handler 的 async 环）。
        route_refresh: tokio::sync::Notify,
    }

    impl Default for SelectState {
        fn default() -> Self {
            Self {
                pickers: Mutex::new(Vec::new()),
                confirms: Mutex::new(Vec::new()),
                routes: Mutex::new(HashMap::new()),
                route_refresh: tokio::sync::Notify::new(),
            }
        }
    }

    /// 单选卡种类（决定点选后做什么；接新命令只需加一档 + 其选中动作）。
    #[derive(Clone, Copy, PartialEq, Eq, Debug)]
    enum PickerKind {
        Watch,
        Status,
        Unwatch,
        /// 发送插话（`/msg` 无编号）：点选把 `PickerEntry::payload` 发给该 agent。
        Msg,
        Diff,
        Stage,
        Transcript,
    }

    /// `/stage` 确认卡台账（不持久化）。业务状态留在此处；通用 view 只负责展示与动作槽映射。
    #[derive(Clone)]
    struct ConfirmEntry {
        channel: String,
        message_id: String,
        session_id: String,
        git_root: std::path::PathBuf,
        paths_fp: String,
        /// 通用展示模型；wire slot 必须经它映射回 `/stage` 的稳定业务 action id。
        view: crate::confirm::ConfirmView,
        created_at: u64,
    }

    /// 一条活动的单选卡台账。选项快照仅存各选项的 session_id（下标即按钮 idx），点击时按下标取 id、
    /// 再由当前快照/订阅重新定位（避免 seq 漂移）。
    #[derive(Clone)]
    struct PickerEntry {
        channel: String,
        message_id: String,
        kind: PickerKind,
        /// 各选项的 session_id（下标 = 按钮 `select:<idx>`）。
        options: Vec<String>,
        /// `Msg` 卡的待发送内容（点「发送」时投递）；其它 kind 恒 `None`。
        payload: Option<String>,
        created_at: u64,
        /// 发卡时刻的渠道扰动水位（Unix 毫秒，同 `WatchState::disturb` 量纲）：与当前渠道水位比较判定
        /// 本单选卡是否仍位于会话底部（其后未再出现非 watch 消息）。用于「仅当单选卡还是最后一条
        /// 消息时才抑制 watch 跟底」（见 `select_is_last_on`）。
        posted_ms: u64,
    }

    /// 单选卡台账治理上限：每渠道最多留存的活动单选卡数（超出丢最旧）。
    const SELECT_MAX_PICKERS_PER_CHANNEL: usize = 10;
    /// 单选卡台账 TTL（秒）：超龄未消费即清理（兜底，避免长期累积）。
    const SELECT_PICKER_TTL_SECS: u64 = 1800;

    /// 热池中一个待命热实例的句柄：`assign` 用于把领用的请求 entry 交给其 holder 任务（`handle_gui_warm`）；
    /// `gui_tx` 仅用于身份比对（热进程自然死亡时判定池中是否仍是本槽）。
    struct WarmSlot {
        assign: tokio::sync::oneshot::Sender<InteractionEntry>,
        gui_tx: tokio::sync::mpsc::UnboundedSender<ServerMsg>,
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

        // 自动迁移：已开启生命周期追踪的家族若 hook 过期（升级新增了事件 / 命令路径变化），
        // 启动时幂等重装一次，让已安装用户无需手动关开开关即可拿到新 hook（仅刷新已安装的家族）。
        {
            let migrated = crate::integrations::agent_lifecycle::migrate_outdated();
            if !migrated.is_empty() {
                let names: Vec<&str> = migrated.iter().map(|k| k.as_str()).collect();
                log(&format!(
                    "migrated outdated lifecycle hooks: {}",
                    names.join(", ")
                ));
            }
        }

        // 保活模式：让 daemon 登录项（下次登录自启）与配置一致（幂等，纯文件；exe 路径变化会刷新）。
        sync_daemon_login_item();

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
            interject: crate::agents::interject::InterjectStore::load(),
            agent_subs: Mutex::new(Vec::new()),
            tray_subs: Mutex::new(Vec::new()),
            active_channel: Mutex::new(crate::autochannel::load_active()),
            inbound_listeners: InboundRegistry::default(),
            warm_pool: Mutex::new(None),
            warm_spawning: AtomicBool::new(false),
            watch: WatchState::default(),
            select: SelectState::default(),
        });

        // 空闲退出检查。
        {
            let state = state.clone();
            tokio::spawn(async move {
                let timeout = idle_timeout();
                loop {
                    tokio::time::sleep(Duration::from_secs(30)).await;
                    // 保活模式（general.daemonLifecycle = keepalive）：永不因空闲退出（让 IM 随时可收）。
                    // 每轮读一次配置（load_without_secrets：仅读 config.json，不碰钥匙串，开销可忽略）。
                    if crate::config::AppConfig::load_without_secrets()
                        .general
                        .daemon_lifecycle
                        == crate::config::DaemonLifecycleMode::KeepAlive
                    {
                        continue;
                    }
                    // 空闲退出守卫（spec D18）：仅当无在途请求、无状态窗口订阅、**且**无「工作中」
                    // agent 时才计空闲。空闲 agent 不保活；版本更新 drain 由 begin_drain 独立处理、不受此影响。
                    // 另：有活跃 /watch 订阅时不退（卡片要持续就地刷新；订阅随 agent 结束而消亡，有界）。
                    if state.active.load(Ordering::SeqCst) == 0
                        && state.agents.working_count() == 0
                        && !has_agent_subs(&state)
                        && !state
                            .watch
                            .subs
                            .lock()
                            .unwrap()
                            .iter()
                            .any(|s| !s.rewatchable)
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
                    // 在途 AskHuman 豁免：先给「正等待人类回答」的 agent 刷新活动，使其不被兜底降级。
                    // pid 版覆盖有 pid 的 agent；session_id 版覆盖无 pid 的（Codex 共享 app-server /
                    // Claude 被 scrub），两者并列（spec D25/D26 + Q1 甲）。
                    state
                        .agents
                        .refresh_by_pids(&state.registry.in_flight_agent_pids());
                    state
                        .agents
                        .refresh_by_session_ids(&state.registry.in_flight_agent_session_ids());
                    // 进程存活轮询 + 无 pid TTL 兜底 + 「工作中」兜底超时（打断这类无 hook 场景）。
                    let changed = state.agents.poll_liveness()
                        | state.agents.ttl_sweep()
                        | state.agents.working_backstop_sweep(working_backstop_secs());
                    if changed {
                        state.agents.persist();
                    }
                    // 插话队列兜底清理（spec agent-interject D2/D8）：不再活动的会话（漏 session-end /
                    // 进程死亡 / pid 轮换 / daemon 停机期间结束）清空其待送达条目、放行等待者。
                    let ij_changed = state
                        .interject
                        .retain_sessions(&state.agents.active_session_ids());
                    if ij_changed {
                        state.interject.persist();
                    }
                    if changed || ij_changed || has_agent_subs(&state) {
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

        // 方案6：daemon ready 后预热一个弹窗实例（self-gated：关 / 无显示则不补）。新 daemon / 二进制换新
        // 后的首个弹窗仍冷，但其余请求都能命中热实例。
        maybe_topup_warm(&state);

        // 「存活即监听」：daemon 一就绪即在后台连启用的 IM 起入站监听（与工作中 agent / 在途提问 /
        // 自动激活开关无关），使在世期间任何 IM 消息都能被收到并回复。后台执行，不挡受理 / 弹窗关键路径；
        // 无启用 IM 时 `ensure_inbound_listeners` 自身零钥匙串直接返回。监听不计入保活，不阻止空闲退出。
        {
            let st = state.clone();
            tokio::spawn(async move {
                ensure_inbound_listeners(&st).await;
            });
        }

        // `/watch` 实时关注引擎（spec docs/specs/im-watch.md）：先恢复持久化订阅（重启后继续
        // 编辑同一张卡），再进入 Notify / 自适应 tick 循环。
        {
            let st = state.clone();
            tokio::spawn(async move {
                watch_restore_and_run(st).await;
            });
        }

        // select/confirm 路由重建旁路任务：避免 card handler → ensure_select_routes 的 async 类型环。
        {
            let st = state.clone();
            tokio::spawn(async move {
                loop {
                    st.select.route_refresh.notified().await;
                    ensure_select_routes(&st).await;
                }
            });
        }

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

        // 方案6：关停前回收热实例（drop 连接 → 热进程收 EOF 自杀，不悬挂）。先置 `draining` 再回收，
        // 否则被回收的热实例走死亡分支会 `maybe_topup_warm` 又拉起一个新热进程（孤儿，daemon 已停）。
        // `begin_drain` 走的是同一道门（先 `draining=true` 再 `recycle_warm`）；plain stop/restart 此前漏了。
        state.draining.store(true, Ordering::SeqCst);
        recycle_warm(&state);

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
        SubmitConfirm(ConfirmTask),
        Gui(String),
        /// 方案6 预热弹窗握手：接管连接，入热池待命、等领用。
        GuiWarm,
        /// 状态窗口订阅：接管连接，持续推送 agent 快照。
        AgentsSub,
        /// 菜单栏宿主订阅：接管连接，持续推送 `TrayState`（非保活）。
        TraySub,
        /// 插话 composer 窗口连接：接管连接，登记「composer 打开」；断开＝关闭（非保活）。
        InterjectComposer {
            session_id: String,
        },
        /// 插话 Hold：hook 连接等待 composer 提交/取消（非保活；首帧 Hold 已回，等二帧）。
        InterjectHold {
            session_id: String,
            rx: tokio::sync::oneshot::Receiver<crate::agents::interject::WaitOutcome>,
        },
        Closed,
    }

    async fn handle_conn(stream: UnixStream, state: Arc<ServerState>) {
        state.active.fetch_add(1, Ordering::SeqCst);
        let (r, w) = stream.into_split();
        let mut reader = BufReader::new(r);
        let mut w = w;

        match control_loop(&mut reader, &mut w, &state).await {
            Control::Submit(task) => handle_submit(task, reader, w, &state).await,
            Control::SubmitConfirm(task) => handle_submit_confirm(task, reader, w, &state).await,
            Control::Gui(token) => handle_gui(token, reader, w, &state).await,
            Control::GuiWarm => handle_gui_warm(reader, w, &state).await,
            Control::AgentsSub => handle_agents_sub(reader, w, &state).await,
            Control::TraySub => handle_tray_sub(reader, w, &state).await,
            Control::InterjectComposer { session_id } => {
                handle_interject_composer(reader, w, &state, session_id).await
            }
            Control::InterjectHold { session_id, rx } => {
                handle_interject_hold(reader, w, &state, session_id, rx).await
            }
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
                ClientMsg::SubmitConfirm(task) => return Control::SubmitConfirm(task),
                ClientMsg::GuiHello { token } => return Control::Gui(token),
                // 方案6 预热弹窗握手（无 token）：接管连接入热池待命。
                ClientMsg::GuiWarmReady => return Control::GuiWarm,
                // Agent 生命周期事件上报（默认即发即走；`interject_poll=true` 时回一帧插话裁决）：
                // 更新注册表，变化则持久化 + 推订阅窗口。
                ClientMsg::AgentEvent {
                    agent,
                    event,
                    session_id,
                    pid,
                    hint_pid,
                    cwd,
                    ts,
                    tool,
                    interject_poll,
                } => {
                    if let (Some(kind), Some(ev)) =
                        (AgentKind::parse(&agent), LifecycleEvent::parse(&event))
                    {
                        // PreToolUse：先回 interject 再做后续（hook 在等，优先解除阻塞）。
                        if interject_poll {
                            use crate::agents::interject::PollOutcome;
                            use crate::ipc::InterjectAction;
                            match state.interject.poll(&session_id) {
                                PollOutcome::None => {
                                    let _ = ipc::write_msg(
                                        w,
                                        &ServerMsg::InterjectDecision {
                                            action: InterjectAction::None,
                                            text: String::new(),
                                        },
                                    )
                                    .await;
                                }
                                PollOutcome::Message {
                                    text,
                                    receipt_channels,
                                } => {
                                    let _ = ipc::write_msg(
                                        w,
                                        &ServerMsg::InterjectDecision {
                                            action: InterjectAction::Message,
                                            text,
                                        },
                                    )
                                    .await;
                                    state.interject.persist();
                                    broadcast_agents_state(state);
                                    spawn_read_receipts(state, &session_id, receipt_channels);
                                }
                                PollOutcome::Hold(rx) => {
                                    let _ = ipc::write_msg(
                                        w,
                                        &ServerMsg::InterjectDecision {
                                            action: InterjectAction::Hold,
                                            text: String::new(),
                                        },
                                    )
                                    .await;
                                    return Control::InterjectHold { session_id, rx };
                                }
                            }
                        }

                        // PID 解析：优先用显式 pid，否则从 hint_pid 走缓存/walk。
                        let resolved_pid =
                            pid.or_else(|| state.agents.resolve_pid(&session_id, kind, hint_pid));

                        let changed =
                            state
                                .agents
                                .apply_event(kind, ev, &session_id, resolved_pid, cwd, ts);
                        if changed {
                            state.agents.persist();
                            broadcast_agents_state(state);
                        }
                        if matches!(ev, LifecycleEvent::SessionEnd) {
                            state.agents.clear_pid_cache(&session_id);
                            if state.interject.remove_session(&session_id) {
                                state.interject.persist();
                            }
                        }
                        match tool {
                            Some(crate::ipc::ToolReport {
                                phase: crate::ipc::ToolPhase::Pre,
                                name,
                                object,
                            }) => state.agents.set_current_tool(
                                kind,
                                &session_id,
                                resolved_pid,
                                name,
                                object,
                            ),
                            Some(crate::ipc::ToolReport {
                                phase: crate::ipc::ToolPhase::Post,
                                ..
                            }) => state.agents.clear_current_tool(kind, &session_id),
                            None => {}
                        }
                        ensure_inbound_listeners(state).await;
                        state.watch.notify.notify_one();
                    } else if interject_poll {
                        let _ = ipc::write_msg(
                            w,
                            &ServerMsg::InterjectDecision {
                                action: crate::ipc::InterjectAction::None,
                                text: String::new(),
                            },
                        )
                        .await;
                    }
                }
                // 状态窗口订阅：接管连接持续推送。
                ClientMsg::AgentsSubscribe => return Control::AgentsSub,
                // 菜单栏宿主订阅：接管连接持续推送 TrayState（非保活）。
                ClientMsg::TraySubscribe => return Control::TraySub,
                // 托盘「待答」子菜单点击：聚焦/闪烁对应请求的弹窗（即发即走，无回包）。
                ClientMsg::FocusRequest { request_id } => {
                    state.registry.focus_popup(&request_id);
                }
                // 状态窗口手动把某 agent 置空闲（纠正漏 hook 卡「工作中」）：变化则持久化 + 推订阅窗口。
                ClientMsg::AgentForceIdle { session_id } => {
                    if state.agents.force_idle(&session_id) {
                        state.agents.persist();
                        broadcast_agents_state(state);
                    }
                }
                // 插话 composer 窗口登记：接管连接（连接断开＝关闭，非保活）。
                ClientMsg::InterjectComposer { session_id } => {
                    return Control::InterjectComposer { session_id };
                }
                // 插话提交（独立连接形态；composer 连接上的提交在 handle_interject_composer 处理）。
                ClientMsg::InterjectSubmit { session_id, text } => {
                    interject_submit(state, &session_id, &text);
                }
                // 插话追加（不覆盖既有待送达；快捷固定插话用）。
                ClientMsg::InterjectAppend { session_id, text } => {
                    interject_append(state, &session_id, &text);
                }
                // 撤回待送达。
                ClientMsg::InterjectClear { session_id } => {
                    if state.interject.clear(&session_id) {
                        state.interject.persist();
                        broadcast_agents_state(state);
                    }
                }
                // 查询待送达全文（composer 预填 / IM 回显）。
                ClientMsg::InterjectQuery { session_id } => {
                    let _ = ipc::write_msg(
                        w,
                        &ServerMsg::InterjectState {
                            text: state.interject.full_text(&session_id),
                            entries: state.interject.pending_count(&session_id),
                        },
                    )
                    .await;
                }
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
                ClientMsg::Answer { .. }
                | ClientMsg::ConfirmAnswer { .. }
                | ClientMsg::ConfirmReady { .. } => {}
            }
        }
    }

    /// 进入排空状态（幂等）：首次置位时 spawn 看门狗，在途请求全部完结后触发退出。
    fn begin_drain(state: &Arc<ServerState>) {
        if state.draining.swap(true, Ordering::SeqCst) {
            return; // 已在排空。
        }
        // 方案6：进入排空即回收热实例（它是旧二进制；新 daemon 起来后再补热）。`draining` 已置位，
        // 故后续不会再补热；排空期到来的请求走冷路径（现有 drain 语义）。
        recycle_warm(state);
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

    /// Accept a structured confirmation task and return only a validated terminal decision.
    /// Surface attachment is layered on top of this lifecycle; until one becomes ready, callers
    /// receive an explicit fallback and continue with the agent's native approval UI.
    async fn handle_submit_confirm(
        task: ConfirmTask,
        mut reader: Reader,
        mut w: OwnedWriteHalf,
        state: &Arc<ServerState>,
    ) {
        if state.draining.load(Ordering::SeqCst) {
            let _ = ipc::write_msg(
                &mut w,
                &ServerMsg::ConfirmFallback {
                    reason: ConfirmFallbackReason::Draining,
                },
            )
            .await;
            return;
        }

        let (entry, mut final_rx) = match state.registry.create_confirm(task) {
            Ok(created) => created,
            Err(error) => {
                log(&format!("invalid confirmation request: {error}"));
                let _ = ipc::write_msg(
                    &mut w,
                    &ServerMsg::ConfirmFallback {
                        reason: ConfirmFallbackReason::InvalidRequest,
                    },
                )
                .await;
                return;
            }
        };
        let request_id = entry.request_id.clone();
        if ipc::write_msg(
            &mut w,
            &ServerMsg::ConfirmAccepted {
                request_id: request_id.clone(),
            },
        )
        .await
        .is_err()
        {
            entry.coordinator.cancel();
            entry.cancel.notify_waiters();
            state.registry.remove_confirm(&request_id);
            return;
        }
        broadcast_tray_state(state);

        let shallow_config = AppConfig::load_without_secrets();
        let config = if any_im_enabled(&shallow_config) {
            AppConfig::load()
        } else {
            shallow_config
        };
        let popup_enabled = config.channels.popup.enabled && has_display();
        let im_candidates = confirm_im_candidates(&entry, state, &config);
        if popup_enabled {
            entry.start_delivery("popup");
        }
        for channel in &im_candidates {
            entry.start_delivery(*channel);
        }
        if !popup_enabled && im_candidates.is_empty() {
            entry
                .coordinator
                .fallback(ConfirmFallbackReason::NoAvailableChannel);
        }

        if popup_enabled {
            if dispatch_interaction_popup(
                InteractionEntry::Confirm(entry.clone()),
                state,
                "",
                false,
            ) {
                spawn_confirm_popup_watchdog(entry.clone());
            } else if entry.mark_failed("popup", "failed to spawn popup helper") {
                entry
                    .coordinator
                    .fallback(ConfirmFallbackReason::NoAvailableChannel);
            }
        }
        attach_confirm_im_channels(&entry, state, &config, &im_candidates).await;
        for channel in &im_candidates {
            mark_watch_disturbed(state, channel);
        }
        ensure_inbound_listeners(state).await;

        let outcome = tokio::select! {
            outcome = final_rx.recv() => outcome,
            _ = tokio::time::sleep_until(entry.deadline) => {
                entry.coordinator.fallback(ConfirmFallbackReason::Expired);
                final_rx.recv().await
            }
            _ = wait_cli_eof(&mut reader) => {
                entry.coordinator.cancel();
                entry.cancel.notify_waiters();
                state.registry.remove_confirm(&request_id);
                broadcast_tray_state(state);
                return;
            }
        };

        match outcome {
            Some(ConfirmOutcome::Final(result)) => {
                let _ = ipc::write_msg(&mut w, &ServerMsg::ConfirmFinal { result }).await;
            }
            Some(ConfirmOutcome::Fallback(reason)) => {
                let _ = ipc::write_msg(&mut w, &ServerMsg::ConfirmFallback { reason }).await;
            }
            None => {
                let _ = ipc::write_msg(
                    &mut w,
                    &ServerMsg::ConfirmFallback {
                        reason: ConfirmFallbackReason::InternalError,
                    },
                )
                .await;
            }
        }
        if config.channels.auto_activation {
            if let Some(winner) = entry.coordinator.winner_channel_id() {
                set_active_channel(state, &winner).await;
            }
        }
        entry.mark_deliveries_terminal();
        entry.cancel.notify_waiters();
        state.registry.remove_confirm(&request_id);
        broadcast_tray_state(state);
        for channel in ["feishu", "telegram", "slack", "dingding"] {
            if entry.has_delivery(channel) {
                for subscription in state
                    .watch
                    .subs
                    .lock()
                    .unwrap()
                    .iter_mut()
                    .filter(|subscription| subscription.channel == channel)
                {
                    subscription.last_move_ms = 0;
                }
            }
        }
        state.watch.notify.notify_one();
        log(&format!("confirmation request {request_id} done"));
    }

    /// CLI 提交一次任务：建请求、spawn GUI Helper、流式回结果；CLI 断开则取消。
    async fn handle_submit(
        task: TaskRequest,
        mut reader: Reader,
        mut w: OwnedWriteHalf,
        state: &Arc<ServerState>,
    ) {
        // 性能埋点关联 id（CLI 透传；空=关闭）。daemon 自身启动时无 ASKHUMAN_PERF env，
        // 故各阶段标记一律以本字段非空为开关（见 perf 模块说明）。
        let perf_id = task.perf_id.clone();
        let perf_autodismiss = task.perf_autodismiss;
        crate::perf::mark(&perf_id, "dmn.submit_recv");
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
        // 方案5(b)：家族 + 会话来自 CLI 的 env 探测（即时、零 ps）；进程 pid 改由 daemon 从 `caller_pid`
        // 异步 walk（见下 spawn_agent_resolve）。这里先用「能即时拿到的」刷新一次（env 有家族+会话 →
        // 按 session 刷新，pid 此刻通常为 None；旧 CLI 仍可能带 pid），async 完成后再补 pid / MCP 兜底。
        let kind_env = task.agent_kind.as_deref().and_then(AgentKind::parse);
        let sid_env = task.agent_session_id.clone();
        let from_mcp = task.from_mcp;
        let caller_pid = task.caller_pid;
        let cwd = Some(task.project.clone()).filter(|s| !s.trim().is_empty());
        let changed = match (kind_env, sid_env.clone(), task.agent_pid) {
            // 有 session_id（shell 工具子进程能从 env 拿到）：按 session 刷新。
            (Some(kind), Some(sid), pid) => {
                // MCP 模式（`from_mcp`）下 `agent_session_id` 取自长驻 MCP server 的启动 env，可能过期；
                // 故即便「自动激活」开启也**只刷新已存在的 session、绝不新建**，避免造出幽灵会话。
                if auto && !from_mcp {
                    state.agents.upsert_working(kind, &sid, pid, cwd.clone())
                } else {
                    state.agents.touch_activity(kind, &sid, pid)
                }
            }
            // 无 session_id 但有 pid（旧 CLI 的 MCP 路径仍可能带 pid）：按 pid 刷新（只更新、绝不新建）。
            (Some(kind), None, Some(pid)) => state.agents.touch_activity_by_pid(kind, pid),
            _ => false,
        };
        if changed {
            state.agents.persist();
            broadcast_agents_state(state);
        }
        let lang = Lang::resolve(&task.lang);
        let (entry, mut final_rx) = state.registry.create(task);
        let request_id = entry.request_id.clone();
        crate::perf::mark(&perf_id, "dmn.created");
        log(&format!("request {} accepted", request_id));

        // Fire `ask-received` once per accepted request, independent of popup state.
        crate::hooks::fire_ask_received(
            &request_id,
            &entry.show.source,
            &entry.show.project,
            entry.request(),
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
        crate::perf::mark(&perf_id, "dmn.accepted");
        // 在途请求数 +1：刷新菜单栏状态（待答数 / 图标圆点）。
        broadcast_tray_state(state);
        // /watch：提问创建 → 被关注 agent 可能进入「正在等待你的回答」，即时进卡。
        state.watch.notify.notify_one();

        // 方案3（spec §6.1）：尽早 spawn GUI Helper（独立短命进程，带一次性 token），让其 WebView
        // 初始化与下面的「入站监听 + IM 建连」并行——token 在 `registry.create()` 即登记，helper 可
        // 立即连上，不存在「helper 先连、entry 未注册」竞态。冷启动下这把 IM 建连（数百 ms）整段移出
        // 弹窗端到端关键路径。
        // 方案6：优先领用热池中的预热弹窗（秒级上屏）；池空 / 关 / 无显示 / holder 死 → 回退冷 spawn。
        let popup_ok = dispatch_popup(&entry, state, &perf_id, perf_autodismiss);
        if !popup_ok {
            let _ = ipc::write_msg(
                &mut w,
                &ServerMsg::Warn {
                    text: format!("{}failed to spawn popup", crate::i18n::err_prefix(lang)),
                },
            )
            .await;
        }

        // 方案5(b)：从 caller_pid 异步向上 walk 进程树解析 agent（家族 + pid，含 env 判不出时的 MCP 兜底），
        // 完成后补刷注册表活动并把结果后推弹窗 badge。整段在独立任务里跑，绝不阻塞本请求的关键路径。
        spawn_agent_resolve(
            entry.clone(),
            state.clone(),
            caller_pid,
            kind_env,
            sid_env,
            from_mcp,
            auto,
            cwd,
        );

        // 以下都已不在弹窗关键路径上（与上面已 spawn 的 helper 并行执行）：
        // 确保入站消费在线（自身按「有工作中 agent」自门控；与开关无关，使 /status 等命令独立可用）。
        ensure_inbound_listeners(state).await;
        // 挂接可用的 IM 渠道（钉钉/…）到本请求的协调器，与弹窗并行抢答。
        let im_attached = attach_im_channels(&entry, state, &mut w, lang).await;
        crate::perf::mark(&perf_id, "dmn.im_done");
        // /watch 跟底：提问卡即将出现在渠道会话里，是一次「非 watch」扰动（提问期间跟底被抑制，
        // 这里先记水位线，供答复完结后立即跟底）。
        for ch in ["feishu", "telegram", "slack"] {
            if entry.coordinator.has_channel(ch) {
                mark_watch_disturbed(state, ch);
            }
        }
        // IM 长连接可能在此刚建立，刷新菜单栏「已连 IM」。
        broadcast_tray_state(state);

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
                // 不标记扰动：取消只是就地 PATCH 提问卡定格，不产生新消息（不淹没 watch 卡）。
                state.watch.notify.notify_one();
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
        // /watch：答复完结 → 「正在等待你的回答」状态解除，即时进卡。对参与了本次问答的渠道，
        // 清零该渠道订阅的跟底节流：**已被提问卡淹没**的卡在下一次内容变化时立即跟底重发
        // （用户定案：答完能马上在底部看到自己的回答产生的更新）。**不**标记扰动——作答本身
        // 只是就地编辑提问卡、不产生新消息；提问期间新发的 watch 卡仍在底部，重发只会造出
        // 连续两张卡（验收反馈修正）。淹没判定完全依据提问卡发出时刻（attach 处已标记）。
        for ch in ["feishu", "telegram", "slack"] {
            if entry.coordinator.has_channel(ch) {
                for s in state
                    .watch
                    .subs
                    .lock()
                    .unwrap()
                    .iter_mut()
                    .filter(|s| s.channel == ch)
                {
                    s.last_move_ms = 0;
                }
            }
        }
        state.watch.notify.notify_one();
        log(&format!("request {} done", request_id));
    }

    /// GUI Helper 连接：凭 token 关联请求，随后进入 `serve_gui`（下发 show、收 answer 投递协调器）。
    async fn handle_gui(
        token: String,
        reader: Reader,
        w: OwnedWriteHalf,
        state: &Arc<ServerState>,
    ) {
        let Some(entry) = state.registry.attach_gui(&token) else {
            log("gui hello with unknown token; closing");
            return;
        };
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
        match entry {
            InteractionEntry::Ask(entry) => serve_gui(entry, reader, gui_tx, writer, state).await,
            InteractionEntry::Confirm(entry) => {
                serve_confirm_gui(entry, reader, gui_tx, writer, state).await
            }
        }
    }

    /// 冷/热弹窗共用的「服务」尾段：登记 `entry.gui` 写端、下发 show、补发 AgentResolved/UpdateState，
    /// 随后读 GUI 答复 / 等取消通知，收尾清空 `entry.gui` 槽并结束写任务（→ Helper EOF 退出）。
    /// 冷路径由 `handle_gui` 在 token 握手后调用；热路径由 `handle_gui_warm` 在被领用后调用。
    async fn serve_gui(
        entry: Arc<request::RequestEntry>,
        mut reader: Reader,
        gui_tx: tokio::sync::mpsc::UnboundedSender<ServerMsg>,
        writer: tokio::task::JoinHandle<()>,
        state: &Arc<ServerState>,
    ) {
        entry.gui_connected.store(true, Ordering::SeqCst);
        if let Ok(mut slot) = entry.gui.lock() {
            *slot = Some(gui_tx.clone());
        }

        // 下发题目。
        let _ = gui_tx.send(request::show_msg(&entry));

        // 方案5(b)：若调用方 agent 已异步解析完成（walk 早于本连接），握手即补发 AgentResolved
        // （覆盖「解析早于 helper 连接」竞态；解析晚于连接的情形由 spawn_agent_resolve 自行推送）。
        if let Some(r) = entry.resolved_agent.lock().unwrap().clone() {
            let _ = gui_tx.send(ServerMsg::AgentResolved {
                kind: r.kind,
                pid: r.pid,
            });
        }

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

    /// Structured confirmation popup service. A helper disconnect is a channel failure, never a
    /// synthetic denial. Readiness is accepted only after the frontend reports its first paint.
    async fn serve_confirm_gui(
        entry: Arc<request::ConfirmEntry>,
        mut reader: Reader,
        gui_tx: tokio::sync::mpsc::UnboundedSender<ServerMsg>,
        writer: tokio::task::JoinHandle<()>,
        state: &Arc<ServerState>,
    ) {
        entry.gui_connected.store(true, Ordering::SeqCst);
        if let Ok(mut slot) = entry.gui.lock() {
            *slot = Some(gui_tx.clone());
        }
        let _ = gui_tx.send(ServerMsg::Show(entry.show.clone()));

        {
            let update = state.update.lock().unwrap();
            if update.available || update.pending {
                let _ = gui_tx.send(ServerMsg::UpdateState {
                    available: update.available,
                    latest_version: update.latest_version.clone(),
                    pending: update.pending,
                });
            }
        }

        let mut failed = false;
        loop {
            tokio::select! {
                msg = ipc::read_msg::<_, ClientMsg>(&mut reader) => {
                    match msg {
                        Ok(Some(ClientMsg::ConfirmReady { request_id }))
                            if request_id == entry.request_id =>
                        {
                            if !entry.mark_ready("popup", String::new()) {
                                failed = true;
                                break;
                            }
                        }
                        Ok(Some(ClientMsg::ConfirmAnswer {
                            request_id,
                            choice_index,
                            comment,
                        })) if request_id == entry.request_id => {
                            if !entry.is_ready("popup") {
                                log(&format!(
                                    "confirmation answer arrived before popup ready for {}",
                                    entry.request_id
                                ));
                                continue;
                            }
                            match entry.coordinator.submit_wire(
                                choice_index,
                                comment,
                                "popup",
                            ) {
                                Ok(_) => break,
                                Err(error) => log(&format!(
                                    "invalid popup confirmation answer for {}: {error}",
                                    entry.request_id
                                )),
                            }
                        }
                        Ok(Some(_)) => {}
                        Ok(None) | Err(_) => {
                            failed = true;
                            break;
                        }
                    }
                }
                _ = entry.cancel.notified() => break,
            }
        }

        if let Ok(mut slot) = entry.gui.lock() {
            *slot = None;
        }
        if failed && entry.mark_failed("popup", "popup helper disconnected") {
            entry
                .coordinator
                .fallback(ConfirmFallbackReason::NoAvailableChannel);
        }
        drop(gui_tx);
        let _ = writer.await;
    }

    /// 方案6 预热弹窗连接（`--popup --warm` 拉起的进程 → `GuiWarmReady`）：建专用写端、入热池待命，
    /// 等二选一——① `dispatch_popup` 领用并经 `assign` 交来请求 entry → 进入 `serve_gui`；
    /// ② 连接 EOF/holder 死亡 → 清池 + 触发补热。**非保活**（不计入 `active`，同托盘订阅）。
    async fn handle_gui_warm(mut reader: Reader, w: OwnedWriteHalf, state: &Arc<ServerState>) {
        // 非保活：抵消 `handle_conn` 入口的 active+1，使「仅有待命热实例」时 daemon 仍可空闲退出。
        state.active.fetch_sub(1, Ordering::SeqCst);

        let (gui_tx, mut gui_rx) = tokio::sync::mpsc::unbounded_channel::<ServerMsg>();
        let mut w = w;
        let writer = tokio::spawn(async move {
            while let Some(msg) = gui_rx.recv().await {
                if ipc::write_msg(&mut w, &msg).await.is_err() {
                    break;
                }
            }
        });

        // 入池（恒 ≤1）：池已占说明已有热实例 → 本连接多余，直接关闭退出。
        // 注意：MutexGuard 非 Send，绝不能跨 await 持有 → 用紧作用域块只算出 `occupied` 布尔，再在块外 await。
        let (assign_tx, assign_rx) = tokio::sync::oneshot::channel::<InteractionEntry>();
        let occupied = {
            let mut pool = state.warm_pool.lock().unwrap();
            if pool.is_some() {
                true
            } else {
                *pool = Some(WarmSlot {
                    assign: assign_tx,
                    gui_tx: gui_tx.clone(),
                });
                state.warm_spawning.store(false, Ordering::SeqCst);
                false
            }
        };
        if occupied {
            drop(gui_tx);
            let _ = writer.await;
            state.active.fetch_add(1, Ordering::SeqCst);
            return;
        }
        log("warm popup ready; standing by");

        // 待命：等领用（biased 优先，避免与 EOF 竞态丢失领用）或连接断开（热进程死亡）。
        let assigned = tokio::select! {
            biased;
            a = assign_rx => a.ok(),
            _ = wait_cli_eof(&mut reader) => None,
        };

        match assigned {
            // 被领用：与冷路径同尾段（下发 show + 读应答）。
            Some(entry) => match entry {
                InteractionEntry::Ask(entry) => {
                    serve_gui(entry, reader, gui_tx, writer, state).await
                }
                InteractionEntry::Confirm(entry) => {
                    serve_confirm_gui(entry, reader, gui_tx, writer, state).await
                }
            },
            // 热进程死亡 / 被回收：若池中仍是本槽则清空。
            None => {
                {
                    let mut pool = state.warm_pool.lock().unwrap();
                    if pool
                        .as_ref()
                        .map(|s| s.gui_tx.same_channel(&gui_tx))
                        .unwrap_or(false)
                    {
                        *pool = None;
                    }
                }
                drop(gui_tx);
                let _ = writer.await;
            }
        }

        state.active.fetch_add(1, Ordering::SeqCst);
        // 领用 / 死亡后维持池满（self-gated：关 / 无显示 / draining 则不补）。
        maybe_topup_warm(state);
    }

    /// 是否有状态窗口在订阅。
    fn has_agent_subs(state: &Arc<ServerState>) -> bool {
        state
            .agent_subs
            .lock()
            .map(|s| !s.is_empty())
            .unwrap_or(false)
    }

    /// 构造给状态窗口的 agent 全量快照：注册表 snapshot + 注入插话「待送达」徽标
    /// （`pendingInterject: true`，spec agent-interject D7；IM /status 等其它 snapshot 消费方不注入）。
    fn agents_snapshot_for_gui(state: &Arc<ServerState>) -> serde_json::Value {
        let mut snap = state.agents.snapshot();
        let pending = state.interject.pending_sessions();
        if !pending.is_empty() {
            if let Some(arr) = snap.as_array_mut() {
                for rec in arr.iter_mut() {
                    let hit = rec
                        .get("sessionId")
                        .and_then(|v| v.as_str())
                        .map(|sid| pending.iter().any(|p| p == sid))
                        .unwrap_or(false);
                    if hit {
                        if let Some(obj) = rec.as_object_mut() {
                            obj.insert("pendingInterject".to_string(), serde_json::json!(true));
                        }
                    }
                }
            }
        }
        snap
    }

    /// 向所有状态窗口推送一次 agent 全量快照（顺带剔除已断开的发送端）。
    /// agent 忙闲变化也影响菜单栏状态，故顺带刷新 TrayState。
    fn broadcast_agents_state(state: &Arc<ServerState>) {
        let msg = ServerMsg::AgentsState {
            agents: agents_snapshot_for_gui(state),
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
            agents: agents_snapshot_for_gui(state),
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
        // Agent 子菜单摘要（spec agent-interject D7）：注册表活动会话 + 插话「待送达」标记。
        let mut agents = state.agents.tray_agent_infos();
        let pending_ij = state.interject.pending_sessions();
        for a in agents.iter_mut() {
            if pending_ij.iter().any(|p| p == &a.session_id) {
                a.pending_interject = true;
            }
        }
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
            pending_requests: state.registry.pending_infos(),
            agents,
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

    /// 插话提交的统一处理：覆盖队列（有等待 hook 时立即交付）→ 落盘 → 刷新徽标。
    fn interject_submit(state: &Arc<ServerState>, session_id: &str, text: &str) {
        state.interject.submit(session_id, text);
        state.interject.persist();
        broadcast_agents_state(state);
    }

    /// 插话追加的统一处理：保留既有队列，追加一条消息；若有等待 hook 则立即交付。
    fn interject_append(state: &Arc<ServerState>, session_id: &str, text: &str) {
        state.interject.append(session_id, text, None);
        state.interject.persist();
        broadcast_agents_state(state);
    }

    /// 插话 composer 窗口连接（spec agent-interject D7）：登记「composer 打开」（此后到来的
    /// PreToolUse poll 挂起等待），同连接上处理提交/查询；**连接断开＝关闭**（放行所有等待 hook）。
    ///
    /// **非保活**（同 `handle_tray_sub` 抵消法）：composer 可能开着放几个小时，不能借此续命 daemon。
    async fn handle_interject_composer(
        mut reader: Reader,
        mut w: OwnedWriteHalf,
        state: &Arc<ServerState>,
        session_id: String,
    ) {
        state.active.fetch_sub(1, Ordering::SeqCst);
        state.interject.composer_opened(&session_id);

        loop {
            match ipc::read_msg::<_, ClientMsg>(&mut reader).await {
                Ok(Some(ClientMsg::InterjectSubmit {
                    session_id: sid,
                    text,
                })) => {
                    interject_submit(state, &sid, &text);
                }
                Ok(Some(ClientMsg::InterjectClear { session_id: sid })) => {
                    if state.interject.clear(&sid) {
                        state.interject.persist();
                        broadcast_agents_state(state);
                    }
                }
                Ok(Some(ClientMsg::InterjectQuery { session_id: sid })) => {
                    let _ = ipc::write_msg(
                        &mut w,
                        &ServerMsg::InterjectState {
                            text: state.interject.full_text(&sid),
                            entries: state.interject.pending_count(&sid),
                        },
                    )
                    .await;
                }
                Ok(Some(_)) => {}
                Ok(None) | Err(_) => break, // 关窗 / 取消 / 宿主崩溃：连接断开即关闭。
            }
        }

        state.interject.composer_closed(&session_id);
        state.active.fetch_add(1, Ordering::SeqCst);
    }

    /// 插话 Hold：hook 连接已收到首帧 `hold`，在此等待 composer 提交/取消后回二帧
    /// `message`/`release`（spec agent-interject D3/D4）。hook 断开（自身超时/被杀）则放弃；
    /// 若消息已交付到本连接但写回失败，重新入队（不丢消息）。
    ///
    /// **非保活**（同 `handle_tray_sub` 抵消法）：等待可长达数小时，不能借此续命 daemon。
    async fn handle_interject_hold(
        mut reader: Reader,
        mut w: OwnedWriteHalf,
        state: &Arc<ServerState>,
        session_id: String,
        rx: tokio::sync::oneshot::Receiver<crate::agents::interject::WaitOutcome>,
    ) {
        use crate::agents::interject::WaitOutcome;
        use crate::ipc::InterjectAction;

        state.active.fetch_sub(1, Ordering::SeqCst);
        tokio::select! {
            outcome = rx => {
                let (action, text) = match outcome {
                    Ok(WaitOutcome::Message(text)) => (InterjectAction::Message, text),
                    // Release / 发送端消失（会话清理）→ 放行。
                    _ => (InterjectAction::Release, String::new()),
                };
                let delivered = ipc::write_msg(
                    &mut w,
                    &ServerMsg::InterjectDecision { action, text: text.clone() },
                )
                .await
                .is_ok();
                if action == InterjectAction::Message && !delivered {
                    // 极端竞态：交付瞬间 hook 恰好断开 → 消息回队，等下一次工具调用送达。
                    state.interject.submit(&session_id, &text);
                    state.interject.persist();
                    broadcast_agents_state(state);
                }
            }
            _ = wait_cli_eof(&mut reader) => {
                // hook 侧放弃（超时 fail-open / 进程被杀）：丢弃接收端即可——
                // 交付时发送端 send 失败会自动跳过本等待者（消息不丢）。
            }
        }
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

    fn spawn_confirm_popup_watchdog(entry: Arc<request::ConfirmEntry>) {
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(request::GUI_CONNECT_TIMEOUT_SECS)).await;
            if entry.mark_starting_failed("popup", "popup did not become ready in time") {
                entry
                    .coordinator
                    .fallback(ConfirmFallbackReason::NoAvailableChannel);
                entry.cancel.notify_waiters();
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
    /// 方案5(b)：accept 后**异步**从 `caller_pid` 向上 walk 进程树解析调用方 agent（家族 + pid），完成后
    /// 补刷注册表活动（补 pid / env 判不出家族的 MCP 兜底）并把结果存入 `entry` + 经 `AgentResolved`
    /// 后推弹窗 badge。`kind_env` 为 CLI 经 env 已判出的家族（None=未判出，典型 MCP `env_clear` → `walk_any`
    /// 兜底）。`ps` 游走是阻塞调用，放 blocking 线程；`caller_pid==0`（旧 CLI 不带）则跳过。
    /// 整段不阻塞请求关键路径（弹窗已先 spawn）。
    fn spawn_agent_resolve(
        entry: Arc<RequestEntry>,
        state: Arc<ServerState>,
        caller_pid: u32,
        kind_env: Option<AgentKind>,
        sid_env: Option<String>,
        from_mcp: bool,
        auto: bool,
        cwd: Option<String>,
    ) {
        if caller_pid == 0 {
            return;
        }
        tokio::spawn(async move {
            let resolved = tokio::task::spawn_blocking(move || match kind_env {
                Some(kind) => {
                    crate::agents::detect::walk_agent_pid(kind, caller_pid).map(|pid| (kind, pid))
                }
                // env 判不出家族（典型 MCP，env_clear）：进程树兜底找任意家族 agent 祖先。
                None if from_mcp => crate::agents::detect::walk_any_agent(caller_pid),
                None => None,
            })
            .await
            .ok()
            .flatten();

            let Some((kind, pid)) = resolved else {
                return;
            };

            // 注册表活动：env 有会话 → 按 session 刷新并补 pid；否则（MCP）按 pid 刷新（只更新、绝不新建）。
            let changed = match (kind_env, &sid_env) {
                (Some(k), Some(sid)) => {
                    if auto && !from_mcp {
                        state.agents.upsert_working(k, sid, Some(pid), cwd.clone())
                    } else {
                        state.agents.touch_activity(k, sid, Some(pid))
                    }
                }
                _ => state.agents.touch_activity_by_pid(kind, pid),
            };
            if changed {
                state.agents.persist();
                broadcast_agents_state(&state);
            }

            // 回填协调器的 agent 家族（MCP 场景 env 判不出，历史记录只有这条路能拿到）。
            entry.coordinator.set_agent_kind(kind.as_str().to_string());

            // 存入 entry + 后推弹窗（helper 已连上则即送；未连则握手时由 handle_gui 补发，覆盖竞态）。
            let resolved = request::ResolvedAgent {
                kind: Some(kind.as_str().to_string()),
                pid: Some(pid),
            };
            if let Ok(mut slot) = entry.resolved_agent.lock() {
                *slot = Some(resolved.clone());
            }
            if let Ok(slot) = entry.gui.lock() {
                if let Some(tx) = slot.as_ref() {
                    let _ = tx.send(ServerMsg::AgentResolved {
                        kind: resolved.kind,
                        pid: resolved.pid,
                    });
                }
            }
        });
    }

    /// 是否启用了任一 IM 渠道（仅看非密钥的 `enabled` 标志）。用于方案4 在读钥匙串前的廉价门控：
    /// 可安全用 `load_without_secrets()` 的结果判定（`enabled` 不是密钥）。
    fn any_im_enabled(config: &AppConfig) -> bool {
        let ch = &config.channels;
        ch.dingding.enabled || ch.feishu.enabled || ch.telegram.enabled || ch.slack.enabled
    }

    async fn attach_im_channels(
        entry: &Arc<RequestEntry>,
        state: &Arc<ServerState>,
        w: &mut OwnedWriteHalf,
        lang: Lang,
    ) -> bool {
        // 方案4（spec §4）：先用 config.json 的 `enabled` 标志（`load_without_secrets`，零钥匙串）判定
        // 有无任何启用的 IM 渠道；都没启用（最常见的「仅弹窗」用户）则**完全跳过** `AppConfig::load()`，
        // 不读 OS 钥匙串。注意只能用 `enabled` 标志：`is_*_active` 要构造 client、依赖密钥，缺密钥会误判。
        if !any_im_enabled(&AppConfig::load_without_secrets()) {
            return false;
        }
        let config = AppConfig::load();
        let request = entry.request().clone();
        let sink = entry.coordinator.clone();
        let mut attached = false;

        // 「IM 会话期自动激活」：开关开时，投放渠道 = 当前活跃槽 ∪ 正在 watch 本次调用方 agent 的
        // 渠道（watch 卡显示「等待回答」却收不到提问卡的困惑，计划 §6 M4 定案；多渠道并发时
        // 抢答收尾机制原样复用）。其余 IM 由入站监听器保持连接、只监听 here，不发卡片。
        // 开关关时维持旧「全发」行为。
        let auto = config.channels.auto_activation;
        let active = state.active_channel.lock().unwrap().clone();
        let watching: Vec<String> = match entry.agent_session_id.as_ref() {
            Some(sid) => state
                .watch
                .subs
                .lock()
                .unwrap()
                .iter()
                .filter(|s| &s.session_id == sid && !s.rewatchable)
                .map(|s| s.channel.clone())
                .collect(),
            None => Vec::new(),
        };
        let want = |id: &str| -> bool {
            !auto || active.as_deref() == Some(id) || watching.iter().any(|w| w == id)
        };

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

    fn confirm_im_candidates(
        entry: &Arc<request::ConfirmEntry>,
        state: &Arc<ServerState>,
        config: &AppConfig,
    ) -> Vec<&'static str> {
        let auto = config.channels.auto_activation;
        let active = state.active_channel.lock().unwrap().clone();
        let watching: Vec<String> = state
            .watch
            .subs
            .lock()
            .unwrap()
            .iter()
            .filter(|watch| watch.session_id == entry.agent_session_id && !watch.rewatchable)
            .map(|watch| watch.channel.clone())
            .collect();
        let want = |id: &str| {
            !auto || active.as_deref() == Some(id) || watching.iter().any(|watch| watch == id)
        };
        let mut candidates = Vec::new();
        if want("dingding") && crate::app::is_dingding_active(config) {
            candidates.push("dingding");
        }
        if want("feishu") && crate::app::is_feishu_active(config) {
            candidates.push("feishu");
        }
        if want("telegram") && crate::app::is_telegram_active(config) {
            candidates.push("telegram");
        }
        if want("slack") && crate::app::is_slack_active(config) {
            candidates.push("slack");
        }
        candidates
    }

    async fn attach_confirm_im_channels(
        entry: &Arc<request::ConfirmEntry>,
        state: &Arc<ServerState>,
        config: &AppConfig,
        candidates: &[&str],
    ) {
        for channel in candidates {
            match *channel {
                "feishu" => match ensure_fs_router(state, &config.channels.feishu).await {
                    Some(router) => crate::channels::confirm::start_feishu(
                        entry.clone(),
                        config.channels.feishu.clone(),
                        router,
                    ),
                    None => {
                        if entry.mark_failed("feishu", "Feishu router unavailable") {
                            entry
                                .coordinator
                                .fallback(ConfirmFallbackReason::NoAvailableChannel);
                        }
                    }
                },
                "telegram" => match ensure_tg_router(state, &config.channels.telegram).await {
                    Some(router) => crate::channels::confirm::start_telegram(
                        entry.clone(),
                        config.channels.telegram.clone(),
                        router,
                    ),
                    None => {
                        if entry.mark_failed("telegram", "Telegram router unavailable") {
                            entry
                                .coordinator
                                .fallback(ConfirmFallbackReason::NoAvailableChannel);
                        }
                    }
                },
                "slack" => match ensure_sl_router(state, &config.channels.slack).await {
                    Some(router) => crate::channels::confirm::start_slack(
                        entry.clone(),
                        config.channels.slack.clone(),
                        router,
                    ),
                    None => {
                        if entry.mark_failed("slack", "Slack router unavailable") {
                            entry
                                .coordinator
                                .fallback(ConfirmFallbackReason::NoAvailableChannel);
                        }
                    }
                },
                "dingding" => match ensure_dd_router(
                    state,
                    config.channels.dingding.client_id.trim(),
                    config.channels.dingding.client_secret.trim(),
                )
                .await
                {
                    Some(router) => crate::channels::confirm::start_dingtalk(
                        entry.clone(),
                        config.channels.dingding.clone(),
                        router,
                    ),
                    None => {
                        if entry.mark_failed("dingding", "DingTalk router unavailable") {
                            entry
                                .coordinator
                                .fallback(ConfirmFallbackReason::NoAvailableChannel);
                        }
                    }
                },
                _ => {}
            }
        }
    }

    // ===== IM 入站消费（命令 /here、/status…）/ 活跃槽 / 补推在途 =====

    /// 确保各已启用 IM 的入站消费任务在线，使守护进程**在世期间**能收到任何入站消息（命令 / 引导 / 作答确认）。
    /// 触发条件 = **daemon 存活 + 有启用 IM**（「存活即监听」，spec R1）：与工作中 agent / 在途提问 / 自动激活
    /// 开关全部无关。连接随 daemon 退出而释放（serve 收尾丢弃 Router → Drop 关长连接），故无需主动断连；
    /// 监听不计入保活、不阻止空闲退出。在 `serve()` 启动后台调用一次，并在受理 / 配置变更处幂等重调。
    /// 各渠道只提供「连接 Router + 取原始消息观察者 + 抽取 (发送者, 文本?) + 期望发送者」这几样传输原语；
    /// 通用循环与命令分派（`spawn_listener` / `handle_inbound`）一份实现，各渠道复用。幂等：可反复调用。
    async fn ensure_inbound_listeners(state: &Arc<ServerState>) {
        // 「存活即监听」：不再用「有工作中 agent」门控——只要 daemon 存活且有启用 IM 就监听，与工作中 agent /
        // 在途提问 / 自动激活开关全部无关（使任何消息在世期间都能被收到并回复）。
        // 方案4（spec §4）：仍先用 `enabled` 标志（零钥匙串）门控——无任何启用的 IM 渠道则无须建监听，
        // 跳过 `AppConfig::load()` 的钥匙串读取。
        if !any_im_enabled(&AppConfig::load_without_secrets()) {
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

        // 兜底：随 Router 重建恢复活动单选卡的按钮回调路由（无 picker 时为 no-op）。
        ensure_select_routes(state).await;
    }

    /// 通用入站监听循环（与渠道无关）：从原始消息流抽取 (发送者, 文本)，按期望发送者过滤后交 `handle_inbound`。
    /// 收到 `stop`（改配置时由 `invalidate_changed_routers` 触发）或流结束（连接断开）即退出，
    /// 退出时**按身份**释放监听位（不误删改配置后新建监听的认领），下次提问 / 配置变更可重建。
    /// 一条经身份过滤前的入站消息（抽取器输出）。`text` 为 `None` 表示来自发送者的**非文本**消息
    /// （图片/文件等）——文本走命令/引导分派，非文本仅在「无在途提问」时回引导（见 `handle_inbound`）。
    struct Inbound {
        sender: String,
        text: Option<String>,
    }

    fn spawn_listener(
        state: &Arc<ServerState>,
        channel_id: &'static str,
        mut rx: tokio::sync::mpsc::UnboundedReceiver<serde_json::Value>,
        extract: fn(&serde_json::Value) -> Option<Inbound>,
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
                            if let Some(inb) = extract(&ev) {
                                // 单聊机器人：仅处理期望发送者发来的消息（期望为空则不过滤）；过滤掉机器人自身回声。
                                if !expected_sender.is_empty() && inb.sender != expected_sender {
                                    continue;
                                }
                                handle_inbound(&state, channel_id, inb.text.as_deref()).await;
                            }
                        }
                        None => break,
                    },
                }
            }
            state.inbound_listeners.release(channel_id, &stop);
        });
    }

    /// 飞书原始消息 → `Inbound`（发送者 open_id + 文本？）；非文本时 `text=None`、非消息事件返回 None。
    fn extract_feishu(ev: &serde_json::Value) -> Option<Inbound> {
        let open_id = ev
            .get("sender")
            .and_then(|s| s.get("sender_id"))
            .and_then(|i| i.get("open_id"))
            .and_then(|v| v.as_str())?
            .to_string();
        ev.get("message")?; // 确保是一条消息事件
        let text = fs_text_and_sender(ev).map(|(_, t)| t);
        Some(Inbound {
            sender: open_id,
            text,
        })
    }

    /// 钉钉原始 bot 消息 → `Inbound`（senderStaffId + 文本？）；非文本时 `text=None`。
    fn extract_dingtalk(ev: &serde_json::Value) -> Option<Inbound> {
        let sender = ev
            .get("senderStaffId")
            .and_then(|v| v.as_str())?
            .to_string();
        let text = ev
            .get("text")
            .and_then(|t| t.get("content"))
            .and_then(|c| c.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string());
        Some(Inbound { sender, text })
    }

    /// Slack 原始消息事件 → `Inbound`（user + 文本？）；非文本时 `text=None`、无发送者返回 None。
    fn extract_slack(ev: &serde_json::Value) -> Option<Inbound> {
        let user = ev.get("user").and_then(|v| v.as_str())?.to_string();
        let text = ev
            .get("text")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string());
        Some(Inbound { sender: user, text })
    }

    /// Telegram 原始 `message` 对象 → `Inbound`（chat id + 文本？）。Router 仅转发文本消息，
    /// 故 `text` 实际恒为 `Some`；为统一签名仍按 `Option` 处理。
    fn extract_telegram(ev: &serde_json::Value) -> Option<Inbound> {
        let chat = ev
            .get("chat")
            .and_then(|c| c.get("id"))
            .and_then(|v| v.as_i64())?
            .to_string();
        let text = ev
            .get("text")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string());
        Some(Inbound { sender: chat, text })
    }

    /// 该渠道当前是否有「活动在途提问」（即有在途请求把本渠道挂进了协调器）。
    /// 用于「普通文本退避」判定：有则交渠道会话确认/引导，观察者不重复回复（spec 协调原则）。
    fn has_active_question_on(state: &Arc<ServerState>, channel_id: &str) -> bool {
        let ask = state
            .registry
            .in_flight_entries()
            .iter()
            .any(|e| e.coordinator.has_channel(channel_id));
        ask || state
            .registry
            .in_flight_confirm_entries()
            .iter()
            .any(|entry| entry.has_live_delivery(channel_id))
    }

    /// 该渠道当前是否有在途单选卡（picker 未被消费）。用于 `remove_picker` 判定是否仍有单选卡残留。
    fn has_active_select_on(state: &Arc<ServerState>, channel_id: &str) -> bool {
        state
            .select
            .pickers
            .lock()
            .unwrap()
            .iter()
            .any(|p| p.channel == channel_id)
    }

    /// 该渠道是否有「仍位于会话底部」的单选卡：picker 发出后未再出现非 watch 消息（`posted_ms >=`
    /// 渠道 disturb 水位）。**仅当单选卡还是最后一条消息时才抑制 watch 跟底**（免打断正在进行的单选）；
    /// 一旦被其它消息淹没即放开跟底（用户定案：忘记选择的旧单选卡不该长期卡住 watch 跟底）。
    fn select_is_last_on(state: &Arc<ServerState>, channel_id: &str) -> bool {
        let disturb = state
            .watch
            .disturb
            .lock()
            .unwrap()
            .get(channel_id)
            .copied()
            .unwrap_or(0);
        state
            .select
            .pickers
            .lock()
            .unwrap()
            .iter()
            .any(|p| p.channel == channel_id && p.posted_ms >= disturb)
    }

    // ===== /watch 实时关注引擎（spec docs/specs/im-watch.md，P1 仅飞书）=====

    /// 从注册表快照数组中按 session_id 找记录。
    fn find_agent_by_session<'a>(
        snapshot: &'a serde_json::Value,
        session_id: &str,
    ) -> Option<&'a serde_json::Value> {
        snapshot
            .as_array()?
            .iter()
            .find(|r| r.get("sessionId").and_then(|v| v.as_str()) == Some(session_id))
    }

    /// 标记某渠道出现一条「非 watch」消息（用户入站消息 / 机器人文本回执 / 提问会话）。
    /// 这是跟底判定的**淹没信号**：发出时刻早于该时刻的 watch 卡已被顶上去，下一次内容变化时
    /// 跟底重发（watch 卡自身的发送/编辑不经此处，watch 卡之间互不影响、无级联）。
    fn mark_watch_disturbed(state: &Arc<ServerState>, channel_id: &str) {
        if !crate::watch::channel_supported(channel_id) {
            return;
        }
        state
            .watch
            .disturb
            .lock()
            .unwrap()
            .insert(channel_id.to_string(), now_ms());
    }

    /// watch 卡的渠道传输：发送 / 就地编辑一张实时状态卡，屏蔽各渠道 API 与标记语言差异。
    /// 渲染入口各渠道自备（飞书 `watch::card_view`+`build_watch_card`、Telegram HTML、
    /// Slack Block Kit），消息 id 编码见 `WatchEntry::message_id`。
    enum WatchClient {
        Feishu(crate::feishu::client::FeishuClient),
        Telegram(crate::telegram::TelegramClient),
        Slack {
            client: crate::slack::client::SlackClient,
            /// DM 频道 id（`conversations.open` 解析；同一 user 稳定）。
            dm: String,
        },
        /// 钉钉互动卡片高级版：专用 watch 模板 + 变量更新（`dingtalk/watch.rs`）。
        DingTalk(crate::dingtalk::client::DingTalkClient),
    }

    impl WatchClient {
        /// 按渠道构造。渠道未配置/不可用 → None（该渠道订阅本拍跳过，下一拍重试）。
        async fn for_channel(channel_id: &str, config: &AppConfig) -> Option<WatchClient> {
            match channel_id {
                "feishu" => crate::feishu::client::FeishuClient::new(&config.channels.feishu)
                    .ok()
                    .map(WatchClient::Feishu),
                "telegram" => {
                    let tg = &config.channels.telegram;
                    crate::telegram::TelegramClient::new(
                        tg.bot_token.clone(),
                        tg.chat_id.clone(),
                        tg.api_base_url.clone(),
                    )
                    .ok()
                    .map(WatchClient::Telegram)
                }
                "slack" => {
                    let client =
                        crate::slack::client::SlackClient::new(&config.channels.slack).ok()?;
                    let dm = client.open_dm().await.ok()?;
                    Some(WatchClient::Slack { client, dm })
                }
                "dingding" => {
                    crate::dingtalk::client::DingTalkClient::new(&config.channels.dingding)
                        .ok()
                        .map(WatchClient::DingTalk)
                }
                _ => None,
            }
        }

        /// 每卡最短编辑间隔（毫秒）。Slack `chat.update` 限频更紧（Tier 3 ≈50/min），取 2s
        /// （计划 R5 定案；签名门控使实际编辑远稀于理论上限）；飞书 PATCH / Telegram
        /// editMessageText / 钉钉实例更新（PoC 实测 2s×150 连发零频控，p50 ≈60–95ms）取 1s。
        fn min_edit_interval_ms(&self) -> u64 {
            match self {
                WatchClient::Slack { .. } => 2000,
                _ => 1000,
            }
        }

        /// 发送一张新卡，返回消息 id（编码见 `WatchEntry::message_id`）。
        async fn send(
            &self,
            frame: &crate::watch::WatchFrame,
            mode: crate::watch::CardMode,
            now: u64,
            lang: Lang,
        ) -> Result<String, String> {
            match self {
                WatchClient::Feishu(c) => {
                    let card = crate::feishu::card::build_watch_card(&crate::watch::card_view(
                        frame, mode, now, lang, None,
                    ));
                    c.send_card(&card).await.map_err(|e| e.to_string())
                }
                WatchClient::Telegram(c) => {
                    // 先按 mode 取 markup（借用），再把 mode 交给渲染（CardMode 非 Copy）。
                    let markup = matches!(mode, crate::watch::CardMode::Active)
                        .then(|| crate::telegram::watch::inline_keyboard(lang));
                    let html = crate::telegram::watch::render_watch_html(frame, mode, now, lang);
                    c.send_message(&html, Some("HTML"), markup)
                        .await
                        .map(|mid| mid.to_string())
                        .map_err(|e| e.to_string())
                }
                WatchClient::Slack { client, dm } => {
                    let (blocks, fallback) =
                        crate::slack::watch::build_watch_blocks(frame, mode, now, lang, None);
                    client
                        .post_message(dm, Some(&blocks), &fallback)
                        .await
                        .map_err(|e| e.to_string())
                }
                WatchClient::DingTalk(c) => {
                    // 消息 id = 自铸 outTrackId（钉钉创建卡片由调用方指定实例 id，天然可编辑）。
                    let otid = format!("watch-{}", uuid::Uuid::new_v4());
                    let map = crate::dingtalk::watch::build_watch_param_map(frame, mode, now, lang);
                    c.create_and_deliver_card(
                        &otid,
                        crate::dingtalk::watch::DEFAULT_WATCH_CARD_TEMPLATE_ID,
                        map,
                        serde_json::json!({}),
                    )
                    .await
                    .map(|()| otid)
                    .map_err(|e| e.to_string())
                }
            }
        }

        /// 就地编辑已发出的卡。`session_id`：`AutoStopped` 终态需传入以嵌入重新关注按钮。
        async fn edit(
            &self,
            message_id: &str,
            frame: &crate::watch::WatchFrame,
            mode: crate::watch::CardMode,
            now: u64,
            lang: Lang,
            session_id: Option<&str>,
        ) -> Result<(), String> {
            match self {
                WatchClient::Feishu(c) => {
                    let card = crate::feishu::card::build_watch_card(&crate::watch::card_view(
                        frame, mode, now, lang, session_id,
                    ));
                    c.patch_card(message_id, &card)
                        .await
                        .map_err(|e| e.to_string())
                }
                WatchClient::Telegram(c) => {
                    let mid: i64 = message_id
                        .parse()
                        .map_err(|_| "bad message id".to_string())?;
                    match (&mode, session_id) {
                        (crate::watch::CardMode::Final(kind), Some(_)) if kind.is_rewatchable() => {
                            let markup = crate::telegram::watch::rewatch_keyboard(kind, lang);
                            let html = crate::telegram::watch::render_watch_html(
                                frame,
                                crate::watch::CardMode::Active,
                                now,
                                lang,
                            );
                            c.edit_message_text(mid, &html, Some("HTML"), Some(markup))
                                .await
                                .map_err(|e| e.to_string())
                        }
                        _ => {
                            let markup = matches!(mode, crate::watch::CardMode::Active)
                                .then(|| crate::telegram::watch::inline_keyboard(lang));
                            let html =
                                crate::telegram::watch::render_watch_html(frame, mode, now, lang);
                            c.edit_message_text(mid, &html, Some("HTML"), markup)
                                .await
                                .map_err(|e| e.to_string())
                        }
                    }
                }
                WatchClient::Slack { client, dm } => {
                    let (blocks, fallback) =
                        crate::slack::watch::build_watch_blocks(frame, mode, now, lang, session_id);
                    client
                        .update_message(dm, message_id, Some(&blocks), &fallback)
                        .await
                        .map_err(|e| e.to_string())
                }
                WatchClient::DingTalk(c) => {
                    let map = crate::dingtalk::watch::build_watch_param_map(frame, mode, now, lang);
                    c.update_card_private(message_id, map, serde_json::json!({}))
                        .await
                        .map_err(|e| e.to_string())
                }
            }
        }
    }

    /// 把当前订阅持久化到 `~/.askhuman/state/watch.json`（daemon 重启后恢复、继续编辑同卡）。
    fn persist_watch_subs(state: &Arc<ServerState>) {
        let items: Vec<crate::watch::PersistedWatch> = state
            .watch
            .subs
            .lock()
            .unwrap()
            .iter()
            .map(|s| crate::watch::PersistedWatch {
                channel: s.channel.clone(),
                session_id: s.session_id.clone(),
                message_id: s.message_id.clone(),
                created_at: s.created_at,
                rewatchable: s.rewatchable,
            })
            .collect();
        crate::watch::save(&items);
    }

    /// watch 引擎入口：恢复持久化订阅（按 session 重解析展示编号——`seq` 不跨重启保留），
    /// 然后进入「Notify 即醒 / 自适应 tick」循环：有「工作中」订阅 2s 一拍、只有空闲订阅 10s
    /// 一拍、无订阅纯等 Notify（零空转）。
    async fn watch_restore_and_run(state: Arc<ServerState>) {
        let persisted = crate::watch::load();
        if !persisted.is_empty() {
            let snapshot = state.agents.snapshot();
            let mut subs: Vec<WatchEntry> = Vec::new();
            for p in persisted {
                if !crate::watch::channel_supported(&p.channel) || p.message_id.is_empty() {
                    continue;
                }
                // 记录已彻底消失 → seq=0 占位；首拍会渲染终态并自动退订。
                let seq = find_agent_by_session(&snapshot, &p.session_id)
                    .and_then(|r| r.get("seq").and_then(|v| v.as_u64()))
                    .unwrap_or(0);
                subs.push(WatchEntry {
                    channel: p.channel,
                    session_id: p.session_id,
                    message_id: p.message_id,
                    seq,
                    created_at: p.created_at,
                    last_sig: String::new(),
                    last_edit_ms: 0,
                    fails: 0,
                    working: false,
                    // 重启后 disturb 从 0 起算，恢复的卡先视为未淹没；一有新扰动即可跟底（不节流）。
                    sent_at_ms: p.created_at.saturating_mul(1000),
                    last_move_ms: 0,
                    rewatchable: p.rewatchable,
                });
            }
            if !subs.is_empty() {
                log(&format!("watch: restored {} subscription(s)", subs.len()));
                *state.watch.subs.lock().unwrap() = subs;
                ensure_watch_routes(&state).await;
            }
        }
        loop {
            let wait = {
                let subs = state.watch.subs.lock().unwrap();
                let has_active = subs.iter().any(|s| !s.rewatchable);
                if !has_active {
                    None
                } else if subs.iter().any(|s| !s.rewatchable && s.working) {
                    Some(Duration::from_secs(2))
                } else {
                    Some(Duration::from_secs(10))
                }
            };
            match wait {
                None => state.watch.notify.notified().await,
                Some(d) => {
                    tokio::select! {
                        _ = state.watch.notify.notified() => {}
                        _ = tokio::time::sleep(d) => {}
                    }
                }
            }
            watch_tick(&state).await;
        }
    }

    /// 引擎一拍：对每个订阅重算帧，**签名变化才**编辑卡片（帧是全量的，丢帧无损）；
    /// agent 结束 → 终态定格 + 自动退订；连续失败 ≥5 退订。按渠道分组：每渠道各建一次
    /// 传输客户端、各取各的淹没水位与在途提问。末尾幂等确保回调路由在位。
    async fn watch_tick(state: &Arc<ServerState>) {
        let all_entries: Vec<WatchEntry> = state.watch.subs.lock().unwrap().clone();
        // 活跃 entry：引擎只驱动非 rewatchable 的订阅（rewatchable 保留仅供回调路由）。
        let entries: Vec<&WatchEntry> = all_entries.iter().filter(|e| !e.rewatchable).collect();
        if entries.is_empty() {
            ensure_watch_routes(state).await;
            return;
        }
        let config = AppConfig::load();
        let lang = Lang::current();
        let now = now_secs();
        let snapshot = state.agents.snapshot();
        let waiting = state.registry.in_flight_agent_session_ids();
        let mut channels: Vec<String> = entries.iter().map(|e| e.channel.clone()).collect();
        channels.sort();
        channels.dedup();
        let mut changed = false;
        for ch in channels {
            // 渠道不可用（配置被关/失效）→ 本拍跳过该渠道订阅，下一拍重试。
            let Some(client) = WatchClient::for_channel(&ch, &config).await else {
                continue;
            };
            // 跟底判定的渠道量：淹没水位线 + 是否有在途提问 + 单选卡是否仍在会话底部（二者期间均抑制
            // 跟底，只就地编辑，不打断问答 / 单选交互）。单选卡抑制**仅在它还是最后一条消息时**生效——
            // 被其它消息淹没（含用户忘记选择后又发了别的）即放开跟底（用户定案）。
            let disturb = state
                .watch
                .disturb
                .lock()
                .unwrap()
                .get(&ch)
                .copied()
                .unwrap_or(0);
            let ask_active = has_active_question_on(state, &ch);
            let select_active = select_is_last_on(state, &ch);
            for e in entries.iter().filter(|e| e.channel == ch) {
                let rec = find_agent_by_session(&snapshot, &e.session_id);
                let frame = crate::watch::build_frame(e.seq, rec, waiting.contains(&e.session_id));
                let ended = frame.phase == crate::watch::WatchPhase::Ended;
                let idle = frame.phase == crate::watch::WatchPhase::Idle;
                let finalize = ended || idle;
                let sig = crate::watch::signature(&frame);
                if !finalize && sig == e.last_sig {
                    continue; // 内容没变，不编辑。
                }
                // 每卡最短编辑间隔按渠道（终态豁免：定格必须落地）；漏掉的变化下一拍补上。
                if !finalize
                    && now_ms().saturating_sub(e.last_edit_ms) < client.min_edit_interval_ms()
                {
                    continue;
                }
                let mode = if ended {
                    crate::watch::CardMode::Final(crate::watch::FinalKind::Ended)
                } else if idle {
                    crate::watch::CardMode::Final(crate::watch::FinalKind::Idle)
                } else {
                    crate::watch::CardMode::Active
                };
                // 跟底：卡已被非 watch 消息淹没 + 无在途提问 + 无在途单选卡 + 30s 节流
                // （`last_move_ms == 0` 豁免：答复 / 单选完结、重启恢复）→ 发新卡到会话底部，
                // 旧卡定格「已移至最新卡片」。
                let buried = disturb > e.sent_at_ms;
                let move_ok = buried
                    && !ask_active
                    && !select_active
                    && (e.last_move_ms == 0
                        || now_ms().saturating_sub(e.last_move_ms) >= WATCH_MOVE_THROTTLE_MS);
                if move_ok {
                    match client.send(&frame, mode, now, lang).await {
                        Ok(new_mid) => {
                            // 旧卡定格（best-effort：失败只记日志，新卡已接管）。
                            if let Err(err) = client
                                .edit(
                                    &e.message_id,
                                    &frame,
                                    crate::watch::CardMode::Final(crate::watch::FinalKind::Moved),
                                    now,
                                    lang,
                                    None,
                                )
                                .await
                            {
                                log(&format!("watch: finalize moved card failed: {}", err));
                            }
                            let mut subs = state.watch.subs.lock().unwrap();
                            if finalize {
                                // 新卡即终态卡（ended / idle）：定格已随发送完成 → 退订。
                                subs.retain(|s| s.message_id != e.message_id);
                            } else if let Some(s) =
                                subs.iter_mut().find(|s| s.message_id == e.message_id)
                            {
                                s.message_id = new_mid;
                                s.sent_at_ms = now_ms();
                                s.last_move_ms = now_ms();
                                s.last_sig = sig;
                                s.last_edit_ms = now_ms();
                                s.fails = 0;
                                s.working = frame.phase == crate::watch::WatchPhase::Working;
                            }
                            changed = true; // message_id 变了：持久化 + 路由重建。
                        }
                        Err(err) => {
                            // 发送失败与编辑失败同流：计失败数、下一拍重试（帧全量，丢帧无损）。
                            log(&format!("watch: move card failed: {}", err));
                            let mut subs = state.watch.subs.lock().unwrap();
                            let mut drop_it = false;
                            if let Some(s) = subs.iter_mut().find(|s| s.message_id == e.message_id)
                            {
                                s.fails += 1;
                                drop_it = s.fails >= 5;
                            }
                            if drop_it {
                                log("watch: too many consecutive failures; unsubscribed");
                                subs.retain(|s| s.message_id != e.message_id);
                                changed = true;
                            }
                        }
                    }
                    continue;
                }
                match client
                    .edit(&e.message_id, &frame, mode, now, lang, None)
                    .await
                {
                    Ok(()) => {
                        let mut subs = state.watch.subs.lock().unwrap();
                        if finalize {
                            // 定格成功（ended / idle）→ 自动退订。
                            subs.retain(|s| s.message_id != e.message_id);
                            changed = true;
                        } else if let Some(s) =
                            subs.iter_mut().find(|s| s.message_id == e.message_id)
                        {
                            s.last_sig = sig;
                            s.last_edit_ms = now_ms();
                            s.fails = 0;
                            s.working = frame.phase == crate::watch::WatchPhase::Working;
                        }
                    }
                    Err(err) => {
                        log(&format!("watch: patch card failed: {}", err));
                        let mut subs = state.watch.subs.lock().unwrap();
                        let mut drop_it = false;
                        if let Some(s) = subs.iter_mut().find(|s| s.message_id == e.message_id) {
                            s.fails += 1;
                            drop_it = s.fails >= 5;
                        }
                        if drop_it {
                            log("watch: too many consecutive failures; unsubscribed");
                            subs.retain(|s| s.message_id != e.message_id);
                            changed = true;
                        }
                    }
                }
            }
        }
        // rewatchable entry TTL 清理。
        {
            let now = now_secs();
            let mut subs = state.watch.subs.lock().unwrap();
            let before = subs.len();
            subs.retain(|s| {
                !s.rewatchable || now.saturating_sub(s.created_at) < REWATCHABLE_TTL_SECS
            });
            if subs.len() != before {
                changed = true;
            }
        }
        if changed {
            persist_watch_subs(state);
        }
        ensure_watch_routes(state).await;
    }

    /// 幂等确保各渠道 watch 卡按钮回调路由在位：在渠道 Router 上注册一条专用路由并认领本渠道
    /// 全部卡片 message_id。绑定的 Router 失活 / 订阅集合变化 → 停旧任务整体重建；无订阅则撤路由。
    async fn ensure_watch_routes(state: &Arc<ServerState>) {
        // 渠道 → 该渠道当前应认领的卡 id 集合（已排序）。
        let mut desired: HashMap<String, Vec<String>> = HashMap::new();
        for s in state.watch.subs.lock().unwrap().iter() {
            desired
                .entry(s.channel.clone())
                .or_default()
                .push(s.message_id.clone());
        }
        for mids in desired.values_mut() {
            mids.sort();
        }
        // 撤掉已无订阅的渠道路由。
        {
            let mut routes = state.watch.routes.lock().unwrap();
            routes.retain(|ch, h| {
                if desired.contains_key(ch) {
                    true
                } else {
                    h.stop.notify_waiters();
                    false
                }
            });
        }
        let config = AppConfig::load();
        for (ch, mids) in desired {
            ensure_watch_route_for(state, &config, &ch, mids).await;
        }
    }

    /// 幂等确保单一渠道的 watch 回调路由任务在位。
    async fn ensure_watch_route_for(
        state: &Arc<ServerState>,
        config: &AppConfig,
        channel_id: &str,
        mids: Vec<String>,
    ) {
        // 取该渠道的共享 Router（渠道不可用则跳过；订阅仍在，渠道恢复后下一拍补挂）。
        let router: WatchChannelRouter = match channel_id {
            "feishu" => {
                if !crate::app::is_feishu_active(config) {
                    return;
                }
                match ensure_fs_router(state, &config.channels.feishu).await {
                    Some(r) => WatchChannelRouter::Feishu(r),
                    None => return,
                }
            }
            "telegram" => {
                if !crate::app::is_telegram_active(config) {
                    return;
                }
                match ensure_tg_router(state, &config.channels.telegram).await {
                    Some(r) => WatchChannelRouter::Telegram(r),
                    None => return,
                }
            }
            "slack" => {
                if !crate::app::is_slack_active(config) {
                    return;
                }
                match ensure_sl_router(state, &config.channels.slack).await {
                    Some(r) => WatchChannelRouter::Slack(r),
                    None => return,
                }
            }
            "dingding" => {
                if !crate::app::is_dingding_active(config) {
                    return;
                }
                let dd = &config.channels.dingding;
                match ensure_dd_router(state, dd.client_id.trim(), dd.client_secret.trim()).await {
                    Some(r) => WatchChannelRouter::DingTalk(r),
                    None => return,
                }
            }
            _ => return,
        };
        // 现任务仍绑定同一存活 Router 且卡集合未变 → 无事可做。
        {
            let routes = state.watch.routes.lock().unwrap();
            if let Some(h) = routes.get(channel_id) {
                if h.router.is_same_alive(&router) && h.mids == mids {
                    return;
                }
            }
        }
        // 重建：注册新路由认领全部卡，替换句柄并停旧任务（其 Routed* Drop 时自清路由表）。
        let stop = Arc::new(tokio::sync::Notify::new());
        let (router_ref, task): (WatchRouterRef, tokio::task::JoinHandle<()>) = match &router {
            WatchChannelRouter::Feishu(r) => {
                let mut routed = r.register();
                for mid in &mids {
                    routed.set_active(Some(mid), "");
                }
                let st = state.clone();
                let stop2 = stop.clone();
                let task = tokio::spawn(async move {
                    loop {
                        tokio::select! {
                            _ = stop2.notified() => break,
                            ev = routed.recv() => match ev {
                                Some(crate::feishu::router::FsInbound::Card { data, ack }) => {
                                    handle_watch_card_action(&st, &data, ack);
                                }
                                Some(_) => {} // 未认领聊天消息，不会到达；防御性忽略。
                                None => break, // Router 断开：下一拍 ensure 重建。
                            },
                        }
                    }
                });
                (WatchRouterRef::Feishu(Arc::downgrade(r)), task)
            }
            WatchChannelRouter::Telegram(r) => {
                // 仅认领卡片回调（`set_card_route`），**不**认领自由文字——不得抢走提问卡答案。
                let routed = r.register();
                for mid in &mids {
                    if let Ok(m) = mid.parse::<i64>() {
                        routed.set_card_route(m);
                    }
                }
                let st = state.clone();
                let stop2 = stop.clone();
                let mut routed = routed;
                let task = tokio::spawn(async move {
                    loop {
                        tokio::select! {
                            _ = stop2.notified() => break,
                            ev = routed.recv() => match ev {
                                Some(crate::telegram::router::TgInbound::Callback(cb)) => {
                                    handle_watch_tg_action(&st, &cb).await;
                                }
                                Some(_) => {} // 未认领自由文字，不会到达；防御性忽略。
                                None => break,
                            },
                        }
                    }
                });
                (WatchRouterRef::Telegram(Arc::downgrade(r)), task)
            }
            WatchChannelRouter::Slack(r) => {
                // user_id 传空 → 只认领卡片交互（message_ts），不认领聊天消息。
                let mut routed = r.register();
                for mid in &mids {
                    routed.set_active(Some(mid), "");
                }
                let st = state.clone();
                let stop2 = stop.clone();
                let task = tokio::spawn(async move {
                    loop {
                        tokio::select! {
                            _ = stop2.notified() => break,
                            ev = routed.recv() => match ev {
                                Some(crate::slack::router::SlInbound::Interactive(payload)) => {
                                    handle_watch_slack_action(&st, &payload).await;
                                }
                                Some(_) => {}
                                None => break,
                            },
                        }
                    }
                });
                (WatchRouterRef::Slack(Arc::downgrade(r)), task)
            }
            WatchChannelRouter::DingTalk(r) => {
                // user_id 传空 → 只认领卡片回调（outTrackId），不认领该用户的聊天消息。
                let mut routed = r.register();
                for mid in &mids {
                    routed.set_active(Some(mid), "");
                }
                let st = state.clone();
                let stop2 = stop.clone();
                let task = tokio::spawn(async move {
                    loop {
                        tokio::select! {
                            _ = stop2.notified() => break,
                            ev = routed.recv() => match ev {
                                Some(crate::dingtalk::router::DdInbound::Card { data, ack }) => {
                                    // 先空 ACK 满足 3 秒回包（钉钉无「回调同步回卡」，新帧走 OpenAPI 编辑）。
                                    let _ = ack.send(serde_json::json!({}));
                                    handle_watch_dd_action(&st, &data).await;
                                }
                                Some(_) => {} // 未认领聊天消息，不会到达；防御性忽略。
                                None => break, // Router 断开：下一拍 ensure 重建。
                            },
                        }
                    }
                });
                (WatchRouterRef::DingTalk(Arc::downgrade(r)), task)
            }
        };
        let _ = task; // 任务由 stop 信号控制生命周期；句柄本身无需保留。
        if let Some(old) = state.watch.routes.lock().unwrap().insert(
            channel_id.to_string(),
            WatchRouteHandle {
                stop,
                router: router_ref,
                mids,
            },
        ) {
            old.stop.notify_waiters();
        }
    }

    /// 处理 watch 卡按钮回调（取消关注 / 立即刷新）：经 oneshot **同步回新卡**——
    /// 按钮 Loading 直接变新帧 / 终态，无闪烁（复用提问卡的 callback_update_card 机制）。
    fn handle_watch_card_action(
        state: &Arc<ServerState>,
        data: &serde_json::Value,
        ack: tokio::sync::oneshot::Sender<Option<serde_json::Value>>,
    ) {
        use crate::feishu::card::{build_watch_card, callback_update_card, WatchAction};
        let Some((mid, action)) = crate::feishu::card::parse_watch_action(data) else {
            let _ = ack.send(None);
            return;
        };
        let entry = {
            let subs = state.watch.subs.lock().unwrap();
            subs.iter()
                .find(|s| s.channel == "feishu" && s.message_id == mid)
                .cloned()
        };
        let Some(entry) = entry else {
            let _ = ack.send(None); // 已退订的卡（终态按钮本应禁用）：空 ACK。
            return;
        };
        let lang = Lang::current();
        let now = now_secs();
        let snapshot = state.agents.snapshot();
        let waiting = state
            .registry
            .in_flight_agent_session_ids()
            .contains(&entry.session_id);
        let rec = find_agent_by_session(&snapshot, &entry.session_id);
        let frame = crate::watch::build_frame(entry.seq, rec, waiting);
        let ended = frame.phase == crate::watch::WatchPhase::Ended;
        match action {
            WatchAction::Unwatch => {
                let card = build_watch_card(&crate::watch::card_view(
                    &frame,
                    crate::watch::CardMode::Final(crate::watch::FinalKind::Cancelled),
                    now,
                    lang,
                    Some(&entry.session_id),
                ));
                let _ = ack.send(Some(callback_update_card(card)));
                {
                    let mut subs = state.watch.subs.lock().unwrap();
                    if let Some(s) = subs.iter_mut().find(|s| s.message_id == mid) {
                        s.rewatchable = true;
                    }
                }
                persist_watch_subs(state);
                state.watch.notify.notify_one();
            }
            WatchAction::Refresh => {
                let mode = if ended {
                    crate::watch::CardMode::Final(crate::watch::FinalKind::Ended)
                } else {
                    crate::watch::CardMode::Active
                };
                let card =
                    build_watch_card(&crate::watch::card_view(&frame, mode, now, lang, None));
                let _ = ack.send(Some(callback_update_card(card)));
                {
                    let mut subs = state.watch.subs.lock().unwrap();
                    if ended {
                        subs.retain(|s| s.message_id != mid);
                    } else if let Some(s) = subs.iter_mut().find(|s| s.message_id == mid) {
                        s.last_sig = crate::watch::signature(&frame);
                        s.last_edit_ms = now_ms();
                        s.fails = 0;
                        s.working = frame.phase == crate::watch::WatchPhase::Working;
                    }
                }
                if ended {
                    persist_watch_subs(state);
                }
                state.watch.notify.notify_one();
            }
            WatchAction::Rewatch(session_id) => {
                // 旧卡立即 ACK 为「已重新关注」禁用态。
                let card = build_watch_card(&crate::watch::card_view(
                    &frame,
                    crate::watch::CardMode::Final(crate::watch::FinalKind::Rewatched),
                    now,
                    lang,
                    None,
                ));
                let _ = ack.send(Some(callback_update_card(card)));
                // 移除旧的 rewatchable entry。
                state
                    .watch
                    .subs
                    .lock()
                    .unwrap()
                    .retain(|s| s.message_id != mid);
                persist_watch_subs(state);
                // 异步发新 watch 卡 + 激活渠道（复用 handle_watch_cmd 路径）。
                let state = Arc::clone(state);
                let sid = session_id;
                tokio::spawn(async move {
                    let config = AppConfig::load();
                    let lang = Lang::current();
                    activate_channel_on_action(&state, "feishu", &config, lang).await;
                    let snapshot = state.agents.snapshot();
                    let rec = find_agent_by_session(&snapshot, &sid);
                    let seq = rec
                        .and_then(|r| r.get("seq").and_then(|v| v.as_u64()))
                        .unwrap_or(0);
                    if seq == 0 {
                        log("watch: rewatch target session not found, skipping");
                        return;
                    }
                    handle_watch_cmd(&state, "feishu", Some(seq), &config, lang).await;
                });
            }
        }
    }

    /// watch 按钮语义（渠道无关）。
    enum WatchBtn {
        Unwatch,
        Refresh,
    }

    /// 非飞书渠道的 rewatch 统一处理：编辑旧卡为 Rewatched 终态，移除旧 entry，激活渠道，异步发新卡。
    async fn handle_rewatch(state: &Arc<ServerState>, channel_id: &str, mid: &str) {
        let entry = {
            let subs = state.watch.subs.lock().unwrap();
            subs.iter()
                .find(|s| s.channel == channel_id && s.message_id == mid && s.rewatchable)
                .cloned()
        };
        let Some(entry) = entry else {
            return;
        };
        let config = AppConfig::load();
        let Some(client) = WatchClient::for_channel(channel_id, &config).await else {
            return;
        };
        let lang = Lang::current();
        activate_channel_on_action(state, channel_id, &config, lang).await;
        let now = now_secs();
        let snapshot = state.agents.snapshot();
        let rec = find_agent_by_session(&snapshot, &entry.session_id);
        let waiting = state
            .registry
            .in_flight_agent_session_ids()
            .contains(&entry.session_id);
        let frame = crate::watch::build_frame(entry.seq, rec, waiting);
        if let Err(err) = client
            .edit(
                mid,
                &frame,
                crate::watch::CardMode::Final(crate::watch::FinalKind::Rewatched),
                now,
                lang,
                None,
            )
            .await
        {
            log(&format!("watch: rewatch ack card failed: {}", err));
        }
        {
            let mut subs = state.watch.subs.lock().unwrap();
            subs.retain(|s| s.message_id != mid);
        }
        persist_watch_subs(state);
        let state = Arc::clone(state);
        let sid = entry.session_id;
        let ch = channel_id.to_string();
        tokio::spawn(async move {
            let config = AppConfig::load();
            let lang = Lang::current();
            let snapshot = state.agents.snapshot();
            let rec = find_agent_by_session(&snapshot, &sid);
            let seq = rec
                .and_then(|r| r.get("seq").and_then(|v| v.as_u64()))
                .unwrap_or(0);
            if seq == 0 {
                log("watch: rewatch target session not found, skipping");
                return;
            }
            handle_watch_cmd(&state, &ch, Some(seq), &config, lang).await;
        });
    }

    /// 非飞书渠道的 watch 按钮统一处理：计算新帧并**就地编辑**卡片（这些渠道无「回调同步回卡」
    /// 机制，编辑即生效；飞书走 `handle_watch_card_action` 的 oneshot 回卡）。
    async fn apply_watch_action(
        state: &Arc<ServerState>,
        channel_id: &str,
        mid: &str,
        btn: WatchBtn,
    ) {
        let entry = {
            let subs = state.watch.subs.lock().unwrap();
            subs.iter()
                .find(|s| s.channel == channel_id && s.message_id == mid)
                .cloned()
        };
        let Some(entry) = entry else {
            return; // 已退订的卡（终态卡无按钮，孤儿回调已在 Router 层应答）。
        };
        let config = AppConfig::load();
        let Some(client) = WatchClient::for_channel(channel_id, &config).await else {
            return;
        };
        let lang = Lang::current();
        let now = now_secs();
        let snapshot = state.agents.snapshot();
        let waiting = state
            .registry
            .in_flight_agent_session_ids()
            .contains(&entry.session_id);
        let rec = find_agent_by_session(&snapshot, &entry.session_id);
        let frame = crate::watch::build_frame(entry.seq, rec, waiting);
        let ended = frame.phase == crate::watch::WatchPhase::Ended;
        match btn {
            WatchBtn::Unwatch => {
                if let Err(err) = client
                    .edit(
                        mid,
                        &frame,
                        crate::watch::CardMode::Final(crate::watch::FinalKind::Cancelled),
                        now,
                        lang,
                        Some(&entry.session_id),
                    )
                    .await
                {
                    log(&format!("watch: finalize cancelled card failed: {}", err));
                }
                {
                    let mut subs = state.watch.subs.lock().unwrap();
                    if let Some(s) = subs.iter_mut().find(|s| s.message_id == mid) {
                        s.rewatchable = true;
                    }
                }
                persist_watch_subs(state);
                state.watch.notify.notify_one();
            }
            WatchBtn::Refresh => {
                let mode = if ended {
                    crate::watch::CardMode::Final(crate::watch::FinalKind::Ended)
                } else {
                    crate::watch::CardMode::Active
                };
                if let Err(err) = client.edit(mid, &frame, mode, now, lang, None).await {
                    log(&format!("watch: refresh card failed: {}", err));
                    return;
                }
                {
                    let mut subs = state.watch.subs.lock().unwrap();
                    if ended {
                        subs.retain(|s| s.message_id != mid);
                    } else if let Some(s) = subs.iter_mut().find(|s| s.message_id == mid) {
                        s.last_sig = crate::watch::signature(&frame);
                        s.last_edit_ms = now_ms();
                        s.fails = 0;
                        s.working = frame.phase == crate::watch::WatchPhase::Working;
                    }
                }
                if ended {
                    persist_watch_subs(state);
                }
                state.watch.notify.notify_one();
            }
        }
    }

    /// 处理 Telegram watch 卡按钮回调：先应答（消除客户端转圈），再就地编辑。
    async fn handle_watch_tg_action(state: &Arc<ServerState>, cb: &serde_json::Value) {
        let data = cb.get("data").and_then(|v| v.as_str()).unwrap_or("");
        let Some(mid) = cb
            .get("message")
            .and_then(|m| m.get("message_id"))
            .and_then(|v| v.as_i64())
        else {
            return;
        };
        // 应答 callback（best-effort）。
        if let Some(id) = cb.get("id").and_then(|i| i.as_str()) {
            let tg = &AppConfig::load().channels.telegram;
            if let Ok(c) = crate::telegram::TelegramClient::new(
                tg.bot_token.clone(),
                tg.chat_id.clone(),
                tg.api_base_url.clone(),
            ) {
                c.answer_callback_query(id).await;
            }
        }
        if data == crate::telegram::watch::CB_REWATCH {
            handle_rewatch(state, "telegram", &mid.to_string()).await;
            return;
        }
        let btn = match data {
            crate::telegram::watch::CB_UNWATCH => WatchBtn::Unwatch,
            crate::telegram::watch::CB_REFRESH => WatchBtn::Refresh,
            _ => return,
        };
        apply_watch_action(state, "telegram", &mid.to_string(), btn).await;
    }

    /// 处理 Slack watch 卡按钮回调（ack 已在 ws 层完成，这里只做编辑）。
    async fn handle_watch_slack_action(state: &Arc<ServerState>, payload: &serde_json::Value) {
        let Some((ts, action_id)) = crate::slack::watch::parse_watch_action(payload) else {
            return;
        };
        if action_id == crate::slack::watch::ACTION_REWATCH {
            handle_rewatch(state, "slack", &ts).await;
            return;
        }
        let btn = if action_id == crate::slack::watch::ACTION_UNWATCH {
            WatchBtn::Unwatch
        } else {
            WatchBtn::Refresh
        };
        apply_watch_action(state, "slack", &ts, btn).await;
    }

    /// 处理钉钉 watch 卡按钮回调（空 ACK 已在路由任务发出，这里只做编辑）。
    async fn handle_watch_dd_action(state: &Arc<ServerState>, data: &serde_json::Value) {
        let Some((otid, action_id)) = crate::dingtalk::watch::parse_watch_action(data) else {
            return;
        };
        if action_id == crate::dingtalk::watch::ACTION_REWATCH {
            handle_rewatch(state, "dingding", &otid).await;
            return;
        }
        let btn = if action_id == crate::dingtalk::watch::ACTION_UNWATCH {
            WatchBtn::Unwatch
        } else {
            WatchBtn::Refresh
        };
        apply_watch_action(state, "dingding", &otid, btn).await;
    }

    /// watch 列表一行：`[编号] 类型 — 标题（项目）· 状态`。记录已消失按已结束显示。
    fn watch_line(snapshot: &serde_json::Value, e: &WatchEntry, lang: Lang) -> String {
        let rec = find_agent_by_session(snapshot, &e.session_id);
        let head = rec
            .map(|r| crate::autochannel::kind_title_project(r, lang))
            .unwrap_or_else(|| crate::i18n::tr(lang, "autoChannel.noTitle").to_string());
        let state_key = match rec.and_then(|r| r.get("state")).and_then(|v| v.as_str()) {
            Some("working") => "autoChannel.stateWorking",
            Some("idle") => "autoChannel.stateIdle",
            _ => "autoChannel.stateEnded",
        };
        format!(
            "[{}] {} · {}",
            e.seq,
            head,
            crate::i18n::tr(lang, state_key)
        )
    }

    /// `/watch` 命令：`Some(编号)` 关注该 agent（发实时状态卡，成功回执就是卡片本身）；
    /// `None` 列出当前关注。渠道门控见 `watch::channel_supported`（四渠道全支持）。
    async fn handle_watch_cmd(
        state: &Arc<ServerState>,
        channel_id: &str,
        sel: Option<u64>,
        config: &AppConfig,
        lang: Lang,
    ) {
        if !crate::watch::channel_supported(channel_id) {
            let _ = reply_channel_text(
                channel_id,
                config,
                crate::i18n::tr(lang, "watch.unsupported"),
            )
            .await;
            return;
        }
        let Some(id) = sel else {
            // `/watch` 无参文本回退：首行提示 + agent 列表 + 已关注段。仅工作中 agent 时才显示
            // 关注提示（空闲 agent 关注没有意义）。列表仍含全部 working + idle 便于了解全貌。
            let snapshot = state.agents.snapshot();
            let has_working = snapshot
                .as_array()
                .map(|l| {
                    l.iter()
                        .any(|r| r.get("state").and_then(|v| v.as_str()) == Some("working"))
                })
                .unwrap_or(false);
            let mut out = String::new();
            if has_working {
                out.push_str(
                    &crate::i18n::tr(lang, "watch.pickHintWorkingOnly")
                        .replace("{p}", crate::autochannel::cmd_prefix(channel_id)),
                );
                out.push_str("\n\n");
            }
            out.push_str(&crate::autochannel::status_text(&snapshot, lang));
            let entries: Vec<WatchEntry> = state
                .watch
                .subs
                .lock()
                .unwrap()
                .iter()
                .filter(|e| e.channel == channel_id && !e.rewatchable)
                .cloned()
                .collect();
            if !entries.is_empty() {
                out.push_str("\n\n");
                out.push_str(crate::i18n::tr(lang, "watch.listTitle"));
                for e in &entries {
                    out.push('\n');
                    out.push_str(&watch_line(&snapshot, e, lang));
                }
            }
            let _ = reply_channel_text(channel_id, config, &out).await;
            return;
        };
        let snapshot = state.agents.snapshot();
        let Some(rec) = snapshot.as_array().and_then(|l| {
            l.iter()
                .find(|r| r.get("seq").and_then(|v| v.as_u64()) == Some(id))
        }) else {
            let text = crate::i18n::tr(lang, "autoChannel.statusDetailNotFound")
                .replace("{id}", &id.to_string())
                .replace("{p}", crate::autochannel::cmd_prefix(channel_id));
            let _ = reply_channel_text(channel_id, config, &text).await;
            return;
        };
        let session_id = rec
            .get("sessionId")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        // 重复 watch 同一 agent＝换新卡：旧卡稍后定格「已由新卡片接替」（把卡拉到会话底部）。
        // 仅限本渠道：同一 agent 在不同渠道的关注互相独立。
        let replaced = {
            let subs = state.watch.subs.lock().unwrap();
            subs.iter()
                .find(|s| s.channel == channel_id && s.session_id == session_id)
                .cloned()
        };
        // 关注上限（每渠道各算；换新卡不算新增）。
        if replaced.is_none()
            && state
                .watch
                .subs
                .lock()
                .unwrap()
                .iter()
                .filter(|s| s.channel == channel_id && !s.rewatchable)
                .count()
                >= crate::watch::MAX_WATCHES
        {
            let text = crate::i18n::tr(lang, "watch.limit")
                .replace("{n}", &crate::watch::MAX_WATCHES.to_string())
                .replace("{p}", crate::autochannel::cmd_prefix(channel_id));
            let _ = reply_channel_text(channel_id, config, &text).await;
            return;
        }
        let Some(client) = WatchClient::for_channel(channel_id, config).await else {
            return;
        };
        let waiting = state
            .registry
            .in_flight_agent_session_ids()
            .contains(&session_id);
        let now = now_secs();
        let frame = crate::watch::build_frame(id, Some(rec), waiting);
        // 已结束 / 空闲的 agent：直接发一张定格终态卡（回顾当前状态，不订阅后续更新）。
        // Waiting（有在途 AskHuman 提问）不算空闲。
        let ended = frame.phase == crate::watch::WatchPhase::Ended;
        let idle = frame.phase == crate::watch::WatchPhase::Idle;
        let one_shot = ended || idle;
        let mode = if ended {
            crate::watch::CardMode::Final(crate::watch::FinalKind::Ended)
        } else if idle {
            crate::watch::CardMode::Final(crate::watch::FinalKind::Idle)
        } else {
            crate::watch::CardMode::Active
        };
        let message_id = match client.send(&frame, mode, now, lang).await {
            Ok(mid) => mid,
            Err(e) => {
                let text = crate::i18n::tr(lang, "watch.sendFailed").replace("{e}", &e);
                let _ = reply_channel_text(channel_id, config, &text).await;
                return;
            }
        };
        // 新卡已发成功 → 换新卡收尾（旧卡定格 Replaced + 退订）+ 登记新订阅（`register_watch_at`
        // 与「单选卡点选就地变卡」共用同一套 bookkeeping；`replaced` 仅供上面的上限判定，收尾在
        // helper 内按 session 重算）。
        register_watch_at(
            state,
            channel_id,
            &session_id,
            id,
            &message_id,
            &frame,
            one_shot,
            config,
            lang,
        )
        .await;
    }

    /// 登记一条新的 watch 订阅到 `message_id`（命令发新卡 / 单选卡点选就地变卡 两条路径共用）：
    /// 本渠道已在关注**同一 session**（且是别的消息）→ 旧卡定格 `Replaced` 并退订（换新卡语义）；
    /// 然后（非 ended 时）push 新 `WatchEntry`；持久化 + 唤醒引擎。调用方已完成「发卡 / 回卡」拿到
    /// `message_id`、并已做上限校验。
    #[allow(clippy::too_many_arguments)]
    async fn register_watch_at(
        state: &Arc<ServerState>,
        channel_id: &str,
        session_id: &str,
        seq: u64,
        message_id: &str,
        frame: &crate::watch::WatchFrame,
        ended: bool,
        config: &AppConfig,
        lang: Lang,
    ) {
        let now = now_secs();
        // 换新卡：本渠道同 session 的旧订阅（message_id 不同）定格 Replaced 并退订。
        let replaced: Option<WatchEntry> = {
            let subs = state.watch.subs.lock().unwrap();
            subs.iter()
                .find(|s| {
                    s.channel == channel_id
                        && s.session_id == session_id
                        && s.message_id != message_id
                })
                .cloned()
        };
        if let Some(old) = replaced {
            if let Some(client) = WatchClient::for_channel(channel_id, config).await {
                let snapshot = state.agents.snapshot();
                let waiting = state
                    .registry
                    .in_flight_agent_session_ids()
                    .contains(&old.session_id);
                let old_frame = crate::watch::build_frame(
                    old.seq,
                    find_agent_by_session(&snapshot, &old.session_id),
                    waiting,
                );
                if let Err(err) = client
                    .edit(
                        &old.message_id,
                        &old_frame,
                        crate::watch::CardMode::Final(crate::watch::FinalKind::Replaced),
                        now,
                        lang,
                        None,
                    )
                    .await
                {
                    log(&format!("watch: finalize replaced card failed: {}", err));
                }
            }
            state
                .watch
                .subs
                .lock()
                .unwrap()
                .retain(|s| s.message_id != old.message_id);
        }
        if !ended {
            state.watch.subs.lock().unwrap().push(WatchEntry {
                channel: channel_id.to_string(),
                session_id: session_id.to_string(),
                message_id: message_id.to_string(),
                seq,
                created_at: now,
                last_sig: crate::watch::signature(frame),
                last_edit_ms: now_ms(),
                fails: 0,
                working: frame.phase == crate::watch::WatchPhase::Working,
                sent_at_ms: now_ms(),
                // 从创建起算 30s 节流（新卡本就在底部，避免刚发就跟底重发）。
                last_move_ms: now_ms(),
                rewatchable: false,
            });
        }
        persist_watch_subs(state);
        // 引擎即醒：重算 tick 间隔 + 挂卡片回调路由（按钮立即可用）。
        state.watch.notify.notify_one();
    }

    /// `/unwatch` 命令：取消关注（编号 / 全部 / 缺省自动），旧卡定格「已取消关注」+ 回确认文本。
    async fn handle_unwatch_cmd(
        state: &Arc<ServerState>,
        channel_id: &str,
        sel: crate::autochannel::WatchSel,
        config: &AppConfig,
        lang: Lang,
    ) {
        use crate::autochannel::WatchSel;
        if !crate::watch::channel_supported(channel_id) {
            let _ = reply_channel_text(
                channel_id,
                config,
                crate::i18n::tr(lang, "watch.unsupported"),
            )
            .await;
            return;
        }
        // 只操作本渠道的活跃订阅（rewatchable 已是终态，不参与 unwatch）。
        let entries: Vec<WatchEntry> = state
            .watch
            .subs
            .lock()
            .unwrap()
            .iter()
            .filter(|e| e.channel == channel_id && !e.rewatchable)
            .cloned()
            .collect();
        let targets: Vec<WatchEntry> = match sel {
            WatchSel::One(id) => {
                let found: Vec<WatchEntry> =
                    entries.iter().filter(|e| e.seq == id).cloned().collect();
                if found.is_empty() {
                    let text = crate::i18n::tr(lang, "watch.notWatching")
                        .replace("{id}", &id.to_string())
                        .replace("{p}", crate::autochannel::cmd_prefix(channel_id));
                    let _ = reply_channel_text(channel_id, config, &text).await;
                    return;
                }
                found
            }
            WatchSel::All => {
                if entries.is_empty() {
                    let _ = reply_channel_text(
                        channel_id,
                        config,
                        crate::i18n::tr(lang, "watch.unwatchNone"),
                    )
                    .await;
                    return;
                }
                entries.clone()
            }
            WatchSel::Auto => match entries.len() {
                0 => {
                    let _ = reply_channel_text(
                        channel_id,
                        config,
                        crate::i18n::tr(lang, "watch.unwatchNone"),
                    )
                    .await;
                    return;
                }
                1 => entries.clone(),
                // 多个：回列表让用户指定编号。
                _ => {
                    let snapshot = state.agents.snapshot();
                    let mut out = crate::i18n::tr(lang, "watch.unwatchWhich")
                        .replace("{p}", crate::autochannel::cmd_prefix(channel_id));
                    for e in &entries {
                        out.push('\n');
                        out.push_str(&watch_line(&snapshot, e, lang));
                    }
                    let _ = reply_channel_text(channel_id, config, &out).await;
                    return;
                }
            },
        };
        // 旧卡定格 Cancelled + 移除订阅（复用共享收尾）→ 回确认。渠道不可用则整段跳过（订阅保留，稍后重试）。
        let dropped = finalize_and_drop_watches(
            state,
            channel_id,
            &targets,
            crate::watch::FinalKind::Cancelled,
            config,
            lang,
        )
        .await;
        if dropped == 0 {
            return; // 渠道客户端不可用：与旧行为一致（不退订、不回执）。
        }
        let text = if targets.len() == 1 {
            crate::i18n::tr(lang, "watch.unwatchDone").replace("{id}", &targets[0].seq.to_string())
        } else {
            crate::i18n::tr(lang, "watch.unwatchAllDone").replace("{n}", &targets.len().to_string())
        };
        let _ = reply_channel_text(channel_id, config, &text).await;
    }

    /// 对某渠道的一批 watch 订阅统一收尾：逐个把卡片定格为 `final_kind`。
    /// `AutoStopped` 的 entry 标记 `rewatchable`（保留路由供重新关注）而非移除；其余终态移除。
    /// 渠道客户端不可用则**整段跳过**（订阅保留、返回 0）。
    async fn finalize_and_drop_watches(
        state: &Arc<ServerState>,
        channel_id: &str,
        targets: &[WatchEntry],
        final_kind: crate::watch::FinalKind,
        config: &AppConfig,
        lang: Lang,
    ) -> usize {
        if targets.is_empty() {
            return 0;
        }
        let Some(client) = WatchClient::for_channel(channel_id, config).await else {
            return 0;
        };
        let keep_rewatchable = final_kind.is_rewatchable();
        let snapshot = state.agents.snapshot();
        let waiting = state.registry.in_flight_agent_session_ids();
        let now = now_secs();
        for e in targets {
            let rec = find_agent_by_session(&snapshot, &e.session_id);
            let frame = crate::watch::build_frame(e.seq, rec, waiting.contains(&e.session_id));
            let sid = if keep_rewatchable {
                Some(e.session_id.as_str())
            } else {
                None
            };
            if let Err(err) = client
                .edit(
                    &e.message_id,
                    &frame,
                    crate::watch::CardMode::Final(final_kind.clone()),
                    now,
                    lang,
                    sid,
                )
                .await
            {
                log(&format!(
                    "watch: finalize card failed ({}): {}",
                    channel_id, err
                ));
            }
        }
        {
            let mut subs = state.watch.subs.lock().unwrap();
            if keep_rewatchable {
                for s in subs.iter_mut() {
                    if targets.iter().any(|t| t.message_id == s.message_id) {
                        s.rewatchable = true;
                    }
                }
            } else {
                subs.retain(|s| !targets.iter().any(|t| t.message_id == s.message_id));
            }
        }
        persist_watch_subs(state);
        state.watch.notify.notify_one();
        targets.len()
    }

    // ===== 通用「单选卡」子系统（spec docs/specs/im-select-card.md）=====

    /// 登记一条单选卡台账（顺带按 TTL + 每渠道软上限清理旧卡）。
    fn register_picker(state: &Arc<ServerState>, entry: PickerEntry) {
        let now = now_secs();
        let mut pickers = state.select.pickers.lock().unwrap();
        // TTL 兜底清理（全渠道）。
        pickers.retain(|p| now.saturating_sub(p.created_at) < SELECT_PICKER_TTL_SECS);
        let channel = entry.channel.clone();
        pickers.push(entry);
        // 每渠道软上限：超出丢最旧（本渠道最靠前的条目）。
        while pickers.iter().filter(|p| p.channel == channel).count()
            > SELECT_MAX_PICKERS_PER_CHANNEL
        {
            if let Some(pos) = pickers.iter().position(|p| p.channel == channel) {
                pickers.remove(pos);
            } else {
                break;
            }
        }
    }

    /// 发一张单选卡到某渠道，返回消息 id（MVP 仅飞书；其它渠道 None → 调用方回文本兜底）。
    async fn send_select_card(
        channel_id: &str,
        config: &AppConfig,
        view: &crate::select::SelectView,
    ) -> Option<String> {
        match channel_id {
            "feishu" => {
                let client =
                    crate::feishu::client::FeishuClient::new(&config.channels.feishu).ok()?;
                let card = crate::feishu::card::build_select_card(view);
                client.send_card(&card).await.ok()
            }
            "dingding" => {
                // 钉钉：模板 + 变量。消息 id = 自铸 outTrackId（与 watch 卡同规，天然可编辑）。
                let client =
                    crate::dingtalk::client::DingTalkClient::new(&config.channels.dingding).ok()?;
                let otid = format!("select-{}", uuid::Uuid::new_v4());
                let map = crate::dingtalk::select::build_select_param_map(view, Lang::current());
                client
                    .create_and_deliver_card(
                        &otid,
                        crate::dingtalk::select::DEFAULT_SELECT_CARD_TEMPLATE_ID,
                        map,
                        serde_json::json!({}),
                    )
                    .await
                    .ok()?;
                Some(otid)
            }
            "telegram" => {
                let tg = &config.channels.telegram;
                let client = crate::telegram::TelegramClient::new(
                    tg.bot_token.clone(),
                    tg.chat_id.clone(),
                    tg.api_base_url.clone(),
                )
                .ok()?;
                let html = crate::telegram::select::render_select_html(view);
                let markup = crate::telegram::select::inline_keyboard(view, Lang::current());
                client
                    .send_message(&html, Some("HTML"), Some(markup))
                    .await
                    .ok()
                    .map(|mid| mid.to_string())
            }
            "slack" => {
                let client = crate::slack::client::SlackClient::new(&config.channels.slack).ok()?;
                let dm = client.open_dm().await.ok()?;
                let (blocks, fallback) =
                    crate::slack::select::build_select_blocks(view, Lang::current());
                client
                    .post_message(&dm, Some(&blocks), &fallback)
                    .await
                    .ok()
            }
            _ => None,
        }
    }

    /// 组装并发一张 agent 单选卡：空选项 / 非支持渠道（send 失败）→ 返回 false（调用方回文本兜底）。
    /// `payload` 仅 `PickerKind::Msg` 用（待发送内容随卡登记，点「发送」时投递）。
    async fn send_agent_picker(
        state: &Arc<ServerState>,
        channel_id: &str,
        config: &AppConfig,
        kind: PickerKind,
        title: String,
        options: Vec<crate::select::SelectOption>,
        payload: Option<String>,
        lang: Lang,
    ) -> bool {
        if options.is_empty() {
            return false;
        }
        let action = match kind {
            PickerKind::Watch => crate::select::SelectAction::Watch,
            PickerKind::Status => crate::select::SelectAction::Status,
            PickerKind::Unwatch => crate::select::SelectAction::Unwatch,
            PickerKind::Msg => crate::select::SelectAction::Msg,
            PickerKind::Diff => crate::select::SelectAction::Diff,
            PickerKind::Stage => crate::select::SelectAction::Stage,
            PickerKind::Transcript => crate::select::SelectAction::Transcript,
        };
        let view = crate::select::build_view(title, options, action, lang);
        let session_ids: Vec<String> = view.options.iter().map(|o| o.id.clone()).collect();
        let Some(mid) = send_select_card(channel_id, config, &view).await else {
            return false;
        };
        register_picker(
            state,
            PickerEntry {
                channel: channel_id.to_string(),
                message_id: mid,
                kind,
                options: session_ids,
                payload,
                created_at: now_secs(),
                posted_ms: now_ms(),
            },
        );
        ensure_select_routes(state).await;
        true
    }

    /// 本渠道已在关注的 session_id 集合（`/watch` 单选卡「· 关注中」徽标用）。
    fn watching_sessions(
        state: &Arc<ServerState>,
        channel_id: &str,
    ) -> std::collections::HashSet<String> {
        state
            .watch
            .subs
            .lock()
            .unwrap()
            .iter()
            .filter(|s| s.channel == channel_id)
            .map(|s| s.session_id.clone())
            .collect()
    }

    /// 本渠道各 watch 订阅 → 单选卡选项（`/unwatch` 单选卡）。按 session 在快照定位记录组装
    /// （圆点/类型·工作目录名/标题）；记录已消失时按 `seq` 兜底降级（见 `agent_option_by_session`）。
    fn unwatch_options(
        state: &Arc<ServerState>,
        channel_id: &str,
        snapshot: &serde_json::Value,
        lang: Lang,
    ) -> Vec<crate::select::SelectOption> {
        let now = now_secs();
        state
            .watch
            .subs
            .lock()
            .unwrap()
            .iter()
            .filter(|s| s.channel == channel_id)
            .map(|s| {
                crate::select::agent_option_by_session(snapshot, &s.session_id, s.seq, now, lang)
            })
            .collect()
    }

    /// `/status` 详情（按 session_id 定位，避免 seq 漂移）。找不到 → notFound 提示。
    fn status_detail_by_session(
        snapshot: &serde_json::Value,
        session_id: &str,
        channel_id: &str,
        lang: Lang,
    ) -> String {
        let prefix = crate::autochannel::cmd_prefix(channel_id);
        let seq = snapshot.as_array().and_then(|l| {
            l.iter()
                .find(|r| r.get("sessionId").and_then(|v| v.as_str()) == Some(session_id))
                .and_then(|r| r.get("seq").and_then(|v| v.as_u64()))
        });
        match seq {
            Some(id) => crate::autochannel::status_detail_text(snapshot, id, prefix, lang),
            None => crate::i18n::tr(lang, "autoChannel.statusDetailNotFound")
                .replace("{id}", "?")
                .replace("{p}", prefix),
        }
    }

    /// 从单选卡台账移除某条卡。移除即视为「单选完结」：清零本渠道全部 watch 订阅的跟底节流
    /// （`last_move_ms=0`）并唤醒引擎——单选期间被抑制的跟底在下一次内容变化时立即重发到会话底部
    /// （用户定案，与「提问完结」一致；此处覆盖到钉钉，补上提问路径遗漏 dingding 的口径差）。
    fn remove_picker(state: &Arc<ServerState>, channel_id: &str, message_id: &str) {
        let removed = {
            let mut pickers = state.select.pickers.lock().unwrap();
            let before = pickers.len();
            pickers.retain(|p| !(p.channel == channel_id && p.message_id == message_id));
            pickers.len() != before
        };
        if !removed {
            return;
        }
        // 本渠道若已无其它在途单选卡，放开跟底：清零节流 + 唤醒引擎。
        if !has_active_select_on(state, channel_id) {
            let mut cleared = false;
            for s in state
                .watch
                .subs
                .lock()
                .unwrap()
                .iter_mut()
                .filter(|s| s.channel == channel_id)
            {
                s.last_move_ms = 0;
                cleared = true;
            }
            if cleared {
                state.watch.notify.notify_one();
            }
        }
    }

    /// 幂等确保各渠道的单选卡回调路由任务在位（撤掉已无 picker 的渠道路由）。飞书 / 钉钉 / TG / Slack。
    /// Confirm 卡 message_id 一并纳入（与 pickers 共享路由任务）。
    async fn ensure_select_routes(state: &Arc<ServerState>) {
        let mut desired: HashMap<String, Vec<String>> = HashMap::new();
        for p in state.select.pickers.lock().unwrap().iter() {
            desired
                .entry(p.channel.clone())
                .or_default()
                .push(p.message_id.clone());
        }
        for c in state.select.confirms.lock().unwrap().iter() {
            desired
                .entry(c.channel.clone())
                .or_default()
                .push(c.message_id.clone());
        }
        for mids in desired.values_mut() {
            mids.sort();
            mids.dedup();
        }
        {
            let mut routes = state.select.routes.lock().unwrap();
            routes.retain(|ch, h| {
                if desired.contains_key(ch) {
                    true
                } else {
                    h.stop.notify_waiters();
                    false
                }
            });
        }
        let config = AppConfig::load();
        for (ch, mids) in desired {
            ensure_select_route_for(state, &config, &ch, mids).await;
        }
    }

    /// 幂等确保单一渠道的单选卡回调路由任务在位（飞书 / 钉钉 / TG / Slack；复用 watch 的路由句柄类型）。
    /// 飞书走「回调同步回卡」(oneshot Option)；钉钉先空 ACK、卡片变化经 OpenAPI；TG/Slack 就地编辑
    /// （见 `handle_select_dd_action` / `handle_select_tg_action` / `handle_select_slack_action`）。
    async fn ensure_select_route_for(
        state: &Arc<ServerState>,
        config: &AppConfig,
        channel_id: &str,
        mids: Vec<String>,
    ) {
        // 取该渠道的共享 Router（渠道不可用则跳过；picker 仍在，渠道恢复后下一拍补挂）。
        let router: WatchChannelRouter = match channel_id {
            "feishu" => {
                if !crate::app::is_feishu_active(config) {
                    return;
                }
                match ensure_fs_router(state, &config.channels.feishu).await {
                    Some(r) => WatchChannelRouter::Feishu(r),
                    None => return,
                }
            }
            "dingding" => {
                if !crate::app::is_dingding_active(config) {
                    return;
                }
                let dd = &config.channels.dingding;
                match ensure_dd_router(state, dd.client_id.trim(), dd.client_secret.trim()).await {
                    Some(r) => WatchChannelRouter::DingTalk(r),
                    None => return,
                }
            }
            "telegram" => {
                if !crate::app::is_telegram_active(config) {
                    return;
                }
                match ensure_tg_router(state, &config.channels.telegram).await {
                    Some(r) => WatchChannelRouter::Telegram(r),
                    None => return,
                }
            }
            "slack" => {
                if !crate::app::is_slack_active(config) {
                    return;
                }
                match ensure_sl_router(state, &config.channels.slack).await {
                    Some(r) => WatchChannelRouter::Slack(r),
                    None => return,
                }
            }
            _ => return,
        };
        // 现任务仍绑定同一存活 Router 且卡集合未变 → 无事可做。
        {
            let routes = state.select.routes.lock().unwrap();
            if let Some(h) = routes.get(channel_id) {
                if h.router.is_same_alive(&router) && h.mids == mids {
                    return;
                }
            }
        }
        let stop = Arc::new(tokio::sync::Notify::new());
        let stop2 = stop.clone();
        let st = state.clone();
        let router_ref: WatchRouterRef = match &router {
            WatchChannelRouter::Feishu(r) => {
                let mut routed = r.register();
                for mid in &mids {
                    routed.set_active(Some(mid), "");
                }
                tokio::spawn(async move {
                    loop {
                        tokio::select! {
                            _ = stop2.notified() => break,
                            ev = routed.recv() => match ev {
                                Some(crate::feishu::router::FsInbound::Card { data, ack }) => {
                                    handle_select_card_action(&st, "feishu", &data, ack).await;
                                }
                                Some(_) => {} // 未认领聊天消息，不会到达；防御性忽略。
                                None => break, // Router 断开：下一拍 ensure 重建。
                            },
                        }
                    }
                });
                WatchRouterRef::Feishu(Arc::downgrade(r))
            }
            WatchChannelRouter::DingTalk(r) => {
                let mut routed = r.register();
                for mid in &mids {
                    routed.set_active(Some(mid), "");
                }
                tokio::spawn(async move {
                    loop {
                        tokio::select! {
                            _ = stop2.notified() => break,
                            ev = routed.recv() => match ev {
                                Some(crate::dingtalk::router::DdInbound::Card { data, ack }) => {
                                    // 先空 ACK 满足 3 秒回包（钉钉无「回调同步回卡」，卡片变化走 OpenAPI）。
                                    let _ = ack.send(serde_json::json!({}));
                                    handle_select_dd_action(&st, &data).await;
                                }
                                Some(_) => {} // 未认领聊天消息，不会到达；防御性忽略。
                                None => break, // Router 断开：下一拍 ensure 重建。
                            },
                        }
                    }
                });
                WatchRouterRef::DingTalk(Arc::downgrade(r))
            }
            WatchChannelRouter::Telegram(r) => {
                let routed = r.register();
                for mid in &mids {
                    if let Ok(m) = mid.parse::<i64>() {
                        // 仅认领卡片回调（`set_card_route`），不认领自由文字（不抢提问卡答案）。
                        routed.set_card_route(m);
                    }
                }
                let mut routed = routed;
                tokio::spawn(async move {
                    loop {
                        tokio::select! {
                            _ = stop2.notified() => break,
                            ev = routed.recv() => match ev {
                                Some(crate::telegram::router::TgInbound::Callback(cb)) => {
                                    handle_select_tg_action(&st, &cb).await;
                                }
                                Some(_) => {} // 未认领自由文字，不会到达；防御性忽略。
                                None => break,
                            },
                        }
                    }
                });
                WatchRouterRef::Telegram(Arc::downgrade(r))
            }
            WatchChannelRouter::Slack(r) => {
                let mut routed = r.register();
                for mid in &mids {
                    // user_id 传空 → 只认领卡片交互（message_ts），不认领聊天消息。
                    routed.set_active(Some(mid), "");
                }
                tokio::spawn(async move {
                    loop {
                        tokio::select! {
                            _ = stop2.notified() => break,
                            ev = routed.recv() => match ev {
                                Some(crate::slack::router::SlInbound::Interactive(payload)) => {
                                    handle_select_slack_action(&st, &payload).await;
                                }
                                Some(_) => {}
                                None => break,
                            },
                        }
                    }
                });
                WatchRouterRef::Slack(Arc::downgrade(r))
            }
        };
        if let Some(old) = state.select.routes.lock().unwrap().insert(
            channel_id.to_string(),
            WatchRouteHandle {
                stop,
                router: router_ref,
                mids,
            },
        ) {
            old.stop.notify_waiters();
        }
    }

    /// 处理飞书单选卡 / 确认卡点击。
    /// 过期 / 越界 / 无台账 → 空 ACK（静默，D7）。
    async fn handle_select_card_action(
        state: &Arc<ServerState>,
        channel_id: &str,
        data: &serde_json::Value,
        ack: tokio::sync::oneshot::Sender<Option<serde_json::Value>>,
    ) {
        // Stage 双按钮确认卡。
        if let Some((mid, slot)) = crate::feishu::card::parse_confirm_action(data) {
            handle_confirm_action(state, channel_id, &mid, slot, Some(ack)).await;
            return;
        }
        let Some((mid, idx)) = crate::feishu::card::parse_select_action(data) else {
            let _ = ack.send(None);
            return;
        };
        let picker = {
            let pickers = state.select.pickers.lock().unwrap();
            pickers
                .iter()
                .find(|p| p.channel == channel_id && p.message_id == mid)
                .cloned()
        };
        let Some(picker) = picker else {
            let _ = ack.send(None); // 已过期 / 被清理：静默（D7）。
            return;
        };
        let Some(session_id) = picker.options.get(idx).cloned() else {
            let _ = ack.send(None);
            return;
        };
        let lang = Lang::current();
        let config = AppConfig::load();
        match picker.kind {
            PickerKind::Watch => {
                // 先完成就地变身（含卡片 ACK），再激活——避免激活的补推/回执拖慢同步 ACK。
                select_pick_watch(state, channel_id, &mid, &session_id, &config, lang, ack).await;
                // 卡片点『关注』＝在该渠道操作 → 设为活跃槽（与 /watch 一致，用户决策）。
                activate_channel_on_action(state, channel_id, &config, lang).await;
            }
            PickerKind::Status => {
                // 单选卡不动：先空 ACK，再回纯文本详情（可继续点其它 agent）。
                let _ = ack.send(None);
                let snapshot = state.agents.snapshot();
                let text = status_detail_by_session(&snapshot, &session_id, channel_id, lang);
                let _ = reply_channel_text(channel_id, &config, &text).await;
                // 卡片点『查看』＝在该渠道操作 → 设为活跃槽（补齐与 /status 文本命令的一致性）。
                activate_channel_on_action(state, channel_id, &config, lang).await;
            }
            PickerKind::Unwatch => {
                select_pick_unwatch(state, channel_id, &mid, &session_id, &config, lang, ack).await;
            }
            PickerKind::Msg => {
                let content = picker.payload.clone().unwrap_or_default();
                select_pick_msg(state, channel_id, &mid, &session_id, &content, lang, ack).await;
                // 卡片点『发送』＝在该渠道操作 → 设为活跃槽（与 /msg 一致，用户决策）。
                activate_channel_on_action(state, channel_id, &config, lang).await;
            }
            PickerKind::Diff | PickerKind::Stage | PickerKind::Transcript => {
                select_pick_export(
                    state,
                    channel_id,
                    &mid,
                    &session_id,
                    picker.kind,
                    &config,
                    lang,
                    Some(ack),
                )
                .await;
                activate_channel_on_action(state, channel_id, &config, lang).await;
            }
        }
    }

    /// 单选卡点选「发送」（飞书就地定格）：校验目标工作中·非 grok → 投递 + 定格「已发送给 [编号]」；
    /// 目标已漂移（不在工作中 / 已结束 / 消失）→ 定格「已不在工作中，未发送」。定格文案随卡回（ack）。
    #[allow(clippy::too_many_arguments)]
    async fn select_pick_msg(
        state: &Arc<ServerState>,
        channel_id: &str,
        mid: &str,
        session_id: &str,
        content: &str,
        lang: Lang,
        ack: tokio::sync::oneshot::Sender<Option<serde_json::Value>>,
    ) {
        let snapshot = state.agents.snapshot();
        let rec = find_agent_by_session(&snapshot, session_id);
        let label = msg_pick_deliver(state, channel_id, session_id, rec, content, lang);
        let card =
            crate::feishu::card::build_select_final_card(&crate::select::title_msg(lang), &label);
        let _ = ack.send(Some(crate::feishu::card::callback_update_card(card)));
        remove_picker(state, channel_id, mid);
    }

    /// 单选卡「发送」的共享收尾：目标仍工作中·非 grok → 投递并返回「已发送给 [编号] · 回执」定格文案；
    /// 否则返回「已不在工作中，未发送」。渲染层各渠道自行把该文案落进定格卡。
    fn msg_pick_deliver(
        state: &Arc<ServerState>,
        channel_id: &str,
        session_id: &str,
        rec: Option<&serde_json::Value>,
        content: &str,
        lang: Lang,
    ) -> String {
        let ok = rec
            .map(|r| {
                r.get("state").and_then(|v| v.as_str()) == Some("working")
                    && r.get("kind").and_then(|v| v.as_str()) != Some("grok")
            })
            .unwrap_or(false);
        if !ok {
            return crate::i18n::tr(lang, "select.msgTargetGone").to_string();
        }
        let seq = rec
            .and_then(|r| r.get("seq").and_then(|v| v.as_u64()))
            .unwrap_or(0);
        let note = deliver_msg(state, channel_id, session_id, content, lang);
        crate::i18n::tr(lang, "select.msgSentCard")
            .replace("{id}", &seq.to_string())
            .replace("{note}", &note)
    }

    /// 单选卡点选「watch」：就地把这张卡编辑成实时 watch 卡（经 oneshot 同步回卡）。
    #[allow(clippy::too_many_arguments)]
    async fn select_pick_watch(
        state: &Arc<ServerState>,
        channel_id: &str,
        mid: &str,
        session_id: &str,
        config: &AppConfig,
        lang: Lang,
        ack: tokio::sync::oneshot::Sender<Option<serde_json::Value>>,
    ) {
        let now = now_secs();
        let snapshot = state.agents.snapshot();
        let rec = find_agent_by_session(&snapshot, session_id);
        let waiting = state
            .registry
            .in_flight_agent_session_ids()
            .contains(&session_id.to_string());
        let seq = rec
            .and_then(|r| r.get("seq").and_then(|v| v.as_u64()))
            .unwrap_or(0);
        let frame = crate::watch::build_frame(seq, rec, waiting);
        let ended = frame.phase == crate::watch::WatchPhase::Ended;
        if ended {
            // 已结束/消失：就地定格终态卡、不订阅、消费掉 picker。
            let card = crate::feishu::card::build_watch_card(&crate::watch::card_view(
                &frame,
                crate::watch::CardMode::Final(crate::watch::FinalKind::Ended),
                now,
                lang,
                None,
            ));
            let _ = ack.send(Some(crate::feishu::card::callback_update_card(card)));
            remove_picker(state, channel_id, mid);
            // 不在此重挂 select 路由（避免 recv-loop 递归 → 非 Send）：残留的 mid 认领无害（卡已定格无按钮），
            // 下次 send_agent_picker / 监听重建时统一收敛。
            return;
        }
        // 上限校验（本渠道；已在关注同一 session＝换新卡，不计新增）。
        let already = state
            .watch
            .subs
            .lock()
            .unwrap()
            .iter()
            .any(|s| s.channel == channel_id && s.session_id == session_id);
        if !already {
            let count = state
                .watch
                .subs
                .lock()
                .unwrap()
                .iter()
                .filter(|s| s.channel == channel_id && !s.rewatchable)
                .count();
            if count >= crate::watch::MAX_WATCHES {
                let _ = ack.send(None);
                let text = crate::i18n::tr(lang, "watch.limit")
                    .replace("{n}", &crate::watch::MAX_WATCHES.to_string())
                    .replace("{p}", crate::autochannel::cmd_prefix(channel_id));
                let _ = reply_channel_text(channel_id, config, &text).await;
                return;
            }
        }
        // 就地回一张实时 watch 卡（这条单选卡消息随即变成 watch 卡）。
        let card = crate::feishu::card::build_watch_card(&crate::watch::card_view(
            &frame,
            crate::watch::CardMode::Active,
            now,
            lang,
            None,
        ));
        let _ = ack.send(Some(crate::feishu::card::callback_update_card(card)));
        // 登记订阅（含换新卡收尾）+ 消费 picker + 让 watch 立即认领本消息（`ensure_watch_routes` 不会递归
        // 回 select）。select 侧不在此重挂（避免 recv-loop 递归 → 非 Send）：本 mid 已被 watch 认领覆盖，
        // 残留的 select 认领无害，下次 send_agent_picker / 监听重建时收敛。
        register_watch_at(
            state, channel_id, session_id, seq, mid, &frame, false, config, lang,
        )
        .await;
        remove_picker(state, channel_id, mid);
        ensure_watch_routes(state).await;
    }

    /// 单选卡点选「unwatch」：取消该关注（旧卡定格）+ 回文本确认 + 就地刷新单选卡（移除该项）。
    #[allow(clippy::too_many_arguments)]
    async fn select_pick_unwatch(
        state: &Arc<ServerState>,
        channel_id: &str,
        mid: &str,
        session_id: &str,
        config: &AppConfig,
        lang: Lang,
        ack: tokio::sync::oneshot::Sender<Option<serde_json::Value>>,
    ) {
        let now = now_secs();
        // 找到该 session 在本渠道的订阅（可能已被别处取消/结束 → 视为已不在关注，只刷新卡）。
        let entry = state
            .watch
            .subs
            .lock()
            .unwrap()
            .iter()
            .find(|s| s.channel == channel_id && s.session_id == session_id)
            .cloned();
        if let Some(entry) = entry {
            // 旧 watch 卡定格「已取消关注」（可重新关注）。
            if let Some(client) = WatchClient::for_channel(channel_id, config).await {
                let snapshot = state.agents.snapshot();
                let waiting = state
                    .registry
                    .in_flight_agent_session_ids()
                    .contains(&entry.session_id);
                let frame = crate::watch::build_frame(
                    entry.seq,
                    find_agent_by_session(&snapshot, &entry.session_id),
                    waiting,
                );
                if let Err(err) = client
                    .edit(
                        &entry.message_id,
                        &frame,
                        crate::watch::CardMode::Final(crate::watch::FinalKind::Cancelled),
                        now,
                        lang,
                        Some(&entry.session_id),
                    )
                    .await
                {
                    log(&format!("select: finalize unwatch card failed: {}", err));
                }
            }
            {
                let mut subs = state.watch.subs.lock().unwrap();
                if let Some(s) = subs.iter_mut().find(|s| s.message_id == entry.message_id) {
                    s.rewatchable = true;
                }
            }
            persist_watch_subs(state);
            state.watch.notify.notify_one();
            ensure_watch_routes(state).await;
            let text =
                crate::i18n::tr(lang, "watch.unwatchDone").replace("{id}", &entry.seq.to_string());
            let _ = reply_channel_text(channel_id, config, &text).await;
        }
        // 就地刷新单选卡：剩余订阅 → 新卡；空 → 定格「已全部取消关注」并消费 picker。
        let snapshot = state.agents.snapshot();
        let options = unwatch_options(state, channel_id, &snapshot, lang);
        if options.is_empty() {
            let card = crate::feishu::card::build_select_final_card(
                &crate::select::title_unwatch(lang),
                crate::i18n::tr(lang, "select.unwatchAllDoneCard"),
            );
            let _ = ack.send(Some(crate::feishu::card::callback_update_card(card)));
            remove_picker(state, channel_id, mid);
            // 不在此重挂 select 路由（同 select_pick_watch 理由）：卡已定格无按钮，残留认领无害。
        } else {
            let view = crate::select::build_view(
                crate::select::title_unwatch(lang),
                options,
                crate::select::SelectAction::Unwatch,
                lang,
            );
            let new_ids: Vec<String> = view.options.iter().map(|o| o.id.clone()).collect();
            let card = crate::feishu::card::build_select_card(&view);
            let _ = ack.send(Some(crate::feishu::card::callback_update_card(card)));
            // 更新 picker 的选项快照（下标对齐新卡）。
            if let Some(p) = state
                .select
                .pickers
                .lock()
                .unwrap()
                .iter_mut()
                .find(|p| p.channel == channel_id && p.message_id == mid)
            {
                p.options = new_ids;
            }
        }
    }

    // ===== 钉钉单选卡点选（无「回调同步回卡」：空 ACK 已在路由任务发出，卡片变化走 OpenAPI）=====

    /// 处理钉钉单选卡点击：解析 `(outTrackId, sid)` → 找 picker → 按 kind 分派。
    /// 过期 / sid 空 / 不属于本卡 → 静默（D7；空 ACK 已由路由任务发出）。
    async fn handle_select_dd_action(state: &Arc<ServerState>, data: &serde_json::Value) {
        // Stage 确认卡（提问模板）：提交后按选项 0=暂存 / 1=取消。
        if handle_stage_dd_submit(state, data).await {
            return;
        }
        let Some((otid, session_id)) = crate::dingtalk::select::parse_select_action(data) else {
            return;
        };
        let picker = {
            let pickers = state.select.pickers.lock().unwrap();
            pickers
                .iter()
                .find(|p| p.channel == "dingding" && p.message_id == otid)
                .cloned()
        };
        let Some(picker) = picker else {
            return; // 已过期 / 被清理：静默（D7）。
        };
        // 路由靠 param 回传的 session_id（不用会漂移的编号）；空 / 不属于本卡 → 无效（模板未绑定或已变）。
        if session_id.is_empty() || !picker.options.contains(&session_id) {
            return;
        }
        let lang = Lang::current();
        let config = AppConfig::load();
        match picker.kind {
            PickerKind::Watch => {
                dd_select_pick_watch(state, &otid, &session_id, &config, lang).await;
                // 卡片点『关注』＝在该渠道操作 → 设为活跃槽（与 /watch 一致，用户决策）。
                activate_channel_on_action(state, "dingding", &config, lang).await;
            }
            PickerKind::Status => {
                // 单选卡不动：回纯文本详情（可继续点其它 agent）。
                let snapshot = state.agents.snapshot();
                let text = status_detail_by_session(&snapshot, &session_id, "dingding", lang);
                let _ = reply_channel_text("dingding", &config, &text).await;
                // 卡片点『查看』＝在该渠道操作 → 设为活跃槽（补齐与 /status 文本命令的一致性）。
                activate_channel_on_action(state, "dingding", &config, lang).await;
            }
            PickerKind::Unwatch => {
                dd_select_pick_unwatch(state, &otid, &session_id, &config, lang).await;
            }
            PickerKind::Msg => {
                let content = picker.payload.clone().unwrap_or_default();
                dd_select_pick_msg(state, &otid, &session_id, &content, &config, lang).await;
                // 卡片点『发送』＝在该渠道操作 → 设为活跃槽（与 /msg 一致，用户决策）。
                activate_channel_on_action(state, "dingding", &config, lang).await;
            }
            PickerKind::Diff | PickerKind::Stage | PickerKind::Transcript => {
                select_pick_export(
                    state,
                    "dingding",
                    &otid,
                    &session_id,
                    picker.kind,
                    &config,
                    lang,
                    None,
                )
                .await;
                activate_channel_on_action(state, "dingding", &config, lang).await;
            }
        }
    }

    /// 钉钉单选卡点选「发送」：投递（若目标仍工作中·非 grok）+ 单选卡定格（OpenAPI 更新）。
    async fn dd_select_pick_msg(
        state: &Arc<ServerState>,
        otid: &str,
        session_id: &str,
        content: &str,
        config: &AppConfig,
        lang: Lang,
    ) {
        let snapshot = state.agents.snapshot();
        let rec = find_agent_by_session(&snapshot, session_id);
        let label = msg_pick_deliver(state, "dingding", session_id, rec, content, lang);
        dd_finalize_select_card(config, otid, &label).await;
        remove_picker(state, "dingding", otid);
    }

    /// 钉钉单选卡点选「watch」：钉钉不能就地变身（模板固定），故**另发一张新的实时 watch 卡** +
    /// 把单选卡定格「已选择 [n]」。已在关注同一 session ＝换新卡（`register_watch_at` 定格旧卡）。
    async fn dd_select_pick_watch(
        state: &Arc<ServerState>,
        otid: &str,
        session_id: &str,
        config: &AppConfig,
        lang: Lang,
    ) {
        let now = now_secs();
        let snapshot = state.agents.snapshot();
        let rec = find_agent_by_session(&snapshot, session_id);
        let waiting = state
            .registry
            .in_flight_agent_session_ids()
            .contains(&session_id.to_string());
        let seq = rec
            .and_then(|r| r.get("seq").and_then(|v| v.as_u64()))
            .unwrap_or(0);
        let frame = crate::watch::build_frame(seq, rec, waiting);
        let ended = frame.phase == crate::watch::WatchPhase::Ended;
        // 上限校验（本渠道；已在关注同一 session＝换新卡，不计新增；已结束不订阅、不计数）。
        let already = state
            .watch
            .subs
            .lock()
            .unwrap()
            .iter()
            .any(|s| s.channel == "dingding" && s.session_id == session_id);
        if !ended && !already {
            let count = state
                .watch
                .subs
                .lock()
                .unwrap()
                .iter()
                .filter(|s| s.channel == "dingding" && !s.rewatchable)
                .count();
            if count >= crate::watch::MAX_WATCHES {
                let text = crate::i18n::tr(lang, "watch.limit")
                    .replace("{n}", &crate::watch::MAX_WATCHES.to_string())
                    .replace("{p}", crate::autochannel::cmd_prefix("dingding"));
                let _ = reply_channel_text("dingding", config, &text).await;
                return; // 单选卡不动，可另选。
            }
        }
        // 另发一张实时 watch 卡（活动态活卡 / 已结束则终态卡）。
        let Some(client) = WatchClient::for_channel("dingding", config).await else {
            return;
        };
        let mode = if ended {
            crate::watch::CardMode::Final(crate::watch::FinalKind::Ended)
        } else {
            crate::watch::CardMode::Active
        };
        let new_mid = match client.send(&frame, mode, now, lang).await {
            Ok(m) => m,
            Err(err) => {
                log(&format!("select: send dingtalk watch card failed: {}", err));
                return;
            }
        };
        // 登记订阅（含换新卡：本渠道同 session 旧卡定格 Replaced）+ 让 watch 引擎认领新卡按钮。
        register_watch_at(
            state, "dingding", session_id, seq, &new_mid, &frame, ended, config, lang,
        )
        .await;
        ensure_watch_routes(state).await;
        // 单选卡定格「已选择 [n]」并消费 picker。
        let label = crate::i18n::tr(lang, "select.pickedCard").replace("{id}", &seq.to_string());
        dd_finalize_select_card(config, otid, &label).await;
        remove_picker(state, "dingding", otid);
    }

    /// 钉钉单选卡点选「unwatch」：取消该关注（旧 watch 卡定格）+ 回文本确认 + 就地刷新单选卡
    /// （经 OpenAPI 更新 loop；取到 0 则定格「已全部取消关注」）。
    async fn dd_select_pick_unwatch(
        state: &Arc<ServerState>,
        otid: &str,
        session_id: &str,
        config: &AppConfig,
        lang: Lang,
    ) {
        let now = now_secs();
        let entry = state
            .watch
            .subs
            .lock()
            .unwrap()
            .iter()
            .find(|s| s.channel == "dingding" && s.session_id == session_id)
            .cloned();
        if let Some(entry) = entry {
            if let Some(client) = WatchClient::for_channel("dingding", config).await {
                let snapshot = state.agents.snapshot();
                let waiting = state
                    .registry
                    .in_flight_agent_session_ids()
                    .contains(&entry.session_id);
                let frame = crate::watch::build_frame(
                    entry.seq,
                    find_agent_by_session(&snapshot, &entry.session_id),
                    waiting,
                );
                if let Err(err) = client
                    .edit(
                        &entry.message_id,
                        &frame,
                        crate::watch::CardMode::Final(crate::watch::FinalKind::Cancelled),
                        now,
                        lang,
                        Some(&entry.session_id),
                    )
                    .await
                {
                    log(&format!(
                        "select: finalize dingtalk unwatch card failed: {}",
                        err
                    ));
                }
            }
            {
                let mut subs = state.watch.subs.lock().unwrap();
                if let Some(s) = subs.iter_mut().find(|s| s.message_id == entry.message_id) {
                    s.rewatchable = true;
                }
            }
            persist_watch_subs(state);
            state.watch.notify.notify_one();
            ensure_watch_routes(state).await;
            let text =
                crate::i18n::tr(lang, "watch.unwatchDone").replace("{id}", &entry.seq.to_string());
            let _ = reply_channel_text("dingding", config, &text).await;
        }
        // 就地刷新单选卡：剩余订阅 → 更新 loop；空 → 定格「已全部取消关注」并消费 picker。
        let snapshot = state.agents.snapshot();
        let options = unwatch_options(state, "dingding", &snapshot, lang);
        if options.is_empty() {
            dd_finalize_select_card(
                config,
                otid,
                crate::i18n::tr(lang, "select.unwatchAllDoneCard"),
            )
            .await;
            remove_picker(state, "dingding", otid);
        } else {
            let view = crate::select::build_view(
                crate::select::title_unwatch(lang),
                options,
                crate::select::SelectAction::Unwatch,
                lang,
            );
            let new_ids: Vec<String> = view.options.iter().map(|o| o.id.clone()).collect();
            if let Ok(client) =
                crate::dingtalk::client::DingTalkClient::new(&config.channels.dingding)
            {
                let map = crate::dingtalk::select::build_select_param_map(&view, lang);
                if let Err(err) = client
                    .update_card_private(otid, map, serde_json::json!({}))
                    .await
                {
                    log(&format!(
                        "select: refresh dingtalk unwatch card failed: {}",
                        err
                    ));
                }
            }
            if let Some(p) = state
                .select
                .pickers
                .lock()
                .unwrap()
                .iter_mut()
                .find(|p| p.channel == "dingding" && p.message_id == otid)
            {
                p.options = new_ids;
            }
        }
    }

    /// 定格一张钉钉单选卡（按 key 更新公有 `finalized=true` + `final_label`）：隐藏循环、显示定格文案。
    async fn dd_finalize_select_card(config: &AppConfig, otid: &str, final_label: &str) {
        if let Ok(client) = crate::dingtalk::client::DingTalkClient::new(&config.channels.dingding)
        {
            let map = crate::dingtalk::select::build_select_final_param_map(final_label);
            if let Err(err) = client
                .update_card_private(otid, map, serde_json::json!({}))
                .await
            {
                log(&format!(
                    "select: finalize dingtalk select card failed: {}",
                    err
                ));
            }
        }
    }

    // ===== Telegram / Slack 单选卡点选（可就地编辑：点 watch → 本消息变身为实时 watch 卡）=====

    /// 处理 Telegram 单选卡 / 确认卡点击：应答消除转圈 → 解析 → 分派。
    async fn handle_select_tg_action(state: &Arc<ServerState>, cb: &serde_json::Value) {
        let data = cb.get("data").and_then(|v| v.as_str()).unwrap_or("");
        let Some(mid) = cb
            .get("message")
            .and_then(|m| m.get("message_id"))
            .and_then(|v| v.as_i64())
        else {
            return;
        };
        let config = AppConfig::load();
        // 应答 callback（消除客户端转圈，best-effort）。
        if let Some(id) = cb.get("id").and_then(|i| i.as_str()) {
            let tg = &config.channels.telegram;
            if let Ok(c) = crate::telegram::TelegramClient::new(
                tg.bot_token.clone(),
                tg.chat_id.clone(),
                tg.api_base_url.clone(),
            ) {
                c.answer_callback_query(id).await;
            }
        }
        if let Some(slot) = crate::telegram::confirm::parse_confirm_action(data) {
            handle_confirm_action(state, "telegram", &mid.to_string(), slot, None).await;
            return;
        }
        let Some(idx) = crate::telegram::select::parse_select_action(data) else {
            return;
        };
        dispatch_select_pick(state, "telegram", &mid.to_string(), idx, &config).await;
    }

    /// 处理 Slack 单选卡 / 确认卡点击（ack 已在 ws 层完成）。
    async fn handle_select_slack_action(state: &Arc<ServerState>, payload: &serde_json::Value) {
        let config = AppConfig::load();
        if let Some((ts, slot)) = crate::slack::confirm::parse_confirm_action(payload) {
            handle_confirm_action(state, "slack", &ts, slot, None).await;
            return;
        }
        let Some((ts, idx)) = crate::slack::select::parse_select_action(payload) else {
            return;
        };
        dispatch_select_pick(state, "slack", &ts, idx, &config).await;
    }

    /// TG/Slack 共用的下标分派：找 picker → 按下标取 session_id → 按 kind 处理。
    /// 过期 / 越界 / 无 picker → 静默（D7）。
    async fn dispatch_select_pick(
        state: &Arc<ServerState>,
        channel_id: &str,
        mid: &str,
        idx: usize,
        config: &AppConfig,
    ) {
        let picker = {
            let pickers = state.select.pickers.lock().unwrap();
            pickers
                .iter()
                .find(|p| p.channel == channel_id && p.message_id == mid)
                .cloned()
        };
        let Some(picker) = picker else {
            return; // 已过期 / 被清理：静默（D7）。
        };
        let Some(session_id) = picker.options.get(idx).cloned() else {
            return;
        };
        let lang = Lang::current();
        match picker.kind {
            PickerKind::Watch => {
                select_pick_watch_inplace(state, channel_id, mid, &session_id, config, lang).await;
                // 卡片点『关注』＝在该渠道操作 → 设为活跃槽（与 /watch 一致，用户决策）。
                activate_channel_on_action(state, channel_id, config, lang).await;
            }
            PickerKind::Status => {
                let snapshot = state.agents.snapshot();
                let text = status_detail_by_session(&snapshot, &session_id, channel_id, lang);
                let _ = reply_channel_text(channel_id, config, &text).await;
                // 卡片点『查看』＝在该渠道操作 → 设为活跃槽（补齐与 /status 文本命令的一致性）。
                activate_channel_on_action(state, channel_id, config, lang).await;
            }
            PickerKind::Unwatch => {
                select_pick_unwatch_inplace(state, channel_id, mid, &session_id, config, lang)
                    .await;
            }
            PickerKind::Msg => {
                let content = picker.payload.clone().unwrap_or_default();
                select_pick_msg_inplace(
                    state,
                    channel_id,
                    mid,
                    &session_id,
                    &content,
                    config,
                    lang,
                )
                .await;
                // 卡片点『发送』＝在该渠道操作 → 设为活跃槽（与 /msg 一致，用户决策）。
                activate_channel_on_action(state, channel_id, config, lang).await;
            }
            PickerKind::Diff | PickerKind::Stage | PickerKind::Transcript => {
                select_pick_export(
                    state,
                    channel_id,
                    mid,
                    &session_id,
                    picker.kind,
                    config,
                    lang,
                    None,
                )
                .await;
                activate_channel_on_action(state, channel_id, config, lang).await;
            }
        }
    }

    /// 单选卡点选「发送」（TG/Slack 就地定格）：投递（若目标仍工作中·非 grok）+ 定格本单选卡。
    async fn select_pick_msg_inplace(
        state: &Arc<ServerState>,
        channel_id: &str,
        mid: &str,
        session_id: &str,
        content: &str,
        config: &AppConfig,
        lang: Lang,
    ) {
        let snapshot = state.agents.snapshot();
        let rec = find_agent_by_session(&snapshot, session_id);
        let label = msg_pick_deliver(state, channel_id, session_id, rec, content, lang);
        finalize_select_card_edit(
            channel_id,
            config,
            mid,
            &crate::select::title_msg(lang),
            &label,
        )
        .await;
        remove_picker(state, channel_id, mid);
    }

    /// 单选卡点选「watch」（TG/Slack 可就地编辑）：把本消息编辑成实时 watch 卡（`WatchClient::edit`），
    /// 登记订阅（含换新卡收尾）+ 消费 picker + 让 watch 引擎认领本消息。已结束则定格终态卡、不订阅。
    async fn select_pick_watch_inplace(
        state: &Arc<ServerState>,
        channel_id: &str,
        mid: &str,
        session_id: &str,
        config: &AppConfig,
        lang: Lang,
    ) {
        let now = now_secs();
        let snapshot = state.agents.snapshot();
        let rec = find_agent_by_session(&snapshot, session_id);
        let waiting = state
            .registry
            .in_flight_agent_session_ids()
            .contains(&session_id.to_string());
        let seq = rec
            .and_then(|r| r.get("seq").and_then(|v| v.as_u64()))
            .unwrap_or(0);
        let frame = crate::watch::build_frame(seq, rec, waiting);
        let ended = frame.phase == crate::watch::WatchPhase::Ended;
        if ended {
            // 已结束/消失：就地把本消息编辑成终态卡、不订阅、消费掉 picker。
            if let Some(client) = WatchClient::for_channel(channel_id, config).await {
                if let Err(err) = client
                    .edit(
                        mid,
                        &frame,
                        crate::watch::CardMode::Final(crate::watch::FinalKind::Ended),
                        now,
                        lang,
                        None,
                    )
                    .await
                {
                    log(&format!(
                        "select: transform to ended watch card failed ({}): {}",
                        channel_id, err
                    ));
                    return;
                }
            }
            remove_picker(state, channel_id, mid);
            return;
        }
        // 上限校验（本渠道；已在关注同一 session＝换新卡，不计新增）。
        let already = state
            .watch
            .subs
            .lock()
            .unwrap()
            .iter()
            .any(|s| s.channel == channel_id && s.session_id == session_id);
        if !already {
            let count = state
                .watch
                .subs
                .lock()
                .unwrap()
                .iter()
                .filter(|s| s.channel == channel_id && !s.rewatchable)
                .count();
            if count >= crate::watch::MAX_WATCHES {
                let text = crate::i18n::tr(lang, "watch.limit")
                    .replace("{n}", &crate::watch::MAX_WATCHES.to_string())
                    .replace("{p}", crate::autochannel::cmd_prefix(channel_id));
                let _ = reply_channel_text(channel_id, config, &text).await;
                return; // 单选卡不动，可另选。
            }
        }
        // 就地把这条单选卡消息编辑成实时 watch 卡。
        let Some(client) = WatchClient::for_channel(channel_id, config).await else {
            return;
        };
        if let Err(err) = client
            .edit(mid, &frame, crate::watch::CardMode::Active, now, lang, None)
            .await
        {
            log(&format!(
                "select: transform select card to watch card failed ({}): {}",
                channel_id, err
            ));
            return;
        }
        register_watch_at(
            state, channel_id, session_id, seq, mid, &frame, false, config, lang,
        )
        .await;
        remove_picker(state, channel_id, mid);
        ensure_watch_routes(state).await;
    }

    /// 单选卡点选「unwatch」（TG/Slack）：取消该关注（旧 watch 卡定格）+ 文本确认 + 就地刷新本单选卡
    /// （移除该项；取到 0 则定格「已全部取消关注」）。
    async fn select_pick_unwatch_inplace(
        state: &Arc<ServerState>,
        channel_id: &str,
        mid: &str,
        session_id: &str,
        config: &AppConfig,
        lang: Lang,
    ) {
        let now = now_secs();
        let entry = state
            .watch
            .subs
            .lock()
            .unwrap()
            .iter()
            .find(|s| s.channel == channel_id && s.session_id == session_id)
            .cloned();
        if let Some(entry) = entry {
            if let Some(client) = WatchClient::for_channel(channel_id, config).await {
                let snapshot = state.agents.snapshot();
                let waiting = state
                    .registry
                    .in_flight_agent_session_ids()
                    .contains(&entry.session_id);
                let frame = crate::watch::build_frame(
                    entry.seq,
                    find_agent_by_session(&snapshot, &entry.session_id),
                    waiting,
                );
                if let Err(err) = client
                    .edit(
                        &entry.message_id,
                        &frame,
                        crate::watch::CardMode::Final(crate::watch::FinalKind::Cancelled),
                        now,
                        lang,
                        Some(&entry.session_id),
                    )
                    .await
                {
                    log(&format!(
                        "select: finalize unwatch card failed ({}): {}",
                        channel_id, err
                    ));
                }
            }
            {
                let mut subs = state.watch.subs.lock().unwrap();
                if let Some(s) = subs.iter_mut().find(|s| s.message_id == entry.message_id) {
                    s.rewatchable = true;
                }
            }
            persist_watch_subs(state);
            state.watch.notify.notify_one();
            ensure_watch_routes(state).await;
            let text =
                crate::i18n::tr(lang, "watch.unwatchDone").replace("{id}", &entry.seq.to_string());
            let _ = reply_channel_text(channel_id, config, &text).await;
        }
        // 就地刷新单选卡：剩余订阅 → 新卡；空 → 定格「已全部取消关注」并消费 picker。
        let snapshot = state.agents.snapshot();
        let options = unwatch_options(state, channel_id, &snapshot, lang);
        if options.is_empty() {
            finalize_select_card_edit(
                channel_id,
                config,
                mid,
                &crate::select::title_unwatch(lang),
                crate::i18n::tr(lang, "select.unwatchAllDoneCard"),
            )
            .await;
            remove_picker(state, channel_id, mid);
        } else {
            let view = crate::select::build_view(
                crate::select::title_unwatch(lang),
                options,
                crate::select::SelectAction::Unwatch,
                lang,
            );
            let new_ids: Vec<String> = view.options.iter().map(|o| o.id.clone()).collect();
            refresh_select_card_edit(channel_id, config, mid, &view, lang).await;
            if let Some(p) = state
                .select
                .pickers
                .lock()
                .unwrap()
                .iter_mut()
                .find(|p| p.channel == channel_id && p.message_id == mid)
            {
                p.options = new_ids;
            }
        }
    }

    /// 就地把 TG/Slack 单选卡编辑为新的一版单选卡（`/unwatch` 移除该项后刷新）。
    async fn refresh_select_card_edit(
        channel_id: &str,
        config: &AppConfig,
        mid: &str,
        view: &crate::select::SelectView,
        lang: Lang,
    ) {
        match channel_id {
            "telegram" => {
                let Ok(m) = mid.parse::<i64>() else { return };
                let tg = &config.channels.telegram;
                if let Ok(c) = crate::telegram::TelegramClient::new(
                    tg.bot_token.clone(),
                    tg.chat_id.clone(),
                    tg.api_base_url.clone(),
                ) {
                    let html = crate::telegram::select::render_select_html(view);
                    let markup = crate::telegram::select::inline_keyboard(view, lang);
                    if let Err(err) = c
                        .edit_message_text(m, &html, Some("HTML"), Some(markup))
                        .await
                    {
                        log(&format!(
                            "select: refresh telegram select card failed: {}",
                            err
                        ));
                    }
                }
            }
            "slack" => {
                if let Ok(c) = crate::slack::client::SlackClient::new(&config.channels.slack) {
                    if let Ok(dm) = c.open_dm().await {
                        let (blocks, fallback) =
                            crate::slack::select::build_select_blocks(view, lang);
                        if let Err(err) = c.update_message(&dm, mid, Some(&blocks), &fallback).await
                        {
                            log(&format!(
                                "select: refresh slack select card failed: {}",
                                err
                            ));
                        }
                    }
                }
            }
            _ => {}
        }
    }

    /// 就地把 TG/Slack 单选卡定格为无按钮终态（标题 + 定格文案）。
    async fn finalize_select_card_edit(
        channel_id: &str,
        config: &AppConfig,
        mid: &str,
        title: &str,
        final_label: &str,
    ) {
        match channel_id {
            "telegram" => {
                let Ok(m) = mid.parse::<i64>() else { return };
                let tg = &config.channels.telegram;
                if let Ok(c) = crate::telegram::TelegramClient::new(
                    tg.bot_token.clone(),
                    tg.chat_id.clone(),
                    tg.api_base_url.clone(),
                ) {
                    let html =
                        crate::telegram::select::render_select_final_html(title, final_label);
                    if let Err(err) = c.edit_message_text(m, &html, Some("HTML"), None).await {
                        log(&format!(
                            "select: finalize telegram select card failed: {}",
                            err
                        ));
                    }
                }
            }
            "slack" => {
                if let Ok(c) = crate::slack::client::SlackClient::new(&config.channels.slack) {
                    if let Ok(dm) = c.open_dm().await {
                        let (blocks, fallback) =
                            crate::slack::select::build_select_final_blocks(title, final_label);
                        if let Err(err) = c.update_message(&dm, mid, Some(&blocks), &fallback).await
                        {
                            log(&format!(
                                "select: finalize slack select card failed: {}",
                                err
                            ));
                        }
                    }
                }
            }
            _ => {}
        }
    }

    /// `/msg` 系命令的寻址公共段（spec agent-interject D9）：编号（复用 `/status` 稳定 seq）→
    /// 注册表快照记录 → 校验可插话（grok 无传话通道、ended 无处送达）。失败时已回提示、返回 None。
    /// `require_working`：为真时（发送场景）目标必须「工作中」，否则回提示（用户定案：只能给工作中的
    /// agent 发送插话）；回显 / 撤回场景传 false（对空闲也可操作）。
    async fn resolve_msg_target(
        state: &Arc<ServerState>,
        channel_id: &str,
        sel: Option<u64>,
        require_working: bool,
        config: &AppConfig,
        lang: Lang,
    ) -> Option<String> {
        let prefix = crate::autochannel::cmd_prefix(channel_id);
        let Some(id) = sel else {
            let text = crate::i18n::tr(lang, "autoChannel.msgUsage").replace("{p}", prefix);
            let _ = reply_channel_text(channel_id, config, &text).await;
            return None;
        };
        let snapshot = state.agents.snapshot();
        let Some(rec) = crate::autochannel::find_by_seq(&snapshot, id) else {
            let text = crate::i18n::tr(lang, "autoChannel.statusDetailNotFound")
                .replace("{id}", &id.to_string())
                .replace("{p}", prefix);
            let _ = reply_channel_text(channel_id, config, &text).await;
            return None;
        };
        if rec.get("kind").and_then(|v| v.as_str()) == Some("grok") {
            let _ = reply_channel_text(
                channel_id,
                config,
                crate::i18n::tr(lang, "autoChannel.msgGrokUnsupported"),
            )
            .await;
            return None;
        }
        if rec.get("state").and_then(|v| v.as_str()) == Some("ended") {
            let _ = reply_channel_text(
                channel_id,
                config,
                crate::i18n::tr(lang, "autoChannel.msgEnded"),
            )
            .await;
            return None;
        }
        if require_working && rec.get("state").and_then(|v| v.as_str()) != Some("working") {
            let _ = reply_channel_text(
                channel_id,
                config,
                crate::i18n::tr(lang, "select.msgNoWorkingTarget"),
            )
            .await;
            return None;
        }
        rec.get("sessionId")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
    }

    /// `/msg <编号> [内容]`（spec agent-interject D2/D9）：有内容 → **追加**排队（IM 看不到旧文本，
    /// 覆盖会静默丢内容；恰有 hook 挂起等待则立即送达）；无内容 → 回显当前待送达全文。
    async fn handle_msg_cmd(
        state: &Arc<ServerState>,
        channel_id: &str,
        sel: Option<u64>,
        content: Option<String>,
        config: &AppConfig,
        lang: Lang,
    ) {
        // `/msg` 插话＝在该渠道主动参与 → 设为活跃槽（用户决策）。
        activate_channel_on_action(state, channel_id, config, lang).await;
        match (sel, content) {
            // 显式编号 + 内容 → 发送（收紧为仅工作中）。
            (Some(_), Some(content)) => {
                let Some(sid) =
                    resolve_msg_target(state, channel_id, sel, true, config, lang).await
                else {
                    return;
                };
                let text = deliver_msg(state, channel_id, &sid, &content, lang);
                let _ = reply_channel_text(channel_id, config, &text).await;
            }
            // 显式编号无内容 → 回显待送达（不限工作中）。
            (Some(_), None) => {
                let Some(sid) =
                    resolve_msg_target(state, channel_id, sel, false, config, lang).await
                else {
                    return;
                };
                let text = msg_echo_text(state, &sid, lang);
                let _ = reply_channel_text(channel_id, config, &text).await;
            }
            // 无编号 + 内容 → 自动选择（关注恰 1 个且工作中直发；否则弹选择卡）。
            (None, Some(content)) => {
                handle_msg_auto(state, channel_id, content, config, lang).await;
            }
            // 无编号无内容 → 增强用法提示（用法示例 + 当前工作中 agent 列表）。
            (None, None) => {
                let text = msg_usage_hint(state, channel_id, lang);
                let _ = reply_channel_text(channel_id, config, &text).await;
            }
        }
    }

    /// 追加一条插话并回执文案（`n==0` ⇒ 恰有 hook 挂起等待 → 立即送达）。发送三路径共用
    /// （显式编号 / 无编号直发 / 单选卡点「发送」）。`channel_id`＝来源渠道：排队时登记，供消息被
    /// agent 消费后回推「已阅读」回执（D9）。
    fn deliver_msg(
        state: &Arc<ServerState>,
        channel_id: &str,
        session_id: &str,
        content: &str,
        lang: Lang,
    ) -> String {
        let n = state
            .interject
            .append(session_id, content, Some(channel_id));
        state.interject.persist();
        broadcast_agents_state(state);
        if n == 0 {
            crate::i18n::tr(lang, "autoChannel.msgDeliveredNow").to_string()
        } else {
            crate::i18n::tr(lang, "autoChannel.msgQueued").replace("{n}", &n.to_string())
        }
    }

    /// 排队插话被消费后，给各来源渠道回推一条「已阅读」回执（编号按当前快照现算）。仅在有待回执
    /// 渠道时才 spawn（罕见），不拖慢 hook 热路径。
    fn spawn_read_receipts(state: &Arc<ServerState>, session_id: &str, channels: Vec<String>) {
        if channels.is_empty() {
            return;
        }
        let state = state.clone();
        let session_id = session_id.to_string();
        tokio::spawn(async move {
            let lang = Lang::current();
            let snapshot = state.agents.snapshot();
            let seq = find_agent_by_session(&snapshot, &session_id)
                .and_then(|r| r.get("seq").and_then(|v| v.as_u64()))
                .unwrap_or(0);
            let text = crate::i18n::tr(lang, "autoChannel.msgReadReceipt")
                .replace("{id}", &seq.to_string());
            let config = AppConfig::load();
            for ch in channels {
                let _ = reply_channel_text(&ch, &config, &text).await;
            }
        });
    }

    /// `/msg <编号>`（无内容）回显该 agent 待送达全文（无则「暂无待送达」）。
    fn msg_echo_text(state: &Arc<ServerState>, session_id: &str, lang: Lang) -> String {
        let full = state.interject.full_text(session_id);
        if full.is_empty() {
            crate::i18n::tr(lang, "autoChannel.msgNone").to_string()
        } else {
            let n = state.interject.pending_count(session_id);
            format!(
                "{}\n{}",
                crate::i18n::tr(lang, "autoChannel.msgEchoHeader").replace("{n}", &n.to_string()),
                full
            )
        }
    }

    /// 快照中该 session 是否「工作中·非 grok」（插话可发的前提；直发短路判定用）。
    fn is_working_non_grok(snapshot: &serde_json::Value, session_id: &str) -> bool {
        find_agent_by_session(snapshot, session_id)
            .map(|r| {
                r.get("state").and_then(|v| v.as_str()) == Some("working")
                    && r.get("kind").and_then(|v| v.as_str()) != Some("grok")
            })
            .unwrap_or(false)
    }

    /// 工作中·非 grok 的 agent 列表行（`[编号] 类型 — 标题（项目）`；用法提示 / 兜底文本用）。
    fn working_agent_lines(snapshot: &serde_json::Value, lang: Lang) -> Vec<String> {
        let empty = Vec::new();
        snapshot
            .as_array()
            .unwrap_or(&empty)
            .iter()
            .filter(|r| {
                r.get("state").and_then(|v| v.as_str()) == Some("working")
                    && r.get("kind").and_then(|v| v.as_str()) != Some("grok")
            })
            .map(|r| {
                let seq = r.get("seq").and_then(|v| v.as_u64()).unwrap_or(0);
                format!(
                    "[{}] {}",
                    seq,
                    crate::autochannel::kind_title_project(r, lang)
                )
            })
            .collect()
    }

    /// `/msg`（无编号无内容）增强用法提示：用法示例 + 当前工作中 agent 列表（带编号，可直接
    /// `/msg <编号> <内容>` 定向）。无工作中 → 附一行「当前没有工作中的 Agent」。
    fn msg_usage_hint(state: &Arc<ServerState>, channel_id: &str, lang: Lang) -> String {
        let prefix = crate::autochannel::cmd_prefix(channel_id);
        let mut out = crate::i18n::tr(lang, "autoChannel.msgUsage").replace("{p}", prefix);
        let snapshot = state.agents.snapshot();
        let lines = working_agent_lines(&snapshot, lang);
        out.push_str("\n\n");
        if lines.is_empty() {
            out.push_str(crate::i18n::tr(lang, "select.msgNoWorking"));
        } else {
            out.push_str(&lines.join("\n"));
        }
        out
    }

    /// `/msg <内容>`（无编号）：本渠道关注恰 1 个且该 agent 工作中·非 grok → 直发；否则弹选择卡
    /// （列工作中·非 grok，每行「发送」按钮）；无可发对象 → 提示、不弹卡。
    async fn handle_msg_auto(
        state: &Arc<ServerState>,
        channel_id: &str,
        content: String,
        config: &AppConfig,
        lang: Lang,
    ) {
        let snapshot = state.agents.snapshot();
        let watching = watching_sessions(state, channel_id);
        // 直发条件（用户定案）：只有明确关注恰 1 个、且它工作中·非 grok 时才直发，避免发错。
        if watching.len() == 1 {
            if let Some(sid) = watching.iter().next().cloned() {
                if is_working_non_grok(&snapshot, &sid) {
                    let text = deliver_msg(state, channel_id, &sid, &content, lang);
                    let _ = reply_channel_text(channel_id, config, &text).await;
                    return;
                }
            }
        }
        // 否则弹选择卡（列工作中·非 grok）；关注中的仍带「· 关注中」徽标。
        let opts = crate::select::msg_options(&snapshot, &watching, now_secs(), lang);
        if opts.is_empty() {
            let _ = reply_channel_text(
                channel_id,
                config,
                crate::i18n::tr(lang, "select.msgNoWorking"),
            )
            .await;
            return;
        }
        let sent = send_agent_picker(
            state,
            channel_id,
            config,
            PickerKind::Msg,
            crate::select::title_msg(lang),
            opts,
            Some(content),
            lang,
        )
        .await;
        if !sent {
            // 发卡失败（非支持渠道 / API 失败）：回工作中列表兜底，用户可 `/msg <编号> <内容>` 定向。
            let text = msg_usage_hint(state, channel_id, lang);
            let _ = reply_channel_text(channel_id, config, &text).await;
        }
    }

    /// `/msg-clear <编号>`（`/撤回`）：清空该 agent 的待送达插话 + 回执。
    async fn handle_msg_clear_cmd(
        state: &Arc<ServerState>,
        channel_id: &str,
        sel: Option<u64>,
        config: &AppConfig,
        lang: Lang,
    ) {
        // `/msg-clear` 撤回插话＝在该渠道操作 → 设为活跃槽（用户决策）。
        activate_channel_on_action(state, channel_id, config, lang).await;
        let Some(sid) = resolve_msg_target(state, channel_id, sel, false, config, lang).await
        else {
            return;
        };
        let text = if state.interject.clear(&sid) {
            state.interject.persist();
            broadcast_agents_state(state);
            crate::i18n::tr(lang, "autoChannel.msgCleared")
        } else {
            crate::i18n::tr(lang, "autoChannel.msgNone")
        };
        let _ = reply_channel_text(channel_id, config, text).await;
    }

    /// 统一入站分派（与渠道无关），spec R3/R4：
    /// - `/status`：始终回状态文本（开关开且因此切槽时附激活回执）。
    /// - `/here`：开关开时激活+补推+回执；开关关时改回**引导文案**（不再静默忽略）。
    /// - `/help` 与未知 `/命令`：回**动态引导文案**（命令永不被卡片当答案，安全回）。
    /// - 普通文本：该渠道**有活动在途提问**时退避（交渠道会话确认/引导，避免重复回复）；
    ///   否则开关开按现状切槽（切换则回激活回执），未切换/未开则回引导（liveness）。
    /// - 非文本消息（`text=None`）：无活动在途提问时回引导（有则交会话确认附件）。
    async fn handle_inbound(state: &Arc<ServerState>, channel_id: &str, text: Option<&str>) {
        use crate::autochannel::{classify, help_text, Command, Parsed};
        let lang = Lang::current();
        let config = AppConfig::load();
        let auto = config.channels.auto_activation;
        // `/watch` 渠道门控（spec docs/specs/im-watch.md）：决定 help 是否列 watch 命令。
        let watch_cmd = crate::watch::channel_supported(channel_id);
        // 命令展示前缀：Slack 客户端拦截 `/` 输入，提示用 `!`；其余渠道 `/`。
        let prefix = crate::autochannel::cmd_prefix(channel_id);
        // 任何用户入站消息都会把 watch 卡顶上去（机器人的文本回执紧随其后，同属一次扰动）。
        mark_watch_disturbed(state, channel_id);

        let Some(text) = text else {
            // 非文本消息（图片/文件）：有活动提问 → 交渠道会话确认；否则回引导（liveness）。
            if !has_active_question_on(state, channel_id) {
                let _ = reply_channel_text(
                    channel_id,
                    &config,
                    &help_text(auto, false, watch_cmd, prefix, lang),
                )
                .await;
            }
            return;
        };

        match classify(text) {
            Parsed::Command(Command::Status(sel)) => {
                // 状态查询是独立功能：始终响应。仅当开关开、且本次因 /status 切了活跃槽时附激活回执。
                let (switched, n) = if auto {
                    set_active_channel(state, channel_id).await
                } else {
                    (false, 0)
                };
                let snapshot = state.agents.snapshot();
                match sel {
                    // /status <编号>：单个 agent 的当前活动详情（直达，不弹卡）。
                    Some(id) => {
                        let mut body = String::new();
                        if switched {
                            body.push_str(&crate::autochannel::activated_receipt(n, lang));
                            body.push_str("\n\n");
                        }
                        body.push_str(&crate::autochannel::status_detail_text(
                            &snapshot, id, prefix, lang,
                        ));
                        let _ = reply_channel_text(channel_id, &config, &body).await;
                    }
                    // /status（无参）：切槽回执（若有）作独立文本，随后推「选择要查看的 Agent」单选卡；
                    // 无 agent / 非飞书 → 回既有工作中/空闲文本列表兜底。
                    None => {
                        if switched {
                            let _ = reply_channel_text(
                                channel_id,
                                &config,
                                &crate::autochannel::activated_receipt(n, lang),
                            )
                            .await;
                        }
                        let opts = crate::select::agent_options(
                            &snapshot,
                            &std::collections::HashSet::new(),
                            now_secs(),
                            lang,
                        );
                        let sent = send_agent_picker(
                            state,
                            channel_id,
                            &config,
                            PickerKind::Status,
                            crate::select::title_status(lang),
                            opts,
                            None,
                            lang,
                        )
                        .await;
                        if !sent {
                            let _ = reply_channel_text(
                                channel_id,
                                &config,
                                &crate::autochannel::status_text(&snapshot, lang),
                            )
                            .await;
                        }
                    }
                }
            }
            Parsed::Command(Command::Here) => {
                if !auto {
                    // 关态无「活跃槽」概念：回引导（替代旧的静默忽略）。
                    let has_q = has_active_question_on(state, channel_id);
                    let _ = reply_channel_text(
                        channel_id,
                        &config,
                        &help_text(auto, has_q, watch_cmd, prefix, lang),
                    )
                    .await;
                    return;
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
            // /watch、/unwatch：实时关注（P1 仅飞书；其余渠道回「暂仅支持飞书」提示）。
            Parsed::Command(Command::Watch(sel)) => {
                // `/watch` 属「在该渠道操作」→ 设为活跃槽（用户决策；配合 D2 让离开时自动结束 watch）。
                activate_channel_on_action(state, channel_id, &config, lang).await;
                match sel {
                    // /watch <编号>：直达关注（不弹卡）。
                    Some(_) => handle_watch_cmd(state, channel_id, sel, &config, lang).await,
                    // /watch（无参）：推「选择要关注的 Agent」单选卡（仅工作中；已关注者带
                    // 「· 关注中」徽标，点它＝换新卡）。无工作中 agent → 回文本列表兜底。
                    None => {
                        let snapshot = state.agents.snapshot();
                        let watching = watching_sessions(state, channel_id);
                        let opts =
                            crate::select::watch_options(&snapshot, &watching, now_secs(), lang);
                        let sent = send_agent_picker(
                            state,
                            channel_id,
                            &config,
                            PickerKind::Watch,
                            crate::select::title_watch(lang),
                            opts,
                            None,
                            lang,
                        )
                        .await;
                        if !sent {
                            handle_watch_cmd(state, channel_id, None, &config, lang).await;
                        }
                    }
                }
            }
            Parsed::Command(Command::Unwatch(sel)) => {
                use crate::autochannel::WatchSel;
                // 仅「无参且本渠道有多个关注」时弹卡；0/1/编号/all 一律直达（行为不变）。
                let multi = matches!(sel, WatchSel::Auto)
                    && state
                        .watch
                        .subs
                        .lock()
                        .unwrap()
                        .iter()
                        .filter(|s| s.channel == channel_id)
                        .count()
                        >= 2;
                let sent = if multi {
                    let snapshot = state.agents.snapshot();
                    let opts = unwatch_options(state, channel_id, &snapshot, lang);
                    send_agent_picker(
                        state,
                        channel_id,
                        &config,
                        PickerKind::Unwatch,
                        crate::select::title_unwatch(lang),
                        opts,
                        None,
                        lang,
                    )
                    .await
                } else {
                    false
                };
                if !sent {
                    handle_unwatch_cmd(state, channel_id, sel, &config, lang).await;
                }
            }
            // /msg、/msg-clear：插话（spec agent-interject D9；与 /status 同门控，始终响应）。
            Parsed::Command(Command::Msg(sel, content)) => {
                handle_msg_cmd(state, channel_id, sel, content, &config, lang).await;
            }
            Parsed::Command(Command::MsgClear(sel)) => {
                handle_msg_clear_cmd(state, channel_id, sel, &config, lang).await;
            }
            // /diff · /stage · /transcript（spec im-diff-stage-transcript）。
            Parsed::Command(Command::Diff(sel)) => {
                handle_export_cmd(state, channel_id, sel, PickerKind::Diff, &config, lang).await;
            }
            Parsed::Command(Command::Stage(sel)) => {
                handle_export_cmd(state, channel_id, sel, PickerKind::Stage, &config, lang).await;
            }
            Parsed::Command(Command::Transcript(sel)) => {
                handle_export_cmd(
                    state,
                    channel_id,
                    sel,
                    PickerKind::Transcript,
                    &config,
                    lang,
                )
                .await;
            }
            Parsed::Command(Command::Help) | Parsed::UnknownCommand => {
                let has_q = has_active_question_on(state, channel_id);
                let _ = reply_channel_text(
                    channel_id,
                    &config,
                    &help_text(auto, has_q, watch_cmd, prefix, lang),
                )
                .await;
            }
            Parsed::Text => {
                let has_q = has_active_question_on(state, channel_id);
                if auto {
                    let (switched, n) = set_active_channel(state, channel_id).await;
                    if switched {
                        let _ = reply_channel_text(
                            channel_id,
                            &config,
                            &crate::autochannel::activated_receipt(n, lang),
                        )
                        .await;
                        return;
                    }
                }
                if has_q {
                    return;
                }
                let _ = reply_channel_text(
                    channel_id,
                    &config,
                    &help_text(auto, false, watch_cmd, prefix, lang),
                )
                .await;
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
                // 反激活提示可在无该渠道入站时发出（如在别的渠道作答切槽）→ 单独记扰动。
                mark_watch_disturbed(state, &old);
                // 「按需发送」子开关：活跃槽从某 IM 切走时自动结束该渠道的全部 watch（D1/D2，
                // spec docs/specs/im-auto-end-watch.md）。卡片定格「已切换到 {new} · 自动结束关注」，
                // 不额外发文字（D4，反激活提示已发）。
                if cfg.channels.auto_activation && cfg.channels.auto_end_watch {
                    let targets: Vec<WatchEntry> = state
                        .watch
                        .subs
                        .lock()
                        .unwrap()
                        .iter()
                        .filter(|s| s.channel == old)
                        .cloned()
                        .collect();
                    let final_kind = crate::watch::FinalKind::AutoStopped(
                        crate::autochannel::channel_label(new_id, Lang::current()),
                    );
                    finalize_and_drop_watches(
                        state,
                        &old,
                        &targets,
                        final_kind,
                        &cfg,
                        Lang::current(),
                    )
                    .await;
                }
            }
        }
        // 激活即补推在途（仅真实 IM；弹窗无卡片概念）。
        let backfilled = if new_id != "popup" {
            backfill_inflight(state, new_id, &cfg).await
        } else {
            0
        };
        if backfilled > 0 {
            mark_watch_disturbed(state, new_id); // 补推的提问卡也是「非 watch」消息。
        }
        (true, backfilled)
    }

    /// 「在该渠道操作即激活」的统一入口：`auto_activation` 开时把活跃槽切到本渠道；真正切换了
    /// 就回一条激活回执（与 `/here`、`/status`、普通文本一致）。用于 `/watch`、`/msg`、`/msg-clear`、
    /// `/diff`/`/stage`/`/transcript` 及单选卡点选——这些本属「在渠道上说话」，理应设为活跃槽。
    async fn activate_channel_on_action(
        state: &Arc<ServerState>,
        channel_id: &str,
        config: &AppConfig,
        lang: Lang,
    ) {
        if !config.channels.auto_activation {
            return;
        }
        let (switched, n) = set_active_channel(state, channel_id).await;
        if switched {
            let _ = reply_channel_text(
                channel_id,
                config,
                &crate::autochannel::activated_receipt(n, lang),
            )
            .await;
        }
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
                ch.start(entry.request(), entry.coordinator.clone());
                n += 1;
            }
        }
        for entry in state.registry.in_flight_confirm_entries() {
            if entry.coordinator.is_terminal() || entry.has_delivery(channel_id) {
                continue;
            }
            let eligible = confirm_im_candidates(&entry, state, config);
            if !eligible.contains(&channel_id) {
                continue;
            }
            entry.start_delivery(channel_id);
            attach_confirm_im_channels(&entry, state, config, &[channel_id]).await;
            n += 1;
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

    // ── /diff · /stage · /transcript（spec im-diff-stage-transcript）──

    async fn handle_export_cmd(
        state: &Arc<ServerState>,
        channel_id: &str,
        sel: Option<u64>,
        kind: PickerKind,
        config: &AppConfig,
        lang: Lang,
    ) {
        activate_channel_on_action(state, channel_id, config, lang).await;
        match sel {
            Some(n) => {
                let snapshot = state.agents.snapshot();
                let Some(rec) = crate::autochannel::find_by_seq(&snapshot, n) else {
                    let prefix = crate::autochannel::cmd_prefix(channel_id);
                    let text = crate::i18n::tr(lang, "export.notFound")
                        .replace("{n}", &n.to_string())
                        .replace("{p}", prefix);
                    let _ = reply_channel_text(channel_id, config, &text).await;
                    return;
                };
                let sid = rec
                    .get("sessionId")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                if sid.is_empty() {
                    let _ = reply_channel_text(
                        channel_id,
                        config,
                        crate::i18n::tr(lang, "export.noCwd"),
                    )
                    .await;
                    return;
                }
                match kind {
                    PickerKind::Diff => run_diff(state, channel_id, &sid, config, lang).await,
                    PickerKind::Transcript => {
                        run_transcript(state, channel_id, &sid, config, lang).await
                    }
                    PickerKind::Stage => {
                        run_stage_confirm(state, channel_id, &sid, config, lang).await
                    }
                    _ => {}
                }
            }
            None => {
                let snapshot = state.agents.snapshot();
                let opts = crate::select::agent_options(
                    &snapshot,
                    &std::collections::HashSet::new(),
                    now_secs(),
                    lang,
                );
                let title = match kind {
                    PickerKind::Diff => crate::select::title_diff(lang),
                    PickerKind::Stage => crate::select::title_stage(lang),
                    PickerKind::Transcript => crate::select::title_transcript(lang),
                    _ => String::new(),
                };
                let sent =
                    send_agent_picker(state, channel_id, config, kind, title, opts, None, lang)
                        .await;
                if !sent {
                    let _ = reply_channel_text(
                        channel_id,
                        config,
                        &crate::autochannel::status_text(&snapshot, lang),
                    )
                    .await;
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn select_pick_export(
        state: &Arc<ServerState>,
        channel_id: &str,
        mid: &str,
        session_id: &str,
        kind: PickerKind,
        config: &AppConfig,
        lang: Lang,
        ack: Option<tokio::sync::oneshot::Sender<Option<serde_json::Value>>>,
    ) {
        let snapshot = state.agents.snapshot();
        let seq = find_agent_by_session(&snapshot, session_id)
            .and_then(|r| r.get("seq").and_then(|v| v.as_u64()))
            .unwrap_or(0);
        let label_key = match kind {
            PickerKind::Diff => "select.diffDoneCard",
            PickerKind::Stage => "select.stageOpenedCard",
            PickerKind::Transcript => "select.transcriptDoneCard",
            _ => "select.diffDoneCard",
        };
        let title = match kind {
            PickerKind::Diff => crate::select::title_diff(lang),
            PickerKind::Stage => crate::select::title_stage(lang),
            PickerKind::Transcript => crate::select::title_transcript(lang),
            _ => String::new(),
        };
        let label = crate::i18n::tr(lang, label_key).replace("{id}", &seq.to_string());
        if channel_id == "feishu" {
            if let Some(ack) = ack {
                let card = crate::feishu::card::build_select_final_card(&title, &label);
                let _ = ack.send(Some(crate::feishu::card::callback_update_card(card)));
            }
        } else if channel_id == "dingding" {
            dd_finalize_select_card(config, mid, &label).await;
        } else {
            finalize_select_card_edit(channel_id, config, mid, &title, &label).await;
        }
        remove_picker(state, channel_id, mid);
        match kind {
            PickerKind::Diff => run_diff(state, channel_id, session_id, config, lang).await,
            PickerKind::Transcript => {
                run_transcript(state, channel_id, session_id, config, lang).await
            }
            PickerKind::Stage => {
                run_stage_confirm(state, channel_id, session_id, config, lang).await
            }
            _ => {}
        }
    }

    fn agent_export_meta(
        snapshot: &serde_json::Value,
        session_id: &str,
    ) -> Option<(u64, String, String, String, Option<String>)> {
        let rec = find_agent_by_session(snapshot, session_id)?;
        let seq = rec.get("seq").and_then(|v| v.as_u64()).unwrap_or(0);
        let kind = rec
            .get("kind")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let title = rec
            .get("title")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let cwd = rec
            .get("cwd")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let project = cwd
            .as_deref()
            .and_then(crate::autochannel::project_name)
            .unwrap_or_else(|| "project".into());
        Some((seq, kind, title, project, cwd))
    }

    async fn run_diff(
        state: &Arc<ServerState>,
        channel_id: &str,
        session_id: &str,
        config: &AppConfig,
        lang: Lang,
    ) {
        let snapshot = state.agents.snapshot();
        let Some((seq, kind, _title, project, cwd)) = agent_export_meta(&snapshot, session_id)
        else {
            let _ =
                reply_channel_text(channel_id, config, crate::i18n::tr(lang, "export.noCwd")).await;
            return;
        };
        let Some(cwd) = cwd else {
            let _ =
                reply_channel_text(channel_id, config, crate::i18n::tr(lang, "export.noCwd")).await;
            return;
        };
        let Some(root) = crate::gitutil::find_git_root(std::path::Path::new(&cwd)) else {
            let text = crate::i18n::tr(lang, "export.notGit").replace("{path}", &cwd);
            let _ = reply_channel_text(channel_id, config, &text).await;
            return;
        };
        let model = match crate::gitutil::build_diff_model(&root) {
            Ok(m) => m,
            Err(e) => {
                let _ = reply_channel_text(channel_id, config, &e).await;
                return;
            }
        };
        if model.total_paths == 0 {
            let _ = reply_channel_text(
                channel_id,
                config,
                crate::i18n::tr(lang, "export.noUnstaged"),
            )
            .await;
            return;
        }
        let meta = format!("Diff · [{seq}] {kind} · {project}");
        // 用户定案：直接发附件，不附摘要消息头。
        let (bytes, name) = match channel_id {
            "feishu" => {
                let md = crate::export::render_diff_md(&model, &meta);
                (
                    md.into_bytes(),
                    crate::export::diff_filename(seq, &project, "md"),
                )
            }
            "dingding" | "slack" => match crate::export::render_diff_docx(&model, &meta) {
                Ok(b) => (b, crate::export::diff_filename(seq, &project, "docx")),
                Err(e) => {
                    let t = crate::i18n::tr(lang, "export.sendFailed").replace("{err}", &e);
                    let _ = reply_channel_text(channel_id, config, &t).await;
                    return;
                }
            },
            _ => {
                // telegram (and any other)
                let html = crate::export::render_diff_html(&model, &meta);
                (
                    html.into_bytes(),
                    crate::export::diff_filename(seq, &project, "html"),
                )
            }
        };
        if let Err(e) = reply_channel_file(channel_id, config, &name, &bytes).await {
            let t = crate::i18n::tr(lang, "export.sendFailed").replace("{err}", &e);
            let _ = reply_channel_text(channel_id, config, &t).await;
        }
    }

    async fn run_transcript(
        state: &Arc<ServerState>,
        channel_id: &str,
        session_id: &str,
        config: &AppConfig,
        lang: Lang,
    ) {
        let snapshot = state.agents.snapshot();
        let Some((seq, kind_s, title, project, _)) = agent_export_meta(&snapshot, session_id)
        else {
            let _ =
                reply_channel_text(channel_id, config, crate::i18n::tr(lang, "export.noCwd")).await;
            return;
        };
        let Some(akind) = crate::agents::AgentKind::parse(&kind_s) else {
            let _ = reply_channel_text(
                channel_id,
                config,
                crate::i18n::tr(lang, "export.noTranscript"),
            )
            .await;
            return;
        };
        let doc = match crate::agents::transcript_full::load_events(akind, session_id) {
            Ok(d) => d,
            Err(_) => {
                let _ = reply_channel_text(
                    channel_id,
                    config,
                    crate::i18n::tr(lang, "export.noTranscript"),
                )
                .await;
                return;
            }
        };
        let meta = format!(
            "Transcript · [{seq}] {kind_s} · {}",
            if title.is_empty() { &project } else { &title }
        );
        // 用户定案：直接发附件，不附摘要消息头。
        let slug_src = if title.is_empty() { &project } else { &title };
        let (bytes, name) = match channel_id {
            "feishu" => {
                let md = crate::export::render_transcript_md(&doc, &meta);
                (
                    md.into_bytes(),
                    crate::export::transcript_filename(seq, slug_src, "md"),
                )
            }
            "dingding" | "slack" => match crate::export::render_transcript_docx(&doc, &meta) {
                Ok(b) => (b, crate::export::transcript_filename(seq, slug_src, "docx")),
                Err(e) => {
                    let t = crate::i18n::tr(lang, "export.sendFailed").replace("{err}", &e);
                    let _ = reply_channel_text(channel_id, config, &t).await;
                    return;
                }
            },
            _ => {
                let html = crate::export::render_transcript_html(&doc, &meta);
                (
                    html.into_bytes(),
                    crate::export::transcript_filename(seq, slug_src, "html"),
                )
            }
        };
        if let Err(e) = reply_channel_file(channel_id, config, &name, &bytes).await {
            let t = crate::i18n::tr(lang, "export.sendFailed").replace("{err}", &e);
            let _ = reply_channel_text(channel_id, config, &t).await;
        }
    }

    async fn run_stage_confirm(
        state: &Arc<ServerState>,
        channel_id: &str,
        session_id: &str,
        config: &AppConfig,
        lang: Lang,
    ) {
        let snapshot = state.agents.snapshot();
        let Some((_seq, _kind, _title, project, cwd)) = agent_export_meta(&snapshot, session_id)
        else {
            let _ =
                reply_channel_text(channel_id, config, crate::i18n::tr(lang, "export.noCwd")).await;
            return;
        };
        let Some(cwd) = cwd else {
            let _ =
                reply_channel_text(channel_id, config, crate::i18n::tr(lang, "export.noCwd")).await;
            return;
        };
        let Some(root) = crate::gitutil::find_git_root(std::path::Path::new(&cwd)) else {
            let text = crate::i18n::tr(lang, "export.notGit").replace("{path}", &cwd);
            let _ = reply_channel_text(channel_id, config, &text).await;
            return;
        };
        let preview = match crate::gitutil::preview_stage(&root) {
            Ok(p) => p,
            Err(e) => {
                let _ = reply_channel_text(channel_id, config, &e).await;
                return;
            }
        };
        if preview.paths.is_empty() {
            let _ = reply_channel_text(
                channel_id,
                config,
                crate::i18n::tr(lang, "export.noUnstaged"),
            )
            .await;
            return;
        }
        let view =
            crate::confirm::stage_confirm_view(lang, &project, &preview.paths, preview.paths.len());
        let Some(mid) = crate::confirm::transport::send(channel_id, config, &view).await else {
            let _ = reply_channel_text(channel_id, config, "Failed to send confirm card").await;
            return;
        };
        {
            let mut cs = state.select.confirms.lock().unwrap();
            let now = now_secs();
            cs.retain(|c| now.saturating_sub(c.created_at) < SELECT_PICKER_TTL_SECS);
            cs.push(ConfirmEntry {
                channel: channel_id.to_string(),
                message_id: mid,
                session_id: session_id.to_string(),
                git_root: root,
                paths_fp: crate::gitutil::paths_fingerprint(&preview.paths),
                view,
                created_at: now,
            });
        }
        state.select.route_refresh.notify_one();
    }

    /// 钉钉 stage 确认（专用确认模板双按钮）：成功返回 true。
    async fn handle_stage_dd_submit(state: &Arc<ServerState>, data: &serde_json::Value) -> bool {
        let Some((otid, slot)) = crate::dingtalk::confirm::parse_confirm_action(data) else {
            return false;
        };
        let has = {
            let cs = state.select.confirms.lock().unwrap();
            cs.iter()
                .any(|c| c.channel == "dingding" && c.message_id == otid)
        };
        if !has {
            return false;
        }
        handle_confirm_action(state, "dingding", &otid, slot, None).await;
        true
    }

    async fn handle_confirm_action(
        state: &Arc<ServerState>,
        channel_id: &str,
        mid: &str,
        slot: crate::confirm::ConfirmSlot,
        ack: Option<crate::confirm::transport::FsAck>,
    ) {
        let lang = Lang::current();
        let config = AppConfig::load();
        let entry = {
            let mut cs = state.select.confirms.lock().unwrap();
            let pos = cs
                .iter()
                .position(|c| c.channel == channel_id && c.message_id == mid);
            pos.map(|i| cs.remove(i))
        };
        let Some(entry) = entry else {
            if let Some(ack) = ack {
                let _ = ack.send(None);
            }
            return;
        };
        let action_id = entry.view.action_id_for_slot(slot).to_string();
        if action_id == crate::confirm::STAGE_CANCEL_ACTION_ID {
            let text = crate::i18n::tr(lang, "confirm.stageCancelled").to_string();
            let fv = crate::confirm::ConfirmFinalView {
                title: entry.view.title.clone(),
                body: text.clone(),
                label: crate::confirm::transport::truncate_for_label(&text),
            };
            crate::confirm::transport::finalize(channel_id, &config, mid, &fv, ack).await;
            state.select.route_refresh.notify_one();
            return;
        }
        if action_id != crate::confirm::STAGE_CONFIRM_ACTION_ID {
            if let Some(ack) = ack {
                let _ = ack.send(None);
            }
            return;
        }
        // Re-check paths fingerprint.
        let preview = match crate::gitutil::preview_stage(&entry.git_root) {
            Ok(p) => p,
            Err(e) => {
                let text = crate::i18n::tr(lang, "confirm.stageFailed").replace("{err}", &e);
                let fv = crate::confirm::ConfirmFinalView {
                    title: entry.view.title.clone(),
                    body: text.clone(),
                    label: crate::confirm::transport::truncate_for_label(&text),
                };
                crate::confirm::transport::finalize(channel_id, &config, mid, &fv, ack).await;
                state.select.route_refresh.notify_one();
                return;
            }
        };
        let fp = crate::gitutil::paths_fingerprint(&preview.paths);
        if fp != entry.paths_fp {
            let text = crate::i18n::tr(lang, "confirm.stageChanged").to_string();
            let fv = crate::confirm::ConfirmFinalView {
                title: entry.view.title.clone(),
                body: text.clone(),
                label: crate::confirm::transport::truncate_for_label(&text),
            };
            crate::confirm::transport::finalize(channel_id, &config, mid, &fv, ack).await;
            let _ = reply_channel_text(channel_id, &config, &text).await;
            state.select.route_refresh.notify_one();
            return;
        }
        match crate::gitutil::stage_all(&entry.git_root) {
            Ok(r) => {
                let text = crate::i18n::tr(lang, "confirm.stageDone")
                    .replace("{n}", &r.paths.len().to_string());
                let fv = crate::confirm::ConfirmFinalView {
                    title: entry.view.title.clone(),
                    body: text.clone(),
                    label: crate::confirm::transport::truncate_for_label(&text),
                };
                crate::confirm::transport::finalize(channel_id, &config, mid, &fv, ack).await;
                let show: Vec<&str> = r
                    .paths
                    .iter()
                    .take(crate::confirm::STAGE_LIST_MAX)
                    .map(|s| s.as_str())
                    .collect();
                let mut detail = text.clone();
                if !show.is_empty() {
                    detail.push('\n');
                    detail.push_str(&show.join("\n"));
                    if r.paths.len() > show.len() {
                        detail.push_str(&format!("\n… +{}", r.paths.len() - show.len()));
                    }
                }
                let _ = reply_channel_text(channel_id, &config, &detail).await;
            }
            Err(e) => {
                let text = crate::i18n::tr(lang, "confirm.stageFailed").replace("{err}", &e);
                let fv = crate::confirm::ConfirmFinalView {
                    title: entry.view.title.clone(),
                    body: text.clone(),
                    label: crate::confirm::transport::truncate_for_label(&text),
                };
                crate::confirm::transport::finalize(channel_id, &config, mid, &fv, ack).await;
                let _ = reply_channel_text(channel_id, &config, &text).await;
            }
        }
        state.select.route_refresh.notify_one();
    }

    async fn reply_channel_file(
        channel_id: &str,
        config: &AppConfig,
        file_name: &str,
        bytes: &[u8],
    ) -> Result<(), String> {
        let dir = crate::paths::request_temp_dir(&format!("export-{}", now_ms()));
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let path = dir.join(file_name);
        std::fs::write(&path, bytes).map_err(|e| e.to_string())?;
        let path_str = path.to_string_lossy().to_string();
        let result = match channel_id {
            "feishu" => {
                let client = crate::feishu::client::FeishuClient::new(&config.channels.feishu)
                    .map_err(|e| e.to_string())?;
                let key = client
                    .upload_file(&path_str, file_name)
                    .await
                    .map_err(|e| e.to_string())?;
                client
                    .send_file(&key)
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
                    .send_document(&path_str, file_name)
                    .await
                    .map(|_| ())
                    .map_err(|e| e.to_string())
            }
            "slack" => {
                let client = crate::slack::client::SlackClient::new(&config.channels.slack)
                    .map_err(|e| e.to_string())?;
                let dm = client.open_dm().await.map_err(|e| e.to_string())?;
                client
                    .upload_file(&dm, &path_str, file_name)
                    .await
                    .map_err(|e| e.to_string())
            }
            "dingding" => {
                let client =
                    crate::dingtalk::client::DingTalkClient::new(&config.channels.dingding)
                        .map_err(|e| e.to_string())?;
                let media_id = client
                    .upload_media(&path_str, "file")
                    .await
                    .map_err(|e| e.to_string())?;
                let ext = std::path::Path::new(file_name)
                    .extension()
                    .and_then(|e| e.to_str())
                    .unwrap_or("docx");
                client
                    .send_oto_file(&media_id, file_name, ext)
                    .await
                    .map_err(|e| e.to_string())
            }
            _ => Err(format!("file send unsupported for channel: {}", channel_id)),
        };
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir(&dir);
        result
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
                    Ok(id) => {
                        // spec R5：识别成功 → 经该 IM 给识别到的用户回一条「识别成功」回执（best-effort）。
                        send_detect_ack(req, &id, lang).await;
                        ServerMsg::Detected { id }
                    }
                    Err(message) => ServerMsg::Error { message },
                };
                let _ = ipc::write_msg(w, &msg).await;
            }
            _ = wait_conn_closed(reader) => {
                log("detect cancelled by client (connection closed)");
            }
        }
    }

    /// spec R5：识别成功后，用识别时的凭据 + 识别到的用户 id 构造一次性 client，回一条「已自动填入<字段>」
    /// 回执（不回显 ID 值）。best-effort——失败仅日志，不影响把 id 回设置进程。
    async fn send_detect_ack(req: &DetectRequest, id: &str, lang: Lang) {
        use crate::autochannel::detect_ack_text;
        let result: Result<(), String> = match req.kind.as_str() {
            "dingtalk" => {
                let field = crate::i18n::tr(lang, "autoChannel.detectFieldUserId");
                let cfg = crate::config::DingTalkChannelConfig {
                    enabled: true,
                    client_id: req.app_key.trim().to_string(),
                    client_secret: req.app_secret.trim().to_string(),
                    user_id: id.to_string(),
                    ..Default::default()
                };
                match crate::dingtalk::client::DingTalkClient::new(&cfg) {
                    Ok(client) => client
                        .send_oto_text(&detect_ack_text(field, lang))
                        .await
                        .map_err(|e| e.to_string()),
                    Err(e) => Err(e.to_string()),
                }
            }
            "feishu" => {
                let field = crate::i18n::tr(lang, "autoChannel.detectFieldOpenId");
                let cfg = crate::config::FeishuChannelConfig {
                    enabled: true,
                    app_id: req.app_key.trim().to_string(),
                    app_secret: req.app_secret.trim().to_string(),
                    open_id: id.to_string(),
                    base_url: req.base_url.trim().to_string(),
                };
                match crate::feishu::client::FeishuClient::new(&cfg) {
                    Ok(client) => client
                        .send_text(&detect_ack_text(field, lang))
                        .await
                        .map(|_| ())
                        .map_err(|e| e.to_string()),
                    Err(e) => Err(e.to_string()),
                }
            }
            "slack" => {
                let field = crate::i18n::tr(lang, "autoChannel.detectFieldUserId");
                let cfg = crate::config::SlackChannelConfig {
                    enabled: true,
                    bot_token: req.app_secret.trim().to_string(),
                    app_token: req.app_key.trim().to_string(),
                    user_id: id.to_string(),
                };
                match crate::slack::client::SlackClient::new(&cfg) {
                    Ok(client) => match client.open_dm().await {
                        Ok(dm) => client
                            .post_text(&dm, &detect_ack_text(field, lang))
                            .await
                            .map(|_| ())
                            .map_err(|e| e.to_string()),
                        Err(e) => Err(e.to_string()),
                    },
                    Err(e) => Err(e.to_string()),
                }
            }
            _ => Ok(()),
        };
        if let Err(e) = result {
            log(&format!("detect ack send failed ({}): {}", req.kind, e));
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
    /// `perf_id` 非空时把性能埋点开关 + 关联 id（及可选的 autodismiss）经 env 透传给 helper，
    /// 使其 GUI/前端各阶段标记并入同一条时间线。
    fn spawn_gui_helper(token: &str, perf_id: &str, perf_autodismiss: bool) -> std::io::Result<()> {
        use std::process::{Command, Stdio};
        let exe = std::env::current_exe()?;
        let mut cmd = Command::new(exe);
        cmd.arg("--popup")
            .arg("--endpoint")
            .arg(transport::socket_path())
            .arg("--token")
            .arg(token)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        if !perf_id.is_empty() {
            cmd.env("ASKHUMAN_PERF", "1");
            cmd.env("ASKHUMAN_PERF_ID", perf_id);
            if perf_autodismiss {
                cmd.env("ASKHUMAN_PERF_AUTODISMISS", "1");
            }
        }
        cmd.spawn().map(|_| ())
    }

    /// 方案6：spawn 一个预热弹窗进程（`--popup --warm`，无 token、无 perf env）。它会建好隐藏窗 + 挂载前端
    /// 后发 `GuiWarmReady` 入热池待命。
    fn spawn_warm_helper() -> std::io::Result<()> {
        use std::process::{Command, Stdio};
        let exe = std::env::current_exe()?;
        let mut cmd = Command::new(exe);
        cmd.arg("--popup")
            .arg("--warm")
            .arg("--endpoint")
            .arg(transport::socket_path())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        cmd.spawn().map(|_| ())
    }

    /// 弹窗预热是否启用（配置开关，读最近一次配置快照，无磁盘 I/O）。
    fn warm_enabled(state: &Arc<ServerState>) -> bool {
        state
            .config
            .lock()
            .map(|c| c.general.popup_prewarm)
            .unwrap_or(false)
    }

    /// 是否有可用显示（§D-M3）：无显示（headless）不预热，零浪费。macOS 恒真（GUI 会话）；
    /// Linux 看 `DISPLAY`/`WAYLAND_DISPLAY`。
    fn has_display() -> bool {
        #[cfg(target_os = "macos")]
        {
            true
        }
        #[cfg(all(unix, not(target_os = "macos")))]
        {
            std::env::var_os("DISPLAY").is_some() || std::env::var_os("WAYLAND_DISPLAY").is_some()
        }
    }

    /// 方案6 弹窗派发：优先领用热池中的预热弹窗（把请求 entry 交给其 holder 任务，秒级上屏）；
    /// 池空 / 开关关 / 无显示 / holder 已死 → 回退冷 spawn。返回弹窗是否成功就绪。
    fn dispatch_popup(
        entry: &Arc<request::RequestEntry>,
        state: &Arc<ServerState>,
        perf_id: &str,
        perf_autodismiss: bool,
    ) -> bool {
        dispatch_interaction_popup(
            InteractionEntry::Ask(entry.clone()),
            state,
            perf_id,
            perf_autodismiss,
        )
    }

    fn dispatch_interaction_popup(
        entry: InteractionEntry,
        state: &Arc<ServerState>,
        perf_id: &str,
        perf_autodismiss: bool,
    ) -> bool {
        // 取出热池槽（恒 ≤1）。
        let slot = state.warm_pool.lock().unwrap().take();
        if let Some(slot) = slot {
            match slot.assign.send(entry.clone()) {
                Ok(()) => {
                    crate::perf::mark(perf_id, "dmn.assigned");
                    // 领用成功 → 立即补热，维持池满。
                    maybe_topup_warm(state);
                    return true;
                }
                Err(_) => {
                    // holder 已死（罕见竞态）：池已 take 空，触发补热后回退冷路径。
                    maybe_topup_warm(state);
                }
            }
        }
        // 冷路径（兜底，完整保留）。
        match spawn_gui_helper(entry.token(), perf_id, perf_autodismiss) {
            Ok(()) => {
                crate::perf::mark(perf_id, "dmn.spawned");
                true
            }
            Err(e) => {
                log(&format!("failed to spawn GUI helper: {}", e));
                false
            }
        }
    }

    /// 方案6 补热（top-up，恒维持 1 个待命热实例）。自门控：开关关 / 无显示 / 排空中 / 池非空 / 已在补热
    /// 任一满足则不补。补热进程连上发 `GuiWarmReady` 后由 `handle_gui_warm` 入池并清 `warm_spawning`。
    fn maybe_topup_warm(state: &Arc<ServerState>) {
        if !warm_enabled(state) || !has_display() || state.draining.load(Ordering::SeqCst) {
            return;
        }
        if state.warm_pool.lock().unwrap().is_some() {
            return;
        }
        // 去重：已在补热中则跳过（避免并发 spawn 多个热实例）。
        if state.warm_spawning.swap(true, Ordering::SeqCst) {
            return;
        }
        match spawn_warm_helper() {
            Ok(()) => {
                log("preheating popup helper");
                // 兜底：若热进程崩溃于连接前，N 秒后清 `warm_spawning` 允许再次补热。
                let state = state.clone();
                tokio::spawn(async move {
                    tokio::time::sleep(Duration::from_secs(10)).await;
                    if state.warm_pool.lock().unwrap().is_none() {
                        state.warm_spawning.store(false, Ordering::SeqCst);
                    }
                });
            }
            Err(e) => {
                state.warm_spawning.store(false, Ordering::SeqCst);
                log(&format!("failed to preheat popup helper: {}", e));
            }
        }
    }

    /// 方案6 回收：清空热池槽（drop `assign`/`gui_tx` → holder 的 `assign_rx` 收 Err → 走死亡分支 →
    /// drop 其写端 → 热进程收 EOF 自杀）。用于关开关 / 进入排空 / 关停。
    fn recycle_warm(state: &Arc<ServerState>) {
        let slot = state.warm_pool.lock().unwrap().take();
        if slot.is_some() {
            log("recycling warm popup helper");
        }
        drop(slot);
        state.warm_spawning.store(false, Ordering::SeqCst);
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
        // 方案6：弹窗预热开关随配置热切换——开则补热（self-gated），关则回收现有热实例。
        // window_effect 变更时也必须回收：热进程建窗材质在 spawn 时固化（Glass 不挂 vibrancy），
        // 待命期切到 Blur 若仍领用旧进程且上屏只挂玻璃，会半透明无材质。重建后按新效果 apply_surface。
        let effect_changed = old.general.window_effect != new.general.window_effect;
        if warm_enabled(state) {
            if effect_changed {
                recycle_warm(state);
            }
            maybe_topup_warm(state);
        } else {
            recycle_warm(state);
        }
        // 保活开关热切换：同步 daemon 登录项（保活→写文件、否则→删文件）。关掉保活后不强杀本进程——
        // 空闲循环会按原 5min 策略让其自然退出。
        sync_daemon_login_item();
        log("config reloaded");
    }

    /// 让 daemon 登录项（开机自启）与 `general.daemonLifecycle` 一致。best-effort、幂等、纯文件操作。
    fn sync_daemon_login_item() {
        let keep_alive = crate::config::AppConfig::load_without_secrets()
            .general
            .daemon_lifecycle
            == crate::config::DaemonLifecycleMode::KeepAlive;
        let _ = crate::integrations::login_item::sync_daemon(keep_alive);
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
            let taken = reg
                .take("feishu")
                .expect("config change takes current stop");
            assert!(Arc::ptr_eq(&old, &taken));
            let new = reg
                .claim("feishu")
                .expect("new listener re-claims after take");
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
